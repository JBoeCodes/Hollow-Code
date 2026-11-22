//! AI value ledger v1 (master plan #6, decision 4, 2026-10-08): what Hollow's
//! runs cost against the hours they saved, by department and work kind.
//!
//! **Hours saved are estimates, and count only accepted work.** A run saves
//! time only when its `outcome_label` is `accepted`: a person accepted a
//! Result it wrote, or (G4) the tracker issue it worked on reached Done after
//! it started (`run_ledger`, `tracker::work::accepted_by_issue`), counted
//! once either way. Until some run in the month is accepted, hours saved are
//! hidden (`hoursSaved.shown` false; Settings, the weekly update and the
//! report leave the figure out). An accepted run saves the person's baseline
//! minutes for its work kind, or the minutes they set for that one run. No
//! baseline, no saving: nothing is guessed. Revised, abandoned and unknown
//! runs count as cost with zero saved. Every saved figure carries its basis
//! (`baseline:<kind>` or `override`) and is labelled an estimate; nothing
//! here is measured.
//!
//! **Cost ties to the ledger.** A run's cost is the sum of its ledger rows
//! (helpers and teammates carry its run id); a row without a run id (a Jev
//! request, a detached one-shot) is its own entry. Runs with no cost
//! (Codex, unpriced models, Jev) are counted as unknown, never $0.
//!
//! **Baselines are the person's.** `value-baselines.json` beside
//! `agent-settings.json` (app data, outside the vault), written only by
//! `set_baseline` or the COO's `value.set_baseline` (L13 COO parity, a confirm
//! card showing the old and new minutes, asked in every auto chat).
//! Schema v1: `{schemaVersion: 1, baselines: {<work kind>: {minutes, note?,
//! setAt}}, runOverrides: {<run id>: {minutes, setAt}}}`.
//!
//! **The report stays on this Mac.** `monthly_report` drafts
//! `outputs/value-report-<YYYY-MM>.md` in a workspace as a draft Result,
//! generated from the ledger alone (no model); an agent may add the
//! narrative. Publishing it anywhere is a separate External card.

use crate::run_attribution::{UNKNOWN, WORK_KINDS};
use crate::run_ledger::{cents, LedgerRow};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const FILE: &str = "value-baselines.json";
const SCHEMA: u32 = 1;
/// Ten working days: a larger baseline for one run is a typo.
const MINUTES_MAX: f64 = 4_800.0;
/// The basis of an accepted run whose work kind has no baseline.
const NO_BASELINE: &str = "no baseline";
/// The basis of an accepted run whose work kind is `unknown` (no baseline
/// can be set for it).
const UNKNOWN_KIND: &str = "work kind unknown";
/// Per-run lines a summary returns at most (the totals cover every run).
const RUN_LINES_MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub minutes: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub set_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOverride {
    pub minutes: f64,
    pub set_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Baselines {
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub baselines: BTreeMap<String, Baseline>,
    #[serde(default)]
    pub run_overrides: BTreeMap<String, RunOverride>,
}

fn store_path() -> Result<PathBuf, String> {
    crate::agent_settings::app_data_dir().map(|dir| dir.join(FILE))
}

pub fn load_at(path: &Path) -> Result<Baselines, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Baselines {
                schema_version: SCHEMA,
                ..Baselines::default()
            })
        }
        Err(e) => return Err(format!("{FILE} unreadable: {e}")),
    };
    let stored: Baselines =
        serde_json::from_str(&text).map_err(|e| format!("{FILE} needs repair: {e}"))?;
    if stored.schema_version != SCHEMA {
        return Err(format!(
            "Unsupported {FILE} version {}",
            stored.schema_version
        ));
    }
    Ok(stored)
}

/// What [`set_at`] changes: one work kind's baseline, or one run's own
/// minutes. `minutes: None` removes it.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineInput {
    pub work_kind: Option<String>,
    pub run_id: Option<String>,
    pub minutes: Option<f64>,
    pub note: Option<String>,
}

pub fn set_at(path: &Path, input: &BaselineInput, now: DateTime<Utc>) -> Result<Baselines, String> {
    if let Some(minutes) = input.minutes {
        if !minutes.is_finite() || !(0.0..=MINUTES_MAX).contains(&minutes) {
            return Err(format!("Minutes must be between 0 and {MINUTES_MAX}."));
        }
    }
    let mut stored = load_at(path)?;
    let set_at = now.to_rfc3339();
    match (input.work_kind.as_deref(), input.run_id.as_deref()) {
        (Some(kind), None) => {
            let kind = crate::run_attribution::normalize_work_kind(kind).ok_or_else(|| {
                format!(
                    "Unknown work kind '{kind}'. Use one of: {}.",
                    WORK_KINDS.join(", ")
                )
            })?;
            match input.minutes {
                Some(minutes) => {
                    let note = input
                        .note
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .map(|n| n.chars().take(200).collect());
                    stored.baselines.insert(
                        kind.to_string(),
                        Baseline {
                            minutes,
                            note,
                            set_at,
                        },
                    );
                }
                None => {
                    stored.baselines.remove(kind);
                }
            }
        }
        (None, Some(run_id)) => {
            let run_id = run_id.trim();
            if run_id.is_empty() || run_id.len() > 120 {
                return Err("Give the run id from the value summary.".into());
            }
            match input.minutes {
                Some(minutes) => {
                    stored
                        .run_overrides
                        .insert(run_id.to_string(), RunOverride { minutes, set_at });
                }
                None => {
                    stored.run_overrides.remove(run_id);
                }
            }
        }
        _ => return Err("Give either a work kind or a run id, not both.".into()),
    }
    stored.schema_version = SCHEMA;
    let value = serde_json::to_value(&stored).map_err(|e| e.to_string())?;
    let map: BTreeMap<String, Value> = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    crate::setup::publish_config(path, &map)?;
    Ok(stored)
}

/// The local month `YYYY-MM` as ledger bounds `[from, to)`.
pub fn month_bounds(month: &str) -> Result<(String, String), String> {
    let first = NaiveDate::parse_from_str(&format!("{}-01", month.trim()), "%Y-%m-%d")
        .map_err(|_| format!("'{month}' is not a month (YYYY-MM)."))?;
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    }
    .ok_or("month out of range")?;
    let at = |d: NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).expect("midnight"))
            .earliest()
            .map(|t| crate::session_store::db_time(t.with_timezone(&Utc)))
            .ok_or_else(|| format!("{d} has no local midnight"))
    };
    Ok((at(first)?, at(next)?))
}

/// One run as the value ledger sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunValue {
    /// The history run id, or the row's key for a row with none.
    pub run: String,
    pub at: String,
    pub department: String,
    pub work_kind: String,
    pub outcome_label: String,
    /// Known cost of every row of the run (unrounded; totals round once).
    pub cost_usd: f64,
    /// Some row of the run started and has no cost.
    pub unknown_cost: bool,
    pub saved_minutes: f64,
    /// `override`, `baseline:<kind>`, `no baseline`, `work kind unknown` or
    /// `not accepted`.
    pub basis: String,
}

/// Fold ledger rows into runs, in order of first appearance.
pub fn runs(rows: &[LedgerRow], baselines: &Baselines) -> Vec<RunValue> {
    let mut order: Vec<String> = Vec::new();
    let mut by_run: BTreeMap<String, Vec<&LedgerRow>> = BTreeMap::new();
    for row in rows {
        let key = row.run_id.clone().unwrap_or_else(|| row.run_key.clone());
        if !by_run.contains_key(&key) {
            order.push(key.clone());
        }
        by_run.entry(key).or_default().push(row);
    }
    order
        .into_iter()
        .map(|run| {
            let rows = &by_run[&run];
            // The run's own row (not a teammate's) names its attribution.
            let main = rows
                .iter()
                .find(|r| r.surface != "delegate")
                .unwrap_or(&rows[0]);
            let cost_usd: f64 = rows.iter().filter_map(|r| r.cost_usd).sum();
            let unknown_cost = rows.iter().any(|r| r.spawned && r.cost_usd.is_none());
            let label = main.outcome_label.clone();
            let (saved_minutes, basis) = if label != "accepted" {
                (0.0, "not accepted".to_string())
            } else if let Some(o) = baselines.run_overrides.get(&run) {
                (o.minutes, "override".to_string())
            } else if let Some(b) = baselines.baselines.get(&main.work_kind) {
                (b.minutes, format!("baseline:{}", main.work_kind))
            } else if main.work_kind == UNKNOWN {
                (0.0, UNKNOWN_KIND.to_string())
            } else {
                (0.0, NO_BASELINE.to_string())
            };
            RunValue {
                run,
                at: main.at.clone(),
                department: main.department.clone(),
                work_kind: main.work_kind.clone(),
                outcome_label: label,
                cost_usd,
                unknown_cost,
                saved_minutes,
                basis,
            }
        })
        .collect()
}

/// Totals for a group of runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueTotals {
    pub runs: u64,
    pub accepted: u64,
    pub cost_usd: f64,
    pub unknown_cost_runs: u64,
    pub saved_minutes: f64,
    /// `saved_minutes / 60`, rounded to a tenth: an estimate.
    pub saved_hours_estimate: f64,
    /// Accepted runs whose work kind has no baseline (and no override): their
    /// saving is unknown, shown as "baseline not set", never as 0 h
    /// (overnight A12).
    pub unbaselined_accepted: u64,
    /// Accepted runs whose work kind is `unknown`: no baseline can be set
    /// for them, so they read "work kind unknown".
    pub unknown_kind_accepted: u64,
}

impl ValueTotals {
    fn add(&mut self, run: &RunValue) {
        self.runs += 1;
        self.accepted += u64::from(run.outcome_label == "accepted");
        self.cost_usd += run.cost_usd;
        self.unknown_cost_runs += u64::from(run.unknown_cost);
        self.saved_minutes += run.saved_minutes;
        self.unbaselined_accepted += u64::from(run.basis == NO_BASELINE);
        self.unknown_kind_accepted += u64::from(run.basis == UNKNOWN_KIND);
    }

    fn rounded(mut self) -> Self {
        self.cost_usd = cents(self.cost_usd);
        self.saved_hours_estimate = (self.saved_minutes / 6.0).round() / 10.0;
        self
    }
}

fn group_key(run: &RunValue, by: &str) -> String {
    match by {
        "department" => run.department.clone(),
        "workKind" => run.work_kind.clone(),
        "outcomeLabel" => run.outcome_label.clone(),
        _ => UNKNOWN.into(),
    }
}

pub const GROUPS: [&str; 3] = ["department", "workKind", "outcomeLabel"];

/// The month's value, grouped (default: department and work kind).
/// `results` are the Results' labels, `issue_done` the runs accepted through
/// their tracker issue (G4); a run in both counts once.
pub fn summary_with(
    vault: &Path,
    month: &str,
    group_by: &[String],
    baselines: &Baselines,
    results: &std::collections::HashMap<String, &'static str>,
    issue_done: &std::collections::BTreeSet<String>,
) -> Result<Value, String> {
    for by in group_by {
        if !GROUPS.contains(&by.as_str()) {
            return Err(format!("Group by one of: {}.", GROUPS.join(", ")));
        }
    }
    let group_by: Vec<String> = if group_by.is_empty() {
        vec!["department".into(), "workKind".into()]
    } else {
        group_by.to_vec()
    };
    let (from, to) = month_bounds(month)?;
    let labels = crate::run_ledger::with_issue_done(results.clone(), issue_done);
    crate::run_ledger::refresh_outcome_labels_with(vault, &labels)?;
    let rows = crate::run_ledger::rows_between(vault, &from, &to)?;
    let runs = runs(&rows, baselines);
    let headline = hours_saved(&runs, results, issue_done);
    let mut totals = ValueTotals::default();
    let mut groups: BTreeMap<Vec<String>, ValueTotals> = BTreeMap::new();
    for run in &runs {
        totals.add(run);
        let key = group_by.iter().map(|by| group_key(run, by)).collect();
        groups.entry(key).or_default().add(run);
    }
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|(key, t)| {
            let key: serde_json::Map<String, Value> = group_by
                .iter()
                .cloned()
                .zip(key.into_iter().map(Value::String))
                .collect();
            json!({ "key": key, "totals": t.rounded() })
        })
        .collect();
    let truncated = runs.len() > RUN_LINES_MAX;
    Ok(json!({
        "month": month.trim(),
        "from": from,
        "to": to,
        "groupBy": group_by,
        "totals": totals.rounded(),
        "groups": groups,
        "baselines": baselines.baselines,
        "runs": runs.iter().take(RUN_LINES_MAX).map(|r| json!({
            "run": r.run,
            "at": r.at,
            "department": r.department,
            "workKind": r.work_kind,
            "outcomeLabel": r.outcome_label,
            "costUsd": cents(r.cost_usd),
            "unknownCost": r.unknown_cost,
            "savedMinutes": r.saved_minutes,
            "basis": r.basis,
        })).collect::<Vec<_>>(),
        "runsTruncated": truncated,
        "hoursSaved": headline,
        "note": "Hours saved are estimates from the person's baselines, counted only for accepted runs (a Result the person accepted, or the run's tracker issue reached Done after it started; once each). They are hidden (hoursSaved.shown false) until a run is accepted. Revised, abandoned and unknown runs save nothing. costUsd sums known costs; unknownCostRuns are runs with no cost (Codex, unpriced models, Jev), never counted as $0.",
    }))
}

/// The basis every hours-saved figure states (G4: the existing estimate
/// model, applied to accepted runs only).
pub const HOURS_BASIS: &str =
    "your minutes-saved baseline per work kind (or a run's own minutes), for accepted runs only";

/// G4's headline: whether hours saved show at all (only once a run is
/// accepted), how many runs they come from, how each was accepted (a run
/// accepted both ways is in both counts, and once in `acceptedRuns`), and
/// the basis of the estimate.
pub fn hours_saved(
    runs: &[RunValue],
    results: &std::collections::HashMap<String, &'static str>,
    issue_done: &std::collections::BTreeSet<String>,
) -> Value {
    let accepted: Vec<&RunValue> = runs
        .iter()
        .filter(|r| r.outcome_label == "accepted")
        .collect();
    let minutes: f64 = accepted.iter().map(|r| r.saved_minutes).sum();
    let shown = !accepted.is_empty();
    json!({
        "shown": shown,
        "acceptedRuns": accepted.len(),
        "byIssueDone": accepted.iter().filter(|r| issue_done.contains(&r.run)).count(),
        "byAcceptedResult": accepted.iter().filter(|r| results.get(&r.run) == Some(&"accepted")).count(),
        "hoursEstimate": if shown { json!((minutes / 6.0).round() / 10.0) } else { Value::Null },
        "basis": HOURS_BASIS,
    })
}

/// The person's stored baselines (defaults when none are saved).
pub fn stored_baselines() -> Result<Baselines, String> {
    load_at(&store_path()?)
}

/// [`summary_with`] from the stored baselines, the vault's Results and the
/// tracker issues its runs finished.
pub fn summary_for(vault: &Path, month: &str, group_by: &[String]) -> Result<Value, String> {
    let baselines = stored_baselines()?;
    summary_with(
        vault,
        month,
        group_by,
        &baselines,
        &crate::run_ledger::result_labels(vault),
        &crate::tracker::work::accepted_by_issue(vault),
    )
}

fn money(usd: &Value) -> String {
    format!("${:.2}", usd.as_f64().unwrap_or(0.0))
}

/// The report's Markdown, from a [`summary_with`] value.
pub fn report_markdown(summary: &Value, generated: DateTime<Local>) -> String {
    let month = summary["month"].as_str().unwrap_or("");
    let t = &summary["totals"];
    let saved = |t: &Value| saved_cell(t);
    let mut out = format!(
        "# AI value report, {month}\n\n\
         > Draft for review, generated {} from Hollow's run ledger. Hours saved are **estimates** from your \
         baselines and count only accepted runs (a Result you accepted, or the run's tracker issue reached Done after it \
         started). Runs with no cost (Codex with no price entered, unpriced models, Jev) are \
         listed as unknown, not $0. This report has not left this Mac.\n\n\
         ## Totals\n\n\
         | Runs | Accepted | Cost (known) | Runs with unknown cost | Hours saved (estimate) |\n\
         |---:|---:|---:|---:|---:|\n\
         | {} | {} | {} | {} | {} |\n\n",
        generated.format("%Y-%m-%d %H:%M"),
        t["runs"],
        t["accepted"],
        money(&t["costUsd"]),
        t["unknownCostRuns"],
        saved(t),
    );
    out.push_str("## By department and work kind\n\n| Department | Work kind | Runs | Accepted | Cost (known) | Unknown cost | Hours saved (estimate) |\n|---|---|---:|---:|---:|---:|---:|\n");
    for g in summary["groups"].as_array().into_iter().flatten() {
        let k = &g["key"];
        let t = &g["totals"];
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            k["department"].as_str().unwrap_or("—"),
            k["workKind"].as_str().unwrap_or("—"),
            t["runs"],
            t["accepted"],
            money(&t["costUsd"]),
            t["unknownCostRuns"],
            saved(t),
        ));
    }
    out.push_str("\n## Baselines used\n\n");
    match summary["baselines"].as_object().filter(|b| !b.is_empty()) {
        Some(baselines) => {
            out.push_str("| Work kind | Minutes per accepted run | Note |\n|---|---:|---|\n");
            for (kind, b) in baselines {
                out.push_str(&format!(
                    "| {kind} | {} | {} |\n",
                    b["minutes"],
                    b["note"].as_str().unwrap_or("")
                ));
            }
        }
        None => out.push_str("No baselines are set, so no hours are counted as saved. Set them in Settings → Usage → Value this month.\n"),
    }
    out.push_str("\n## Runs\n\nEvery figure above is the sum of these lines (cost to the cent).\n\n| Run | When | Department | Work kind | Outcome | Cost | Saved (min) | Basis |\n|---|---|---|---|---|---:|---:|---|\n");
    for r in summary["runs"].as_array().into_iter().flatten() {
        let cost = if r["unknownCost"] == true && r["costUsd"].as_f64() == Some(0.0) {
            "unknown".to_string()
        } else if r["unknownCost"] == true {
            format!("{} + unknown", money(&r["costUsd"]))
        } else {
            money(&r["costUsd"])
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} |\n",
            r["run"].as_str().unwrap_or(""),
            r["at"].as_str().unwrap_or("").get(..10).unwrap_or(""),
            r["department"].as_str().unwrap_or(""),
            r["workKind"].as_str().unwrap_or(""),
            r["outcomeLabel"].as_str().unwrap_or(""),
            cost,
            if r["basis"] == NO_BASELINE || r["basis"] == UNKNOWN_KIND {
                "not set".to_string()
            } else {
                r["savedMinutes"].to_string()
            },
            r["basis"].as_str().unwrap_or(""),
        ));
    }
    if summary["runsTruncated"] == true {
        out.push_str("\nOnly the first 500 runs are listed; the totals cover every run.\n");
    }
    out.push_str("\n## Narrative\n\n_Not written yet. An agent may add a short narrative here from the figures above; it must not change them._\n");
    out
}

/// A totals row's "Hours saved (estimate)" cell: "no accepted runs" when
/// none is accepted (G4: hidden, never "0.0"), "baseline not set" when every
/// accepted run lacks one, else the hours, with the runs still missing one
/// said apart.
pub(crate) fn saved_cell(t: &Value) -> String {
    let hours = t["savedHoursEstimate"].as_f64().unwrap_or(0.0);
    let accepted = t["accepted"].as_u64().unwrap_or(0);
    let unset = t["unbaselinedAccepted"].as_u64().unwrap_or(0);
    let unknown = t["unknownKindAccepted"].as_u64().unwrap_or(0);
    if accepted == 0 {
        return "no accepted runs".to_string();
    }
    if accepted > 0 && unknown == accepted {
        return "work kind unknown".to_string();
    }
    if accepted > 0 && unset + unknown == accepted {
        return "baseline not set".to_string();
    }
    let mut extra = Vec::new();
    if unset > 0 {
        extra.push(format!("+{unset} without a baseline"));
    }
    if unknown > 0 {
        extra.push(format!("+{unknown} of unknown kind"));
    }
    if extra.is_empty() {
        format!("{hours:.1}")
    } else {
        format!("{hours:.1} ({})", extra.join(", "))
    }
}

/// Draft the month's report as a Result in `workspace` (never overwrites).
pub fn monthly_report_with(
    vault: &Path,
    workspace: &str,
    summary: &Value,
    now: DateTime<Local>,
) -> Result<Value, String> {
    crate::workspaces::summary(vault, workspace)?;
    let month = summary["month"].as_str().unwrap_or_default();
    let rel = format!("workspaces/{workspace}/outputs/value-report-{month}.md");
    if vault.join(&rel).exists() {
        return Err(format!(
            "{rel} already exists. Rename or remove it first; a report is never overwritten."
        ));
    }
    let text = report_markdown(summary, now);
    crate::vault::create_note(vault, &rel, &text)?;
    Ok(json!({ "path": rel, "totals": summary["totals"], "review": "draft" }))
}

// ── Commands ───────────────────────────────────────────────────────────────

fn vault_of(state: &tauri::State<'_, crate::commands::AppState>) -> PathBuf {
    state.config.vault_path.clone()
}

/// `value.summary`: the month's cost and estimated hours saved.
#[tauri::command]
pub async fn summary(
    month: Option<String>,
    group_by: Option<Vec<String>>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Value, String> {
    let vault = vault_of(&state);
    let month = month.unwrap_or_else(|| Local::now().format("%Y-%m").to_string());
    tauri::async_runtime::spawn_blocking(move || {
        summary_for(&vault, &month, &group_by.unwrap_or_default())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `value.monthly_report`: draft the month's report (default: last month).
#[tauri::command]
pub async fn monthly_report(
    workspace: String,
    month: Option<String>,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Value, String> {
    let vault = vault_of(&state);
    let month = month.unwrap_or_else(|| {
        let now = Local::now();
        let first = now.date_naive().with_day(1).expect("day 1");
        (first - chrono::Duration::days(1))
            .format("%Y-%m")
            .to_string()
    });
    tauri::async_runtime::spawn_blocking(move || {
        let summary = summary_for(&vault, &month, &[])?;
        monthly_report_with(&vault, &workspace, &summary, Local::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The person sets a work kind's baseline or one run's minutes. The COO's
/// `value.set_baseline` does the same, only on its Confirm card, which auto
/// mode always shows, so no agent raises its own savings unasked (COO parity L13).
#[tauri::command]
pub async fn set_baseline(input: BaselineInput) -> Result<Baselines, String> {
    tauri::async_runtime::spawn_blocking(move || set_now(&input))
        .await
        .map_err(|e| e.to_string())?
}

/// [`set_at`] on the app's own store, now (what the command and the COO's
/// `value.set_baseline` both run).
pub(crate) fn set_now(input: &BaselineInput) -> Result<Baselines, String> {
    set_at(&store_path()?, input, Utc::now())
}

/// The minutes `input` would replace (a work kind's baseline or one run's
/// override), for the confirm card's old to new line; `None` when none is
/// set yet.
pub(crate) fn minutes_now(input: &BaselineInput) -> Result<Option<f64>, String> {
    let stored = load_at(&store_path()?)?;
    Ok(
        match (input.work_kind.as_deref(), input.run_id.as_deref()) {
            (Some(kind), None) => crate::run_attribution::normalize_work_kind(kind)
                .and_then(|kind| stored.baselines.get(kind))
                .map(|b| b.minutes),
            (None, Some(run)) => stored.run_overrides.get(run.trim()).map(|o| o.minutes),
            _ => None,
        },
    )
}

/// The stored baselines, for Settings.
#[tauri::command]
pub fn get_baselines() -> Result<Baselines, String> {
    load_at(&store_path()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_ledger::tests::{attribute, row};
    use std::collections::HashMap;

    fn baselines() -> Baselines {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let now = Utc::now();
        set_at(
            &path,
            &BaselineInput {
                work_kind: Some("data".into()),
                minutes: Some(90.0),
                note: Some("reconciliation by hand".into()),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        set_at(
            &path,
            &BaselineInput {
                run_id: Some("r5".into()),
                minutes: Some(30.0),
                ..Default::default()
            },
            now,
        )
        .unwrap()
    }

    /// A fixture month: totals tie to the ledger to the cent, only accepted
    /// runs save time, unknown costs stay unknown, and every figure in the
    /// report is traceable to a run line.
    #[test]
    fn a_month_ties_to_the_ledger_and_counts_only_accepted_work() {
        // Rows are stamped now and the month read after: one local day, so
        // one month (overnight D2).
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        // r1: accepted data run with a teammate row (cost 1.105 + 0.20).
        row(vault, "a", "r1", "success", Some(1.105));
        attribute(vault, "a", "Finance", "data", "acct-1");
        row(vault, "a2", "r1", "success", Some(0.2));
        // r2: revised data run: cost, no saving.
        row(vault, "b", "r2", "success", Some(2.0));
        attribute(vault, "b", "Finance", "data", "acct-1");
        // r3: Codex (no cost), accepted, admin with no baseline.
        row(vault, "c", "r3", "success", None);
        attribute(vault, "c", "HR", "admin", "login");
        // r4: failed: abandoned.
        row(vault, "d", "r4", "error", Some(0.333));
        // r5: accepted, its own override.
        row(vault, "e", "r5", "success", Some(0.01));
        attribute(vault, "e", "AI", "code", "acct-2");
        let mut results: HashMap<String, &'static str> = HashMap::new();
        results.insert("r1".into(), "accepted");
        results.insert("r2".into(), "revised");
        results.insert("r3".into(), "accepted");
        results.insert("r5".into(), "accepted");
        let month = Local::now().format("%Y-%m").to_string();
        let b = baselines();
        let out = summary_with(vault, &month, &[], &b, &results, &Default::default()).unwrap();
        let t = &out["totals"];
        assert_eq!(t["runs"], 5);
        assert_eq!(t["accepted"], 3);
        // 1.105 + 0.2 + 2.0 + 0.333 + 0.01 = 3.648 → 3.65, as the ledger says.
        assert_eq!(t["costUsd"], 3.65);
        let ledger = crate::run_ledger::query_with(
            vault,
            &crate::run_ledger::Query::default(),
            Utc::now(),
            &results,
        )
        .unwrap();
        assert_eq!(
            t["costUsd"], ledger["totals"]["costUsd"],
            "ties to the ledger"
        );
        assert_eq!(t["unknownCostRuns"], 1, "Codex is unknown, not $0");
        // r1 saves 90 (data baseline), r5 its 30 override; r3 has no
        // baseline; r2 revised and r4 abandoned save nothing.
        assert_eq!(t["savedMinutes"], 120.0);
        assert_eq!(t["savedHoursEstimate"], 2.0);
        let line = |run: &str| {
            out["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["run"] == run)
                .unwrap()
                .clone()
        };
        assert_eq!(line("r1")["basis"], "baseline:data");
        assert_eq!(line("r1")["costUsd"], 1.31);
        assert_eq!(line("r2")["basis"], "not accepted");
        assert_eq!(line("r3")["basis"], "no baseline");
        assert_eq!(line("r4")["outcomeLabel"], "abandoned");
        assert_eq!(line("r5")["basis"], "override");
        let finance = out["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["key"]["department"] == "Finance")
            .unwrap();
        assert_eq!(finance["totals"]["costUsd"], 3.31);
        assert_eq!(finance["totals"]["savedMinutes"], 90.0);

        // The report: every run is a line, the totals are the summary's,
        // estimates are labelled, and it never overwrites.
        std::fs::create_dir_all(vault.join("workspaces/main")).unwrap();
        std::fs::write(
            vault.join("workspaces/main/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Main agent\n---\n",
        )
        .unwrap();
        let drafted = monthly_report_with(vault, "main", &out, Local::now()).unwrap();
        let path = vault.join(drafted["path"].as_str().unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "`r1`",
            "`r2`",
            "`r3`",
            "`r4`",
            "`r5`",
            "$3.65",
            "estimate",
            "unknown",
            "| 2.0 (+1 without a baseline) |",
            "| baseline not set |",
            "| not set | no baseline |",
        ] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
        assert!(monthly_report_with(vault, "main", &out, Local::now()).is_err());
    }

    /// G4: a run is accepted when its tracker issue reached Done after it
    /// started or its Result was accepted, counted once; Done before the
    /// run or reopened counts nothing; with no accepted run, hours saved
    /// are hidden.
    #[test]
    fn issue_done_or_an_accepted_result_counts_each_run_once() {
        crate::test_clock::clear_of_midnight(chrono::Duration::seconds(10));
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let path = dir.path().join(FILE);
        let b = set_at(
            &path,
            &BaselineInput {
                work_kind: Some("data".into()),
                minutes: Some(90.0),
                ..Default::default()
            },
            Utc::now(),
        )
        .unwrap();
        let team = crate::tracker::linear::TEAM_ID;
        let started = Utc::now() - chrono::Duration::hours(2);
        let iso = crate::session_store::db_time;
        // r1 issue Done after; r2 Done before; r3 reopened; r4 Result
        // accepted, no issue; r5 both; r6 neither.
        let runs = [
            ("a", "r1", Some("i-after")),
            ("b", "r2", Some("i-before")),
            ("c", "r3", Some("i-reopened")),
            ("d", "r4", None),
            ("e", "r5", Some("i-both")),
            ("f", "r6", None),
        ];
        for (n, (key, run, issue)) in runs.iter().enumerate() {
            row(vault, key, run, "success", Some(0.1));
            attribute(vault, key, "AI", "data", "acct");
            let session = format!("s-{run}");
            if let Some(issue) = issue {
                crate::tracker::work::insert_link(
                    vault,
                    &crate::tracker::work::WorkLink {
                        session_key: session.clone(),
                        issue_id: Some((*issue).into()),
                        issue_key: Some("ENG-1".into()),
                        workspace_id: None,
                        linked_by: "matched".into(),
                        created_issue: false,
                        at: iso(started),
                        depth: 1,
                        tracked: false,
                        outcome_at: None,
                        title: None,
                    },
                )
                .unwrap();
            }
            crate::session_store::with_write(vault, "test turn", |tx| {
                tx.execute(
                    "INSERT INTO turns(session_key, ord, run_id, started_at, status) VALUES(?1, ?2, ?3, ?4, 'done')",
                    rusqlite::params![session, n as i64 + 1, run, iso(started)],
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            })
            .unwrap();
        }
        let after = iso(started + chrono::Duration::hours(1));
        let before = iso(started - chrono::Duration::hours(1));
        let state = crate::tracker::work::tests::ledger_with(
            team,
            &[
                ("i-after", "completed", Some(after.as_str())),
                ("i-before", "completed", Some(before.as_str())),
                ("i-reopened", "started", Some(after.as_str())),
                ("i-both", "completed", Some(after.as_str())),
            ],
        );
        std::fs::write(
            vault.join(crate::tracker_sync::STATE),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        let issue_done = crate::tracker::work::accepted_by_issue(vault);
        assert_eq!(
            issue_done.iter().cloned().collect::<Vec<_>>(),
            vec!["r1".to_string(), "r5".to_string()]
        );
        let mut results: HashMap<String, &'static str> = HashMap::new();
        results.insert("r4".into(), "accepted");
        results.insert("r5".into(), "accepted");
        let month = Local::now().format("%Y-%m").to_string();
        let out = summary_with(vault, &month, &[], &b, &results, &issue_done).unwrap();
        assert_eq!(out["totals"]["runs"], 6);
        assert_eq!(out["totals"]["accepted"], 3, "r1, r4, r5 once each");
        assert_eq!(out["totals"]["savedMinutes"], 270.0);
        let h = &out["hoursSaved"];
        assert_eq!(h["shown"], true);
        assert_eq!(h["acceptedRuns"], 3);
        assert_eq!(h["byIssueDone"], 2);
        assert_eq!(h["byAcceptedResult"], 2);
        assert_eq!(h["hoursEstimate"], 4.5);
        assert_eq!(h["basis"], HOURS_BASIS);
        // The acceptance labels the ledger query and the digest read agree.
        let labels = crate::run_ledger::with_issue_done(results.clone(), &issue_done);
        assert_eq!(labels.values().filter(|l| **l == "accepted").count(), 3);
        // Nothing accepted: hidden, no hours.
        let none =
            summary_with(vault, &month, &[], &b, &HashMap::new(), &Default::default()).unwrap();
        assert_eq!(none["totals"]["accepted"], 0);
        assert_eq!(none["hoursSaved"]["shown"], false);
        assert!(none["hoursSaved"]["hoursEstimate"].is_null());
        assert_eq!(saved_cell(&none["totals"]), "no accepted runs");
    }

    #[test]
    fn baselines_validate_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let now = Utc::now();
        let bad = |input: BaselineInput| set_at(&path, &input, now).unwrap_err();
        bad(BaselineInput {
            work_kind: Some("coding".into()),
            minutes: Some(10.0),
            ..Default::default()
        });
        bad(BaselineInput {
            work_kind: Some("code".into()),
            minutes: Some(f64::NAN),
            ..Default::default()
        });
        bad(BaselineInput {
            work_kind: Some("code".into()),
            run_id: Some("r1".into()),
            minutes: Some(10.0),
            ..Default::default()
        });
        bad(BaselineInput::default());
        let saved = set_at(
            &path,
            &BaselineInput {
                work_kind: Some("Code".into()),
                minutes: Some(45.0),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        assert_eq!(saved.baselines["code"].minutes, 45.0);
        let removed = set_at(
            &path,
            &BaselineInput {
                work_kind: Some("code".into()),
                ..Default::default()
            },
            now,
        )
        .unwrap();
        assert!(removed.baselines.is_empty());
        assert_eq!(load_at(&path).unwrap(), removed);
    }

    #[test]
    fn months_are_local_and_validated() {
        let (from, to) = month_bounds("2026-12").unwrap();
        assert!(from < to);
        assert!(month_bounds("2026-13").is_err());
        assert!(month_bounds("Oct").is_err());
    }

    /// Overnight A12: accepted work with no baseline reads "baseline not
    /// set", never 0 h; partly set says how many still lack one.
    #[test]
    fn work_with_no_baseline_reads_not_set_never_zero_hours() {
        let t = |accepted: u64, unset: u64, hours: f64| json!({ "accepted": accepted, "unbaselinedAccepted": unset, "savedHoursEstimate": hours });
        let u = |accepted: u64, unset: u64, unknown: u64, hours: f64| json!({ "accepted": accepted, "unbaselinedAccepted": unset, "unknownKindAccepted": unknown, "savedHoursEstimate": hours });
        assert_eq!(
            saved_cell(&u(2, 0, 2, 0.0)),
            "work kind unknown",
            "no baseline can be set for it"
        );
        assert_eq!(saved_cell(&u(3, 1, 2, 0.0)), "baseline not set");
        assert_eq!(
            saved_cell(&u(4, 1, 1, 1.0)),
            "1.0 (+1 without a baseline, +1 of unknown kind)"
        );
        assert_eq!(saved_cell(&t(3, 3, 0.0)), "baseline not set");
        assert_eq!(saved_cell(&t(3, 1, 1.5)), "1.5 (+1 without a baseline)");
        assert_eq!(saved_cell(&t(3, 0, 2.0)), "2.0");
        assert_eq!(
            saved_cell(&t(0, 0, 0.0)),
            "no accepted runs",
            "hidden until a run is accepted (G4)"
        );
        let mut totals = ValueTotals::default();
        for basis in [NO_BASELINE, "baseline:code", "not accepted", UNKNOWN_KIND] {
            totals.add(&RunValue {
                run: "r".into(),
                at: String::new(),
                department: String::new(),
                work_kind: String::new(),
                outcome_label: if basis == "not accepted" {
                    "revised".into()
                } else {
                    "accepted".into()
                },
                cost_usd: 0.0,
                unknown_cost: false,
                saved_minutes: if basis == "baseline:code" { 30.0 } else { 0.0 },
                basis: basis.into(),
            });
        }
        assert_eq!(
            (
                totals.accepted,
                totals.unbaselined_accepted,
                totals.unknown_kind_accepted
            ),
            (3, 1, 1)
        );
    }
}
