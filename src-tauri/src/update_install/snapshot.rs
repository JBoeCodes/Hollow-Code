//! The Sessions snapshot taken before the swap (design section 9): an
//! online, consistent copy of each Sessions database, so going back past a
//! schema migration can restore the conversations from before the update.
//!
//! **Where:** `<state>/snapshots/<install id>-from-<old version>/<name>.db`,
//! that is `~/Library/Application Support/com.hollow.app/installer/
//! snapshots/...`, folders 0700 and files 0600. Not in `/Applications`
//! beside `.previous`: conversations are private and `/Applications` is
//! readable by every account on the Mac (and may not be writable by this
//! one); the app data folder is per user and on the same volume as the live
//! databases. The journal names the folder, so the kept previous app and
//! its data are paired by install id.
//!
//! **How:** `VACUUM INTO` through the project's rusqlite (its `backup`
//! feature is off, and `VACUUM INTO` needs none): one read transaction, so
//! the copy is a consistent snapshot even while a writer is open, and it
//! keeps the header's `user_version`. Each copy is then opened read-only and
//! must pass `PRAGMA quick_check` with the same `user_version` as its source,
//! which the journal records. Any failure stops the install.

use super::journal::{Snapshot, SnapshotFile};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) const DIR: &str = "snapshots";
/// Snapshot folders kept after a healthy install (newest first).
pub(crate) const KEEP: usize = 3;

/// The databases to copy. A file is itself; a folder with a `Sessions`
/// folder means its `*.db`; any other folder means its own `*.db`. A
/// missing default source (a Mac with no chats yet) is an empty list; a
/// missing explicit one is an error.
pub(crate) fn sources(source: &Path, explicit: bool) -> Result<Vec<PathBuf>, String> {
    if !source.exists() {
        return if explicit {
            Err(format!(
                "the snapshot source {} does not exist",
                source.display()
            ))
        } else {
            Ok(Vec::new())
        };
    }
    if source.is_file() {
        return Ok(vec![source.to_path_buf()]);
    }
    let folder = if source.join("Sessions").is_dir() {
        source.join("Sessions")
    } else {
        source.to_path_buf()
    };
    let mut found: Vec<PathBuf> = fs::read_dir(&folder)
        .map_err(|e| format!("could not list {}: {e}", folder.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "db"))
        .collect();
    found.sort();
    Ok(found)
}

/// Copy every source into `folder` and check each copy.
pub(crate) fn take(sources: &[PathBuf], folder: &Path) -> Result<Snapshot, String> {
    super::lock::create_private_dir(folder)
        .map_err(|e| format!("could not create {}: {e}", folder.display()))?;
    let mut files = Vec::new();
    for source in sources {
        let name = source
            .file_name()
            .ok_or_else(|| format!("{} has no file name", source.display()))?;
        let copy = folder.join(name);
        files.push(copy_one(source, &copy)?);
    }
    Ok(Snapshot {
        skipped: false,
        folder: Some(folder.to_path_buf()),
        files,
    })
}

fn copy_one(source: &Path, copy: &Path) -> Result<SnapshotFile, String> {
    let fail = |what: &str, e: rusqlite::Error| format!("{what} {}: {e}", source.display());
    if copy.exists() {
        fs::remove_file(copy).map_err(|e| format!("{}: {e}", copy.display()))?;
    }
    // Read-only: never makes a missing file and can't write to the source.
    // `VACUUM INTO` works on a read-only connection (tested with an open WAL
    // writer and with a closed WAL file that has no `-wal` or `-shm`).
    let conn = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| fail("could not open", e))?;
    conn.busy_timeout(Duration::from_secs(10))
        .map_err(|e| fail("could not set a busy timeout on", e))?;
    let source_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| fail("could not read the version of", e))?;
    conn.execute("VACUUM INTO ?1", [copy.to_string_lossy()])
        .map_err(|e| fail("could not copy", e))?;
    drop(conn);
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(copy, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", copy.display()))?;
    }
    let check = Connection::open_with_flags(copy, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("could not open the copy {}: {e}", copy.display()))?;
    quick_check(&check)
        .map_err(|why| format!("the copy {} fails quick_check: {why}", copy.display()))?;
    let copy_version: i64 = check
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| format!("could not read the copy's version: {e}"))?;
    if copy_version != source_version {
        return Err(format!(
            "the copy {} has user_version {copy_version}, the source {source_version}",
            copy.display()
        ));
    }
    let bytes = fs::metadata(copy).map(|m| m.len()).unwrap_or(0);
    Ok(SnapshotFile {
        source: source.to_path_buf(),
        copy: copy.to_path_buf(),
        user_version: source_version,
        bytes,
    })
}

/// `PRAGMA quick_check` on a read-only connection: `Ok` when it reports
/// nothing but `ok`. SQLite's check of an FTS5 table's inverted index needs
/// to write, so read-only it reports "unable to validate the inverted index
/// for FTS5 table ...: attempt to write a readonly database" (the Sessions
/// database's `chat_search_fts`); that line is not damage and is dropped,
/// as `hollow_databases` does (the FTS shadow tables' b-trees are still
/// checked). Found by row U6: without this, every real snapshot failed.
/// The cost: damage inside `chat_search_fts`'s inverted index is no longer
/// detected here. That is accepted because the index is derived: the
/// store's own check (`session_search.rs`, `verify`) finds such damage at
/// use and rebuilds the index from the chats, which this check covers.
pub(crate) fn quick_check(conn: &Connection) -> Result<(), String> {
    let mut lines: Vec<String> = conn
        .prepare("PRAGMA quick_check")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
        .map_err(|e| format!("could not run it: {e}"))?;
    lines.retain(|l| l != "ok" && !(l.contains("FTS5 table") && l.contains("readonly database")));
    if lines.is_empty() {
        Ok(())
    } else {
        Err(lines.join("; "))
    }
}

/// Keep the newest `keep` snapshot folders and always `current`. Only real
/// folders named `<id>-from-<version>` are touched (never a symlink or a
/// stray folder), newest by modification time (the folder is written once,
/// when its snapshot is taken).
pub(crate) fn prune(root: &Path, keep: usize, current: Option<&Path>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    let mut folders: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains("-from-"))
        .filter_map(|e| {
            let meta = fs::symlink_metadata(e.path()).ok()?;
            (meta.is_dir() && !meta.file_type().is_symlink())
                .then(|| (meta.modified().unwrap_or(std::time::UNIX_EPOCH), e.path()))
        })
        .collect();
    folders.sort();
    folders.reverse();
    for (_, old) in folders.into_iter().skip(keep) {
        if Some(old.as_path()) != current {
            let _ = fs::remove_dir_all(old);
        }
    }
}
