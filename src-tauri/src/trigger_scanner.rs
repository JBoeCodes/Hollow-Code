//! Triggered agents — the LIVE filesystem facts behind the pure
//! [`crate::trigger_evaluator`] decision, plus the `.workflow-triggers.json`
//! watermark IO. Mirrors the heartbeat's live FS walkers + JSON state IO.
//!
//! The watermark file is dot-prefixed, which keeps the watermark file ITSELF
//! out of the watcher and out of the `changed_files` facts. It does not stop a
//! triggered run's own writes from re-triggering it: for a `file-changed`
//! glob that matches the run's outputs, the committed watermark is the
//! pre-run max mtime, so the run's later writes fire it again on a later
//! tick (PR 9's business). `workflows::enqueue_due` commits a firing
//! workflow's advanced watermark once the fire is ENQUEUED on the run queue
//! (Paperclip PR 7; delivery is at least once), and a re-fire while that row
//! is still queued coalesces into it. The row records the fire's facts, and
//! the pump's claim re-checks them, so a task
//! that is no longer overdue never runs late. Port of hollow-mac's
//! `TriggerScanner`.
//!
//! [`changed_since_in`] (Paperclip PR 9b) is the quiet-skip walk: every file
//! under one watch root whose change time is after `since`, with no age
//! cutoff. It shares the pruned walker with `changed_files`, but it uses
//! `max(mtime, ctime)` (content copied in with a preserved mtime still
//! counts), fails with `Err` instead of skipping on any walk error, and caps
//! the entries it visits. Every failure leans toward running the workflow.
//! The walk reports every change, Hollow's own included: the quiet gate that
//! calls it ignores a previous run's write only when that run's manifest
//! holds the file's current hash AND a completed structured write tool event
//! confirmed it (user decision, 2026-09-25), so a Codex `file_change` or a
//! shell-written output still counts as a change.

use crate::trigger_evaluator::{ChangedFile, TriggerFacts, TriggerWatermark};
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

/// The watermark file at the vault root. Dot-prefixed → ignored by the watcher.
pub const STATE_FILE: &str = ".workflow-triggers.json";

/// Only files modified within this window are reported as "changed" — bounds the
/// walk + means triggers react to RECENT activity (the watermark then dedups).
const CHANGE_WINDOW_SECS: i64 = 7 * 24 * 3600;

fn mtime_utc(meta: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    meta.modified().ok().map(DateTime::<Utc>::from)
}

/// Assemble the live `TriggerFacts` for `now`. The task ids are injected
/// ([`task_trigger_ids`]; task parsing, bucketing and blocker resolution live
/// in `vault_tasks`, not duplicated here).
pub fn live_facts(vault: &Path, now: DateTime<Utc>, task_ids: TaskTriggerIds) -> TriggerFacts {
    TriggerFacts {
        now_utc: now,
        changed_files: changed_files(vault, now),
        overdue_task_ids: task_ids.overdue,
        unblocked_task_ids: task_ids.unblocked,
    }
}

/// True if any path component begins with `.` or `_` (the watcher's ignore rule),
/// so the state file + hidden/underscore dirs never count as a change.
fn ignored_rel(rel: &str) -> bool {
    ["emails", "teams", "slack"].contains(&rel.split('/').next().unwrap_or_default())
        || rel
            .split('/')
            .any(|c| c.starts_with('.') || c.starts_with('_'))
}

/// Trigger facts: vault files modified within the last 7 days (by mtime).
/// Walks through [`walk_pruned`] with [`ScanIgnore::Vault`], so it no longer
/// descends into `.git`, `.hollow-history` or the message folders. Pruning a
/// directory is equivalent to skipping each file below it, because
/// [`ignored_rel`] holds for every descendant of an ignored directory, so the
/// set is exactly what the old walk-everything loop returned (pinned by
/// `pruned_changed_files_matches_the_unpruned_walk`).
fn changed_files(vault: &Path, now: DateTime<Utc>) -> Vec<ChangedFile> {
    let cutoff = now - chrono::Duration::seconds(CHANGE_WINDOW_SECS);
    let mut out = Vec::new();
    for entry in walk_pruned(vault, vault, ScanIgnore::Vault).filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(vault) else {
            continue;
        };
        let Some(rel) = rel.to_str() else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        let Some(mtime) = mtime_utc(&meta) else {
            continue;
        };
        if mtime < cutoff {
            continue;
        }
        out.push(ChangedFile {
            path: rel.to_string(),
            mtime,
        });
    }
    out
}

// ── Quiet-skip change walk (Paperclip PR 9) ──────────────────────────────────

/// Which paths a change walk prunes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanIgnore {
    /// Vault paths: [`ignored_rel`] on base-relative paths (any `.`- or
    /// `_`-prefixed component, and the retired `emails/`, `teams/`, `slack/`
    /// folders). The same rule as the watcher and the trigger facts.
    Vault,
    /// A folder outside the vault (a code repo): prunes only the VCS, build
    /// and cache directories in [`EXTERNAL_PRUNED_DIRS`] below the root, and
    /// [`EXTERNAL_IGNORED_FILES`]. It never prunes a path just for a `.` or
    /// `_` prefix, so `_app.tsx`, `__init__.py`, `__tests__/` and `.github/`
    /// are still seen.
    External,
}

/// Directories an [`ScanIgnore::External`] walk never descends into.
pub const EXTERNAL_PRUNED_DIRS: [&str; 12] = [
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".turbo",
    "__pycache__",
    ".venv",
    ".hollow-attachments",
];

/// Files an [`ScanIgnore::External`] walk never reports.
pub const EXTERNAL_IGNORED_FILES: [&str; 1] = [".DS_Store"];

/// How a [`changed_since_in`] walk prunes and how far it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanOptions {
    pub ignore: ScanIgnore,
    /// The walk fails (`Err`) once it has visited more entries than this,
    /// directories included.
    pub max_entries: usize,
}

impl ScanOptions {
    /// The default visit cap for both kinds of walk.
    pub const MAX_ENTRIES: usize = 100_000;

    #[cfg(test)]
    pub fn vault() -> Self {
        Self {
            ignore: ScanIgnore::Vault,
            max_entries: Self::MAX_ENTRIES,
        }
    }

    #[cfg(test)]
    pub fn external() -> Self {
        Self {
            ignore: ScanIgnore::External,
            max_entries: Self::MAX_ENTRIES,
        }
    }
}

/// Walk `start` (a directory under `base`) without following symlinks,
/// pruning per `ignore` on paths relative to `base`. The root itself
/// (`depth() == 0`) is never pruned: a tempdir root is `.tmpXXXX`, and a
/// dot-prefixed root must not empty the walk (the `walk_notes` quirk in
/// CLAUDE.md → Quirks to know about). Walk errors are passed through for the
/// caller to decide.
fn walk_pruned(
    base: &Path,
    start: &Path,
    ignore: ScanIgnore,
) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> {
    let base = base.to_path_buf();
    WalkDir::new(start)
        .follow_links(false)
        .into_iter()
        .filter_entry(move |entry| keep_entry(&base, entry, ignore))
}

fn keep_entry(base: &Path, entry: &walkdir::DirEntry, ignore: ScanIgnore) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    match ignore {
        ScanIgnore::Vault => match entry
            .path()
            .strip_prefix(base)
            .ok()
            .and_then(|r| r.to_str())
        {
            Some(rel) => !ignored_rel(rel),
            // Not UTF-8: kept here; `changed_files` skips such files and
            // `changed_since_in` reports them lossily (a change, never quiet).
            None => true,
        },
        ScanIgnore::External => {
            let Some(name) = entry.file_name().to_str() else {
                return true;
            };
            if entry.file_type().is_dir() {
                !EXTERNAL_PRUNED_DIRS.contains(&name)
            } else {
                !EXTERNAL_IGNORED_FILES.contains(&name)
            }
        }
    }
}

// Test seam for `changed_since_in`: when set on the walking thread, a `Some`
// it returns for a file's path replaces that file's change time, so a test
// can place changes days in the past (setting an mtime moves the ctime to
// now, which no test can undo).
#[cfg(test)]
thread_local! {
    pub(crate) static CHANGE_TIME_OVERRIDE: std::cell::RefCell<
        Option<Box<dyn Fn(&Path) -> Option<DateTime<Utc>>>>,
    > = const { std::cell::RefCell::new(None) };
}

/// [`changed_since_in`] over the vault with the vault's ignore rules (the
/// quiet check calls `changed_since_in` directly, since its base may be a
/// folder outside the vault).
#[cfg(test)]
pub fn changed_since(
    vault: &Path,
    root_rel: &str,
    since: DateTime<Utc>,
) -> Result<Vec<ChangedFile>, String> {
    changed_since_in(vault, root_rel, since, ScanOptions::vault())
}

/// Every file under `base/root_rel` whose change time, `max(mtime, ctime)`,
/// is strictly after `since`. There is no age cutoff. Paths are relative to
/// `base` and sorted; a returned [`ChangedFile::mtime`] holds the change time.
///
/// `Err` (so the caller runs the workflow instead of skipping it) when
/// `root_rel` is absolute or has a `..` component, when a vault walk's root is
/// itself ignored (`_inbox`, `.obsidian`, `emails/…`), when it is not an
/// existing directory ("watch root missing"), on any walk or metadata error, and once
/// more than `opts.max_entries` entries have been visited.
///
/// Symlinks are not followed while walking. A symlink to a regular file is
/// reported when the link's or the target's change time is after `since`; a
/// symlink to a directory is not descended, and a broken one is skipped.
/// Deletions and renames leave nothing to find, so they are never reported.
/// Called by `workflow_hygiene::quiet_check` (Paperclip PR 9b).
pub fn changed_since_in(
    base: &Path,
    root_rel: &str,
    since: DateTime<Utc>,
    opts: ScanOptions,
) -> Result<Vec<ChangedFile>, String> {
    let start = watch_root(base, root_rel, opts.ignore)?;
    let mut out = Vec::new();
    let mut visited: usize = 0;
    for entry in walk_pruned(base, &start, opts.ignore) {
        let entry = entry.map_err(|e| format!("watch walk failed: {e}"))?;
        visited += 1;
        if visited > opts.max_entries {
            return Err(format!(
                "watch walk visited more than {} entries",
                opts.max_entries
            ));
        }
        let file_type = entry.file_type();
        if file_type.is_dir() {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| format!("watch walk failed: {e}"))?;
        let changed = if file_type.is_symlink() {
            match std::fs::metadata(entry.path()) {
                Ok(target) if target.is_file() => later(change_time(&meta), change_time(&target)),
                _ => continue,
            }
        } else if file_type.is_file() {
            change_time(&meta)
        } else {
            // Sockets, FIFOs and devices are not content.
            continue;
        };
        #[cfg(test)]
        let changed = CHANGE_TIME_OVERRIDE
            .with(|seam| seam.borrow().as_ref().and_then(|at| at(entry.path())))
            .or(changed);
        let Some(changed) = changed else {
            return Err(format!(
                "watch walk could not read a change time for {}",
                entry.path().display()
            ));
        };
        if changed <= since {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(base)
            .map_err(|_| format!("watch walk left its base: {}", entry.path().display()))?;
        out.push(ChangedFile {
            path: rel.to_string_lossy().into_owned(),
            mtime: changed,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `base/root_rel`, when `root_rel` is relative, has no `..`, and names an
/// existing directory (a symlinked root is followed, like `WalkDir`'s root).
/// A vault walk also refuses a root the vault rules ignore: its children
/// would all be pruned, and an empty walk reads as "quiet", which must never
/// be the result of a misconfiguration.
fn watch_root(base: &Path, root_rel: &str, ignore: ScanIgnore) -> Result<PathBuf, String> {
    let rel = Path::new(root_rel);
    if rel.is_absolute()
        || rel
            .components()
            .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(format!(
            "watch root must be a relative path without '..': {root_rel}"
        ));
    }
    if ignore == ScanIgnore::Vault {
        let normalized = rel
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if !normalized.is_empty() && ignored_rel(&normalized) {
            return Err(format!(
                "watch root is ignored by the vault rules: {root_rel}"
            ));
        }
    }
    let start = base.join(rel);
    match std::fs::metadata(&start) {
        Ok(meta) if meta.is_dir() => Ok(start),
        _ => Err(format!("watch root missing: {root_rel}")),
    }
}

fn later(a: Option<DateTime<Utc>>, b: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

/// `max(mtime, ctime)`. The inode change time moves whenever content arrives
/// with a preserved mtime (iCloud, Obsidian Sync, `cp -p`, `rsync -a`,
/// `unzip`). It also moves on xattr, permission and ownership changes, which
/// makes quiet-skip run more often, never less.
fn change_time(meta: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    later(mtime_utc(meta), ctime_utc(meta))
}

#[cfg(unix)]
fn ctime_utc(meta: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    use std::os::unix::fs::MetadataExt;
    let nanos = u32::try_from(meta.ctime_nsec()).ok()?;
    DateTime::<Utc>::from_timestamp(meta.ctime(), nanos)
}

#[cfg(not(unix))]
fn ctime_utc(_meta: &std::fs::Metadata) -> Option<DateTime<Utc>> {
    None
}

/// The task facts behind the `task-overdue` and `task-unblocked` triggers,
/// from one [`crate::vault_tasks::list`] pass. Every id is the task's
/// `vault:tasks/…` URL (`Task.url`; stable across title renames, unlike the
/// slug).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskTriggerIds {
    /// Non-terminal tasks whose `due` date is strictly before today (local).
    pub overdue: HashSet<String>,
    /// Tasks with a NON-EMPTY `blocked_by` whose blockers all resolved, and
    /// whose own status is neither terminal nor cancelled. An explicit hold
    /// status (`Waiting`, `On hold`) is deliberately included: that is the
    /// task whose unblocking should prompt someone, although it is not
    /// `ready`.
    pub unblocked: HashSet<String>,
}

/// One task-list pass for both task triggers ([`TaskTriggerIds`]).
pub fn task_trigger_ids(vault: &Path) -> TaskTriggerIds {
    use crate::vault_tasks::{is_cancelled_status, is_terminal_status};
    let today = chrono::Local::now().date_naive();
    let mut ids = TaskTriggerIds::default();
    for t in crate::vault_tasks::list(vault) {
        if is_terminal_status(&t.status) {
            continue;
        }
        let overdue = t
            .due
            .as_deref()
            .and_then(|due| chrono::NaiveDate::parse_from_str(due.trim(), "%Y-%m-%d").ok())
            .is_some_and(|d| d < today);
        if overdue {
            ids.overdue.insert(t.url.clone());
        }
        if !t.blocked_by.is_empty()
            && t.unresolved_blockers.is_empty()
            && !is_cancelled_status(&t.status)
        {
            ids.unblocked.insert(t.url.clone());
        }
    }
    ids
}

/// Overdue task ids (the `task-overdue` fact): [`task_trigger_ids`]`.overdue`.
#[cfg(test)]
pub fn overdue_task_ids(vault: &Path) -> HashSet<String> {
    task_trigger_ids(vault).overdue
}

// ── Watermark IO (`.workflow-triggers.json`) ─────────────────────────────────

type StateMap = HashMap<String, HashMap<String, TriggerWatermark>>;

/// Load the per-(slug,token) watermark map; any failure → empty (default-on-fail).
pub fn load_state(vault: &Path) -> StateMap {
    let path = vault.join(STATE_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the watermark map (pretty, atomic so a crash mid-write can't corrupt).
/// A host write: attributed so a run spanning a watermark save isn't flagged
/// (a direct agent edit of the file still is — see `run_guard`).
pub fn save_state(vault: &Path, state: &StateMap) {
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = crate::run_guard::write_host_state(vault, STATE_FILE, json.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_folders_never_trigger_workflows_but_calendar_still_does() {
        // Deliberately a plain `.tmpXXXX` tempdir root: a regression pin for
        // the pruned walker, which must never prune its own root.
        let dir = tempfile::tempdir().unwrap();
        for folder in ["emails/inbox", "teams/dms", "slack/dms", "calendar"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
            std::fs::write(dir.path().join(folder).join("note.md"), "context").unwrap();
        }
        let facts = live_facts(dir.path(), Utc::now(), TaskTriggerIds::default());
        assert_eq!(facts.changed_files.len(), 1);
        assert_eq!(facts.changed_files[0].path, "calendar/note.md");
    }
    use std::fs;

    fn vault_dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    #[test]
    fn watermark_state_round_trips() {
        let dir = vault_dir();
        let v = dir.path();
        let mut state: StateMap = HashMap::new();
        let mut wf: HashMap<String, TriggerWatermark> = HashMap::new();
        wf.insert(
            "task-overdue".into(),
            TriggerWatermark {
                last_overdue_ids: vec!["a".into(), "b".into()],
                ..Default::default()
            },
        );
        state.insert("my-workflow".into(), wf);
        save_state(v, &state);
        let loaded = load_state(v);
        assert_eq!(
            loaded["my-workflow"]["task-overdue"].last_overdue_ids,
            vec!["a", "b"]
        );
        // Missing file → empty.
        assert!(load_state(vault_dir().path()).is_empty());
    }

    #[test]
    fn overdue_ids_picks_past_due_non_terminal() {
        let dir = vault_dir();
        let v = dir.path();
        fs::create_dir_all(v.join("tasks")).unwrap();
        fs::write(
            v.join("tasks/late.md"),
            "---\ntitle: Late\nstatus: Not started\ndue: 2020-01-01\n---\n",
        )
        .unwrap();
        fs::write(
            v.join("tasks/done-late.md"),
            "---\ntitle: Done late\nstatus: Delivered\ndue: 2020-01-01\n---\n",
        )
        .unwrap();
        fs::write(
            v.join("tasks/future.md"),
            "---\ntitle: Future\nstatus: Not started\ndue: 2099-01-01\n---\n",
        )
        .unwrap();
        let ids = overdue_task_ids(v);
        assert!(ids.iter().any(|id| id.contains("tasks/late.md")));
        assert!(!ids.iter().any(|id| id.contains("done-late")));
        assert!(!ids.iter().any(|id| id.contains("future")));
        // The ids are the tasks' vault: URLs (what the run queue re-checks).
        assert_eq!(ids, HashSet::from(["vault:tasks/late.md".to_string()]));
    }

    // ── task-unblocked facts (Paperclip PR 11b) ──────────────────────────────

    #[test]
    fn unblocked_ids_hold_only_open_tasks_whose_nonempty_blockers_all_resolved() {
        let dir = vault_dir();
        let v = dir.path();
        let task = |rel: &str, status: &str, blocked_by: &str| {
            let extra = if blocked_by.is_empty() {
                String::new()
            } else {
                format!("blocked_by: \"{blocked_by}\"\n")
            };
            put(
                v,
                rel,
                &format!("---\ntitle: {rel}\nstatus: {status}\n{extra}---\n"),
            );
        };
        task("tasks/done-blocker.md", "Done", "");
        task("tasks/open-blocker.md", "In progress", "");
        // a: every blocker resolved → included.
        task("tasks/a.md", "Not started", "vault:tasks/done-blocker.md");
        // b: one blocker still open → excluded.
        task(
            "tasks/b.md",
            "Not started",
            "vault:tasks/done-blocker.md, vault:tasks/open-blocker.md",
        );
        // c: no blocked_by at all → excluded (the set is non-empty blocked_by only).
        task("tasks/c.md", "Not started", "");
        // d: finished itself → excluded.
        task("tasks/d.md", "Delivered", "vault:tasks/done-blocker.md");
        // e: its blocker is missing (never resolves) → excluded.
        task("tasks/e.md", "Not started", "vault:tasks/gone.md");
        // f: own status Cancelled with a Done blocker → excluded.
        task("tasks/f.md", "Cancelled", "vault:tasks/done-blocker.md");
        // g: own status Waiting with a Done blocker → INCLUDED (hold statuses
        // are exactly what the trigger should announce).
        task("tasks/g.md", "Waiting", "vault:tasks/done-blocker.md");
        let ids = task_trigger_ids(v);
        assert_eq!(
            ids.unblocked,
            HashSet::from([
                "vault:tasks/a.md".to_string(),
                "vault:tasks/g.md".to_string()
            ])
        );
        assert!(ids.overdue.is_empty());
        let facts = live_facts(v, Utc::now(), ids.clone());
        assert_eq!(facts.unblocked_task_ids, ids.unblocked);
    }

    #[test]
    fn old_watermarks_load_without_unblocked_ids() {
        let dir = vault_dir();
        let v = dir.path();
        let old = r#"{"wf":{"task-overdue":{"lastOverdueIds":["vault:tasks/a.md"],"firedMeetingPaths":[]}}}"#;
        fs::write(v.join(STATE_FILE), old).unwrap();
        let loaded = load_state(v);
        assert_eq!(loaded["wf"]["task-overdue"].last_unblocked_ids, None);
        save_state(v, &loaded);
        let saved = fs::read_to_string(v.join(STATE_FILE)).unwrap();
        assert!(!saved.contains("lastUnblockedIds"), "{saved}");
        assert_eq!(load_state(v), loaded);
    }

    // ── Quiet-skip change walk (Paperclip PR 9) ──────────────────────────────

    fn put(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// Set only the mtime. The kernel moves the ctime to now as it does so,
    /// which is exactly what a sync client preserving an old mtime looks like.
    fn set_mtime(path: &Path, at: DateTime<Utc>) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::from(at)))
            .unwrap();
    }

    fn paths(files: &[ChangedFile]) -> Vec<String> {
        files.iter().map(|f| f.path.clone()).collect()
    }

    /// The pre-PR-9 `changed_files`, kept verbatim as the oracle for the
    /// pruned walk: it visits everything and filters each file.
    fn changed_files_unpruned(vault: &Path, now: DateTime<Utc>) -> Vec<ChangedFile> {
        let cutoff = now - chrono::Duration::seconds(CHANGE_WINDOW_SECS);
        let mut out = Vec::new();
        for entry in WalkDir::new(vault)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(vault) else {
                continue;
            };
            let Some(rel) = rel.to_str() else { continue };
            if ignored_rel(rel) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Some(mtime) = mtime_utc(&meta) else {
                continue;
            };
            if mtime < cutoff {
                continue;
            }
            out.push(ChangedFile {
                path: rel.to_string(),
                mtime,
            });
        }
        out
    }

    /// Review: the quiet walk must never grow the trigger facts' 7-day
    /// window (a workflow idle for a week would then read quiet forever).
    #[test]
    fn the_quiet_walk_never_references_the_trigger_window() {
        const SRC: &str = include_str!("trigger_scanner.rs");
        let start = SRC.find(concat!("pub fn ", "changed_since_in(")).unwrap();
        let end = start + SRC[start..].find("\n}\n").unwrap();
        let body = &SRC[start..end];
        assert!(!body.contains(concat!("CHANGE_", "WINDOW_SECS")));
        assert!(!body.contains("cutoff"));
    }

    #[test]
    fn changed_since_has_no_seven_day_cutoff_and_sees_preserved_mtimes() {
        let dir = vault_dir();
        let v = dir.path();
        let now = Utc::now();
        put(v, "notes/old.md", "old");
        set_mtime(&v.join("notes/old.md"), now - chrono::Duration::days(10));

        // The trigger facts keep their 7-day window...
        assert!(!paths(&changed_files(v, now)).contains(&"notes/old.md".to_string()));
        // ...the quiet-skip walk has no cutoff at all.
        let got = changed_since(v, "", now - chrono::Duration::days(11)).unwrap();
        assert_eq!(paths(&got), vec!["notes/old.md"]);

        // Nothing has changed after a `since` in the future.
        let later_since = Utc::now() + chrono::Duration::hours(1);
        assert!(changed_since(v, "", later_since).unwrap().is_empty());

        // Content copied in with a preserved, older mtime: the mtime is before
        // `since`, but the inode change time is new, so it is a change.
        #[cfg(unix)]
        {
            let since = Utc::now() - chrono::Duration::seconds(5);
            put(v, "notes/synced.md", "arrived with an old mtime");
            set_mtime(&v.join("notes/synced.md"), now - chrono::Duration::days(2));
            let meta = fs::metadata(v.join("notes/synced.md")).unwrap();
            assert!(mtime_utc(&meta).unwrap() < since);
            let got = changed_since(v, "notes", since).unwrap();
            assert!(paths(&got).contains(&"notes/synced.md".to_string()));
            let synced = got.iter().find(|f| f.path == "notes/synced.md").unwrap();
            assert!(synced.mtime > since, "the reported time is the change time");
        }
    }

    #[test]
    fn vault_and_external_ignore_rules_differ() {
        let since = Utc::now() - chrono::Duration::hours(1);

        let vd = vault_dir();
        let v = vd.path();
        for rel in [
            ".git/HEAD",
            ".hollow-history/runs/r.json",
            "_calendar/week.json",
            "emails/inbox/a.md",
            "notes/_draft.md",
            "workspaces/w/PLAN.md",
        ] {
            put(v, rel, "x");
        }
        assert_eq!(
            paths(&changed_since(v, "", since).unwrap()),
            vec!["workspaces/w/PLAN.md"]
        );

        let rd = tempfile::Builder::new().prefix("repo-").tempdir().unwrap();
        let r = rd.path();
        for rel in [
            "pages/_app.tsx",
            "pkg/__init__.py",
            "__tests__/a.ts",
            ".github/workflows/ci.yml",
            ".git/config",
            "node_modules/x/index.js",
            "web/node_modules/y.js",
            "target/debug/out",
            ".DS_Store",
            "src/.DS_Store",
        ] {
            put(r, rel, "x");
        }
        assert_eq!(
            paths(&changed_since_in(r, "", since, ScanOptions::external()).unwrap()),
            vec![
                ".github/workflows/ci.yml",
                "__tests__/a.ts",
                "pages/_app.tsx",
                "pkg/__init__.py",
            ]
        );
        // The vault rules on the same repo would hide exactly the source
        // files and keep the build output.
        assert_eq!(
            paths(&changed_since_in(r, "", since, ScanOptions::vault()).unwrap()),
            vec![
                "node_modules/x/index.js",
                "target/debug/out",
                "web/node_modules/y.js"
            ]
        );

        // A missing root and a walk over its visit cap both fail.
        assert!(changed_since(v, "nope", since)
            .unwrap_err()
            .starts_with("watch root missing"));
        for i in 0..10 {
            put(v, &format!("many/n{i}.md"), "x");
        }
        let capped = ScanOptions {
            max_entries: 3,
            ..ScanOptions::vault()
        };
        assert!(changed_since_in(v, "many", since, capped)
            .unwrap_err()
            .contains("more than 3"));
        assert_eq!(changed_since(v, "many", since).unwrap().len(), 10);
    }

    #[test]
    fn pruned_changed_files_matches_the_unpruned_walk() {
        let vd = vault_dir();
        let v = vd.path();
        for rel in [
            "notes/a.md",
            "notes/deep/er/b.md",
            "notes/emails.md",
            "notes/teams/t.md",
            "notes/~backup.md",
            "emailsx/e.md",
            "tasks/t.md",
            "a/_b/c.md",
            "a/.c/d.md",
            "_calendar/week.json",
            ".git/objects/x",
            ".hollow-history/objects/y",
            ".workflow-triggers.json",
            "emails/inbox/e.md",
            "teams/dms/t.md",
            "slack/dms/s.md",
        ] {
            put(v, rel, "x");
        }
        put(v, "notes/stale.md", "x");
        set_mtime(
            &v.join("notes/stale.md"),
            Utc::now() - chrono::Duration::days(9),
        );

        let now = Utc::now();
        let key = |mut files: Vec<ChangedFile>| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            files
                .into_iter()
                .map(|f| (f.path, f.mtime))
                .collect::<Vec<_>>()
        };
        let pruned = key(changed_files(v, now));
        assert_eq!(pruned, key(changed_files_unpruned(v, now)));
        let kept: Vec<&str> = pruned.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            kept,
            vec![
                "emailsx/e.md",
                "notes/a.md",
                "notes/deep/er/b.md",
                "notes/emails.md",
                "notes/teams/t.md",
                "notes/~backup.md",
                "tasks/t.md",
            ]
        );
    }

    #[test]
    fn change_walks_never_prune_a_dot_prefixed_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(root
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.')));
        put(root, "notes/n.md", "x");
        let since = Utc::now() - chrono::Duration::hours(1);
        assert_eq!(
            paths(&changed_since(root, "", since).unwrap()),
            vec!["notes/n.md"]
        );
        assert_eq!(
            paths(&changed_since_in(root, "", since, ScanOptions::external()).unwrap()),
            vec!["notes/n.md"]
        );
        assert_eq!(paths(&changed_files(root, Utc::now())), vec!["notes/n.md"]);
    }

    #[test]
    fn watch_roots_must_be_existing_relative_unignored_directories() {
        let vd = vault_dir();
        let v = vd.path();
        put(v, "notes/n.md", "x");
        put(v, "_inbox/i.md", "x");
        let since = Utc::now() - chrono::Duration::hours(1);
        assert!(changed_since(v, "../x", since).is_err());
        assert!(changed_since(v, "/etc", since).is_err());
        assert!(changed_since(v, "notes/n.md", since)
            .unwrap_err()
            .starts_with("watch root missing"));
        // A vault root the vault rules ignore would walk to nothing and read
        // as quiet, so it is refused rather than walked.
        assert!(changed_since(v, "_inbox", since)
            .unwrap_err()
            .contains("ignored"));
        assert!(changed_since(v, "./_inbox", since)
            .unwrap_err()
            .contains("ignored"));
        assert!(changed_since(v, "emails/inbox", since)
            .unwrap_err()
            .contains("ignored"));
        // An external walk has no such rule.
        assert_eq!(
            paths(&changed_since_in(v, "_inbox", since, ScanOptions::external()).unwrap()),
            vec!["_inbox/i.md"]
        );
        assert_eq!(
            paths(&changed_since(v, "notes", since).unwrap()),
            vec!["notes/n.md"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_counts_and_a_symlinked_directory_is_not_walked() {
        let vd = vault_dir();
        let v = vd.path();
        let od = tempfile::Builder::new()
            .prefix("outside-")
            .tempdir()
            .unwrap();
        let o = od.path();
        put(o, "target.md", "x");
        put(o, "dir/inner.md", "x");
        fs::create_dir_all(v.join("notes")).unwrap();
        std::os::unix::fs::symlink(o.join("target.md"), v.join("notes/link.md")).unwrap();
        std::os::unix::fs::symlink(o.join("dir"), v.join("notes/dirlink")).unwrap();
        std::os::unix::fs::symlink(o.join("gone.md"), v.join("notes/broken.md")).unwrap();
        let since = Utc::now() - chrono::Duration::hours(1);
        assert_eq!(
            paths(&changed_since(v, "notes", since).unwrap()),
            vec!["notes/link.md"]
        );
    }
}
