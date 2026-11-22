//! The gardener (master plan 2026-10, #19; see `docs/SCHEDULER.md#the-gardener`).
//!
//! **Scan** ([`scan`], pure core [`candidates`]): what could be tidied, each
//! line with its reason and how to undo it:
//! - a build workspace's worktree that is clean, has nothing unpushed and has
//!   been idle [`WORKTREE_IDLE_DAYS`] days → remove the worktree (the branch
//!   stays; `build_workspaces::remove` refuses unsaved work again). Never one
//!   `worktree_hygiene`'s holds keep (overnight S01e): a build running,
//!   waiting to install or installing still needs it, or the installed app or
//!   its rollback copy was built in it or from its HEAD. [`apply`] reads
//!   those holds again right before the removal (overnight R2) and once
//!   more right before `git worktree remove` (overnight R4);
//! - a workspace idle [`WORKSPACE_IDLE_DAYS`] days whose plan has no real
//!   open rows → archive it (built-in, tracker-managed and live build
//!   workspaces are never listed; open rows mean paused work, overnight B2),
//!   or one never used at all (still the blank template) at any age from a
//!   day old (10-11 B1);
//! - a learning whose statement exactly repeats an older one → move the newer
//!   copy to the vault trash;
//! - a local task titled `[Hollow test]…` (the live checks' throwaways) → move
//!   it to the trash. A tracker-linked one is listed for the person to
//!   remove in the tracker by hand: the gardener never touches the tracker;
//! - an iCloud/Finder conflict copy beside its original (overnight A10) →
//!   listed for the person to resolve in Waiting on you; never applied.
//!
//! **It only proposes** (decision 5). The `nightly` window job scans, writes
//! `workspaces/main/outputs/gardener/<date>.json` and keeps one live review
//! card ("N things can be tidied", kind `gardener`). Only the person's Do it
//! on that card (`resolve_review_item`) or the `gardener.apply`
//! action, which asks on a card in every mode and is refused from automations
//! and check-ins, applies anything, and only the ticked lines that a fresh
//! scan still finds ([`apply`]). It archives, trashes or removes clean
//! worktrees through their existing writers; it never deletes, never touches
//! the tracker or remote branches, and never removes unpushed work.

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub(crate) const REVIEW_KIND: &str = "gardener";
pub(crate) const REVIEW_SOURCE: &str = "gardener";
/// A workspace idle this long is offered for archiving (the plan's 30).
pub(crate) const WORKSPACE_IDLE_DAYS: i64 = 30;
/// A clean, pushed worktree idle this long is offered for removal.
pub(crate) const WORKTREE_IDLE_DAYS: i64 = 7;
/// The title prefix of the live checks' throwaway pages and tasks.
pub(crate) const TEST_PREFIX: &str = "[hollow test]";
const REPORT_DIR: &str = "workspaces/main/outputs/gardener";

/// What applying a line does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GardenAction {
    RemoveWorktree,
    Archive,
    Trash,
    /// Listed for the person to do by hand in its provider; never applied.
    Manual,
    /// Resolved by the person in Waiting on you (a conflict copy, overnight
    /// A10): keeping one of the two is their click there; never applied.
    Resolve,
}

/// One line of the gardener's card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GardenItem {
    /// Stable across scans: `<kind>:<target>`.
    pub id: String,
    /// `worktree`, `idle-workspace`, `duplicate-learning`, `test-task`,
    /// `conflict-copy`.
    pub kind: String,
    /// A workspace id or a vault path.
    pub target: String,
    pub title: String,
    pub reason: String,
    pub action: GardenAction,
    pub undo: String,
}

/// One scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GardenScan {
    pub date: String,
    pub scanned_at: String,
    pub items: Vec<GardenItem>,
}

/// A build worktree as the scan sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WorktreeFacts {
    pub exists: bool,
    pub branch: String,
    /// `None` when git couldn't say (never offered then).
    pub uncommitted: Option<usize>,
    pub unpushed: Option<usize>,
    /// Ignored files that aren't rebuildable output (`.env.local`, a local
    /// database): `git worktree remove` would delete them.
    pub ignored_kept: Option<usize>,
    /// Its canonical folder, matched against held folders.
    pub path: PathBuf,
    /// Its HEAD commit, matched against the apps' commits (`None` when git
    /// wasn't asked).
    pub head: Option<String>,
}

/// A workspace as the scan sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct WorkspaceFacts {
    pub id: String,
    pub title: String,
    pub status: String,
    pub builtin: bool,
    /// Legacy field name: true for a tracker-managed project.
    pub notion: bool,
    pub last_active: Option<DateTime<Utc>>,
    pub worktree: Option<WorktreeFacts>,
    /// Real open rows in PLAN.md (the two seed rows every new plan starts
    /// with don't count; see `workspace_shape::open_plan_rows`).
    pub open_plan: usize,
    /// Conversations that ran in it (newest 1,000 unarchived).
    pub conversations: usize,
    /// Still the blank template: no conversations, files or edits
    /// (`workspace_shape::never_used`); offered at any age.
    pub never_used: bool,
}

/// A learning as the scan sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct LearningFacts {
    pub path: String,
    pub title: String,
    pub statement: String,
    pub learned_at: String,
}

/// A task as the scan sees it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct TaskFacts {
    pub path: String,
    pub title: String,
    /// Legacy field name: tracker sync owns it, whatever its file is called.
    pub notion: bool,
}

fn days_idle(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Option<i64> {
    last.map(|t| (now - t).num_days())
}

/// What could be tidied (see the module doc), in a stable order. `held` is
/// what keeps a worktree because an app or a build came from it
/// (`worktree_hygiene::Holds`, folders canonical).
pub(crate) fn candidates(
    workspaces: &[WorkspaceFacts],
    learnings: &[LearningFacts],
    tasks: &[TaskFacts],
    held: &crate::worktree_hygiene::Holds,
    now: DateTime<Utc>,
) -> Vec<GardenItem> {
    let mut out = Vec::new();
    for ws in workspaces {
        if ws.builtin || ws.notion {
            continue;
        }
        let idle = days_idle(ws.last_active, now);
        match &ws.worktree {
            Some(wt) if wt.exists => {
                let clean = wt.uncommitted == Some(0)
                    && wt.unpushed == Some(0)
                    && wt.ignored_kept == Some(0);
                // A build waiting to install, or the checkout an app came
                // from, stays however idle (overnight S01e: removing it made
                // Install and restart say "The worktree is gone").
                let kept =
                    crate::worktree_hygiene::held_for(wt.head.as_deref(), &wt.path, held).is_some();
                if clean && !kept && idle.is_some_and(|d| d >= WORKTREE_IDLE_DAYS) {
                    out.push(GardenItem {
                        id: format!("worktree:{}", ws.id),
                        kind: "worktree".into(),
                        target: ws.id.clone(),
                        title: format!("Remove the worktree of “{}”", ws.title),
                        reason: format!(
                            "Clean, nothing unpushed, no ignored files to lose, idle {} days.",
                            idle.unwrap_or_default()
                        ),
                        action: GardenAction::RemoveWorktree,
                        undo: format!(
                            "The branch {} is kept; create the worktree again from it.",
                            wt.branch
                        ),
                    });
                }
                // A live worktree's workspace is never archived from here.
                continue;
            }
            _ => {}
        }
        if ws.status.trim().eq_ignore_ascii_case("archived") {
            continue;
        }
        // Never used: still the blank template, offered at any age (10-11 B1).
        if ws.never_used
            && ws.worktree.is_none()
            && idle.is_some_and(|d| d >= i64::from(crate::workspace_shape::NEVER_USED_MIN_DAYS))
        {
            out.push(GardenItem {
                id: format!("idle-workspace:{}", ws.id),
                kind: "idle-workspace".into(),
                target: ws.id.clone(),
                title: format!("Archive “{}”", ws.title),
                reason: crate::workspace_shape::never_used_reason(
                    u32::try_from(idle.unwrap_or_default()).unwrap_or(u32::MAX),
                ),
                action: GardenAction::Archive,
                undo: "Set its status back to active.".into(),
            });
            continue;
        }
        // Open plan rows mean paused work, not finished work (overnight B2).
        if ws.open_plan == 0 && idle.is_some_and(|d| d >= WORKSPACE_IDLE_DAYS) {
            out.push(GardenItem {
                id: format!("idle-workspace:{}", ws.id),
                kind: "idle-workspace".into(),
                target: ws.id.clone(),
                title: format!("Archive “{}”", ws.title),
                reason: format!(
                    "Nothing happened in it for {} days, and its plan has no open rows.",
                    idle.unwrap_or_default()
                ),
                action: GardenAction::Archive,
                undo: "Set its status back to active.".into(),
            });
        }
    }

    let mut by_statement: BTreeMap<String, Vec<&LearningFacts>> = BTreeMap::new();
    for l in learnings {
        let key = crate::vault_learning::statement_key(&l.statement);
        if !key.is_empty() {
            by_statement.entry(key).or_default().push(l);
        }
    }
    for group in by_statement.values_mut().filter(|g| g.len() > 1) {
        group.sort_by(|a, b| a.learned_at.cmp(&b.learned_at).then(a.path.cmp(&b.path)));
        let kept = group[0];
        for dup in &group[1..] {
            out.push(GardenItem {
                id: format!("duplicate-learning:{}", dup.path),
                kind: "duplicate-learning".into(),
                target: dup.path.clone(),
                title: format!("Trash the duplicate learning “{}”", dup.title),
                reason: format!("Same statement as the older {}.", kept.path),
                action: GardenAction::Trash,
                undo: "Restore it from the vault trash.".into(),
            });
        }
    }

    for t in tasks {
        if !t.title.trim().to_lowercase().starts_with(TEST_PREFIX) {
            continue;
        }
        if t.notion {
            out.push(GardenItem {
                id: format!("test-task:{}", t.path),
                kind: "test-task".into(),
                target: t.path.clone(),
                title: format!("Review the linked test task “{}” in its provider", t.title),
                reason: "A live check's throwaway owned by its provider. This cleanup requires your action there.".into(),
                action: GardenAction::Manual,
                undo: "Use the provider's recovery controls if you remove it.".into(),
            });
        } else {
            out.push(GardenItem {
                id: format!("test-task:{}", t.path),
                kind: "test-task".into(),
                target: t.path.clone(),
                title: format!("Trash the test task “{}”", t.title),
                reason: "A live check's throwaway.".into(),
                action: GardenAction::Trash,
                undo: "Restore it from the vault trash.".into(),
            });
        }
    }
    out
}

fn parse_time(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// `dir`'s HEAD commit (read-only, as [`git_count`]).
fn git_head(dir: &Path) -> Option<String> {
    let out = scan_git(dir).args(["rev-parse", "HEAD"]).output().ok()?;
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !head.is_empty()).then_some(head)
}

/// The scan's git in `dir`: Hollow's hardened helper
/// ([`crate::build_workspaces::git_command`]: `GIT_*` stripped, no prompts,
/// the pinned `SUBMODULE_SAFE` flags, a linked worktree pinned from its
/// `.git` file), so a planted submodule entry and decoy in an idle worktree
/// run nothing in Hollow's process (Q-R7, 2026-11-13). Read-only, no index
/// lock (the scan may run beside a build).
fn scan_git(dir: &Path) -> std::process::Command {
    let mut cmd = crate::build_workspaces::git_command(dir).into_std();
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

fn git_count(dir: &Path, args: &[&str]) -> Option<usize> {
    let out = scan_git(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if args.first() == Some(&"status") {
        Some(text.lines().filter(|l| !l.is_empty()).count())
    } else {
        text.trim().parse().ok()
    }
}

/// Rebuildable output a worktree may hold, as the first or last part of an
/// ignored path; any other ignored file counts.
const REBUILDABLE: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".vite",
    "coverage",
    ".DS_Store",
    "test-results",
    "playwright-report",
    ".playwright-cli",
];

/// Hollow's own ignored output, by full path (overnight S5: without these
/// every Hollow worktree read as holding work). Anchored, so a `models/` or
/// `gen/` in another repository still counts as work.
const HOLLOW_OUTPUT: &[&str] = &[
    "src-tauri/gen",
    "src-tauri/models",
    "src-tauri/binaries",
    "src-tauri/build-provenance.json",
];

/// Whether an ignored path (as `git status --ignored=matching` names it) is
/// rebuildable output; shared with `worktree_hygiene` (overnight S5), so the
/// gardener and the cleanup keep the same files. A secrets file or a local
/// database is never output, wherever it sits.
pub(crate) fn rebuildable(path: &str) -> bool {
    let path = path.trim_matches('"').trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    let lower = last.to_ascii_lowercase();
    if lower.starts_with(".env")
        || [".sqlite", ".sqlite3", ".db"]
            .iter()
            .any(|ext| lower.ends_with(ext))
    {
        return false;
    }
    let first = path.split('/').next().unwrap_or("");
    REBUILDABLE.contains(&first)
        || REBUILDABLE.contains(&last)
        || HOLLOW_OUTPUT
            .iter()
            .any(|p| path == *p || path.starts_with(&format!("{p}/")))
}

/// Ignored files in `dir` that aren't rebuildable output.
fn ignored_kept(dir: &Path) -> Option<usize> {
    let out = scan_git(dir)
        .args([
            "status",
            "--porcelain",
            "--ignored=matching",
            "--ignore-submodules=dirty",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.strip_prefix("!! "))
            .filter(|path| !rebuildable(path))
            .count(),
    )
}

/// Every conversation folder's count and newest activity, read once per
/// scan (the newest 1,000 unarchived conversations, as the rail lists them).
pub(crate) type SessionIndex =
    std::collections::HashMap<std::path::PathBuf, (usize, Option<DateTime<Utc>>)>;

/// [`SessionIndex`] for the vault.
pub(crate) fn session_index(vault: &Path) -> SessionIndex {
    let mut index = SessionIndex::new();
    for s in crate::session_store::list_sessions(vault, 1000).unwrap_or_default() {
        let entry = index
            .entry(crate::workspaces::session_dir(&s.cwd))
            .or_insert((0, None));
        entry.0 += 1;
        entry.1 = entry.1.max(parse_time(&s.last_activity_at));
    }
    index
}

/// One workspace as the scan sees it. Git runs read-only commands in its
/// worktree only when it has been idle `git_after_days` or longer (the only
/// case any caller reads the counts in).
pub(crate) fn workspace_facts(
    vault: &Path,
    ws: crate::workspaces::Workspace,
    sessions: &SessionIndex,
    git_after_days: i64,
    now: DateTime<Utc>,
) -> WorkspaceFacts {
    let builtin =
        crate::workspaces::is_main_agent(&ws.id) || crate::workspaces::is_quick_chats(&ws.id);
    let notion = crate::workspaces::managed_project(vault, &ws.id);
    // Its conversations' last activity, not when they were created: daily
    // work in an old conversation is not idle.
    let (conversations, newest) = crate::workspaces::folder(vault, &ws.id)
        .map(|dir| {
            crate::workspaces::session_dirs(&dir, &ws)
                .iter()
                .filter_map(|d| sessions.get(d))
                .fold((0, None), |(n, t), (c, last)| (n + c, t.max(*last)))
        })
        .unwrap_or((0, None));
    let last_active = [parse_time(&ws.modified), newest]
        .into_iter()
        .flatten()
        .max();
    let want_git = days_idle(last_active, now).is_some_and(|d| d >= git_after_days);
    let worktree = crate::build_workspaces::binding_for(vault, &ws.id).map(|b| {
        let dir = Path::new(&b.worktree);
        let live = b.exists && dir.is_dir();
        let git = live && want_git;
        WorktreeFacts {
            exists: live,
            branch: b.branch.clone(),
            uncommitted: git
                // `dirty`: a submodule's new commit counts; git never
                // enters a submodule to look at its files.
                .then(|| git_count(dir, &["status", "--porcelain", "--ignore-submodules=dirty"]))
                .flatten(),
            unpushed: git
                .then(|| {
                    git_count(
                        dir,
                        &["rev-list", "--count", "HEAD", "--not", &b.base, "--remotes"],
                    )
                })
                .flatten(),
            ignored_kept: git.then(|| ignored_kept(dir)).flatten(),
            path: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
            head: git.then(|| git_head(dir)).flatten(),
        }
    });
    let open_plan = crate::workspace_shape::open_plan_rows(vault, &ws.id);
    // A connected project's workspace still in the index is its scoped
    // retrieval and session routing, not its files: never "never used".
    let connected = ws.id.starts_with("local-")
        && crate::directory_connections::projects(vault)
            .iter()
            .any(|p| p.id == ws.id);
    let never_used = conversations == 0
        && open_plan == 0
        && worktree.is_none()
        && !builtin
        && !notion
        && !connected
        && crate::workspace_shape::never_used(vault, &ws.id)
        // The count above sees the newest 1,000 unarchived conversations;
        // any conversation at all, archived or old, is use.
        && !crate::workspaces::folder(vault, &ws.id).is_ok_and(|dir| {
            crate::workspace_shape::ever_had_a_conversation(
                vault,
                &crate::workspaces::session_dirs(&dir, &ws),
            )
        });
    WorkspaceFacts {
        id: ws.id,
        title: ws.title,
        status: ws.status,
        builtin,
        notion,
        last_active,
        worktree,
        open_plan,
        conversations,
        never_used,
    }
}

/// Read what the scan needs (git runs read-only commands in worktrees).
pub(crate) fn scan(vault: &Path, now: DateTime<Utc>) -> Result<GardenScan, String> {
    let sessions = session_index(vault);
    let workspaces = crate::workspaces::list(vault)?
        .into_iter()
        .map(|ws| workspace_facts(vault, ws, &sessions, WORKTREE_IDLE_DAYS, now))
        .collect::<Vec<_>>();
    let learnings: Vec<LearningFacts> = crate::vault_learning::list_context(vault, false)
        .into_iter()
        .map(|l| LearningFacts {
            path: l.path,
            title: l.title,
            statement: l.statement,
            learned_at: l.learned_at,
        })
        .collect();
    // `list` leaves `vault_path` to the UI's enrichment; the url is `vault:<rel>`.
    let tasks: Vec<TaskFacts> = crate::vault_tasks::list(vault)
        .into_iter()
        .filter_map(|t| {
            let path = t
                .vault_path
                .clone()
                .or_else(|| t.url.strip_prefix("vault:").map(str::to_string))?;
            Some(TaskFacts {
                notion: t.tracker.is_some()
                    || path
                        .rsplit('/')
                        .next()
                        .and_then(|f| f.strip_suffix(".md"))
                        .is_some_and(|id| crate::workspaces::managed_project(vault, id)),
                path,
                title: t.title,
            })
        })
        .collect();
    let mut tasks = tasks;
    tasks.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(GardenScan {
        date: now.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        scanned_at: now.to_rfc3339(),
        items: {
            // Read once per scan; `apply`'s fresh scan reads them again.
            let held = crate::worktree_hygiene::Holds::read().canonical();
            let mut items = candidates(&workspaces, &learnings, &tasks, &held, now);
            items.extend(conflict_lines(&crate::conflict_copies::scan(vault)));
            items
        },
    })
}

/// One informational line per conflict copy (overnight A10): resolving is
/// the person's click in Waiting on you, never the gardener's.
pub(crate) fn conflict_lines(copies: &[crate::conflict_copies::ConflictCopy]) -> Vec<GardenItem> {
    copies
        .iter()
        .map(|c| GardenItem {
            id: format!("conflict-copy:{}", c.copy),
            kind: "conflict-copy".into(),
            target: c.copy.clone(),
            title: c.copy.clone(),
            reason: if c.identical {
                format!("A conflict copy of {}, with the same contents.", c.original)
            } else {
                format!("A conflict copy of {} that differs from it.", c.original)
            },
            action: GardenAction::Resolve,
            undo: "Resolve it in Bell → Needs you → See all → Decisions: you keep one; the other goes to the vault's trash."
                .into(),
        })
        .collect()
}

/// What [`apply`] did with each ticked line.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub applied: Vec<String>,
    /// `(id, why)` for lines not applied.
    pub skipped: Vec<(String, String)>,
}

/// Apply the ticked lines `ids` that a fresh scan still finds, each through
/// its existing writer. `caller` is a Hollow app caller's run id (its lease
/// lets the worktree removal run inside it). Never deletes; a manual line is
/// never applied. A worktree's holds are read again right before it is
/// removed (overnight R2).
pub(crate) async fn apply(
    vault: &Path,
    ids: &[String],
    caller: Option<&str>,
    now: DateTime<Utc>,
) -> Result<ApplyReport, String> {
    let fresh = {
        let vault = vault.to_path_buf();
        tauri::async_runtime::spawn_blocking(move || scan(&vault, now))
            .await
            .map_err(|e| e.to_string())??
    };
    let held_now = || crate::worktree_hygiene::Holds::read().canonical();
    Ok(apply_scan(vault, &fresh, ids, caller, held_now).await)
}

/// [`apply`] once the fresh scan is made. `held_now` reads the holds again
/// (this Mac's in [`apply`]), right before each worktree removal.
async fn apply_scan(
    vault: &Path,
    fresh: &GardenScan,
    ids: &[String],
    caller: Option<&str>,
    held_now: impl Fn() -> crate::worktree_hygiene::Holds,
) -> ApplyReport {
    let wanted: BTreeSet<&str> = ids.iter().map(String::as_str).collect();
    let mut report = ApplyReport::default();
    for id in &wanted {
        if !fresh.items.iter().any(|i| i.id == *id) {
            report.skipped.push((
                id.to_string(),
                "No longer a candidate; left as it is.".into(),
            ));
        }
    }
    for item in fresh
        .items
        .iter()
        .filter(|i| wanted.contains(i.id.as_str()))
    {
        let done = match item.action {
            GardenAction::Manual => {
                Err("Yours to do by hand; Hollow never changes the tracker's copy.".to_string())
            }
            GardenAction::Resolve => Err(
                "Yours to resolve in Bell → Needs you → See all → Decisions: keep one of the two there.".to_string(),
            ),
            GardenAction::Archive => {
                crate::workspaces::update_meta(vault, &item.target, None, Some("archived"))
                    .map(|_| ())
            }
            GardenAction::Trash => crate::vault::delete_note(vault, &item.target),
            GardenAction::RemoveWorktree => {
                remove_worktree(vault, &item.target, caller, &held_now).await
            }
        };
        match done {
            Ok(()) => report.applied.push(item.id.clone()),
            Err(e) => report.skipped.push((item.id.clone(), e)),
        }
    }
    report
}

/// Remove workspace `id`'s worktree unless the holds `holds` reads just now
/// keep it (overnight R2: a Build or Install pressed since the scan needs
/// it), read once before the removal starts and once more right before `git
/// worktree remove`, after its services stopped (overnight R4). No lock is
/// held at either read: `self_install::held_worktrees` takes the build job's.
async fn remove_worktree(
    vault: &Path,
    id: &str,
    caller: Option<&str>,
    holds: &impl Fn() -> crate::worktree_hygiene::Holds,
) -> Result<(), String> {
    let Some(b) = crate::build_workspaces::binding_for(vault, id) else {
        return crate::build_workspaces::remove_for(vault, id, false, caller).await;
    };
    let dir = PathBuf::from(&b.worktree);
    if let Some(why) = crate::worktree_hygiene::held_now(&dir, &holds()).await {
        return Err(why);
    }
    let kept = move || async move { crate::worktree_hygiene::held_now(&dir, &holds()).await };
    crate::build_workspaces::remove_for_unless(vault, id, false, caller, kept).await
}

fn fingerprint(scan: &GardenScan) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for item in &scan.items {
        h.update(item.id.as_bytes());
        h.update([0]);
    }
    format!("{:x}", h.finalize())
}

/// The card's title.
pub(crate) fn card_title(n: usize) -> String {
    format!("{n} thing{} can be tidied", if n == 1 { "" } else { "s" })
}

/// Keep the one live gardener card in step with `scan`: a card for its lines,
/// or none when nothing is left to tidy.
pub(crate) fn post(vault: &Path, scan: &GardenScan) -> Result<(), String> {
    if scan.items.is_empty() {
        crate::review_queue::retire_live(
            vault,
            REVIEW_KIND,
            REVIEW_KIND,
            "Superseded: nothing left to tidy.",
        )?;
        return Ok(());
    }
    crate::review_queue::enqueue_exclusive(
        vault,
        crate::review_queue::NewReviewItem {
            kind: REVIEW_KIND.into(),
            fingerprint: fingerprint(scan),
            payload: serde_json::json!({
                "title": card_title(scan.items.len()),
                "date": scan.date,
                "items": scan.items,
            }),
            risk: "local_write".into(),
            source: REVIEW_SOURCE.into(),
            source_path: Some(REVIEW_KIND.into()),
        },
        "Superseded: a newer gardener scan.",
    )
    .map(|_| ())
}

/// Write the scan's report into the Main agent's outputs (when it exists).
pub(crate) fn write_report(vault: &Path, scan: &GardenScan) -> Result<Option<String>, String> {
    if crate::workspaces::main_agent_cwd(vault).is_none() {
        return Ok(None);
    }
    let rel = format!("{REPORT_DIR}/{}.json", scan.date);
    let path = vault.join(&rel);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("gardener folder: {e}"))?;
    }
    let text = serde_json::to_string_pretty(scan).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, text.as_bytes())?;
    Ok(Some(rel))
}

/// The person's Do it on the gardener card (`resolve_review_item`): apply
/// the ticked lines (`edited.ticked`, default every line that isn't manual),
/// then accept the card. Lines come from a fresh scan, never the payload.
pub(crate) async fn resolve_review(
    vault: &Path,
    item: &crate::review_queue::ReviewItem,
    edited: Option<&serde_json::Value>,
) -> Result<ApplyReport, String> {
    let ticked: Vec<String> = match edited.and_then(|e| e.get("ticked")) {
        Some(list) => serde_json::from_value(list.clone())
            .map_err(|e| format!("invalid gardener selection: {e}"))?,
        None => item
            .payload
            .get("items")
            .and_then(|v| serde_json::from_value::<Vec<GardenItem>>(v.clone()).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|i| !matches!(i.action, GardenAction::Manual | GardenAction::Resolve))
            .map(|i| i.id)
            .collect(),
    };
    crate::review_queue::claim(vault, &item.id, None)?;
    match apply(vault, &ticked, None, Utc::now()).await {
        // Nothing applied although something was ticked: the card stays open
        // with why, rather than reading as done.
        Ok(report) if report.applied.is_empty() && !report.skipped.is_empty() => {
            let why = report
                .skipped
                .iter()
                .map(|(id, why)| format!("{id}: {why}"))
                .collect::<Vec<_>>()
                .join("; ");
            let why = format!("Nothing was tidied. {why}");
            let _ = crate::review_queue::fail(vault, &item.id, &why);
            Err(why)
        }
        Ok(report) => {
            crate::review_queue::accept(vault, &item.id)?;
            Ok(report)
        }
        Err(e) => {
            let _ = crate::review_queue::fail(vault, &item.id, &e);
            Err(e)
        }
    }
}

/// The `nightly` window job: scan, write the report, keep the card. It only
/// scans: nothing is applied without the person.
pub(crate) async fn run_nightly(
    _app: tauri::AppHandle,
    config: crate::config::Config,
    _key: String,
    _late: bool,
) -> Result<(), String> {
    let vault = config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Cheap check first: while a run holds the vault, don't run git in
        // every worktree each minute only to find the lease busy.
        if crate::automated_history::lease_holder(&vault).is_some() {
            return Err("the vault is busy".to_string());
        }
        let scan = scan(&vault, Utc::now())?;
        let guard = crate::automated_history::scheduler_write_lease(&vault)
            .map_err(|_| "the vault is busy".to_string())?;
        let written = write_report(&vault, &scan);
        drop(guard);
        written?;
        post(&vault, &scan)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// [`run_nightly`] as a window job.
pub(crate) fn nightly_job(
    app: tauri::AppHandle,
    config: crate::config::Config,
    key: String,
    late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(run_nightly(app, config, key, late))
}

/// What could be tidied now (no change is made).
#[tauri::command]
pub async fn gardener_scan(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<GardenScan, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || scan(&vault, Utc::now()))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a real Hollow worktree ignores is output; secrets and local
    /// data are not (overnight S5).
    #[test]
    fn rebuildable_output_is_told_from_kept_files() {
        for out in [
            "node_modules/",
            "src-tauri/gen/",
            "src-tauri/models/bge-small-en-v1.5/",
            "src-tauri/build-provenance.json",
            "src-tauri/binaries/server-aarch64-apple-darwin",
            "\"dist/a b.js\"",
            ".DS_Store",
        ] {
            assert!(rebuildable(out), "{out}");
        }
        for kept in [
            ".env",
            ".env.local",
            "config.json",
            "data/notion/x.json",
            "app.sqlite",
            "deploy/build/.env.production",
            "dist/local.db",
            "models/checkpoint.pt",
            "gen/notes.md",
        ] {
            assert!(!rebuildable(kept), "{kept}");
        }
    }
    use chrono::{Duration, TimeZone};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 9, 3, 0, 0).unwrap()
    }

    fn ago(days: i64) -> Option<DateTime<Utc>> {
        Some(now() - Duration::days(days))
    }

    fn ws(id: &str, idle: i64) -> WorkspaceFacts {
        WorkspaceFacts {
            id: id.into(),
            title: format!("WS {id}"),
            status: "active".into(),
            last_active: ago(idle),
            ..Default::default()
        }
    }

    fn worktree(uncommitted: usize, unpushed: usize) -> Option<WorktreeFacts> {
        Some(WorktreeFacts {
            exists: true,
            branch: "feat/x".into(),
            uncommitted: Some(uncommitted),
            unpushed: Some(unpushed),
            ignored_kept: Some(0),
            ..Default::default()
        })
    }

    /// Nothing held.
    fn none_held() -> crate::worktree_hygiene::Holds {
        crate::worktree_hygiene::Holds::default()
    }

    #[test]
    fn a_dirty_or_unpushed_worktree_is_never_offered() {
        let items = candidates(
            &[
                WorkspaceFacts {
                    worktree: worktree(0, 0),
                    ..ws("clean", 10)
                },
                WorkspaceFacts {
                    worktree: worktree(3, 0),
                    ..ws("dirty", 90)
                },
                WorkspaceFacts {
                    worktree: worktree(0, 2),
                    ..ws("unpushed", 90)
                },
                WorkspaceFacts {
                    worktree: Some(WorktreeFacts {
                        exists: true,
                        uncommitted: None,
                        unpushed: Some(0),
                        ..Default::default()
                    }),
                    ..ws("unknown", 90)
                },
                WorkspaceFacts {
                    worktree: worktree(0, 0),
                    ..ws("recent", 2)
                },
                WorkspaceFacts {
                    worktree: Some(WorktreeFacts {
                        ignored_kept: Some(1),
                        ..worktree(0, 0).unwrap()
                    }),
                    ..ws("env-local", 90)
                },
            ],
            &[],
            &[],
            &none_held(),
            now(),
        );
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        // Live worktrees are never archived either, however idle.
        assert_eq!(ids, vec!["worktree:clean"]);
        assert_eq!(items[0].action, GardenAction::RemoveWorktree);
        assert!(items[0].undo.contains("feat/x"));
    }

    /// Overnight S01e: a clean, pushed, idle build worktree that a build
    /// still needs (waiting to install: the install step runs in it), or
    /// that the installed app was built in or from, is not offered; the
    /// same worktree unheld is.
    #[test]
    fn a_worktree_a_build_or_an_app_holds_is_never_offered() {
        use crate::worktree_hygiene::Holds;
        let facts = [WorkspaceFacts {
            worktree: Some(WorktreeFacts {
                path: PathBuf::from("/w/release"),
                head: Some("abcdef1234567890".into()),
                ..worktree(0, 0).unwrap()
            }),
            ..ws("release", 30)
        }];
        let ids = |held: &Holds| -> Vec<String> {
            candidates(&facts, &[], &[], held, now())
                .into_iter()
                .map(|i| i.id)
                .collect()
        };
        assert_eq!(ids(&none_held()), vec!["worktree:release".to_string()]);
        let elsewhere = Holds {
            commits: vec![("0123456789abcdef".into(), "built the rollback copy".into())],
            paths: vec![(PathBuf::from("/w/other"), "built the installed app".into())],
        };
        assert_eq!(ids(&elsewhere), vec!["worktree:release".to_string()]);
        for why in [
            "a finished build waiting to install",
            "a build running now",
            "built the installed app",
        ] {
            let by_folder = Holds {
                paths: vec![(PathBuf::from("/w/release"), why.into())],
                ..none_held()
            };
            assert!(ids(&by_folder).is_empty(), "{why}");
        }
        let by_commit = Holds {
            commits: vec![("abcdef1".into(), "built the installed app".into())],
            ..none_held()
        };
        assert!(ids(&by_commit).is_empty());
    }

    /// Overnight R2: a Build or Install pressed after the fresh scan holds
    /// its worktree by the time the removal comes. Holds are read again
    /// right before each removal: that line is skipped with why, and the
    /// next one still goes.
    #[tokio::test]
    async fn a_worktree_held_after_the_scan_is_kept_and_the_next_still_goes() {
        use crate::build_workspaces::tests::sh;
        use crate::worktree_hygiene::Holds;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (repo, vault) = (root.join("app"), root.join("vault"));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&vault).unwrap();
        sh(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "first"]);
        let mut items = Vec::new();
        for id in ["held", "free"] {
            let wt = root.join("Worktrees").join(id);
            sh(
                &repo,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    &format!("b/{id}"),
                    wt.to_str().unwrap(),
                    "main",
                ],
            );
            crate::build_workspaces::bind_repo_for_test(&vault, id, &repo, &wt);
            items.push(GardenItem {
                id: format!("worktree:{id}"),
                kind: "worktree".into(),
                target: id.into(),
                title: id.into(),
                reason: "Idle, clean and pushed.".into(),
                action: GardenAction::RemoveWorktree,
                undo: format!("Its branch b/{id} stays."),
            });
        }
        let fresh = GardenScan {
            date: "2026-10-09".into(),
            scanned_at: now().to_rfc3339(),
            items,
        };
        let why = "a finished build waiting to install";
        let held = root.join("Worktrees/held");
        let ids = ["worktree:held".to_string(), "worktree:free".to_string()];
        let report = apply_scan(&vault, &fresh, &ids, None, || Holds {
            paths: vec![(held.clone(), why.to_string())],
            ..none_held()
        })
        .await;
        assert_eq!(report.applied, vec!["worktree:free".to_string()]);
        assert_eq!(
            report.skipped,
            vec![(
                "worktree:held".to_string(),
                format!("Kept: {why}, since the list was shown")
            )]
        );
        assert!(held.exists());
        assert!(!root.join("Worktrees/free").exists());
        assert!(
            crate::build_workspaces::binding_for(&vault, "held")
                .unwrap()
                .exists,
            "its workspace keeps its worktree"
        );
    }

    /// Overnight R4: a Build pressed while the removal is stopping the
    /// worktree's services and checking for unsaved work. The holds are free
    /// at the first read and hold it at the second, right before `git
    /// worktree remove`: the line is skipped with why, and the worktree and
    /// its binding stay.
    #[tokio::test]
    async fn a_worktree_held_during_its_removal_is_kept_with_its_binding() {
        use crate::build_workspaces::tests::sh;
        use crate::worktree_hygiene::Holds;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let (repo, vault) = (root.join("app"), root.join("vault"));
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&vault).unwrap();
        sh(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        sh(&repo, &["add", "."]);
        sh(&repo, &["commit", "-q", "-m", "first"]);
        let wt = root.join("Worktrees").join("busy");
        sh(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "b/busy",
                wt.to_str().unwrap(),
                "main",
            ],
        );
        crate::build_workspaces::bind_repo_for_test(&vault, "busy", &repo, &wt);
        let fresh = GardenScan {
            date: "2026-10-09".into(),
            scanned_at: now().to_rfc3339(),
            items: vec![GardenItem {
                id: "worktree:busy".into(),
                kind: "worktree".into(),
                target: "busy".into(),
                title: "busy".into(),
                reason: "Idle, clean and pushed.".into(),
                action: GardenAction::RemoveWorktree,
                undo: "Its branch b/busy stays.".into(),
            }],
        };
        let why = "a build running now";
        let reads = std::cell::Cell::new(0);
        let ids = ["worktree:busy".to_string()];
        let report = apply_scan(&vault, &fresh, &ids, None, || {
            reads.set(reads.get() + 1);
            if reads.get() == 1 {
                none_held()
            } else {
                Holds {
                    paths: vec![(wt.clone(), why.to_string())],
                    ..none_held()
                }
            }
        })
        .await;
        assert_eq!(
            reads.get(),
            2,
            "read before the removal and right before git's"
        );
        assert!(report.applied.is_empty(), "{:?}", report.applied);
        assert_eq!(
            report.skipped,
            vec![(
                "worktree:busy".to_string(),
                format!("Kept: {why}, since the list was shown")
            )]
        );
        assert!(wt.exists());
        assert!(
            crate::build_workspaces::binding_for(&vault, "busy")
                .unwrap()
                .exists,
            "its workspace keeps its worktree"
        );
    }

    #[test]
    fn idle_workspaces_are_archived_except_builtin_notion_and_archived_ones() {
        let items = candidates(
            &[
                ws("old", 45),
                ws("fresh", 29),
                WorkspaceFacts {
                    builtin: true,
                    ..ws("main", 400)
                },
                WorkspaceFacts {
                    notion: true,
                    ..ws("notion-11111111111111111111111111111111", 400)
                },
                WorkspaceFacts {
                    status: "archived".into(),
                    ..ws("done", 400)
                },
                WorkspaceFacts {
                    last_active: None,
                    ..ws("never", 0)
                },
                WorkspaceFacts {
                    worktree: Some(WorktreeFacts::default()),
                    ..ws("worktree-gone", 60)
                },
                // Open plan rows: paused, not finished (overnight B2).
                WorkspaceFacts {
                    open_plan: 2,
                    ..ws("paused", 90)
                },
            ],
            &[],
            &[],
            &none_held(),
            now(),
        );
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["idle-workspace:old", "idle-workspace:worktree-gone"]
        );
        assert!(items.iter().all(|i| i.action == GardenAction::Archive));
    }

    #[test]
    fn a_duplicate_learning_keeps_the_oldest_and_a_test_page_is_listed() {
        let l = |path: &str, statement: &str, at: &str| LearningFacts {
            path: path.into(),
            title: path.into(),
            statement: statement.into(),
            learned_at: at.into(),
        };
        let items = candidates(
            &[],
            &[
                l("learnings/b.md", "Use  the NEW command map", "2026-10-05"),
                l("learnings/a.md", "use the new command map", "2026-10-01"),
                l("learnings/c.md", "Something else", "2026-10-02"),
            ],
            &[
                TaskFacts {
                    path: "tasks/hollow-test-1.md".into(),
                    title: "[Hollow test] throwaway".into(),
                    notion: false,
                },
                TaskFacts {
                    path: "tasks/renamed.md".into(),
                    title: "[Hollow test] TASK-186".into(),
                    notion: true,
                },
                TaskFacts {
                    path: "tasks/real.md".into(),
                    title: "Real work".into(),
                    notion: false,
                },
            ],
            &none_held(),
            now(),
        );
        let ids: Vec<(&str, GardenAction)> =
            items.iter().map(|i| (i.id.as_str(), i.action)).collect();
        assert_eq!(
            ids,
            vec![
                ("duplicate-learning:learnings/b.md", GardenAction::Trash),
                ("test-task:tasks/hollow-test-1.md", GardenAction::Trash),
                ("test-task:tasks/renamed.md", GardenAction::Manual),
            ]
        );
        assert!(items[0].reason.contains("learnings/a.md"));
    }

    fn task_file(v: &Path, name: &str, title: &str) {
        std::fs::create_dir_all(v.join("tasks")).unwrap();
        std::fs::write(
            v.join(format!("tasks/{name}.md")),
            format!("---\ntitle: \"{title}\"\nstatus: To do\n---\n\nBody\n"),
        )
        .unwrap();
    }

    #[test]
    fn applying_a_subset_trashes_only_the_ticked_lines_and_nothing_is_deleted() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        task_file(v, "t1", "[Hollow test] one");
        task_file(v, "t2", "[Hollow test] two");
        std::fs::write(
            v.join("tasks/t0-tracked-3dbb.md"),
            "---\ntitle: \"[Hollow test] in the tracker\"\nstatus: To do\ntracker: linear\ntracker_id: i-3dbb\n---\n\nBody\n",
        )
        .unwrap();
        let scan = scan(v, Utc::now()).unwrap();
        let ids: Vec<&str> = scan.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "test-task:tasks/t0-tracked-3dbb.md",
                "test-task:tasks/t1.md",
                "test-task:tasks/t2.md"
            ]
        );
        let report = tauri::async_runtime::block_on(apply(
            v,
            &[
                "test-task:tasks/t1.md".into(),
                "test-task:tasks/t0-tracked-3dbb.md".into(),
                "worktree:gone".into(),
            ],
            None,
            Utc::now(),
        ))
        .unwrap();
        assert_eq!(report.applied, vec!["test-task:tasks/t1.md".to_string()]);
        assert_eq!(report.skipped.len(), 2);
        assert!(report
            .skipped
            .iter()
            .any(|(id, why)| id == "worktree:gone" && why.contains("No longer")));
        assert!(report
            .skipped
            .iter()
            .any(|(id, why)| id == "test-task:tasks/t0-tracked-3dbb.md"
                && why.contains("never changes the tracker")));
        // t1 moved to the trash (undo: restore it), t2 and the tracked one untouched.
        assert!(!v.join("tasks/t1.md").exists());
        let trashed: Vec<_> = std::fs::read_dir(v.join(".trash"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(trashed.len(), 1);
        assert!(v.join("tasks/t2.md").exists());
        assert!(v.join("tasks/t0-tracked-3dbb.md").exists());
    }

    /// Review finding 7: when nothing could be tidied the card is not
    /// accepted; it says why.
    #[test]
    fn a_do_it_that_tidies_nothing_leaves_the_card_open() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        task_file(v, "gone", "[Hollow test] gone");
        post(v, &scan(v, Utc::now()).unwrap()).unwrap();
        let card = crate::review_queue::list(v, Some(REVIEW_KIND))
            .unwrap()
            .remove(0);
        std::fs::remove_file(v.join("tasks/gone.md")).unwrap();
        let err = tauri::async_runtime::block_on(resolve_review(v, &card, None)).unwrap_err();
        assert!(err.contains("Nothing was tidied"), "{err}");
        let after = crate::review_queue::list(v, Some(REVIEW_KIND))
            .unwrap()
            .remove(0);
        assert_ne!(after.status, "accepted");
        assert!(after
            .error
            .unwrap_or_default()
            .contains("No longer a candidate"));
    }

    #[test]
    fn the_card_is_one_live_item_that_follows_the_latest_scan() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        task_file(v, "t1", "[Hollow test] one");
        let first = scan(v, Utc::now()).unwrap();
        post(v, &first).unwrap();
        task_file(v, "t2", "[Hollow test] two");
        let second = scan(v, Utc::now()).unwrap();
        post(v, &second).unwrap();
        let live: Vec<_> = crate::review_queue::list(v, Some(REVIEW_KIND))
            .unwrap()
            .into_iter()
            .filter(|i| i.status == "pending")
            .collect();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].payload["title"], "2 things can be tidied");
        // The person's Do it with one line ticked.
        let edited = serde_json::json!({ "ticked": ["test-task:tasks/t2.md"] });
        let report =
            tauri::async_runtime::block_on(resolve_review(v, &live[0], Some(&edited))).unwrap();
        assert_eq!(report.applied, vec!["test-task:tasks/t2.md".to_string()]);
        assert!(v.join("tasks/t1.md").exists());
        // Nothing left: the card goes.
        std::fs::remove_file(v.join("tasks/t1.md")).unwrap();
        post(v, &scan(v, Utc::now()).unwrap()).unwrap();
        assert!(crate::review_queue::list(v, Some(REVIEW_KIND))
            .unwrap()
            .iter()
            .all(|i| i.status != "pending"));
    }

    /// Overnight A10: a conflict copy is a line the gardener never applies,
    /// even when its id is ticked.
    #[tokio::test]
    async fn a_conflict_copy_is_listed_and_never_applied() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("notes")).unwrap();
        std::fs::write(v.join("notes/Plan.md"), "a").unwrap();
        std::fs::write(v.join("notes/Plan 2.md"), "b").unwrap();
        crate::vault::invalidate_walk_cache();
        let lines = conflict_lines(&crate::conflict_copies::scan(v));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].id, "conflict-copy:notes/Plan 2.md");
        assert_eq!(lines[0].action, GardenAction::Resolve);
        assert!(
            lines[0].reason.contains("notes/Plan.md that differs"),
            "{}",
            lines[0].reason
        );
        let fresh = scan(v, Utc::now()).unwrap();
        assert!(fresh.items.iter().any(|i| i.id == lines[0].id));
        let out = apply(v, &[lines[0].id.clone()], None, Utc::now())
            .await
            .unwrap();
        assert!(v.join("notes/Plan 2.md").exists(), "never applied: {out:?}");
    }

    /// Q-R7 (2026-11-13): the scan's git is the hardened helper, so a
    /// planted submodule entry for a decoy with a `core.fsmonitor` payload in
    /// an idle worktree runs nothing in Hollow's process. Real git.
    #[test]
    fn the_scan_runs_no_planted_submodule_payload() {
        let git = |dir: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
                .args(args)
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let top = root_path.join("repo");
        std::fs::create_dir_all(&top).unwrap();
        git(&top, &["init", "-q", "-b", "main"]);
        std::fs::write(top.join("a"), "a").unwrap();
        git(&top, &["add", "a"]);
        git(&top, &["commit", "-q", "-m", "init"]);
        // A decoy repository with a `core.fsmonitor` payload, and a planted
        // `160000` entry for it in the index.
        let decoy = top.join("decoy");
        std::fs::create_dir_all(&decoy).unwrap();
        git(&decoy, &["init", "-q", "."]);
        git(&decoy, &["commit", "-q", "--allow-empty", "-m", "d"]);
        let sha = git(&decoy, &["rev-parse", "HEAD"]);
        let ran = root_path.join("RAN");
        git(
            &decoy,
            &[
                "config",
                "core.fsmonitor",
                &format!("touch {}; true", ran.display()),
            ],
        );
        git(
            &top,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{sha},decoy"),
            ],
        );
        git(&top, &["status", "--porcelain"]);
        assert!(ran.exists(), "control: plain git status runs the payload");
        std::fs::remove_file(&ran).unwrap();
        let dirty = ["status", "--porcelain", "--ignore-submodules=dirty"];
        assert_eq!(git_count(&top, &dirty), Some(1), "the staged entry counts");
        assert!(git_count(&top, &["status", "--porcelain"]).is_some());
        assert_eq!(ignored_kept(&top), Some(0));
        assert!(git_head(&top).is_some());
        assert!(!ran.exists(), "the scan ran the planted fsmonitor");
    }
}
