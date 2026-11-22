//! One read-only folder walker (H5a), shared by the session import and the
//! context scanners that read instruction and memory files (CX19).
//!
//! **Rules.** The root is a parameter: nothing here knows `~/.claude` or
//! `~/.codex` (the command layer resolves the default folders, so a test
//! hands in a temp dir and never reaches `$HOME`). A root that is itself a
//! symbolic link is refused, and no link below it is ever followed: an entry
//! whose own type is a link is counted in [`Walk::symlinks`] and skipped,
//! file or folder. Every entry is a path under the root (`read_dir` of a real
//! folder of the root), so a walk never leaves it. Nothing is opened for
//! writing; files are not even read here, only listed with their size and
//! modification time.
//!
//! **Budget.** [`Budget`] caps the entries visited, the files kept, their
//! total bytes, the depth and the wall time. The first cap reached stops the
//! walk and is named in [`Walk::stopped`], so a consumer can say "stopped
//! after 5,000 files" instead of importing part of a folder silently.
//!
//! **Typed entries.** Each kept file comes back as an [`Entry`] with its
//! path relative to the root split into components and a coarse [`Kind`]
//! from its extension. The consumer's filter decides what it keeps:
//! `session_import` keeps transcript files (`.jsonl` at the layouts its
//! readers know); CX19 adds its own filter for `CLAUDE.md`, `AGENTS.md` and
//! memory files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// A coarse file type from the extension, for a consumer's filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `.jsonl`: one JSON value per line (agent transcripts).
    JsonLines,
    /// `.json`.
    Json,
    /// `.md`, `.markdown`.
    Markdown,
    /// Anything else.
    Other,
}

impl Kind {
    fn of(path: &Path) -> Kind {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("jsonl") => Kind::JsonLines,
            Some("json") => Kind::Json,
            Some("md") | Some("markdown") => Kind::Markdown,
            _ => Kind::Other,
        }
    }
}

/// One regular file under the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// The file's path (the root joined with [`Entry::rel`]).
    pub path: PathBuf,
    /// Its path below the root, one component per folder.
    pub rel: Vec<String>,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Entry {
    /// The file name (the last component).
    pub fn name(&self) -> &str {
        self.rel.last().map(String::as_str).unwrap_or("")
    }

    /// The relative path with `/` between components.
    pub fn rel_path(&self) -> String {
        self.rel.join("/")
    }
}

/// Caps on one walk. [`Budget::default`] suits a person's agent folders.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    /// Folders and files looked at, kept or not.
    pub max_visited: usize,
    /// Files the filter kept.
    pub max_files: usize,
    /// The kept files' total size.
    pub max_bytes: u64,
    /// Folder levels below the root.
    pub max_depth: usize,
    pub max_time: Duration,
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_visited: 400_000,
            max_files: 20_000,
            max_bytes: 64 * 1024 * 1024 * 1024,
            max_depth: 8,
            max_time: Duration::from_secs(120),
        }
    }
}

/// Which cap stopped a walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Visited,
    Files,
    Bytes,
    Time,
    Cancelled,
}

impl Stop {
    /// A sentence for the person.
    pub fn sentence(self, budget: &Budget) -> String {
        match self {
            Stop::Visited => format!(
                "Stopped after looking at {} files and folders.",
                budget.max_visited
            ),
            Stop::Files => format!("Stopped after {} files.", budget.max_files),
            Stop::Bytes => format!(
                "Stopped after {} MB of files.",
                budget.max_bytes / (1024 * 1024)
            ),
            Stop::Time => format!(
                "Stopped after {} seconds.",
                budget.max_time.as_secs().max(1)
            ),
            Stop::Cancelled => "Cancelled.".to_string(),
        }
    }
}

/// What one walk found.
#[derive(Debug, Clone, Default)]
pub(crate) struct Walk {
    /// Kept files, in path order within each folder.
    pub entries: Vec<Entry>,
    /// Entries looked at.
    pub visited: usize,
    /// Links skipped (never followed).
    pub symlinks: usize,
    /// Folders or files that could not be listed or stat'ed.
    pub unreadable: usize,
    /// Folders not entered because they were deeper than the budget allows.
    pub too_deep: usize,
    /// The cap that stopped the walk early; `None` when it finished.
    pub stopped: Option<Stop>,
}

/// Walk `root` read-only under `budget`, keeping each regular file `keep`
/// accepts. `cancel` is checked between entries. Refuses a root that is
/// missing, not a folder, or a symbolic link.
pub(crate) fn walk(
    root: &Path,
    budget: &Budget,
    cancel: &AtomicBool,
    keep: impl Fn(&Entry) -> bool,
) -> Result<Walk, String> {
    walk_pruned(root, budget, cancel, keep, |_| false)
}

/// [`walk`] that does not enter a folder `skip_dir` names (by its own name),
/// for a consumer that reads project trees (CX19 skips `node_modules`).
pub(crate) fn walk_pruned(
    root: &Path,
    budget: &Budget,
    cancel: &AtomicBool,
    keep: impl Fn(&Entry) -> bool,
    skip_dir: impl Fn(&str) -> bool,
) -> Result<Walk, String> {
    let meta = std::fs::symlink_metadata(root)
        .map_err(|_| format!("There is no folder at {}.", root.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "{} is a link to another folder; Hollow reads only the folder itself.",
            root.display()
        ));
    }
    if !meta.is_dir() {
        return Err(format!("{} is not a folder.", root.display()));
    }
    let started = Instant::now();
    let mut out = Walk::default();
    let mut kept_bytes: u64 = 0;
    // Depth-first with an explicit stack: (folder, its components below root).
    let mut stack: Vec<(PathBuf, Vec<String>)> = vec![(root.to_path_buf(), Vec::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            out.unreadable += 1;
            continue;
        };
        let mut children: Vec<std::fs::DirEntry> = Vec::new();
        for child in read {
            match child {
                Ok(child) => children.push(child),
                Err(_) => out.unreadable += 1,
            }
        }
        children.sort_by_key(|c| c.file_name());
        let mut subdirs: Vec<(PathBuf, Vec<String>)> = Vec::new();
        for child in children {
            if cancel.load(Ordering::Relaxed) {
                out.stopped = Some(Stop::Cancelled);
                return Ok(out);
            }
            if started.elapsed() >= budget.max_time {
                out.stopped = Some(Stop::Time);
                return Ok(out);
            }
            if out.visited >= budget.max_visited {
                out.stopped = Some(Stop::Visited);
                return Ok(out);
            }
            out.visited += 1;
            // `DirEntry::file_type` is the entry's own type: a link reads as
            // a link, never as what it points at.
            let Ok(file_type) = child.file_type() else {
                out.unreadable += 1;
                continue;
            };
            if file_type.is_symlink() {
                out.symlinks += 1;
                continue;
            }
            let Some(name) = child.file_name().to_str().map(str::to_string) else {
                out.unreadable += 1;
                continue;
            };
            let mut child_rel = rel.clone();
            child_rel.push(name);
            let path = child.path();
            if file_type.is_dir() {
                if child_rel.last().is_some_and(|n| skip_dir(n)) {
                    continue;
                }
                if child_rel.len() > budget.max_depth {
                    out.too_deep += 1;
                } else {
                    subdirs.push((path, child_rel));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(meta) = child.metadata() else {
                out.unreadable += 1;
                continue;
            };
            let entry = Entry {
                kind: Kind::of(&path),
                path,
                rel: child_rel,
                size: meta.len(),
                modified: meta.modified().ok(),
            };
            if !keep(&entry) {
                continue;
            }
            if out.entries.len() >= budget.max_files {
                out.stopped = Some(Stop::Files);
                return Ok(out);
            }
            if kept_bytes.saturating_add(entry.size) > budget.max_bytes {
                out.stopped = Some(Stop::Bytes);
                return Ok(out);
            }
            kept_bytes += entry.size;
            out.entries.push(entry);
        }
        // Pushed in reverse so folders are entered in name order.
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(out)
}
