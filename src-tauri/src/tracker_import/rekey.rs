//! The rekey: tracker identity on the open task files, the tracked
//! projects' `PROJECT.md`, and `.tracker-sync.json` (design section 8,
//! steps 3 to 6).
//!
//! Bindings, in order (one file, one issue, always):
//! 1. Files that already carry this provider's `tracker_id` keep it (a
//!    second rekey changes nothing).
//! 2. The mapping tables (`--mapping`, the 11-07 setup record): each row is
//!    checked against the tracker by key and title. A row whose issue is
//!    missing or whose title differs is reported and not applied, and its
//!    file isn't matched any other way. A file named by several rows takes
//!    the lowest-numbered issue; the others get their own
//!    `tasks/<provider>-<id>.md`, and the file's body lists all of them.
//! 3. Open Notion task files with no row: the issue whose description cites
//!    the file's path, then its `TASK-n`, then the one issue with the same
//!    normalized title. Anything else is reported as needing an issue.
//! 4. Every other issue of the team gets `tasks/<provider>-<id>.md`.
//!
//! Projects, in order: a `PROJECT.md` that already names the project;
//! `--bind-project KEY=DIR`; a workspace the project's description cites; the
//! one workspace with its title; then the workspace its issues cite most (the
//! mapping rows' 5th column, else the issues' descriptions), when it has at
//! least 2 votes (1 for a project of at most 2 issues), twice the runner-up's
//! count, is free, and has twice the votes of any other project claiming it.
//! Anything else is reported unbound with the reason. A workspace without
//! `PROJECT.md` gets one.
//!
//! Files are rendered by `tracker::mirror` (the sync engine's renderer) and
//! the ledger's `base` is the tracker's values, so the first sync pass after
//! install has nothing to pull or push. Only the adapter's read methods are
//! called.

use super::files::Overlay;
use super::records::{append_section, closed_word, norm};
use super::{Item, Report, Settings};
use crate::frontmatter::{self, KeyEdit};
use crate::tracker::ipc::{TrackerProjectOption, TrackerTeamMeta};
use crate::tracker::{mirror, TrackerAdapter, TrackerIssue, TrackerProject};
use crate::tracker_sync::{Ledger, Record};
use chrono::DateTime;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One mapping-table row: issue key, its title then, and the task file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub key: String,
    pub title: String,
    pub path: String,
}

fn cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                cell.push('|');
                chars.next();
            }
            '|' => out.push(std::mem::take(&mut cell).trim().to_string()),
            _ => cell.push(c),
        }
    }
    out.push(cell.trim().to_string());
    out
}

fn issue_key(text: &str) -> bool {
    let Some((team, number)) = text.split_once('-') else {
        return false;
    };
    !team.is_empty()
        && team.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && team
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// What the rekey binds by: the mapping tables' rows, the workspaces each
/// row's source column cites, and the `--bind-project` overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mapping {
    pub rows: Vec<Row>,
    /// Issue key -> the workspace folders its row's 5th column (the source,
    /// like `workspaces/<dir>/PLAN.md`) cites, from mapping rows only (a 6th
    /// column `vault:tasks/<file>.md`).
    pub sources: BTreeMap<String, Vec<String>>,
    /// Tracker project key or id -> workspace folder.
    pub overrides: Vec<(String, String)>,
}

/// The cells and task path of every mapping-table row: an issue key first
/// and a 6th column `vault:tasks/<file>.md`. Other tables in the file count
/// for nothing.
fn mapping_cells(text: &str) -> impl Iterator<Item = (Vec<String>, String)> + '_ {
    text.lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .map(cells)
        .filter(|c| c.len() >= 6 && issue_key(&c[0]))
        .filter_map(|c| {
            let path = c[5].trim_matches('`').strip_prefix("vault:")?.to_string();
            (path.starts_with("tasks/")
                && path.ends_with(".md")
                && super::files::check_rel(&path).is_ok())
            .then_some((c, path))
        })
}

/// Issue key -> workspace folders cited in the 5th column (the source) of
/// every mapping row ([`mapping_cells`]).
pub(crate) fn parse_sources(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (c, _) in mapping_cells(text) {
        let dirs = out.entry(c[0].clone()).or_default();
        for dir in cited_workspaces(&c[4]) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    out
}

/// Rows of every Markdown table whose 6th column is `vault:tasks/<file>.md`.
pub(crate) fn parse_mapping(text: &str) -> Vec<Row> {
    mapping_cells(text)
        .map(|(c, path)| Row {
            key: c[0].clone(),
            title: c[1].clone(),
            path,
        })
        .collect()
}

pub(crate) fn read_mapping(
    paths: &[PathBuf],
    overrides: &[(String, String)],
) -> Result<Mapping, String> {
    let mut mapping = Mapping {
        overrides: overrides.to_vec(),
        ..Mapping::default()
    };
    for path in paths {
        let text = std::fs::read_to_string(path)
            .map_err(|_| format!("The mapping file {} couldn't be read.", path.display()))?;
        mapping.rows.extend(parse_mapping(&text));
        for (key, dirs) in parse_sources(&text) {
            let known = mapping.sources.entry(key).or_default();
            for dir in dirs {
                if !known.contains(&dir) {
                    known.push(dir);
                }
            }
        }
    }
    Ok(mapping)
}

fn key_number(key: &str) -> u64 {
    key.rsplit_once('-')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(u64::MAX)
}

/// Titles compare after lower-casing, single spaces and plain dashes.
fn title_norm(text: &str) -> String {
    norm(&text.replace(['\u{2014}', '\u{2013}'], "-"))
}

/// `TASK-62` as a whole token (not `TASK-620`).
fn cites_token(text: &str, token: &str) -> bool {
    text.match_indices(token).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + token.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// The tracker as it is now: every issue of the team with its comments and
/// links, the projects, the team's metadata and the cursors.
pub(crate) struct Snapshot {
    pub issues: BTreeMap<String, TrackerIssue>,
    pub projects: BTreeMap<String, TrackerProject>,
    pub meta: TrackerTeamMeta,
    pub writable: Vec<crate::tracker::Field>,
    pub cursor_issues: Option<String>,
    pub cursor_comments: Option<String>,
    pub cursor_projects: Option<String>,
}

fn newest(values: impl Iterator<Item = String>) -> Option<String> {
    values
        .filter_map(|s| DateTime::parse_from_rfc3339(&s).ok().map(|t| (t, s)))
        .max_by_key(|(t, _)| *t)
        .map(|(_, s)| s)
}

async fn snapshot(
    remote: &dyn TrackerAdapter,
    settings: &Settings,
    report: &mut Report,
) -> Result<Snapshot, String> {
    let scope = settings.scope();
    let e = |e: crate::tracker::TrackerError| e.to_string();
    let team = remote.scope().await.map_err(e)?;
    settings.check_scope()?;
    if team.id != scope || team.id != super::DEFAULT_TEAM_ID || team.key != super::DEFAULT_TEAM_KEY
    {
        return Err(format!(
            "The tracker key reaches a team other than {} ({}); nothing was read.",
            super::DEFAULT_TEAM_KEY,
            super::DEFAULT_TEAM_ID
        ));
    }
    let mut issues = BTreeMap::new();
    let mut after = None;
    let mut seen = BTreeSet::new();
    loop {
        let page = remote
            .issues_changed(None, after.as_deref())
            .await
            .map_err(e)?;
        for issue in page.items {
            if issue.team_id == scope {
                issues.insert(issue.id.clone(), issue);
            } else {
                report.dropped_out_of_scope += 1;
            }
        }
        match page.next {
            Some(next) if seen.insert(next.clone()) => after = Some(next),
            Some(_) => return Err("The tracker repeated an issues cursor.".into()),
            None => break,
        }
    }
    let mut comment_times = Vec::new();
    let mut after = None;
    let mut seen = BTreeSet::new();
    loop {
        let page = remote
            .comments_changed(None, after.as_deref())
            .await
            .map_err(e)?;
        for change in page.items {
            if change.team_id != scope {
                report.dropped_out_of_scope += 1;
                continue;
            }
            comment_times.push(change.updated_at.clone());
            if let Some(issue) = issues.get_mut(&change.issue_id) {
                issue.comments.retain(|c| c.id != change.comment.id);
                issue.comments.push(change.comment);
            }
        }
        match page.next {
            Some(next) if seen.insert(next.clone()) => after = Some(next),
            Some(_) => return Err("The tracker repeated a comments cursor.".into()),
            None => break,
        }
    }
    for issue in issues.values_mut() {
        issue
            .comments
            .sort_by(|a, b| (&a.created_at, &a.id).cmp(&(&b.created_at, &b.id)));
    }
    let ids: Vec<String> = issues.keys().cloned().collect();
    for (id, links) in remote.attachments(&ids).await.map_err(e)? {
        if let Some(issue) = issues.get_mut(&id) {
            issue.links = links
                .iter()
                .filter(|l| l.source.as_deref() == Some("url"))
                .cloned()
                .collect();
            issue.attachments = links
                .into_iter()
                .filter(|l| l.source.as_deref() != Some("url"))
                .collect();
        }
    }
    let mut projects = BTreeMap::new();
    for project in remote.projects_changed(None).await.map_err(e)? {
        if project.team_ids.iter().any(|t| t == scope) {
            projects.insert(project.id.clone(), project);
        } else {
            report.dropped_out_of_scope += 1;
        }
    }
    let project_ids: Vec<String> = projects.keys().cloned().collect();
    let meta = TrackerTeamMeta {
        team,
        states: remote.states().await.map_err(e)?,
        labels: remote.labels().await.map_err(e)?,
        members: remote.members().await.map_err(e)?,
        cycles: remote.cycles().await.map_err(e)?,
        milestones: remote.milestones(&project_ids).await.map_err(e)?,
        projects: projects
            .values()
            .map(|p| TrackerProjectOption {
                id: p.id.clone(),
                name: p.name.clone(),
            })
            .collect(),
        viewer: remote.whoami().await.ok(),
    };
    Ok(Snapshot {
        cursor_issues: newest(issues.values().map(|i| i.updated_at.clone())),
        cursor_comments: newest(comment_times.into_iter()),
        cursor_projects: newest(projects.values().map(|p| p.updated_at.clone())),
        issues,
        projects,
        meta,
        writable: remote.writable_fields().to_vec(),
    })
}

/// Workspace folders a project description names, in order.
fn cited_workspaces(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in text.match_indices("workspaces/") {
        let rest = &text[at + "workspaces/".len()..];
        let dir: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let dir = dir.trim_end_matches('.').to_string();
        if !dir.is_empty() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// The PROJECT.md a bound workspace gets when it has none, through
/// `frontmatter::patch` like every other write (the mirror adds the rest).
fn project_seed(name: &str) -> Result<String, String> {
    frontmatter::patch(
        "---\ntitle: \"\"\n---\n",
        &[("title", KeyEdit::Quoted(name.to_string()))],
        Some("PROJECT.md mirrors this workspace's tracker project: its fields above and its description below come from the tracker. Keep notes and decisions in WORKSPACE.md."),
    )
}

fn fm_get(text: &str, key: &str) -> Option<String> {
    frontmatter::parse(text)
        .get(key)
        .map(str::to_string)
        .filter(|v| !v.trim().is_empty())
}

/// Read the tracker, bind files, and plan every rekeyed file and the ledger.
pub(crate) async fn rekey(
    remote: &dyn TrackerAdapter,
    settings: &Settings,
    mapping: &Mapping,
    overlay: &mut Overlay,
    report: &mut Report,
) -> Result<(), String> {
    let snap = snapshot(remote, settings, report).await?;
    plan(
        &snap,
        settings.provider(),
        settings.scope(),
        mapping,
        overlay,
        report,
    )
}

/// Workspace folders cited by a tracker project's issues, most first: the
/// mapping rows' source column when any of its issues has a row citing one,
/// else the issues' own descriptions. Each issue counts once per folder;
/// only folders that are workspaces count. Also which basis was used and
/// the project's whole issue count.
fn issue_votes(
    snap: &Snapshot,
    project: &TrackerProject,
    sources: &BTreeMap<String, Vec<String>>,
    exists: &dyn Fn(&str) -> bool,
) -> (Vec<(String, usize)>, &'static str, usize) {
    let issues: Vec<&TrackerIssue> = snap
        .issues
        .values()
        .filter(|i| i.project_id.as_deref() == Some(project.id.as_str()))
        .collect();
    let rows: Vec<&Vec<String>> = issues
        .iter()
        .filter_map(|i| sources.get(&i.key))
        .filter(|dirs| dirs.iter().any(|d| exists(d)))
        .collect();
    let (cites, basis): (Vec<Vec<String>>, &'static str) = if rows.is_empty() {
        (
            issues
                .iter()
                .map(|i| cited_workspaces(&i.description))
                .collect(),
            "issue descriptions",
        )
    } else {
        (rows.into_iter().cloned().collect(), "mapping rows")
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for dirs in cites {
        for dir in dirs.into_iter().filter(|d| exists(d)) {
            *counts.entry(dir).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    (ranked, basis, issues.len())
}

/// A project's vote winner, waiting for the other projects' winners.
struct Claim<'a> {
    project: &'a TrackerProject,
    dir: String,
    votes: usize,
    why: String,
}

/// Bind `project` to `workspaces/<dir>/PROJECT.md` and report why.
struct Binder {
    project_files: BTreeMap<String, String>,
    taken_dirs: BTreeMap<String, String>, // dir -> project name
    first_bound: BTreeSet<String>,
}

impl Binder {
    fn bind(&mut self, report: &mut Report, project: &TrackerProject, dir: &str, why: String) {
        let rel = format!("workspaces/{dir}/PROJECT.md");
        report.projects_bound.push(Item::new(
            &rel,
            &project.name,
            format!("{} ({why})", project.key.as_deref().unwrap_or(&project.id)),
        ));
        self.taken_dirs
            .insert(dir.to_string(), project.name.clone());
        self.first_bound.insert(project.id.clone());
        self.project_files.insert(project.id.clone(), rel);
    }
}

pub(crate) fn plan(
    snap: &Snapshot,
    provider: &str,
    scope: &str,
    mapping: &Mapping,
    overlay: &mut Overlay,
    report: &mut Report,
) -> Result<(), String> {
    let rows = &mapping.rows;
    report.tracker_issues = snap.issues.len();
    report.mapping_rows = rows.len();

    // Projects to workspaces, in this order: a PROJECT.md that already
    // names the project; `--bind-project`; a workspace its description
    // cites (a Notion project first); the one workspace with its title; the
    // workspace its issues cite most, by a clear margin. Anything else is
    // reported, unbound.
    let mut binder = Binder {
        project_files: BTreeMap::new(),
        taken_dirs: BTreeMap::new(),
        first_bound: BTreeSet::new(),
    };
    let dirs = overlay.dirs_in("workspaces");
    let mut briefs: BTreeMap<String, String> = BTreeMap::new();
    for dir in &dirs {
        if let Some(text) = overlay.text(&format!("workspaces/{dir}/PROJECT.md")) {
            if fm_get(&text, "tracker").as_deref() == Some(provider) {
                if let Some(id) = fm_get(&text, "tracker_project") {
                    let name = snap
                        .projects
                        .get(&id)
                        .map_or_else(|| id.clone(), |p| p.name.clone());
                    binder
                        .project_files
                        .insert(id, format!("workspaces/{dir}/PROJECT.md"));
                    binder.taken_dirs.insert(dir.clone(), name);
                }
            }
            briefs.insert(dir.clone(), text);
        }
    }
    let workspace_dirs: BTreeSet<String> = dirs
        .iter()
        .filter(|dir| {
            overlay.exists(&format!("workspaces/{dir}/PROJECT.md"))
                || overlay.exists(&format!("workspaces/{dir}/WORKSPACE.md"))
        })
        .cloned()
        .collect();
    let exists = |dir: &str| workspace_dirs.contains(dir);
    let mut projects: Vec<&TrackerProject> = snap.projects.values().collect();
    projects.sort_by_key(|p| (p.key.clone().unwrap_or_default(), p.name.clone()));

    for (wanted, dir) in &mapping.overrides {
        let flag = format!("--bind-project {wanted}={dir}");
        let Some(project) = projects
            .iter()
            .find(|p| p.id == *wanted || p.key.as_deref() == Some(wanted.as_str()))
        else {
            report.errors.push(format!(
                "{flag}: the team has no such project; not applied."
            ));
            continue;
        };
        if dir.contains('/') || dir.starts_with('.') || !exists(dir) {
            report.errors.push(format!(
                "{flag}: workspaces/{dir} isn't a workspace (no PROJECT.md or WORKSPACE.md); not applied."
            ));
            continue;
        }
        let rel = format!("workspaces/{dir}/PROJECT.md");
        match binder.project_files.get(&project.id) {
            Some(bound) if *bound == rel => continue,
            Some(bound) => {
                report.errors.push(format!(
                    "{flag}: the project is already bound to {bound}; not moved."
                ));
                continue;
            }
            None => {}
        }
        if let Some(other) = binder.taken_dirs.get(dir) {
            report.errors.push(format!(
                "{flag}: workspaces/{dir} is already bound to {other}; not applied."
            ));
            continue;
        }
        binder.bind(report, project, dir, "--bind-project".into());
    }

    let mut unbound: Vec<&TrackerProject> = Vec::new();
    for project in &projects {
        if binder.project_files.contains_key(&project.id) {
            continue;
        }
        let cited: Vec<String> = cited_workspaces(&project.description)
            .into_iter()
            .filter(|d| !binder.taken_dirs.contains_key(d) && exists(d))
            .collect();
        let notion_project = |d: &String| {
            briefs
                .get(d)
                .is_some_and(|t| fm_get(t, "notion_id").is_some())
        };
        let by_title: Vec<&String> = briefs
            .iter()
            .filter(|(d, t)| {
                !binder.taken_dirs.contains_key(*d)
                    && fm_get(t, "title")
                        .is_some_and(|title| title_norm(&title) == title_norm(&project.name))
            })
            .map(|(d, _)| d)
            .collect();
        let chosen = cited
            .iter()
            .find(|d| notion_project(d))
            .or(cited.first())
            .map(|d| (d.clone(), "its description cites the workspace"))
            .or_else(|| (by_title.len() == 1).then(|| (by_title[0].clone(), "same title")));
        match chosen {
            Some((dir, how)) => binder.bind(report, project, &dir, how.into()),
            None => unbound.push(project),
        }
    }

    // Last: the workspace the project's issues cite most. A winner needs
    // at least 2 votes (1 when the project has at most 2 issues), twice the
    // runner-up's count, and a workspace no earlier rule took. Every
    // project's winner is found first; a workspace several projects claim
    // goes to the one with twice the votes of every other claimant, else to
    // none of them.
    let label = |p: &TrackerProject| p.key.clone().unwrap_or_else(|| p.id.clone());
    let unbound_item = |project: &TrackerProject, detail: String| {
        let key = label(project);
        Item::new(
            "",
            &project.name,
            format!(
                "{key}: {detail}; the first sync pass would create a workspace. Bind it with --bind-project {key}=<workspace dir>."
            ),
        )
    };
    let mut claims: BTreeMap<String, Vec<Claim>> = BTreeMap::new();
    for project in unbound {
        let (ranked, basis, total) = issue_votes(snap, project, &mapping.sources, &exists);
        let Some((best, count)) = ranked.first().cloned() else {
            report.tracker_projects_without_workspace.push(unbound_item(
                project,
                "no workspace is cited by its description, has its title, or is cited by its issues' mapping rows or descriptions".into(),
            ));
            continue;
        };
        let runner_up = ranked.get(1).map_or(0, |(_, n)| *n);
        let tally = ranked
            .iter()
            .take(3)
            .map(|(d, n)| format!("{d} {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let refused = if count < 2 && total > 2 {
            Some(format!(
                "only {count} of {total} issues cite a workspace (workspaces/{best}); one is too few"
            ))
        } else if runner_up > 0 && count < 2 * runner_up {
            Some(format!(
                "its {basis} cite several workspaces with no clear winner ({tally}; {total} issues)"
            ))
        } else {
            binder.taken_dirs.get(&best).map(|other| {
                format!(
                    "its {basis} cite workspaces/{best} most ({tally}), but that workspace is bound to {other}"
                )
            })
        };
        if let Some(detail) = refused {
            report
                .tracker_projects_without_workspace
                .push(unbound_item(project, detail));
            continue;
        }
        let why = if runner_up == 0 {
            format!("its {basis} cite the workspace: {count} of {total} issues")
        } else {
            format!("its {basis} cite the workspace most: {count} of {total} issues ({tally})")
        };
        claims.entry(best.clone()).or_default().push(Claim {
            project,
            dir: best,
            votes: count,
            why,
        });
    }
    for (dir, mut claimants) in claims {
        claimants.sort_by(|a, b| {
            b.votes
                .cmp(&a.votes)
                .then_with(|| label(a.project).cmp(&label(b.project)))
        });
        let others = |skip: usize| {
            claimants
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != skip)
                .map(|(_, c)| format!("{} {}", label(c.project), c.votes))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let second = claimants.get(1).map_or(0, |c| c.votes);
        if second == 0 || claimants[0].votes >= 2 * second {
            for (i, loser) in claimants.iter().enumerate().skip(1) {
                report.tracker_projects_without_workspace.push(unbound_item(
                    loser.project,
                    format!(
                        "its issues cite workspaces/{dir} ({} votes), but {} has more ({})",
                        loser.votes,
                        label(claimants[0].project),
                        others(i)
                    ),
                ));
            }
            let winner = &claimants[0];
            let why = if claimants.len() > 1 {
                format!("{}; also claimed by {}", winner.why, others(0))
            } else {
                winner.why.clone()
            };
            binder.bind(report, winner.project, &winner.dir, why);
        } else {
            for (i, claimant) in claimants.iter().enumerate() {
                report.tracker_projects_without_workspace.push(unbound_item(
                    claimant.project,
                    format!(
                        "its issues cite workspaces/{dir} ({} votes), and so do other projects with no clear winner: {}",
                        claimant.votes,
                        others(i)
                    ),
                ));
            }
        }
    }
    let Binder {
        project_files,
        taken_dirs,
        first_bound,
    } = binder;
    report.projects_tracked = project_files
        .keys()
        .filter(|id| snap.projects.contains_key(*id))
        .count();
    report.projects_vault_only = briefs
        .iter()
        .filter(|(d, t)| !taken_dirs.contains_key(*d) && fm_get(t, "notion_id").is_some())
        .count();

    // Tasks to issues.
    let by_key: BTreeMap<&str, &TrackerIssue> =
        snap.issues.values().map(|i| (i.key.as_str(), i)).collect();
    let mut bound: BTreeMap<String, String> = BTreeMap::new(); // issue id -> path
    let mut file_of: BTreeMap<String, String> = BTreeMap::new(); // path -> issue id
    let mut task_texts: BTreeMap<String, String> = BTreeMap::new();
    for name in overlay.files_in("tasks") {
        if !name.ends_with(".md") {
            continue;
        }
        let rel = format!("tasks/{name}");
        let Some(text) = overlay.text(&rel) else {
            continue;
        };
        if fm_get(&text, "tracker").as_deref() == Some(provider) {
            if let Some(id) = fm_get(&text, "tracker_id") {
                if snap.issues.contains_key(&id) && !bound.contains_key(&id) {
                    bound.insert(id.clone(), rel.clone());
                    file_of.insert(rel.clone(), id);
                } else {
                    report.errors.push(format!("{rel} names an issue the team doesn't have (or a second file names it); left alone."));
                }
            }
        }
        task_texts.insert(rel, text);
    }
    let open_unbound: Vec<String> = task_texts
        .iter()
        .filter(|(rel, text)| {
            !file_of.contains_key(*rel)
                && fm_get(text, "notion_id").is_some()
                && fm_get(text, "tracker").is_none()
                && !fm_get(text, "notion_status")
                    .or_else(|| fm_get(text, "status"))
                    .is_some_and(|s| closed_word(&s))
        })
        .map(|(rel, _)| rel.clone())
        .collect();
    report.open_tasks = open_unbound.len()
        + file_of
            .keys()
            .filter(|rel| {
                task_texts
                    .get(*rel)
                    .is_some_and(|t| fm_get(t, "notion_id").is_some())
            })
            .count();

    // Mapping rows.
    let mut groups: BTreeMap<String, Vec<&TrackerIssue>> = BTreeMap::new();
    let mut row_files: BTreeSet<String> = BTreeSet::new();
    let mut key_files: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        key_files
            .entry(row.key.clone())
            .or_default()
            .insert(row.path.clone());
    }
    for row in rows {
        if file_of.contains_key(&row.path)
            || snap
                .issues
                .values()
                .any(|i| i.key == row.key && bound.contains_key(&i.id))
        {
            continue; // bound by an earlier rekey
        }
        row_files.insert(row.path.clone());
        let fail = |detail: String, report: &mut Report| {
            report.mapping_not_applied.push(Item::new(
                &row.path,
                &row.title,
                format!("{}: {detail}", row.key),
            ));
        };
        if !task_texts.contains_key(&row.path) {
            fail("the task file doesn't exist".into(), report);
            continue;
        }
        let Some(issue) = by_key.get(row.key.as_str()) else {
            fail("no such issue in the team".into(), report);
            continue;
        };
        if title_norm(&issue.title) != title_norm(&row.title) {
            fail(
                format!("the issue's title is now \"{}\"", issue.title),
                report,
            );
            continue;
        }
        if key_files[&row.key].len() > 1 {
            fail("several files name this issue".into(), report);
            continue;
        }
        groups.entry(row.path.clone()).or_default().push(issue);
    }
    let mut primary_of: BTreeMap<String, Vec<String>> = BTreeMap::new(); // path -> keys listed
    let mut siblings: Vec<(String, String)> = Vec::new(); // issue id, source path
    for (path, mut issues) in groups {
        issues.sort_by_key(|i| key_number(&i.key));
        issues.dedup_by_key(|i| i.id.clone());
        let first = issues[0];
        bound.insert(first.id.clone(), path.clone());
        file_of.insert(path.clone(), first.id.clone());
        report.open_mapped += 1;
        if issues.len() > 1 {
            let keys: Vec<String> = issues.iter().map(|i| i.key.clone()).collect();
            report
                .split_files
                .push(Item::new(&path, &first.title, keys.join(", ")));
            primary_of.insert(path.clone(), keys);
            for other in &issues[1..] {
                siblings.push((other.id.clone(), path.clone()));
            }
        }
    }

    // Open files with no row: description cite, TASK-n, then title.
    // Issues a mapping row names (applied or not) and split siblings are
    // never matched to another file.
    let reserved: BTreeSet<&str> = rows
        .iter()
        .map(|r| r.key.as_str())
        .chain(
            siblings
                .iter()
                .filter_map(|(id, _)| snap.issues.get(id).map(|i| i.key.as_str())),
        )
        .collect();
    let claimed = |bound: &BTreeMap<String, String>, issue: &TrackerIssue| {
        bound.contains_key(&issue.id) || reserved.contains(issue.key.as_str())
    };
    let mut wanted: BTreeMap<String, Vec<String>> = BTreeMap::new(); // issue id -> files
    let mut matched: BTreeMap<String, (String, &'static str)> = BTreeMap::new(); // path -> (issue, how)
    for rel in &open_unbound {
        if row_files.contains(rel) || file_of.contains_key(rel) {
            continue;
        }
        let text = &task_texts[rel];
        let title = fm_get(text, "title").unwrap_or_default();
        let task_id = fm_get(text, "notion_task_id");
        let free: Vec<&TrackerIssue> = snap
            .issues
            .values()
            .filter(|i| !claimed(&bound, i))
            .collect();
        let path_hits: Vec<&&TrackerIssue> = free
            .iter()
            .filter(|i| i.description.contains(rel.as_str()))
            .collect();
        let id_hits: Vec<&&TrackerIssue> = task_id
            .as_ref()
            .map(|t| {
                free.iter()
                    .filter(|i| cites_token(&i.description, t))
                    .collect()
            })
            .unwrap_or_default();
        let title_hits: Vec<&&TrackerIssue> = free
            .iter()
            .filter(|i| !title.is_empty() && title_norm(&i.title) == title_norm(&title))
            .collect();
        let pick = [
            (path_hits, "its description cites the file"),
            (id_hits, "its description cites the task id"),
            (title_hits, "same title"),
        ]
        .into_iter()
        .find(|(hits, _)| !hits.is_empty());
        match pick {
            Some((hits, how)) if hits.len() == 1 => {
                wanted
                    .entry(hits[0].id.clone())
                    .or_default()
                    .push(rel.clone());
                matched.insert(rel.clone(), (hits[0].id.clone(), how));
            }
            Some((hits, how)) => report.open_need_issue.push(Item::new(
                rel,
                &title,
                format!(
                    "{} issues match ({how}): {}",
                    hits.len(),
                    hits.iter()
                        .map(|i| i.key.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
            None => report.open_need_issue.push(Item::new(
                rel,
                &title,
                "no issue cites it or has its title",
            )),
        }
    }
    for (rel, (id, how)) in matched {
        let issue = &snap.issues[&id];
        let title = fm_get(&task_texts[&rel], "title").unwrap_or_default();
        if wanted[&id].len() > 1 {
            report.open_need_issue.push(Item::new(
                &rel,
                &title,
                format!("{} matches several files; none were bound", issue.key),
            ));
            continue;
        }
        bound.insert(id.clone(), rel.clone());
        file_of.insert(rel.clone(), id);
        report
            .open_matched
            .push(Item::new(&rel, &title, format!("{} ({how})", issue.key)));
    }

    // Every other issue gets its own file.
    let mut new_files: BTreeMap<String, Option<String>> = BTreeMap::new(); // id -> split source
    let split_source: BTreeMap<String, String> = siblings.into_iter().collect();
    for id in snap.issues.keys() {
        if bound.contains_key(id) {
            continue;
        }
        let rel = format!("tasks/{provider}-{id}.md");
        if let Some(text) = overlay.text(&rel) {
            if fm_get(&text, "tracker_id").as_deref() != Some(id) {
                report.errors.push(format!(
                    "{rel} exists and isn't this issue's mirror; left alone."
                ));
                continue;
            }
        } else {
            report.tracker_issue_files_new += 1;
        }
        bound.insert(id.clone(), rel.clone());
        file_of.insert(rel, id.clone());
        new_files.insert(id.clone(), split_source.get(id).cloned());
    }

    // Render.
    let issue_map = &snap.issues;
    let mut ledger = Ledger::new(provider, scope);
    for (id, rel) in &project_files {
        let Some(project) = snap.projects.get(id) else {
            report.errors.push(format!(
                "{rel} names tracker project {id}, which the team doesn't have; left alone."
            ));
            continue;
        };
        let mut old = overlay.text(rel).unwrap_or_default();
        if first_bound.contains(id) && !old.is_empty() {
            old = frontmatter::patch(&old, &[("description", KeyEdit::Remove)], None)
                .map_err(|e| format!("{rel}: {e}"))?;
        } else if old.is_empty() {
            // A workspace with no PROJECT.md yet (a local one): a minimal
            // brief for the mirror to fill in.
            old = project_seed(&project.name).map_err(|e| format!("{rel}: {e}"))?;
        }
        let text = mirror::project(&old, provider, project).map_err(|e| format!("{rel}: {e}"))?;
        overlay.set(rel, text.into_bytes())?;
        ledger.records.insert(
            id.clone(),
            Record {
                id: id.clone(),
                key: project.key.clone().unwrap_or_default(),
                kind: "project".into(),
                path: rel.clone(),
                workspace_id: rel.split('/').nth(1).map(str::to_string),
                base: mirror::project_values(project),
                conflicts: Vec::new(),
                error: None,
                remote_missing: false,
                state: "active".into(),
                issue: None,
                project: Some(project.clone()),
            },
        );
    }
    for (id, rel) in &bound {
        let issue = &issue_map[id];
        let mut old = overlay.text(rel).unwrap_or_default();
        // Not yet bound: a Notion file, or one the import froze as done
        // whose issue the mapping names (the tracker's state wins).
        let first_bind = !old.is_empty() && fm_get(&old, "tracker").is_none_or(|t| t == "none");
        if first_bind {
            let summary = fm_get(&old, "description");
            old = frontmatter::patch(
                &old,
                &[
                    ("description", KeyEdit::Remove),
                    ("notion_project_ids", KeyEdit::Remove),
                    ("archived_from", KeyEdit::Remove),
                ],
                None,
            )
            .map_err(|e| format!("{rel}: {e}"))?;
            if let Some(summary) = summary.filter(|s| norm(s) != norm(&issue.description)) {
                let date = fm_get(&old, "notion_last_edited")
                    .and_then(|d| d.get(..10).map(str::to_string))
                    .unwrap_or_else(|| "2026-11-08".into());
                old = append_section(
                    &old,
                    "## Notion summary",
                    &format!("## Notion summary ({date})\n\n{summary}"),
                );
            }
        }
        if let Some(keys) = primary_of.get(rel) {
            let lines: Vec<String> = keys
                .iter()
                .filter_map(|k| by_key.get(k.as_str()))
                .map(|i| format!("- {}: {}", i.key, i.title))
                .collect();
            old = append_section(
                &old,
                "## Tracker issues from this task",
                &format!("## Tracker issues from this task\n\nThe setup split this task into {} issues. This file mirrors {}; each of the others has its own file.\n\n{}", keys.len(), keys[0], lines.join("\n")),
            );
        }
        let values = mirror::values(issue, &snap.projects, issue_map);
        let project_path = issue
            .project_id
            .as_ref()
            .and_then(|p| project_files.get(p))
            .map(String::as_str);
        let state = if issue.archived_at.is_some() || issue.trashed {
            "archived"
        } else {
            "active"
        };
        let mut text = mirror::task(
            &old,
            provider,
            issue,
            &values,
            project_path,
            state,
            false,
            "",
        )
        .map_err(|e| format!("{rel}: {e}"))?;
        if let Some(Some(source)) = new_files.get(id) {
            let task_id = task_texts
                .get(source)
                .and_then(|t| fm_get(t, "notion_task_id"))
                .map(|t| format!(" (Notion {t})"))
                .unwrap_or_default();
            let note = format!("Split from `vault:{source}`{task_id}.");
            if !text.contains(&note) {
                text = format!("{}\n{note}\n", text.trim_end());
            }
        }
        overlay.set(rel, text.into_bytes())?;
        ledger.records.insert(
            id.clone(),
            Record {
                id: id.clone(),
                key: issue.key.clone(),
                kind: "task".into(),
                path: rel.clone(),
                workspace_id: None,
                base: values,
                conflicts: Vec::new(),
                error: None,
                remote_missing: false,
                state: state.into(),
                issue: Some(issue.clone()),
                project: None,
            },
        );
    }
    ledger.writable_fields = Some(snap.writable.clone());
    ledger.cursor_issues = snap.cursor_issues.clone();
    ledger.cursor_comments = snap.cursor_comments.clone();
    ledger.cursor_projects = snap.cursor_projects.clone();
    ledger.meta = Some(snap.meta.clone());
    let bytes = serde_json::to_vec_pretty(&ledger)
        .map_err(|_| "The tracker ledger couldn't be encoded.".to_string())?;
    let state = crate::tracker_sync::STATE;
    if let Some(existing) = overlay.read(state) {
        if existing != bytes {
            let prior: Ledger = serde_json::from_slice(&existing).map_err(|_| {
                "The vault's tracker ledger is unreadable; restore it before a rekey.".to_string()
            })?;
            if prior.last_sync.is_some() {
                // Sync has run since: its ledger holds conflicts and baselines
                // a rekey would replace, and a local edit could be reverted.
                return Err("Tracker sync has already run on this vault (its ledger has a last sync time), so a rekey would replace its history and could revert local edits. Nothing was changed; roll back to before the sync or leave the vault to the sync engine.".into());
            }
            if !prior.outbox.is_empty()
                || !prior.creates.is_empty()
                || !prior.project_creates.is_empty()
                || !prior.archives.is_empty()
                || !prior.comments.is_empty()
            {
                return Err("The vault's tracker ledger holds unsent edits or creates; let sync finish before a rekey.".into());
            }
        }
    }
    overlay.set(state, bytes)
}
