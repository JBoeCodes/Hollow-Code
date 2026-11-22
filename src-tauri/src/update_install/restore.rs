//! `--go-back --restore-snapshot <folder>`: put the Sessions databases back
//! as they were before an update (updates design section 9; row U6), in
//! the same journaled run as the app swap.
//!
//! The folder must be one of this installer's own snapshots
//! (`<state>/snapshots/<id>-from-<version>`), named by a journal; that
//! journal's snapshot record says which live database each copy belongs to.
//! Nothing is read from the folder's names.
//!
//! **Order** (all before anything moves): each copy passes `quick_check`
//! with the `user_version` the journal recorded; it is copied to
//! `<live>.go-back-restore` beside the live database, and that copy is
//! checked again; no other process has the live database open (the `-shm`
//! dead-man byte, `hollow_databases::shm_in_use`); the copy and its folder
//! are fsynced, so a power cut after the renames can't leave a renamed but
//! unwritten database. Then the moves join the go back's plan, before the
//! app swap: the live `<name>.db` (and any `-wal`, `-shm` or `-journal`) is
//! renamed to `<file>.before-go-back`, or every one of them to
//! `<file>.before-go-back-<install id>` when any plain name is taken (one
//! suffix per database, so its files stay a set), and the restore copy is
//! renamed to `<name>.db`. The restored database is checked
//! once more before the swap. Any failure undoes every move: the data is
//! untouched and the apps are not swapped. A crash is recovered from the
//! journal like any other move (`swap::recover`), and the leftover restore
//! copies are removed ([`remove_temps`]); copies no journal plan names (a
//! crash before the plan was saved) are swept by every later run
//! ([`sweep_orphans`], from `run::recover_all`).
//!
//! The moved-aside files are kept, never deleted: they hold the chats from
//! after the update (putting them back is by hand for now).

use super::journal::{Move, Op, Record, Snapshot, SnapshotFile};
use rusqlite::{Connection, OpenFlags};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// The restore copy's suffix, beside the live database.
pub(crate) const TEMP_SUFFIX: &str = ".go-back-restore";
/// The moved-aside live files' suffix.
pub(crate) const ASIDE_SUFFIX: &str = ".before-go-back";
/// The files that go with a database.
const COMPANIONS: [&str; 4] = ["", "-wal", "-shm", "-journal"];

/// What the journal records about a restore.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreRecord {
    /// The snapshot folder.
    pub(crate) folder: PathBuf,
    /// The install whose snapshot it is.
    pub(crate) from_install: String,
    /// The live databases restored.
    pub(crate) databases: Vec<PathBuf>,
    /// Where the live files were moved (`from`, `to`).
    pub(crate) aside: Vec<(PathBuf, PathBuf)>,
}

/// A restore ready to join the plan.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    pub(crate) record: RestoreRecord,
    pub(crate) moves: Vec<Move>,
    /// The checked copies beside the live databases, with the version each
    /// must keep.
    pub(crate) temps: Vec<(PathBuf, i64)>,
}

/// `quick_check` passes (`snapshot::quick_check`) and `user_version` is
/// `expected` (read-only).
pub(crate) fn check(path: &Path, expected: i64) -> Result<(), String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{} can't be opened: {e}", path.display()))?;
    super::snapshot::quick_check(&conn)
        .map_err(|why| format!("{} fails quick_check: {why}", path.display()))?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| format!("{} has no readable version: {e}", path.display()))?;
    if version != expected {
        return Err(format!(
            "{} has user_version {version}, the snapshot recorded {expected}",
            path.display()
        ));
    }
    Ok(())
}

/// The journal whose snapshot is `folder` (which must be under
/// `<state>/snapshots/`), and that snapshot.
pub(crate) fn find(state_dir: &Path, folder: &Path) -> Result<(String, Snapshot), String> {
    let root = state_dir
        .join(super::snapshot::DIR)
        .canonicalize()
        .map_err(|e| format!("there are no snapshots ({e})"))?;
    let folder = folder
        .canonicalize()
        .map_err(|e| format!("the snapshot {} can't be read ({e})", folder.display()))?;
    if folder.parent() != Some(root.as_path()) {
        return Err(format!(
            "{} is not one of the installer's snapshots",
            folder.display()
        ));
    }
    let (journals, _) = super::journal::list(state_dir);
    journals
        .into_iter()
        .rev()
        .find_map(|j| {
            let snap = j.record.snapshot.clone()?;
            let same = snap
                .folder
                .as_ref()
                .and_then(|f| f.canonicalize().ok())
                .is_some_and(|f| f == folder);
            (same && !snap.skipped).then_some((j.record.id, snap))
        })
        .ok_or_else(|| format!("no install journal names the snapshot {}", folder.display()))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// The suffix a live database's files go aside with: `.before-go-back`,
/// or with the install id when any of its files' plain names is taken (an
/// earlier go back's), the same for all of them.
pub(crate) fn aside_suffix(live: &Path, install_id: &str) -> String {
    let taken = COMPANIONS.iter().any(|c| {
        std::fs::symlink_metadata(with_suffix(&with_suffix(live, c), ASIDE_SUFFIX)).is_ok()
    });
    if taken {
        format!("{ASIDE_SUFFIX}-{}", super::journal::safe_id(install_id))
    } else {
        ASIDE_SUFFIX.to_string()
    }
}

/// Remove a restore copy and any `-wal`, `-shm` or `-journal` a check of
/// it left (never live data: the copy is only ever opened read-only).
pub(crate) fn remove_temp_files(temp: &Path) {
    for suffix in COMPANIONS {
        let _ = std::fs::remove_file(with_suffix(temp, suffix));
    }
}

/// `fsync` a file and then its folder (as `journal::write_durably` does),
/// so the file's bytes are on disk before a rename makes it live.
pub(crate) fn sync_file_and_folder(file: &Path) -> Result<(), String> {
    std::fs::File::open(file)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("could not flush {}: {e}", file.display()))?;
    let folder = file.parent().unwrap_or(Path::new("."));
    std::fs::File::open(folder)
        .and_then(|f| f.sync_all())
        .map_err(|e| format!("could not flush {}: {e}", folder.display()))
}

fn ino(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path).ok().map(|m| m.ino())
}

fn prepare_one(file: &SnapshotFile, install_id: &str, out: &mut Prepared) -> Result<(), String> {
    check(&file.copy, file.user_version).map_err(|e| format!("the snapshot is not intact: {e}"))?;
    let live = &file.source;
    if !live.parent().is_some_and(Path::is_dir) {
        return Err(format!("the Sessions folder of {} is gone", live.display()));
    }
    if live.exists() && crate::hollow_databases::shm_in_use(live) {
        return Err(format!(
            "{} is still open in another process (a Hollow helper or a second Hollow); quit it and try again",
            live.display()
        ));
    }
    let temp = with_suffix(live, TEMP_SUFFIX);
    remove_temp_files(&temp);
    std::fs::copy(&file.copy, &temp)
        .map_err(|e| format!("could not copy the snapshot beside {}: {e}", live.display()))?;
    out.temps.push((temp.clone(), file.user_version));
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", temp.display()))?;
    }
    check(&temp, file.user_version).map_err(|e| format!("the restore copy is not intact: {e}"))?;
    // Only the copy itself becomes the live database.
    for suffix in &COMPANIONS[1..] {
        let _ = std::fs::remove_file(with_suffix(&temp, suffix));
    }
    sync_file_and_folder(&temp)?;
    let aside = aside_suffix(live, install_id);
    for suffix in COMPANIONS {
        let from = with_suffix(live, suffix);
        let Some(i) = ino(&from) else { continue };
        let to = with_suffix(&from, &aside);
        out.record.aside.push((from.clone(), to.clone()));
        out.moves.push(Move {
            op: Op::Rename,
            from,
            to,
            ino: i,
        });
    }
    let temp_ino = ino(&temp).ok_or("the restore copy vanished")?;
    out.moves.push(Move {
        op: Op::Rename,
        from: temp,
        to: live.clone(),
        ino: temp_ino,
    });
    out.record.databases.push(live.clone());
    Ok(())
}

/// Check the snapshot and lay the restore copies beside the live databases;
/// returns the moves. Nothing live is touched. On failure the copies made
/// so far are removed.
pub(crate) fn prepare(
    state_dir: &Path,
    folder: &Path,
    install_id: &str,
) -> Result<Prepared, String> {
    let (from_install, snapshot) = find(state_dir, folder)?;
    if snapshot.files.is_empty() {
        return Err("the snapshot holds no databases".into());
    }
    let mut out = Prepared {
        record: RestoreRecord {
            folder: folder.to_path_buf(),
            from_install,
            ..RestoreRecord::default()
        },
        moves: Vec::new(),
        temps: Vec::new(),
    };
    for file in &snapshot.files {
        if let Err(why) = prepare_one(file, install_id, &mut out) {
            discard(&out);
            return Err(why);
        }
    }
    Ok(out)
}

/// The restored databases, checked in place before the swap.
pub(crate) fn verify_restored(prepared: &Prepared) -> Result<(), String> {
    for ((_, version), live) in prepared.temps.iter().zip(&prepared.record.databases) {
        check(live, *version).map_err(|e| format!("the restored database is not intact: {e}"))?;
    }
    Ok(())
}

/// Remove the restore copies still at their temporary names.
pub(crate) fn discard(prepared: &Prepared) {
    for (temp, _) in &prepared.temps {
        remove_temp_files(temp);
    }
}

/// After recovery settled a journal with a restore: remove restore copies
/// its undone moves left at their temporary names.
pub(crate) fn remove_temps(record: &Record) {
    if record.restore.is_none() {
        return;
    }
    for m in record.plan.iter().flatten() {
        if m.from.to_string_lossy().ends_with(TEMP_SUFFIX) {
            if ino(&m.from) == Some(m.ino) {
                let _ = std::fs::remove_file(&m.from);
            }
            for suffix in &COMPANIONS[1..] {
                let _ = std::fs::remove_file(with_suffix(&m.from, suffix));
            }
        }
    }
}

/// Once every journal is settled (`run::recover_all`, under the installer
/// lock): remove restore copies beside any database a journal names (its
/// snapshot's sources, its restore's databases). A copy left at its
/// temporary name is never needed: a settled go back renamed it into place
/// or undid that. Catches a crash between laying the copies and saving the
/// plan, which no plan names.
pub(crate) fn sweep_orphans(journals: &[super::journal::Journal]) {
    let mut lives: Vec<&Path> = Vec::new();
    for r in journals.iter().map(|j| &j.record) {
        if let Some(snap) = &r.snapshot {
            lives.extend(snap.files.iter().map(|f| f.source.as_path()));
        }
        if let Some(restore) = &r.restore {
            lives.extend(restore.databases.iter().map(PathBuf::as_path));
        }
    }
    lives.sort();
    lives.dedup();
    for live in lives {
        let temp = with_suffix(live, TEMP_SUFFIX);
        if COMPANIONS
            .iter()
            .any(|c| std::fs::symlink_metadata(with_suffix(&temp, c)).is_ok())
        {
            remove_temp_files(&temp);
        }
    }
}
