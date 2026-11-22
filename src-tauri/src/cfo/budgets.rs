//! Budgets per project and per subagent (COO rebuild L9, design section 4).
//!
//! Both count real usage: tokens (input plus output, as the usage ledger
//! records them) and, for a subagent, working time. Dollars stay a side
//! estimate in the spend view and never limit anything here.
//!
//! - A **project budget** (`CfoSettings::project_budgets`) counts the tokens
//!   of every ledger row whose `workspace_id` is the project over the last 5
//!   hours and the last 7 days, rolling like the accounts' own windows.
//! - A **subagent budget** counts its whole chain (its handover successors
//!   too, its own consultations included, as `subagents` sums them). The
//!   policy's `subagent_budget` is every subagent's default; a row in
//!   `cfo_subagent_budgets` overrides it for one (`sessions.subagent_budget`).
//!
//! Only unattended work is bound (`run_pump::cfo_unattended`): the person's
//! own prompts always run. Reaching a budget:
//! - at claim, a subagent past its own budget fails with the pause note (a
//!   hold would wait forever, silently); a turn in a project past its budget
//!   waits ([`HOLD_PREFIX`]) until usage rolls out of the window;
//! - mid-turn, a running turn checks at each safe point ([`Watch`]) and ends
//!   there with the note, as the night guard does.
//!
//! The pause is recorded on the subagent (`paused_at`, `paused_reason`), so
//! every report its parent gets carries it (`descendants::child_summary`),
//! and the person gets one Hollow notification per pause.

use super::settings::{self, CfoSettings, ProjectBudget, SubagentBudget};
use crate::session_store;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::Instant;

/// How a held row's note starts (a project past its budget).
pub const HOLD_PREFIX: &str = "CFO budget: ";
/// How a paused subagent's note starts.
pub const PAUSE_PREFIX: &str = "Paused by the CFO: ";
/// A running turn reads the ledger at most this often.
const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(15);
/// One notification per project and window at most this often.
const PROJECT_NOTICE_EVERY: Duration = Duration::hours(5);

/// What a subagent has used so far.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Used {
    pub tokens: u64,
    pub working_ms: i64,
}

fn group(n: u64) -> String {
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

/// The pause note when `used` reached `budget`, else none. Tokens first.
pub fn subagent_reached(budget: &SubagentBudget, used: Used) -> Option<String> {
    let detail = if let Some(cap) = budget.tokens.filter(|cap| used.tokens >= *cap) {
        format!("{} tokens of its {}", group(used.tokens), group(cap))
    } else if let Some(cap) = budget
        .minutes
        .filter(|cap| used.working_ms >= i64::from(*cap) * 60_000)
    {
        format!(
            "{} minutes of working time of its {cap}",
            used.working_ms / 60_000
        )
    } else {
        return None;
    };
    Some(format!(
        "{PAUSE_PREFIX}this subagent reached its budget ({detail}). The conversation that started it can raise it with sessions.subagent_budget and message it again, or stop it."
    ))
}

/// The hold note when a project's usage reached its budget, else none.
pub fn project_reached(
    budget: &ProjectBudget,
    title: &str,
    five_hour: u64,
    week: u64,
) -> Option<String> {
    let (used, cap, window) = if let Some(cap) = budget.five_hour_tokens.filter(|c| five_hour >= *c)
    {
        (five_hour, cap, "the last 5 hours")
    } else if let Some(cap) = budget.week_tokens.filter(|c| week >= *c) {
        (week, cap, "the last 7 days")
    } else {
        return None;
    };
    Some(format!(
        "the project “{title}” used {} tokens in {window}, past its budget of {} (Settings, Usage). Its unattended work waits; your own prompts still run.",
        group(used),
        group(cap)
    ))
}

/// Tokens of the project's ledger rows recorded at or after `since`.
pub(crate) fn project_tokens(
    conn: &Connection,
    workspace_id: &str,
    since: DateTime<Utc>,
) -> Result<u64, String> {
    // Every id the project has had (Move Hollow home changes it; see
    // `workspace_aliases`), so a budget set before a move still counts.
    let ids = crate::workspace_aliases::Aliases::load(conn).ids_for(workspace_id);
    let sql = format!(
        "SELECT COALESCE(SUM(COALESCE(input_tokens, 0) + COALESCE(output_tokens, 0)), 0) \
         FROM usage_ledger WHERE workspace_id IN ({}) AND at >= ?",
        crate::workspace_aliases::placeholders(ids.len())
    );
    let mut args: Vec<String> = ids;
    args.push(session_store::db_time(since));
    let sum: i64 = conn
        .query_row(&sql, rusqlite::params_from_iter(args.iter()), |r| r.get(0))
        .map_err(|e| format!("project usage: {e}"))?;
    Ok(sum.max(0) as u64)
}

/// The project's tokens over the last 5 hours and the last 7 days.
pub(crate) fn project_windows(
    vault: &Path,
    workspace_id: &str,
    now: DateTime<Utc>,
) -> Result<(u64, u64), String> {
    session_store::with_read(vault, |conn| {
        Ok((
            project_tokens(conn, workspace_id, now - Duration::hours(5))?,
            project_tokens(conn, workspace_id, now - Duration::days(7))?,
        ))
    })
}

/// `key`'s handover chain, its last conversation first, then the ones that
/// led to it (`key` always in it). Handovers only: a conversation in the
/// COO's folder is not chained to every COO thread here (delta review), so
/// a subagent started there counts only its own usage.
pub(crate) fn budget_chain(vault: &Path, key: &str) -> Vec<String> {
    let tail = crate::chat_chain::handover_tail(vault, key);
    let mut out = vec![tail.clone()];
    for earlier in crate::chat_chain::predecessors(vault, &tail) {
        if !out.contains(&earlier) {
            out.push(earlier);
        }
    }
    if !out.iter().any(|k| k == key) {
        out.push(key.to_string());
    }
    out
}

/// The subagent `key` belongs to: `(started key, parent key)` when `key`
/// or a conversation its handover chain came from was started as a
/// subagent. `chain` is [`budget_chain`]`(key)`.
fn subagent_of(conn: &Connection, chain: &[String]) -> Result<Option<(String, String)>, String> {
    for key in chain {
        let row: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT parent_key, fork_kind FROM sessions WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("subagent lookup: {e}"))?;
        if let Some((Some(parent), Some(kind))) = row {
            if kind == session_store::FORK_SUBAGENT && !parent.trim().is_empty() {
                return Ok(Some((key.clone(), parent)));
            }
        }
    }
    Ok(None)
}

/// One subagent's override row, newest across its chain.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Override {
    pub budget: SubagentBudget,
    pub paused_reason: Option<String>,
}

fn override_of(conn: &Connection, chain: &[String]) -> Result<Option<Override>, String> {
    if chain.is_empty() {
        return Ok(None);
    }
    let marks = vec!["?"; chain.len()].join(",");
    conn.query_row(
        &format!(
            "SELECT tokens, minutes, paused_reason FROM cfo_subagent_budgets \
             WHERE session_key IN ({marks}) ORDER BY set_at DESC LIMIT 1"
        ),
        rusqlite::params_from_iter(chain.iter()),
        |r| {
            Ok(Override {
                budget: SubagentBudget {
                    tokens: r.get::<_, Option<i64>>(0)?.map(|t| t.max(0) as u64),
                    minutes: r
                        .get::<_, Option<i64>>(1)?
                        .map(|m| m.clamp(0, i64::from(u32::MAX)) as u32),
                },
                paused_reason: r
                    .get::<_, Option<String>>(2)?
                    .filter(|s| !s.trim().is_empty()),
            })
        },
    )
    .optional()
    .map_err(|e| format!("subagent budget: {e}"))
}

/// The budget that binds a subagent: each figure its override names, else
/// the policy's default.
pub(crate) fn effective(settings: &CfoSettings, over: Option<&Override>) -> SubagentBudget {
    let default = &settings.subagent_budget;
    match over {
        Some(o) => SubagentBudget {
            tokens: o.budget.tokens.or(default.tokens),
            minutes: o.budget.minutes.or(default.minutes),
        },
        None => default.clone(),
    }
}

/// What binds one conversation's unattended turns, read once.
#[derive(Debug, Clone, Default)]
pub(crate) struct Binding {
    /// `(started key, parent key, budget, used so far)` when it is a
    /// subagent with a budget.
    pub subagent: Option<(String, String, SubagentBudget, Used)>,
    /// The project's budget and title when it has one.
    pub project: Option<(ProjectBudget, String)>,
    /// `(started key, parent key)` when it is a subagent at all
    /// (with or without a budget of its own): a project budget ends only a
    /// subagent's turn mid-way.
    pub project_subagent: Option<(String, String)>,
    /// Its handover chain ([`budget_chain`]), when it is a subagent.
    pub chain: Vec<String>,
    /// What that chain used when this was read, when it is a subagent.
    pub base: Used,
}

impl Binding {
    pub fn is_empty(&self) -> bool {
        self.subagent.is_none() && self.project.is_none()
    }
}

/// The workspace a turn in `cwd` belongs to, with its title.
fn workspace_of(vault: &Path, cwd: &Path) -> Option<(String, String)> {
    crate::workspaces::workspace_for_cwd(vault, cwd)
        .ok()
        .flatten()
        .map(|w| (w.id, w.title))
}

/// What binds conversation `key` (working in `cwd`) under `settings`.
pub(crate) fn binding(
    vault: &Path,
    settings: &CfoSettings,
    key: &str,
    cwd: Option<&Path>,
    now: DateTime<Utc>,
) -> Result<Binding, String> {
    let mut out = Binding::default();
    if let Some((id, title)) = cwd.and_then(|cwd| workspace_of(vault, cwd)) {
        if let Some(budget) = settings
            .project_budgets
            .iter()
            .find(|b| b.workspace_id == id)
        {
            out.project = Some((budget.clone(), title));
        }
    }
    let chain = budget_chain(vault, key);
    let found = session_store::with_read(vault, |conn| {
        let Some((started, parent)) = subagent_of(conn, &chain)? else {
            return Ok(None);
        };
        let over = override_of(conn, &chain)?;
        Ok(Some((started, parent, over)))
    })?;
    if let Some((started, parent, over)) = found {
        out.project_subagent = Some((started.clone(), parent.clone()));
        let budget = effective(settings, over.as_ref());
        if !budget.is_empty() || out.project.is_some() {
            out.base = session_store::with_read(vault, |conn| {
                crate::subagents::chain_usage(conn, &chain, now)
            })?;
        }
        if !budget.is_empty() {
            out.subagent = Some((started, parent, budget, out.base));
        }
        out.chain = chain;
    }
    Ok(out)
}

/// Record the pause on the subagent (its override row; the figures it had
/// stay) and tell the person once.
pub(crate) fn record_pause(
    vault: &Path,
    started: &str,
    parent: &str,
    note: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let at = session_store::db_time(now);
    let first = session_store::with_write(vault, "subagent budget pause", |tx| {
        let was: Option<Option<String>> = tx
            .query_row(
                "SELECT paused_at FROM cfo_subagent_budgets WHERE session_key = ?1",
                [started],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("subagent pause: {e}"))?;
        tx.execute(
            "INSERT INTO cfo_subagent_budgets(session_key, parent_key, paused_at, paused_reason, set_at) \
             VALUES(?1, ?2, ?3, ?4, ?3) \
             ON CONFLICT(session_key) DO UPDATE SET parent_key = excluded.parent_key, \
             paused_at = excluded.paused_at, paused_reason = excluded.paused_reason",
            params![started, parent, at, note],
        )
        .map_err(|e| format!("subagent pause: {e}"))?;
        Ok(!matches!(was, Some(Some(_))))
    })?;
    if first {
        notify(
            vault,
            crate::notifications::NotificationInput {
                title: format!("Subagent paused: {}", title_of(vault, started)),
                body: note.to_string(),
                kind: "warning".into(),
                source: Some("cfo".into()),
                action_label: None,
                action_url: None,
                notify_native: false,
            },
        );
    }
    Ok(())
}

fn title_of(vault: &Path, key: &str) -> String {
    session_store::with_read(vault, |conn| {
        conn.query_row("SELECT title FROM sessions WHERE key = ?1", [key], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .flatten()
    .filter(|t| !t.trim().is_empty())
    .unwrap_or_else(|| key.to_string())
}

#[cfg(test)]
static SENT: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Titles of the notifications sent (tests).
#[cfg(test)]
pub(crate) fn sent_for_test() -> Vec<String> {
    SENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn notify(vault: &Path, input: crate::notifications::NotificationInput) {
    #[cfg(test)]
    SENT.lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(input.title.clone());
    let _ = crate::notifications::create_registered(vault, input);
}

/// When each project's last notice went out, so a hold rechecked every
/// minute tells the person once per window.
static PROJECT_NOTICES: Mutex<Option<HashMap<String, DateTime<Utc>>>> = Mutex::new(None);

fn notify_project(vault: &Path, workspace_id: &str, title: &str, note: &str, now: DateTime<Utc>) {
    {
        let mut guard = PROJECT_NOTICES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        let key = format!("{}|{workspace_id}", vault.display());
        if map
            .get(&key)
            .is_some_and(|at| now - *at < PROJECT_NOTICE_EVERY)
        {
            return;
        }
        map.insert(key, now);
    }
    notify(
        vault,
        crate::notifications::NotificationInput {
            title: format!("Project budget reached: {title}"),
            body: format!("The CFO holds {note}"),
            kind: "warning".into(),
            source: Some("cfo".into()),
            action_label: Some("Open Usage".into()),
            action_url: Some(crate::budgets::USAGE_ACTION.into()),
            notify_native: false,
        },
    );
}

/// What the pump does with an unattended chat turn of `key` in `cwd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ClaimBudget {
    /// A subagent past its own budget: the turn fails with this note.
    Fail(String),
    /// A project past its budget: the turn waits with this note.
    Hold(String),
}

/// The claim check (`run_pump::claim_verdict`) for an unattended chat turn.
/// A budget that can't be read never holds work (the reserve and the
/// existing budgets still apply).
pub(crate) fn claim(
    vault: &Path,
    key: &str,
    cwd: Option<&Path>,
    now: DateTime<Utc>,
) -> Option<ClaimBudget> {
    let settings = settings::peek(vault).ok()?;
    if settings.project_budgets.is_empty() && settings.subagent_budget.is_empty() {
        // Only an override could bind: skip the reads when none exists.
        let any: bool = session_store::with_read(vault, |conn| {
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM cfo_subagent_budgets)",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap_or(false);
        if !any {
            return None;
        }
    }
    let bound = binding(vault, &settings, key, cwd, now).ok()?;
    if let Some((started, parent, budget, used)) = &bound.subagent {
        if let Some(note) = subagent_reached(budget, *used) {
            if let Err(error) = record_pause(vault, started, parent, &note, now) {
                eprintln!("[cfo] recording a subagent pause: {error}");
            }
            return Some(ClaimBudget::Fail(note));
        }
    }
    if let Some((budget, title)) = &bound.project {
        let (five, week) = project_windows(vault, &budget.workspace_id, now).ok()?;
        if let Some(note) = project_reached(budget, title, five, week) {
            notify_project(vault, &budget.workspace_id, title, &note, now);
            return Some(ClaimBudget::Hold(format!("{HOLD_PREFIX}{note}")));
        }
    }
    // It runs: an earlier pause no longer holds (a raised budget, usage
    // rolled out of the window).
    if bound.project_subagent.is_some() {
        clear_pause(vault, key);
    }
    None
}

/// A running unattended turn's budget guard: checked at each safe point,
/// the ledger and the budgets read at most every [`CHECK_EVERY`] (a budget
/// raised while the turn runs counts at the next check: Codex review).
pub(crate) struct Watch {
    vault: std::path::PathBuf,
    bound: Binding,
    started: Instant,
    last: Option<Instant>,
    /// The pause `check` found, recorded only once the turn really yields
    /// to it ([`Watch::commit`]): a person's Stop at the same moment wins.
    pending: Option<(String, String, String)>,
}

impl Watch {
    /// The guard for an unattended turn of `key`, or none when no budget
    /// binds it (or the budgets can't be read).
    pub(crate) fn new(vault: &Path, key: &str, cwd: Option<&Path>) -> Option<Watch> {
        let settings = settings::peek(vault).ok()?;
        let bound = binding(vault, &settings, key, cwd, Utc::now()).ok()?;
        (!bound.is_empty()).then(|| Watch {
            vault: vault.to_path_buf(),
            bound,
            started: Instant::now(),
            last: None,
            pending: None,
        })
    }

    /// The pause note once this turn (with `own_tokens` so far) takes its
    /// subagent or project past a budget, else none. Nothing is recorded
    /// here; the turn calls [`Watch::commit`] once it has yielded.
    pub(crate) fn check(&mut self, own_tokens: u64) -> Option<String> {
        if self.last.is_some_and(|at| at.elapsed() < CHECK_EVERY) {
            return None;
        }
        self.last = Some(Instant::now());
        let now = Utc::now();
        let settings = settings::peek(&self.vault).ok()?;
        let ident = self.bound.project_subagent.clone();
        if let Some((started, parent)) = &ident {
            let over =
                session_store::with_read(&self.vault, |conn| override_of(conn, &self.bound.chain))
                    .ok()
                    .flatten();
            let budget = effective(&settings, over.as_ref());
            // `base` was read as this turn began (its open turn counted up
            // to then); the time since is `started`'s.
            let used = Used {
                tokens: self.bound.base.tokens.saturating_add(own_tokens),
                working_ms: self
                    .bound
                    .base
                    .working_ms
                    .saturating_add(self.started.elapsed().as_millis() as i64),
            };
            if let Some(note) = subagent_reached(&budget, used) {
                self.pending = Some((started.clone(), parent.clone(), note.clone()));
                return Some(note);
            }
        }
        let project = self.bound.project.as_ref().and_then(|(was, title)| {
            settings
                .project_budgets
                .iter()
                .find(|b| b.workspace_id == was.workspace_id)
                .map(|b| (b.clone(), title.clone()))
        });
        if let Some((budget, title)) = project {
            // The ledger already holds this turn's checkpoints.
            if let Ok((five, week)) = project_windows(&self.vault, &budget.workspace_id, now) {
                if let Some(note) = project_reached(&budget, &title, five, week) {
                    notify_project(&self.vault, &budget.workspace_id, &title, &note, now);
                    // Only a subagent is ended mid-turn; the project's other
                    // unattended turns wait at their next claim.
                    if let Some((started, parent)) = ident {
                        let paused = format!("{PAUSE_PREFIX}{note}");
                        self.pending = Some((started, parent, paused.clone()));
                        return Some(paused);
                    }
                }
            }
        }
        None
    }

    /// Record the pause `check` found, once the turn has yielded to it.
    pub(crate) fn commit(&self) {
        if let Some((started, parent, note)) = &self.pending {
            if let Err(error) = record_pause(&self.vault, started, parent, note, Utc::now()) {
                eprintln!("[cfo] recording a subagent pause: {error}");
            }
        }
    }
}

/// Clear the recorded pause of the subagent `key` belongs to: it runs again
/// (a raised budget, usage rolled out of a project's window, or the
/// person's own prompt). Cheap when nothing is paused.
pub(crate) fn clear_pause(vault: &Path, key: &str) {
    let any: bool = session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM cfo_subagent_budgets WHERE paused_at IS NOT NULL)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap_or(false);
    if !any {
        return;
    }
    let chain = budget_chain(vault, key);
    let marks = vec!["?"; chain.len()].join(",");
    let cleared = session_store::with_write(vault, "subagent budget pause", |tx| {
        tx.execute(
            &format!(
                "UPDATE cfo_subagent_budgets SET paused_at = NULL, paused_reason = NULL \
                 WHERE paused_at IS NOT NULL AND session_key IN ({marks})"
            ),
            rusqlite::params_from_iter(chain.iter()),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    });
    if let Err(error) = cleared {
        eprintln!("[cfo] clearing a subagent pause: {error}");
    }
}

/// The recorded pause of the subagent `key` belongs to, for its parent's
/// reports (`descendants::child_summary`).
pub(crate) fn pause_note(vault: &Path, key: &str) -> Option<String> {
    let chain = budget_chain(vault, key);
    session_store::with_read(vault, |conn| {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='cfo_subagent_budgets'",
                [],
                |_| Ok(true),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .unwrap_or(false);
        if !exists {
            return Ok(None);
        }
        Ok(override_of(conn, &chain)?.and_then(|o| o.paused_reason))
    })
    .ok()
    .flatten()
}

/// Whether a conversation of `own` (the caller's chain) is `parent`'s chain
/// or above it in the subagent line (at most 16 steps up).
fn ancestor_of(vault: &Path, parent: &str, own: &[String]) -> Result<bool, String> {
    let mut at = parent.to_string();
    for _ in 0..16 {
        let chain = budget_chain(vault, &at);
        if chain.iter().any(|k| own.contains(k)) {
            return Ok(true);
        }
        match session_store::with_read(vault, |conn| subagent_of(conn, &chain))? {
            Some((_, up)) if up != at => at = up,
            _ => return Ok(false),
        }
    }
    Ok(false)
}

/// `sessions.subagent_budget`: set one subagent's budget (a figure left
/// out keeps the default) and clear its pause. Refused for the caller's own
/// conversation or chain (a subagent never raises its own budget) and for a
/// conversation that is no subagent.
pub(crate) fn set_subagent_budget(
    vault: &Path,
    caller_scope: Option<&str>,
    session: &str,
    tokens: Option<u64>,
    minutes: Option<u32>,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let session = session.trim();
    let budget = SubagentBudget { tokens, minutes };
    settings::check_subagent_budget(&budget).map_err(|e| {
        format!(
            "{} Nothing changed.",
            e.trim_start_matches("CFO_SETTINGS: ")
        )
    })?;
    let chain = budget_chain(vault, session);
    let Some(scope) = caller_scope else {
        return Err(
            "subagent_budget: only a conversation sets a subagent's budget (the one that started it, or the COO). Nothing changed."
                .into(),
        );
    };
    let own = budget_chain(vault, scope);
    if chain.iter().any(|k| own.contains(k)) {
        return Err(
            "subagent_budget: a conversation can't change its own budget; the one that started it (or the COO) can. Nothing changed."
                .into(),
        );
    }
    let Some((started, parent)) =
        session_store::with_read(vault, |conn| subagent_of(conn, &chain))?
    else {
        return Err(format!(
            "subagent_budget: {session} is not a subagent (only conversations another one started have a budget). Nothing changed."
        ));
    };
    // Review: only the COO or a conversation up the subagent's own line
    // (its parent, the parent's parent, ...) may change it: never a
    // sibling or one of its own subagents.
    // The COO is a conversation in its folder that is no subagent itself
    // (delta review: a subagent started in the COO's folder is not the COO).
    let coo = own
        .iter()
        .any(|k| crate::chat_chain::is_main_conversation(vault, k))
        && session_store::with_read(vault, |conn| subagent_of(conn, &own))?.is_none();
    if !coo && !ancestor_of(vault, &parent, &own)? {
        return Err(
            "subagent_budget: only the conversation that started this subagent (or one above it, or the COO) can change its budget. Nothing changed."
                .into(),
        );
    }
    let at = session_store::db_time(now);
    session_store::with_write(vault, "subagent budget", |tx| {
        tx.execute(
            "INSERT INTO cfo_subagent_budgets(session_key, tokens, minutes, parent_key, paused_at, paused_reason, set_at) \
             VALUES(?1, ?2, ?3, ?4, NULL, NULL, ?5) \
             ON CONFLICT(session_key) DO UPDATE SET tokens = excluded.tokens, minutes = excluded.minutes, \
             parent_key = excluded.parent_key, paused_at = NULL, paused_reason = NULL, set_at = excluded.set_at",
            params![
                started,
                tokens.map(|t| t as i64),
                minutes.map(i64::from),
                parent,
                at
            ],
        )
        .map_err(|e| format!("subagent budget: {e}"))?;
        // Other rows of the chain (a successor's pause) are cleared too.
        let marks = vec!["?"; chain.len()].join(",");
        tx.execute(
            &format!(
                "UPDATE cfo_subagent_budgets SET paused_at = NULL, paused_reason = NULL \
                 WHERE session_key IN ({marks})"
            ),
            rusqlite::params_from_iter(chain.iter()),
        )
        .map_err(|e| format!("subagent budget: {e}"))?;
        Ok(())
    })?;
    let settings = settings::peek(vault).unwrap_or_default();
    let over = Override {
        budget: SubagentBudget { tokens, minutes },
        paused_reason: None,
    };
    let now_budget = effective(&settings, Some(&over));
    Ok(json!({
        "session": started,
        "tokens": now_budget.tokens,
        "minutes": now_budget.minutes,
        "note": "Set, and any pause cleared. Message the subagent to let it go on; a figure left out uses the default from Settings, Usage.",
    }))
}

#[cfg(test)]
#[path = "budgets_tests.rs"]
mod tests;
