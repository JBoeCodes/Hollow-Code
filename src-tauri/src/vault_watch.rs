//! Live vault file-watching.
//!
//! The vault is the local operating surface for every tab, but Hollow doesn't
//! *own* the files — Obsidian, the embedded Claude, the Notion/Plaud/email/
//! calendar syncs, and a plain text editor all write to the same folders.
//! Without a watcher the UI mirrors the vault only as of the last fetch, so an
//! external edit (or a change made through the app's own background sync)
//! doesn't show up until a `Cmd+R`.
//!
//! This module places one recursive watcher on the vault root. Raw filesystem
//! events are noisy and bursty (an editor writes a temp file, renames it,
//! touches mtimes), so we coalesce a burst within a short debounce window into
//! a single [`VaultChange`] payload and emit it on the `vault-changed` Tauri
//! event. The frontend (`store.ts`) maps the touched top-level folders to the
//! specific resources to refetch and live-reloads the open note when it's the
//! one that changed.
//!
//! Design notes:
//! - The debounce loop blocks on a channel, so it runs on a dedicated
//!   `std::thread` (never a tokio worker). The `notify` watcher itself runs on
//!   its own backend thread; we keep it alive by moving it into that thread.
//! - We never react to our own *reads* (`EventKind::Access`) and ignore
//!   tool/editor internals — anything under a dot- or underscore-prefixed
//!   path component (`.git`, `.obsidian`, `.task-filters.json`,
//!   `.notifications.jsonl`, `_calendar`) plus `~`-suffixed backup files.
//! - App-made writes also trip the watcher; the resulting refetch just
//!   re-reads what we wrote (idempotent), and the open-note reload compares
//!   content so it never disrupts the editor on an autosave.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::Serialize;

/// Tauri event name the frontend listens on.
const EVENT: &str = "vault-changed";
/// Coalesce a burst of filesystem events within this window into one emit.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// Cap the per-emit path list so a bulk write (e.g. an email backfill of
/// hundreds of files) doesn't ship a huge payload — the `folders` set still
/// drives the right refetches regardless of how many individual paths changed.
const MAX_PATHS: usize = 200;

/// Only the first call per process actually starts the watcher.
static STARTED: AtomicBool = AtomicBool::new(false);

/// What changed in the vault since the last emit. `folders` is the distinct
/// set of top-level folders touched (drives which resources the UI refetches);
/// `paths` is the vault-relative file paths (forward-slashed), capped — the UI
/// uses them to decide whether the currently-open note should live-reload.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VaultChange {
    pub folders: Vec<String>,
    pub paths: Vec<String>,
}

/// Start watching `vault` recursively and emit `vault-changed` on every
/// debounced burst. Idempotent and non-fatal: a failure to create or arm the
/// watcher logs and returns, leaving the app fully functional (just without
/// live updates — a `Cmd+R` still works).
pub(crate) fn start(app: impl Into<crate::core_host::Core>, config: Arc<crate::config::Config>) {
    let app = app.into();
    if STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    // Canonicalize so prefix-stripping is robust against symlinks / `..` —
    // FSEvents reports canonical absolute paths, so the watch root must match.
    let vault = config.vault_path.clone();
    let root = std::fs::canonicalize(&vault).unwrap_or(vault);

    let (tx, rx) = channel::<notify::Result<Event>>();
    let mut watcher = match notify::recommended_watcher(move |res| {
        // The send only fails once the debounce thread has gone away (app
        // shutdown); nothing to do in that case.
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("[vault-watch] failed to create watcher: {e}");
            STARTED.store(false, Ordering::SeqCst);
            return;
        }
    };
    if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
        eprintln!("[vault-watch] failed to watch {}: {e}", root.display());
        STARTED.store(false, Ordering::SeqCst);
        return;
    }

    std::thread::spawn(move || {
        // Hold the watcher for the life of this thread — dropping it stops the
        // backend watch.
        let _watcher = watcher;
        // Block until the first event of a burst; an `Err` here means the
        // sender is gone (app shutting down), so end the thread.
        while let Ok(first) = rx.recv() {
            if let Some(change) = build_burst(&root, first, &rx) {
                // A real external change landed; patch the cached link index
                // for the changed paths (or fully invalidate on a large /
                // truncated burst) and drop the walk cache so the next graph /
                // backlinks / autocomplete / tree read reflects disk. Do this
                // BEFORE emitting so the refetches the frontend fires in
                // response never read the stale caches.
                crate::vault::update_index_paths(&root, &change.paths);
                crate::vault::invalidate_walk_cache();
                // An automation file changed (Obsidian, the Library editor, an
                // agent, a restore): create, refresh, supersede or retire its
                // permission review now, quietly, so an autosave never
                // notifies. Also before emitting, so the Permission changes
                // panel's refetch sees the result.
                if change
                    .folders
                    .iter()
                    .any(|folder| folder == crate::vault_workflows::WORKFLOWS_DIR)
                {
                    crate::workflow_permissions::sweep(&root, false);
                }
                let _ = app.emit(EVENT, change.clone());
            }
        }
    });
}

/// Coalesce one burst into a [`VaultChange`]. Given the burst's first event,
/// drain follow-on events until the vault goes quiet for `DEBOUNCE` (or the
/// channel disconnects), folding each into the path set. Returns `None` when
/// the burst held only ignored/read-only paths — the caller then waits for the
/// next event. Extracted from the watch thread so the debounce + filter + emit
/// assembly is unit-testable against a synthetic channel (no FS timing).
fn build_burst(
    root: &Path,
    first: notify::Result<Event>,
    rx: &Receiver<notify::Result<Event>>,
) -> Option<VaultChange> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    collect(root, first, &mut paths);
    // Reset the quiet timer on every new event; a Timeout (quiet) or
    // Disconnected (shutdown) both end the burst.
    while let Ok(ev) = rx.recv_timeout(DEBOUNCE) {
        collect(root, ev, &mut paths);
    }
    if paths.is_empty() {
        return None;
    }
    let folders = top_level_folders(&paths);
    let mut paths_vec: Vec<String> = paths.into_iter().collect();
    paths_vec.truncate(MAX_PATHS);
    Some(VaultChange {
        folders,
        paths: paths_vec,
    })
}

/// Fold one filesystem event into the burst's path set, skipping read-only
/// access events and ignored paths.
fn collect(root: &Path, res: notify::Result<Event>, out: &mut BTreeSet<String>) {
    let event = match res {
        Ok(e) => e,
        Err(_) => return,
    };
    // `Access` = a file was read/opened — pure noise for our purposes.
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for p in &event.paths {
        if let Some(rel) = rel_path(root, p) {
            if is_ignored(&rel) {
                continue;
            }
            out.insert(rel);
        }
    }
}

/// Vault-relative, forward-slashed path for an absolute event path, or `None`
/// if it isn't under the vault root (or is the root itself).
fn rel_path(root: &Path, p: &Path) -> Option<String> {
    let stripped = p.strip_prefix(root).ok()?;
    let s = stripped.to_string_lossy().replace('\\', "/");
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// True for tool/editor internals we never want to react to: any path with a
/// dot- or underscore-prefixed component (`.git`, `.obsidian`,
/// `.task-filters.json`, `.notifications.jsonl`, `_calendar`, …) and
/// `~`-suffixed editor backup files.
fn is_ignored(rel: &str) -> bool {
    if rel.is_empty() {
        return true;
    }
    for comp in rel.split('/') {
        if comp.starts_with('.') || comp.starts_with('_') {
            return true;
        }
    }
    // `rsplit` always yields at least the whole string, so this also covers
    // a `~`-suffixed file sitting at the vault root.
    if let Some(name) = rel.rsplit('/').next() {
        if name.ends_with('~') {
            return true;
        }
    }
    false
}

/// Distinct first path components across the burst (the folder a file lives
/// in). Root-level files contribute no folder — they still refresh the tree
/// via the `paths` list.
fn top_level_folders(paths: &BTreeSet<String>) -> Vec<String> {
    let mut set = BTreeSet::new();
    for p in paths {
        if let Some((head, _)) = p.split_once('/') {
            set.insert(head.to_string());
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::mpsc::channel;

    fn modify_event(path: &str) -> notify::Result<Event> {
        Ok(Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        })
    }

    fn access_event(path: &str) -> notify::Result<Event> {
        Ok(Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        })
    }

    #[test]
    fn rel_path_strips_root_and_forward_slashes() {
        let root = PathBuf::from("/vault");
        assert_eq!(
            rel_path(&root, &PathBuf::from("/vault/tasks/foo.md")),
            Some("tasks/foo.md".to_string())
        );
        // The root itself yields nothing.
        assert_eq!(rel_path(&root, &PathBuf::from("/vault")), None);
        // Outside the vault → nothing.
        assert_eq!(rel_path(&root, &PathBuf::from("/etc/passwd")), None);
    }

    #[test]
    fn ignores_tool_and_editor_internals() {
        assert!(is_ignored(".git/HEAD"));
        assert!(is_ignored(".obsidian/workspace.json"));
        assert!(is_ignored(".task-filters.json"));
        assert!(is_ignored(".notifications.jsonl"));
        assert!(is_ignored("_calendar/week.json"));
        assert!(is_ignored("tasks/foo.md~")); // editor backup
        assert!(is_ignored(".DS_Store"));
        // Real user content is not ignored.
        assert!(!is_ignored("tasks/foo.md"));
        assert!(!is_ignored("projects/active/launch.md"));
        assert!(!is_ignored("daily/2026/2026-06-06.md"));
    }

    #[test]
    fn top_level_folders_are_distinct_and_skip_root_files() {
        let mut set = BTreeSet::new();
        set.insert("tasks/a.md".to_string());
        set.insert("tasks/b.md".to_string());
        set.insert("projects/active/x.md".to_string());
        set.insert("README.md".to_string()); // root file — no folder
        assert_eq!(
            top_level_folders(&set),
            vec!["projects".to_string(), "tasks".to_string()]
        );
    }

    #[test]
    fn collect_skips_access_events_and_ignored_paths() {
        let root = PathBuf::from("/vault");
        let mut out = BTreeSet::new();

        // An access (read) event is dropped entirely.
        let access = Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![PathBuf::from("/vault/tasks/foo.md")],
            attrs: Default::default(),
        };
        collect(&root, Ok(access), &mut out);
        assert!(out.is_empty());

        // A modify event keeps the user file but drops the ignored sibling.
        let modify = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![
                PathBuf::from("/vault/tasks/foo.md"),
                PathBuf::from("/vault/.git/index"),
            ],
            attrs: Default::default(),
        };
        collect(&root, Ok(modify), &mut out);
        assert_eq!(out.into_iter().collect::<Vec<_>>(), vec!["tasks/foo.md"]);
    }

    #[test]
    fn build_burst_coalesces_debounces_and_filters() {
        let root = PathBuf::from("/vault");
        let (tx, rx) = channel();
        // Queue the follow-on events of one burst: a second user file, a
        // read-only access (dropped), and a `.git` write (ignored). Dropping
        // the sender makes the drain loop's `recv_timeout` return Disconnected
        // immediately once the queue empties — so the test never waits out the
        // 300ms debounce.
        tx.send(modify_event("/vault/projects/x.md")).unwrap();
        tx.send(access_event("/vault/tasks/a.md")).unwrap();
        tx.send(modify_event("/vault/.git/index")).unwrap();
        drop(tx);

        let first = modify_event("/vault/tasks/a.md");
        let change = build_burst(&root, first, &rx).expect("burst had real changes");
        // Folders + paths are distinct and sorted; the access + ignored events
        // contributed nothing.
        assert_eq!(
            change.folders,
            vec!["projects".to_string(), "tasks".to_string()]
        );
        assert_eq!(
            change.paths,
            vec!["projects/x.md".to_string(), "tasks/a.md".to_string()]
        );
    }

    #[test]
    fn build_burst_returns_none_for_an_all_ignored_burst() {
        let root = PathBuf::from("/vault");
        let (tx, rx) = channel();
        tx.send(modify_event("/vault/.obsidian/workspace.json"))
            .unwrap();
        drop(tx);
        // The first event is itself ignored and nothing else survives → no emit.
        let first = access_event("/vault/tasks/a.md");
        assert!(build_burst(&root, first, &rx).is_none());
    }
}
