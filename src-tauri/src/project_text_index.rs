//! The content index for connected projects (repo features R14b): a BM25
//! word index of the text inside every listed text, PDF and Office file in
//! the Projects folder, so retrieval can choose connected files by what they
//! say instead of by name and then the newest.
//!
//! **Storage.** One SQLite file per vault and Projects folder,
//! `{vault_search_index::index_root()}/<vault stem>.projects-<16 hex of the
//! canonical Projects path>.db` ([`db_file_name`]): the vault stem is
//! `session_store::vault_file_stem` (never a `.` in it), so one vault's
//! switch never touches another vault's index, and `hollow_databases` tells
//! whose a file is by name (its `projectIndex` kind; another vault's unused
//! one is offered for removal like its search index, this vault's never).
//! Created mode 0600 (SQLite gives its `-wal`/`-shm` the same mode), WAL with
//! R7a's `journal_size_limit`, `secure_delete` on. Schema v1: `files(id
//! INTEGER PRIMARY KEY, path UNIQUE, size, mtime, kind, units, chars, error)`
//! and the FTS5 table `words(path UNINDEXED, name, body,
//! tokenize='unicode61 remove_diacritics 2')`, `words.rowid = files.id`.
//! `words` is contentless (`content=''`, `contentless_delete=1`), which
//! halves the file: retrieval reads files fresh, so a second copy of their
//! text would buy nothing. That is a size property, not a privacy one: the
//! positional index still holds every word. So a pass that drops rows merges
//! segments under `secure_delete` (freed pages are zeroed): a full FTS5
//! `optimize` (every tombstoned entry goes) when a dropped file is now in a
//! restricted folder or secret-named, or more than 5% of rows went; else a
//! bounded `merge` step of 500 pages. It is a derived cache: deleting it means
//! a rebuild.
//!
//! **Indexing.** [`reconcile`] diffs the listed inventory
//! (`directory_connections::load_inventory`) against `files`: it drops rows
//! for files removed, left out or no longer readable, and reads changed Text
//! (`read_text`) and Extracted (`doc_text::extract`) files, newest first, at
//! most [`BUDGET`] (1,500 files or 48 MB on disk) per call. It runs from
//! `directory_connections::start` after each refresh, whether or not the
//! refresh worked (so the off sweep below runs with the Projects folder
//! gone, RF-830-F1), on a blocking thread. Unchanged files (same size and
//! modification time) are never re-read.
//!
//! **Privacy.** The index never holds text a read would refuse: only files
//! in the listed inventory are read, a fresh membership walk
//! (`directory_connections::confirm`, the check `read` makes) runs over each
//! batch first, and a file with a `restricted` folder anywhere in its path or
//! a component `secret_name` matches is skipped and counted (`skipped`).
//!
//! **Crash-loop guard, two strikes.** PDFKit runs in this process. Before a
//! document is parsed its row is written with `error = 'parsing'` and
//! committed; the result replaces it. A row still `parsing` at the same size
//! and modification time on a later pass means that parse never returned:
//! Hollow crashed, or was quit mid-parse. It is tried once more, marked
//! `parsing (retry)`; a row still `parsing (retry)` on the pass after is
//! marked `crashed while parsing` and never parsed again, so one malformed
//! PDF can't crash every launch. Retrieval never reads a file in any of these
//! states. A changed file starts over. The parse itself goes through
//! `doc_text::extract_guarded`, the crash marker retrieval uses, so a file
//! that crashed either reader twice is refused by both (stored as `crashed
//! while parsing`), and turning the index off and on, which drops its rows,
//! doesn't buy it more parses (RF-795-F1). It runs on its own thread: a pass
//! waits for it at most [`PARSE_WAIT`] (two minutes) and only while the
//! index stays on, then leaves it running with its `parsing` row and skips
//! the file until it returns, so a parse that never returns can't hold
//! [`WRITE`] (turning the index off ends the pass's wait within about 100
//! ms) or stop later passes (RF-795-F2). A left parse that does return (a
//! slow, healthy file) is no strike: the next pass stores what it read
//! (RF-R1 review F1); that text is held in memory only while the index is
//! on, and goes when the index does (off, disconnect, reconnect: D1). A file another reader is parsing (`doc_text`'s "still
//! being read") is no strike either: its row goes and the next pass reads
//! it as new.
//!
//! **On only when the user turns it on** (Settings, Advanced, Search & indexes, default
//! off since 2026-10-19): turning it on writes a marker beside the vault's
//! manifest (`<vault stem>.content-index-on`) and starts a pass. Turning it
//! off removes that marker and deletes every project index of this vault,
//! with its `-wal` and `-shm`, whether or not its Projects folder is
//! reachable, and any `-wal`/`-shm` of it left without its `.db`; so does a
//! pass while it is off (an index a default-on build left). The legacy off
//! marker (`<vault stem>.content-index-off`, from when it was on by default)
//! still wins over the on marker; turning it on removes it. Connecting another Projects folder deletes
//! this vault's index of the old one ([`forget_other_roots_now`]; never
//! waiting on a running pass, which leaves it to the next one), and so does a
//! pass that finds the connection gone or moved.
//!
//! **Reading.** Retrieval (`directory_connections::project_passages`) opens
//! the file read-only per request ([`Snapshot`]), in the GUI and in the
//! `hollow_vault` subprocess alike, as `vault_search_index`'s read-only
//! path does, and uses it only when it covers at least 95% of the files in
//! scope.

use crate::directory_connections::{FileState, Source};
use rusqlite::{params, Connection, OpenFlags};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 1: `files` + contentless `words`.
pub(crate) const SCHEMA_VERSION: i64 = 1;
/// A document's row while it is being parsed (the crash-loop guard).
pub(crate) const PARSING: &str = "parsing";
/// The same, on the one retry after a parse that never returned.
pub(crate) const RETRYING: &str = "parsing (retry)";
/// A document whose parse never returned twice running.
pub(crate) const CRASHED: &str = "crashed while parsing";
/// `bm25()` over `words`' columns (path, name, body): the name counts twice.
const RANK_SQL: &str = "SELECT rowid, bm25(words, 0.0, 2.0, 1.0) AS s FROM words \
                        WHERE words MATCH ?1 ORDER BY s, rowid;";

/// What one [`reconcile`] may read.
pub(crate) struct Budget {
    pub files: usize,
    /// Bytes on disk. The first file is read whatever its size (a document
    /// is at most `doc_text`'s 64 MB), so a large one still gets indexed.
    pub bytes: u64,
}

pub(crate) const BUDGET: Budget = Budget {
    files: 1_500,
    bytes: 48 * 1024 * 1024,
};

/// Held by [`reconcile`] and by [`set_enabled`] while it deletes the file.
static WRITE: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    /// Files [`reconcile`] read on this thread, for tests.
    static READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Tests make storing a read's result fail.
    static FAIL_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test seam: [`PARSE_WAIT`] for passes on this thread.
    static PARSE_WAIT_FOR_TEST: std::cell::Cell<Option<Duration>> =
        const { std::cell::Cell::new(None) };
}

/// How long a pass waits on one document's parse before it leaves it
/// running and moves on (RF-795-F2): generous, since a 500-page PDF takes a
/// while, but bounded, so a parse that never returns can't hold [`WRITE`].
const PARSE_WAIT: Duration = Duration::from_secs(120);

/// [`PARSE_WAIT`], or a test's own on its thread.
fn parse_wait() -> Duration {
    #[cfg(test)]
    if let Some(wait) = PARSE_WAIT_FOR_TEST.with(|w| w.get()) {
        return wait;
    }
    PARSE_WAIT
}

/// Settings, Advanced, Search & indexes's line, `connected_index_status`.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    /// Listed files whose current text is in the index.
    pub indexed: usize,
    /// Listed text, PDF and Office files the index is for (not `skipped`).
    pub total: usize,
    /// Read and failed (a damaged file, `crashed while parsing`).
    pub failed: usize,
    /// PDFs with no text layer (they need OCR, which Hollow doesn't do).
    pub image_only: usize,
    /// Left out on purpose: in a `restricted` folder or secret-named.
    pub skipped: usize,
}

/// What one [`reconcile`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Reconciled {
    /// Files read (text or parsed).
    pub read: usize,
    /// Rows dropped (file gone, left out, skipped).
    pub removed: usize,
    /// Rows newly marked [`CRASHED`].
    pub crashed: usize,
    /// Documents tried again after one parse that never returned.
    pub retried: usize,
    /// Rows went and the whole index was rewritten (FTS5 `optimize`), not
    /// just merged a step.
    pub optimized: bool,
}

/// A pass that drops more than this percentage of rows rewrites the whole
/// index (`optimize`); fewer get a bounded `merge` step, unless privacy asks.
const FULL_OPTIMIZE_PERCENT: usize = 5;

/// 16 hex digits of the sha256 of `root`'s canonical path.
fn root_hex(root: &Path) -> String {
    let canon = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let hash = format!("{:x}", Sha256::digest(canon.to_string_lossy().as_bytes()));
    hash[..16].to_string()
}

/// `<vault stem>.projects-<16 hex>.db` for `vault`'s index of the Projects
/// folder `root`.
pub(crate) fn db_file_name(vault: &Path, root: &Path) -> String {
    format!(
        "{}.projects-{}.db",
        crate::session_store::vault_file_stem(vault),
        root_hex(root)
    )
}

/// The vault stem and root hex of a name [`db_file_name`] gives; `None` for
/// any other name (a vault index's `<name>-<16 hex>.db` among them).
pub(crate) fn parse_db_file_name(name: &str) -> Option<(&str, &str)> {
    let (stem, hex) = name.strip_suffix(".db")?.split_once(".projects-")?;
    let hex16 = |h: &str| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit());
    let (vault_name, vault_hex) = stem.rsplit_once('-')?;
    (!vault_name.is_empty() && !stem.contains('.') && hex16(vault_hex) && hex16(hex))
        .then_some((stem, hex))
}

/// Whether `name` is a file name [`db_file_name`] gives.
pub(crate) fn is_db_file_name(name: &str) -> bool {
    parse_db_file_name(name).is_some()
}

pub(crate) fn db_path_for(vault: &Path, root: &Path) -> Option<PathBuf> {
    Some(crate::vault_search_index::index_root()?.join(db_file_name(vault, root)))
}

/// Every project index database of `vault` (any Projects folder), by the
/// regular files there: a `.db`, or a `-wal`/`-shm` left without one (an
/// orphan, which [`remove_db`] sweeps the same way). Sorted, no repeats.
fn vault_dbs(vault: &Path) -> Vec<PathBuf> {
    let Some(dir) = crate::vault_search_index::index_root() else {
        return Vec::new();
    };
    let this = crate::session_store::vault_file_stem(vault);
    let mut out: Vec<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file()))
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?;
            let db = ["-wal", "-shm"]
                .iter()
                .find_map(|s| name.strip_suffix(s))
                .unwrap_or(name);
            let (stem, _) = parse_db_file_name(db)?;
            (stem == this).then(|| dir.join(db))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Delete `vault`'s project indexes of every Projects folder but `keep`
/// (all of them with `None`), each with its `-wal`/`-shm`, and what left
/// parses read outside `keep` ([`RETURNED`]); how many indexes went. For a
/// caller holding [`WRITE`].
fn forget_locked(vault: &Path, keep: Option<&Path>) -> Result<usize, String> {
    // What left parses read for this vault goes with its index (D1).
    RETURNED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|k| k.vault != vault || keep.is_some_and(|root| k.path.starts_with(root)));
    let kept = keep.and_then(|root| db_path_for(vault, root));
    let mut gone = 0;
    for db in vault_dbs(vault) {
        if kept.as_ref() != Some(&db) {
            remove_db(&db)?;
            gone += 1;
        }
    }
    Ok(gone)
}

/// [`forget_locked`], waiting for a running [`reconcile`]: what turning the
/// index off calls.
pub(crate) fn forget_other_roots(vault: &Path, keep: Option<&Path>) -> Result<usize, String> {
    let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    forget_locked(vault, keep)
}

/// [`forget_locked`] only when no [`reconcile`] is running (`None` then):
/// what connecting another Projects folder calls, so `connect` (which holds
/// the connection's lock) never waits on an indexing pass. A skipped
/// deletion is the next pass's: it forgets every folder but the one now
/// connected before it writes anything.
pub(crate) fn forget_other_roots_now(
    vault: &Path,
    keep: Option<&Path>,
) -> Option<Result<usize, String>> {
    let _guard = match WRITE.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    Some(forget_locked(vault, keep))
}

/// `<vault stem>.content-index-<state>` beside `vault`'s manifest.
fn marker_path(vault: &Path, state: &str) -> PathBuf {
    let manifest = crate::directory_connections::manifest_path(vault);
    let stem = manifest
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    manifest.with_file_name(format!("{stem}.content-index-{state}"))
}

/// The marker that turns the index on for `vault`.
pub(crate) fn on_path(vault: &Path) -> PathBuf {
    marker_path(vault, "on")
}

/// The legacy marker that turned it off while it was on by default. It
/// still wins over [`on_path`].
fn off_path(vault: &Path) -> PathBuf {
    marker_path(vault, "off")
}

/// Off unless the user turned it on (the default is off): the on marker
/// exists and no off marker does.
pub(crate) fn enabled(vault: &Path) -> bool {
    fs::symlink_metadata(on_path(vault)).is_ok() && fs::symlink_metadata(off_path(vault)).is_err()
}

/// A path under the Projects folder the index never holds: a `restricted`
/// folder anywhere in it, or a component `secret_name` matches.
pub(crate) fn private(relative: &str) -> bool {
    Path::new(relative).components().any(|c| {
        let part = c.as_os_str().to_string_lossy().to_lowercase();
        part == "restricted" || crate::directory_connections::secret_name(&part)
    })
}

/// A listed file the index is for, privacy aside.
fn indexable(f: &Source) -> bool {
    matches!(f.state, FileState::Text | FileState::Extracted)
}

fn kind_word(f: &Source) -> &'static str {
    match crate::doc_text::kind(Path::new(&f.path)) {
        Some(crate::doc_text::DocKind::Pdf) => "pdf",
        Some(crate::doc_text::DocKind::Docx) => "docx",
        Some(crate::doc_text::DocKind::Pptx) => "pptx",
        Some(crate::doc_text::DocKind::Xlsx) => "xlsx",
        None => "text",
    }
}

/// One `files` row.
struct Row {
    /// `files.id`, also its `words` rowid.
    id: i64,
    size: u64,
    mtime: u64,
    error: Option<String>,
}

impl Row {
    fn current(&self, f: &Source) -> bool {
        self.size == f.size && self.mtime == f.modified
    }
}

fn load_rows(conn: &Connection) -> Option<HashMap<String, Row>> {
    let mut stmt = conn
        .prepare("SELECT id, path, size, mtime, error FROM files;")
        .ok()?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                Row {
                    id: r.get(0)?,
                    size: r.get::<_, i64>(2)? as u64,
                    mtime: r.get::<_, i64>(3)? as u64,
                    error: r.get(4)?,
                },
            ))
        })
        .ok()?;
    let mut out = HashMap::new();
    for row in rows {
        let (path, row) = row.ok()?;
        out.insert(path, row);
    }
    Some(out)
}

/// Create `db` mode 0600 (refusing a link), then open it read-write at this
/// schema. A newer build's file is left alone.
fn open_rw(db: &Path) -> Result<Connection, String> {
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if fs::symlink_metadata(db).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(format!("{} is not a regular file", db.display()));
    }
    {
        let mut options = fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        options.open(db).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(db, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
    }
    let conn = Connection::open(db).map_err(|e| e.to_string())?;
    if let Some(stored) = crate::vault_search_index::newer_schema(&conn, SCHEMA_VERSION) {
        return Err(format!(
            "the project content index is at schema {stored}, newer than this build's {SCHEMA_VERSION}; left as it is"
        ));
    }
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL");
    let _ = conn.pragma_update(
        None,
        "journal_size_limit",
        crate::vault_search_index::WAL_LIMIT_BYTES,
    );
    let _ = conn.busy_timeout(std::time::Duration::from_millis(2000));
    // Freed pages are zeroed, so a dropped file's words don't linger in the
    // free list (with `optimize` after a pass that drops rows).
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(|e| e.to_string())?;
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap_or(0);
    if version != 0 && version != SCHEMA_VERSION {
        conn.execute_batch("DROP TABLE IF EXISTS words; DROP TABLE IF EXISTS files;")
            .map_err(|e| e.to_string())?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS files(\
             id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, \
             size INTEGER NOT NULL, mtime INTEGER NOT NULL, \
             kind TEXT NOT NULL, units INTEGER NOT NULL DEFAULT 0, \
             chars INTEGER NOT NULL DEFAULT 0, error TEXT);\
         CREATE VIRTUAL TABLE IF NOT EXISTS words USING fts5(\
             path UNINDEXED, name, body, content='', contentless_delete=1, \
             tokenize='unicode61 remove_diacritics 2');",
    )
    .map_err(|e| e.to_string())?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| e.to_string())?;
    Ok(conn)
}

/// `files` row for `f` with `error`, its words cleared; its id.
fn put_row(conn: &Connection, f: &Source, error: Option<&str>) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO files(path, size, mtime, kind, units, chars, error) \
         VALUES(?1, ?2, ?3, ?4, 0, 0, ?5) \
         ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, \
         kind = excluded.kind, units = 0, chars = 0, error = excluded.error;",
        params![
            f.path,
            f.size as i64,
            f.modified as i64,
            kind_word(f),
            error
        ],
    )?;
    let id: i64 = conn.query_row("SELECT id FROM files WHERE path = ?1;", [&f.path], |r| {
        r.get(0)
    })?;
    conn.execute("DELETE FROM words WHERE rowid = ?1;", [id])?;
    Ok(id)
}

/// Store what reading `f` gave.
fn put_result(
    conn: &Connection,
    f: &Source,
    result: Result<(String, usize), String>,
) -> rusqlite::Result<()> {
    #[cfg(test)]
    if FAIL_WRITE.with(|c| c.get()) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let tx = conn.unchecked_transaction()?;
    match result {
        Ok((text, units)) => {
            let id = put_row(&tx, f, None)?;
            tx.execute(
                "UPDATE files SET units = ?2, chars = ?3 WHERE id = ?1;",
                params![id, units as i64, text.chars().count() as i64],
            )?;
            let name = Path::new(&f.relative)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            tx.execute(
                "INSERT INTO words(rowid, path, name, body) VALUES(?1, ?2, ?3, ?4);",
                params![id, f.path, name, text],
            )?;
        }
        Err(e) => {
            put_row(&tx, f, Some(&e))?;
        }
    }
    tx.commit()
}

/// What one document's parse gave: its text and units, or why not.
type ParseResult = Result<(String, usize), String>;

/// Documents the index is parsing ([`parse_document`]), until the parse
/// returns: one a pass left running isn't parsed again meanwhile.
static LEFT: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// What a parse a pass left running read once it returned: the next pass
/// stores it instead of counting a strike or parsing again (RF-R1 review
/// F1). Only while the index is on: a parse that returns while it is off
/// keeps nothing, and [`forget_locked`] (off, the off sweep, disconnecting
/// and reconnecting) drops the vault's entries outside the folder it keeps
/// (delta review D1).
static RETURNED: Mutex<Vec<Kept>> = Mutex::new(Vec::new());

/// One [`RETURNED`] entry: the file by path, size and modification time as
/// listed when its parse began.
struct Kept {
    vault: PathBuf,
    path: PathBuf,
    size: u64,
    mtime: u64,
    result: ParseResult,
}

impl Kept {
    fn is(&self, f: &Source) -> bool {
        self.path.as_os_str() == f.path.as_str() && (self.size, self.mtime) == (f.size, f.modified)
    }
}

fn left_running(path: &Path) -> bool {
    let left = LEFT.lock().unwrap_or_else(|p| p.into_inner());
    left.iter().any(|p| p == path)
}

/// `path`'s parse returned: out of [`LEFT`].
fn unleave(path: &Path) {
    let mut left = LEFT.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(i) = left.iter().position(|p| p == path) {
        left.swap_remove(i);
    }
}

/// Whether a left parse of `f` as listed now returned ([`RETURNED`]).
fn has_returned(f: &Source) -> bool {
    let returned = RETURNED.lock().unwrap_or_else(|p| p.into_inner());
    returned.iter().any(|k| k.is(f))
}

/// Whether [`RETURNED`] keeps anything read from `path`, for tests.
#[cfg(test)]
fn kept(path: &Path) -> bool {
    let returned = RETURNED.lock().unwrap_or_else(|p| p.into_inner());
    returned.iter().any(|k| k.path == path)
}

/// What a left parse of `f` as listed now read, taken out of [`RETURNED`].
fn take_returned(f: &Source) -> Option<ParseResult> {
    let mut returned = RETURNED.lock().unwrap_or_else(|p| p.into_inner());
    let at = returned.iter().position(|k| k.is(f))?;
    Some(returned.swap_remove(at).result)
}

/// What parsing one document for a pass came to.
enum Parsed {
    Done(ParseResult),
    /// Another reader's parse of it is still running (`doc_text`'s
    /// `BEING_READ`): no strike; read it as new next pass.
    Busy,
    /// This pass stopped waiting: the parse outlasted [`PARSE_WAIT`] or the
    /// index was turned off. It runs on, in [`LEFT`], and what it reads goes
    /// to [`RETURNED`].
    Left,
}

/// A parse's result in the index's terms.
fn settle(result: ParseResult) -> Parsed {
    match result {
        Err(e) if e == crate::doc_text::BEING_READ => Parsed::Busy,
        // Refused by the marker: the index's own word for it.
        Err(e) if e == crate::doc_text::CRASHED_READER => Parsed::Done(Err(CRASHED.into())),
        done => Parsed::Done(done),
    }
}

/// Between a pass and its parse thread: the result, or that the pass left.
struct Handoff {
    result: Option<ParseResult>,
    left: bool,
}

/// Parse `f` behind `doc_text`'s crash marker (shared with retrieval, so a
/// document is parsed at most twice in all: RF-795-F1) on its own thread,
/// and wait for it while the index stays on, at most [`PARSE_WAIT`]
/// (RF-795-F2): a parse that never returns then holds only its own thread
/// and the file's marker, never [`WRITE`]. One that returns after the pass
/// left leaves its result in [`RETURNED`] for the next pass.
fn parse_document(vault: &Path, f: &Source) -> Parsed {
    use std::sync::{Arc, Condvar};
    let owned = PathBuf::from(&f.path);
    let (size, mtime, vault_owned) = (f.size, f.modified, vault.to_path_buf());
    let shared = Arc::new((
        Mutex::new(Handoff {
            result: None,
            left: false,
        }),
        Condvar::new(),
    ));
    LEFT.lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(owned.clone());
    let theirs = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("project-index-parse".into())
        .spawn(move || {
            // A panic is an error here (debug builds; release ones abort).
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::doc_text::extract_guarded(&owned)
            }))
            .unwrap_or_else(|_| Err("the document reader stopped".into()))
            .map(|doc| (doc.text, doc.units));
            let (slot, ready) = &*theirs;
            let mut slot = slot.lock().unwrap_or_else(|p| p.into_inner());
            if slot.left {
                // Kept before it leaves LEFT, so a pass never sees neither;
                // and only while the index is on, checked under the lock
                // `forget_locked` purges under, so turning it off meanwhile
                // can't miss it.
                let mut returned = RETURNED.lock().unwrap_or_else(|p| p.into_inner());
                if enabled(&vault_owned) {
                    returned.push(Kept {
                        vault: vault_owned,
                        path: owned.clone(),
                        size,
                        mtime,
                        result: parsed,
                    });
                }
            } else {
                slot.result = Some(parsed);
                ready.notify_one();
            }
            drop(slot);
            unleave(&owned);
        });
    if let Err(e) = spawned {
        unleave(Path::new(&f.path));
        return Parsed::Done(Err(e.to_string()));
    }
    let deadline = Instant::now() + parse_wait();
    let (slot, ready) = &*shared;
    let mut slot = slot.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if let Some(result) = slot.result.take() {
            return settle(result);
        }
        if !enabled(vault) || Instant::now() >= deadline {
            slot.left = true;
            return Parsed::Left;
        }
        slot = ready
            .wait_timeout(slot, Duration::from_millis(100))
            .unwrap_or_else(|p| p.into_inner())
            .0;
    }
}

/// Drop `path`'s row (and its words), so the next pass reads it as new.
/// Best effort.
fn drop_row(conn: &Connection, path: &str) {
    let _ = conn.execute(
        "DELETE FROM words WHERE rowid IN (SELECT id FROM files WHERE path = ?1);",
        [path],
    );
    let _ = conn.execute("DELETE FROM files WHERE path = ?1;", [path]);
}

/// Read one file's text for the index: a text file through `read_text`, a
/// document through [`parse_document`] after its in-progress row (`marker`:
/// [`PARSING`], or [`RETRYING`] on the retry) is committed. Whether its
/// result was stored (not when the parse is still running).
fn read_one(vault: &Path, conn: &Connection, f: &Source, marker: &str) -> rusqlite::Result<bool> {
    #[cfg(test)]
    READS.with(|c| c.set(c.get() + 1));
    let path = Path::new(&f.path);
    let result = if f.state == FileState::Extracted {
        let parsed = match take_returned(f) {
            // A parse an earlier pass left running read it: no parse now.
            Some(result) => settle(result),
            None => {
                // The crash-loop guard: committed before PDFKit sees the file.
                put_row(conn, f, Some(marker))?;
                match path.canonicalize() {
                    Ok(real) if real == path => parse_document(vault, f),
                    Ok(_) => {
                        Parsed::Done(Err("Source path changed; refresh the connection".into()))
                    }
                    Err(e) => Parsed::Done(Err(e.to_string())),
                }
            }
        };
        match parsed {
            Parsed::Done(result) => result,
            Parsed::Busy => {
                drop_row(conn, &f.path);
                return Ok(false);
            }
            // The in-progress row stays: a parse that hasn't returned, which
            // retrieval doesn't start beside it (`held`). Passes skip the
            // file while it runs; what it reads when it returns is stored by
            // the next one (`RETURNED`). If Hollow quits first, the row counts
            // the strike as any other would.
            Parsed::Left => return Ok(false),
        }
    } else {
        crate::directory_connections::read_text(path).map(|text| (text, 0))
    };
    let stored = put_result(conn, f, result);
    if stored.is_err() {
        // The parse returned; only the write failed. Leaving the in-progress
        // row would count as a strike next pass: drop it, so the file is
        // read again as new.
        drop_row(conn, &f.path);
    }
    stored.map(|()| true)
}

/// Bring the index for `vault`'s Projects folder up to date with its listed
/// inventory, reading at most `budget` (see the module doc). Blocking: run
/// it on a blocking thread. Does nothing while the index is off.
pub(crate) fn reconcile(vault: &Path, budget: &Budget) -> Result<Reconciled, String> {
    let mut done = Reconciled::default();
    if !enabled(vault) {
        // Off keeps no index, even one a default-on build made; unless it
        // was turned on while this waited (review: a stale off pass must
        // not delete the index the new on pass just built).
        let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
        if !enabled(vault) {
            forget_locked(vault, None)?;
        }
        return Ok(done);
    }
    let Ok(connection) = crate::directory_connections::load(vault) else {
        return Ok(done);
    };
    if connection.projects_path.is_empty() {
        // Not connected (any more): no index of any folder stays.
        forget_other_roots(vault, None)?;
        return Ok(done);
    }
    // Unreachable (unmounted, say): keep the index for when it is back.
    let Some(root) = crate::directory_connections::connected_root(vault) else {
        return Ok(done);
    };
    let inv = crate::directory_connections::load_inventory(vault);
    if inv.root != root {
        return Ok(done);
    }
    let _guard = WRITE.lock().unwrap_or_else(|p| p.into_inner());
    // Turned off while this waited: it must not make the file again.
    if !enabled(vault) {
        return Ok(done);
    }
    // The connection moved to another folder: the old one's index goes.
    forget_locked(vault, Some(&root))?;
    let db = db_path_for(vault, &root).ok_or("Hollow has no app support folder")?;
    let conn = open_rw(&db)?;
    let rows = load_rows(&conn).ok_or("the project content index can't be read")?;
    let wanted: HashMap<&str, &Source> = inv
        .files
        .iter()
        .filter(|f| indexable(f) && !private(&f.relative))
        .map(|f| (f.path.as_str(), f))
        .collect();
    // Listed files the index must not hold, by name, size and time: a row
    // dropped because its file moved into a restricted folder matches one.
    let file_name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let moved_private: HashSet<(String, u64, u64)> = inv
        .files
        .iter()
        .filter(|f| indexable(f) && private(&f.relative))
        .map(|f| (file_name(&f.path), f.size, f.modified))
        .collect();
    let mut private_drop = false;
    for (path, row) in &rows {
        if !wanted.contains_key(path.as_str()) {
            conn.execute("DELETE FROM words WHERE rowid = ?1;", [row.id])
                .and_then(|_| conn.execute("DELETE FROM files WHERE id = ?1;", [row.id]))
                .map_err(|e| e.to_string())?;
            done.removed += 1;
            let relative = Path::new(path)
                .strip_prefix(&root)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_default();
            private_drop |= private(&relative)
                || moved_private.contains(&(file_name(path), row.size, row.mtime));
        }
    }
    if done.removed > 0 {
        // The dropped rows' entries (contentless tombstones) go when their
        // segments merge, and `secure_delete` zeroes the pages they held. A
        // full `optimize` rewrites the whole index, so it runs only when
        // privacy asks (a file now in a restricted folder or secret-named)
        // or many rows went; otherwise a bounded merge step (review: lock
        // files coming and going would rewrite a large index each minute).
        done.optimized = private_drop || done.removed * 100 > rows.len() * FULL_OPTIMIZE_PERCENT;
        let step = if done.optimized {
            "INSERT INTO words(words) VALUES('optimize');"
        } else {
            "INSERT INTO words(words, rank) VALUES('merge', 500);"
        };
        conn.execute(step, []).map_err(|e| e.to_string())?;
    }
    // A left parse's result for a file of this vault no longer wanted as it
    // is now (changed, gone, private, another folder) is never stored.
    RETURNED
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|k| {
            k.vault != vault
                || k.path
                    .to_str()
                    .and_then(|p| wanted.get(p))
                    .is_some_and(|f| k.is(f))
        });
    let mut todo: Vec<&Source> = Vec::new();
    let mut retry: HashSet<&str> = HashSet::new();
    for f in wanted.values() {
        // A parse an earlier pass left running is no strike yet, and isn't
        // started again beside itself.
        if left_running(Path::new(&f.path)) {
            continue;
        }
        // It returned since: stored as read (`read_one`), not a retry.
        if has_returned(f) {
            todo.push(f);
            continue;
        }
        match rows.get(&f.path) {
            Some(row) if row.current(f) => match row.error.as_deref() {
                // One parse that never returned (a crash, or a quit
                // mid-parse): try it once more.
                Some(PARSING) => {
                    retry.insert(f.path.as_str());
                    todo.push(f);
                }
                // The retry never returned either: never again at this
                // size and time.
                Some(RETRYING) => {
                    conn.execute(
                        "UPDATE files SET error = ?2 WHERE id = ?1;",
                        params![row.id, CRASHED],
                    )
                    .map_err(|e| e.to_string())?;
                    done.crashed += 1;
                }
                _ => {}
            },
            _ => todo.push(f),
        }
    }
    todo.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
    let mut chosen: Vec<&Source> = Vec::new();
    let mut bytes = 0u64;
    for f in todo {
        if chosen.len() >= budget.files {
            break;
        }
        if !chosen.is_empty() && bytes + f.size > budget.bytes {
            continue;
        }
        bytes += f.size;
        chosen.push(f);
    }
    if !chosen.is_empty() {
        // The membership check `read` makes, once for the batch.
        let targets: Vec<PathBuf> = chosen.iter().map(|f| PathBuf::from(&f.path)).collect();
        let present = crate::directory_connections::confirm(&root, vault, &targets);
        for f in chosen {
            if !enabled(vault) {
                break;
            }
            if !present.contains(Path::new(&f.path)) {
                continue;
            }
            let retried = retry.contains(f.path.as_str());
            if read_one(vault, &conn, f, if retried { RETRYING } else { PARSING })
                .map_err(|e| e.to_string())?
            {
                done.read += 1;
                done.retried += usize::from(retried);
            }
        }
    }
    if done.read + done.removed + done.crashed > 0 {
        let _ = crate::vault_search_index::checkpoint_truncate(&conn);
    }
    Ok(done)
}

/// Delete `db` and its `-wal`/`-shm`.
fn remove_db(db: &Path) -> Result<(), String> {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = db.as_os_str().to_owned();
        name.push(suffix);
        match fs::remove_file(PathBuf::from(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Couldn't delete the project content index: {e}")),
        }
    }
    Ok(())
}

/// Turn the index on or off for `vault`. Off deletes every project index of
/// this vault (each with its `-wal` and `-shm`, whether or not its Projects
/// folder is reachable now) after any running [`reconcile`] stops; on lets
/// the next one build it. Blocking.
pub(crate) fn set_enabled(vault: &Path, on: bool) -> Result<Status, String> {
    let remove = |path: &Path| match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    };
    let marker = on_path(vault);
    if on {
        // The legacy off marker would still win.
        remove(&off_path(vault))?;
        if let Some(parent) = marker.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(
            &marker,
            "Search inside project files is on (Settings > Advanced > Search & indexes).\n",
        )
        .map_err(|e| e.to_string())?;
    } else {
        remove(&marker)?;
        forget_other_roots(vault, None)?;
    }
    Ok(status(vault))
}

/// `db` opened read-only, if it exists at this schema.
fn open_read_only(db: &Path) -> Option<Connection> {
    if !db.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .ok()?;
    (version == SCHEMA_VERSION).then_some(conn)
}

/// One read-only look at the index, for a request or a status line: the
/// rows as one read transaction saw them, and BM25 over the same snapshot.
pub(crate) struct Snapshot {
    conn: Connection,
    rows: HashMap<String, Row>,
    paths: HashMap<i64, String>,
}

impl Snapshot {
    /// `None` while the index is off, before it exists, at another schema
    /// or on any SQLite error.
    pub(crate) fn open(vault: &Path, root: &Path) -> Option<Snapshot> {
        if !enabled(vault) {
            return None;
        }
        let conn = open_read_only(&db_path_for(vault, root)?)?;
        conn.execute_batch("BEGIN;").ok()?;
        let rows = load_rows(&conn)?;
        let paths = rows.iter().map(|(p, r)| (r.id, p.clone())).collect();
        Some(Snapshot { conn, rows, paths })
    }

    /// The index holds `f` as listed now (read, whatever came of it).
    pub(crate) fn covers(&self, f: &Source) -> bool {
        self.rows.get(&f.path).is_some_and(|r| {
            r.current(f) && !matches!(r.error.as_deref(), Some(PARSING | RETRYING))
        })
    }

    /// Why retrieval must not parse `f` now, as its line-0 excerpt: its
    /// parse never returned twice running, or once (or it is running now)
    /// and the index is trying it again. `None` for any other file.
    pub(crate) fn held(&self, f: &Source) -> Option<&'static str> {
        let row = self.rows.get(&f.path).filter(|r| r.current(f))?;
        match row.error.as_deref()? {
            CRASHED => Some("File discovered in place; not read: Hollow stopped twice while reading this file, so it skips it until the file changes. Open the original."),
            PARSING | RETRYING => Some("File discovered in place; not read yet: an earlier read of it was interrupted, and Hollow is trying it again in the background."),
            _ => None,
        }
    }

    /// The best `k` files `eligible` admits for `terms_lower` (ORed prefix
    /// terms, `vault_search_index::words_query`), best BM25 first, ties by
    /// rowid. `None` when no term is one word (the caller reads as before)
    /// or on an error; a partial ranking is never trusted.
    pub(crate) fn rank(
        &self,
        terms_lower: &[String],
        k: usize,
        eligible: impl Fn(&str) -> bool,
    ) -> Option<Vec<(String, f64)>> {
        let query = crate::vault_search_index::words_query(terms_lower)?;
        let mut stmt = self.conn.prepare(RANK_SQL).ok()?;
        let rows = stmt
            .query_map([query], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?)))
            .ok()?;
        let mut out = Vec::new();
        for row in rows {
            let (rowid, bm25) = row.ok()?;
            if let Some(path) = self.paths.get(&rowid) {
                if eligible(path) {
                    out.push((path.clone(), -bm25));
                    if out.len() == k {
                        break;
                    }
                }
            }
        }
        Some(out)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = self.conn.execute_batch("COMMIT;");
    }
}

/// Connected files in `project` (an id; every project when `None`) ranked
/// by their contents for `terms_lower`, best first, at most `k`: the index's
/// answer alone, scoped by the project's path prefix.
#[cfg(test)]
pub(crate) fn ranked(
    vault: &Path,
    terms_lower: &[String],
    project: Option<&str>,
    k: usize,
) -> Option<Vec<String>> {
    let root = crate::directory_connections::connected_root(vault)?;
    let prefix = match project {
        Some(id) => Some(format!(
            "{}/",
            crate::directory_connections::project_root(vault, id)?.display()
        )),
        None => None,
    };
    let snapshot = Snapshot::open(vault, &root)?;
    let ranked = snapshot.rank(terms_lower, k, |p| {
        prefix.as_ref().is_none_or(|x| p.starts_with(x.as_str()))
    })?;
    Some(ranked.into_iter().map(|(p, _)| p).collect())
}

/// The index's progress for Settings and Library: listed files it is for,
/// how many it holds as they are now, image-only PDFs, failures and files
/// left out for privacy. Blocking (reads the inventory).
pub(crate) fn status(vault: &Path) -> Status {
    let mut out = Status {
        enabled: enabled(vault),
        ..Status::default()
    };
    let Some(root) = crate::directory_connections::connected_root(vault) else {
        return out;
    };
    let inv = crate::directory_connections::load_inventory(vault);
    if inv.root != root {
        return out;
    }
    let snapshot = out.enabled.then(|| Snapshot::open(vault, &root)).flatten();
    let mut seen = HashSet::new();
    for f in inv.files.iter().filter(|f| indexable(f)) {
        if !seen.insert(f.path.as_str()) {
            continue;
        }
        if private(&f.relative) {
            out.skipped += 1;
            continue;
        }
        out.total += 1;
        let Some(row) = snapshot
            .as_ref()
            .and_then(|s| s.rows.get(&f.path))
            .filter(|r| r.current(f))
        else {
            continue;
        };
        match row.error.as_deref() {
            None => out.indexed += 1,
            Some(PARSING | RETRYING) => {}
            Some(e) if e.starts_with("image-only") => out.image_only += 1,
            Some(_) => out.failed += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directory_connections as dc;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("Projects");
        let v = t.path().join("Vault");
        fs::create_dir(&p).unwrap();
        fs::create_dir(&v).unwrap();
        (t, p.canonicalize().unwrap(), v.canonicalize().unwrap())
    }
    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    /// Connect `p` and turn the index on (it is off by default).
    fn connect(t: &tempfile::TempDir, p: &Path, v: &Path) {
        connect_only(t, p, v);
        set_enabled(v, true).unwrap();
    }
    fn connect_only(t: &tempfile::TempDir, p: &Path, v: &Path) {
        dc::connect(
            p.to_str().unwrap(),
            v.to_str().unwrap(),
            &t.path().join("config.json"),
        )
        .unwrap();
    }
    fn reads() -> usize {
        READS.with(|c| c.replace(0))
    }
    fn words(q: &str) -> Vec<String> {
        q.split_whitespace().map(str::to_lowercase).collect()
    }
    fn names(paths: &[String]) -> Vec<&str> {
        let mut out: Vec<&str> = paths
            .iter()
            .map(|p| p.rsplit('/').next().unwrap())
            .collect();
        out.sort();
        out
    }

    #[test]
    fn unchanged_files_are_not_read_again_and_a_changed_one_is() {
        let (t, p, v) = fixture();
        put(&p, "A/one.md", "zebrafish quota\n");
        put(&p, "A/two.md", "nothing here\n");
        connect(&t, &p, &v);
        reads();
        let first = reconcile(&v, &BUDGET).unwrap();
        assert_eq!((first.read, reads()), (2, 2));
        let again = reconcile(&v, &BUDGET).unwrap();
        assert_eq!((again, reads()), (Reconciled::default(), 0));
        put(&p, "A/two.md", "now about zebrafish too\n");
        // A new file changes the folder's time, so the refresh re-stats it.
        put(&p, "A/three.md", "three\n");
        dc::refresh(&v).unwrap();
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 2);
        assert_eq!(reads(), 2, "two.md (changed) and three.md (new) only");
        let s = status(&v);
        assert_eq!((s.enabled, s.indexed, s.total), (true, 3, 3));
    }

    #[test]
    fn the_budget_bounds_one_call_newest_first() {
        let (t, p, v) = fixture();
        for i in 0..5 {
            put(&p, &format!("A/n{i}.md"), "text\n");
        }
        connect(&t, &p, &v);
        let small = Budget {
            files: 2,
            bytes: u64::MAX,
        };
        assert_eq!(reconcile(&v, &small).unwrap().read, 2);
        assert_eq!(status(&v).indexed, 2);
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 3);
        let s = status(&v);
        assert_eq!((s.indexed, s.total), (5, 5));
    }

    #[test]
    fn a_removed_file_leaves_the_index() {
        let (t, p, v) = fixture();
        put(&p, "A/keep.md", "zebrafish keep\n");
        put(&p, "A/gone.md", "zebrafish gone\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        assert_eq!(
            names(&ranked(&v, &words("zebrafish"), None, 200).unwrap()),
            ["gone.md", "keep.md"]
        );
        fs::remove_file(p.join("A/gone.md")).unwrap();
        dc::refresh(&v).unwrap();
        assert_eq!(reconcile(&v, &BUDGET).unwrap().removed, 1);
        assert_eq!(
            names(&ranked(&v, &words("zebrafish"), None, 200).unwrap()),
            ["keep.md"]
        );
    }

    /// Privacy: a file in a restricted folder and a secret-named one are
    /// never indexed (reads of the second are refused; the first is left out
    /// on purpose), and the status counts the first as skipped.
    #[test]
    fn restricted_and_secret_named_files_are_never_indexed() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish plan\n");
        put(&p, "A/restricted/payroll.md", "zebrafish payroll\n");
        put(&p, "A/HR/Restricted/review.md", "zebrafish review\n");
        put(&p, "A/client-secrets.md", "zebrafish secret\n");
        put(&p, "A/credentials/notes.md", "zebrafish creds\n");
        connect(&t, &p, &v);
        reads();
        reconcile(&v, &BUDGET).unwrap();
        assert_eq!(reads(), 1, "only plan.md is read");
        assert_eq!(
            names(&ranked(&v, &words("zebrafish"), None, 200).unwrap()),
            ["plan.md"]
        );
        let conn = open_read_only(&db_path_for(&v, &p).unwrap()).unwrap();
        let held: Vec<String> = conn
            .prepare("SELECT path FROM files ORDER BY path;")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(names(&held), ["plan.md"]);
        let s = status(&v);
        assert_eq!((s.indexed, s.total, s.skipped), (1, 1, 2));
        // The rule itself, whatever the scan lists.
        assert!(private("A/restricted/x.md") && private("A/RESTRICTED/x.md"));
        assert!(private("A/my-secret-plan.md") && private("A/id_rsa.md"));
        assert!(private("A/token-list.md") && private("Ops/credentials/a.md"));
        assert!(!private("A/restricted-ish/x.md") && !private("A/plan.md"));
    }

    #[test]
    fn the_index_is_read_only_elsewhere_and_scoped_by_project() {
        let (t, p, v) = fixture();
        put(&p, "Alpha/.git/HEAD", "ref: refs/heads/main");
        put(&p, "Alpha/notes.md", "zebrafish alpha\n");
        put(&p, "Beta/.git/HEAD", "ref: refs/heads/main");
        put(&p, "Beta/notes.md", "zebrafish beta\n");
        put(&p, "Beta/other.md", "nothing\n");
        connect(&t, &p, &v);
        // Nothing to read before the first reconcile.
        assert!(ranked(&v, &words("zebrafish"), None, 200).is_none());
        reconcile(&v, &BUDGET).unwrap();
        let db = db_path_for(&v, &p).unwrap();
        #[cfg(unix)]
        {
            // The database, and its -wal and -shm while a writer has them.
            use std::os::unix::fs::PermissionsExt;
            let writer = open_rw(&db).unwrap();
            writer
                .execute("UPDATE files SET chars = chars;", [])
                .unwrap();
            for suffix in ["", "-wal", "-shm"] {
                let mut name = db.as_os_str().to_owned();
                name.push(suffix);
                let mode = fs::metadata(PathBuf::from(&name))
                    .unwrap_or_else(|e| panic!("{name:?}: {e}"))
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o600, "{name:?}");
            }
            drop(writer);
        }
        // The writer is closed: this is a fresh read-only connection.
        let all = ranked(&v, &words("zebrafish"), None, 200).unwrap();
        assert_eq!(all.len(), 2);
        let ro = open_read_only(&db).unwrap();
        assert!(ro.execute("DELETE FROM files;", []).is_err(), "read-only");
        let beta = dc::projects(&v)
            .into_iter()
            .find(|x| x.title == "Beta")
            .unwrap();
        let scoped = ranked(&v, &words("zebrafish"), Some(&beta.id), 200).unwrap();
        assert_eq!(scoped.len(), 1);
        assert!(scoped[0].ends_with("Beta/notes.md"), "{scoped:?}");
        // A prefix term finds the longer word; the name counts too.
        let by_prefix = ranked(&v, &words("zebra"), None, 200).unwrap();
        assert_eq!(by_prefix.len(), 2);
        let by_name = ranked(&v, &words("other"), None, 200).unwrap();
        assert_eq!(names(&by_name), ["other.md"]);
    }

    #[test]
    fn turning_the_index_off_deletes_it_and_keeps_it_off() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish plan\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let db = db_path_for(&v, &p).unwrap();
        assert!(db.is_file());
        // A reader keeps -wal/-shm present, as the subprocess's would.
        let reader = Snapshot::open(&v, &p).unwrap();
        let s = set_enabled(&v, false).unwrap();
        drop(reader);
        assert!(!s.enabled);
        for suffix in ["", "-wal", "-shm"] {
            let mut name = db.as_os_str().to_owned();
            name.push(suffix);
            assert!(!PathBuf::from(&name).exists(), "{name:?} is left");
        }
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        assert!(!db.exists(), "off stays off");
        assert!(ranked(&v, &words("zebrafish"), None, 200).is_none());
        let s = set_enabled(&v, true).unwrap();
        assert!(s.enabled && s.indexed == 0);
        reconcile(&v, &BUDGET).unwrap();
        assert_eq!(status(&v).indexed, 1);
        assert!(on_path(&v).is_file() && !off_path(&v).exists());
        set_enabled(&v, false).unwrap();
        assert!(!on_path(&v).exists(), "off removes the on marker");
    }

    /// Off by default (Alex, 2026-10-19): a fresh vault reads and keeps
    /// nothing until the switch is turned on, and status, ranking and
    /// retrieval's snapshot all treat "never turned on" as off.
    #[test]
    fn a_fresh_vault_is_off_and_indexes_nothing_until_turned_on() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish plan\n");
        connect_only(&t, &p, &v);
        assert!(!enabled(&v));
        reads();
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        assert_eq!(reads(), 0);
        let db = db_path_for(&v, &p).unwrap();
        assert!(!db.exists(), "off made an index");
        assert!(Snapshot::open(&v, &p).is_none());
        assert!(ranked(&v, &words("zebrafish"), None, 200).is_none());
        let s = status(&v);
        assert_eq!((s.enabled, s.indexed, s.total), (false, 0, 1));
        let s = set_enabled(&v, true).unwrap();
        assert!(s.enabled && s.indexed == 0);
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 1);
        assert_eq!(
            names(&ranked(&v, &words("zebrafish"), None, 200).unwrap()),
            ["plan.md"]
        );
    }

    /// The off marker a default-on build wrote still reads as off, even
    /// beside an on marker; turning the switch on removes it. A pass while
    /// off deletes an index a default-on build left.
    #[test]
    fn the_legacy_off_marker_stays_off() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish plan\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let db = db_path_for(&v, &p).unwrap();
        assert!(db.is_file());
        // As a default-on build left it: its index and its off marker.
        fs::write(off_path(&v), "off\n").unwrap();
        fs::remove_file(on_path(&v)).unwrap();
        assert!(!enabled(&v));
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        assert!(!db.exists(), "a pass while off left the old index");
        assert!(!status(&v).enabled);
        // Both markers: off wins.
        fs::write(on_path(&v), "on\n").unwrap();
        assert!(!enabled(&v));
        reads();
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        assert_eq!(reads(), 0);
        assert!(!db.exists());
        let s = set_enabled(&v, true).unwrap();
        assert!(s.enabled && !off_path(&v).exists());
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 1);
    }

    /// The crash-loop guard, two strikes: a document whose row still says
    /// `parsing` (a crash or a quit mid-parse) is tried once more; one still
    /// `parsing (retry)` after that is never parsed again, and is marked;
    /// once it changes, it is tried again.
    #[test]
    fn a_parse_that_never_returned_is_retried_once_then_skipped() {
        use crate::doc_text::tests::docx_with;
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        fs::write(p.join("A/deck.docx"), docx_with(&["zebrafish minutes"])).unwrap();
        put(&p, "A/plan.md", "plan\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        assert_eq!(status(&v).indexed, 2);
        // What a crash mid-parse leaves: the in-progress row.
        let inv = dc::load_inventory(&v);
        let deck = inv
            .files
            .iter()
            .find(|f| f.relative == "A/deck.docx")
            .unwrap();
        let db = db_path_for(&v, &p).unwrap();
        let strike = |marker: &str| {
            let conn = open_rw(&db).unwrap();
            put_row(&conn, deck, Some(marker)).unwrap();
        };
        strike(PARSING);
        // Retrieval doesn't parse it meanwhile, and says why.
        let snapshot = Snapshot::open(&v, &p).unwrap();
        assert!(snapshot.held(deck).unwrap().contains("interrupted"));
        assert!(!snapshot.covers(deck));
        drop(snapshot);
        // First strike (a quit looks the same): tried again, and it reads.
        reads();
        let next = reconcile(&v, &BUDGET).unwrap();
        assert_eq!(
            (next.read, next.retried, next.crashed, reads()),
            (1, 1, 0, 1)
        );
        assert_eq!(status(&v).indexed, 2);
        // What a second crash leaves: the retry's in-progress row.
        strike(RETRYING);
        reads();
        let next = reconcile(&v, &BUDGET).unwrap();
        assert_eq!((next.read, next.crashed, reads()), (0, 1, 0));
        let snapshot = Snapshot::open(&v, &p).unwrap();
        assert!(snapshot.held(deck).unwrap().contains("stopped twice"));
        assert!(snapshot.covers(deck));
        drop(snapshot);
        let s = status(&v);
        assert_eq!((s.indexed, s.failed), (1, 1));
        // Never again at this size and time.
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        // A changed file is tried again.
        fs::write(
            p.join("A/deck.docx"),
            docx_with(&["zebrafish minutes", "and more"]),
        )
        .unwrap();
        put(&p, "A/new.md", "new\n");
        dc::refresh(&v).unwrap();
        reads();
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 2);
        assert_eq!(reads(), 2, "deck.docx again, and new.md");
        assert_eq!(status(&v).indexed, 3);
    }

    #[test]
    fn db_names_name_the_vault_and_never_look_like_a_vault_index() {
        let (_t, p, v) = fixture();
        let name = db_file_name(&v, &p);
        let stem = crate::session_store::vault_file_stem(&v);
        assert_eq!(
            parse_db_file_name(&name).map(|(s, _)| s),
            Some(stem.as_str())
        );
        assert!(!is_db_file_name("projects-0123456789abcdef.db"));
        assert!(!is_db_file_name("Vault-0123456789abcdef.db"));
        assert!(!is_db_file_name("Vault.projects-0123456789abcdef.db"));
        assert!(!is_db_file_name(
            "a.b-0123456789abcdef.projects-0123456789abcdef.db"
        ));
    }

    /// Review blocker: off deletes the index even when the Projects folder
    /// can't be reached (unmounted), and only this vault's.
    #[test]
    fn off_deletes_the_index_with_its_projects_folder_unreachable() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish payroll\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let db = db_path_for(&v, &p).unwrap();
        // Another vault's index of the same folder stays.
        let other_vault = t.path().join("Other");
        fs::create_dir(&other_vault).unwrap();
        let other = db_path_for(&other_vault, &p).unwrap();
        fs::copy(&db, &other).unwrap();
        // An orphan -wal of this vault (its .db gone) goes too.
        let mut orphan = db_path_for(&v, &t.path().join("Gone"))
            .unwrap()
            .into_os_string();
        orphan.push("-wal");
        let orphan = PathBuf::from(orphan);
        fs::write(&orphan, "zebrafish payroll").unwrap();
        // Unmounted: the folder is gone from its path.
        fs::rename(&p, t.path().join("Elsewhere")).unwrap();
        assert!(dc::connected_root(&v).is_none());
        // A pass meanwhile keeps the index for when it is back.
        reconcile(&v, &BUDGET).unwrap();
        assert!(db.is_file());
        set_enabled(&v, false).unwrap();
        assert!(!db.exists(), "off left the index of an unreachable folder");
        assert!(!orphan.exists(), "off left an orphan -wal");
        assert!(
            other.is_file(),
            "another vault's index is not this switch's"
        );
    }

    /// Review blocker: connecting another Projects folder deletes this
    /// vault's index of the old one: `connect` itself when no pass is running
    /// (another test's pass may hold the lock, so this checks after the next
    /// pass; `connecting_another_folder_never_waits_on_an_indexing_pass`
    /// covers the deferral).
    #[test]
    fn connecting_another_projects_folder_deletes_the_old_index() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish payroll\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let old = db_path_for(&v, &p).unwrap();
        assert!(old.is_file());
        let next = t.path().join("Projects2");
        fs::create_dir(&next).unwrap();
        put(&next, "B/notes.md", "other\n");
        connect(&t, &next, &v);
        reconcile(&v, &BUDGET).unwrap();
        assert!(!old.exists(), "the old folder's index stayed");
        assert!(db_path_for(&v, &next).unwrap().is_file());
        assert_eq!(vault_dbs(&v).len(), 1);
    }

    /// Delta review: a retry whose parse returned but whose write failed is
    /// not a second strike; the file is read again as new.
    #[test]
    fn a_failed_write_after_a_retry_is_not_a_strike() {
        use crate::doc_text::tests::docx_with;
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        fs::write(p.join("A/deck.docx"), docx_with(&["zebrafish minutes"])).unwrap();
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let inv = dc::load_inventory(&v);
        let deck = inv
            .files
            .iter()
            .find(|f| f.relative == "A/deck.docx")
            .unwrap();
        put_row(
            &open_rw(&db_path_for(&v, &p).unwrap()).unwrap(),
            deck,
            Some(PARSING),
        )
        .unwrap();
        FAIL_WRITE.with(|c| c.set(true));
        let failed = reconcile(&v, &BUDGET);
        FAIL_WRITE.with(|c| c.set(false));
        assert!(failed.is_err());
        let next = reconcile(&v, &BUDGET).unwrap();
        assert_eq!((next.crashed, next.retried, next.read), (0, 0, 1));
        assert_eq!(status(&v).indexed, 1);
    }

    /// Delta review: a few dropped rows get a bounded merge step; a dropped
    /// row whose file moved into a restricted folder, or many dropped rows,
    /// rewrite the whole index.
    #[test]
    fn a_few_dropped_rows_merge_a_step_and_privacy_or_many_rewrite_the_index() {
        let (t, p, v) = fixture();
        for i in 0..30 {
            put(&p, &format!("A/n-{i:02}.md"), &format!("note {i}\n"));
        }
        put(&p, "A/hr.md", "salary review\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        fs::remove_file(p.join("A/n-00.md")).unwrap();
        dc::refresh(&v).unwrap();
        let few = reconcile(&v, &BUDGET).unwrap();
        assert_eq!(
            (few.removed, few.optimized),
            (1, false),
            "1 of 31: a merge step"
        );
        fs::create_dir(p.join("A/restricted")).unwrap();
        fs::rename(p.join("A/hr.md"), p.join("A/restricted/hr.md")).unwrap();
        dc::refresh(&v).unwrap();
        let moved = reconcile(&v, &BUDGET).unwrap();
        assert_eq!(
            (moved.removed, moved.optimized),
            (1, true),
            "moved into restricted/"
        );
        for i in 1..6 {
            fs::remove_file(p.join(format!("A/n-{i:02}.md"))).unwrap();
        }
        dc::refresh(&v).unwrap();
        let many = reconcile(&v, &BUDGET).unwrap();
        assert_eq!((many.removed, many.optimized), (5, true), "5 of 29");
    }

    /// Delta review: Office's `~$` lock files are neither listed nor indexed.
    #[test]
    fn office_lock_files_are_not_indexed() {
        use crate::doc_text::tests::docx_with;
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        fs::write(p.join("A/plan.docx"), docx_with(&["zebrafish plan"])).unwrap();
        fs::write(p.join("A/~$plan.docx"), "\u{8}dev owner").unwrap();
        put(&p, "A/~$notes.md", "lock\n");
        connect(&t, &p, &v);
        let inv = dc::load_inventory(&v);
        let listed: Vec<&str> = inv.files.iter().map(|f| f.relative.as_str()).collect();
        assert_eq!(listed, ["A/plan.docx"]);
        reads();
        reconcile(&v, &BUDGET).unwrap();
        assert_eq!(reads(), 1);
        assert_eq!(status(&v).total, 1);
    }

    /// Delta review: `connect` holds the connection's lock, so it must never
    /// wait on a running indexing pass; the old folder's index is then the
    /// next pass's to delete.
    #[test]
    fn connecting_another_folder_never_waits_on_an_indexing_pass() {
        let (t, p, v) = fixture();
        put(&p, "A/plan.md", "zebrafish payroll\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let old = db_path_for(&v, &p).unwrap();
        let next = t.path().join("Projects2");
        fs::create_dir(&next).unwrap();
        put(&next, "B/notes.md", "other\n");
        let pass = WRITE.lock().unwrap_or_else(|e| e.into_inner());
        let (done, connected) = std::sync::mpsc::channel();
        let (n, vv, config) = (next.clone(), v.clone(), t.path().join("config.json"));
        let worker = std::thread::spawn(move || {
            dc::connect(n.to_str().unwrap(), vv.to_str().unwrap(), &config).unwrap();
            let _ = done.send(());
        });
        // A no-hang bound: connect takes milliseconds; blocked, it never ends.
        let finished = connected.recv_timeout(std::time::Duration::from_secs(60));
        drop(pass);
        worker.join().unwrap();
        assert!(finished.is_ok(), "connect waited on the indexing pass");
        assert!(old.is_file(), "left for the next pass");
        reconcile(&v, &BUDGET).unwrap();
        assert!(
            !old.exists(),
            "the next pass deletes the old folder's index"
        );
    }

    /// Should-fix: a dropped file's words don't stay readable in the file
    /// (secure_delete and `optimize`).
    #[test]
    fn a_dropped_files_words_do_not_linger_on_disk() {
        let (t, p, v) = fixture();
        put(&p, "A/keep.md", "ordinary words here\n");
        put(&p, "A/gone.md", "qqxsalaryfortyone\n");
        connect(&t, &p, &v);
        reconcile(&v, &BUDGET).unwrap();
        let db = db_path_for(&v, &p).unwrap();
        let on_disk = || {
            let mut bytes = fs::read(&db).unwrap();
            let mut wal = db.as_os_str().to_owned();
            wal.push("-wal");
            bytes.extend(fs::read(PathBuf::from(wal)).unwrap_or_default());
            bytes.windows(b"qqxsalary".len()).any(|w| w == b"qqxsalary")
        };
        assert!(on_disk(), "the word is indexed to begin with");
        fs::remove_file(p.join("A/gone.md")).unwrap();
        dc::refresh(&v).unwrap();
        assert_eq!(reconcile(&v, &BUDGET).unwrap().removed, 1);
        assert!(
            !on_disk(),
            "a dropped file's word is still in the index file"
        );
    }

    /// Retrieval and the index share a document's two strikes (`doc_text`'s
    /// crash marker): a file that crashed retrieval's reader twice is never
    /// parsed by the index, nor after the index is turned off and on (which
    /// drops its rows), nor by retrieval with the index off (RF-795-F1).
    #[test]
    fn one_bad_document_is_parsed_at_most_twice_across_retrieval_and_the_index() {
        use crate::doc_text::tests::docx_with;
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        let path = p.join("A/deck.docx");
        fs::write(&path, docx_with(&["zebrafish minutes"])).unwrap();
        put(&p, "A/plan.md", "plan\n");
        // Two crashes in retrieval's reader (a turn, or the vault subprocess).
        crate::doc_text::tests::crashed_in_app_cache(&path, 2);
        connect(&t, &p, &v);
        let deck = || {
            dc::load_inventory(&v)
                .files
                .iter()
                .find(|f| f.relative == "A/deck.docx")
                .unwrap()
                .clone()
        };
        for _ in 0..2 {
            reconcile(&v, &BUDGET).unwrap();
            assert_eq!(
                crate::doc_text::seam::extracts(&path),
                0,
                "never parsed again"
            );
            let snapshot = Snapshot::open(&v, &p).unwrap();
            let why = snapshot.held(&deck()).unwrap_or_default();
            assert!(why.contains("stopped twice"), "{why}");
            drop(snapshot);
            let s = status(&v);
            assert_eq!((s.indexed, s.failed), (1, 1));
            set_enabled(&v, false).unwrap();
            set_enabled(&v, true).unwrap();
        }
        set_enabled(&v, false).unwrap();
        assert_eq!(
            crate::doc_text::cached(&path).unwrap_err(),
            crate::doc_text::CRASHED_READER
        );
        assert_eq!(crate::doc_text::seam::extracts(&path), 0);
    }

    /// A parse that never returns wedges nothing and piles nothing up:
    /// turning the index off returns and deletes it while that parse still
    /// runs; a pass on gives up waiting at its deadline; and a later pass
    /// doesn't parse a file whose parse is still running (RF-795-F2).
    #[test]
    fn turning_off_does_not_wait_on_a_parse_that_never_returns() {
        use crate::doc_text::tests::docx_with;
        use std::sync::mpsc;
        let long = Duration::from_secs(60);
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        let deck = p.join("A/deck.docx");
        fs::write(&deck, docx_with(&["zebrafish minutes"])).unwrap();
        connect(&t, &p, &v);
        // `extract` of `path` blocks until its sender is dropped.
        let hang = |path: &Path| {
            let (started_tx, started) = mpsc::channel();
            let (release, released) = mpsc::channel::<()>();
            crate::doc_text::seam::on_extract(path, move || {
                let _ = started_tx.send(());
                let _ = released.recv();
            });
            (started, release)
        };
        let (started, _release_deck) = hang(&deck);
        let (pass_tx, pass) = mpsc::channel();
        let vv = v.clone();
        std::thread::spawn(move || {
            let _ = pass_tx.send(reconcile(&vv, &BUDGET));
        });
        started.recv_timeout(long).expect("the parse began");
        let (off_tx, off) = mpsc::channel();
        let vv = v.clone();
        std::thread::spawn(move || {
            let _ = off_tx.send(set_enabled(&vv, false));
        });
        let turned_off = off.recv_timeout(long).expect("turning off returned");
        assert!(!turned_off.unwrap().enabled);
        assert!(
            !db_path_for(&v, &p).unwrap().exists(),
            "the index is deleted"
        );
        let left = pass.recv_timeout(long).expect("the pass ended");
        assert_eq!(left.unwrap().read, 0);
        // On again, with a second document that hangs too and a short wait.
        set_enabled(&v, true).unwrap();
        let slow = p.join("A/slow.docx");
        fs::write(&slow, docx_with(&["walrus agenda"])).unwrap();
        dc::refresh(&v).unwrap();
        let (started_slow, _release_slow) = hang(&slow);
        PARSE_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(300))));
        let first = reconcile(&v, &BUDGET).unwrap();
        started_slow
            .recv_timeout(long)
            .expect("the second parse began");
        assert_eq!(first.read, 0, "neither parse returned");
        let again = reconcile(&v, &BUDGET).unwrap();
        PARSE_WAIT_FOR_TEST.with(|w| w.set(None));
        assert_eq!(again.read, 0);
        assert_eq!(
            (
                crate::doc_text::seam::extracts(&deck),
                crate::doc_text::seam::extracts(&slow)
            ),
            (1, 1),
            "a running parse is never started again"
        );
        let inv = dc::load_inventory(&v);
        let slow_row = inv
            .files
            .iter()
            .find(|f| f.relative == "A/slow.docx")
            .unwrap();
        let snapshot = Snapshot::open(&v, &p).unwrap();
        assert!(snapshot.held(slow_row).unwrap().contains("interrupted"));
    }

    /// A slow but healthy parse that outlasts a pass's wait is no strike:
    /// when it returns, the next pass stores what it read, without parsing
    /// the file again or calling it a retry (RF-R1 review F1).
    #[test]
    fn a_parse_that_outlasts_its_wait_is_stored_when_it_returns() {
        use crate::doc_text::tests::docx_with;
        use std::sync::mpsc;
        let long = Duration::from_secs(60);
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        let deck = p.join("A/deck.docx");
        fs::write(&deck, docx_with(&["zebrafish minutes"])).unwrap();
        connect(&t, &p, &v);
        let (started_tx, started) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        crate::doc_text::seam::on_extract(&deck, move || {
            let _ = started_tx.send(());
            let _ = released.recv();
        });
        PARSE_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(200))));
        assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 0, "left running");
        started.recv_timeout(long).expect("the parse began");
        drop(release);
        // Wait (bounded) for the left parse to return.
        let by = Instant::now() + long;
        while left_running(&deck) && Instant::now() < by {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!left_running(&deck), "the parse returned");
        let next = reconcile(&v, &BUDGET).unwrap();
        assert_eq!(
            (next.read, next.retried, next.crashed),
            (1, 0, 0),
            "stored, not retried"
        );
        assert_eq!(crate::doc_text::seam::extracts(&deck), 1, "parsed once");
        assert_eq!(reconcile(&v, &BUDGET).unwrap(), Reconciled::default());
        let s = status(&v);
        assert_eq!((s.indexed, s.failed), (1, 0));
        let found = ranked(&v, &words("zebrafish"), None, 5).unwrap();
        assert_eq!(names(&found), ["deck.docx"]);
    }

    /// Off keeps no index, in memory either: a left parse that returns
    /// while the index is off keeps nothing, and turning it off drops what
    /// one kept while it was on (RF-R1 delta review D1).
    #[test]
    fn turning_off_drops_what_a_left_parse_read() {
        use crate::doc_text::tests::docx_with;
        use std::sync::mpsc;
        let long = Duration::from_secs(60);
        let (t, p, v) = fixture();
        fs::create_dir_all(p.join("A")).unwrap();
        let deck = p.join("A/deck.docx");
        fs::write(&deck, docx_with(&["zebrafish minutes"])).unwrap();
        connect(&t, &p, &v);
        PARSE_WAIT_FOR_TEST.with(|w| w.set(Some(Duration::from_millis(200))));
        // A pass leaves the parse running, then the parse returns.
        let leave = |off_first: bool| {
            let (started_tx, started) = mpsc::channel();
            let (release, released) = mpsc::channel::<()>();
            crate::doc_text::seam::on_extract(&deck, move || {
                let _ = started_tx.send(());
                let _ = released.recv();
            });
            assert_eq!(reconcile(&v, &BUDGET).unwrap().read, 0, "left running");
            started.recv_timeout(long).expect("the parse began");
            if off_first {
                set_enabled(&v, false).unwrap();
            }
            drop(release);
            let by = Instant::now() + long;
            while left_running(&deck) && Instant::now() < by {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!left_running(&deck), "the parse returned");
        };
        leave(true);
        assert!(!kept(&deck), "returned while off: kept nothing");
        set_enabled(&v, true).unwrap();
        leave(false);
        assert!(kept(&deck), "returned while on: kept for the next pass");
        set_enabled(&v, false).unwrap();
        assert!(!kept(&deck), "turning off drops it");
    }
}
