//! The Friday status per tracked project (overhaul A18; design section 9,
//! "Hollow is where Alex reads").
//!
//! For each tracked workspace (a `PROJECT.md` whose `tracker_project` names
//! this provider's project) Hollow writes the week's status from the local
//! mirror (the sync ledger's issue snapshots: state kind, `updatedAt`, due
//! date, assignee, relations), never the network and never a model: done
//! this week (completed, changed in the last seven days), in progress
//! (started), blocked or overdue (an open blocker, a `Blocked` label, or a
//! due date before today), next up (unstarted, soonest due first) and the
//! workspace's build log entries dated this week. The mirror has no
//! completion time, so "done this week" reads a completed issue's last
//! change.
//!
//! **Written into Hollow first.** `workspaces/<id>/outputs/status/<date>-
//! friday-status.md` per project; for a whole run, the COO's digest at
//! `workspaces/main/outputs/status/<date>-friday-status.md` (a Result on the
//! COO's page), whose `title` is the one line the Herald reads (its
//! `workspaces.list_all_results` pass lists results by title); then one
//! Hollow notification, "Friday status is ready", opening the digest.
//!
//! **Tracker copy, optional.** With Settings' "Also post it to the tracker"
//! (`tracker.postStatusUpdates`, off by default) each project's status then
//! goes to the tracker as a project status update, team ENG only, through the
//! ledger's journal (`tracker_sync::post_status_with`), once a project a
//! week. Never the only copy: a failed post leaves the note and is recorded
//! (the journal, the audit record and this run's `copies`).
//!
//! **When.** The `friday-pm` window job (`schedule_windows::JOBS`,
//! `friday-status`, Friday 15:00 local) while `tracker.fridayStatus` is on
//! (off by default); the project detail's "Write status now"; the agent
//! action `tracker.friday_status`.

use super::ipc::{TrackerFridayStatus, TrackerStatusCopy, TrackerStatusNote};
use super::{ProjectHealth, RelationKind, StateKind, TrackerAdapter, TrackerIssue, TrackerProject};
use crate::config::Config;
use chrono::{DateTime, Duration, Local, NaiveDate};
use std::path::Path;

/// Where the notes go inside a workspace.
pub(crate) const NOTE_DIR: &str = "outputs/status";
/// Items listed per section (the count says how many there are).
const SECTION_MAX: usize = 15;
const NEXT_MAX: usize = 5;
const LOG_MAX: usize = 10;

/// One tracked workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tracked {
    pub workspace_id: String,
    pub title: String,
    pub project_id: String,
}

/// Every workspace whose `PROJECT.md` names a project of `provider`, by id.
pub(crate) fn tracked_workspaces(vault: &Path, provider: &str) -> Vec<Tracked> {
    let Ok(entries) = std::fs::read_dir(vault.join("workspaces")) else {
        return Vec::new();
    };
    let mut out: Vec<Tracked> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            if id.starts_with('.') || crate::workspaces::folder(vault, &id).is_err() {
                return None;
            }
            let text = std::fs::read_to_string(e.path().join("PROJECT.md")).ok()?;
            let fm = crate::frontmatter::parse(&text);
            let project = fm
                .get("tracker_project")
                .map(str::trim)
                .filter(|p| !p.is_empty())?;
            if provider.is_empty() || fm.get("tracker") != Some(provider) {
                return None;
            }
            let title = fm
                .get("title")
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(Tracked {
                project_id: project.to_string(),
                workspace_id: id,
                title,
            })
        })
        .collect();
    out.sort_by_key(|t| t.title.to_lowercase());
    out
}

/// A workspace's build log entries dated from `since` to `until` (headings
/// `## YYYY-MM-DD ...` in `BUILD_LOG.md` or `docs/BUILD_LOG.md`), newest first.
pub(crate) fn build_log_entries(
    workspace: &Path,
    since: NaiveDate,
    until: NaiveDate,
) -> Vec<String> {
    let Some(text) = ["BUILD_LOG.md", "docs/BUILD_LOG.md"]
        .iter()
        .find_map(|name| std::fs::read_to_string(workspace.join(name)).ok())
    else {
        return Vec::new();
    };
    let mut out: Vec<(NaiveDate, String)> = text
        .lines()
        .filter_map(|line| {
            let head = line
                .strip_prefix("## ")
                .or_else(|| line.strip_prefix("### "))?
                .trim();
            let date = NaiveDate::parse_from_str(head.get(..10)?, "%Y-%m-%d").ok()?;
            if date < since || date > until {
                return None;
            }
            let rest = head[10..]
                .trim_start_matches(|c: char| c.is_whitespace() || "·:|-".contains(c))
                .trim();
            Some((
                date,
                format!("{date} {}", one_line(rest, 160)).trim().to_string(),
            ))
        })
        .collect();
    out.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    out.into_iter().take(LOG_MAX).map(|(_, l)| l).collect()
}

fn one_line(text: &str, chars: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = flat.chars().take(chars).collect();
    if cut.chars().count() < flat.chars().count() {
        cut.push_str("...");
    }
    cut
}

/// One project's status, composed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Composed {
    /// The note's Markdown body (no frontmatter): also the tracker copy.
    pub body: String,
    /// One sentence: health and counts.
    pub line: String,
    pub done: usize,
    pub in_progress: usize,
    pub blocked: usize,
    pub overdue: usize,
    pub next: usize,
    pub health: ProjectHealth,
}

fn due_date(issue: &TrackerIssue) -> Option<NaiveDate> {
    issue
        .due
        .as_deref()
        .and_then(|d| NaiveDate::parse_from_str(d.get(..10)?, "%Y-%m-%d").ok())
}

fn changed_at(issue: &TrackerIssue) -> Option<DateTime<chrono::Utc>> {
    DateTime::parse_from_rfc3339(&issue.updated_at)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

/// Why an open issue is blocked: an open blocker, or a `Blocked` label.
fn blocked_by(issue: &TrackerIssue, all: &[TrackerIssue]) -> Option<String> {
    let blockers: Vec<&str> = issue
        .relations
        .iter()
        .filter(|r| r.kind == RelationKind::BlockedBy)
        .filter(|r| {
            all.iter()
                .find(|i| i.key.eq_ignore_ascii_case(&r.key))
                .is_none_or(|b| !b.state.kind.is_closed())
        })
        .map(|r| r.key.as_str())
        .collect();
    if !blockers.is_empty() {
        return Some(format!("blocked by {}", blockers.join(", ")));
    }
    issue
        .labels
        .iter()
        .any(|l| l.trim().eq_ignore_ascii_case("blocked"))
        .then(|| "labelled Blocked".to_string())
}

fn item(issue: &TrackerIssue, extra: &[String]) -> String {
    let mut facts: Vec<String> = Vec::new();
    if let Some(who) = &issue.assignee {
        facts.push(who.display_name.clone().unwrap_or_else(|| who.name.clone()));
    }
    facts.extend(extra.iter().cloned());
    let title = one_line(&issue.title, 140).replace(['[', ']'], "");
    let link = if issue.url.starts_with("https://") {
        format!("[{}]({})", issue.key, issue.url)
    } else {
        issue.key.clone()
    };
    if facts.is_empty() {
        format!("- {link} {title}")
    } else {
        format!("- {link} {title} ({})", facts.join(", "))
    }
}

fn section(out: &mut String, heading: &str, lines: &[String], total: usize) {
    out.push_str(&format!("\n## {heading}\n\n"));
    if lines.is_empty() {
        out.push_str("Nothing.\n");
        return;
    }
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    if total > lines.len() {
        out.push_str(&format!("- and {} more\n", total - lines.len()));
    }
}

/// Health from the counts: nothing blocked or overdue is on track; three or
/// more overdue, or blocked work with nothing moving, is off track.
pub(crate) fn health(
    done: usize,
    in_progress: usize,
    blocked: usize,
    overdue: usize,
) -> ProjectHealth {
    if blocked == 0 {
        ProjectHealth::OnTrack
    } else if overdue >= 3 || (done == 0 && in_progress == 0) {
        ProjectHealth::OffTrack
    } else {
        ProjectHealth::AtRisk
    }
}

/// Compose one project's status from its mirrored issues (`issues`, the
/// project's own; `all`, the whole mirror, for blockers) at `now`.
pub(crate) fn compose(
    tracked: &Tracked,
    project: Option<&TrackerProject>,
    issues: &[TrackerIssue],
    all: &[TrackerIssue],
    build_log: &[String],
    now: DateTime<Local>,
) -> Composed {
    let today = now.date_naive();
    let since = now.with_timezone(&chrono::Utc) - Duration::days(7);
    let mut sorted: Vec<&TrackerIssue> = issues.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));

    let done: Vec<&TrackerIssue> = sorted
        .iter()
        .copied()
        .filter(|i| i.state.kind == StateKind::Completed)
        .filter(|i| changed_at(i).is_some_and(|at| at >= since))
        .collect();
    let open: Vec<&TrackerIssue> = sorted
        .iter()
        .copied()
        .filter(|i| !i.state.kind.is_closed())
        .collect();
    let mut stuck: Vec<(String, &TrackerIssue)> = Vec::new();
    let mut overdue = 0;
    for issue in &open {
        let mut why: Vec<String> = Vec::new();
        if let Some(due) = due_date(issue).filter(|d| *d < today) {
            overdue += 1;
            why.push(format!("due {due}, overdue"));
        }
        if let Some(blocked) = blocked_by(issue, all) {
            why.push(blocked);
        }
        if !why.is_empty() {
            stuck.push((why.join("; "), issue));
        }
    }
    let is_stuck = |i: &TrackerIssue| stuck.iter().any(|(_, s)| s.id == i.id);
    let started: Vec<&TrackerIssue> = open
        .iter()
        .copied()
        .filter(|i| i.state.kind == StateKind::Started && !is_stuck(i))
        .collect();
    let mut next: Vec<&TrackerIssue> = open
        .iter()
        .copied()
        .filter(|i| i.state.kind == StateKind::Unstarted && !is_stuck(i))
        .collect();
    next.sort_by_key(|i| {
        (
            due_date(i).is_none(),
            due_date(i),
            if i.priority == 0 { 5 } else { i.priority },
            i.key.clone(),
        )
    });

    let health = health(done.len(), started.len(), stuck.len(), overdue);
    let line = format!(
        "{}: {} done this week, {} in progress, {} blocked or overdue, {} next up.",
        health.label(),
        done.len(),
        started.len(),
        stuck.len(),
        next.len()
    );
    let mut body = String::new();
    body.push_str(&format!(
        "# {}: Friday status, {}\n\n{line}\n",
        tracked.title,
        today.format("%-d %B %Y")
    ));
    if let Some(p) = project {
        let mut facts = vec![format!("Project status: {}", p.status.name)];
        if let Some(target) = p.target_date.as_deref().filter(|t| !t.is_empty()) {
            facts.push(format!("target {target}"));
        }
        if let Some(lead) = &p.lead {
            facts.push(format!("lead {}", lead.name));
        }
        body.push_str(&format!("\n{}.\n", facts.join(", ")));
    }
    let done_lines: Vec<String> = done
        .iter()
        .take(SECTION_MAX)
        .map(|i| item(i, &[]))
        .collect();
    section(&mut body, "Done this week", &done_lines, done.len());
    let started_lines: Vec<String> = started
        .iter()
        .take(SECTION_MAX)
        .map(|i| {
            item(
                i,
                &due_date(i)
                    .map(|d| vec![format!("due {d}")])
                    .unwrap_or_default(),
            )
        })
        .collect();
    section(&mut body, "In progress", &started_lines, started.len());
    let stuck_lines: Vec<String> = stuck
        .iter()
        .take(SECTION_MAX)
        .map(|(why, i)| item(i, std::slice::from_ref(why)))
        .collect();
    section(&mut body, "Blocked or overdue", &stuck_lines, stuck.len());
    let next_lines: Vec<String> = next
        .iter()
        .take(NEXT_MAX)
        .map(|i| {
            item(
                i,
                &due_date(i)
                    .map(|d| vec![format!("due {d}")])
                    .unwrap_or_default(),
            )
        })
        .collect();
    section(&mut body, "Next up", &next_lines, next.len());
    if !build_log.is_empty() {
        let lines: Vec<String> = build_log.iter().map(|l| format!("- {l}")).collect();
        section(&mut body, "Build log this week", &lines, lines.len());
    }
    body.push_str("\nWritten by Hollow from the tracker mirror and the workspace's build log; no model wrote these facts.\n");
    Composed {
        body,
        line,
        done: done.len(),
        in_progress: started.len(),
        blocked: stuck.len(),
        overdue,
        next: next.len(),
        health,
    }
}

fn quoted(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

/// A note's file: frontmatter (title, summary, kind, week) then the body.
fn note(title: &str, summary: &str, week: &str, extra: &[(&str, String)], body: &str) -> String {
    let mut out = format!(
        "---\ntitle: {}\nsummary: {}\nkind: friday-status\nweek: {}\n",
        quoted(title),
        quoted(summary),
        quoted(week)
    );
    for (key, value) in extra {
        out.push_str(&format!("{key}: {}\n", quoted(value)));
    }
    out.push_str("---\n\n");
    out.push_str(body);
    out
}

fn write(vault: &Path, rel: &str, text: &str) -> Result<(), String> {
    let full = crate::vault::safe_join(vault, rel)?;
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Friday status: {e}"))?;
    }
    crate::vault::atomic_write(&full, text.as_bytes())
}

/// What [`write_notes`] wrote, with each project's tracker copy body.
pub(crate) struct Written {
    pub status: TrackerFridayStatus,
    pub week: String,
    pub bodies: Vec<String>,
}

/// Write the notes: every tracked project's (or `only`'s), and for a whole
/// run the COO's digest. No network, no model.
pub(crate) fn write_notes(
    config: &Config,
    only: Option<&str>,
    now: DateTime<Local>,
) -> Result<Written, String> {
    let vault = &config.vault_path;
    let provider = config
        .tracker
        .provider
        .as_deref()
        .ok_or("Choose a task tracker first (Settings, Sources, Task tracker).")?;
    let ledger = super::work::read_ledger(config).ok_or(
        "The task tracker hasn't synced yet, so there is no status to write. Sync it first.",
    )?;
    let mirror = super::work::Mirror::of(&ledger);
    let mut tracked = tracked_workspaces(vault, provider);
    if let Some(only) = only.map(str::trim).filter(|o| !o.is_empty()) {
        tracked.retain(|t| t.workspace_id == only);
        if tracked.is_empty() {
            return Err(
                "That workspace isn't a tracked project (its PROJECT.md names no tracker project)."
                    .into(),
            );
        }
    }
    if tracked.is_empty() {
        return Err(
            "No workspace is a tracked project yet, so there is no status to write.".into(),
        );
    }
    let today = now.date_naive();
    let date = today.format("%Y-%m-%d").to_string();
    let week = crate::digest::week_of(today);
    let file = format!("{date}-friday-status.md");
    let mut notes = Vec::new();
    let mut bodies = Vec::new();
    for t in &tracked {
        let project = ledger
            .records
            .get(&t.project_id)
            .and_then(|r| r.project.as_ref());
        let issues: Vec<TrackerIssue> = mirror
            .issues
            .iter()
            .filter(|i| i.project_id.as_deref() == Some(t.project_id.as_str()))
            .cloned()
            .collect();
        let log = build_log_entries(
            &vault.join("workspaces").join(&t.workspace_id),
            today - Duration::days(6),
            today,
        );
        let composed = compose(t, project, &issues, &mirror.issues, &log, now);
        let rel = format!("workspaces/{}/{NOTE_DIR}/{file}", t.workspace_id);
        let title = format!("Friday status: {}, {date}", t.title);
        let text = note(
            &title,
            &composed.line,
            &week,
            &[
                ("project", t.title.clone()),
                ("tracker_project", t.project_id.clone()),
            ],
            &composed.body,
        );
        write(vault, &rel, &text)?;
        notes.push(TrackerStatusNote {
            workspace_id: t.workspace_id.clone(),
            title: t.title.clone(),
            project_id: t.project_id.clone(),
            path: rel,
            done: composed.done as u32,
            in_progress: composed.in_progress as u32,
            blocked: composed.blocked as u32,
            next: composed.next as u32,
            health: composed.health,
        });
        bodies.push(composed.body);
    }
    let sum = |f: fn(&TrackerStatusNote) -> u32| notes.iter().map(f).sum::<u32>();
    let (digest_path, herald_line) = if only.is_some_and(|o| !o.trim().is_empty()) {
        let n = &notes[0];
        (
            None,
            format!(
                "Friday status for {}: {} done this week, {} in progress, {} blocked or overdue.",
                n.title, n.done, n.in_progress, n.blocked
            ),
        )
    } else {
        let line = format!(
            "Friday status {date}: {} project{}, {} done this week, {} in progress, {} blocked or overdue.",
            notes.len(),
            if notes.len() == 1 { "" } else { "s" },
            sum(|n| n.done),
            sum(|n| n.in_progress),
            sum(|n| n.blocked)
        );
        let main = crate::workspaces::MAIN_AGENT_ID;
        if vault.join("workspaces").join(main).is_dir() {
            let rel = format!("workspaces/{main}/{NOTE_DIR}/{file}");
            let mut body = format!("# Friday status, {}\n\n{line}\n", today.format("%-d %B %Y"));
            for n in &notes {
                body.push_str(&format!(
                    "\n## {}\n\n{}: {} done, {} in progress, {} blocked or overdue, {} next up. [The status](../../../{}/{NOTE_DIR}/{file})\n",
                    n.title,
                    n.health.label(),
                    n.done,
                    n.in_progress,
                    n.blocked,
                    n.next,
                    n.workspace_id
                ));
            }
            body.push_str("\nWritten by Hollow from the tracker mirror; each project's note has the details.\n");
            write(vault, &rel, &note(&line, &line, &week, &[], &body))?;
            (Some(rel), line)
        } else {
            (None, line)
        }
    };
    Ok(Written {
        status: TrackerFridayStatus {
            date,
            digest_path,
            herald_line,
            projects: notes,
            copies: Vec::new(),
        },
        week,
        bodies,
    })
}

/// The run's one notification: "Friday status is ready", opening the digest
/// (or the one project's note).
pub(crate) fn notice(status: &TrackerFridayStatus) -> crate::notifications::NotificationInput {
    let open = status
        .digest_path
        .clone()
        .or_else(|| status.projects.first().map(|p| p.path.clone()));
    crate::notifications::NotificationInput {
        title: "Friday status is ready".into(),
        body: status.herald_line.clone(),
        kind: "info".into(),
        source: Some("friday-status".into()),
        action_label: Some("Open".into()),
        action_url: open.map(|p| format!("vault:{p}")),
        notify_native: true,
    }
}

#[cfg(test)]
thread_local! {
    /// The notifications this thread's runs queued (tests have no app handle).
    pub(crate) static NOTICES: std::cell::RefCell<Vec<crate::notifications::NotificationInput>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// After the notes: the notification, then (only with the setting on) each
/// project's tracker copy. A copy's failure is recorded, never fatal.
pub(crate) async fn deliver(
    config: &Config,
    written: Written,
    remote: Option<&dyn TrackerAdapter>,
) -> TrackerFridayStatus {
    let Written {
        mut status,
        week,
        bodies,
    } = written;
    let input = notice(&status);
    #[cfg(test)]
    NOTICES.with(|sent| sent.borrow_mut().push(input.clone()));
    if let Some(Err(error)) = crate::notifications::create_registered(&config.vault_path, input) {
        eprintln!("[friday status] notification: {error}");
    }
    if !config.tracker.post_status_updates {
        return status;
    }
    let owned;
    let remote: &dyn TrackerAdapter = match remote {
        Some(remote) => remote,
        None => match super::adapter(&config.tracker) {
            Ok(adapter) => {
                owned = adapter;
                owned.as_ref()
            }
            Err(error) => {
                status.copies = status
                    .projects
                    .iter()
                    .map(|p| TrackerStatusCopy {
                        workspace_id: p.workspace_id.clone(),
                        project_id: p.project_id.clone(),
                        posted: false,
                        note: error.to_string(),
                    })
                    .collect();
                return status;
            }
        },
    };
    for (project, body) in status.projects.iter().zip(&bodies) {
        let result = crate::tracker_sync::post_status_with(
            config,
            remote,
            &project.project_id,
            body,
            project.health,
            &week,
        )
        .await;
        status.copies.push(match result {
            Ok(outcome) => TrackerStatusCopy {
                workspace_id: project.workspace_id.clone(),
                project_id: project.project_id.clone(),
                posted: outcome.id.is_some(),
                note: outcome.note,
            },
            Err(why) => {
                eprintln!(
                    "[friday status] {}: tracker copy not posted: {why}",
                    project.workspace_id
                );
                TrackerStatusCopy {
                    workspace_id: project.workspace_id.clone(),
                    project_id: project.project_id.clone(),
                    posted: false,
                    note: why,
                }
            }
        });
    }
    status
}

/// Write the notes, notify, then the optional tracker copies.
pub(crate) async fn run(
    config: &Config,
    only: Option<&str>,
    now: DateTime<Local>,
    remote: Option<&dyn TrackerAdapter>,
) -> Result<TrackerFridayStatus, String> {
    let written = write_notes(config, only, now)?;
    Ok(deliver(config, written, remote).await)
}

/// The `friday-pm` window job: a whole run while `tracker.fridayStatus` is
/// on (the running config's; off by default), under the scheduler's write
/// lease for the notes. Off, or no tracker, stamps the week and does nothing.
pub(crate) fn friday_job(
    _app: tauri::AppHandle,
    config: Config,
    _key: String,
    _late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(async move {
        if !config.tracker.friday_status || crate::tracker_sync::enabled(&config).is_err() {
            return Ok(());
        }
        let written = {
            let config = config.clone();
            tauri::async_runtime::spawn_blocking(move || {
                let guard = crate::automated_history::scheduler_write_lease(&config.vault_path)
                    .map_err(|_| "the vault is busy".to_string())?;
                let out = write_notes(&config, None, Local::now());
                drop(guard);
                out
            })
            .await
            .map_err(|e| e.to_string())?
        };
        match written {
            Ok(written) => {
                let status = deliver(&config, written, None).await;
                for copy in status.copies.iter().filter(|c| !c.posted) {
                    eprintln!("[friday status] {}: {}", copy.workspace_id, copy.note);
                }
                Ok(())
            }
            // Busy: the next minute tries again. Nothing to write: done.
            Err(why) if why == "the vault is busy" => Err(why),
            Err(why) => {
                eprintln!("[friday status] {why}");
                Ok(())
            }
        }
    })
}
