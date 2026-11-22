//! Proactive nudges — deterministic scheduled scans over the vault that
//! fire notification-center entries so Hollow initiates instead of only
//! answering. No Claude in the loop: every rule is a cheap filesystem
//! read, so the scan can run every few minutes for free.
//!
//! Rules (v1):
//!   - **Frog carried** — an open task 3+ days past due.
//!   - **Commitment overdue** — an open promise whose due date has passed.
//!   - **Project gone cold** — an Active project whose file and tasks
//!     haven't been touched in 14+ days.
//!
//! Dedup state lives at `vault/.nudges-state.json` (dot-prefixed — the
//! vault watcher and tree ignore it): a map of nudge key → last-fired
//! RFC3339 stamp. A nudge fires once, then again only after its class's
//! refire window. (The meeting-soon rule was removed with Outlook on
//! 2026-09-26.)
//! Historical message mirrors do not produce nudges. The **first scan on a vault with no state file seeds
//! quietly**: existing backlog is stamped without firing, so enabling nudges never
//! dumps 30 notifications at once.

use crate::notifications::{self, NotificationInput};
use chrono::{DateTime, Duration as ChronoDuration, Local, NaiveDate, Utc};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const STATE_FILE: &str = ".nudges-state.json";
/// Scan cadence. The meeting rule needs sub-hour resolution; everything
/// else is daily-scale. 5 minutes is plenty and costs one directory walk.
const SCAN_INTERVAL_SECS: u64 = 300;
/// Hold the very first scan back a little so boot isn't doing a vault
/// walk while every resource warm-up is also hitting the disk.
const FIRST_SCAN_DELAY_SECS: u64 = 30;
/// Drop state entries older than this on save — keys are per-event /
/// per-message, so the map would otherwise grow forever.
const STATE_RETENTION_DAYS: i64 = 30;

/// Process-wide toggle, synced from Settings via `set_nudges_enabled`.
/// Defaults on — the whole point is initiative.
static ENABLED: AtomicBool = AtomicBool::new(true);
static TIMER_STARTED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

pub fn set_enabled(v: bool) {
    ENABLED.store(v, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Which rule produced a candidate — drives the per-scan caps and the
/// refire window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Frog,
    Commitment,
    ProjectCold,
}

impl Class {
    /// How many of this class may fire in a single scan. Excess
    /// candidates stay unstamped and fire on later scans — a trickle,
    /// not a flood.
    fn cap(self) -> usize {
        match self {
            Class::Frog => 3,
            Class::Commitment => 2,
            Class::ProjectCold => 2,
        }
    }

    /// After firing, how long before the same key may fire again.
    /// `None` = never (the key itself is per-occurrence).
    fn refire(self) -> Option<ChronoDuration> {
        match self {
            Class::Frog => Some(ChronoDuration::days(3)),
            Class::Commitment => Some(ChronoDuration::days(2)),
            Class::ProjectCold => Some(ChronoDuration::days(7)),
        }
    }
}

#[derive(Debug, Clone)]
struct Candidate {
    class: Class,
    /// Stable dedup key, e.g. `frog:tasks/follow-up.md`.
    key: String,
    title: String,
    body: String,
    /// Notification kind: `info` | `warning`.
    kind: String,
    action_url: Option<String>,
}

// ── State ──────────────────────────────────────────────────────────────

type State = HashMap<String, String>;

fn state_path(vault: &Path) -> PathBuf {
    vault.join(STATE_FILE)
}

fn load_state(vault: &Path) -> Option<State> {
    let raw = fs::read_to_string(state_path(vault)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_state(vault: &Path, state: &State, now: DateTime<Utc>) -> Result<(), String> {
    // Prune ancient entries so per-occurrence keys don't accumulate.
    let cutoff = now - ChronoDuration::days(STATE_RETENTION_DAYS);
    let pruned: State = state
        .iter()
        .filter(|(_, stamp)| {
            DateTime::parse_from_rfc3339(stamp)
                .map(|t| t.with_timezone(&Utc) > cutoff)
                .unwrap_or(false)
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let json = serde_json::to_string_pretty(&pruned).map_err(|e| format!("serialize: {e}"))?;
    fs::write(state_path(vault), json).map_err(|e| format!("write state: {e}"))
}

/// True when `key` is allowed to fire at `now` given the stored stamp.
fn should_fire(state: &State, key: &str, class: Class, now: DateTime<Utc>) -> bool {
    match state.get(key) {
        None => true,
        Some(stamp) => match class.refire() {
            None => false,
            Some(window) => DateTime::parse_from_rfc3339(stamp)
                .map(|t| now - t.with_timezone(&Utc) >= window)
                .unwrap_or(true),
        },
    }
}

// ── Rules ──────────────────────────────────────────────────────────────

/// Open tasks 3+ days past due — the frogs. Most-overdue first.
fn frogs_carried(vault: &Path, today: NaiveDate) -> Vec<Candidate> {
    let mut frogs: Vec<(i64, Candidate)> = Vec::new();
    for t in crate::vault_tasks::list(vault) {
        if crate::vault_tasks::is_terminal_status(&t.status) {
            continue;
        }
        let Some(due_raw) = t.due.as_deref() else {
            continue;
        };
        let Ok(due) = NaiveDate::parse_from_str(due_raw, "%Y-%m-%d") else {
            continue;
        };
        let late = (today - due).num_days();
        if late < 3 {
            continue;
        }
        let rel = t.url.strip_prefix("vault:").unwrap_or(&t.url).to_string();
        let prio = t.priority.clone().unwrap_or_else(|| "P4".into());
        frogs.push((
            late,
            Candidate {
                class: Class::Frog,
                key: format!("frog:{rel}"),
                title: format!("Carried {late} days — {}", t.title),
                body: format!("{prio} · due {due_raw}. Still the right next step?"),
                kind: "warning".into(),
                action_url: Some(format!("vault:{rel}")),
            },
        ));
    }
    frogs.sort_by_key(|f| std::cmp::Reverse(f.0));
    frogs.into_iter().map(|(_, c)| c).collect()
}

/// Open commitments whose due date has passed. Direction-aware copy keeps the
/// nudge honest about whether the user owes the follow-through or is waiting
/// on somebody else. Most-overdue first; the class cap limits each scan to two.
fn commitments_overdue(vault: &Path, today: NaiveDate) -> Vec<Candidate> {
    let mut overdue: Vec<(i64, Candidate)> = Vec::new();
    let today_key = today.format("%Y-%m-%d").to_string();
    for commitment in crate::vault_commitments::list(vault) {
        // Snoozed in the ledger (overnight 10-11 A5): quiet until its day.
        if commitment.status != "open"
            || crate::vault_commitments::is_snoozed(&commitment, &today_key)
        {
            continue;
        }
        let Some(due_raw) = commitment.due.as_deref() else {
            continue;
        };
        let Ok(due) = NaiveDate::parse_from_str(due_raw, "%Y-%m-%d") else {
            continue;
        };
        let late = (today - due).num_days();
        if late < 1 {
            continue;
        }
        let owner = if commitment.direction == "owed_by_me" {
            format!("You owe {}", commitment.counterparty)
        } else {
            format!("{} owes you", commitment.counterparty)
        };
        overdue.push((
            late,
            Candidate {
                class: Class::Commitment,
                key: format!("commitment:{}", commitment.fingerprint),
                title: format!("Commitment overdue — {}", commitment.title),
                body: format!("{owner} · {late}d overdue"),
                kind: "warning".into(),
                action_url: Some(commitment.url),
            },
        ));
    }
    overdue.sort_by_key(|o| std::cmp::Reverse(o.0));
    overdue
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect()
}

/// Active projects with no file activity (project file or its tasks) in
/// 14+ days.
fn projects_cold(vault: &Path, now: DateTime<Utc>) -> Vec<Candidate> {
    let threshold = ChronoDuration::days(14);
    let tasks = crate::vault_tasks::list(vault);
    let mut out = Vec::new();
    for p in crate::vault_projects::list(vault) {
        if p.status != "Active" {
            continue;
        }
        let Some(rel) = p.vault_path.as_deref() else {
            continue;
        };
        let mut newest = file_age_stamp(vault, rel);
        for t in &tasks {
            let mine = t.project_url.as_deref() == Some(p.url.as_str())
                || t.project_name.as_deref() == Some(p.name.as_str());
            if !mine {
                continue;
            }
            if let Some(trel) = t.url.strip_prefix("vault:") {
                if let Some(ts) = file_age_stamp(vault, trel) {
                    newest = Some(newest.map_or(ts, |n: DateTime<Utc>| n.max(ts)));
                }
            }
        }
        let Some(latest) = newest else { continue };
        let idle = now - latest;
        if idle < threshold {
            continue;
        }
        let days = idle.num_days();
        out.push(Candidate {
            class: Class::ProjectCold,
            key: format!("cold:{rel}"),
            title: format!("Project gone cold — {}", p.name),
            body: format!("No activity in {days} days. Park it or push it?"),
            kind: "info".into(),
            action_url: Some(format!("vault:{rel}")),
        });
    }
    out
}

fn file_age_stamp(vault: &Path, rel: &str) -> Option<DateTime<Utc>> {
    let meta = fs::metadata(vault.join(rel)).ok()?;
    let mtime = meta.modified().ok()?;
    Some(DateTime::<Utc>::from(mtime))
}

/// The current nudges as `(stable key, title, body)`, unfiltered by what
/// already fired (main agent M3: the check-in's token-free signals).
pub(crate) fn current(vault: &Path, now: DateTime<Utc>) -> Vec<(String, String, String)> {
    compute_candidates(vault, now)
        .into_iter()
        .map(|c| (c.key, c.title, c.body))
        .collect()
}

fn compute_candidates(vault: &Path, now: DateTime<Utc>) -> Vec<Candidate> {
    let today = now.with_timezone(&Local).date_naive();
    let mut all = frogs_carried(vault, today);
    all.extend(commitments_overdue(vault, today));
    all.extend(projects_cold(vault, now));
    all
}

// ── Scan ───────────────────────────────────────────────────────────────

/// Run one scan: compute candidates, dedupe against state, fire the
/// survivors as notifications. Returns how many fired. The first scan on
/// a vault without a state file seeds quietly (meetings excepted).
pub(crate) fn scan(app: &impl crate::core_host::CoreHost, vault: &Path) -> Result<usize, String> {
    let now = Utc::now();
    let candidates = compute_candidates(vault, now);
    let (mut state, first_run) = match load_state(vault) {
        Some(s) => (s, false),
        None => (State::new(), true),
    };

    let mut fired = 0usize;
    let mut per_class: HashMap<u8, usize> = HashMap::new();
    for c in candidates {
        if !should_fire(&state, &c.key, c.class, now) {
            continue;
        }
        // First run: stamp the backlog without firing.
        if first_run {
            state.insert(c.key, now.to_rfc3339());
            continue;
        }
        let count = per_class.entry(c.class as u8).or_insert(0);
        if *count >= c.class.cap() {
            continue; // unstamped — fires on a later scan
        }
        notifications::create(
            app,
            vault,
            NotificationInput {
                title: c.title,
                body: c.body,
                kind: c.kind,
                source: Some("nudge".into()),
                action_label: Some("Open".into()),
                action_url: c.action_url,
                notify_native: true,
            },
        )?;
        *count += 1;
        fired += 1;
        state.insert(c.key, now.to_rfc3339());
    }

    save_state(vault, &state, now)?;
    Ok(fired)
}

/// Boot the background scan loop. Idempotent — the second call is a
/// no-op (mirrors `workflows::start_scheduler`).
pub(crate) fn start_timer(app: impl Into<crate::core_host::Core>, vault: PathBuf) {
    let app = app.into();
    {
        let mut started = match TIMER_STARTED.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if *started {
            return;
        }
        *started = true;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(FIRST_SCAN_DELAY_SECS)).await;
        loop {
            if enabled() {
                if let Err(e) = scan(&app, &vault) {
                    eprintln!("[nudges] scan failed: {e}");
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(SCAN_INTERVAL_SECS)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn vault() -> TempDir {
        TempDir::new().unwrap()
    }

    fn write(vault: &Path, rel: &str, content: &str) {
        let p = vault.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    #[test]
    fn frog_needs_three_days_and_open_status() {
        let dir = vault();
        let today = Local::now().date_naive();
        let frog_due = today - ChronoDuration::days(4);
        let fresh_due = today - ChronoDuration::days(1);
        write(
            dir.path(),
            "tasks/old-frog.md",
            &format!(
                "---\ntitle: Old frog\nstatus: Not started\npriority: P2\ndue: {frog_due}\n---\n"
            ),
        );
        write(
            dir.path(),
            "tasks/just-late.md",
            &format!("---\ntitle: Just late\nstatus: Not started\ndue: {fresh_due}\n---\n"),
        );
        write(
            dir.path(),
            "tasks/done-frog.md",
            &format!("---\ntitle: Done frog\nstatus: Delivered\ndue: {frog_due}\n---\n"),
        );
        let c = frogs_carried(dir.path(), today);
        assert_eq!(c.len(), 1);
        assert!(c[0].title.contains("Old frog"));
        assert!(c[0].key.starts_with("frog:tasks/"));
    }

    #[test]
    fn retained_messages_do_not_nudge() {
        let dir = vault();
        let now = Utc::now();
        for folder in ["emails/inbox", "teams/dms", "slack/dms"] {
            let ts = (now - ChronoDuration::hours(30)).to_rfc3339();
            write(
                dir.path(),
                &format!("{folder}/unanswered.md"),
                &format!(
                    "---\nsubject: Unanswered\nfrom: Alice\nreceived: {ts}\nunread: true\ntriageState: waitingOnMe\n---\nbody\n"
                ),
            );
        }
        assert!(compute_candidates(dir.path(), now).is_empty());
    }

    #[test]
    fn overdue_commitments_are_direction_aware_and_terminal_items_are_skipped() {
        let dir = vault();
        let today = Local::now().date_naive();
        let due = today - ChronoDuration::days(2);
        let future = today + ChronoDuration::days(2);
        for (slug, title, direction, status, date) in [
            ("mine", "Send the deck", "owed_by_me", "open", due),
            ("theirs", "Approve the draft", "owed_to_me", "open", due),
            ("done", "Old promise", "owed_by_me", "fulfilled", due),
            ("future", "Later promise", "owed_by_me", "open", future),
        ] {
            write(
                dir.path(),
                &format!("commitments/{slug}.md"),
                &format!(
                    "---\ntype: commitment\ntitle: {title}\ndirection: {direction}\ncounterparty: Jordan\ndue: {date}\nstatus: {status}\nchannel: manual\ncreated: 2026-07-14T12:00:00Z\n---\n"
                ),
            );
        }
        // Snoozed past today: no nudge. Snoozed until today: it woke up.
        for (slug, until) in [("snoozed", future), ("woke", today)] {
            write(
                dir.path(),
                &format!("commitments/{slug}.md"),
                &format!(
                    "---\ntype: commitment\ntitle: Snoozed {slug}\ndirection: owed_by_me\ncounterparty: Sam\ndue: {due}\nsnoozed_until: {until}\nstatus: open\nchannel: manual\ncreated: 2026-07-14T12:00:00Z\n---\n"
                ),
            );
        }

        let candidates = commitments_overdue(dir.path(), today);
        assert_eq!(candidates.len(), 3);
        assert!(candidates.iter().any(|c| c.title.contains("Snoozed woke")));
        assert!(!candidates
            .iter()
            .any(|c| c.title.contains("Snoozed snoozed")));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.body.contains("You owe Jordan")));
        assert!(candidates
            .iter()
            .any(|candidate| candidate.body.contains("Jordan owes you")));
        assert!(candidates
            .iter()
            .all(|candidate| candidate.class == Class::Commitment));
    }

    #[test]
    fn should_fire_respects_refire_windows() {
        let now = Utc::now();
        let mut state = State::new();
        // Unknown key always fires.
        assert!(should_fire(&state, "frog:a", Class::Frog, now));
        // Just-fired frog is silenced…
        state.insert("frog:a".into(), now.to_rfc3339());
        assert!(!should_fire(&state, "frog:a", Class::Frog, now));
        // …but refires after 3 days.
        state.insert(
            "frog:a".into(),
            (now - ChronoDuration::days(4)).to_rfc3339(),
        );
        assert!(should_fire(&state, "frog:a", Class::Frog, now));
        state.insert(
            "commitment:x".into(),
            (now - ChronoDuration::days(3)).to_rfc3339(),
        );
        assert!(should_fire(&state, "commitment:x", Class::Commitment, now));
    }

    #[test]
    fn state_save_prunes_old_entries() {
        let dir = vault();
        let now = Utc::now();
        let mut state = State::new();
        state.insert("old".into(), (now - ChronoDuration::days(45)).to_rfc3339());
        state.insert("new".into(), now.to_rfc3339());
        save_state(dir.path(), &state, now).unwrap();
        let loaded = load_state(dir.path()).unwrap();
        assert!(loaded.contains_key("new"));
        assert!(!loaded.contains_key("old"));
    }

    #[test]
    fn cold_project_needs_14_idle_days() {
        let dir = vault();
        write(
            dir.path(),
            "projects/active/Old Push.md",
            "---\ntitle: Old Push\nstatus: active\n---\n",
        );
        // Fresh mtime → not cold.
        assert!(projects_cold(dir.path(), Utc::now()).is_empty());
        // Same file evaluated as if 20 days from now → cold.
        let later = Utc::now() + ChronoDuration::days(20);
        let c = projects_cold(dir.path(), later);
        assert_eq!(c.len(), 1);
        assert!(c[0].title.contains("Old Push"));
        assert_eq!(c[0].class, Class::ProjectCold);
    }
}
