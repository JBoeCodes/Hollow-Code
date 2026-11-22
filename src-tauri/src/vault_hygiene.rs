//! Startup removal of state files Hollow itself used to keep in the vault
//! and no longer reads. Each entry is an exact root-level name that only
//! Hollow ever wrote: one-time migration markers whose migration was removed,
//! and derived caches of retired features. User notes are never on this
//! list, and a symlink or folder at one of these names is left alone.
//!
//! Retired 2026-09-26 (see docs/BUILD_LOG.md):
//! - `.automation-seeded`: the routine seed marker (the seed had no caller).
//! - `.workflow-schedule-tz-fixed`: the one-time UTC → local `next_run_at`
//!   repair marker (the repair shipped in June and was removed).
//! - `.heartbeat-review-migration-v1.json`: the one-time import of the
//!   JSONL heartbeat queues into the review queue (removed).
//! - `.hollow-change-index.json`, `.hollow-change-journal.jsonl`: the
//!   "Since last visit" change feed, whose card was never mounted.
//! - `.notion-content-index.json`: the retired Notion content import.
//!
//! Moved out of the vault the same day, by their own modules on first use:
//! `.hollow-directories.json` (`directory_connections::manifest_path`) and
//! `_calendar/week.json` (`calendar::cache_path`).

use std::path::Path;

pub const RETIRED_STATE_FILES: &[&str] = &[
    ".automation-seeded",
    ".workflow-schedule-tz-fixed",
    ".heartbeat-review-migration-v1.json",
    ".hollow-change-index.json",
    ".hollow-change-journal.jsonl",
    ".notion-content-index.json",
];

/// Remove every retired state file present at the vault root. Returns the
/// names removed. Never fails: an unremovable file is logged and kept.
pub fn retire_legacy_state(vault: &Path) -> Vec<&'static str> {
    let mut removed = Vec::new();
    for name in RETIRED_STATE_FILES {
        let path = vault.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => match std::fs::remove_file(&path) {
                Ok(()) => removed.push(*name),
                Err(error) => eprintln!("[vault-hygiene] could not remove {name}: {error}"),
            },
            _ => {}
        }
    }
    if !removed.is_empty() {
        eprintln!(
            "[vault-hygiene] removed retired state: {}",
            removed.join(", ")
        );
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn removes_only_retired_regular_files() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        for name in RETIRED_STATE_FILES {
            fs::write(v.join(name), "x").unwrap();
        }
        fs::write(v.join(".review-queue.json"), "{}").unwrap();
        fs::write(v.join("note.md"), "keep").unwrap();
        let removed = retire_legacy_state(v);
        assert_eq!(removed.len(), RETIRED_STATE_FILES.len());
        for name in RETIRED_STATE_FILES {
            assert!(!v.join(name).exists(), "{name} removed");
        }
        assert!(v.join(".review-queue.json").exists());
        assert!(v.join("note.md").exists());
        assert!(retire_legacy_state(v).is_empty(), "idempotent");
    }

    #[cfg(unix)]
    #[test]
    fn leaves_symlinks_and_folders_alone() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let target = v.join("target.txt");
        fs::write(&target, "outside").unwrap();
        std::os::unix::fs::symlink(&target, v.join(".automation-seeded")).unwrap();
        fs::create_dir(v.join(".hollow-change-index.json")).unwrap();
        assert!(retire_legacy_state(v).is_empty());
        assert!(fs::symlink_metadata(v.join(".automation-seeded")).is_ok());
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside");
        assert!(v.join(".hollow-change-index.json").is_dir());
    }
}
