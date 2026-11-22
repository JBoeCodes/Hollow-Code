//! The self-writing weekly update (master plan 2026-10, #4; see
//! `docs/SCHEDULER.md#the-weekly-update`).
//!
//! **Gather** ([`gather`], pure core [`gather_from`]): one week's facts from
//! records Hollow already keeps, never from a model: each workspace's
//! Activity (`workspace_activity`: conversations, runs and their cost,
//! results and their reviews, installs) and the tasks. Every line carries its
//! evidence (a vault path or a record id); a line with none, or whose date
//! only a file's modified time gives, is labelled *not verified*. Cost and
//! department come from the usage ledger's own columns (run ledger v2, F2:
//! the department a run's workspace had when it launched; "not available"
//! when no run has one), over every run that week, the Main agent's and
//! automations' included. Hours saved are the value ledger's (#6): accepted
//! runs only (an accepted Result, or the run's tracker issue Done after it
//! started; G4), from the person's baselines, always labelled an estimate,
//! left out entirely while no run that week is accepted, and "not
//! available" until a baseline is set.
//!
//! **Draft** ([`write_draft`]): the facts fill the person's template
//! (`workspaces/main/templates/weekly-update.md`, created with
//! [`DEFAULT_TEMPLATE`] when missing) into
//! `workspaces/main/outputs/weekly/<Friday>.md`. An existing draft is never
//! overwritten: the prose pass may have edited it.
//!
//! **Prose** ([`run_weekly_job`]): the `friday-pm` window job (15:00, decision
//! 2) drafts, notifies "Your weekly update is ready", then queues one Main
//! agent turn (the configured agent; unattended limits) that rewrites the
//! prose from those facts only. Nothing is sent from here.

use crate::types::Task;
use crate::workspace_activity::{ActivityEvent, WorkspaceActivity};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc, Weekday};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

pub(crate) const TITLE: &str = "Operations AI Requests – Weekly Update";
pub(crate) const TEMPLATE_REL: &str = "workspaces/main/templates/weekly-update.md";
const DRAFT_DIR: &str = "workspaces/main/outputs/weekly";
/// The Activity events read per workspace.
const EVENTS_PER_WORKSPACE: usize = 2_000;

/// The template written when the person has none. `{{…}}` are filled in.
pub(crate) const DEFAULT_TEMPLATE: &str = "# {{title}}

Week {{week}} ({{range}}). Drafted by Hollow from its own records on {{drafted}}. Every line links its evidence;
a line marked *not verified* has none. Review it before sharing.

## Shipped

{{shipped}}

## In progress

{{in_progress}}

## Blocked

{{blocked}}

## Numbers

{{numbers}}

## Next week

{{next_week}}
";

/// Where a fact comes from.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// What the evidence is (`conversation s-…`, a file name).
    pub label: String,
    /// Vault-relative, when the evidence is a file.
    pub path: Option<String>,
}

/// One line of the update.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestLine {
    pub text: String,
    pub evidence: Option<Evidence>,
    /// Why it is not verified, when it isn't (no evidence, or a date only a
    /// file's modified time gives).
    pub unverified: Option<String>,
}

impl DigestLine {
    fn with(text: impl Into<String>, label: impl Into<String>, path: Option<String>) -> Self {
        DigestLine {
            text: text.into(),
            evidence: Some(Evidence {
                label: label.into(),
                path,
            }),
            unverified: None,
        }
    }

    fn bare(text: impl Into<String>, why: impl Into<String>) -> Self {
        DigestLine {
            text: text.into(),
            evidence: None,
            unverified: Some(why.into()),
        }
    }
}

/// A week's facts, by section.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeekFacts {
    /// ISO week, `2026-W41`.
    pub week: String,
    pub start: String,
    pub end: String,
    pub shipped: Vec<DigestLine>,
    pub in_progress: Vec<DigestLine>,
    pub blocked: Vec<DigestLine>,
    pub numbers: Vec<DigestLine>,
    pub next_week: Vec<DigestLine>,
    /// Results accepted and tasks done (the weekly review's tiles).
    pub accepted: usize,
    pub tasks_done: usize,
}

/// One workspace's week, as [`gather_from`] reads it.
pub(crate) struct WorkspaceWeek {
    pub id: String,
    pub title: String,
    pub activity: WorkspaceActivity,
}

/// The week's usage ledger rows (run ledger v2) and the person's hours-saved
/// baselines (`None`: unreadable), as [`gather_from`] reads them.
pub(crate) struct LedgerWeek {
    pub rows: Vec<crate::run_ledger::LedgerRow>,
    pub baselines: Option<crate::value::Baselines>,
}

/// Monday and Friday of ISO week `2026-W41`.
pub(crate) fn week_days(week: &str) -> Option<(NaiveDate, NaiveDate)> {
    let (year, num) = week.trim().split_once("-W")?;
    let monday = NaiveDate::from_isoywd_opt(year.parse().ok()?, num.parse().ok()?, Weekday::Mon)?;
    Some((monday, monday + Duration::days(4)))
}

/// The ISO week `at` falls in.
pub(crate) fn week_of(at: NaiveDate) -> String {
    let iso = at.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

fn parse_at(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn in_range(event: &ActivityEvent, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
    parse_at(&event.at).is_some_and(|at| at >= start && at < end)
}

fn result_path(ws: &WorkspaceWeek, e: &ActivityEvent) -> Option<String> {
    e.path.clone().or_else(|| {
        e.result
            .as_ref()
            .map(|r| format!("workspaces/{}/{}", ws.id, r.name))
    })
}

fn result_name(e: &ActivityEvent) -> String {
    e.result
        .as_ref()
        .map(|r| r.name.clone())
        .unwrap_or_else(|| e.title.clone())
}

fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// The week's facts from what was read (see the module doc). `tasks` come
/// with each file's modified time; `next` is the following week's days.
pub(crate) fn gather_from(
    week: &str,
    workspaces: &[WorkspaceWeek],
    tasks: &[(Task, Option<DateTime<Utc>>)],
    ledger: &Result<LedgerWeek, String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    next: (NaiveDate, NaiveDate),
) -> WeekFacts {
    let mut f = WeekFacts {
        week: week.to_string(),
        start: start.to_rfc3339(),
        end: end.to_rfc3339(),
        ..WeekFacts::default()
    };
    let mut conversations = 0usize;
    let mut runs = 0usize;
    let mut failed = 0usize;
    let mut accepted = 0usize;
    let mut changed = 0usize;

    for ws in workspaces {
        let events: Vec<&ActivityEvent> = ws
            .activity
            .events
            .iter()
            .filter(|e| in_range(e, start, end))
            .collect();
        let mut ws_runs = 0usize;
        let mut ws_sessions: Vec<&str> = Vec::new();
        let mut drafts: BTreeMap<String, &ActivityEvent> = BTreeMap::new();
        for e in &events {
            match e.kind.as_str() {
                "session" => {
                    conversations += 1;
                    if let Some(key) = e.session_key.as_deref() {
                        ws_sessions.push(key);
                    }
                }
                "run" => {
                    runs += 1;
                    ws_runs += 1;
                    if e.status == "failed" {
                        failed += 1;
                        let label = e
                            .session_key
                            .as_deref()
                            .map(|k| format!("conversation {k}"))
                            .or_else(|| e.receipt_id.as_deref().map(|r| format!("run record {r}")));
                        let text = format!("A run failed in {}: {}", ws.title, e.title);
                        f.blocked.push(match label {
                            Some(label) => DigestLine::with(text, label, None),
                            None => DigestLine::bare(text, "no record names the run"),
                        });
                    }
                }
                "result" => {
                    changed += 1;
                    let name = result_name(e);
                    if e.status == "accepted" {
                        accepted += 1;
                        drafts.remove(&name);
                        f.shipped.push(DigestLine::with(
                            format!("Accepted “{name}” in {}", ws.title),
                            name.clone(),
                            result_path(ws, e),
                        ));
                    } else if e.status != "deleted"
                        && !f
                            .shipped
                            .iter()
                            .any(|l| l.evidence.as_ref().is_some_and(|ev| ev.label == name))
                    {
                        drafts.entry(name).or_insert(e);
                    }
                }
                "install" if e.status == "installed" => {
                    let commit = e
                        .install
                        .as_ref()
                        .and_then(|i| i.commit.clone())
                        .unwrap_or_else(|| "a build".into());
                    f.shipped.push(DigestLine::with(
                        format!("Installed {commit} from {}", ws.title),
                        "the install record",
                        None,
                    ));
                }
                _ => {}
            }
        }
        for (name, e) in drafts {
            f.in_progress.push(DigestLine::with(
                format!("“{name}” in {} ({})", ws.title, e.status),
                name.clone(),
                result_path(ws, e),
            ));
        }
        if ws_runs > 0 {
            let text = format!(
                "{}: {ws_runs} run{} in {} conversation{}",
                ws.title,
                if ws_runs == 1 { "" } else { "s" },
                ws_sessions.len().max(1),
                if ws_sessions.len() > 1 { "s" } else { "" }
            );
            f.in_progress.push(match ws_sessions.first() {
                Some(key) => DigestLine::with(text, format!("conversation {key}"), None),
                None => DigestLine::with(text, format!("workspace {}", ws.id), None),
            });
        }
    }

    for (task, modified) in tasks {
        let path = task.vault_path.clone();
        let label = path.clone().unwrap_or_else(|| task.title.clone());
        if crate::vault_tasks::is_terminal_status(&task.status) {
            if modified.is_some_and(|m| m >= start && m < end) {
                f.tasks_done += 1;
                f.shipped.push(DigestLine {
                    text: format!("Done: {}", task.title),
                    evidence: Some(Evidence {
                        label,
                        path: path.clone(),
                    }),
                    unverified: Some(
                        "when it was done comes only from the file's modified time".into(),
                    ),
                });
            }
            continue;
        }
        if crate::vault_tasks::is_cancelled_status(&task.status) {
            continue;
        }
        if crate::vault_tasks::is_hold_status(&task.status) {
            f.blocked.push(DigestLine::with(
                format!("{} ({})", task.title, task.status.trim()),
                label.clone(),
                path.clone(),
            ));
        }
        let due = task
            .due
            .as_deref()
            .and_then(|d| NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d").ok());
        if let Some(due) = due.filter(|d| *d >= next.0 && *d <= next.1) {
            f.next_week.push(DigestLine::with(
                format!("Due {}: {}", due.format("%a %b %-d"), task.title),
                label,
                path,
            ));
        }
    }

    let n = |text: String| DigestLine::with(text, "Hollow's records for the week", None);
    f.numbers.push(n(format!(
        "{conversations} conversation{} active, {runs} run{} ({failed} failed)",
        if conversations == 1 { "" } else { "s" },
        if runs == 1 { "" } else { "s" }
    )));
    f.numbers.push(n(format!(
        "{changed} result change{}, {accepted} accepted",
        if changed == 1 { "" } else { "s" }
    )));
    f.accepted = accepted;
    ledger_numbers(&mut f, ledger);
    f
}

/// The money lines: spend and spend by department from the ledger's own
/// columns, then hours saved from the value ledger (#6), an estimate.
fn ledger_numbers(f: &mut WeekFacts, ledger: &Result<LedgerWeek, String>) {
    let week = match ledger {
        Ok(week) => week,
        Err(_) => {
            f.numbers.push(DigestLine::bare(
                "Model spend: not available",
                "the usage record couldn't be read",
            ));
            f.numbers.push(DigestLine::bare(
                "Hours saved: not available",
                "the usage record couldn't be read",
            ));
            return;
        }
    };
    let baselines = week.baselines.clone().unwrap_or_default();
    let runs = crate::value::runs(&week.rows, &baselines);
    let cost: f64 = runs.iter().map(|r| r.cost_usd).sum();
    let unpriced = runs.iter().filter(|r| r.unknown_cost).count();
    let mut line = format!(
        "Model spend: {} (usage ledger, {} run{})",
        money(cost),
        runs.len(),
        if runs.len() == 1 { "" } else { "s" }
    );
    if unpriced > 0 {
        line.push_str(&format!(
            "; {unpriced} run{} without a known cost",
            if unpriced == 1 { "" } else { "s" }
        ));
    }
    f.numbers.push(DigestLine::with(line, "usage ledger", None));
    let mut by_department: BTreeMap<String, f64> = BTreeMap::new();
    for run in &runs {
        let dept = if run.department == crate::run_attribution::UNKNOWN {
            "No department".to_string()
        } else {
            run.department.clone()
        };
        *by_department.entry(dept).or_default() += run.cost_usd;
    }
    if runs
        .iter()
        .any(|r| r.department != crate::run_attribution::UNKNOWN)
    {
        for (dept, usd) in &by_department {
            f.numbers.push(DigestLine::with(
                format!("  {dept}: {}", money(*usd)),
                "usage ledger (department column)",
                None,
            ));
        }
    } else {
        f.numbers.push(DigestLine::bare(
            "Spend by department: not available",
            "no run this week was in a workspace with a department",
        ));
    }
    let accepted = runs
        .iter()
        .filter(|r| r.outcome_label == "accepted")
        .count();
    // G4: hidden until a run is accepted.
    if accepted == 0 {
        return;
    }
    match &week.baselines {
        None => f.numbers.push(DigestLine::bare(
            "Hours saved: not available",
            "the hours-saved baselines couldn't be read",
        )),
        Some(b) if b.baselines.is_empty() && b.run_overrides.is_empty() => {
            f.numbers.push(DigestLine::bare(
                "Hours saved: not available",
                "no hours-saved baseline is set (Settings → Usage → Value this month)",
            ))
        }
        Some(_) => {
            let minutes: f64 = runs.iter().map(|r| r.saved_minutes).sum();
            f.numbers.push(DigestLine::with(
                format!(
                    "Hours saved (estimate): {:.1} h from {accepted} accepted run{}",
                    minutes / 60.0,
                    if accepted == 1 { "" } else { "s" }
                ),
                "value ledger: your baselines, accepted runs only (accepted Result or issue Done)",
                None,
            ));
        }
    }
}

/// A path from the draft's folder to vault path `rel`.
fn link_from_draft(rel: &str) -> String {
    let up = "../".repeat(DRAFT_DIR.split('/').count());
    let target = format!("{up}{rel}");
    if target.contains(' ') || target.contains('(') || target.contains(')') {
        format!("<{target}>")
    } else {
        target
    }
}

fn render_lines(lines: &[DigestLine]) -> String {
    if lines.is_empty() {
        return "- Nothing recorded.".into();
    }
    lines
        .iter()
        .map(|l| {
            let mut out = format!("- {}", l.text);
            match &l.evidence {
                Some(Evidence {
                    label,
                    path: Some(path),
                }) => out.push_str(&format!(" ([{label}]({}))", link_from_draft(path))),
                Some(Evidence { label, path: None }) => {
                    out.push_str(&format!(" (evidence: {label})"))
                }
                None => {}
            }
            if let Some(why) = &l.unverified {
                out.push_str(&format!(" — *not verified: {why}*"));
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Fill the template with the facts.
pub(crate) fn render(template: &str, f: &WeekFacts, range: &str, drafted: &str) -> String {
    template
        .replace("{{title}}", TITLE)
        .replace("{{week}}", &f.week)
        .replace("{{range}}", range)
        .replace("{{drafted}}", drafted)
        .replace("{{shipped}}", &render_lines(&f.shipped))
        .replace("{{in_progress}}", &render_lines(&f.in_progress))
        .replace("{{blocked}}", &render_lines(&f.blocked))
        .replace("{{numbers}}", &render_lines(&f.numbers))
        .replace("{{next_week}}", &render_lines(&f.next_week))
}

pub(crate) fn local_start(day: NaiveDate) -> DateTime<Utc> {
    crate::schedule_windows::resolve_local(&Local, day.and_hms_opt(0, 0, 0).unwrap_or_default())
}

/// The ledger rows of `[start, end)`, outcome labels refreshed first (an
/// unrefreshed label only undercounts hours saved), and the baselines.
fn ledger_week(
    vault: &Path,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<LedgerWeek, String> {
    if let Err(e) = crate::run_ledger::refresh_outcome_labels(vault) {
        eprintln!("[digest] outcome labels: {e}");
    }
    let rows = crate::run_ledger::rows_between(
        vault,
        &crate::session_store::db_time(start),
        &crate::session_store::db_time(end),
    )?;
    Ok(LedgerWeek {
        rows,
        baselines: crate::value::stored_baselines().ok(),
    })
}

/// Read week `week`'s facts, up to `now` (see the module doc).
pub(crate) fn gather(vault: &Path, week: &str, now: DateTime<Utc>) -> Result<WeekFacts, String> {
    let (monday, _) =
        week_days(week).ok_or_else(|| format!("'{week}' is not an ISO week like 2026-W41."))?;
    let start = local_start(monday);
    let end = local_start(monday + Duration::days(7)).min(now);
    let next = (monday + Duration::days(7), monday + Duration::days(13));
    let mut weeks = Vec::new();
    for ws in crate::workspaces::list(vault)? {
        if crate::workspaces::is_main_agent(&ws.id) {
            continue;
        }
        let Ok(activity) =
            crate::workspace_activity::list(vault, &ws.id, Some(EVENTS_PER_WORKSPACE), None)
        else {
            continue;
        };
        weeks.push(WorkspaceWeek {
            id: ws.id,
            title: ws.title,
            activity,
        });
    }
    let tasks: Vec<(Task, Option<DateTime<Utc>>)> = crate::vault_tasks::list(vault)
        .into_iter()
        .map(|mut t| {
            // `list` leaves `vault_path` to the UI's enrichment; the stable
            // url is `vault:<rel>`.
            if t.vault_path.is_none() {
                t.vault_path = t.url.strip_prefix("vault:").map(str::to_string);
            }
            let modified = t
                .vault_path
                .as_deref()
                .and_then(|p| std::fs::metadata(vault.join(p)).ok())
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from);
            (t, modified)
        })
        .collect();
    let ledger = ledger_week(vault, start, end);
    Ok(gather_from(week, &weeks, &tasks, &ledger, start, end, next))
}

/// The draft's vault path for week `week` (named for its Friday).
pub(crate) fn draft_rel(week: &str) -> Option<String> {
    week_days(week).map(|(_, friday)| format!("{DRAFT_DIR}/{friday}.md"))
}

/// What `digest_weekly` answers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeeklyDraft {
    pub week: String,
    pub path: String,
    /// False when the draft already existed (it was left as it is).
    pub created: bool,
    pub facts: WeekFacts,
}

/// Draft week `week` into the vault (never over an existing draft). The
/// Main agent's workspace must exist.
pub(crate) fn write_draft(
    vault: &Path,
    week: &str,
    now: DateTime<Utc>,
) -> Result<WeeklyDraft, String> {
    if crate::workspaces::main_agent_cwd(vault).is_none() {
        return Err(
            "There is no COO workspace yet, so there is nowhere to draft the weekly update.".into(),
        );
    }
    let rel =
        draft_rel(week).ok_or_else(|| format!("'{week}' is not an ISO week like 2026-W41."))?;
    let facts = gather(vault, week, now)?;
    let path = vault.join(&rel);
    if path.exists() {
        return Ok(WeeklyDraft {
            week: week.into(),
            path: rel,
            created: false,
            facts,
        });
    }
    let template_path = vault.join(TEMPLATE_REL);
    let template = match std::fs::read_to_string(&template_path) {
        Ok(text) if !text.trim().is_empty() => text,
        _ => {
            if let Some(dir) = template_path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| format!("template folder: {e}"))?;
            }
            crate::vault::atomic_write(&template_path, DEFAULT_TEMPLATE.as_bytes())?;
            DEFAULT_TEMPLATE.to_string()
        }
    };
    let (monday, friday) = week_days(week).unwrap_or_default();
    let range = format!(
        "{} – {}",
        monday.format("%b %-d"),
        friday.format("%b %-d, %Y")
    );
    let drafted = now
        .with_timezone(&Local)
        .format("%a %b %-d, %H:%M")
        .to_string();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("weekly folder: {e}"))?;
    }
    crate::vault::atomic_write(
        &path,
        render(&template, &facts, &range, &drafted).as_bytes(),
    )?;
    Ok(WeeklyDraft {
        week: week.into(),
        path: rel,
        created: true,
        facts,
    })
}

/// The week to draft when none is named: this one.
pub(crate) fn this_week(now: DateTime<Utc>) -> String {
    week_of(now.with_timezone(&Local).date_naive())
}

/// Draft the weekly update for `week` (ISO, `2026-W41`; default this week)
/// into `workspaces/main/outputs/weekly/<Friday>.md`, from Hollow's records.
/// An existing draft is left as it is.
#[tauri::command]
pub async fn digest_weekly(
    state: tauri::State<'_, crate::commands::AppState>,
    week: Option<String>,
) -> Result<WeeklyDraft, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let now = Utc::now();
        let week = week
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .unwrap_or_else(|| this_week(now));
        write_draft(&vault, &week, now)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The prose turn's message.
pub(crate) fn prose_message(week: &str, path: &str) -> String {
    format!(
        "{} your weekly update for {week} is drafted at `{path}` from Hollow's records. Rewrite its prose in place so it reads well for the people who get it: use only the facts already in it, keep every evidence link and every *not verified* label, add nothing new, and keep the section headings. Then reply in one or two lines: \"Your weekly update is ready\" with the path. Don't publish or send it anywhere: the person reviews it first. This is a check-in, not a new request: don't start new work.",
        crate::main_agent::WAKE_PREFIX
    )
}

/// The ISO week a window occurrence key names: the key itself for a week
/// key, the week of its date for a day key (a `friday-pm` edited to several
/// days).
pub(crate) fn week_for_key(key: &str) -> Option<String> {
    if week_days(key).is_some() {
        return Some(key.to_string());
    }
    NaiveDate::parse_from_str(key, "%Y-%m-%d").ok().map(week_of)
}

/// Whether the prose turn for `week` is still due: only for a draft this job
/// created (`draft:<week>`), never one the person drafted (and maybe edited)
/// earlier, and only once (`prose:<week>`).
pub(crate) fn prose_due(state: &crate::schedule_windows::JobState, week: &str) -> bool {
    state.at.contains_key(&format!("draft:{week}"))
        && !state.at.contains_key(&format!("prose:{week}"))
}

/// The `friday-pm` window job (`schedule_windows::JOBS`): draft, notify, then
/// one Main agent prose turn. `Err` retries next minute (the vault or the
/// Main agent's thread was busy); the draft is never written twice.
pub(crate) async fn run_weekly_job(
    app: tauri::AppHandle,
    config: crate::config::Config,
    key: String,
    _late: bool,
) -> Result<(), String> {
    let vault = config.vault_path.clone();
    let week = week_for_key(&key).ok_or_else(|| format!("'{key}' names no week"))?;
    if crate::workspaces::main_agent_cwd(&vault).is_none() {
        eprintln!("[weekly update] no COO workspace; {week} not drafted");
        return Ok(());
    }
    let drafted = {
        let vault = vault.clone();
        let week = week.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let guard = crate::automated_history::scheduler_write_lease(&vault)
                .map_err(|_| "the vault is busy".to_string())?;
            let out = write_draft(&vault, &week, Utc::now());
            drop(guard);
            out
        })
        .await
        .map_err(|e| e.to_string())??
    };
    if drafted.created {
        crate::schedule_windows::stamp_job(&format!("draft:{week}"), &week, Utc::now());
        let _ = crate::notifications::create(
            &app,
            &vault,
            crate::notifications::NotificationInput {
                title: "Your weekly update is ready".into(),
                body: format!(
                    "Drafted {} at {}. Review it before sharing.",
                    drafted.week, drafted.path
                ),
                kind: "info".into(),
                source: Some("weekly-update".into()),
                action_label: Some("Open".into()),
                action_url: Some(format!("vault:{}", drafted.path)),
                notify_native: true,
            },
        );
    }
    // The week in review (overnight 10-11 A7): its snapshot beside the
    // update, written once per week by this job (a fuller one replaces a
    // mid-week Save), under the same lease, then a notification. A failure
    // is logged, never retried here: the prose turn below still runs.
    let review = if crate::schedule_windows::load_jobs()
        .at
        .contains_key(&format!("review:{week}"))
    {
        Ok(None)
    } else {
        let vault = vault.clone();
        let week = week.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let guard = crate::automated_history::scheduler_write_lease(&vault)
                .map_err(|_| "the vault is busy".to_string())?;
            let out = crate::weekly_review::save_snapshot(&vault, &week, Local::now()).map(Some);
            drop(guard);
            out
        })
        .await
        .unwrap_or_else(|e| Err(format!("the snapshot task stopped: {e}")))
    };
    match review {
        Ok(Some(path)) => {
            crate::schedule_windows::stamp_job(&format!("review:{week}"), &week, Utc::now());
            let _ = crate::notifications::create(
                &app,
                &vault,
                crate::notifications::NotificationInput {
                    title: "Your week in review is ready".into(),
                    body: format!("{week}: what shipped, what slipped and next week. Press g then e in Hollow."),
                    kind: "info".into(),
                    source: Some("weekly-review".into()),
                    action_label: Some("Open".into()),
                    action_url: Some(format!("vault:{path}")),
                    notify_native: true,
                },
            );
        }
        Ok(None) => {}
        Err(e) => eprintln!("[weekly review] {week}: {e}"),
    }
    if !prose_due(&crate::schedule_windows::load_jobs(), &week) {
        return Ok(());
    }
    match crate::main_agent::queue_hollow_turn(
        &app,
        &config,
        prose_message(&week, &drafted.path),
        crate::main_agent::ASKED_BY_WEEKLY,
    )
    .await
    {
        crate::main_agent::TurnQueued::Queued => {
            crate::schedule_windows::stamp_job(&format!("prose:{week}"), &week, Utc::now());
            Ok(())
        }
        crate::main_agent::TurnQueued::Busy => Err("the COO's thread is busy".into()),
        crate::main_agent::TurnQueued::Skipped(why) => {
            eprintln!("[weekly update] {week}: prose turn skipped: {why}");
            Ok(())
        }
    }
}

/// [`run_weekly_job`] as a window job.
pub(crate) fn weekly_job(
    app: tauri::AppHandle,
    config: crate::config::Config,
    key: String,
    late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(run_weekly_job(app, config, key, late))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_ledger::LedgerRow;
    use crate::value::{Baseline, Baselines};
    use crate::workspace_activity::{ActivityGaps, ActivityInstall, ActivityResult, ActivityRun};
    use chrono::TimeZone;

    fn at(d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap()
    }

    fn event(kind: &str, status: &str, at: DateTime<Utc>) -> ActivityEvent {
        ActivityEvent {
            id: format!("{kind}-{status}-{}", at.timestamp()),
            kind: kind.into(),
            at: at.to_rfc3339(),
            status: status.into(),
            title: "Draft the RFP answer".into(),
            ..Default::default()
        }
    }

    fn run(status: &str, at: DateTime<Utc>, cost: Option<f64>) -> ActivityEvent {
        ActivityEvent {
            session_key: Some("s-20261006-aaaa".into()),
            run: Some(ActivityRun {
                source: "chat".into(),
                cost_usd: cost,
                ..Default::default()
            }),
            ..event("run", status, at)
        }
    }

    fn result(name: &str, status: &str, at: DateTime<Utc>) -> ActivityEvent {
        ActivityEvent {
            path: Some(format!("workspaces/rfp/{name}")),
            result: Some(ActivityResult {
                name: name.into(),
                kind: "document".into(),
                exists: true,
                ..Default::default()
            }),
            ..event("result", status, at)
        }
    }

    fn ws(id: &str, events: Vec<ActivityEvent>) -> WorkspaceWeek {
        WorkspaceWeek {
            id: id.into(),
            title: format!("Workspace {id}"),
            activity: WorkspaceActivity {
                workspace_id: id.into(),
                as_of: String::new(),
                total: events.len(),
                events,
                gaps: ActivityGaps::default(),
                next_cursor: None,
            },
        }
    }

    fn task(title: &str, status: &str, due: Option<&str>) -> Task {
        let mut t: Task = serde_json::from_value(serde_json::json!({
            "url": format!("tasks/{title}.md"), "title": title, "status": status,
            "description": "", "bucket": "", "labels": [], "subtasks": []
        }))
        .unwrap_or_else(|e| panic!("task fixture: {e}"));
        t.vault_path = Some(format!("tasks/{title}.md"));
        t.due = due.map(str::to_string);
        t
    }

    fn fixture_week() -> WeekFacts {
        let (start, end) = (at(5, 4), at(9, 19));
        let rfp = ws(
            "rfp",
            vec![
                ActivityEvent {
                    session_key: Some("s-20261006-aaaa".into()),
                    ..event("session", "succeeded", at(6, 14))
                },
                run("succeeded", at(6, 14), Some(1.25)),
                run("failed", at(7, 10), None),
                result("outputs/answer.md", "created", at(6, 15)),
                result("outputs/answer.md", "accepted", at(8, 9)),
                result("outputs/pricing.md", "draft", at(8, 10)),
                result("outputs/old.md", "accepted", at(1, 10)),
            ],
        );
        let ops = ws(
            "ops",
            vec![
                run("succeeded", at(9, 9), Some(0.75)),
                ActivityEvent {
                    install: Some(ActivityInstall {
                        step: "install".into(),
                        commit: Some("1936d7e9".into()),
                        ..Default::default()
                    }),
                    ..event("install", "installed", at(8, 14))
                },
            ],
        );
        let tasks = vec![
            (task("Ship the report", "Done", None), Some(at(7, 12))),
            (task("Old done", "Done", None), Some(at(1, 12))),
            (task("Vendor quote", "Waiting on Sam", None), None),
            (task("Board deck", "To do", Some("2026-10-14")), None),
            (task("Later thing", "To do", Some("2026-10-30")), None),
            (task("Dropped", "Cancelled", Some("2026-10-13")), None),
        ];
        let next = (
            NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
        );
        let ledger = ledger(Some(document_baseline()));
        gather_from("2026-W41", &[rfp, ops], &tasks, &ledger, start, end, next)
    }

    fn ledger_row(run: &str, cost: Option<f64>, dept: &str, label: &str) -> LedgerRow {
        LedgerRow {
            run_key: format!("k-{run}"),
            run_id: Some(run.into()),
            at: "2026-10-06T14:00:00Z".into(),
            surface: "chat".into(),
            agent: "claude".into(),
            model: "m".into(),
            outcome: "success".into(),
            spawned: true,
            cost_usd: cost,
            input_tokens: None,
            output_tokens: None,
            account: "unknown".into(),
            workspace: "rfp".into(),
            department: dept.into(),
            work_kind: "document".into(),
            outcome_label: label.into(),
        }
    }

    /// The week's ledger: an accepted Revenue run, an unpriced one, and a
    /// run whose workspace had no department when it launched.
    fn ledger(baselines: Option<Baselines>) -> Result<LedgerWeek, String> {
        Ok(LedgerWeek {
            rows: vec![
                ledger_row("R1", Some(1.25), "Revenue", "accepted"),
                ledger_row("R2", None, "Revenue", "unknown"),
                ledger_row("R3", Some(0.75), "unknown", "unknown"),
            ],
            baselines,
        })
    }

    fn document_baseline() -> Baselines {
        let mut b = Baselines::default();
        b.baselines.insert(
            "document".into(),
            Baseline {
                minutes: 90.0,
                note: None,
                set_at: "2026-10-01T00:00:00Z".into(),
            },
        );
        b
    }

    #[test]
    fn a_fixture_week_gives_the_expected_sections() {
        let f = fixture_week();
        let texts = |lines: &[DigestLine]| lines.iter().map(|l| l.text.clone()).collect::<Vec<_>>();
        assert_eq!(
            texts(&f.shipped),
            vec![
                "Accepted “outputs/answer.md” in Workspace rfp",
                "Installed 1936d7e9 from Workspace ops",
                "Done: Ship the report",
            ]
        );
        assert_eq!(
            texts(&f.in_progress),
            vec![
                "“outputs/pricing.md” in Workspace rfp (draft)",
                "Workspace rfp: 2 runs in 1 conversation",
                "Workspace ops: 1 run in 1 conversation",
            ]
        );
        assert_eq!(
            texts(&f.blocked),
            vec![
                "A run failed in Workspace rfp: Draft the RFP answer",
                "Vendor quote (Waiting on Sam)",
            ]
        );
        assert_eq!(texts(&f.next_week), vec!["Due Wed Oct 14: Board deck"]);
        let numbers = texts(&f.numbers);
        assert_eq!(numbers[0], "1 conversation active, 3 runs (1 failed)");
        assert_eq!(numbers[1], "3 result changes, 1 accepted");
        assert_eq!(
            numbers[2],
            "Model spend: $2.00 (usage ledger, 3 runs); 1 run without a known cost"
        );
        assert!(numbers.contains(&"  Revenue: $1.25".to_string()));
        assert!(numbers.contains(&"  No department: $0.75".to_string()));
        assert_eq!(
            numbers.last().unwrap(),
            "Hours saved (estimate): 1.5 h from 1 accepted run"
        );
    }

    #[test]
    fn a_fact_with_no_evidence_is_labelled_not_verified() {
        let f = fixture_week();
        // A done date that only a file's modified time gives.
        let done = f
            .shipped
            .iter()
            .find(|l| l.text.starts_with("Done:"))
            .unwrap();
        assert!(done.unverified.is_some());
        let hours = f.numbers.last().unwrap();
        assert!(hours.evidence.is_some(), "the value ledger is its evidence");
        let text = render(
            DEFAULT_TEMPLATE,
            &f,
            "Oct 5 – Oct 9, 2026",
            "Fri Oct 9, 15:00",
        );
        assert!(text.contains("- Hours saved (estimate): 1.5 h from 1 accepted run"));
        assert!(text.contains(
            "- Done: Ship the report ([tasks/Ship the report.md](<../../../../tasks/Ship the report.md>)) — *not verified:"
        ));
        assert!(text.contains(
            "- Accepted “outputs/answer.md” in Workspace rfp ([outputs/answer.md](../../../../workspaces/rfp/outputs/answer.md))"
        ));
        assert!(text.contains("(evidence: conversation s-20261006-aaaa)"));
        // Every list line has evidence or says it is not verified.
        for line in text
            .lines()
            .filter(|l| l.starts_with("- ") && *l != "- Nothing recorded.")
        {
            assert!(
                line.contains("](")
                    || line.contains("(evidence: ")
                    || line.contains("*not verified"),
                "{line}"
            );
        }
        assert!(text.starts_with("# Operations AI Requests – Weekly Update\n"));
        assert!(!text.contains("{{"));
    }

    #[test]
    fn without_departments_baselines_or_ledger_the_numbers_say_not_available() {
        let quiet = ws("x", vec![run("succeeded", at(6, 9), Some(1.0))]);
        let gather = |ledger: &Result<LedgerWeek, String>| {
            gather_from(
                "2026-W41",
                std::slice::from_ref(&quiet),
                &[],
                ledger,
                at(5, 4),
                at(9, 19),
                (NaiveDate::MIN, NaiveDate::MIN),
            )
        };
        let line = |f: &WeekFacts, text: &str| {
            f.numbers
                .iter()
                .find(|l| l.text == text)
                .cloned()
                .unwrap_or_else(|| panic!("no line {text:?} in {:?}", f.numbers))
        };
        let no_dept = Ok(LedgerWeek {
            rows: vec![ledger_row("R9", Some(1.0), "unknown", "accepted")],
            baselines: Some(Baselines::default()),
        });
        let f = gather(&no_dept);
        assert!(line(&f, "Spend by department: not available")
            .unverified
            .is_some());
        let hours = line(&f, "Hours saved: not available");
        assert!(hours
            .unverified
            .unwrap()
            .contains("no hours-saved baseline"));
        let f = gather(&ledger(None));
        assert!(line(&f, "Hours saved: not available")
            .unverified
            .unwrap()
            .contains("couldn't be read"));
        let f = gather(&Err("locked".into()));
        assert!(line(&f, "Model spend: not available").unverified.is_some());
        // G4: with no accepted run that week, hours saved are left out,
        // baselines set or not.
        for baselines in [Some(document_baseline()), Some(Baselines::default()), None] {
            let none_accepted = Ok(LedgerWeek {
                rows: vec![ledger_row("R8", Some(1.0), "Revenue", "unknown")],
                baselines,
            });
            let f = gather(&none_accepted);
            assert!(
                !f.numbers.iter().any(|l| l.text.starts_with("Hours saved")),
                "{:?}",
                f.numbers
            );
        }
        let empty = render(DEFAULT_TEMPLATE, &WeekFacts::default(), "", "");
        assert!(empty.contains("## Shipped\n\n- Nothing recorded."));
    }

    /// Phase 1 integration (F2 + #4): department and cost come from the
    /// ledger's own columns, as recorded when each run launched, not from
    /// what a workspace's brief says today (this vault has no workspace at
    /// all).
    #[test]
    fn the_week_reads_the_ledgers_department_and_cost_columns() {
        let dir = tempfile::Builder::new()
            .prefix("hollow-digest-")
            .tempdir()
            .unwrap();
        let v = dir.path();
        crate::run_ledger::tests::row(v, "k1", "R1", "success", Some(2.5));
        crate::run_ledger::tests::attribute(v, "k1", "Finance", "data", "acct-1");
        crate::run_ledger::tests::row(v, "k2", "R2", "success", Some(0.5));
        let now = Utc::now();
        let week = ledger_week(v, now - Duration::hours(1), now + Duration::hours(1)).unwrap();
        assert_eq!(week.rows.len(), 2);
        let f = gather_from(
            "2026-W41",
            &[],
            &[],
            &Ok(week),
            now - Duration::hours(1),
            now + Duration::hours(1),
            (NaiveDate::MIN, NaiveDate::MIN),
        );
        let texts: Vec<_> = f.numbers.iter().map(|l| l.text.as_str()).collect();
        assert!(
            texts.contains(&"Model spend: $3.00 (usage ledger, 2 runs)"),
            "{texts:?}"
        );
        assert!(texts.contains(&"  Finance: $2.50"), "{texts:?}");
        assert!(texts.contains(&"  No department: $0.50"), "{texts:?}");
        // Outside the week: nothing.
        let before = ledger_week(v, now - Duration::days(9), now - Duration::days(8)).unwrap();
        assert!(before.rows.is_empty());
    }

    #[test]
    fn iso_weeks_map_to_their_monday_and_friday() {
        let (mon, fri) = week_days("2026-W41").unwrap();
        assert_eq!(
            (mon.to_string(), fri.to_string()),
            ("2026-10-05".into(), "2026-10-09".into())
        );
        assert_eq!(
            week_of(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap()),
            "2026-W41"
        );
        assert_eq!(
            draft_rel("2026-W41").unwrap(),
            "workspaces/main/outputs/weekly/2026-10-09.md"
        );
        assert!(week_days("October").is_none());
        assert!(week_days("2026-W60").is_none());
    }

    #[test]
    fn the_draft_needs_the_main_agent_and_never_overwrites() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        assert!(write_draft(v, "2026-W41", at(9, 19))
            .unwrap_err()
            .contains("no COO"));
        crate::workspaces::ensure_main_agent(v).unwrap();
        let first = write_draft(v, "2026-W41", at(9, 19)).unwrap();
        assert!(first.created);
        assert_eq!(first.path, "workspaces/main/outputs/weekly/2026-10-09.md");
        assert!(v.join(TEMPLATE_REL).is_file());
        let draft = v.join(&first.path);
        std::fs::write(&draft, "edited by the prose pass").unwrap();
        let second = write_draft(v, "2026-W41", at(9, 20)).unwrap();
        assert!(!second.created);
        assert_eq!(
            std::fs::read_to_string(&draft).unwrap(),
            "edited by the prose pass"
        );
        // The person's template wins.
        std::fs::write(v.join(TEMPLATE_REL), "Custom {{week}}\n{{numbers}}").unwrap();
        let other = write_draft(v, "2026-W42", at(16, 19)).unwrap();
        let text = std::fs::read_to_string(v.join(&other.path)).unwrap();
        assert!(text.starts_with("Custom 2026-W42\n- "), "{text}");
    }

    /// Regression: `vault_tasks::list` leaves `vault_path` empty, so a real
    /// gather must still link a task done this week (and date it by its file).
    #[test]
    fn a_real_task_file_done_this_week_is_linked_from_the_vault() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("tasks")).unwrap();
        std::fs::write(
            v.join("tasks/ship-it.md"),
            "---\ntitle: Ship it\nstatus: Done\n---\n",
        )
        .unwrap();
        let now = Utc::now();
        let week = this_week(now);
        let f = gather(v, &week, now).unwrap();
        let done = f
            .shipped
            .iter()
            .find(|l| l.text == "Done: Ship it")
            .unwrap_or_else(|| panic!("{:?}", f.shipped));
        assert_eq!(
            done.evidence.as_ref().and_then(|e| e.path.as_deref()),
            Some("tasks/ship-it.md")
        );
    }

    /// Review finding 2: a draft the person made earlier (and may have
    /// edited) never gets the prose turn; the job's own draft gets it once.
    #[test]
    fn the_prose_turn_only_follows_the_jobs_own_draft_once() {
        let mut state = crate::schedule_windows::JobState::default();
        assert!(!prose_due(&state, "2026-W41"));
        state.at.insert("draft:2026-W41".into(), "t".into());
        assert!(prose_due(&state, "2026-W41"));
        state.at.insert("prose:2026-W41".into(), "t".into());
        assert!(!prose_due(&state, "2026-W41"));
        assert_eq!(week_for_key("2026-W41").as_deref(), Some("2026-W41"));
        assert_eq!(week_for_key("2026-10-08").as_deref(), Some("2026-W41"));
        assert_eq!(week_for_key("someday"), None);
    }

    #[test]
    fn the_prose_turn_is_hollows_and_sends_nothing() {
        let m = prose_message("2026-W41", "workspaces/main/outputs/weekly/2026-10-09.md");
        assert!(m.starts_with(crate::main_agent::WAKE_PREFIX));
        assert!(m.contains("use only the facts already in it"));
        assert!(m.contains("Don't publish or send it anywhere"));
    }
}
