//! Run ledger v2 reads (master plan F2, 2026-10-08): outcome labels and the
//! grouped ledger query behind `usage.ledger_query` and the value ledger.
//! `typical_duration_ms` is the usual length of a finished chat of one
//! work kind (`usage.job_timing`). It is not the hours that kind is said
//! to save.
//!
//! **Outcome labels.** `usage_ledger.outcome` says how a run's process ended
//! (`ok`, `failed`, `abandoned` = dropped unfinished, …). `outcome_label`
//! says what became of its work, and is derived, never typed in:
//! - `accepted`: a person accepted a Result whose accepted version this run
//!   wrote, and it is unchanged since (`workspace_results`, acceptance
//!   `runId`); or (G4, 2026-11-10) the tracker issue the run is tied to
//!   (`work_links`) is Done now and was completed after the run started
//!   (`tracker::work::accepted_by_issue`). Both count the run once; a
//!   reopened issue stops counting at the next read.
//! - `revised`: that Result was accepted and has changed since.
//! - `abandoned`: the run ended without finishing (an error, a timeout,
//!   cancelled, interrupted, stopped by a budget, never started), or the person marked
//!   the turn not useful (`turn_feedback`). Note the clash with the
//!   `outcome` column's own `abandoned` (a run dropped unfinished): that is
//!   one of the ways a label becomes `abandoned`.
//! - `unknown`: everything else (no Result accepted, no vote). A useful vote
//!   is not an accepted Result, so it stays `unknown`.
//!
//! An accepted Result outranks a not-useful vote on the same run. Every row
//! of a run (its helpers and teammates carry the run id) gets the run's
//! label. [`refresh_outcome_labels`] recomputes them before every read, so a
//! later acceptance is reflected without a hook in the Results code.
//!
//! **Unknown is never zero.** A row without a cost (Codex, Antigravity, an
//! unpriced model) is counted in `unknownCost`, not added as $0.

use crate::run_attribution::UNKNOWN;
use crate::session_store;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// The ledger outcomes whose work was not finished.
const UNFINISHED: [&str; 7] = [
    "error",
    "cancelled",
    "timeout",
    "abandoned",
    "interrupted",
    "budget_stopped",
    "spawn_failed",
];

/// What a run's Results say: `accepted` or `revised` per history run id.
pub fn result_labels(root: &Path) -> HashMap<String, &'static str> {
    let mut labels: HashMap<String, &'static str> = HashMap::new();
    let results =
        crate::workspace_results::list_all_unattended(root, None, usize::MAX).unwrap_or_default();
    for item in results {
        let r = item.result;
        let Some(run_id) = r.acceptance.as_ref().and_then(|a| a.run_id.clone()) else {
            continue;
        };
        let label = match r.review.as_str() {
            "accepted" => "accepted",
            "changed" => "revised",
            _ => continue,
        };
        let entry = labels.entry(run_id).or_insert(label);
        if label == "accepted" {
            *entry = "accepted";
        }
    }
    labels
}

/// `results` with every run accepted through its issue marked `accepted`
/// (an issue reaching Done outranks a revised Result).
pub fn with_issue_done(
    mut results: HashMap<String, &'static str>,
    issue_done: &std::collections::BTreeSet<String>,
) -> HashMap<String, &'static str> {
    for run_id in issue_done {
        results.insert(run_id.clone(), "accepted");
    }
    results
}

/// What counts as accepted now: the Results' labels plus the runs whose
/// tracker issue reached Done after they started (G4).
pub fn acceptance_labels(root: &Path) -> HashMap<String, &'static str> {
    with_issue_done(
        result_labels(root),
        &crate::tracker::work::accepted_by_issue(root),
    )
}

/// The label of one run, from its Results, its turn vote and how it ended.
pub fn label_for(
    result: Option<&'static str>,
    vote: Option<&str>,
    outcome: &str,
) -> Option<&'static str> {
    if outcome == "running" {
        return None;
    }
    if let Some(label) = result {
        return Some(label);
    }
    if vote == Some("notUseful") || UNFINISHED.contains(&outcome) {
        return Some("abandoned");
    }
    Some(UNKNOWN)
}

/// Recompute every finished row's `outcome_label`; returns how many changed.
/// `results` is [`result_labels`] (a parameter so tests can supply it).
pub fn refresh_outcome_labels_with(
    vault: &Path,
    results: &HashMap<String, &'static str>,
) -> Result<usize, String> {
    let (rows, votes) = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT run_key, run_id, outcome, outcome_label FROM usage_ledger")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        let mut stmt = conn
            .prepare("SELECT run_id, vote FROM turn_feedback")
            .map_err(|e| e.to_string())?;
        let votes = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect::<HashMap<_, _>>();
        Ok((rows, votes))
    })?;
    let changes: Vec<(String, Option<&'static str>)> = rows
        .into_iter()
        .filter_map(|(key, run_id, outcome, current)| {
            let result = run_id.as_deref().and_then(|id| results.get(id).copied());
            let vote = run_id
                .as_deref()
                .and_then(|id| votes.get(id))
                .map(String::as_str);
            let label = label_for(result, vote, &outcome);
            (label != current.as_deref()).then_some((key, label))
        })
        .collect();
    if changes.is_empty() {
        return Ok(0);
    }
    session_store::with_write(vault, "outcome labels", |tx| {
        let mut stmt = tx
            .prepare("UPDATE usage_ledger SET outcome_label = ?2 WHERE run_key = ?1")
            .map_err(|e| e.to_string())?;
        for (key, label) in &changes {
            stmt.execute(params![key, label])
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })?;
    Ok(changes.len())
}

/// [`refresh_outcome_labels_with`] from the vault's own Results and the
/// tracker issues its runs finished.
pub fn refresh_outcome_labels(vault: &Path) -> Result<usize, String> {
    refresh_outcome_labels_with(vault, &acceptance_labels(vault))
}

/// One ledger row as v2 reads it. Attribution values are `unknown` when the
/// row has none (every row written before v2).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRow {
    pub run_key: String,
    pub run_id: Option<String>,
    pub at: String,
    pub surface: String,
    pub agent: String,
    pub model: String,
    pub outcome: String,
    pub spawned: bool,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub account: String,
    pub workspace: String,
    pub department: String,
    pub work_kind: String,
    pub outcome_label: String,
}

/// Rows whose `at` falls in `[from, to)`.
pub fn rows_between(vault: &Path, from: &str, to: &str) -> Result<Vec<LedgerRow>, String> {
    let unknown = |v: Option<String>| v.filter(|v| !v.trim().is_empty()).unwrap_or(UNKNOWN.into());
    session_store::with_read(vault, |conn| {
        // A row recorded under a workspace's id from before Move Hollow home
        // reads as the id it has now (`workspace_aliases`).
        let aliases = crate::workspace_aliases::Aliases::load(conn);
        let mut stmt = conn
            .prepare(
                "SELECT run_key, run_id, at, surface, agent, COALESCE(model, requested_model), \
                 outcome, spawned, cost_usd, input_tokens, output_tokens, account_ref, \
                 workspace_id, department, work_kind, outcome_label FROM usage_ledger \
                 WHERE at >= ?1 AND at < ?2 ORDER BY at ASC, id ASC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![from, to], |row| {
                Ok(LedgerRow {
                    run_key: row.get(0)?,
                    run_id: row.get(1)?,
                    at: row.get(2)?,
                    surface: row.get(3)?,
                    agent: unknown(row.get(4)?),
                    model: unknown(row.get(5)?),
                    outcome: row.get(6)?,
                    spawned: row.get::<_, i64>(7)? != 0,
                    cost_usd: row.get(8)?,
                    input_tokens: row.get(9)?,
                    output_tokens: row.get(10)?,
                    account: unknown(row.get(11)?),
                    workspace: aliases.current(&unknown(row.get(12)?)),
                    department: unknown(row.get(13)?),
                    work_kind: unknown(row.get(14)?),
                    outcome_label: unknown(row.get(15)?),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    })
}

/// The dimensions `usage.ledger_query` can group and filter by.
pub const DIMENSIONS: [&str; 11] = [
    "account",
    "workspace",
    "department",
    "workKind",
    "outcomeLabel",
    "agent",
    "model",
    "surface",
    "outcome",
    "day",
    "month",
];

impl LedgerRow {
    /// The row's value for a [`DIMENSIONS`] entry (`day`/`month` in local time).
    pub fn dimension(&self, dimension: &str) -> String {
        match dimension {
            "account" => self.account.clone(),
            "workspace" => self.workspace.clone(),
            "department" => self.department.clone(),
            "workKind" => self.work_kind.clone(),
            "outcomeLabel" => self.outcome_label.clone(),
            "agent" => self.agent.clone(),
            "model" => self.model.clone(),
            "surface" => self.surface.clone(),
            "outcome" => self.outcome.clone(),
            "day" | "month" => DateTime::parse_from_rfc3339(&self.at)
                .map(|at| {
                    let at = at.with_timezone(&Local);
                    if dimension == "day" {
                        at.format("%Y-%m-%d").to_string()
                    } else {
                        at.format("%Y-%m").to_string()
                    }
                })
                .unwrap_or_else(|_| UNKNOWN.into()),
            _ => UNKNOWN.into(),
        }
    }
}

/// Spend and runs for one group. `cost_usd` sums known costs only, rounded
/// to the cent at the end; `unknown_cost` counts started runs with no cost.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Totals {
    pub runs: u64,
    pub cost_usd: f64,
    pub unknown_cost: u64,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

impl Totals {
    pub fn add(&mut self, row: &LedgerRow) {
        self.runs += 1;
        match row.cost_usd {
            Some(cost) => self.cost_usd += cost,
            None if row.spawned => self.unknown_cost += 1,
            None => {}
        }
        self.input_tokens += row.input_tokens.unwrap_or(0);
        self.output_tokens += row.output_tokens.unwrap_or(0);
    }

    fn rounded(mut self) -> Self {
        self.cost_usd = cents(self.cost_usd);
        self
    }
}

pub fn cents(usd: f64) -> f64 {
    (usd * 100.0).round() / 100.0
}

/// A date (`YYYY-MM-DD`, local midnight) or an RFC 3339 time, as the
/// ledger's `at` format.
pub fn parse_bound(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if let Ok(day) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let local = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
            .earliest()
            .ok_or_else(|| format!("'{raw}' has no local midnight"))?;
        return Ok(session_store::db_time(local.with_timezone(&Utc)));
    }
    DateTime::parse_from_rfc3339(raw)
        .map(|at| session_store::db_time(at.with_timezone(&Utc)))
        .map_err(|_| format!("'{raw}' is not a date (YYYY-MM-DD) or an RFC 3339 time."))
}

/// What `usage.ledger_query` asks.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub from: Option<String>,
    pub to: Option<String>,
    pub group_by: Vec<String>,
    pub filters: BTreeMap<String, String>,
}

/// Group and filter the ledger. Defaults: the last 30 days, no grouping.
pub fn query(vault: &Path, q: &Query, now: DateTime<Utc>) -> Result<Value, String> {
    query_with(vault, q, now, &acceptance_labels(vault))
}

/// [`query`] with the Results' labels supplied (tests).
pub fn query_with(
    vault: &Path,
    q: &Query,
    now: DateTime<Utc>,
    results: &HashMap<String, &'static str>,
) -> Result<Value, String> {
    for dimension in q.group_by.iter().chain(q.filters.keys()) {
        if !DIMENSIONS.contains(&dimension.as_str()) {
            return Err(format!(
                "Unknown ledger dimension '{dimension}'. Use one of: {}.",
                DIMENSIONS.join(", ")
            ));
        }
    }
    if q.group_by.len() > 3 {
        return Err("Group by at most three dimensions.".into());
    }
    let from = match &q.from {
        Some(raw) => parse_bound(raw)?,
        None => session_store::db_time(now - chrono::Duration::days(30)),
    };
    let to = match &q.to {
        Some(raw) => parse_bound(raw)?,
        None => session_store::db_time(now + chrono::Duration::seconds(1)),
    };
    if let Err(error) = refresh_outcome_labels_with(vault, results) {
        eprintln!("[usage] outcome labels not refreshed: {error}");
    }
    let rows = rows_between(vault, &from, &to)?;
    // Rows carry the workspace id they have now (`rows_between`); an old id
    // asked for (from before Move Hollow home) means the same workspace.
    let aliases = session_store::with_read(vault, |conn| {
        Ok(crate::workspace_aliases::Aliases::load(conn))
    })
    .unwrap_or_default();
    let filters: BTreeMap<&String, String> = q
        .filters
        .iter()
        .map(|(dimension, want)| {
            let want = if dimension == "workspace" {
                aliases.current(want.trim())
            } else {
                want.clone()
            };
            (dimension, want)
        })
        .collect();
    let mut totals = Totals::default();
    let mut groups: BTreeMap<Vec<String>, Totals> = BTreeMap::new();
    for row in rows.iter().filter(|row| {
        filters
            .iter()
            .all(|(dimension, want)| row.dimension(dimension).eq_ignore_ascii_case(want))
    }) {
        totals.add(row);
        if !q.group_by.is_empty() {
            let key = q.group_by.iter().map(|d| row.dimension(d)).collect();
            groups.entry(key).or_default().add(row);
        }
    }
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|(key, t)| {
            let key: serde_json::Map<String, Value> = q
                .group_by
                .iter()
                .cloned()
                .zip(key.into_iter().map(Value::String))
                .collect();
            json!({ "key": key, "totals": t.rounded() })
        })
        .collect();
    Ok(json!({
        "from": from,
        "to": to,
        "groupBy": q.group_by,
        "filters": q.filters,
        "totals": totals.rounded(),
        "groups": groups,
        "note": "costUsd sums known costs only; unknownCost counts started runs with no cost (Codex, unpriced models), which are never counted as $0. Values recorded before run ledger v2 read as unknown.",
    }))
}

/// How many finished chats of one kind are required before a usual time is shown.
pub(crate) const TYPICAL_MIN_RUNS: usize = 3;
/// A finished chat longer than this is left out (a stuck run, not a usual job).
pub(crate) const TYPICAL_MAX_MS: i64 = 24 * 60 * 60 * 1000;
/// The newest successful chats of the kind that the median is taken from.
pub(crate) const TYPICAL_SAMPLE: i64 = 200;

/// The usual length of a finished chat of `work_kind`, in milliseconds.
/// A successful `chat` row whose agent launched. A delegate, a workflow,
/// a run still going, and a row that never started are other surfaces or
/// other flags, so they are left out. A length that is not positive, or
/// longer than a day, is left out too. Fewer than [`TYPICAL_MIN_RUNS`]
/// samples, or a kind that is not one of the six, returns `None`.
pub fn typical_duration_ms(vault: &Path, work_kind: &str) -> Result<Option<u64>, String> {
    let Some(kind) = crate::run_attribution::normalize_work_kind(work_kind) else {
        return Ok(None);
    };
    let samples = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT started_at, at FROM usage_ledger \
                 WHERE work_kind = ?1 AND outcome = 'success' AND spawned = 1 AND surface = 'chat' \
                 ORDER BY at DESC, id DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![kind, TYPICAL_SAMPLE], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })?;
    let durations = samples
        .into_iter()
        .filter_map(|(started, finished)| run_length_ms(&started, &finished))
        .collect();
    Ok(median_ms(durations))
}

/// What the open chat should say about its kind of job. `work_kind` is null
/// when the folder has none. `typical_ms` is null until enough chats of
/// that kind have finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobTiming {
    pub work_kind: Option<String>,
    pub typical_ms: Option<u64>,
}

/// The kind of work `cwd` is, and how long a finished chat of that kind
/// usually takes. A folder that is not a workspace has neither.
pub fn job_timing_for(vault: &Path, cwd: &Path) -> Result<JobTiming, String> {
    let work_kind = crate::run_attribution::for_cwd(vault, cwd).work_kind;
    let typical_ms = match work_kind.as_deref() {
        Some(kind) => typical_duration_ms(vault, kind)?,
        None => None,
    };
    Ok(JobTiming {
        work_kind,
        typical_ms,
    })
}

fn run_length_ms(started: &str, finished: &str) -> Option<i64> {
    let start = DateTime::parse_from_rfc3339(started).ok()?;
    let end = DateTime::parse_from_rfc3339(finished).ok()?;
    Some(end.signed_duration_since(start).num_milliseconds())
}

/// The middle length, after dropping a length that is not a finished chat
/// a person waited through. An even count averages the two middle lengths.
/// Both are at most [`TYPICAL_MAX_MS`], so the sum fits in `i64`.
pub(crate) fn median_ms(mut durations: Vec<i64>) -> Option<u64> {
    durations.retain(|&ms| ms > 0 && ms <= TYPICAL_MAX_MS);
    if durations.len() < TYPICAL_MIN_RUNS {
        return None;
    }
    durations.sort_unstable();
    let mid = durations.len() / 2;
    let median = if durations.len() % 2 == 1 {
        durations[mid]
    } else {
        (durations[mid - 1] + durations[mid]) / 2
    };
    Some(median as u64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::run_attribution::Attribution;
    use crate::session_store::tests::usage_row_fixture;
    use crate::session_store::upsert_usage;

    pub(crate) fn row(vault: &Path, key: &str, run: &str, outcome: &str, cost: Option<f64>) {
        let mut r = usage_row_fixture(key, &session_store::db_time(Utc::now()), outcome);
        r.run_id = Some(run.into());
        r.cost_usd = cost;
        r.spawned = true;
        upsert_usage(vault, &r).unwrap();
    }

    pub(crate) fn attribute(vault: &Path, key: &str, dept: &str, kind: &str, account: &str) {
        session_store::set_usage_attribution(
            vault,
            key,
            &Attribution {
                account_ref: Some(account.into()),
                workspace_id: Some("w".into()),
                department: Some(dept.into()),
                work_kind: Some(kind.into()),
            },
        )
        .unwrap();
    }

    fn timed(
        vault: &Path,
        key: &str,
        kind: &str,
        start: &str,
        end: &str,
        outcome: &str,
        surface: &str,
        spawned: bool,
    ) {
        let mut r = usage_row_fixture(key, end, outcome);
        r.started_at = start.into();
        r.surface = surface.into();
        r.spawned = spawned;
        upsert_usage(vault, &r).unwrap();
        attribute(vault, key, "AI", kind, "acct-1");
    }

    #[test]
    fn the_usual_length_is_the_middle_of_finished_chats_of_that_kind() {
        assert_eq!(median_ms(vec![]), None);
        assert_eq!(median_ms(vec![60_000, 120_000]), None);
        assert_eq!(
            median_ms(vec![0, 60_000, 120_000]),
            None,
            "a zero length does not count"
        );
        assert_eq!(
            median_ms(vec![60_000, 120_000, TYPICAL_MAX_MS + 1]),
            None,
            "a run longer than a day does not count"
        );
        assert_eq!(median_ms(vec![180_000, 60_000, 120_000]), Some(120_000));
        assert_eq!(
            median_ms(vec![60_000, 120_000, 180_000, 240_000]),
            Some(150_000)
        );

        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let start = "2026-11-08T10:00:00.000Z";
        timed(
            vault,
            "a",
            "code",
            start,
            "2026-11-08T10:01:00.000Z",
            "success",
            "chat",
            true,
        );
        timed(
            vault,
            "b",
            "code",
            start,
            "2026-11-08T10:03:00.000Z",
            "success",
            "chat",
            true,
        );
        timed(
            vault,
            "c",
            "code",
            start,
            "2026-11-08T10:02:00.000Z",
            "success",
            "chat",
            true,
        );
        // These must not move the middle. `spawned` is true once the agent
        // process has launched, which a real chat does.
        timed(
            vault,
            "other",
            "research",
            start,
            "2026-11-08T10:09:00.000Z",
            "success",
            "chat",
            true,
        );
        timed(
            vault,
            "err",
            "code",
            start,
            "2026-11-08T10:00:01.000Z",
            "error",
            "chat",
            true,
        );
        timed(
            vault,
            "live",
            "code",
            start,
            "2026-11-08T10:00:01.000Z",
            "running",
            "chat",
            true,
        );
        timed(
            vault,
            "child",
            "code",
            start,
            "2026-11-08T10:00:01.000Z",
            "success",
            "delegate",
            true,
        );
        timed(
            vault,
            "flow",
            "code",
            start,
            "2026-11-08T10:00:01.000Z",
            "success",
            "workflow",
            true,
        );
        timed(vault, "zero", "code", start, start, "success", "chat", true);
        timed(
            vault,
            "stuck",
            "code",
            "2026-11-07T09:00:00.000Z",
            "2026-11-08T10:00:00.000Z",
            "success",
            "chat",
            true,
        );
        timed(
            vault,
            "unlaunched",
            "code",
            start,
            "2026-11-08T10:10:00.000Z",
            "success",
            "chat",
            false,
        );
        assert_eq!(typical_duration_ms(vault, "code").unwrap(), Some(120_000));
        assert_eq!(typical_duration_ms(vault, "Code").unwrap(), Some(120_000));
        assert_eq!(
            typical_duration_ms(vault, "research").unwrap(),
            None,
            "one research chat is not enough"
        );
        assert_eq!(typical_duration_ms(vault, "coding").unwrap(), None);
        assert_eq!(typical_duration_ms(vault, "unknown").unwrap(), None);

        let ws = vault.join("workspaces/payroll");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join("WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: \"payroll\"\nstatus: active\nwork_kind: code\n---\n\n# payroll\n",
        )
        .unwrap();
        assert_eq!(
            job_timing_for(vault, &ws).unwrap(),
            JobTiming {
                work_kind: Some("code".into()),
                typical_ms: Some(120_000),
            }
        );
        assert_eq!(
            job_timing_for(vault, vault).unwrap(),
            JobTiming {
                work_kind: None,
                typical_ms: None,
            }
        );
    }

    #[test]
    fn labels_follow_results_votes_and_how_the_run_ended() {
        assert_eq!(
            label_for(Some("accepted"), Some("notUseful"), "success"),
            Some("accepted")
        );
        assert_eq!(label_for(Some("revised"), None, "success"), Some("revised"));
        assert_eq!(
            label_for(None, Some("notUseful"), "success"),
            Some("abandoned")
        );
        assert_eq!(label_for(None, None, "error"), Some("abandoned"));
        assert_eq!(label_for(None, Some("useful"), "success"), Some(UNKNOWN));
        assert_eq!(label_for(None, None, "running"), None);
    }

    /// The F2 migration test: a store written before v2 (no attribution
    /// columns) opens, gains them, and its rows read as unknown.
    #[test]
    fn rows_from_before_v2_read_as_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let db = session_store::db_path_for(vault).unwrap();
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage_ledger(id INTEGER PRIMARY KEY AUTOINCREMENT, \
                 run_key TEXT NOT NULL UNIQUE, at TEXT NOT NULL, started_at TEXT NOT NULL, \
                 surface TEXT NOT NULL, scope TEXT, parent_surface TEXT, run_id TEXT, \
                 agent TEXT, requested_model TEXT, model TEXT, native_session TEXT, \
                 billing TEXT NOT NULL, outcome TEXT NOT NULL, spawned INTEGER NOT NULL, \
                 basis TEXT NOT NULL, input_tokens INTEGER, cached_input_tokens INTEGER, \
                 output_tokens INTEGER, cost_usd REAL, cost_status TEXT NOT NULL, \
                 usage_json TEXT); \
                 INSERT INTO usage_ledger(run_key, at, started_at, surface, agent, model, \
                 billing, outcome, spawned, basis, cost_usd, cost_status) VALUES('old', \
                 '2026-10-01T12:00:00.000Z', '2026-10-01T12:00:00.000Z', 'chat', 'claude', \
                 'claude-opus-5-5', 'subscription', 'success', 1, 'per_run', 1.25, 'estimated');",
            )
            .unwrap();
        }
        let rows = rows_between(
            vault,
            "2026-10-01T00:00:00.000Z",
            "2026-10-02T00:00:00.000Z",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(
            (
                &*r.account,
                &*r.workspace,
                &*r.department,
                &*r.work_kind,
                &*r.outcome_label
            ),
            (UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN, UNKNOWN)
        );
        assert_eq!(r.cost_usd, Some(1.25));
        // A refresh labels it from how it ended (no Result, no vote).
        refresh_outcome_labels_with(vault, &HashMap::new()).unwrap();
        let rows = rows_between(
            vault,
            "2026-10-01T00:00:00.000Z",
            "2026-10-02T00:00:00.000Z",
        )
        .unwrap();
        assert_eq!(rows[0].outcome_label, UNKNOWN);
    }

    #[test]
    fn the_query_groups_and_keeps_unknown_costs_apart() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        row(vault, "a", "r1", "success", Some(1.105));
        attribute(vault, "a", "Finance", "data", "acct-1");
        row(vault, "b", "r2", "success", Some(2.0));
        attribute(vault, "b", "Finance", "data", "acct-2");
        row(vault, "c", "r3", "success", None); // Codex: unpriced.
        attribute(vault, "c", "HR", "admin", "login");
        row(vault, "d", "r4", "error", Some(0.5));
        let mut results = HashMap::new();
        results.insert("r1".to_string(), "accepted");
        refresh_outcome_labels_with(vault, &results).unwrap();
        let q = Query {
            group_by: vec!["department".into()],
            ..Query::default()
        };
        let out = query_with(vault, &q, Utc::now(), &results).unwrap();
        assert_eq!(out["totals"]["runs"], 4);
        assert_eq!(out["totals"]["costUsd"], 3.61);
        assert_eq!(out["totals"]["unknownCost"], 1);
        let groups = out["groups"].as_array().unwrap();
        let finance = groups
            .iter()
            .find(|g| g["key"]["department"] == "Finance")
            .unwrap();
        assert_eq!(finance["totals"]["runs"], 2);
        assert_eq!(finance["totals"]["costUsd"], 3.11);
        let hr = groups
            .iter()
            .find(|g| g["key"]["department"] == "HR")
            .unwrap();
        assert_eq!(hr["totals"]["costUsd"], 0.0);
        assert_eq!(hr["totals"]["unknownCost"], 1, "unknown, not zero");
        assert!(groups.iter().any(|g| g["key"]["department"] == UNKNOWN));
        // Labels: r1 accepted, r4 abandoned (failed).
        let labels = rows_between(
            vault,
            "2000-01-01T00:00:00.000Z",
            "2100-01-01T00:00:00.000Z",
        )
        .unwrap();
        let label = |k: &str| {
            labels
                .iter()
                .find(|r| r.run_key == k)
                .unwrap()
                .outcome_label
                .clone()
        };
        assert_eq!(label("a"), "accepted");
        assert_eq!(label("b"), UNKNOWN);
        assert_eq!(label("d"), "abandoned");
        // Filters and unknown dimensions.
        let mut filtered = Query::default();
        filtered.filters.insert("account".into(), "acct-2".into());
        assert_eq!(
            query_with(vault, &filtered, Utc::now(), &results).unwrap()["totals"]["runs"],
            1
        );
        let bad = Query {
            group_by: vec!["colour".into()],
            ..Query::default()
        };
        assert!(query(vault, &bad, Utc::now()).is_err());
    }
}
