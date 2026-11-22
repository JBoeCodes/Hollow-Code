//! Hollow's databases (repo features R7b): the SQLite files Hollow keeps in
//! its app support folder, for Settings, Advanced, Hollow's databases and the
//! `system.hollow_databases` action, and the other vaults' rebuildable
//! indexes that Settings, Advanced, Disk space, Remove old indexes… (or
//! `system.remove_stale_indexes`, behind a card) removes.
//!
//! Four kinds: `Sessions/*.db` (conversations: primary data, never offered
//! for removal), `SearchIndex/*.db` and `EmbedIndex/*.db` (rebuildable from
//! the vault) and `Companion/applied.db`. Each per-vault file is named
//! `<vault folder>-<16 hex>.db` (`session_store::vault_file_stem`; the two
//! index modules' `db_path_for` build the same stem, and a test here keeps
//! them equal), so the stem says whether it belongs to this vault.
//!
//! A fifth kind (repo features R14b): `SearchIndex/<vault stem>.projects-<16
//! hex>.db`, a vault's content index of a Projects folder
//! (`project_text_index`). Whose it is comes from the vault stem before
//! `.projects-` (a vault stem never holds a `.`), so this vault's is never
//! offered, and another vault's unused one is offered like its search index.
//!
//! Everything here reads without blocking the app. Measuring opens no
//! connection to a file with no `-wal` (it reads `user_version` from the
//! 100-byte header, so it creates no `-wal`/`-shm` beside a closed WAL-mode
//! file and takes no lock); a file with a `-wal` gets a short-lived
//! `SQLITE_OPEN_READ_ONLY` connection with a 500 ms busy timeout. Check
//! integrity (`check`) opens every file read-only and runs `quick_check` and
//! one `count(*)` per table (never row contents), interrupted once
//! [`CHECK_BUDGET`] has passed. SQLite's FTS5 inverted-index check needs to
//! write, so it can't run read-only; the answer then says `ok (FTS index not
//! validated read-only)` (the FTS shadow tables' b-trees are still checked).
//! A WAL reader never blocks the app's writers. An index's TRUNCATE
//! checkpoint (`compact_wal`, after the launch sweep) waits for it up to
//! that connection's 2 s busy timeout and then gives up for that launch;
//! `journal_size_limit` trims the WAL at a later checkpoint.
//!
//! An index is offered for removal only when nothing uses it: another
//! vault's, unchanged for an hour, and with no SQLite connection in another
//! process (a second Hollow with that vault open). The last is read from the
//! `-shm`'s dead-man-switch byte, on which every WAL connection holds a
//! shared lock ([`shm_in_use`]); an idle connection may write nothing for
//! hours, and a TRUNCATE checkpoint leaves its `-wal` empty. A removal
//! probes each file again just before renaming it, and puts a database back
//! with a link that never replaces one made at its path meanwhile.
//!
//! Every function takes the support folder explicitly, so tests never touch
//! the real one; [`support_root`] gives the app's.

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// An index of another vault is offered for removal only when neither its
/// `.db` nor a non-empty `-wal` changed for this long: a branch build that
/// shares the support folder and has that vault open writes to one of them.
pub const STALE_AFTER: Duration = Duration::from_secs(60 * 60);
/// How long a measure's read-only connection waits on a lock.
const BUSY: Duration = Duration::from_millis(500);
/// The whole Check integrity run stops (each query interrupted) after this.
pub const CHECK_BUDGET: Duration = Duration::from_secs(30);
/// A passing check whose FTS5 inverted-index step couldn't run read-only.
pub const FTS_UNCHECKED: &str = "ok (FTS index not validated read-only)";

/// The app's support folder (`instance::support_dir`); a per-process temp
/// folder under test, so the tier sweep's card never reads the real one.
#[cfg(not(test))]
pub fn support_root() -> Option<PathBuf> {
    crate::instance::support_dir()
}

#[cfg(test)]
pub fn support_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir("hollow-test-databases-"))
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum DbKind {
    Sessions,
    SearchIndex,
    EmbedIndex,
    Companion,
    /// A vault's content index of a Projects folder (`project_text_index`,
    /// repo features R14b): `SearchIndex/<vault stem>.projects-<16 hex>.db`.
    /// Whose it is comes from the vault stem before `.projects-`; another
    /// vault's is rebuildable and offered like its search index.
    ProjectIndex,
}

impl DbKind {
    const ALL: [DbKind; 5] = [
        DbKind::Sessions,
        DbKind::SearchIndex,
        DbKind::EmbedIndex,
        DbKind::Companion,
        DbKind::ProjectIndex,
    ];

    fn folder(self) -> &'static str {
        match self {
            DbKind::Sessions => "Sessions",
            DbKind::SearchIndex | DbKind::ProjectIndex => "SearchIndex",
            DbKind::EmbedIndex => "EmbedIndex",
            DbKind::Companion => "Companion",
        }
    }

    /// Rebuildable from the vault (or its Projects folder), so another
    /// vault's copy may go.
    fn rebuildable(self) -> bool {
        matches!(
            self,
            DbKind::SearchIndex | DbKind::EmbedIndex | DbKind::ProjectIndex
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Owner {
    ThisVault,
    OtherVault,
    /// Not per vault (`Companion/applied.db`).
    App,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Table {
    pub name: String,
    pub rows: u64,
}

/// One database file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DbFile {
    pub kind: DbKind,
    pub stem: String,
    pub path: String,
    pub owner: Owner,
    /// The owning vault's folder name (from the stem), for another vault's.
    pub owner_name: Option<String>,
    pub bytes: u64,
    /// The `-wal` file's length; 0 when there is none.
    pub wal_bytes: u64,
    /// The `.db`'s modification time (RFC 3339).
    pub modified: Option<String>,
    /// `PRAGMA user_version`; null when it couldn't be read (`error` says why).
    pub schema_version: Option<i64>,
    /// Check integrity only: `ok`, the first problems, or why it didn't finish.
    pub quick_check: Option<String>,
    /// Check integrity only: each ordinary table's row count (no FTS shadow
    /// or virtual tables, never row contents).
    pub tables: Option<Vec<Table>>,
    pub error: Option<String>,
}

/// What `hollow_databases` answers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The support folder the files are in.
    pub folder: String,
    pub files: Vec<DbFile>,
    /// What Remove old indexes… would remove now (see [`stale_indexes`]).
    pub stale_count: usize,
    pub stale_bytes: u64,
}

/// Another vault's index that [`remove_stale_indexes`] would remove.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StaleFile {
    /// The `.db`; its `-wal` and `-shm` go with it.
    pub path: String,
    pub kind: DbKind,
    pub owner_name: String,
    /// `.db` + `-wal` + `-shm`.
    pub bytes: u64,
    pub modified: Option<String>,
    /// Set when it isn't an ordinary old index: `left by an interrupted
    /// removal` for a `<db>.removing` tombstone (with any `-wal`/`-shm` its
    /// database left).
    pub why: Option<String>,
}

/// Another vault's file that stays, and why.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KeptFile {
    pub path: String,
    pub kind: DbKind,
    pub owner_name: String,
    pub why: String,
}

/// What [`remove_stale_indexes`] would remove now.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub folder: String,
    pub files: Vec<StaleFile>,
    pub bytes: u64,
    /// Binds a confirm to exactly these files, their lengths and times.
    pub hash: String,
    /// Other vaults' conversations, and indexes used in the last hour.
    pub kept: Vec<KeptFile>,
}

/// What [`remove_stale_indexes`] did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    /// Each `.db` removed with its `-wal` and `-shm`.
    pub removed: Vec<String>,
    /// [path, error] for each file that couldn't be removed.
    pub failed: Vec<(String, String)>,
    pub bytes: u64,
}

/// `<path>-wal` / `<path>-shm`.
fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn rfc3339(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Local>::from(t).to_rfc3339()
}

/// `Second-brain` from `Second-brain-0123456789abcdef`: the vault folder name
/// of a `<name>-<16 hex>` stem, the only shape Hollow names a per-vault
/// file; `None` for any other name.
fn vault_stem_name(stem: &str) -> Option<&str> {
    match stem.rsplit_once('-') {
        Some((name, hex))
            if !name.is_empty()
                && hex.len() == 16
                && hex.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Some(name)
        }
        _ => None,
    }
}

/// [`vault_stem_name`], else the stem itself.
fn vault_name(stem: &str) -> String {
    vault_stem_name(stem).unwrap_or(stem).to_string()
}

/// Every `.db` of each kind, sorted by name. Regular files only: a symlink
/// is never listed, so nothing outside the folder can be offered.
fn db_files(support: &Path) -> Vec<(DbKind, PathBuf)> {
    let mut out = Vec::new();
    for kind in DbKind::ALL {
        let dir = support.join(kind.folder());
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().is_some_and(|x| x == "db")
                    && std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file())
            })
            .filter(|p| {
                kind != DbKind::Companion || p.file_name().is_some_and(|n| n == "applied.db")
            })
            // SearchIndex holds both vault indexes and project content
            // indexes; each file is one kind.
            .filter(|p| {
                let project = p.file_name().is_some_and(|n| {
                    crate::project_text_index::is_db_file_name(&n.to_string_lossy())
                });
                match kind {
                    DbKind::SearchIndex => !project,
                    DbKind::ProjectIndex => project,
                    _ => true,
                }
            })
            .collect();
        found.sort();
        out.extend(found.into_iter().map(|p| (kind, p)));
    }
    out
}

/// SQLite's dead-man-switch byte in a `-shm` (`UNIX_SHM_DMS`: 120 bytes of
/// lock slots, then 8 more): every connection to a WAL database holds a
/// shared lock on it for as long as it is open.
const SHM_DMS: i64 = 128;

/// Whether another process has a connection open on `db`: a lock held on
/// its `-shm`'s [`SHM_DMS`] byte. POSIX locks never conflict with this
/// process's own, so its own connections don't count (it only opens this
/// vault's indexes, which are never offered; a Check integrity running at
/// the same moment on that file is the one overlap, and only reads). No
/// `-shm`: nothing has it open. Anything unreadable counts as in use.
#[cfg(unix)]
pub(crate) fn shm_in_use(db: &Path) -> bool {
    use std::os::unix::io::AsRawFd;
    let file = match std::fs::File::open(sidecar(db, "-shm")) {
        Ok(f) => f,
        Err(e) => return e.kind() != std::io::ErrorKind::NotFound,
    };
    // SAFETY: a zeroed `flock` is a valid value, and F_GETLK only writes
    // into the struct it is given.
    let mut lock: libc::flock = unsafe { std::mem::zeroed() };
    lock.l_type = libc::F_WRLCK as _;
    lock.l_whence = libc::SEEK_SET as _;
    lock.l_start = SHM_DMS as _;
    lock.l_len = 1;
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) };
    // `l_type` and `F_UNLCK` differ in width between platforms.
    #[allow(clippy::unnecessary_cast)]
    let unlocked = lock.l_type as i32 == libc::F_UNLCK as i32;
    rc != 0 || !unlocked
}

#[cfg(not(unix))]
pub(crate) fn shm_in_use(db: &Path) -> bool {
    sidecar(db, "-shm").exists()
}

/// Whose the per-vault file at `path` is, by its name alone; `None` for a
/// name Hollow never gives one (so it is never offered). A project index's
/// owner is the vault stem before `.projects-` (R14b).
fn named_owner(kind: DbKind, path: &Path, this: &str) -> Option<(Owner, Option<String>)> {
    if kind == DbKind::ProjectIndex {
        let name = path.file_name()?.to_str()?;
        let (stem, _) = crate::project_text_index::parse_db_file_name(name)?;
        return Some(owner_of(kind, stem, this));
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    vault_stem_name(&stem)?;
    Some(owner_of(kind, &stem, this))
}

/// The kind of a `.db` in `folder_kind`'s folder: SearchIndex holds vault
/// search indexes and project content indexes.
fn kind_in(folder_kind: DbKind, db: &Path) -> DbKind {
    let project = db
        .file_name()
        .is_some_and(|n| crate::project_text_index::is_db_file_name(&n.to_string_lossy()));
    if folder_kind == DbKind::SearchIndex && project {
        DbKind::ProjectIndex
    } else {
        folder_kind
    }
}

fn owner_of(kind: DbKind, stem: &str, this: &str) -> (Owner, Option<String>) {
    if kind == DbKind::Companion {
        (Owner::App, None)
    } else if stem == this {
        (Owner::ThisVault, None)
    } else {
        (Owner::OtherVault, Some(vault_name(stem)))
    }
}

/// `user_version` from the database header (offset 60, big-endian): no
/// connection, no lock, no file created.
fn header_version(path: &Path) -> Result<i64, String> {
    use std::io::Read;
    let mut head = [0u8; 100];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .map_err(|e| format!("couldn't read the header: {e}"))?;
    if &head[..16] != b"SQLite format 3\0" {
        return Err("not an SQLite database".into());
    }
    Ok(i64::from(i32::from_be_bytes([
        head[60], head[61], head[62], head[63],
    ])))
}

fn read_only(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("couldn't open it read-only: {e}"))?;
    conn.busy_timeout(BUSY).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn user_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| format!("couldn't read its version: {e}"))
}

/// Raise `user_version` to `version` when it is lower; never lower it (a
/// newer build's stamp stays for Go back to see). Sessions and the
/// companion ledger stamp their `SCHEMA_VERSION` this way (row U6).
pub(crate) fn stamp_schema_version(conn: &Connection, version: i64) -> Result<(), String> {
    if user_version(conn)? < version {
        conn.pragma_update(None, "user_version", version)
            .map_err(|e| format!("couldn't stamp its version: {e}"))?;
    }
    Ok(())
}

/// Interrupts `conn`'s running query at `deadline`, until dropped.
struct Deadline(Option<std::sync::mpsc::Sender<()>>);

impl Deadline {
    fn arm(conn: &Connection, deadline: Instant) -> Deadline {
        let handle = conn.get_interrupt_handle();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let wait = deadline.saturating_duration_since(Instant::now());
        std::thread::spawn(move || {
            if rx.recv_timeout(wait) == Err(std::sync::mpsc::RecvTimeoutError::Timeout) {
                handle.interrupt();
            }
        });
        Deadline(Some(tx))
    }
}

impl Drop for Deadline {
    fn drop(&mut self) {
        // Dropping the sender wakes the watcher, which then exits.
        self.0.take();
    }
}

/// `quick_check` and each ordinary table's row count, read-only, stopped at
/// `deadline`.
fn check_file(path: &Path, deadline: Instant, file: &mut DbFile) {
    let out_of_time = format!("not finished within {} s", CHECK_BUDGET.as_secs());
    if Instant::now() >= deadline {
        file.quick_check = Some(out_of_time);
        return;
    }
    let conn = match read_only(path) {
        Ok(c) => c,
        Err(e) => {
            file.error = Some(e);
            return;
        }
    };
    let _deadline = Deadline::arm(&conn, deadline);
    let timed_out = |e: &rusqlite::Error| matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == rusqlite::ErrorCode::OperationInterrupted);
    match user_version(&conn) {
        Ok(v) => file.schema_version = Some(v),
        Err(e) => file.error = Some(e),
    }
    let check: Result<Vec<String>, rusqlite::Error> = conn
        .prepare("PRAGMA quick_check(5)")
        .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect());
    file.quick_check = Some(match check {
        // SQLite's check of an FTS5 table's inverted index writes, so a
        // read-only connection gets "attempt to write a readonly database"
        // for it; its b-trees (the shadow tables) were checked like any
        // other. Said, rather than reported as damage.
        Ok(mut lines) => {
            let before = lines.len();
            lines.retain(|l| !(l.contains("FTS5 table") && l.contains("readonly database")));
            match (lines.is_empty(), lines.len() < before) {
                (true, true) => FTS_UNCHECKED.to_string(),
                (true, false) => "ok".to_string(),
                _ => lines.join("; "),
            }
        }
        Err(e) if timed_out(&e) => out_of_time.clone(),
        Err(e) => format!("couldn't run: {e}"),
    });
    let listed: Result<Vec<(String, String)>, rusqlite::Error> = conn
        .prepare("SELECT name, type FROM pragma_table_list WHERE schema = 'main'")
        .and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect());
    let Ok(listed) = listed else {
        return;
    };
    // FTS keeps its data in `<virtual table>_*` shadow tables; `table_list`
    // marks them `shadow` only when the module is loaded, so skip by name too.
    let virtuals: Vec<String> = listed
        .iter()
        .filter(|(_, t)| t == "virtual")
        .map(|(n, _)| format!("{n}_"))
        .collect();
    let mut tables = Vec::new();
    for (name, kind) in &listed {
        if kind != "table"
            || name.starts_with("sqlite_")
            || virtuals.iter().any(|v| name.starts_with(v.as_str()))
        {
            continue;
        }
        let sql = format!(
            "SELECT count(*) FROM main.\"{}\"",
            name.replace('"', "\"\"")
        );
        match conn.query_row(&sql, [], |r| r.get::<_, i64>(0)) {
            Ok(n) => tables.push(Table {
                name: name.clone(),
                rows: n.max(0) as u64,
            }),
            Err(e) if timed_out(&e) => {
                file.error
                    .get_or_insert(format!("row counts {out_of_time}"));
                break;
            }
            Err(e) => {
                file.error
                    .get_or_insert(format!("couldn't count {name}: {e}"));
            }
        }
    }
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    file.tables = Some(tables);
}

/// Every database file under `support`, kind by kind, this vault's first.
/// With `check`, also each one's `quick_check` and table row counts.
pub fn list(support: &Path, vault: &Path, check: bool) -> Vec<DbFile> {
    let this = crate::session_store::vault_file_stem(vault);
    let deadline = Instant::now() + CHECK_BUDGET;
    let mut files: Vec<DbFile> = db_files(support)
        .into_iter()
        .map(|(kind, path)| {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            // A project index's owner is the vault stem in its name (R14b).
            let (owner, owner_name) = if kind == DbKind::ProjectIndex {
                named_owner(kind, &path, &this).unwrap_or_else(|| owner_of(kind, &stem, &this))
            } else {
                owner_of(kind, &stem, &this)
            };
            let meta = std::fs::metadata(&path).ok();
            let wal = std::fs::metadata(sidecar(&path, "-wal")).ok();
            let mut file = DbFile {
                kind,
                stem,
                path: path.to_string_lossy().into_owned(),
                owner,
                owner_name,
                bytes: meta.as_ref().map_or(0, |m| m.len()),
                wal_bytes: wal.as_ref().map_or(0, |m| m.len()),
                modified: meta.and_then(|m| m.modified().ok()).map(rfc3339),
                schema_version: None,
                quick_check: None,
                tables: None,
                error: None,
            };
            if check {
                check_file(&path, deadline, &mut file);
            } else {
                // A `-wal` means a connection may hold changes the header
                // doesn't show yet: ask SQLite. Without one, the header is
                // the whole truth, and a connection would create a `-wal`
                // and `-shm` beside a closed WAL-mode file.
                let version = if wal.is_some() {
                    read_only(&path).and_then(|c| user_version(&c))
                } else {
                    header_version(&path)
                };
                match version {
                    Ok(v) => file.schema_version = Some(v),
                    Err(e) => file.error = Some(e),
                }
            }
            file
        })
        .collect();
    files.sort_by(|a, b| (a.kind, a.owner, &a.stem).cmp(&(b.kind, b.owner, &b.stem)));
    files
}

/// [`list`] with the folder and what Remove old indexes… would free.
pub fn report(support: &Path, vault: &Path, check: bool) -> Report {
    let files = list(support, vault, check);
    // A vault folder that can't be resolved offers nothing (see `this_stem`).
    let stale = stale_indexes(support, vault).ok();
    Report {
        folder: support.to_string_lossy().into_owned(),
        files,
        stale_count: stale.as_ref().map_or(0, |s| s.files.len()),
        stale_bytes: stale.as_ref().map_or(0, |s| s.bytes),
    }
}

/// This vault's stem, for telling its files from others' before anything is
/// offered for removal. `vault_file_stem` falls back to the raw path when the
/// folder can't be resolved, which would name a different stem and make this
/// vault's own indexes look like another's: refused instead.
fn this_stem(vault: &Path) -> Result<String, String> {
    std::fs::canonicalize(vault).map_err(|e| {
        format!("Hollow can't resolve this vault's folder ({e}), so it can't tell its indexes from other vaults'. Nothing was removed.")
    })?;
    Ok(crate::session_store::vault_file_stem(vault))
}

/// Changed less than [`STALE_AFTER`] before `now`: the `.db`, or a `-wal`
/// that holds anything (an empty one is what a TRUNCATE checkpoint or a
/// read-only check leaves). Unknown times count as recent.
fn recently_used(db: &Path, now: SystemTime) -> bool {
    let recent = |t: std::io::Result<SystemTime>| {
        t.map_or(true, |t| {
            now.duration_since(t).map_or(true, |age| age < STALE_AFTER)
        })
    };
    let db_recent = std::fs::metadata(db).map_or(true, |m| recent(m.modified()));
    let wal_recent = match std::fs::metadata(sidecar(db, "-wal")) {
        Ok(m) => m.len() > 0 && recent(m.modified()),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    };
    db_recent || wal_recent
}

/// The files that go with one `.db`, those that exist: (path, len, mtime).
fn file_set(db: &Path) -> Vec<(PathBuf, u64, u128)> {
    [sidecar(db, ""), sidecar(db, "-wal"), sidecar(db, "-shm")]
        .into_iter()
        .filter_map(|p| {
            let m = std::fs::metadata(&p).ok()?;
            let nanos = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            Some((p, m.len(), nanos))
        })
        .collect()
}

fn hash_of(mut set: Vec<(PathBuf, u64, u128)>) -> String {
    set.sort();
    let mut hash = Sha256::new();
    for (path, len, mtime) in &set {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update(format!("\t{len}\t{mtime}\n").as_bytes());
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What a removal renames a `.db` to before removing its `-wal`/`-shm`.
const TOMB: &str = ".removing";
/// Why a tombstone is offered.
const INTERRUPTED: &str = "left by an interrupted removal";

/// The index files this process's `engage` opened (search and meaning
/// search), resolved: never offered, whatever their stem says.
fn engaged_db_paths() -> Vec<PathBuf> {
    [
        crate::vault_search_index::engaged_db_path(),
        crate::vault_embed_index::engaged_db_path(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// `path` is one of `engaged` (compared resolved, so a symlinked folder on
/// either side still matches).
fn is_engaged(path: &Path, engaged: &[PathBuf]) -> bool {
    let resolve = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let path = resolve(path);
    engaged.iter().any(|e| resolve(e) == path)
}

/// `<db>.removing` files in SearchIndex and EmbedIndex: what a removal
/// interrupted between its rename and its last delete left. Regular files
/// only.
fn tombstones(support: &Path) -> Vec<(DbKind, PathBuf)> {
    let mut out = Vec::new();
    for kind in [DbKind::SearchIndex, DbKind::EmbedIndex] {
        let mut found: Vec<PathBuf> = std::fs::read_dir(support.join(kind.folder()))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().ends_with(&format!(".db{TOMB}")))
                    && std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file())
            })
            .collect();
        found.sort();
        out.extend(found.into_iter().map(|p| (kind, p)));
    }
    out
}

/// The `.db` a tombstone was.
fn untombed(tomb: &Path) -> PathBuf {
    let s = tomb.to_string_lossy();
    PathBuf::from(s.strip_suffix(TOMB).unwrap_or(&s).to_string())
}

/// A tombstone and, when its database wasn't made again since, the `-wal`
/// and `-shm` it left: (path, len, mtime) of those that exist.
fn tomb_set(tomb: &Path) -> Vec<(PathBuf, u64, u128)> {
    let db = untombed(tomb);
    let mut set = file_set(tomb)
        .into_iter()
        .filter(|(p, _, _)| p == tomb)
        .collect::<Vec<_>>();
    if !db.exists() {
        set.extend(file_set(&db));
    }
    set
}

/// Renamed more than [`STALE_AFTER`] before `now`. A rename keeps the
/// mtime, so this reads the status-change time, which it sets.
#[cfg(unix)]
fn tombed_long_ago(tomb: &Path, now: SystemTime) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(m) = std::fs::symlink_metadata(tomb) else {
        return false;
    };
    let changed = SystemTime::UNIX_EPOCH
        + Duration::from_secs(m.ctime().max(0) as u64)
        + Duration::from_nanos(m.ctime_nsec().max(0) as u64);
    now.duration_since(changed)
        .is_ok_and(|age| age >= STALE_AFTER)
}

#[cfg(not(unix))]
fn tombed_long_ago(tomb: &Path, now: SystemTime) -> bool {
    std::fs::metadata(tomb)
        .and_then(|m| m.modified())
        .is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age >= STALE_AFTER))
}

/// Other vaults' SearchIndex (project content indexes included) and
/// EmbedIndex files unused for an hour, open
/// in no other process and not the ones this process engaged, plus
/// tombstones an interrupted removal left. Refused when this vault's folder
/// can't be resolved.
pub fn stale_indexes(support: &Path, vault: &Path) -> Result<Plan, String> {
    stale_at(support, vault, SystemTime::now(), &engaged_db_paths())
}

fn stale_at(
    support: &Path,
    vault: &Path,
    now: SystemTime,
    engaged: &[PathBuf],
) -> Result<Plan, String> {
    let this = this_stem(vault)?;
    let mut files = Vec::new();
    let mut kept = Vec::new();
    let mut set = Vec::new();
    for (folder_kind, tomb) in tombstones(support) {
        let db = untombed(&tomb);
        let kind = kind_in(folder_kind, &db);
        let Some((Owner::OtherVault, Some(owner_name))) = named_owner(kind, &db, &this) else {
            continue;
        };
        // A removal still running, or a database made again and in use.
        if !tombed_long_ago(&tomb, now) || is_engaged(&db, engaged) || shm_in_use(&db) {
            continue;
        }
        let parts = tomb_set(&tomb);
        files.push(StaleFile {
            path: tomb.to_string_lossy().into_owned(),
            kind,
            owner_name,
            bytes: parts.iter().map(|(_, len, _)| len).sum(),
            modified: None,
            why: Some(INTERRUPTED.into()),
        });
        set.extend(parts);
    }
    for (kind, path) in db_files(support) {
        // Only names Hollow gives a vault's file (`<name>-<16 hex>`, or a
        // project index's `<name>-<16 hex>.projects-<16 hex>`).
        let Some((Owner::OtherVault, Some(owner_name))) = named_owner(kind, &path, &this) else {
            continue;
        };
        let shown = path.to_string_lossy().into_owned();
        if !kind.rebuildable() {
            kept.push(KeptFile {
                path: shown,
                kind,
                owner_name,
                why: "another vault's conversations: never removed".into(),
            });
            continue;
        }
        // The index this process engaged, under any stem (a symlinked vault
        // whose target changed since launch hashes to another one).
        if is_engaged(&path, engaged) {
            kept.push(KeptFile {
                path: shown,
                kind,
                owner_name,
                why: "open in this Hollow".into(),
            });
            continue;
        }
        if shm_in_use(&path) {
            kept.push(KeptFile {
                path: shown,
                kind,
                owner_name,
                why: "open in another Hollow".into(),
            });
            continue;
        }
        if recently_used(&path, now) {
            kept.push(KeptFile {
                path: shown,
                kind,
                owner_name,
                why: "used in the last hour".into(),
            });
            continue;
        }
        let parts = file_set(&path);
        files.push(StaleFile {
            path: shown,
            kind,
            owner_name,
            bytes: parts.iter().map(|(_, len, _)| len).sum(),
            modified: std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(rfc3339),
            why: None,
        });
        set.extend(parts);
    }
    Ok(Plan {
        folder: support.to_string_lossy().into_owned(),
        bytes: files.iter().map(|f| f.bytes).sum(),
        files,
        hash: hash_of(set),
        kept,
    })
}

/// Remove exactly the plan `expected_hash` names: anything that changed
/// since (a file written, added or gone) refuses and removes nothing.
pub fn remove_stale_indexes(
    support: &Path,
    vault: &Path,
    expected_hash: &str,
) -> Result<Removed, String> {
    remove_at(
        support,
        vault,
        expected_hash,
        SystemTime::now(),
        &engaged_db_paths(),
        &mut |_| {},
    )
}

/// Points inside a removal where tests act (start a connection, write a
/// file) to stand in for another process doing it at that moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step<'a> {
    /// The plan was made and its hash matched; nothing was touched yet.
    AfterPlan,
    /// This database's sidecar stayed, so it is about to be put back.
    BeforeRollback(&'a Path),
}

/// Why an index the plan offered was kept at its rename: in use since.
const OPENED_SINCE: &str = "opened since the list was made; kept";

fn remove_at(
    support: &Path,
    vault: &Path,
    expected_hash: &str,
    now: SystemTime,
    engaged: &[PathBuf],
    hook: &mut dyn FnMut(Step<'_>),
) -> Result<Removed, String> {
    let plan = stale_at(support, vault, now, engaged)?;
    if plan.hash != expected_hash {
        return Err("The old indexes to remove changed since the card was shown. Nothing was removed; ask again to see the new list.".into());
    }
    hook(Step::AfterPlan);
    let mut out = Removed {
        removed: Vec::new(),
        failed: Vec::new(),
        bytes: 0,
    };
    let failed =
        |path: &Path, e: std::io::Error| (path.to_string_lossy().into_owned(), e.to_string());
    for file in &plan.files {
        if file.why.is_some() {
            // A tombstone: the `-wal`/`-shm` its database left, then itself.
            let tomb = PathBuf::from(&file.path);
            let mut whole = true;
            let mut parts = tomb_set(&tomb);
            parts.sort_by_key(|(p, _, _)| *p == tomb);
            for (path, len, _) in parts {
                match std::fs::remove_file(&path) {
                    Ok(()) => out.bytes += len,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        out.failed.push(failed(&path, e));
                        whole = false;
                        break;
                    }
                }
            }
            if whole {
                out.removed.push(file.path.clone());
            }
            continue;
        }
        let db = PathBuf::from(&file.path);
        let parts = file_set(&db);
        let db_len = parts
            .iter()
            .find(|(p, _, _)| *p == db)
            .map_or(0, |(_, len, _)| *len);
        // The `.db` goes out of SQLite's sight first (a rename, which can't
        // half-happen), so no step below leaves a `-wal` beside a missing
        // database. If it can't be renamed, nothing of this index is touched.
        // A crash before the last delete leaves `<db>.removing`, which the
        // next plan offers once it is an hour old (`INTERRUPTED`).
        // The plan above was checked before the loop: a Hollow that opened
        // this file while earlier files were being removed keeps it. A
        // connection opened between this probe and the
        // rename is the window left (closing it needs a lock on the
        // `-shm`, not taken here).
        if shm_in_use(&db) || recently_used(&db, SystemTime::now()) {
            out.failed
                .push((file.path.clone(), OPENED_SINCE.to_string()));
            continue;
        }
        let tomb = sidecar(&db, TOMB);
        if let Err(e) = std::fs::rename(&db, &tomb) {
            out.failed.push(failed(&db, e));
            continue;
        }
        // `-shm` before `-wal`: if the `-wal` then stays, the database goes
        // back with its frames (SQLite rebuilds a missing `-shm`), never
        // without them.
        let shm = sidecar(&db, "-shm");
        let mut sidecars: Vec<_> = parts.iter().filter(|(p, _, _)| *p != db).collect();
        sidecars.sort_by_key(|(p, _, _)| *p != shm);
        let mut sidecars_gone = true;
        for (path, len, _) in sidecars {
            match std::fs::remove_file(path) {
                Ok(()) => out.bytes += len,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    out.failed.push(failed(path, e));
                    sidecars_gone = false;
                    break;
                }
            }
        }
        if !sidecars_gone {
            // A `-wal` or `-shm` stayed: the database goes back beside it,
            // whole. A link, not a rename, so a database made at that path
            // meanwhile is never replaced: then this one stays the tombstone,
            // offered once an hour old like an interrupted removal's.
            hook(Step::BeforeRollback(&db));
            match std::fs::hard_link(&tomb, &db) {
                Ok(()) => {
                    if let Err(e) = std::fs::remove_file(&tomb) {
                        out.failed.push(failed(&tomb, e));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    out.failed.push((
                        tomb.to_string_lossy().into_owned(),
                        format!(
                            "a new {} was made meanwhile, so this one was not put back over it",
                            db.file_name().unwrap_or_default().to_string_lossy()
                        ),
                    ));
                }
                Err(e) => out.failed.push(failed(&tomb, e)),
            }
            continue;
        }
        match std::fs::remove_file(&tomb) {
            Ok(()) => {
                out.bytes += db_len;
                out.removed.push(file.path.clone());
            }
            Err(e) => out.failed.push(failed(&tomb, e)),
        }
    }
    Ok(out)
}

/// Megabytes for a card, `0.4 MB` / `86 MB` / `1.2 GB`.
fn size(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

fn kind_words(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Sessions => "conversations",
        DbKind::SearchIndex => "search index",
        DbKind::EmbedIndex => "meaning-search index",
        DbKind::Companion => "companion ledger",
        DbKind::ProjectIndex => "project contents index",
    }
}

/// The card's lines for `system.remove_stale_indexes`: every file that goes
/// with its size, the total, and what stays. Settings' dialog says the same
/// in its own words (`diskGuard.ts` `staleConfirmBody`): change both.
pub fn describe(plan: &Plan) -> Vec<String> {
    let n = plan.files.len();
    let mut lines = vec![
        format!(
            "Removes {n} {} of other vaults from {}, {} in all. They are rebuilt from that vault if Hollow opens it again (meaning search embeds every note again).",
            if n == 1 { "index" } else { "indexes" },
            plan.folder,
            size(plan.bytes)
        ),
        "These go, each with its -wal and -shm:".into(),
    ];
    for f in &plan.files {
        let day = f
            .modified
            .as_deref()
            .map(|m| format!(", last changed {}", &m[..m.len().min(10)]))
            .unwrap_or_default();
        let day = f.why.as_deref().map_or(day, |why| format!(", {why}"));
        lines.push(format!(
            "• {} ({}, {}{}) · {}",
            Path::new(&f.path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            f.owner_name,
            kind_words(f.kind),
            day,
            size(f.bytes)
        ));
    }
    if !plan.kept.is_empty() {
        lines.push("These stay:".into());
        for k in &plan.kept {
            lines.push(format!(
                "• {} ({}, {}): {}",
                Path::new(&k.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                k.owner_name,
                kind_words(k.kind),
                k.why
            ));
        }
    }
    lines.push("This vault's files and every Sessions database are never removed. If this list changes before you confirm, nothing is removed.".into());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Bed {
        _dir: tempfile::TempDir,
        support: PathBuf,
        vault: PathBuf,
        other: PathBuf,
    }

    fn bed() -> Bed {
        let dir = tempfile::Builder::new()
            .prefix("hollow-databases-")
            .tempdir()
            .unwrap();
        let support = dir.path().join("support");
        let vault = dir.path().join("my-vault");
        let other = dir.path().join("Second brain");
        for d in [&support, &vault, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        Bed {
            _dir: dir,
            support,
            vault,
            other,
        }
    }

    /// A WAL-mode database at `<support>/<kind>/<stem>.db` with `rows` rows,
    /// closed (so its `-wal` is gone), at `age` old.
    fn make(support: &Path, kind: DbKind, stem: &str, rows: usize, age: Duration) -> PathBuf {
        let dir = support.join(kind.folder());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{stem}.db"));
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute_batch(
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT);
             CREATE VIRTUAL TABLE notes_fts USING fts5(body);
             PRAGMA user_version = 3;",
        )
        .unwrap();
        for i in 0..rows {
            conn.execute(
                "INSERT INTO notes(body) VALUES (?1)",
                [format!("secret row {i}")],
            )
            .unwrap();
        }
        drop(conn);
        set_age(&path, age);
        path
    }

    fn set_age(path: &Path, age: Duration) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    const OLD: Duration = Duration::from_secs(3 * 60 * 60);
    const YOUNG: Duration = Duration::from_secs(60);

    fn stem(vault: &Path) -> String {
        crate::session_store::vault_file_stem(vault)
    }

    /// The two indexes name their files exactly as `vault_file_stem` does,
    /// so "this vault's" is told by the stem alone (guards against drift).
    #[test]
    fn index_stems_match_vault_file_stem() {
        let b = bed();
        for vault in [&b.vault, &b.other] {
            let want = stem(vault);
            for path in [
                crate::vault_search_index::db_path_for(vault).unwrap(),
                crate::vault_embed_index::db_path_for(vault).unwrap(),
                crate::session_store::db_path_for(vault).unwrap(),
            ] {
                assert_eq!(path.file_stem().unwrap().to_string_lossy(), want);
            }
        }
    }

    #[test]
    fn files_are_classified_by_stem() {
        let b = bed();
        make(&b.support, DbKind::Sessions, &stem(&b.vault), 1, OLD);
        make(&b.support, DbKind::Sessions, &stem(&b.other), 1, OLD);
        make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 1, OLD);
        make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 1, OLD);
        make(&b.support, DbKind::Companion, "applied", 1, OLD);
        // Not a database of ours: ignored.
        std::fs::write(b.support.join("Companion").join("devices.json"), "{}").unwrap();
        let files = list(&b.support, &b.vault, false);
        let got: Vec<(DbKind, Owner, Option<&str>)> = files
            .iter()
            .map(|f| (f.kind, f.owner, f.owner_name.as_deref()))
            .collect();
        assert_eq!(
            got,
            [
                (DbKind::Sessions, Owner::ThisVault, None),
                (DbKind::Sessions, Owner::OtherVault, Some("Second-brain")),
                (DbKind::SearchIndex, Owner::ThisVault, None),
                (DbKind::EmbedIndex, Owner::OtherVault, Some("Second-brain")),
                (DbKind::Companion, Owner::App, None),
            ]
        );
        assert!(files
            .iter()
            .all(|f| f.schema_version == Some(3) && f.bytes > 0));
    }

    /// Measuring runs no counts and no check, and opens no connection to a
    /// closed WAL-mode file: no `-wal` or `-shm` appears beside it.
    #[test]
    fn measure_runs_no_counts_and_creates_nothing() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 3, OLD);
        let files = list(&b.support, &b.vault, false);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].tables, None);
        assert_eq!(files[0].quick_check, None);
        assert_eq!(files[0].schema_version, Some(3));
        assert!(!sidecar(&db, "-wal").exists());
        assert!(!sidecar(&db, "-shm").exists());
    }

    /// A live file (another connection open, so a `-wal` exists) is read
    /// through a read-only connection, which sees the WAL's changes.
    #[test]
    fn measure_reads_a_live_file_through_its_wal() {
        let b = bed();
        let db = make(&b.support, DbKind::Sessions, &stem(&b.vault), 1, OLD);
        let live = Connection::open(&db).unwrap();
        live.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        live.pragma_update(None, "user_version", 9).unwrap();
        let files = list(&b.support, &b.vault, false);
        assert_eq!(files[0].schema_version, Some(9));
        assert!(files[0].wal_bytes > 0);
        drop(live);
    }

    #[test]
    fn check_runs_quick_check_and_counts_ordinary_tables_only() {
        let b = bed();
        make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 4, OLD);
        let files = list(&b.support, &b.vault, true);
        // The FTS5 step can't run read-only: said, not reported as damage.
        assert_eq!(files[0].quick_check.as_deref(), Some(FTS_UNCHECKED));
        assert_eq!(files[0].error, None);
        // `notes` only: the FTS table and its shadow tables are left out.
        assert_eq!(
            files[0].tables,
            Some(vec![Table {
                name: "notes".into(),
                rows: 4
            }])
        );
        // Counts and names only: no row content in the answer.
        let json = serde_json::to_string(&files).unwrap();
        assert!(!json.contains("secret row"), "{json}");
    }

    #[test]
    fn a_check_past_its_deadline_says_so() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 1, OLD);
        let mut file = list(&b.support, &b.vault, false).remove(0);
        check_file(&db, Instant::now(), &mut file);
        assert_eq!(
            file.quick_check.as_deref(),
            Some("not finished within 30 s")
        );
    }

    /// The plan offers other vaults' old indexes only: never this vault's,
    /// never a Sessions file (listed as kept), never one used in the hour.
    #[test]
    fn the_plan_leaves_out_this_vault_sessions_and_young_files() {
        let b = bed();
        let third = b.vault.parent().unwrap().join("Vault");
        std::fs::create_dir_all(&third).unwrap();
        make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 1, OLD);
        make(&b.support, DbKind::EmbedIndex, &stem(&b.vault), 1, OLD);
        make(&b.support, DbKind::Sessions, &stem(&b.other), 1, OLD);
        let old_search = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 50, OLD);
        let old_embed = make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 50, OLD);
        make(&b.support, DbKind::SearchIndex, &stem(&third), 1, YOUNG);
        // An old .db whose -wal was written a minute ago is in use.
        let busy = make(&b.support, DbKind::EmbedIndex, &stem(&third), 1, OLD);
        std::fs::write(sidecar(&busy, "-wal"), b"frames").unwrap();
        // An empty -wal (a TRUNCATE checkpoint, a read-only check) is not use.
        std::fs::write(sidecar(&old_search, "-wal"), b"").unwrap();
        std::fs::write(sidecar(&old_search, "-shm"), [0u8; 64]).unwrap();

        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        let goes: Vec<&str> = plan.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            goes,
            [
                old_search.to_string_lossy().as_ref(),
                old_embed.to_string_lossy().as_ref()
            ]
        );
        assert!(plan.files.iter().all(|f| f.owner_name == "Second-brain"));
        let size = |p: &Path| std::fs::metadata(p).unwrap().len();
        assert_eq!(
            plan.bytes,
            size(&old_search) + 64 + size(&old_embed),
            "the -shm counts toward what is freed"
        );
        let mut kept: Vec<(DbKind, &str)> =
            plan.kept.iter().map(|k| (k.kind, k.why.as_str())).collect();
        kept.sort();
        assert_eq!(
            kept,
            [
                (
                    DbKind::Sessions,
                    "another vault's conversations: never removed"
                ),
                (DbKind::SearchIndex, "used in the last hour"),
                (DbKind::EmbedIndex, "used in the last hour"),
            ]
        );
        let card = describe(&plan).join("\n");
        assert!(card.contains("Removes 2 indexes of other vaults"), "{card}");
        assert!(card.contains("never removed"), "{card}");
        assert_eq!(report(&b.support, &b.vault, false).stale_count, 2);
    }

    /// Repo features R14b: a vault's content index of a Projects folder is
    /// its own kind, whose by the vault stem in its name. This vault's is
    /// never offered, however old; another vault's unused one is, like its
    /// search index. A vault folder named `projects` keeps its search
    /// index's kind.
    #[test]
    fn project_content_indexes_belong_to_their_vault_and_only_others_are_offered() {
        let b = bed();
        let root = b.vault.parent().unwrap();
        let projects = root.join("Projects");
        std::fs::create_dir_all(&projects).unwrap();
        let stem_of = |vault: &Path| {
            crate::project_text_index::db_file_name(vault, &projects)
                .trim_end_matches(".db")
                .to_string()
        };
        let live = make(&b.support, DbKind::ProjectIndex, &stem_of(&b.vault), 1, OLD);
        let elsewhere = make(&b.support, DbKind::ProjectIndex, &stem_of(&b.other), 1, OLD);
        // Not beside `Projects`: macOS folders are case-insensitive.
        let named_projects = root.join("old").join("projects");
        std::fs::create_dir_all(&named_projects).unwrap();
        make(
            &b.support,
            DbKind::SearchIndex,
            &stem(&named_projects),
            1,
            OLD,
        );

        let listed: Vec<(DbKind, Owner, Option<String>)> = list(&b.support, &b.vault, false)
            .into_iter()
            .map(|f| (f.kind, f.owner, f.owner_name))
            .collect();
        assert_eq!(
            listed,
            [
                (
                    DbKind::SearchIndex,
                    Owner::OtherVault,
                    Some("projects".into())
                ),
                (DbKind::ProjectIndex, Owner::ThisVault, None),
                (
                    DbKind::ProjectIndex,
                    Owner::OtherVault,
                    Some("Second-brain".into())
                ),
            ]
        );
        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        let goes: Vec<(&str, DbKind, &str)> = plan
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.kind, f.owner_name.as_str()))
            .collect();
        assert!(
            !goes.iter().any(|g| g.0 == live.to_string_lossy()),
            "{goes:?}"
        );
        assert_eq!(goes.len(), 2, "{goes:?}");
        assert_eq!(
            goes[1],
            (
                elsewhere.to_string_lossy().as_ref(),
                DbKind::ProjectIndex,
                "Second-brain"
            )
        );
        assert!(describe(&plan)
            .join("\n")
            .contains("project contents index"));
        // Removing them leaves this vault's.
        remove_stale_indexes(&b.support, &b.vault, &plan.hash).unwrap();
        assert!(live.is_file() && !elsewhere.exists());
    }

    #[test]
    fn a_hash_mismatch_removes_nothing() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 1, OLD);
        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        assert_eq!(plan.files.len(), 1);
        // Written since the plan (its length and time change): refused.
        let conn = Connection::open(&db).unwrap();
        conn.execute("INSERT INTO notes(body) VALUES ('later')", [])
            .unwrap();
        drop(conn);
        set_age(&db, OLD);
        let err = remove_stale_indexes(&b.support, &b.vault, &plan.hash).unwrap_err();
        assert!(err.contains("changed since the card was shown"), "{err}");
        assert!(db.exists());
        let err = remove_stale_indexes(&b.support, &b.vault, "not-a-hash").unwrap_err();
        assert!(err.contains("Nothing was removed"), "{err}");
        assert!(db.exists());
    }

    #[test]
    fn removal_takes_the_wal_and_shm_and_leaves_the_rest() {
        let b = bed();
        let mine = make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 1, OLD);
        let sessions = make(&b.support, DbKind::Sessions, &stem(&b.other), 1, OLD);
        let db = make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 1, OLD);
        let (wal, shm) = (sidecar(&db, "-wal"), sidecar(&db, "-shm"));
        std::fs::write(&wal, b"").unwrap();
        std::fs::write(&shm, [0u8; 32]).unwrap();
        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        let out = remove_stale_indexes(&b.support, &b.vault, &plan.hash).unwrap();
        assert_eq!(out.removed, [db.to_string_lossy().into_owned()]);
        assert!(out.failed.is_empty());
        assert_eq!(out.bytes, plan.bytes);
        assert!(!db.exists() && !wal.exists() && !shm.exists());
        assert!(mine.exists() && sessions.exists());
        // Nothing left to offer.
        assert!(stale_indexes(&b.support, &b.vault)
            .unwrap()
            .files
            .is_empty());
    }

    /// Without an FTS table a healthy file is plain `ok`; a file with
    /// damaged pages is not.
    #[test]
    fn check_reports_damage() {
        let b = bed();
        let dir = b.support.join("Sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.db", stem(&b.vault)));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA page_size = 4096; PRAGMA journal_mode = WAL; CREATE TABLE t(x TEXT);",
        )
        .unwrap();
        for i in 0..500 {
            conn.execute("INSERT INTO t VALUES (?1)", [format!("{i:0>200}")])
                .unwrap();
        }
        drop(conn);
        assert_eq!(
            list(&b.support, &b.vault, true)[0].quick_check.as_deref(),
            Some("ok")
        );
        // Every page but the first (the schema) overwritten with garbage.
        let len = std::fs::metadata(&path).unwrap().len() as usize;
        assert!(len > 4 * 4096, "{len}");
        let mut bytes = std::fs::read(&path).unwrap();
        bytes[4096..].fill(0xA5);
        std::fs::write(&path, &bytes).unwrap();
        let checked = list(&b.support, &b.vault, true).remove(0);
        let result = checked.quick_check.unwrap_or_default();
        assert!(!result.is_empty() && !result.starts_with("ok"), "{result}");
    }

    /// The child side of [`Holder`] (an_index_open_in_another_process_is_kept
    /// and an_index_opened_after_the_plan_is_kept): holds a WAL connection
    /// open on `HOLLOW_TEST_HOLD_DB`, idle with an empty `-wal` (what R7a's
    /// TRUNCATE checkpoint leaves), until its stdin closes. Does nothing
    /// otherwise.
    #[test]
    #[ignore = "a child-process helper, not a test; an_index_open_in_another_process_is_kept and an_index_opened_after_the_plan_is_kept run it with HOLLOW_TEST_HOLD_DB"]
    fn hold_open_helper() {
        let Some(db) = std::env::var_os("HOLLOW_TEST_HOLD_DB") else {
            return;
        };
        let conn = Connection::open(&db).unwrap();
        conn.execute("INSERT INTO notes(body) VALUES ('idle')", [])
            .unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
        println!("READY");
        let mut rest = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut rest);
        drop(conn);
    }

    /// Another process (this test binary running `hold_open_helper`) with
    /// a connection open on a database, until [`Holder::release`].
    struct Holder {
        child: std::process::Child,
        out: std::io::Lines<std::io::BufReader<std::process::ChildStdout>>,
    }

    impl Holder {
        /// Starts the helper on `db` and waits until its connection is open
        /// (it has written a row and checkpointed, so the `.db` is new).
        fn open(db: &Path) -> Holder {
            use std::io::BufRead;
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "hollow_databases::tests::hold_open_helper",
                    "--ignored",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("HOLLOW_TEST_HOLD_DB", db)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            // libtest prints the line as `test … ... READY`.
            let mut out = std::io::BufReader::new(child.stdout.take().unwrap()).lines();
            let ready = out
                .by_ref()
                .map_while(Result::ok)
                .any(|l| l.trim_end().ends_with("READY"));
            if !ready {
                let _ = child.kill();
            }
            assert!(ready, "the helper never opened the index");
            Holder { child, out }
        }

        /// Closes the helper's connection and waits for it to exit cleanly.
        fn release(mut self) {
            drop(self.child.stdin.take());
            self.out.for_each(drop);
            assert!(self.child.wait().unwrap().success());
        }
    }

    /// Another Hollow with that vault open, idle for hours: its index has an
    /// old `.db` and an empty `-wal`, and must not be offered. POSIX locks
    /// don't conflict within a process, so the holder is a child process
    /// (this test binary running `hold_open_helper`).
    #[test]
    fn an_index_open_in_another_process_is_kept() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 3, OLD);
        let holder = Holder::open(&db);
        assert_eq!(std::fs::metadata(sidecar(&db, "-wal")).unwrap().len(), 0);
        set_age(&db, OLD);
        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        // Let the helper go before asserting, so a failure can't leave it waiting.
        holder.release();
        let kept: Vec<&str> = plan.kept.iter().map(|k| k.why.as_str()).collect();
        assert!(
            plan.files.is_empty(),
            "offered while open elsewhere: {plan:?}"
        );
        assert_eq!(kept, ["open in another Hollow"]);
        // Closed: the same file, as old, is offered.
        set_age(&db, OLD);
        assert_eq!(stale_indexes(&b.support, &b.vault).unwrap().files.len(), 1);
    }

    /// A `-wal` that can't be removed (here a directory) puts the `.db`
    /// back: no step leaves a `-wal` beside a missing database.
    #[test]
    fn a_failed_removal_leaves_the_database_whole() {
        let b = bed();
        let db = make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 1, OLD);
        let wal = sidecar(&db, "-wal");
        std::fs::create_dir(&wal).unwrap();
        std::fs::write(wal.join("x"), b"x").unwrap();
        std::fs::File::open(&wal)
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        let plan = stale_indexes(&b.support, &b.vault).unwrap();
        assert_eq!(plan.files.len(), 1, "{plan:?}");
        let out = remove_stale_indexes(&b.support, &b.vault, &plan.hash).unwrap();
        assert!(out.removed.is_empty(), "{out:?}");
        assert_eq!(out.failed.len(), 1, "{out:?}");
        assert_eq!(out.failed[0].0, wal.to_string_lossy());
        assert!(db.exists(), "the database went back");
        assert!(!sidecar(&db, ".removing").exists());
        assert!(wal.exists());
    }

    /// A database made at the path while a removal had it renamed aside is
    /// never replaced by the put-back: its bytes survive, and the old one
    /// stays the tombstone, reported.
    #[test]
    fn a_put_back_never_replaces_a_database_made_meanwhile() {
        let b = bed();
        let db = make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 1, OLD);
        let wal = sidecar(&db, "-wal");
        std::fs::create_dir(&wal).unwrap();
        std::fs::write(wal.join("x"), b"x").unwrap();
        std::fs::File::open(&wal)
            .unwrap()
            .set_modified(SystemTime::now() - OLD)
            .unwrap();
        let tomb = sidecar(&db, TOMB);
        let old_bytes = std::fs::read(&db).unwrap();
        let now = SystemTime::now();
        let plan = stale_at(&b.support, &b.vault, now, &[]).unwrap();
        assert_eq!(plan.files.len(), 1, "{plan:?}");
        let mut rolled_back = Vec::new();
        let out = remove_at(&b.support, &b.vault, &plan.hash, now, &[], &mut |step| {
            if let Step::BeforeRollback(at) = step {
                rolled_back.push(at.to_path_buf());
                std::fs::write(at, b"a new database").unwrap();
            }
        })
        .unwrap();
        assert_eq!(rolled_back, std::slice::from_ref(&db));
        assert!(out.removed.is_empty(), "{out:?}");
        assert_eq!(std::fs::read(&db).unwrap(), b"a new database");
        assert_eq!(
            std::fs::read(&tomb).unwrap(),
            old_bytes,
            "the old one stays"
        );
        let failed: Vec<&str> = out.failed.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            failed,
            [
                wal.to_string_lossy().as_ref(),
                tomb.to_string_lossy().as_ref()
            ]
        );
        assert!(out.failed[1].1.contains("meanwhile"), "{out:?}");
    }

    /// A Hollow that opens an index after the list was made keeps it: the
    /// removal probes again just before each rename.
    #[test]
    fn an_index_opened_after_the_plan_is_kept() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 3, OLD);
        let now = SystemTime::now();
        let plan = stale_at(&b.support, &b.vault, now, &[]).unwrap();
        assert_eq!(plan.files.len(), 1, "{plan:?}");
        let mut holder = None;
        let out = remove_at(&b.support, &b.vault, &plan.hash, now, &[], &mut |step| {
            if step == Step::AfterPlan {
                holder = Some(Holder::open(&db));
                // Idle for hours with an empty `-wal`: only the `-shm`
                // lock says it is open.
                set_age(&db, OLD);
            }
        });
        // Let the helper go before asserting, so a failure can't leave it waiting.
        holder.expect("the hook ran").release();
        let out = out.unwrap();
        assert!(out.removed.is_empty(), "{out:?}");
        assert_eq!(
            out.failed,
            [(db.to_string_lossy().into_owned(), OPENED_SINCE.to_string())]
        );
        assert!(!sidecar(&db, TOMB).exists());
        let rows: i64 = Connection::open(&db)
            .unwrap()
            .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 4, "the three rows and the helper's");
    }

    /// A vault folder that can't be resolved can't be told from others':
    /// nothing is planned or removed.
    #[test]
    fn an_unresolvable_vault_plans_nothing() {
        let b = bed();
        let gone = b.vault.parent().unwrap().join("gone");
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 1, OLD);
        let err = stale_indexes(&b.support, &gone).unwrap_err();
        assert!(err.contains("can't resolve this vault's folder"), "{err}");
        assert!(remove_stale_indexes(&b.support, &gone, "x").is_err());
        assert!(db.exists());
        assert_eq!(report(&b.support, &gone, false).stale_count, 0);
    }

    /// Only regular files named `<name>-<16 hex>.db` are offered: a symlink
    /// isn't listed at all, an odd name is listed but never offered.
    #[test]
    fn symlinks_and_odd_names_are_never_offered() {
        let b = bed();
        let outside = b.vault.parent().unwrap().join("outside");
        let target = make(&outside, DbKind::SearchIndex, "elsewhere", 1, OLD);
        let dir = b.support.join("SearchIndex");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&target, dir.join(format!("{}.db", stem(&b.other)))).unwrap();
        make(&b.support, DbKind::SearchIndex, "notes", 1, OLD);
        let listed: Vec<String> = list(&b.support, &b.vault, false)
            .into_iter()
            .map(|f| f.stem)
            .collect();
        assert_eq!(listed, ["notes"]);
        assert!(stale_indexes(&b.support, &b.vault)
            .unwrap()
            .files
            .is_empty());
    }

    /// The index this process engaged is never offered, even under another
    /// vault's stem and when named through a symlinked folder.
    #[test]
    fn the_engaged_index_is_never_offered() {
        let b = bed();
        let db = make(&b.support, DbKind::SearchIndex, &stem(&b.other), 1, OLD);
        let now = SystemTime::now();
        assert_eq!(
            stale_at(&b.support, &b.vault, now, &[])
                .unwrap()
                .files
                .len(),
            1
        );
        let link = b.vault.parent().unwrap().join("support-link");
        std::os::unix::fs::symlink(&b.support, &link).unwrap();
        let engaged = [link.join("SearchIndex").join(db.file_name().unwrap())];
        let plan = stale_at(&b.support, &b.vault, now, &engaged).unwrap();
        assert!(plan.files.is_empty(), "{plan:?}");
        let kept: Vec<&str> = plan.kept.iter().map(|k| k.why.as_str()).collect();
        assert_eq!(kept, ["open in this Hollow"]);
        // Removal plans the same way: the engaged file stays.
        let out = remove_at(&b.support, &b.vault, &plan.hash, now, &engaged, &mut |_| {}).unwrap();
        assert!(out.removed.is_empty() && db.exists());
    }

    /// A removal that stopped after its rename leaves `<db>.removing` and
    /// the database's `-wal`/`-shm`: offered once an hour old, then removed
    /// by the same flow.
    #[test]
    fn an_interrupted_removal_is_offered_and_cleaned() {
        let b = bed();
        let db = make(&b.support, DbKind::EmbedIndex, &stem(&b.other), 1, OLD);
        let tomb = sidecar(&db, TOMB);
        std::fs::rename(&db, &tomb).unwrap();
        let (wal, shm) = (sidecar(&db, "-wal"), sidecar(&db, "-shm"));
        std::fs::write(&wal, b"").unwrap();
        std::fs::write(&shm, [0u8; 32]).unwrap();
        // Not listed as a database, and not offered while it may be a
        // removal still running.
        assert!(list(&b.support, &b.vault, false).is_empty());
        assert!(stale_indexes(&b.support, &b.vault)
            .unwrap()
            .files
            .is_empty());
        let later = SystemTime::now() + 2 * STALE_AFTER;
        let plan = stale_at(&b.support, &b.vault, later, &[]).unwrap();
        assert_eq!(plan.files.len(), 1, "{plan:?}");
        assert_eq!(plan.files[0].path, tomb.to_string_lossy());
        assert_eq!(plan.files[0].why.as_deref(), Some(INTERRUPTED));
        let size = std::fs::metadata(&tomb).unwrap().len();
        assert_eq!(plan.bytes, size + 32);
        assert!(describe(&plan).join("\n").contains(INTERRUPTED));
        let out = remove_at(&b.support, &b.vault, &plan.hash, later, &[], &mut |_| {}).unwrap();
        assert_eq!(out.removed, [tomb.to_string_lossy().into_owned()]);
        assert!(!tomb.exists() && !wal.exists() && !shm.exists());
        // A tombstone of this vault's index is never offered.
        let mine = make(&b.support, DbKind::SearchIndex, &stem(&b.vault), 1, OLD);
        std::fs::rename(&mine, sidecar(&mine, TOMB)).unwrap();
        assert!(stale_at(&b.support, &b.vault, later, &[])
            .unwrap()
            .files
            .is_empty());
    }

    #[test]
    fn vault_names_come_from_stems() {
        assert_eq!(vault_name("hollow-vault-1d1d4c39cd10e040"), "hollow-vault");
        assert_eq!(vault_name("applied"), "applied");
        assert_eq!(vault_name("x-notsixteenhexdigit"), "x-notsixteenhexdigit");
    }
}
