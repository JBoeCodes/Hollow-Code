//! Scheduler v2: named windows, catch-up and host window jobs (master plan
//! 2026-10, F1; see `docs/SCHEDULER.md`).
//!
//! **Named windows.** `tonight` (18:00–07:00, weekdays), `weekday-evening`
//! (18:00–22:00, weekdays), `friday-pm` (Friday 15:00–18:00) and `nightly`
//! (02:00–05:00, every day) are defined once, in
//! `<app support>/Scheduler/windows.json` (the defaults below until the person
//! changes them). Each window is wall-clock time in the Mac's own time zone
//! (decision 7's "ET" is this Mac's zone); DST is resolved per occurrence by
//! [`resolve_local`]: a start inside the spring-forward gap moves to the first
//! minute that exists, and one inside the repeated autumn hour takes the first
//! of the two.
//!
//! **Automations** say `schedule: window:friday-pm` instead of a cron
//! ([`WINDOW_PREFIX`]). The schedule string is a stamped permission field, so
//! a run that rewrote its own window is refused like any other schedule edit;
//! `vault_workflows::next_run_after` fires it at the window's next start.
//!
//! **Catch-up** ([`CatchUp`], frontmatter `catch_up:`) says what happens to an
//! occurrence missed while Hollow was closed: `skip` (the misfire grace, as
//! before), `run-once-on-wake` (one run, however many were missed; the
//! default for windows) or `run-all` (each missed one, at most
//! [`RUN_ALL_MAX`]). A cron automation without `catch_up:` keeps its grace.
//!
//! **Window jobs** ([`decide_job`]) are host jobs that run once per window
//! occurrence: their keys come from `period.rs` (a day, a weekday or an ISO
//! week), so N missed occurrences collapse to one run, the property that
//! module's harness pins. The first time a job is seen, occurrences that
//! already ended are stamped, never run: installing Hollow on a Thursday does
//! not draft last Friday's update.

use crate::period::{self, Period};
use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc,
    Weekday,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How an automation's schedule names a window: `window:friday-pm`.
pub(crate) const WINDOW_PREFIX: &str = "window:";
/// The most missed occurrences `run-all` catches up on.
pub(crate) const RUN_ALL_MAX: usize = 5;
/// How far back a window job looks for a missed occurrence.
const LOOKBACK_DAYS: i64 = 8;
/// The overnight hard stop (decision 7, 2026-10-08), in US dollars.
pub(crate) const OVERNIGHT_CAP_USD: f64 = 40.0;

/// One named window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WindowDef {
    pub name: String,
    /// What the person sees.
    pub label: String,
    /// `mon` … `sun`: the days an occurrence may start.
    pub days: Vec<String>,
    /// `HH:MM`, local wall-clock time.
    pub start: String,
    /// `HH:MM`; at or before `start` means the next day.
    pub end: String,
}

/// Every window, plus the overnight hard stop (windows are on the Automations page, the cap in Settings, Usage, Spending caps, Overnight cap).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WindowSettings {
    pub windows: Vec<WindowDef>,
    /// The night queue's hard stop per `tonight` occurrence; 0 refuses every
    /// night run.
    pub overnight_cap_usd: f64,
}

fn def(name: &str, label: &str, days: &[&str], start: &str, end: &str) -> WindowDef {
    WindowDef {
        name: name.into(),
        label: label.into(),
        days: days.iter().map(|d| d.to_string()).collect(),
        start: start.into(),
        end: end.into(),
    }
}

const WEEKDAYS: &[&str] = &["mon", "tue", "wed", "thu", "fri"];
const EVERY_DAY: &[&str] = &["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            windows: vec![
                def(
                    "tonight",
                    "Tonight (weekdays 18:00–07:00)",
                    WEEKDAYS,
                    "18:00",
                    "07:00",
                ),
                def(
                    "weekday-evening",
                    "Weekday evenings (18:00–22:00)",
                    WEEKDAYS,
                    "18:00",
                    "22:00",
                ),
                def(
                    "friday-pm",
                    "Friday afternoon (15:00–18:00)",
                    &["fri"],
                    "15:00",
                    "18:00",
                ),
                def(
                    "nightly",
                    "Nightly (02:00–05:00)",
                    EVERY_DAY,
                    "02:00",
                    "05:00",
                ),
            ],
            overnight_cap_usd: OVERNIGHT_CAP_USD,
        }
    }
}

fn parse_day(raw: &str) -> Option<Weekday> {
    match raw.trim().to_ascii_lowercase().get(..3)? {
        "mon" => Some(Weekday::Mon),
        "tue" => Some(Weekday::Tue),
        "wed" => Some(Weekday::Wed),
        "thu" => Some(Weekday::Thu),
        "fri" => Some(Weekday::Fri),
        "sat" => Some(Weekday::Sat),
        "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_hm(raw: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(raw.trim(), "%H:%M").ok()
}

impl WindowDef {
    fn weekdays(&self) -> Vec<Weekday> {
        self.days.iter().filter_map(|d| parse_day(d)).collect()
    }

    /// The `period.rs` period one occurrence belongs to: every day → a day,
    /// Monday to Friday → a weekday, one day a week → an ISO week.
    pub(crate) fn period(&self) -> Period {
        let days = self.weekdays();
        let weekdays = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ];
        if days.len() == 1 {
            Period::Week
        } else if days.len() == 5 && weekdays.iter().all(|d| days.contains(d)) {
            Period::Weekday
        } else {
            Period::Day
        }
    }

    /// Why this window can't be used, if it can't.
    pub(crate) fn problem(&self) -> Option<String> {
        if self.name.trim().is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Some(format!(
                "Window name '{}' must be lowercase letters, digits and hyphens.",
                self.name
            ));
        }
        if self.days.is_empty() || self.days.iter().any(|d| parse_day(d).is_none()) {
            return Some(format!(
                "Window '{}' needs days from mon, tue, wed, thu, fri, sat, sun.",
                self.name
            ));
        }
        if parse_hm(&self.start).is_none() || parse_hm(&self.end).is_none() {
            return Some(format!(
                "Window '{}' needs start and end as HH:MM.",
                self.name
            ));
        }
        None
    }
}

impl WindowSettings {
    /// Settings a person could have meant, or why not.
    pub(crate) fn validated(self) -> Result<Self, String> {
        let mut seen = std::collections::HashSet::new();
        for w in &self.windows {
            if let Some(problem) = w.problem() {
                return Err(problem);
            }
            if !seen.insert(w.name.clone()) {
                return Err(format!("Window '{}' is defined twice.", w.name));
            }
        }
        if !self.overnight_cap_usd.is_finite() || !(0.0..=1000.0).contains(&self.overnight_cap_usd)
        {
            return Err("The overnight cap must be between $0 and $1,000.".into());
        }
        Ok(self)
    }

    pub(crate) fn window(&self, name: &str) -> Option<&WindowDef> {
        self.windows
            .iter()
            .find(|w| w.name == name.trim() && w.problem().is_none())
    }
}

fn state_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-scheduler-"))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|root| root.join("Scheduler"))
    }
}

fn settings_path() -> Option<PathBuf> {
    state_root().map(|root| root.join("windows.json"))
}

/// The saved settings, or the defaults (also for an unreadable file: a typo
/// must not unschedule everything).
pub(crate) fn settings() -> WindowSettings {
    settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str::<WindowSettings>(&text).ok())
        .and_then(|s| s.validated().ok())
        .unwrap_or_default()
}

fn write_json(path: &std::path::Path, value: &impl Serialize) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("scheduler folder: {e}"))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("scheduler write: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("scheduler write: {e}"))
}

// ── Window math ────────────────────────────────────────────────────────────

/// A local wall-clock time as an instant in `tz`: a time inside a DST gap
/// moves forward to the first minute that exists; a repeated time takes the
/// first of the two.
pub(crate) fn resolve_local<Tz: TimeZone>(tz: &Tz, naive: NaiveDateTime) -> DateTime<Utc> {
    let mut at = naive;
    for _ in 0..=180 {
        match tz.from_local_datetime(&at) {
            LocalResult::Single(t) => return t.with_timezone(&Utc),
            LocalResult::Ambiguous(first, _) => return first.with_timezone(&Utc),
            LocalResult::None => at += Duration::minutes(1),
        }
    }
    // No zone has a gap of three hours; treat the wall time as UTC.
    Utc.from_utc_datetime(&naive)
}

/// One occurrence of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Occurrence {
    /// The `period.rs` key of its start: a date, or an ISO week.
    pub key: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

fn occurrence_on<Tz: TimeZone>(w: &WindowDef, tz: &Tz, date: NaiveDate) -> Option<Occurrence> {
    if !w.weekdays().contains(&date.weekday()) {
        return None;
    }
    let (start_t, end_t) = (parse_hm(&w.start)?, parse_hm(&w.end)?);
    let end_date = if end_t > start_t {
        date
    } else {
        date.succ_opt()?
    };
    let start_naive = date.and_time(start_t);
    let key = period::period_key(start_naive, w.period())?;
    Some(Occurrence {
        key,
        start: resolve_local(tz, start_naive),
        end: resolve_local(tz, end_date.and_time(end_t)),
    })
}

/// Occurrences starting in `[from, to]`, oldest first.
pub(crate) fn occurrences<Tz: TimeZone>(
    w: &WindowDef,
    tz: &Tz,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<Occurrence> {
    let first = from.with_timezone(tz).date_naive() - Duration::days(1);
    let last = to.with_timezone(tz).date_naive();
    let mut out = Vec::new();
    let mut date = first;
    while date <= last {
        if let Some(o) = occurrence_on(w, tz, date) {
            if o.start >= from && o.start <= to {
                out.push(o);
            }
        }
        match date.succ_opt() {
            Some(next) => date = next,
            None => break,
        }
    }
    out
}

/// The first start strictly after `after`, within five weeks.
pub(crate) fn next_start<Tz: TimeZone>(
    w: &WindowDef,
    tz: &Tz,
    after: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    occurrences(w, tz, after, after + Duration::days(35))
        .into_iter()
        .find(|o| o.start > after)
        .map(|o| o.start)
}

/// The occurrence `now` falls inside, if any.
pub(crate) fn current<Tz: TimeZone>(
    w: &WindowDef,
    tz: &Tz,
    now: DateTime<Utc>,
) -> Option<Occurrence> {
    occurrences(w, tz, now - Duration::days(2), now)
        .into_iter()
        .rev()
        .find(|o| o.start <= now && now < o.end)
}

/// The window an automation schedule names (`window:<name>`).
pub(crate) fn schedule_window(schedule: &str) -> Option<&str> {
    schedule
        .trim()
        .strip_prefix(WINDOW_PREFIX)
        .map(|rest| rest.split('/').next().unwrap_or("").trim())
        .filter(|name| !name.is_empty())
}

/// The catch-up a window schedule names after its `/` (`window:nightly/run-all`):
/// `Ok(None)` for none (the default), `Err` for an unknown one.
fn schedule_policy(schedule: &str) -> Result<Option<CatchUp>, String> {
    let rest = schedule.trim().strip_prefix(WINDOW_PREFIX).unwrap_or("");
    match rest.split_once('/') {
        None => Ok(None),
        Some((_, raw)) => CatchUp::parse(raw).map(Some).ok_or_else(|| {
            format!(
                "Catch-up '{}' is not skip, run-once-on-wake or run-all.",
                raw.trim()
            )
        }),
    }
}

/// Whether `schedule` names a window the settings define.
pub(crate) fn validate_window_schedule(schedule: &str) -> Result<(), String> {
    let name = schedule_window(schedule).ok_or("Not a window schedule.")?;
    schedule_policy(schedule)?;
    let s = settings();
    if s.window(name).is_some() {
        Ok(())
    } else {
        Err(format!(
            "Unknown window '{name}'. Known: {}.",
            s.windows
                .iter()
                .map(|w| w.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }
}

/// The next start of the window an automation schedule names, after `now`,
/// in the Mac's zone.
pub(crate) fn next_window_run(schedule: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let name = schedule_window(schedule)?;
    let s = settings();
    next_start(s.window(name)?, &chrono::Local, now)
}

// ── Catch-up ───────────────────────────────────────────────────────────────

/// What happens to occurrences missed while Hollow was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatchUp {
    /// Past its grace, a missed occurrence is skipped (logged).
    Skip,
    /// One run on wake, however many were missed.
    RunOnceOnWake,
    /// Each missed occurrence, oldest first, at most [`RUN_ALL_MAX`].
    RunAll,
}

impl CatchUp {
    /// The frontmatter value; anything else is `None` (the default applies).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "skip" => Some(CatchUp::Skip),
            "run-once-on-wake" | "run-once" | "once" => Some(CatchUp::RunOnceOnWake),
            "run-all" | "all" => Some(CatchUp::RunAll),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CatchUp::Skip => "skip",
            CatchUp::RunOnceOnWake => "run-once-on-wake",
            CatchUp::RunAll => "run-all",
        }
    }
}

/// The policy an automation runs under. It lives in the stamped schedule
/// string (`window:<name>[/<policy>]`), so a run can't change it without the
/// permission check noticing: run-once-on-wake by default for a window, and
/// `None` (the misfire grace, unchanged) for a cron.
pub(crate) fn effective_catch_up(schedule: &str) -> Option<CatchUp> {
    schedule_window(schedule)?;
    Some(
        schedule_policy(schedule)
            .ok()
            .flatten()
            .unwrap_or(CatchUp::RunOnceOnWake),
    )
}

/// Where `next_run_at` goes after a run of occurrence `due` finished at
/// `now`. `occurrences_after(t)` yields the schedule's starts after `t`, in
/// order. Under `run-all` the next missed occurrence comes next (with the one
/// that ran, at most [`RUN_ALL_MAX`] runs); otherwise the first start after
/// `now`.
pub(crate) fn next_after_run(
    policy: Option<CatchUp>,
    due: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    occurrences_after: &dyn Fn(DateTime<Utc>) -> Vec<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    if let (Some(CatchUp::RunAll), Some(due)) = (policy, due) {
        let missed: Vec<DateTime<Utc>> = occurrences_after(due)
            .into_iter()
            .take_while(|t| *t <= now)
            .collect();
        // The due occurrence just ran: at most RUN_ALL_MAX - 1 more.
        if let Some(first_kept) = missed.get(missed.len().saturating_sub(RUN_ALL_MAX - 1)) {
            return Some(*first_kept);
        }
    }
    occurrences_after(now).into_iter().find(|t| *t > now)
}

// ── Window jobs ────────────────────────────────────────────────────────────

/// A window job's verdict at one tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JobDecision {
    Idle,
    /// Run once and stamp `key`; `late` when its window already ended.
    Run {
        key: String,
        late: bool,
    },
    /// Stamp `key` without running: a missed occurrence under `skip`, or one
    /// that ended before the job was first seen.
    Stamp {
        key: String,
    },
}

/// Should a window job run now? `last_key` is the occurrence it last
/// stamped (`None`: never seen). See the module doc.
pub(crate) fn decide_job<Tz: TimeZone>(
    w: &WindowDef,
    tz: &Tz,
    now: DateTime<Utc>,
    policy: CatchUp,
    last_key: Option<&str>,
) -> JobDecision {
    let mut pending: Vec<Occurrence> = Vec::new();
    for o in occurrences(w, tz, now - Duration::days(LOOKBACK_DAYS), now) {
        // Keys compare only within one shape: a window edited from one day a
        // week (ISO week keys) to several (date keys) starts afresh rather
        // than reading "2026-10-16" <= "2026-W41" as done.
        let done = last_key.is_some_and(|last| {
            last == o.key || (last.contains("-W") == o.key.contains("-W") && o.key.as_str() <= last)
        });
        if !done && pending.last().is_none_or(|p| p.key != o.key) {
            pending.push(o);
        }
    }
    let Some(latest) = pending.last().cloned() else {
        return JobDecision::Idle;
    };
    if policy == CatchUp::RunAll && last_key.is_some() {
        let oldest = &pending[pending.len().saturating_sub(RUN_ALL_MAX)];
        return JobDecision::Run {
            key: oldest.key.clone(),
            late: oldest.end <= now,
        };
    }
    if latest.start <= now && now < latest.end {
        return JobDecision::Run {
            key: latest.key,
            late: false,
        };
    }
    if last_key.is_none() {
        return JobDecision::Stamp { key: latest.key };
    }
    match policy {
        CatchUp::Skip => JobDecision::Stamp { key: latest.key },
        CatchUp::RunOnceOnWake | CatchUp::RunAll => JobDecision::Run {
            key: latest.key,
            late: true,
        },
    }
}

/// Each window job's last stamped occurrence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct JobState {
    pub last: BTreeMap<String, String>,
    /// When each job last ran or stamped (RFC 3339), for the person.
    pub at: BTreeMap<String, String>,
}

fn jobs_path() -> Option<PathBuf> {
    state_root().map(|root| root.join("jobs.json"))
}

pub(crate) fn load_jobs() -> JobState {
    jobs_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(crate) fn stamp_job(job: &str, key: &str, now: DateTime<Utc>) {
    let mut state = load_jobs();
    state.last.insert(job.into(), key.into());
    state.at.insert(job.into(), now.to_rfc3339());
    if let Some(path) = jobs_path() {
        if let Err(e) = write_json(&path, &state) {
            eprintln!("[scheduler] {e}");
        }
    }
}

// ── The timer ──────────────────────────────────────────────────────────────

/// A host job that runs once per occurrence of a window (see [`decide_job`]).
pub(crate) struct WindowJob {
    pub id: &'static str,
    pub window: &'static str,
    pub catch_up: CatchUp,
    /// The occurrence key and whether it is late; `Ok` stamps the occurrence.
    pub run: fn(
        tauri::AppHandle,
        crate::config::Config,
        String,
        bool,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>,
}

/// Hollow's window jobs.
pub(crate) const JOBS: &[WindowJob] = &[
    // #4: the Friday weekly update (decision 2: drafted Friday 15:00).
    WindowJob {
        id: "weekly-update",
        window: "friday-pm",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::digest::weekly_job,
    },
    // A18: each tracked project's Friday status, written into Hollow (and
    // optionally copied to the tracker). Does nothing while
    // `tracker.fridayStatus` is off (the default).
    WindowJob {
        id: "friday-status",
        window: "friday-pm",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::tracker::status::friday_job,
    },
    // #19: the gardener scans nightly and keeps one review card; it never
    // applies anything on its own (decision 5).
    WindowJob {
        id: "gardener",
        window: "nightly",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::gardener::nightly_job,
    },
    // F5: the nightly as-of snapshot and per-workspace metrics.
    WindowJob {
        id: "as-of-snapshot",
        window: "nightly",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::asof::nightly_job,
    },
    // #13: recount the friction patterns nightly; it never starts a draft.
    WindowJob {
        id: "friction",
        window: "nightly",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::friction::nightly_job,
    },
    // CX8b: the digest of org changes merged as the owner (a note and an item).
    WindowJob {
        id: "org-digest",
        window: "nightly",
        catch_up: CatchUp::RunOnceOnWake,
        run: crate::context_org_digest::nightly_job,
    },
];

async fn run_jobs(app: &tauri::AppHandle, config: &crate::config::Config, now: DateTime<Utc>) {
    let s = settings();
    for job in JOBS {
        let Some(w) = s.window(job.window) else {
            continue;
        };
        let state = load_jobs();
        let decision = decide_job(
            w,
            &chrono::Local,
            now,
            job.catch_up,
            state.last.get(job.id).map(String::as_str),
        );
        match decision {
            JobDecision::Idle => {}
            JobDecision::Stamp { key } => stamp_job(job.id, &key, now),
            JobDecision::Run { key, late } => {
                match (job.run)(app.clone(), config.clone(), key.clone(), late).await {
                    Ok(()) => stamp_job(job.id, &key, now),
                    Err(e) => eprintln!("[scheduler] {} for {key}: {e}", job.id),
                }
            }
        }
    }
}

/// The minute tick: the window jobs (the night queue drain went 11-10, S6). Queue
/// owner only, and never in a candidate build unless asked (`lib.rs`).
pub(crate) fn start_timer(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            use tauri::Manager;
            let Some(config) = app
                .try_state::<crate::commands::AppState>()
                .map(|s| s.config.clone())
            else {
                continue;
            };
            run_jobs(&app, &config, Utc::now()).await;
            // P1c: paused chats whose resume time has come (a time missed
            // while Hollow was closed comes due at the first tick after
            // launch), each through Resume's own claim.
            crate::commands::resume_due_paused(Some(&app), &config).await;
        }
    });
}

// ── Commands ───────────────────────────────────────────────────────────────

/// One window as the person and agents see it, with its next start.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct WindowView {
    pub name: String,
    pub label: String,
    pub days: Vec<String>,
    pub start: String,
    pub end: String,
    /// `day`, `weekday` or `week`: what one occurrence counts as.
    pub period: String,
    /// Whether an occurrence is open now.
    pub open_now: bool,
    #[cfg_attr(test, ts(optional))]
    pub next_start: Option<String>,
    /// Next three strictly future starts, resolved by the host's window rules.
    #[cfg_attr(test, ts(optional))]
    pub next_starts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AutomationWindows {
    pub windows: Vec<WindowView>,
    pub overnight_cap_usd: f64,
    /// The zone windows are read in.
    pub time_zone: String,
    /// Current or next configured Tonight occurrence and its pending host jobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub night_plan: Option<NightPlan>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NightPlan {
    pub start: String,
    pub end: String,
    pub jobs: Vec<NightJobView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct NightJobView {
    pub id: String,
    pub label: String,
    pub window: String,
    pub at: String,
}

fn night_plan<Tz: TimeZone>(
    settings: &WindowSettings,
    tz: &Tz,
    now: DateTime<Utc>,
    state: &JobState,
) -> Option<NightPlan> {
    let tonight = settings.window("tonight")?;
    let night = current(tonight, tz, now).or_else(|| {
        occurrences(tonight, tz, now, now + Duration::days(35))
            .into_iter()
            .find(|o| o.start > now)
    })?;
    let jobs = JOBS
        .iter()
        .filter_map(|job| {
            let window = settings.window(job.window)?;
            let due = occurrences(window, tz, night.start, night.end)
                .into_iter()
                .find(|o| {
                    o.start < night.end
                        && !state.last.get(job.id).is_some_and(|last| {
                            last == &o.key
                                || (last.contains("-W") == o.key.contains("-W") && last > &o.key)
                        })
                })?;
            let label = match job.id {
                "weekly-update" => "Weekly update draft",
                "friday-status" => "Friday status",
                "gardener" => "Cleanup review",
                "as-of-snapshot" => "Workspace snapshot",
                "friction" => "Friction review",
                "org-digest" => "Org changes digest",
                other => other,
            };
            Some(NightJobView {
                id: job.id.into(),
                label: label.into(),
                window: job.window.into(),
                at: due.start.to_rfc3339(),
            })
        })
        .collect();
    Some(NightPlan {
        start: night.start.to_rfc3339(),
        end: night.end.to_rfc3339(),
        jobs,
    })
}

/// This Mac's zone name (`America/New_York`), from `/etc/localtime`.
fn mac_zone() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let p = p.to_string_lossy().to_string();
            p.split_once("zoneinfo/").map(|(_, zone)| zone.to_string())
        })
        .unwrap_or_else(|| "this Mac's zone".into())
}

pub(crate) fn view(s: &WindowSettings, now: DateTime<Utc>, jobs: &JobState) -> AutomationWindows {
    let tz = chrono::Local;
    AutomationWindows {
        windows: s
            .windows
            .iter()
            .map(|w| WindowView {
                name: w.name.clone(),
                label: w.label.clone(),
                days: w.days.clone(),
                start: w.start.clone(),
                end: w.end.clone(),
                period: w.period().as_str().into(),
                open_now: current(w, &tz, now).is_some(),
                next_start: next_start(w, &tz, now).map(|t| t.to_rfc3339()),
                next_starts: Some({
                    let mut after = now;
                    let mut starts = Vec::new();
                    for _ in 0..3 {
                        let Some(next) = next_start(w, &tz, after) else {
                            break;
                        };
                        starts.push(next.to_rfc3339());
                        after = next;
                    }
                    starts
                }),
            })
            .collect(),
        overnight_cap_usd: s.overnight_cap_usd,
        time_zone: mac_zone(),
        night_plan: night_plan(s, &tz, now, jobs),
    }
}

/// The named windows, the overnight cap and the zone they are read in.
#[tauri::command]
pub fn automation_windows() -> AutomationWindows {
    view(&settings(), Utc::now(), &load_jobs())
}

/// Save the windows and the overnight cap (windows are on the Automations page, the cap in Settings, Usage, Spending caps, Overnight cap). The COO's
/// route is [`agent_set_windows`], behind a confirm card.
#[tauri::command]
pub fn set_automation_windows(settings: WindowSettings) -> Result<AutomationWindows, String> {
    save_settings(settings)
}

fn save_settings(settings: WindowSettings) -> Result<AutomationWindows, String> {
    let settings = settings.validated()?;
    let path = settings_path().ok_or("No app support folder.")?;
    write_json(&path, &settings)?;
    Ok(view(&settings, Utc::now(), &load_jobs()))
}

/// `base` with the windows and cap a call names (one left out keeps its
/// value), validated as Settings validates a save.
pub(crate) fn merged_settings(
    base: WindowSettings,
    windows: Option<Vec<WindowDef>>,
    overnight_cap_usd: Option<f64>,
) -> Result<WindowSettings, String> {
    WindowSettings {
        windows: windows.unwrap_or(base.windows),
        overnight_cap_usd: overnight_cap_usd.unwrap_or(base.overnight_cap_usd),
    }
    .validated()
}

/// The windows and cap an action's arguments name (`windows`: a list of
/// definitions, `overnightCapUsd`: dollars).
pub(crate) fn settings_args(
    args: &serde_json::Value,
) -> Result<(Option<Vec<WindowDef>>, Option<f64>), String> {
    let windows = match args.get("windows") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            serde_json::from_value::<Vec<WindowDef>>(value.clone()).map_err(|e| {
                format!("`windows` doesn't fit: {e}. Each needs name, label, days, start, end. Nothing changed.")
            })?,
        ),
    };
    let cap = match args.get("overnightCapUsd") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .ok_or("`overnightCapUsd` must be a number. Nothing changed.")?,
        ),
    };
    Ok((windows, cap))
}

/// What changing `old` to `new` shows on a card: windows added, removed or
/// redefined, and the overnight cap, then a line when the cap rises.
pub(crate) fn settings_diff(old: &WindowSettings, new: &WindowSettings) -> Vec<String> {
    fn shown(w: &WindowDef) -> String {
        format!("{} {} {} to {}", w.name, w.days.join(","), w.start, w.end)
    }
    let mut lines = Vec::new();
    for w in &new.windows {
        match old.windows.iter().find(|o| o.name == w.name) {
            None => lines.push(format!("Add window: {}", shown(w))),
            Some(o) if o != w => {
                lines.push(format!("Change window: {} → {}", shown(o), shown(w)));
            }
            Some(_) => {}
        }
    }
    for o in &old.windows {
        if !new.windows.iter().any(|w| w.name == o.name) {
            lines.push(format!(
                "Remove window: {} (automations on it stop running)",
                shown(o)
            ));
        }
    }
    if old.overnight_cap_usd != new.overnight_cap_usd {
        lines.push(format!(
            "Overnight cap: ${} → ${}",
            old.overnight_cap_usd, new.overnight_cap_usd
        ));
        if new.overnight_cap_usd > old.overnight_cap_usd {
            lines.push("A higher cap lets the night queue spend more in one night.".into());
        }
    }
    lines
}

/// `automations.set_automation_windows`' card.
pub(crate) fn windows_card(
    _cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let old = settings();
    let (windows, cap) = settings_args(&serde_json::Value::Object(a.0.clone()))?;
    let new = merged_settings(old.clone(), windows, cap)?;
    let lines = settings_diff(&old, &new);
    if lines.is_empty() {
        return Err("Those are already the schedule windows. Nothing changed.".into());
    }
    let mut card = crate::app_actions::Card::new("Hollow app tools: change the schedule windows");
    for line in lines {
        card = card.line(line);
    }
    Ok(Some(card))
}

/// `automations.set_automation_windows`: save the merged settings (after
/// the card) and return the same view Settings shows.
pub(crate) fn agent_set_windows(args: &serde_json::Value) -> Result<AutomationWindows, String> {
    let (windows, cap) = settings_args(args)?;
    save_settings(merged_settings(settings(), windows, cap)?)
}

/// The JSON schema of the `windows` argument, for agents.
pub(crate) fn windows_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "description": "The complete list of named windows (replaces the current list; read automation_windows first).",
        "items": {
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Lowercase letters, digits and hyphens: what an automation's `window:<name>` uses." },
                "label": { "type": "string", "description": "What the person sees." },
                "days": { "type": "array", "items": { "type": "string", "enum": ["mon", "tue", "wed", "thu", "fri", "sat", "sun"] } },
                "start": { "type": "string", "description": "HH:MM, local time." },
                "end": { "type": "string", "description": "HH:MM; at or before start means the next day." }
            },
            "required": ["name", "label", "days", "start", "end"],
            "additionalProperties": false
        }
    })
}

/// Put an automation on a window (`window:<name>`), or back to `manual`
/// with `window: "none"`, and optionally set its catch-up. The schedule is a
/// stamped field: this is the owned `set_schedule` write, so an approved
/// automation stays approved.
#[tauri::command]
pub async fn set_automation_window(
    state: tauri::State<'_, crate::commands::AppState>,
    slug: String,
    window: String,
    catch_up: Option<String>,
) -> Result<crate::types::Workflow, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        apply_window(&vault, &slug, &window, catch_up.as_deref())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The schedule string a window choice means (`window:<name>`, with
/// `/<policy>` unless it is the default run-once-on-wake).
pub(crate) fn window_schedule(window: &str, catch_up: Option<&str>) -> Result<String, String> {
    let name = window.trim();
    if matches!(name, "none" | "manual" | "") {
        return Ok("manual".into());
    }
    let policy = match catch_up.map(str::trim).filter(|c| !c.is_empty()) {
        None => None,
        Some(raw) => Some(CatchUp::parse(raw).ok_or_else(|| {
            format!("Catch-up '{raw}' is not skip, run-once-on-wake or run-all. Nothing changed.")
        })?),
    };
    let schedule = match policy {
        None | Some(CatchUp::RunOnceOnWake) => format!("{WINDOW_PREFIX}{name}"),
        Some(p) => format!("{WINDOW_PREFIX}{name}/{}", p.as_str()),
    };
    validate_window_schedule(&schedule)?;
    Ok(schedule)
}

pub(crate) fn apply_window(
    vault: &std::path::Path,
    slug: &str,
    window: &str,
    catch_up: Option<&str>,
) -> Result<crate::types::Workflow, String> {
    let schedule = window_schedule(window, catch_up)?;
    crate::vault_workflows::read_full(vault, slug)?;
    crate::vault_workflows::set_schedule(vault, slug, &schedule)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::FixedOffset;

    /// US Eastern with its real DST rules (second Sunday of March 02:00 →
    /// first Sunday of November 02:00), so DST is tested without a zone
    /// database or a process-wide `TZ`.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct Eastern;

    fn nth_sunday(year: i32, month: u32, n: u32) -> NaiveDate {
        let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let offset = (7 - first.weekday().num_days_from_sunday()) % 7;
        first + Duration::days(i64::from(offset + 7 * (n - 1)))
    }

    fn edt() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }
    fn est() -> FixedOffset {
        FixedOffset::west_opt(5 * 3600).unwrap()
    }

    impl TimeZone for Eastern {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Eastern
        }
        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<FixedOffset> {
            let y = local.date().year();
            let spring = nth_sunday(y, 3, 2).and_hms_opt(2, 0, 0).unwrap();
            let fall = nth_sunday(y, 11, 1).and_hms_opt(1, 0, 0).unwrap();
            if *local >= spring && *local < spring + Duration::hours(1) {
                LocalResult::None
            } else if *local >= fall && *local < fall + Duration::hours(1) {
                LocalResult::Ambiguous(edt(), est())
            } else if *local >= spring && *local < fall {
                LocalResult::Single(edt())
            } else {
                LocalResult::Single(est())
            }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            let y = utc.date().year();
            let spring = nth_sunday(y, 3, 2).and_hms_opt(7, 0, 0).unwrap();
            let fall = nth_sunday(y, 11, 1).and_hms_opt(6, 0, 0).unwrap();
            if *utc >= spring && *utc < fall {
                edt()
            } else {
                est()
            }
        }
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        resolve_local(
            &Eastern,
            NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .and_hms_opt(h, min, 0)
                .unwrap(),
        )
    }

    fn named(name: &str) -> WindowDef {
        WindowSettings::default().window(name).unwrap().clone()
    }

    #[test]
    fn the_test_zone_is_eastern() {
        assert_eq!(
            Eastern.offset_from_utc_datetime(&local(2026, 1, 5, 12, 0).naive_utc()),
            est()
        );
        assert_eq!(
            local(2026, 7, 1, 12, 0).to_rfc3339(),
            "2026-07-01T16:00:00+00:00"
        );
        assert_eq!(
            local(2026, 1, 1, 12, 0).to_rfc3339(),
            "2026-01-01T17:00:00+00:00"
        );
    }

    #[test]
    fn defaults_validate_and_have_the_decided_hours() {
        let s = WindowSettings::default().validated().unwrap();
        assert_eq!(s.overnight_cap_usd, 40.0);
        let tonight = s.window("tonight").unwrap();
        assert_eq!(
            (tonight.start.as_str(), tonight.end.as_str()),
            ("18:00", "07:00")
        );
        assert_eq!(tonight.period(), Period::Weekday);
        assert_eq!(s.window("friday-pm").unwrap().period(), Period::Week);
        assert_eq!(s.window("nightly").unwrap().period(), Period::Day);
        assert!(s.window("nope").is_none());
    }

    #[test]
    fn automation_window_view_names_tonights_interval_and_host_jobs() {
        let data = serde_json::to_value(view(
            &WindowSettings::default(),
            local(2026, 10, 23, 17, 0),
            &JobState::default(),
        ))
        .unwrap();
        let night = &data["nightPlan"];
        assert!(night["start"].as_str().is_some(), "{data}");
        assert!(night["end"].as_str().is_some(), "{data}");
        let plan = night_plan(
            &WindowSettings::default(),
            &Eastern,
            local(2026, 10, 23, 17, 0),
            &JobState::default(),
        )
        .unwrap();
        let ids: Vec<_> = plan.jobs.iter().map(|j| j.id.as_str()).collect();
        assert_eq!(
            ids,
            ["gardener", "as-of-snapshot", "friction", "org-digest"]
        );
    }

    #[test]
    fn tonight_plan_retains_current_night_and_excludes_stamped_jobs() {
        let s = WindowSettings::default();
        let now = local(2026, 10, 24, 3, 0);
        let mut jobs = JobState::default();
        jobs.last.insert("gardener".into(), "2026-10-24".into());
        let plan = night_plan(&s, &Eastern, now, &jobs).unwrap();
        assert_eq!(
            DateTime::parse_from_rfc3339(&plan.start)
                .unwrap()
                .with_timezone(&Utc),
            local(2026, 10, 23, 18, 0)
        );
        assert!(!plan.jobs.iter().any(|j| j.id == "gardener"));
        assert!(plan.jobs.iter().any(|j| j.id == "friction"));
    }

    #[test]
    fn tonight_plan_uses_real_dst_boundaries_and_missing_is_unknown() {
        let mut s = WindowSettings::default();
        s.windows
            .iter_mut()
            .find(|w| w.name == "tonight")
            .unwrap()
            .days = EVERY_DAY.iter().map(|s| s.to_string()).collect();
        let plan = night_plan(
            &s,
            &Eastern,
            local(2026, 10, 31, 23, 0),
            &JobState::default(),
        )
        .unwrap();
        let start = DateTime::parse_from_rfc3339(&plan.start).unwrap();
        let end = DateTime::parse_from_rfc3339(&plan.end).unwrap();
        assert_eq!((end - start).num_hours(), 14);
        s.windows.retain(|w| w.name != "tonight");
        assert!(night_plan(
            &s,
            &Eastern,
            local(2026, 10, 31, 23, 0),
            &JobState::default()
        )
        .is_none());
    }

    #[test]
    fn tonight_plan_keeps_older_or_different_kind_stamps_pending() {
        let mut s = WindowSettings::default();
        s.windows
            .iter_mut()
            .find(|w| w.name == "tonight")
            .unwrap()
            .start = "14:00".into();
        let now = local(2026, 10, 23, 14, 30);
        for (stamp, pending) in [
            ("2026-W42", true),
            ("2026-W43", false),
            ("2026-W44", false),
            ("2026-10-24", true),
        ] {
            let mut state = JobState::default();
            state.last.insert("weekly-update".into(), stamp.into());
            let plan = night_plan(&s, &Eastern, now, &state).unwrap();
            assert_eq!(
                plan.jobs.iter().any(|j| j.id == "weekly-update"),
                pending,
                "{stamp}"
            );
        }
        for (stamp, pending) in [
            ("2026-10-23", true),
            ("2026-10-24", false),
            ("2026-10-25", false),
            ("2026-W99", true),
        ] {
            let mut state = JobState::default();
            state.last.insert("gardener".into(), stamp.into());
            let plan = night_plan(&s, &Eastern, now, &state).unwrap();
            assert_eq!(
                plan.jobs.iter().any(|j| j.id == "gardener"),
                pending,
                "{stamp}"
            );
        }
        let next = night_plan(
            &WindowSettings::default(),
            &Eastern,
            local(2026, 10, 24, 14, 0),
            &JobState::default(),
        )
        .unwrap();
        assert_eq!(
            DateTime::parse_from_rfc3339(&next.start)
                .unwrap()
                .with_timezone(&Utc),
            local(2026, 10, 26, 18, 0)
        );
    }

    #[test]
    fn bad_settings_are_refused() {
        let mut s = WindowSettings::default();
        s.windows[0].start = "25:00".into();
        assert!(s.validated().unwrap_err().contains("HH:MM"));
        let mut s = WindowSettings::default();
        s.windows.push(s.windows[0].clone());
        assert!(s.validated().unwrap_err().contains("twice"));
        let s = WindowSettings {
            overnight_cap_usd: f64::NAN,
            ..WindowSettings::default()
        };
        assert!(s.validated().is_err());
    }

    #[test]
    fn window_view_supplies_three_authoritative_starts() {
        let now = local(2026, 10, 30, 12, 0);
        let data =
            serde_json::to_value(view(&WindowSettings::default(), now, &JobState::default()))
                .unwrap();
        for window in data["windows"].as_array().unwrap() {
            let starts = window["nextStarts"]
                .as_array()
                .expect("three starts for the editor");
            assert_eq!(starts.len(), 3);
            assert_eq!(window["nextStart"], starts[0]);
            let times: Vec<_> = starts
                .iter()
                .map(|value| DateTime::parse_from_rfc3339(value.as_str().unwrap()).unwrap())
                .collect();
            assert!(times[0] > now);
            assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
            for time in times {
                let wall = time.with_timezone(&chrono::Local);
                assert_eq!(
                    wall.format("%H:%M").to_string(),
                    window["start"].as_str().unwrap()
                );
                let day = wall.format("%a").to_string().to_lowercase();
                assert!(window["days"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value.as_str() == Some(day.as_str())));
            }
        }
    }

    #[test]
    fn friday_pm_starts_at_15_local_on_both_sides_of_dst() {
        let w = named("friday-pm");
        // Thursday 2026-10-08 noon EDT → Friday 10-09 15:00 EDT = 19:00 UTC.
        let next = next_start(&w, &Eastern, local(2026, 10, 8, 12, 0)).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-10-09T19:00:00+00:00");
        // After DST ends (Nov 1): Friday 11-06 15:00 EST = 20:00 UTC.
        let next = next_start(&w, &Eastern, local(2026, 11, 2, 12, 0)).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-11-06T20:00:00+00:00");
        // The key is the ISO week.
        let o = current(&w, &Eastern, local(2026, 10, 9, 16, 0)).unwrap();
        assert_eq!(o.key, "2026-W41");
        assert!(current(&w, &Eastern, local(2026, 10, 9, 18, 0)).is_none());
    }

    #[test]
    fn nightly_inside_the_spring_gap_starts_at_the_first_real_minute() {
        // 2026-03-08 02:00 doesn't exist in Eastern: 03:00 EDT = 07:00 UTC.
        let w = named("nightly");
        let next = next_start(&w, &Eastern, local(2026, 3, 7, 12, 0)).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-03-08T07:00:00+00:00");
        // The night before: 02:00 EST = 07:00 UTC; the night after: 06:00 UTC.
        let day_before = next_start(&w, &Eastern, local(2026, 3, 6, 12, 0)).unwrap();
        assert_eq!(day_before.to_rfc3339(), "2026-03-07T07:00:00+00:00");
        let day_after = next_start(&w, &Eastern, next).unwrap();
        assert_eq!(day_after.to_rfc3339(), "2026-03-09T06:00:00+00:00");
    }

    #[test]
    fn a_repeated_autumn_hour_starts_once_at_the_first() {
        let w = def("late", "Late", EVERY_DAY, "01:30", "03:00");
        // 2026-11-01 01:30 happens twice; the first is 05:30 UTC (EDT).
        let next = next_start(&w, &Eastern, local(2026, 10, 31, 12, 0)).unwrap();
        assert_eq!(next.to_rfc3339(), "2026-11-01T05:30:00+00:00");
        let after = next_start(&w, &Eastern, next).unwrap();
        assert_eq!(after.to_rfc3339(), "2026-11-02T06:30:00+00:00");
    }

    #[test]
    fn tonight_crosses_midnight_and_skips_weekend_starts() {
        let w = named("tonight");
        // Tuesday 23:00 and Wednesday 06:59 are both inside Tuesday's night.
        let tue = current(&w, &Eastern, local(2026, 10, 6, 23, 0)).unwrap();
        assert_eq!(tue.key, "2026-10-06");
        let early = current(&w, &Eastern, local(2026, 10, 7, 6, 59)).unwrap();
        assert_eq!(early.key, "2026-10-06");
        assert!(current(&w, &Eastern, local(2026, 10, 7, 7, 0)).is_none());
        // Friday night runs into Saturday; Saturday night has no window.
        assert!(current(&w, &Eastern, local(2026, 10, 10, 2, 0)).is_some());
        assert!(current(&w, &Eastern, local(2026, 10, 10, 20, 0)).is_none());
        // Saturday noon → Monday 18:00.
        let next = next_start(&w, &Eastern, local(2026, 10, 10, 12, 0)).unwrap();
        assert_eq!(next, local(2026, 10, 12, 18, 0));
    }

    #[test]
    fn closed_through_a_friday_window_catches_up_exactly_once() {
        let w = named("friday-pm");
        let job =
            |now, last: Option<&str>| decide_job(&w, &Eastern, now, CatchUp::RunOnceOnWake, last);
        // Last ran the week before; Hollow is closed all Friday 10-09 and
        // opens Saturday 09:00.
        let open = local(2026, 10, 10, 9, 0);
        assert_eq!(
            job(open, Some("2026-W40")),
            JobDecision::Run {
                key: "2026-W41".into(),
                late: true
            }
        );
        // Stamped, every later tick that week is idle.
        assert_eq!(job(open, Some("2026-W41")), JobDecision::Idle);
        assert_eq!(
            job(local(2026, 10, 11, 20, 0), Some("2026-W41")),
            JobDecision::Idle
        );
        // Missed three Fridays: still one run, for the latest.
        assert_eq!(
            job(open, Some("2026-W38")),
            JobDecision::Run {
                key: "2026-W41".into(),
                late: true
            }
        );
        // Inside the window it runs on time.
        assert_eq!(
            job(local(2026, 10, 9, 15, 1), Some("2026-W40")),
            JobDecision::Run {
                key: "2026-W41".into(),
                late: false
            }
        );
    }

    #[test]
    fn skip_stamps_a_missed_occurrence_and_run_all_runs_each() {
        let w = named("nightly");
        let open = local(2026, 10, 9, 10, 0);
        assert_eq!(
            decide_job(&w, &Eastern, open, CatchUp::Skip, Some("2026-10-07")),
            JobDecision::Stamp {
                key: "2026-10-09".into()
            }
        );
        // run-all: 10-08 and 10-09 were missed; the older runs first.
        assert_eq!(
            decide_job(&w, &Eastern, open, CatchUp::RunAll, Some("2026-10-07")),
            JobDecision::Run {
                key: "2026-10-08".into(),
                late: true
            }
        );
        assert_eq!(
            decide_job(&w, &Eastern, open, CatchUp::RunAll, Some("2026-10-08")),
            JobDecision::Run {
                key: "2026-10-09".into(),
                late: true
            }
        );
        // run-all after a long gap keeps only the last RUN_ALL_MAX.
        assert_eq!(
            decide_job(&w, &Eastern, open, CatchUp::RunAll, Some("2026-09-01")),
            JobDecision::Run {
                key: "2026-10-05".into(),
                late: true
            }
        );
    }

    #[test]
    fn a_changed_key_shape_is_not_read_as_done() {
        // friday-pm edited to two days: its keys become dates.
        let w = def("friday-pm", "Fri", &["thu", "fri"], "15:00", "18:00");
        assert_eq!(
            decide_job(
                &w,
                &Eastern,
                local(2026, 10, 9, 15, 30),
                CatchUp::RunOnceOnWake,
                Some("2026-W41")
            ),
            JobDecision::Run {
                key: "2026-10-09".into(),
                late: false
            }
        );
    }

    #[test]
    fn a_job_seen_for_the_first_time_never_runs_an_old_occurrence() {
        let w = named("friday-pm");
        // Installed on a Thursday: last Friday is stamped, not drafted.
        assert_eq!(
            decide_job(
                &w,
                &Eastern,
                local(2026, 10, 8, 12, 0),
                CatchUp::RunOnceOnWake,
                None
            ),
            JobDecision::Stamp {
                key: "2026-W40".into()
            }
        );
        // Installed during the window: it runs.
        assert_eq!(
            decide_job(
                &w,
                &Eastern,
                local(2026, 10, 9, 16, 0),
                CatchUp::RunOnceOnWake,
                None
            ),
            JobDecision::Run {
                key: "2026-W41".into(),
                late: false
            }
        );
    }

    #[test]
    fn catch_up_parses_and_lives_in_the_schedule_string() {
        assert_eq!(CatchUp::parse("run-all"), Some(CatchUp::RunAll));
        assert_eq!(CatchUp::parse(" Skip "), Some(CatchUp::Skip));
        assert_eq!(CatchUp::parse("sometimes"), None);
        assert_eq!(
            effective_catch_up("window:friday-pm"),
            Some(CatchUp::RunOnceOnWake)
        );
        assert_eq!(
            effective_catch_up("window:friday-pm/run-all"),
            Some(CatchUp::RunAll)
        );
        assert_eq!(effective_catch_up("0 0 9 * * * *"), None);
        assert_eq!(schedule_window("window: nightly "), Some("nightly"));
        assert_eq!(schedule_window("window:nightly/skip"), Some("nightly"));
        assert_eq!(schedule_window("window:"), None);
        assert_eq!(schedule_window("manual"), None);
        assert!(validate_window_schedule("window:nightly/often").is_err());
        assert!(validate_window_schedule("window:nightly/run-all").is_ok());
        assert_eq!(window_schedule("nightly", None).unwrap(), "window:nightly");
        assert_eq!(
            window_schedule("nightly", Some("run-once-on-wake")).unwrap(),
            "window:nightly"
        );
        assert_eq!(
            window_schedule("nightly", Some("skip")).unwrap(),
            "window:nightly/skip"
        );
        assert_eq!(window_schedule("none", Some("skip")).unwrap(), "manual");
    }

    #[test]
    fn next_after_run_walks_missed_occurrences_only_under_run_all() {
        let t0 = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        // Hourly occurrences.
        let after = |t: DateTime<Utc>| {
            let first = t0 + Duration::hours((t - t0).num_hours() + 1);
            (0..200)
                .map(|i| first + Duration::hours(i))
                .collect::<Vec<_>>()
        };
        let now = t0 + Duration::hours(3) + Duration::minutes(10);
        // run-once: the next start after now.
        assert_eq!(
            next_after_run(Some(CatchUp::RunOnceOnWake), Some(t0), now, &after),
            Some(t0 + Duration::hours(4))
        );
        // run-all: the next missed one.
        assert_eq!(
            next_after_run(Some(CatchUp::RunAll), Some(t0), now, &after),
            Some(t0 + Duration::hours(1))
        );
        // run-all far behind: the due run plus the last RUN_ALL_MAX - 1.
        let late = t0 + Duration::hours(50) + Duration::minutes(5);
        assert_eq!(
            next_after_run(Some(CatchUp::RunAll), Some(t0), late, &after),
            Some(t0 + Duration::hours(47))
        );
        // Caught up: the next start after now.
        assert_eq!(
            next_after_run(
                Some(CatchUp::RunAll),
                Some(t0 + Duration::hours(3)),
                now,
                &after
            ),
            Some(t0 + Duration::hours(4))
        );
    }

    #[test]
    fn a_windows_call_changes_only_what_it_names_and_is_validated() {
        let base = WindowSettings::default();
        let args = serde_json::json!({ "overnightCapUsd": 60.0 });
        let (windows, cap) = settings_args(&args).unwrap();
        let next = merged_settings(base.clone(), windows, cap).unwrap();
        assert_eq!(next.windows, base.windows);
        assert_eq!(next.overnight_cap_usd, 60.0);
        for bad in [
            serde_json::json!({ "overnightCapUsd": 5000.0 }),
            serde_json::json!({ "overnightCapUsd": "lots" }),
            serde_json::json!({ "windows": "all" }),
            serde_json::json!({ "windows": [{ "name": "Bad Name", "label": "x", "days": ["mon"], "start": "18:00", "end": "07:00" }] }),
            serde_json::json!({ "windows": [{ "name": "a", "label": "x", "days": ["mon"], "start": "6pm", "end": "07:00" }] }),
            serde_json::json!({ "windows": [
                { "name": "a", "label": "x", "days": ["mon"], "start": "18:00", "end": "07:00" },
                { "name": "a", "label": "y", "days": ["tue"], "start": "18:00", "end": "07:00" }
            ] }),
        ] {
            let result = settings_args(&bad).and_then(|(w, c)| merged_settings(base.clone(), w, c));
            assert!(result.is_err(), "{bad}");
        }
    }

    #[test]
    fn the_windows_diff_names_added_changed_and_removed_windows_and_a_raised_cap() {
        let old = WindowSettings::default();
        let mut new = old.clone();
        new.windows.retain(|w| w.name != "nightly");
        new.windows[0].start = "19:00".into();
        new.windows.push(WindowDef {
            name: "early".into(),
            label: "Early".into(),
            days: vec!["mon".into()],
            start: "05:00".into(),
            end: "06:00".into(),
        });
        new.overnight_cap_usd = 80.0;
        let lines = settings_diff(&old, &new);
        assert!(
            lines.iter().any(|l| l.starts_with("Add window: early")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Change window: tonight")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.starts_with("Remove window: nightly")),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"Overnight cap: $40 → $80".to_string()),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("higher cap")), "{lines:?}");
        assert!(settings_diff(&old, &old).is_empty());
    }
}
