//! Move Hollow home: the planner and dry-run probes (read-only).
//!
//! [`plan_with`] decides every move, re-key and rewrite from config and
//! `$HOME` and measures what it can within a time budget: sizes, file
//! counts, iCloud-evicted (dataless) files, git repositories and their
//! linked worktrees, collisions, running chats, processes, launchd jobs,
//! scripts, agent stores and Time Machine. It never writes, never follows a
//! symlink while walking and never reads a file whose `SF_DATALESS` flag is
//! set (that would download it).

use crate::move_home::{
    AgentStoreChange, Blocker, Collision, ColumnRewrite, DatalessReport, DatalessRoot, FileRewrite,
    IdChange, LaunchItem, MoveKind, Options, Plan, PlannedMove, Probes, RepoRepair, Roots,
    StoreRekey, METHOD_COPY, METHOD_RENAME, PLAN_VERSION, SF_DATALESS,
};
use crate::move_home_rekey as rekey;
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

// `what` values the run dispatches on.
pub const WHAT_SESSIONS: &str = "Sessions database";
pub const WHAT_STORE: &str = "vault store";
pub const WHAT_CLAUDE_DIR: &str = "Claude project folder";
pub const WHAT_CLAUDE_MERGE: &str = "Claude project entry (merged)";
pub const WHAT_CLAUDE_MERGE_RENAMED: &str = "Claude project entry (merged, renamed)";
pub const FILE_CONFIG: &str = "Hollow config";
pub const FILE_STORE: &str = "Hollow store";
pub const FILE_NOTE: &str = "vault note";
pub const FILE_CLAUDE_JSON: &str = "Claude settings";
pub const FILE_CODEX: &str = "Codex config";
pub const FILE_GEMINI: &str = "Gemini projects";
pub const FILE_CLAUDE_DESKTOP: &str = "Claude desktop config";
pub const FILE_OBSIDIAN: &str = "Obsidian vaults";
pub const FILE_LAUNCH_AGENT: &str = "LaunchAgent";
pub const FILE_SCRIPT: &str = "script";

/// Sessions DB columns that hold paths (missing ones are skipped), and
/// whether each holds a path as its whole value (matched whole) or text that
/// mentions paths (JSON text, prose: matched as free text).
pub const PATH_COLUMNS: &[(&str, &str, ColKind)] = &[
    ("sessions", "cwd", ColKind::Path),
    ("external_sessions", "cwd", ColKind::Path),
    ("external_sessions", "path", ColKind::Path),
    ("external_sessions", "transcript_path", ColKind::Path),
    ("external_export_files", "path", ColKind::Path),
    ("interactions", "target_path", ColKind::Path),
    ("interactions", "target_abs", ColKind::Path),
    ("interactions", "request_json", ColKind::Text),
    ("runbook_runs", "copy_path", ColKind::Path),
    ("work_origin_allocation_pins", "source_root", ColKind::Path),
    ("run_queue", "payload_json", ColKind::Text),
    ("run_queue", "detail", ColKind::Text),
    ("derived_state", "payload", ColKind::Text),
];

/// How a Sessions DB column is rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColKind {
    /// The whole value is a path.
    Path,
    /// Text that mentions paths.
    Text,
    /// A workspace id, exact match.
    Id,
}

/// Columns holding a workspace id (exact-match rewrite).
pub fn is_id_column(name: &str) -> bool {
    matches!(name, "workspace" | "workspace_id" | "workspaceId")
}

const DEFAULT_BUDGET: u64 = 120;
/// JSON stores larger than this are not rewritten (reported).
const MAX_STORE_BYTES: u64 = 64 * 1024 * 1024;

// ── Small helpers ─────────────────────────────────────────────────────────

pub(crate) fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// `path` canonicalized; when it doesn't exist, its nearest existing
/// ancestor canonicalized with the rest appended.
pub(crate) fn canonical_lenient(path: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(path) {
        return c;
    }
    let mut rest = Vec::new();
    let mut cur = path;
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            rest.push(name.to_os_string());
        }
        if let Ok(c) = std::fs::canonicalize(parent) {
            let mut out = c;
            for n in rest.iter().rev() {
                out.push(n);
            }
            return out;
        }
        cur = parent;
    }
    path.to_path_buf()
}

/// The nearest existing ancestor of `path` (itself included).
pub(crate) fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| std::fs::symlink_metadata(p).is_ok())
        .map(Path::to_path_buf)
}

#[cfg(unix)]
pub(crate) fn dev_of(path: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let p = existing_ancestor(path)?;
    std::fs::metadata(&p).ok().map(|m| m.dev())
}

#[cfg(not(unix))]
pub(crate) fn dev_of(_path: &Path) -> Option<u64> {
    Some(0)
}

pub(crate) fn free_bytes(path: &Path) -> u64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let Some(p) = existing_ancestor(path) else {
            return 0;
        };
        let Ok(c) = CString::new(p.as_os_str().as_bytes()) else {
            return 0;
        };
        // SAFETY: a zeroed statvfs is valid and statvfs only writes into it.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: valid NUL-terminated path and a valid out pointer.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return 0;
        }
        #[allow(clippy::unnecessary_cast)]
        let free = (st.f_bavail as u64).saturating_mul(st.f_frsize as u64);
        free
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        0
    }
}

pub(crate) fn size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let b = bytes as f64;
    if b >= K * K * K {
        format!("{:.1} GB", b / (K * K * K))
    } else if b >= K * K {
        format!("{:.1} MB", b / (K * K))
    } else if b >= K {
        format!("{:.0} KB", b / K)
    } else {
        format!("{bytes} B")
    }
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

/// Read a text file unless it is a symlink or iCloud-evicted (reading an
/// evicted file downloads it). `Ok(None)`: skipped for that reason.
pub(crate) fn read_checked(path: &Path, probes: &dyn Probes) -> Result<Option<String>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    if !meta.file_type().is_file() || probes.flags(path, &meta) & SF_DATALESS != 0 {
        return Ok(None);
    }
    std::fs::read_to_string(path)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `dir` is a Hollow vault: `workspaces/` plus `AGENTS.md` or a
/// `.hollow*` entry.
pub(crate) fn is_hollow_vault(dir: &Path) -> bool {
    if !dir.join("workspaces").is_dir() {
        return false;
    }
    if std::fs::symlink_metadata(dir.join("AGENTS.md")).is_ok() {
        return true;
    }
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with(".hollow"))
}

/// A file name that may hold a secret: never read.
pub(crate) fn secret_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with(".env")
        || n.contains("token")
        || n.contains("secret")
        || n.ends_with(".key")
        || n.contains("password")
}

/// The Sessions database of `stem`.
pub(crate) fn sessions_db(roots: &Roots, stem: &str) -> PathBuf {
    roots.support.join("Sessions").join(format!("{stem}.db"))
}

pub(crate) fn sidecar(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// A read-only connection that never blocks the app: a database with a
/// `-wal` gets a normal read-only connection (500 ms busy timeout); one
/// without is opened immutable, so no `-wal`/`-shm` appears beside it.
pub(crate) fn open_read_only(db: &Path) -> Result<Connection, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let mut uri = String::from("file:");
    for c in db.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            ' ' => uri.push_str("%20"),
            c => uri.push(c),
        }
    }
    let open = |params: &str| -> Result<Connection, String> {
        let conn = Connection::open_with_flags(format!("{uri}?{params}"), flags)
            .map_err(|e| format!("couldn't open {} read-only: {e}", db.display()))?;
        conn.busy_timeout(Duration::from_millis(500))
            .map_err(|e| e.to_string())?;
        // Opening is lazy: read once so a connection that can't read fails here.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
            .map_err(|e| format!("couldn't read {} read-only: {e}", db.display()))?;
        Ok(conn)
    };
    if !sidecar(db, "-wal").exists() {
        return open("immutable=1");
    }
    if sidecar(db, "-shm").exists() {
        // A live writer's index: join it as an ordinary reader (nothing is
        // created; the `-shm` is there).
        return open("mode=ro");
    }
    // A `-wal` with no `-shm`: nothing has it open, and a reader here would
    // create a `-shm` beside it. Read a private clone instead (free on APFS):
    // checkpoint the clone, open it immutable, then remove it (the open file
    // stays readable).
    let tmp = tempfile::Builder::new()
        .prefix("hollow-move-home-read-")
        .tempdir()
        .map_err(|e| format!("couldn't make a scratch folder: {e}"))?;
    let copy = tmp.path().join("copy.db");
    for suffix in ["", "-wal"] {
        crate::move_home_run::clone_or_copy(&sidecar(db, suffix), &sidecar(&copy, suffix))?;
    }
    {
        let c = Connection::open(&copy)
            .map_err(|e| format!("couldn't read a copy of {}: {e}", db.display()))?;
        c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .map_err(|e| format!("couldn't read a copy of {}: {e}", db.display()))?;
    }
    open_read_only(&copy)
}

/// The columns of `table` (empty when it doesn't exist).
pub(crate) fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!(
        "SELECT name FROM pragma_table_info('{}')",
        table.replace('\'', "''")
    );
    conn.prepare(&sql)
        .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(0))?.collect())
        .unwrap_or_default()
}

pub(crate) fn tables(conn: &Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .and_then(|mut st| st.query_map([], |r| r.get::<_, String>(0))?.collect())
        .unwrap_or_default()
}

/// Tables a trigger keeps append-only (a `BEFORE UPDATE` trigger that
/// raises): `audit_log` and `workspace_aliases` today, and any later one.
/// The re-key never writes them; their workspace ids are read through
/// `workspace_aliases` instead (run 2 of 11-13 failed on `audit_log`).
pub(crate) fn append_only_tables(conn: &Connection) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = conn
        .prepare("SELECT tbl_name, COALESCE(sql, '') FROM sqlite_master WHERE type = 'trigger'")
        .and_then(|mut st| {
            st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, sql)| {
            let sql = sql.to_ascii_uppercase();
            sql.contains("BEFORE UPDATE") && sql.contains("RAISE(ABORT")
        })
        .map(|(t, _)| t)
        .collect();
    out.insert("audit_log".into());
    out.insert("workspace_aliases".into());
    out
}

/// Workspace id columns in append-only tables: (table, column). Their rows
/// keep the ids they were written with.
pub(crate) fn kept_id_columns(conn: &Connection) -> Vec<(String, String)> {
    let keep = append_only_tables(conn);
    let mut out = Vec::new();
    for table in tables(conn) {
        if !keep.contains(&table) || table == "workspace_aliases" {
            continue;
        }
        for column in table_columns(conn, &table) {
            if is_id_column(&column) {
                out.push((table.clone(), column));
            }
        }
    }
    out
}

/// Every (table, column, kind) the re-key touches in this database, and the
/// notes for the ones missing. Append-only tables are left out
/// ([`append_only_tables`]).
pub(crate) fn rewrite_columns(conn: &Connection) -> (Vec<(String, String, ColKind)>, Vec<String>) {
    let keep = append_only_tables(conn);
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for (table, column, kind) in PATH_COLUMNS {
        if keep.contains(*table) {
            continue;
        }
        if table_columns(conn, table).iter().any(|c| c == column) {
            out.push((table.to_string(), column.to_string(), *kind));
        } else {
            missing.push(format!("{table}.{column}"));
        }
    }
    for table in tables(conn) {
        if keep.contains(&table) {
            continue;
        }
        for column in table_columns(conn, &table) {
            if is_id_column(&column) && !out.iter().any(|(t, c, _)| *t == table && *c == column) {
                out.push((table.clone(), column, ColKind::Id));
            }
        }
    }
    (out, missing)
}

/// One value's new text: a path column's whole value then its ids, free
/// text then ids for a text column, the exact id for an id column.
pub(crate) fn rewrite_value(
    text: &str,
    kind: ColKind,
    mappings: &[(String, String)],
    ids: &BTreeMap<String, String>,
) -> Option<String> {
    match kind {
        ColKind::Id => ids.get(text).cloned(),
        ColKind::Path => {
            let path = rekey::rewrite_whole(text, mappings);
            let base = path.as_deref().unwrap_or(text);
            let (new, n) = rekey::rewrite_ids(base, ids);
            (path.is_some() || n > 0).then_some(new)
        }
        ColKind::Text => {
            let (new, n) = rekey::rewrite_text(text, mappings, ids);
            (n > 0).then_some(new)
        }
    }
}

/// Rows each column would change.
fn count_column(
    conn: &Connection,
    table: &str,
    column: &str,
    kind: ColKind,
    mappings: &[(String, String)],
    ids: &BTreeMap<String, String>,
) -> Result<u64, String> {
    let sql = format!(
        "SELECT \"{c}\" FROM \"{t}\" WHERE typeof(\"{c}\") = 'text'",
        c = column.replace('"', "\"\""),
        t = table.replace('"', "\"\"")
    );
    let mut st = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let mut rows = st.query([]).map_err(|e| e.to_string())?;
    let mut n = 0;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let v: String = row.get(0).map_err(|e| e.to_string())?;
        if rewrite_value(&v, kind, mappings, ids).is_some() {
            n += 1;
        }
    }
    Ok(n)
}

/// Live run-queue rows (queued, claimed, running) other than the caller's
/// own chat: (label, status).
pub(crate) fn live_queue(db: &Path, caller: Option<&str>) -> Result<Vec<(String, String)>, String> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_read_only(db)?;
    if table_columns(&conn, "run_queue").is_empty() {
        return Ok(Vec::new());
    }
    let mut st = conn
        .prepare(
            "SELECT source, COALESCE(source_id, ''), label, status FROM run_queue \
             WHERE status IN ('queued', 'claimed', 'running') ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<(String, String, String, String)> = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .and_then(|it| it.collect())
        .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .filter(|(source, id, _, _)| !(source == "chat" && Some(id.as_str()) == caller))
        .map(|(_, _, label, status)| (label, status))
        .collect())
}

// ── Walk ──────────────────────────────────────────────────────────────────

/// What one bounded lstat walk of a folder found.
#[derive(Debug, Default, Clone)]
pub(crate) struct Walk {
    pub bytes: u64,
    pub files: u64,
    pub dataless: u64,
    /// Size of the evicted files (their size once downloaded).
    pub dataless_bytes: u64,
    /// Evicted folders among `dataless` (their contents are unknown until
    /// they are listed, which downloads them).
    pub dataless_dirs: u64,
    pub dataless_top: BTreeMap<String, u64>,
    pub dataless_sample: Vec<String>,
    pub newest: Option<SystemTime>,
    /// Folders holding a `.git` folder (main repositories).
    pub mains: Vec<PathBuf>,
    /// Folders holding a `.git` file (linked worktrees or submodules).
    pub linked: Vec<PathBuf>,
    /// Relative symlinks whose target leaves the folder.
    pub escaping_links: Vec<String>,
    pub nosync: Vec<String>,
    /// Known throwaway special files (git's fsmonitor socket): left out of
    /// a copy and its comparison.
    pub special_throwaway: Vec<String>,
    /// Any other socket, pipe or device file: "path (kind)". A copy stops.
    pub special_unexpected: Vec<String>,
    pub partial: bool,
}

impl Walk {
    fn note_dataless(&mut self, root: &Path, path: &Path) {
        self.dataless += 1;
        self.dataless_dirs += 1;
        let top = path
            .strip_prefix(root)
            .ok()
            .and_then(|r| r.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_default();
        *self.dataless_top.entry(top).or_default() += 1;
        if self.dataless_sample.len() < 10 {
            self.dataless_sample.push(s(path));
        }
    }
}

/// Walk `root` without following symlinks until `deadline`.
pub(crate) fn walk(root: &Path, probes: &dyn Probes, deadline: Instant) -> Walk {
    let mut w = Walk::default();
    let Ok(meta) = std::fs::symlink_metadata(root) else {
        return w;
    };
    if !meta.is_dir() {
        if meta.file_type().is_file() {
            w.files = 1;
            w.bytes = meta.len();
        }
        return w;
    }
    if probes.flags(root, &meta) & SF_DATALESS != 0 {
        w.note_dataless(root, root);
        return w;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut seen = 0u64;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            seen += 1;
            if seen.is_multiple_of(512) && Instant::now() > deadline {
                w.partial = true;
                return w;
            }
            let path = entry.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Ok(t) = m.modified() {
                if w.newest.is_none_or(|n| t > n) {
                    w.newest = Some(t);
                }
            }
            let ft = m.file_type();
            if name == ".git" {
                if ft.is_dir() {
                    w.mains.push(dir.clone());
                } else if ft.is_file() {
                    w.linked.push(dir.clone());
                }
            }
            if name.ends_with(".nosync") && w.nosync.len() < 20 {
                w.nosync.push(s(&path));
            }
            if ft.is_symlink() {
                if let Ok(target) = std::fs::read_link(&path) {
                    if target.is_relative()
                        && !inside(&dir.join(&target), root)
                        && w.escaping_links.len() < 20
                    {
                        w.escaping_links
                            .push(format!("{} -> {}", s(&path), s(&target)));
                    }
                }
                continue;
            }
            if ft.is_dir() {
                // An evicted folder: listing it would download it. Counted
                // as evicted, never opened.
                if probes.flags(&path, &m) & SF_DATALESS != 0 {
                    w.note_dataless(root, &path);
                    continue;
                }
                stack.push(path);
                continue;
            }
            if let Some(kind) = crate::move_home_icloud::special_kind(&ft) {
                if crate::move_home_icloud::is_throwaway_special(&entry.file_name()) {
                    w.special_throwaway.push(s(&path));
                } else {
                    w.special_unexpected.push(format!("{} ({kind})", s(&path)));
                }
                continue;
            }
            if ft.is_file() {
                w.files += 1;
                w.bytes += m.len();
                if probes.flags(&path, &m) & SF_DATALESS != 0 {
                    w.dataless += 1;
                    w.dataless_bytes += m.len();
                    let top = path
                        .strip_prefix(root)
                        .ok()
                        .and_then(|r| r.components().next())
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .unwrap_or_default();
                    *w.dataless_top.entry(top).or_default() += 1;
                    if w.dataless_sample.len() < 10 {
                        w.dataless_sample.push(s(&path));
                    }
                }
            }
        }
    }
    w
}

/// Lexically normalize `p` (`..` and `.`) and check it stays under `root`.
fn inside(p: &Path, root: &Path) -> bool {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.starts_with(root)
}

// ── Locating paths across earlier moves ───────────────────────────────────

/// Each move's `from` where it is when that move runs.
pub(crate) fn current_froms(moves: &[PlannedMove]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(moves.len());
    for (j, m) in moves.iter().enumerate() {
        let f = apply_moves(&m.from, moves, &out, j);
        out.push(f);
    }
    out
}

fn apply_moves(p: &str, moves: &[PlannedMove], froms: &[String], upto: usize) -> String {
    let mut cur = p.to_string();
    for j in 0..upto.min(froms.len()) {
        if cur == froms[j] {
            cur = moves[j].to.clone();
        } else if let Some(rest) = cur.strip_prefix(&format!("{}/", froms[j])) {
            cur = format!("{}/{rest}", moves[j].to);
        }
    }
    cur
}

// ── Planning ──────────────────────────────────────────────────────────────

struct Ctx<'a> {
    roots: &'a Roots,
    options: &'a Options,
    probes: &'a dyn Probes,
    deadline: Instant,
    home: PathBuf,
    hollow: PathBuf,
    vault: PathBuf,
    projects: Option<PathBuf>,
    plan: Plan,
    /// Lower-cased names already taken in Archive (on disk and planned).
    archive_taken: BTreeSet<String>,
    walks: Vec<(usize, Walk)>,
}

fn lower(name: &str) -> String {
    name.to_lowercase()
}

fn child_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    out.sort();
    out
}

fn label_of(root: &Path) -> String {
    let name = root
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "other".into());
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

impl Ctx<'_> {
    fn blocker(&mut self, kind: &str, detail: String, blocking: bool) {
        self.plan.blockers.push(Blocker {
            kind: kind.into(),
            detail,
            blocking,
        });
    }

    fn collision(
        &mut self,
        kind: &str,
        path: &Path,
        detail: String,
        winner: Option<String>,
        blocking: bool,
    ) {
        self.plan.collisions.push(Collision {
            kind: kind.into(),
            path: s(path),
            detail: detail.clone(),
            winner,
        });
        if blocking {
            self.blocker("collision", format!("{}: {detail}", path.display()), true);
        }
    }

    /// A free Archive name for `name` from `root`.
    fn archive_target(&mut self, name: &str, root: &Path) -> PathBuf {
        let archive = self.hollow.join("Archive");
        let mut candidate = name.to_string();
        if self.archive_taken.contains(&lower(&candidate)) {
            candidate = format!("{name}-{}", label_of(root));
            let base = candidate.clone();
            let mut n = 2;
            while self.archive_taken.contains(&lower(&candidate)) {
                candidate = format!("{base}-{n}");
                n += 1;
            }
        }
        self.archive_taken.insert(lower(&candidate));
        archive.join(candidate)
    }

    /// Check a move's target: a symlink is replaced (recorded), an empty
    /// folder is replaced, anything else blocks.
    fn target_state(&mut self, to: &Path, from: &Path) -> Option<String> {
        match std::fs::symlink_metadata(to) {
            Err(_) => None,
            Ok(m) if m.file_type().is_symlink() => {
                let target = std::fs::read_link(to).map(|t| s(&t)).unwrap_or_default();
                let resolves = std::fs::canonicalize(to).ok();
                if resolves.as_deref() != Some(from) {
                    self.plan.notes.push(format!(
                        "{} is a link to {target}, not to {}; the link is replaced (and put back on rollback).",
                        to.display(),
                        from.display()
                    ));
                }
                Some(target)
            }
            Ok(m) if m.is_dir() && std::fs::read_dir(to).is_ok_and(|mut d| d.next().is_none()) => {
                self.plan.notes.push(format!(
                    "{} is an empty folder; it is replaced (and made again on rollback).",
                    to.display()
                ));
                None
            }
            Ok(_) => {
                self.collision(
                    "targetExists",
                    to,
                    format!("a different folder is already at {}", to.display()),
                    None,
                    true,
                );
                None
            }
        }
    }

    fn add_move(
        &mut self,
        from: &Path,
        to: &Path,
        kind: MoveKind,
        compat: bool,
        replaces: Option<String>,
        why: String,
    ) {
        self.plan.moves.push(PlannedMove {
            from: s(from),
            to: s(to),
            kind,
            bytes: 0,
            files: 0,
            dataless: 0,
            dataless_bytes: 0,
            method: METHOD_RENAME.into(),
            replaces_symlink: replaces,
            compat_symlink: compat,
            why,
        });
    }
}

/// The dry run with explicit probes.
pub fn plan_with(
    roots: &Roots,
    options: &Options,
    caller: Option<&str>,
    probes: &dyn Probes,
) -> Result<Plan, String> {
    let budget = Duration::from_secs(options.budget_secs.unwrap_or(DEFAULT_BUDGET).max(1));
    let home = std::fs::canonicalize(&roots.home).map_err(|e| {
        format!(
            "Hollow can't resolve your home folder {}: {e}",
            roots.home.display()
        )
    })?;
    let vault = std::fs::canonicalize(&roots.vault).map_err(|e| {
        format!(
            "The vault folder {} can't be found ({e}); there is nothing to move.",
            roots.vault.display()
        )
    })?;
    let hollow = canonical_lenient(&roots.hollow_home_path());
    let projects = match &roots.projects {
        Some(p) => std::fs::canonicalize(p).ok(),
        None => None,
    };
    if let Some(p) = &projects {
        if *p == vault {
            return Err(
                "The vault and the Projects folder are the same folder; Hollow can't split them."
                    .into(),
            );
        }
        if p.starts_with(&vault) {
            return Err("The Projects folder is inside the vault; move it out first.".into());
        }
        if hollow.starts_with(p) {
            return Err("The Hollow folder is inside the Projects folder.".into());
        }
    }
    if hollow.starts_with(&vault) {
        return Err("The Hollow folder is inside the vault.".into());
    }
    let mut cx = Ctx {
        roots,
        options,
        probes,
        deadline: Instant::now() + budget,
        home,
        hollow: hollow.clone(),
        vault: vault.clone(),
        projects: projects.clone(),
        plan: Plan {
            version: PLAN_VERSION,
            created: now_rfc3339(),
            hollow_home: s(&hollow),
            vault_from: s(&vault),
            ..Default::default()
        },
        archive_taken: BTreeSet::new(),
        walks: Vec::new(),
    };
    if roots.projects.is_some() && projects.is_none() {
        cx.plan.notes.push(format!(
            "The Projects folder {} doesn't exist; only the vault is planned.",
            roots.projects.as_ref().map(|p| s(p)).unwrap_or_default()
        ));
    }
    let archive_dir = hollow.join("Archive");
    cx.archive_taken = child_names(&archive_dir);
    plan_moves(&mut cx);
    plan_mkdirs(&mut cx);
    measure_moves(&mut cx);
    let vault_to = hollow.join("Vault");
    cx.plan.vault_to = s(&vault_to);
    cx.plan.old_stem = rekey::stem_for(&vault);
    cx.plan.new_stem = rekey::stem_for(&vault_to);
    // Mappings from the moves (original from to final to), with the
    // configured (non-canonical) forms and the `<home>/Projects` link form.
    let mut maps: Vec<(String, String)> = cx
        .plan
        .moves
        .iter()
        .map(|m| (m.from.clone(), m.to.clone()))
        .collect();
    let move_maps_only = maps.clone();
    for m in &move_maps_only {
        if m.0 == s(&vault) && roots.vault != vault {
            maps.push((s(&roots.vault), m.1.clone()));
        }
        if let (Some(p), Some(pc)) = (&roots.projects, &projects) {
            if let Some(rest) = m.0.strip_prefix(&s(pc)) {
                if p != pc {
                    maps.push((format!("{}{rest}", s(p)), m.1.clone()));
                }
                let link = hollow.join("Projects");
                if s(&link) != s(pc) && !rest.is_empty() {
                    maps.push((format!("{}{rest}", s(&link)), m.1.clone()));
                }
            }
        }
    }
    let move_maps = rekey::sorted_mappings(&maps);
    plan_ids(&mut cx, &move_maps);
    let mut maps_ws = move_maps.clone();
    for id in &cx.plan.ids {
        if id.has_workspace {
            let new_vault = rekey::whole_or_same(&s(&vault), &move_maps);
            maps_ws.push((
                format!("{}/workspaces/{}", s(&vault), id.old),
                format!("{new_vault}/workspaces/{}", id.new),
            ));
        }
    }
    let maps_ws = rekey::sorted_mappings(&maps_ws);
    plan_stores(&mut cx);
    let mut all = maps_ws.clone();
    for st in &cx.plan.stores {
        all.push((st.from.clone(), st.to.clone()));
    }
    plan_claude_dirs(&mut cx, &maps_ws, &mut all);
    let all = rekey::sorted_mappings(&all);
    cx.plan.mappings = all.clone();
    let ids: BTreeMap<String, String> = cx
        .plan
        .ids
        .iter()
        .map(|i| (i.old.clone(), i.new.clone()))
        .collect();
    plan_files(&mut cx, &all, &ids);
    plan_columns(&mut cx, &all, &ids, caller);
    plan_launch_items(&mut cx, &all);
    plan_repos(&mut cx, &all);
    plan_checks(&mut cx);
    cx.plan.backup = probes.time_machine(&hollow);
    if cx.plan.backup.covered != Some(true) {
        let detail = cx.plan.backup.detail.clone();
        cx.blocker("backup", format!("Backup not confirmed: {detail}"), false);
    }
    let mut proc_roots = vec![vault.clone()];
    proc_roots.extend(projects.clone());
    proc_roots.extend(cx.plan.moves.iter().map(|m| PathBuf::from(&m.from)));
    proc_roots.sort();
    proc_roots.dedup();
    cx.plan.processes = probes.processes(&proc_roots);
    // Claude Code and Codex rewrite their own settings; the run refuses to
    // start while one runs outside Hollow (Hollow's own chats end at quit).
    for cli in probes.agent_clis(Some(std::process::id())) {
        cx.blocker(
            "agentCli",
            format!("{cli} is running outside Hollow and rewrites its own settings: the move won't start until it quits."),
            false,
        );
    }
    cx.plan
        .notes
        .push("Search and meaning-search indexes keep the old paths until Hollow reindexes; the links left at the old paths keep them resolving.".into());
    if !cx.plan.ids.is_empty() {
        cx.plan.notes.push("Workspace ids change for moved projects: each workspace folder is renamed and every id reference is rewritten; old ids stay in conversation history as text. The old-to-new list is saved in the backup folder.".into());
    }
    let mut plan = cx.plan;
    plan.ready = plan.blockers.iter().all(|b| !b.blocking);
    plan.hash = plan_hash(&plan, options);
    if plan.is_noop() {
        plan.notes
            .insert(0, "Already home: nothing to move or re-key.".into());
    }
    plan.text = describe(&plan);
    Ok(plan)
}

fn child_names(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| lower(&e.file_name().to_string_lossy()))
        .collect()
}

fn plan_moves(cx: &mut Ctx<'_>) {
    let vault = cx.vault.clone();
    let hollow = cx.hollow.clone();
    let vault_to = hollow.join("Vault");
    let projects_to = hollow.join("Projects");
    // 1. The vault.
    if vault != vault_to {
        let replaces = cx.target_state(&vault_to, &vault);
        cx.add_move(
            &vault,
            &vault_to,
            MoveKind::Vault,
            true,
            replaces,
            "The vault moves into the Hollow folder.".into(),
        );
    }
    // 2. The Projects folder, as a whole.
    let projects = cx.projects.clone();
    if let Some(p) = &projects {
        if *p != projects_to {
            let replaces = cx.target_state(&projects_to, p);
            cx.add_move(
                p,
                &projects_to,
                MoveKind::ProjectsRoot,
                true,
                replaces,
                "The Projects folder moves into the Hollow folder in one rename.".into(),
            );
        }
    }
    // Names in Projects after the move: (lower name, original path).
    let mut in_projects: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut personal_dir: Option<(String, PathBuf)> = None;
    if let Some(p) = &projects {
        for (name, path) in child_dirs(p) {
            if path == vault {
                continue;
            }
            if lower(&name) == "personal" {
                personal_dir = Some((name.clone(), path.clone()));
            }
            if is_hollow_vault(&path) {
                let to = cx.archive_target(&name, p);
                cx.add_move(
                    &path,
                    &to,
                    MoveKind::Archive,
                    false,
                    None,
                    "Another Hollow vault (not the live one): archived, never kept in Projects."
                        .into(),
                );
                continue;
            }
            in_projects.insert(lower(&name), path);
        }
    }
    // 3. `personal` becomes `Personal`.
    if let Some((name, path)) = &personal_dir {
        if name != "Personal" {
            let to = projects_to.join("Personal");
            cx.add_move(
                path,
                &to,
                MoveKind::Personal,
                false,
                None,
                format!("{name} becomes Personal."),
            );
        }
    }
    let personal_names = personal_dir
        .as_ref()
        .map(|(_, p)| child_names(p))
        .unwrap_or_default();
    // 4. Folders beside the vault.
    let Some(parent) = vault.parent().map(Path::to_path_buf) else {
        return;
    };
    let offered = parent != cx.home
        && !cx.home.starts_with(&parent)
        && parent != hollow
        && !projects.as_ref().is_some_and(|p| parent.starts_with(p));
    if !offered {
        return;
    }
    let mut siblings = Vec::new();
    for (name, path) in child_dirs(&parent) {
        if path == vault
            || projects
                .as_ref()
                .is_some_and(|p| *p == path || p.starts_with(&path))
            || hollow.starts_with(&path)
        {
            continue;
        }
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            continue;
        }
        cx.plan.beside_vault.push(s(&path));
        siblings.push((name, path));
    }
    let personal_opts: BTreeSet<String> = cx.options.personal.iter().map(|n| lower(n)).collect();
    let mut planned_projects: BTreeSet<String> = BTreeSet::new();
    let mut planned_personal: BTreeSet<String> = BTreeSet::new();
    for (name, path) in siblings {
        let key = lower(&name);
        let is_vault = is_hollow_vault(&path);
        let to_personal = personal_opts.contains(&key);
        // Same name in Projects: newer content wins.
        let rival = if to_personal {
            personal_names.contains(&key).then(|| {
                personal_dir
                    .as_ref()
                    .map(|(_, p)| p.join(&name))
                    .unwrap_or_default()
            })
        } else {
            in_projects.get(&key).cloned()
        };
        if !cx.options.include_vault_siblings {
            if let (Some(rival), false) = (&rival, is_vault) {
                let (a, b) = (newest(&path, cx), newest(rival, cx));
                let winner = if a >= b { s(&path) } else { s(rival) };
                cx.collision(
                    "sameName",
                    &path,
                    format!(
                        "{} is also in the Projects folder ({}); newest content: beside the vault {}, Projects {}",
                        name,
                        rival.display(),
                        fmt_time(a),
                        fmt_time(b)
                    ),
                    Some(winner),
                    false,
                );
            }
            continue;
        }
        if is_vault {
            let to = cx.archive_target(&name, &parent);
            cx.add_move(
                &path,
                &to,
                MoveKind::Archive,
                true,
                None,
                "Another Hollow vault (not the live one): archived.".into(),
            );
            continue;
        }
        let base = if to_personal {
            projects_to.join("Personal")
        } else {
            projects_to.clone()
        };
        let taken = if to_personal {
            &mut planned_personal
        } else {
            &mut planned_projects
        };
        if !taken.insert(key.clone()) {
            let to = cx.archive_target(&name, &parent);
            cx.add_move(
                &path,
                &to,
                MoveKind::Archive,
                true,
                None,
                "Same name as another folder beside the vault: archived.".into(),
            );
            continue;
        }
        let kind = if to_personal {
            MoveKind::Personal
        } else {
            MoveKind::Project
        };
        match rival {
            Some(rival) => {
                let (a, b) = (newest(&path, cx), newest(&rival, cx));
                let detail = format!(
                    "{name} is in both places; newest content: beside the vault {}, Projects {}",
                    fmt_time(a),
                    fmt_time(b)
                );
                if a >= b {
                    // The Projects copy goes to Archive first.
                    let rival_root = if to_personal {
                        personal_dir
                            .as_ref()
                            .map(|(_, p)| p.clone())
                            .unwrap_or_default()
                    } else {
                        projects.clone().unwrap_or_default()
                    };
                    let to = cx.archive_target(&name, &rival_root);
                    cx.collision("sameName", &path, detail, Some(s(&path)), false);
                    cx.add_move(
                        &rival,
                        &to,
                        MoveKind::Archive,
                        false,
                        None,
                        format!("Older copy of {name}: archived."),
                    );
                    let to = base.join(&name);
                    cx.add_move(
                        &path,
                        &to,
                        kind,
                        true,
                        None,
                        format!("Newer copy of {name} (beside the vault)."),
                    );
                } else {
                    cx.collision("sameName", &path, detail, Some(s(&rival)), false);
                    let to = cx.archive_target(&name, &parent);
                    cx.add_move(
                        &path,
                        &to,
                        MoveKind::Archive,
                        true,
                        None,
                        format!("Older copy of {name}: archived."),
                    );
                }
            }
            None => {
                let to = base.join(&name);
                if std::fs::symlink_metadata(&to).is_ok() && projects.is_none() {
                    cx.collision(
                        "targetExists",
                        &to,
                        format!("{} already exists", to.display()),
                        None,
                        true,
                    );
                    continue;
                }
                cx.add_move(
                    &path,
                    &to,
                    kind,
                    true,
                    None,
                    "Beside the vault: joins Projects.".into(),
                );
            }
        }
    }
    // Moves run in this order: vault, Projects folder, Personal, then the
    // rest in the order planned (an archived rival before its winner).
    let rank = |m: &PlannedMove| match m.kind {
        MoveKind::Vault => 0,
        MoveKind::ProjectsRoot => 1,
        MoveKind::Personal if PathBuf::from(&m.from).parent() == projects.as_deref() => 2,
        _ => 3,
    };
    cx.plan.moves.sort_by_key(rank);
}

fn newest(path: &Path, cx: &Ctx<'_>) -> Option<SystemTime> {
    let deadline = (Instant::now() + Duration::from_secs(10))
        .min(cx.deadline.max(Instant::now() + Duration::from_secs(2)));
    walk(path, cx.probes, deadline).newest
}

fn fmt_time(t: Option<SystemTime>) -> String {
    t.map(|t| {
        chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    })
    .unwrap_or_else(|| "unknown".into())
}

fn plan_mkdirs(cx: &mut Ctx<'_>) {
    let h = cx.hollow.clone();
    let targets: BTreeSet<String> = cx.plan.moves.iter().map(|m| m.to.clone()).collect();
    let p_moves = cx
        .plan
        .moves
        .iter()
        .any(|m| m.kind == MoveKind::ProjectsRoot);
    for d in [
        h.clone(),
        h.join("Repositories"),
        h.join("Worktrees"),
        h.join("Archive"),
        h.join("Projects"),
    ] {
        if targets.contains(&s(&d)) || (p_moves && d == h.join("Projects")) {
            continue;
        }
        if std::fs::symlink_metadata(&d).is_err() {
            cx.plan.mkdirs.push(s(&d));
        }
    }
    // Personal: needed when a folder beside the vault goes there and neither
    // a `personal` child of Projects nor a planned rename provides it.
    let personal = h.join("Projects").join("Personal");
    let needs = cx
        .plan
        .moves
        .iter()
        .any(|m| m.to.starts_with(&format!("{}/", s(&personal))));
    let provided = targets.contains(&s(&personal))
        || cx
            .projects
            .as_ref()
            .is_some_and(|p| child_dirs(p).iter().any(|(n, _)| lower(n) == "personal"))
        || std::fs::symlink_metadata(&personal).is_ok();
    if needs && !provided {
        cx.plan.mkdirs.push(s(&personal));
    }
}

/// A folder that moves by copy and sits inside another copy folder (a vault
/// inside the Projects root, listed before it): the clone of the outer folder
/// would hold a second, stale copy of the inner one. `Some(why)`.
pub(crate) fn copy_layout_conflict(moves: &[PlannedMove]) -> Option<String> {
    for (j, outer) in moves.iter().enumerate() {
        for (i, inner) in moves.iter().enumerate() {
            if i != j
                && outer.method == METHOD_COPY
                && inner.method == METHOD_COPY
                && inner.from.starts_with(&format!("{}/", outer.from))
            {
                return Some(format!(
                    "{} is inside {}, and both move by copy: the copy of {} would hold a second, stale copy of it. Move {} out of {} first.",
                    inner.from, outer.from, outer.from, inner.from, outer.from
                ));
            }
        }
    }
    None
}

/// The command that downloads the iCloud-only files of this plan's copy
/// folders (the installed app's binary; same options as the plan).
pub(crate) fn download_command(options: &Options) -> String {
    let mut c = String::from("/Applications/Hollow.app/Contents/MacOS/hollow move-home --download");
    if options.include_vault_siblings {
        c.push_str(" --with-vault-siblings");
    }
    for p in &options.personal {
        c.push_str(&format!(" --personal '{}'", p.replace('\'', "'\\''")));
    }
    c
}

fn measure_moves(cx: &mut Ctx<'_>) {
    let mut total = 0u64;
    let mut report = DatalessReport::default();
    let moves = cx.plan.moves.clone();
    for (i, m) in moves.iter().enumerate() {
        let w = walk(Path::new(&m.from), cx.probes, cx.deadline);
        let nested = moves[..i]
            .iter()
            .any(|o| m.from.starts_with(&format!("{}/", o.from)));
        // A folder in iCloud (a File Provider domain) moving out of it is
        // copied, not renamed: a rename out of the domain takes the File
        // Provider daemon minutes per 30,000 files and failed on the real
        // vault with ESTALE (move_home_icloud.rs).
        let from_domain = cx.probes.file_provider_domain(Path::new(&m.from));
        let to_domain =
            existing_ancestor(Path::new(&m.to)).and_then(|a| cx.probes.file_provider_domain(&a));
        let copy = !nested
            && from_domain.is_some()
            && from_domain != to_domain
            && !m.from.eq_ignore_ascii_case(&m.to);
        let pm = &mut cx.plan.moves[i];
        pm.bytes = w.bytes;
        pm.files = w.files;
        pm.dataless = w.dataless;
        pm.dataless_bytes = w.dataless_bytes;
        if copy {
            pm.method = METHOD_COPY.into();
        }
        if !nested {
            total += w.bytes;
            report.total += w.dataless;
            report.bytes += w.dataless_bytes;
            if copy {
                report.dirs += w.dataless_dirs;
                report.to_download += w.dataless;
                report.to_download_bytes += w.dataless_bytes;
                report.copy_folders.push(m.from.clone());
                report.copy_secs += crate::move_home_icloud::copy_secs(w.files);
                if from_domain.is_some() && report.file_provider_domain.is_none() {
                    report.file_provider_domain = from_domain.clone();
                }
            }
        }
        if w.dataless > 0 && !nested {
            let mut top: Vec<(String, u64)> = w.dataless_top.clone().into_iter().collect();
            top.sort_by_key(|b| std::cmp::Reverse(b.1));
            top.truncate(10);
            report.roots.push(DatalessRoot {
                root: m.from.clone(),
                count: w.dataless,
                bytes: w.dataless_bytes,
                top,
                sample: w.dataless_sample.clone(),
            });
        }
        if w.partial {
            report.partial = true;
            cx.plan.partial = true;
        }
        for l in &w.escaping_links {
            if !nested {
                cx.plan.notes.push(format!(
                    "A relative link leaves {} and will point elsewhere after the move: {l}",
                    m.from
                ));
            }
        }
        if !nested && copy {
            if !w.special_unexpected.is_empty() {
                let more = match w.special_unexpected.len() {
                    1 => String::new(),
                    n => format!(" and {} more", n - 1),
                };
                cx.blocker(
                    "specialFile",
                    format!(
                        "{}{more} is a socket, pipe or device file in a folder that moves by copy; a copy can't carry it. Move or remove it (after closing what made it), then check again.",
                        w.special_unexpected[0]
                    ),
                    true,
                );
            }
            if !w.special_throwaway.is_empty() {
                cx.plan.notes.push(format!(
                    "{} known throwaway runtime file(s) in {} (git's fsmonitor socket, made again when needed) are left out of the copy and its comparison; the original keeps them until it is removed: {}.",
                    w.special_throwaway.len(),
                    m.from,
                    w.special_throwaway.join(", ")
                ));
            }
        }
        for n in &w.nosync {
            if !nested {
                cx.plan.notes.push(format!(
                    "{n} is a .nosync folder; it moves with its parent and stays out of iCloud."
                ));
            }
        }
        cx.walks.push((i, w));
    }
    if report.file_provider_domain.is_none() {
        report.file_provider_domain = moves
            .iter()
            .find_map(|m| cx.probes.file_provider_domain(Path::new(&m.from)));
    }
    if let Some(why) = copy_layout_conflict(&cx.plan.moves) {
        cx.blocker("layout", why, true);
    }
    report.download_secs =
        crate::move_home_icloud::download_secs(report.to_download, report.to_download_bytes);
    if !report.copy_folders.is_empty() {
        cx.plan.notes.push(format!(
            "{} folder(s) are in iCloud ({}): they move by copy, not rename (a rename out of iCloud takes the File Provider daemon minutes per 30,000 files and failed on the first real run with ESTALE). Each is cloned on this volume (no extra disk space), compared entry by entry, renamed into place, and its original removed last, after every other step; a failure puts it back. About {} for the copies.",
            report.copy_folders.len(),
            report.file_provider_domain.clone().unwrap_or_default(),
            crate::move_home_icloud::duration_text(report.copy_secs)
        ));
    }
    if report.to_download > 0 {
        let partial = if report.partial {
            " (count partial)"
        } else {
            ""
        };
        let detail = format!(
            "{} iCloud-only (evicted) files{partial}, {}, are in folders that move by copy and must be downloaded first (copying one downloads it alone and waits for it). That takes about {} (small files ~{} a second, large ones ~{} MB/s). Run `{}` in Terminal (Hollow can stay open; ctrl+c stops it and a rerun continues), then check again.{}",
            report.to_download,
            size(report.to_download_bytes),
            crate::move_home_icloud::duration_text(report.download_secs),
            crate::move_home_icloud::FILES_PER_SEC,
            (crate::move_home_icloud::BYTES_PER_SEC / 1048576.0).round(),
            download_command(cx.options),
            if report.dirs > 0 {
                format!(" {} are whole evicted folders; what they hold shows only after they download, so the size and time are a floor.", report.dirs)
            } else {
                String::new()
            }
        );
        cx.blocker("dataless", detail, true);
        let need = report.to_download_bytes;
        let free = free_bytes(Path::new(&cx.plan.hollow_home));
        if free < need + need / 10 + 2 * 1024 * 1024 * 1024 {
            cx.blocker(
                "space",
                format!(
                    "{} free; downloading the iCloud-only files needs {} and a margin.",
                    size(free),
                    size(need)
                ),
                true,
            );
        }
    } else if report.total > 0 {
        let partial = if report.partial {
            " (count partial)"
        } else {
            ""
        };
        cx.blocker(
            "dataless",
            format!(
                "{} iCloud-evicted (dataless) files{partial} are in folders that move by rename.",
                report.total
            ),
            false,
        );
    }
    cx.plan.dataless = report;
    cx.plan.total_bytes = total;
    if cx.plan.partial {
        cx.plan.notes.push(format!(
            "The walk stopped at the {} s budget: sizes, counts and repositories are partial (--budget raises it).",
            cx.options.budget_secs.unwrap_or(DEFAULT_BUDGET)
        ));
    }
}

/// The connected projects' ids (from `Directories/<stem>.json`) that change.
fn plan_ids(cx: &mut Ctx<'_>, maps: &[(String, String)]) {
    let manifest = cx
        .roots
        .support
        .join("Directories")
        .join(format!("{}.json", rekey::stem_for(&cx.vault)));
    let mut known: BTreeSet<String> = BTreeSet::new();
    if let Ok(text) = std::fs::read_to_string(&manifest) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            for p in v
                .get("projects")
                .and_then(|p| p.as_array())
                .into_iter()
                .flatten()
            {
                let (Some(id), Some(path)) = (
                    p.get("id").and_then(|x| x.as_str()),
                    p.get("path").and_then(|x| x.as_str()),
                ) else {
                    continue;
                };
                known.insert(id.to_string());
                let Some(new_path) = rekey::rewrite_whole(path, maps) else {
                    continue;
                };
                if !rekey::is_local_id(id) {
                    continue;
                }
                let new_id = rekey::workspace_id(Path::new(&new_path));
                if new_id == id {
                    continue;
                }
                let ws = cx.vault.join("workspaces");
                let has = ws.join(id).is_dir();
                if std::fs::symlink_metadata(ws.join(&new_id)).is_ok() {
                    cx.collision(
                        "idCollision",
                        &ws.join(&new_id),
                        format!("{path} would become {new_id}, and a workspace with that id already exists"),
                        None,
                        true,
                    );
                }
                cx.plan.ids.push(IdChange {
                    old: id.to_string(),
                    new: new_id,
                    project_from: path.to_string(),
                    project_to: new_path,
                    has_workspace: has,
                });
            }
        }
    }
    for (name, _) in child_dirs(&cx.vault.join("workspaces")) {
        if rekey::is_local_id(&name) && !known.contains(&name) {
            cx.plan.orphans.push(name);
        }
    }
}

/// Every entry at depth 1 or 2 under the support and app data folders named
/// by the old stem.
pub(crate) fn stem_entries(roots: &Roots, stem: &str) -> Vec<PathBuf> {
    let matches = |name: &str| {
        name == stem
            || name
                .strip_prefix(stem)
                .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('-'))
    };
    let mut out = Vec::new();
    for base in [&roots.support, &roots.app_data] {
        for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if matches(&name) {
                out.push(path.clone());
                continue;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                for e2 in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    if matches(&e2.file_name().to_string_lossy()) {
                        out.push(e2.path());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn plan_stores(cx: &mut Ctx<'_>) {
    let (old, new) = (cx.plan.old_stem.clone(), cx.plan.new_stem.clone());
    if old == new {
        return;
    }
    let db = sessions_db(cx.roots, &old);
    for entry in stem_entries(cx.roots, &old) {
        let name = entry
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = entry.parent().map(Path::to_path_buf).unwrap_or_default();
        if entry == sidecar(&db, "-wal") || entry == sidecar(&db, "-shm") {
            continue;
        }
        let to = parent.join(format!("{new}{}", &name[old.len()..]));
        if std::fs::symlink_metadata(&to).is_ok() {
            cx.collision(
                "stemCollision",
                &to,
                format!(
                    "{} already exists, so {} can't take its name",
                    to.display(),
                    entry.display()
                ),
                None,
                true,
            );
        }
        let what = if entry == db {
            WHAT_SESSIONS
        } else {
            WHAT_STORE
        };
        cx.plan.stores.push(StoreRekey {
            from: s(&entry),
            to: s(&to),
            what: what.into(),
        });
    }
    // The new Sessions trio must not exist either (a -wal alone is a set).
    let new_db = sessions_db(cx.roots, &new);
    for suffix in ["-wal", "-shm"] {
        let p = sidecar(&new_db, suffix);
        if std::fs::symlink_metadata(&p).is_ok()
            && !cx.plan.collisions.iter().any(|c| c.path == s(&p))
        {
            cx.collision(
                "stemCollision",
                &p,
                format!("{} already exists", p.display()),
                None,
                true,
            );
        }
    }
}

/// The real cwd of a Claude project folder: the first `cwd` in one of its
/// transcripts (never an evicted file).
fn claude_cwd(dir: &Path, probes: &dyn Probes) -> Option<String> {
    use std::io::{BufRead, Read};
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect();
    files.sort();
    for f in files.iter().take(5) {
        let Ok(meta) = std::fs::symlink_metadata(f) else {
            continue;
        };
        if !meta.file_type().is_file() || probes.flags(f, &meta) & SF_DATALESS != 0 {
            continue;
        }
        let Ok(file) = std::fs::File::open(f) else {
            continue;
        };
        for line in std::io::BufReader::new(file.take(256 * 1024))
            .lines()
            .take(50)
            .map_while(Result::ok)
        {
            if !line.contains("\"cwd\"") {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
                    return Some(cwd.to_string());
                }
            }
        }
    }
    None
}

/// The entry-by-entry moves that merge `src` into an existing `dst`.
fn plan_merge(src: &Path, dst: &Path, out: &mut Vec<StoreRekey>, merges: &mut u64) {
    let mut entries: Vec<_> = std::fs::read_dir(src)
        .into_iter()
        .flatten()
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let from = e.path();
        let to = dst.join(&name);
        let src_dir = e.file_type().is_ok_and(|t| t.is_dir());
        match std::fs::symlink_metadata(&to) {
            Err(_) => out.push(StoreRekey {
                from: s(&from),
                to: s(&to),
                what: WHAT_CLAUDE_MERGE.into(),
            }),
            Ok(m) if src_dir && m.is_dir() => plan_merge(&from, &to, out, merges),
            Ok(_) => {
                let mut n = 1;
                let mut alt = dst.join(rekey::moved_name(&name, n));
                while std::fs::symlink_metadata(&alt).is_ok() || out.iter().any(|o| o.to == s(&alt))
                {
                    n += 1;
                    alt = dst.join(rekey::moved_name(&name, n));
                }
                *merges += 1;
                out.push(StoreRekey {
                    from: s(&from),
                    to: s(&alt),
                    what: WHAT_CLAUDE_MERGE_RENAMED.into(),
                });
            }
        }
    }
}

fn plan_claude_dirs(cx: &mut Ctx<'_>, maps: &[(String, String)], all: &mut Vec<(String, String)>) {
    let root = cx.roots.claude_projects();
    let Ok(rd) = std::fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    dirs.sort();
    let mut change = AgentStoreChange {
        agent: "Claude Code projects".into(),
        path: s(&root),
        changes: 0,
        dead: 0,
        merges: 0,
        ambiguous: Vec::new(),
    };
    let mut targets: BTreeSet<String> = BTreeSet::new();
    let mut stores = Vec::new();
    for (name, dir) in dirs {
        // A folder a merge emptied (only empty subfolders left) has nothing
        // to move.
        if !has_files(&dir) {
            continue;
        }
        let by_cwd = claude_cwd(&dir, cx.probes).filter(|cwd| rekey::claude_dir_name(cwd) == name);
        let new_name = if let Some(cwd) = by_cwd {
            let Some(new_cwd) = rekey::rewrite_whole(&cwd, maps) else {
                continue;
            };
            rekey::claude_dir_name(&new_cwd)
        } else {
            // No transcript names its folder. The name is lossy (every
            // non-alphanumeric is `-`), so rebuild the real folder: under each
            // moved folder whose encoding starts the name, follow the rest
            // through the folders on disk (lstat and listing only). Rename only
            // when exactly one folder fits.
            let mut found: Vec<PathBuf> = Vec::new();
            let mut hit = false;
            let mut evicted = false;
            for (old, _) in maps {
                let e = rekey::claude_dir_name(old);
                if name == e || name.starts_with(&format!("{e}-")) {
                    hit = true;
                    let mut walk = Decode {
                        probes: cx.probes,
                        found: &mut found,
                        budget: 5000,
                        evicted: false,
                    };
                    walk.under(Path::new(old), &name[e.len()..]);
                    evicted |= walk.evicted;
                }
            }
            if !hit {
                continue;
            }
            if evicted {
                change.ambiguous.push(format!(
                    "{name}: an iCloud-evicted folder is on the way to it (listing it would download it), and no transcript says its folder"
                ));
                continue;
            }
            let mut real: Vec<PathBuf> = found
                .iter()
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()))
                .collect();
            real.sort();
            real.dedup();
            match real.len() {
                0 => {
                    change.ambiguous.push(format!(
                        "{name}: no folder on disk fits its name and no transcript says its folder"
                    ));
                    continue;
                }
                1 => {}
                _ => {
                    change.ambiguous.push(format!("{name}: more than one folder on disk fits its name, and no transcript says its folder"));
                    continue;
                }
            }
            // The path as the mappings know it (not through links).
            let Some(path) = found
                .iter()
                .find(|p| rekey::rewrite_whole(&s(p), maps).is_some())
            else {
                continue;
            };
            let Some(new_path) = rekey::rewrite_whole(&s(path), maps) else {
                continue;
            };
            rekey::claude_dir_name(&new_path)
        };
        if new_name == name {
            continue;
        }
        if new_name.len() > 200 {
            change.ambiguous.push(format!(
                "{name}: the new name would be longer than Claude Code keeps"
            ));
            continue;
        }
        if !targets.insert(new_name.clone()) {
            change.ambiguous.push(format!(
                "{name}: another folder maps to {new_name} too; left as it is"
            ));
            continue;
        }
        let to = root.join(&new_name);
        change.changes += 1;
        all.push((s(&dir), s(&to)));
        if std::fs::symlink_metadata(&to).is_ok() {
            let mut merges = 0;
            let start = stores.len();
            plan_merge(&dir, &to, &mut stores, &mut merges);
            change.merges += merges.max(1);
            for st in &stores[start..] {
                if st.what == WHAT_CLAUDE_MERGE_RENAMED {
                    all.push((st.from.clone(), st.to.clone()));
                }
            }
        } else {
            stores.push(StoreRekey {
                from: s(&dir),
                to: s(&to),
                what: WHAT_CLAUDE_DIR.into(),
            });
        }
    }
    cx.plan.stores.extend(stores);
    if change.changes > 0 || !change.ambiguous.is_empty() {
        cx.plan.agents.push(change);
    }
}

/// Finds the folders under a base whose path after it encodes to a rest
/// (Claude's folder-name encoding, the rest empty or starting with `-`). At
/// most two are collected; `budget` caps the folders listed. An
/// iCloud-evicted folder is never listed (that would download it): reaching
/// one sets `evicted`.
struct Decode<'a> {
    probes: &'a dyn Probes,
    found: &'a mut Vec<PathBuf>,
    budget: u32,
    evicted: bool,
}

impl Decode<'_> {
    fn under(&mut self, base: &Path, rest: &str) {
        if self.found.len() >= 2 || self.budget == 0 || self.evicted {
            return;
        }
        self.budget -= 1;
        let Ok(meta) = std::fs::symlink_metadata(base) else {
            return;
        };
        if rest.is_empty() {
            self.found.push(base.to_path_buf());
            return;
        }
        if !meta.is_dir() {
            return;
        }
        if self.probes.flags(base, &meta) & SF_DATALESS != 0 {
            self.evicted = true;
            return;
        }
        for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let enc = format!("-{}", rekey::claude_dir_name(&name));
            // A whole name only: `-fo` must not take the start of `-foo-bar`.
            if let Some(left) = rest.strip_prefix(enc.as_str()) {
                if left.is_empty() || left.starts_with('-') {
                    self.under(&base.join(&name), left);
                }
            }
        }
    }
}

/// Whether `dir` holds any entry other than (empty) folders.
fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let p = e.path();
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => has_files(&p),
                Ok(_) => true,
                Err(_) => false,
            }
        })
}

/// Hollow's JSON stores under the support and app data folders (depth 1-2,
/// the database folders skipped).
fn store_json_files(roots: &Roots) -> Vec<PathBuf> {
    let skip = ["Sessions", "SearchIndex", "EmbedIndex", "Backups"];
    let mut out = Vec::new();
    for base in [&roots.support, &roots.app_data] {
        for e in std::fs::read_dir(base).into_iter().flatten().flatten() {
            let path = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_file()
                && path.extension().is_some_and(|x| x == "json")
                && path != roots.config_path
            {
                out.push(path);
            } else if ft.is_dir() && !skip.contains(&e.file_name().to_string_lossy().as_ref()) {
                for e2 in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    let p2 = e2.path();
                    if e2.file_type().is_ok_and(|t| t.is_file())
                        && p2.extension().is_some_and(|x| x == "json")
                    {
                        out.push(p2);
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The config keys a move changes: (key, new value).
pub(crate) fn config_changes(
    plan: &Plan,
    map: &BTreeMap<String, serde_json::Value>,
) -> Vec<(String, String)> {
    let maps = &plan.mappings;
    let mut out = Vec::new();
    let get = |k: &str| map.get(k).and_then(|v| v.as_str()).map(str::to_string);
    if let Some(v) = get("vaultPath") {
        if v != plan.vault_to
            && !plan.moves.is_empty()
            && plan
                .moves
                .iter()
                .any(|m| m.kind == crate::move_home::MoveKind::Vault)
        {
            out.push(("vaultPath".into(), plan.vault_to.clone()));
        }
    }
    let projects_to = format!("{}/Projects", plan.hollow_home);
    let into_projects = plan
        .moves
        .iter()
        .any(|m| m.to == projects_to || m.to.starts_with(&format!("{projects_to}/")));
    if into_projects && get("projectsPath").as_deref() != Some(projects_to.as_str()) {
        out.push(("projectsPath".into(), projects_to.clone()));
    }
    if get("hollowHome").as_deref() != Some(plan.hollow_home.as_str()) && !plan.is_noop() {
        out.push(("hollowHome".into(), plan.hollow_home.clone()));
    }
    if let Some(v) = get("gitReposPath") {
        if let Some(n) = rekey::rewrite_whole(&v, maps) {
            out.push(("gitReposPath".into(), n));
        }
    }
    out
}

fn plan_files(cx: &mut Ctx<'_>, maps: &[(String, String)], ids: &BTreeMap<String, String>) {
    let (old, new) = (cx.plan.old_stem.clone(), cx.plan.new_stem.clone());
    let mut files = Vec::new();
    // Hollow config.
    if let Ok(map) = crate::setup::load_config_map(&cx.roots.config_path) {
        let n = config_changes(&cx.plan, &map).len() as u64;
        if n > 0 {
            files.push(FileRewrite {
                path: s(&cx.roots.config_path),
                changes: n,
                what: FILE_CONFIG.into(),
            });
        }
    }
    // Hollow stores.
    for f in store_json_files(cx.roots) {
        let Ok(meta) = std::fs::symlink_metadata(&f) else {
            continue;
        };
        if meta.len() > MAX_STORE_BYTES {
            cx.plan.notes.push(format!(
                "{} is larger than 64 MB and is not rewritten.",
                f.display()
            ));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        match rekey::rewrite_store_json(&text, &old, &new, maps, ids) {
            Some((_, n)) if n > 0 => files.push(FileRewrite {
                path: s(&f),
                changes: n as u64,
                what: FILE_STORE.into(),
            }),
            Some(_) => {}
            None => cx.plan.notes.push(format!(
                "{} isn't readable JSON; left as it is.",
                f.display()
            )),
        }
    }
    // Vault notes.
    let mut notes: Vec<PathBuf> = Vec::new();
    for (_, ws) in child_dirs(&cx.vault.join("workspaces")) {
        notes.push(ws.join("WORKSPACE.md"));
    }
    for e in std::fs::read_dir(cx.vault.join("workflows"))
        .into_iter()
        .flatten()
        .flatten()
    {
        if e.path().extension().is_some_and(|x| x == "md") {
            notes.push(e.path());
        }
    }
    notes.push(cx.vault.join("README.md"));
    notes.sort();
    let mut evicted = 0;
    for f in notes {
        match read_checked(&f, cx.probes) {
            Ok(Some(text)) => {
                let (_, n) = rekey::rewrite_text(&text, maps, ids);
                if n > 0 {
                    files.push(FileRewrite {
                        path: s(&f),
                        changes: n as u64,
                        what: FILE_NOTE.into(),
                    });
                }
            }
            Ok(None) => {
                if std::fs::symlink_metadata(&f).is_ok() {
                    evicted += 1;
                }
            }
            Err(e) => cx.plan.notes.push(e),
        }
    }
    if evicted > 0 {
        cx.plan.notes.push(format!("{evicted} vault notes are iCloud-evicted or links and are not rewritten (reading them would download them)."));
    }
    // Agent stores.
    let claude_json = cx.roots.claude_json();
    if let Ok(Some(text)) = read_checked(&claude_json, cx.probes) {
        if let Some((_, n, taken)) = rekey::rewrite_claude_json(&text, maps) {
            let dead = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| {
                    v.get("projects").and_then(|p| p.as_object()).map(|m| {
                        m.keys()
                            .filter(|k| {
                                !Path::new(k).exists() && rekey::rewrite_whole(k, maps).is_none()
                            })
                            .count() as u64
                    })
                })
                .unwrap_or(0);
            if n > 0 {
                files.push(FileRewrite {
                    path: s(&claude_json),
                    changes: n as u64,
                    what: FILE_CLAUDE_JSON.into(),
                });
            }
            if n > 0 || !taken.is_empty() || dead > 0 {
                cx.plan.agents.push(AgentStoreChange {
                    agent: "Claude Code settings".into(),
                    path: s(&claude_json),
                    changes: n as u64,
                    dead,
                    merges: 0,
                    ambiguous: taken
                        .into_iter()
                        .map(|k| format!("{k}: its new path is already a key; kept as it is"))
                        .collect(),
                });
            }
        }
    }
    let codex = cx.roots.codex_config();
    if let Ok(Some(text)) = read_checked(&codex, cx.probes) {
        let (_, n, taken) = rekey::rewrite_codex_headers(&text, maps);
        let dead = text
            .lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix("[projects.\"")?
                    .strip_suffix("\"]")
                    .map(str::to_string)
            })
            .filter(|p| !Path::new(p).exists() && rekey::rewrite_whole(p, maps).is_none())
            .count() as u64;
        if n > 0 {
            files.push(FileRewrite {
                path: s(&codex),
                changes: n as u64,
                what: FILE_CODEX.into(),
            });
        }
        if n > 0 || !taken.is_empty() || dead > 0 {
            cx.plan.agents.push(AgentStoreChange {
                agent: "Codex".into(),
                path: s(&codex),
                changes: n as u64,
                dead,
                merges: 0,
                ambiguous: taken
                    .into_iter()
                    .map(|k| format!("{k}: its new path is already a table; kept as it is"))
                    .collect(),
            });
        }
    }
    for (path, what, agent) in [
        (cx.roots.gemini_projects(), FILE_GEMINI, "Gemini"),
        (
            cx.roots.claude_desktop(),
            FILE_CLAUDE_DESKTOP,
            "Claude desktop",
        ),
        (cx.roots.obsidian(), FILE_OBSIDIAN, "Obsidian"),
    ] {
        if let Ok(Some(text)) = read_checked(&path, cx.probes) {
            if let Some((_, n)) = rekey::rewrite_json_paths(&text, maps) {
                if n > 0 {
                    files.push(FileRewrite {
                        path: s(&path),
                        changes: n as u64,
                        what: what.into(),
                    });
                    cx.plan.agents.push(AgentStoreChange {
                        agent: agent.into(),
                        path: s(&path),
                        changes: n as u64,
                        dead: 0,
                        merges: 0,
                        ambiguous: Vec::new(),
                    });
                }
            }
        }
    }
    cx.plan.files = files;
}

fn plan_columns(
    cx: &mut Ctx<'_>,
    maps: &[(String, String)],
    ids: &BTreeMap<String, String>,
    caller: Option<&str>,
) {
    let db = sessions_db(cx.roots, &cx.plan.old_stem);
    if !db.exists() {
        cx.plan.notes.push(format!(
            "No Sessions database at {}; nothing to rewrite there.",
            db.display()
        ));
        return;
    }
    match live_queue(&db, caller) {
        Ok(rows) => {
            for (label, status) in rows {
                cx.blocker("runningChat", format!("{label} ({status})"), true);
            }
        }
        Err(e) => cx.blocker(
            "runningChat",
            format!("Couldn't read the run queue: {e}"),
            true,
        ),
    }
    if crate::hollow_databases::shm_in_use(&db) {
        cx.blocker(
            "hollowOpen",
            "Hollow has its Sessions database open; the move runs after Hollow quits.".into(),
            false,
        );
    }
    let conn = match open_read_only(&db) {
        Ok(c) => c,
        Err(e) => {
            cx.plan.notes.push(e);
            return;
        }
    };
    let (cols, missing) = rewrite_columns(&conn);
    if !missing.is_empty() {
        cx.plan.notes.push(format!(
            "Not in this Sessions database (skipped): {}.",
            missing.join(", ")
        ));
    }
    for (table, column, kind) in cols {
        match count_column(&conn, &table, &column, kind, maps, ids) {
            Ok(rows) => cx.plan.columns.push(ColumnRewrite {
                table,
                column,
                rows,
            }),
            Err(e) => cx.plan.notes.push(format!("{table}.{column}: {e}")),
        }
    }
    let moved_ids = ids.iter().filter(|(a, b)| a != b).count();
    for (table, column) in kept_id_columns(&conn) {
        if let Ok(rows) = count_column(&conn, &table, &column, ColKind::Id, maps, ids) {
            if rows > 0 {
                cx.plan.notes.push(format!(
                    "{table}.{column}: {rows} rows keep their old workspace ids (append-only, never rewritten); the move records {moved_ids} workspace aliases so they still show under the moved workspaces."
                ));
            }
        }
    }
}

fn plist_label(text: &str) -> Option<String> {
    let v = plist::Value::from_reader(std::io::Cursor::new(text.as_bytes())).ok()?;
    v.as_dictionary()?
        .get("Label")?
        .as_string()
        .map(str::to_string)
}

fn plan_launch_items(cx: &mut Ctx<'_>, maps: &[(String, String)]) {
    let forms = rekey::home_forms(maps, &cx.roots.home);
    let mut agents = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(cx.roots.launch_agents())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "plist"))
        .collect();
    entries.sort();
    for p in entries {
        let Ok(Some(text)) = read_checked(&p, cx.probes) else {
            continue;
        };
        let label = plist_label(&text).unwrap_or_else(|| {
            p.file_stem()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let mentions = rekey::mentions(&text, &forms);
        let on_plan = cx.options.launch_agents.contains(&label);
        if mentions.is_empty() && !on_plan {
            continue;
        }
        if !text.trim_start().starts_with('<') {
            cx.plan.notes.push(format!(
                "{} is a binary plist; it is listed but never rewritten.",
                p.display()
            ));
        }
        if on_plan && !mentions.is_empty() {
            let (_, n) = rekey::rewrite_paths(&text, &forms);
            cx.plan.files.push(FileRewrite {
                path: s(&p),
                changes: n as u64,
                what: FILE_LAUNCH_AGENT.into(),
            });
        }
        if !on_plan {
            cx.blocker(
                "launchAgent",
                format!("{label} names a moved folder and is not on the plan; it keeps using the old path through the link left there (--launch-agent {label} rewrites it)."),
                false,
            );
        }
        agents.push(LaunchItem {
            label,
            path: s(&p),
            mentions,
            on_plan,
        });
    }
    for l in &cx.options.launch_agents {
        if !agents.iter().any(|a| a.label == *l) {
            cx.plan
                .notes
                .push(format!("LaunchAgent {l} is on the plan but wasn't found."));
        }
    }
    let mut scripts = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(cx.roots.local_bin())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if secret_name(&name) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if !meta.file_type().is_file() || meta.len() > 1024 * 1024 {
            continue;
        }
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        if bytes.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mentions = rekey::mentions(&text, &forms);
        let on_plan = cx.options.scripts.iter().any(|o| *o == s(&p) || *o == name);
        if mentions.is_empty() {
            continue;
        }
        if on_plan {
            let (_, n) = rekey::rewrite_paths(&text, &forms);
            cx.plan.files.push(FileRewrite {
                path: s(&p),
                changes: n as u64,
                what: FILE_SCRIPT.into(),
            });
        } else {
            cx.blocker(
                "script",
                format!("{} names a moved folder and is not on the plan; it keeps working through the link left at the old path (--script {name} rewrites it).", p.display()),
                false,
            );
        }
        scripts.push(LaunchItem {
            label: name,
            path: s(&p),
            mentions,
            on_plan,
        });
    }
    cx.plan.launch_agents = agents;
    cx.plan.scripts = scripts;
}

/// A linked worktree's main repository, from its `.git` file.
pub(crate) fn main_of_linked(wt: &Path, probes: &dyn Probes) -> Option<PathBuf> {
    let text = read_checked(&wt.join(".git"), probes).ok()??;
    let gitdir = text.trim().strip_prefix("gitdir:")?.trim().to_string();
    let idx = gitdir.find("/.git/worktrees/")?;
    Some(PathBuf::from(&gitdir[..idx]))
}

/// A main repository's linked worktrees: (folder, `.git/worktrees/<id>/gitdir`).
pub(crate) fn worktrees_of(
    main: &Path,
    probes: &dyn Probes,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(main.join(".git").join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
    {
        let gitdir = e.path().join("gitdir");
        match read_checked(&gitdir, probes)? {
            Some(text) => {
                let p = PathBuf::from(text.trim());
                let folder = p.parent().map(Path::to_path_buf).unwrap_or(p);
                out.push((folder, gitdir));
            }
            None if std::fs::symlink_metadata(&gitdir).is_ok() => {
                return Err("its git metadata is iCloud-evicted".into());
            }
            None => {}
        }
    }
    out.sort();
    Ok(out)
}

fn plan_repos(cx: &mut Ctx<'_>, maps: &[(String, String)]) {
    let mut mains: BTreeSet<PathBuf> = BTreeSet::new();
    for (i, w) in &cx.walks {
        let nested = cx.plan.moves[..*i]
            .iter()
            .any(|o| cx.plan.moves[*i].from.starts_with(&format!("{}/", o.from)));
        if nested {
            continue;
        }
        mains.extend(w.mains.iter().cloned());
        for l in &w.linked {
            if let Some(m) = main_of_linked(l, cx.probes) {
                mains.insert(m);
            }
        }
    }
    for main in mains {
        let main_s = s(&main);
        let main_to = rekey::whole_or_same(&main_s, maps);
        let (worktrees, skipped) = match worktrees_of(&main, cx.probes) {
            Ok(w) => (w, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let broken = worktrees
            .iter()
            .filter(|(f, _)| std::fs::symlink_metadata(f.join(".git")).is_err())
            .count() as u64;
        let moves_any = main_to != main_s
            || worktrees
                .iter()
                .any(|(f, _)| rekey::rewrite_whole(&s(f), maps).is_some());
        if !moves_any && skipped.is_none() {
            continue;
        }
        if worktrees.is_empty() && skipped.is_none() {
            continue;
        }
        cx.plan.repos.push(RepoRepair {
            repo_from: main_s,
            repo_to: main_to,
            worktrees: worktrees.len() as u64,
            broken,
            skipped,
        });
    }
}

fn plan_checks(cx: &mut Ctx<'_>) {
    let hollow = cx.hollow.clone();
    let home = cx.home.clone();
    // Inside $HOME.
    if !cx.options.allow_outside_home {
        let mut outside: Vec<String> = Vec::new();
        for m in &cx.plan.moves {
            for p in [&m.from, &m.to] {
                if !canonical_lenient(Path::new(p)).starts_with(&home) {
                    outside.push(p.clone());
                }
            }
        }
        if !hollow.starts_with(&home) {
            outside.push(s(&hollow));
        }
        outside.sort();
        outside.dedup();
        for p in outside {
            cx.blocker(
                "outsideHome",
                format!("{p} is outside your home folder (--allow-outside-home allows it)."),
                true,
            );
        }
    }
    // One volume.
    let hdev = dev_of(&hollow);
    let mut same = true;
    for m in cx.plan.moves.clone() {
        let to_parent = Path::new(&m.to)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        if dev_of(Path::new(&m.from)) != dev_of(&to_parent) || dev_of(Path::new(&m.from)) != hdev {
            same = false;
            cx.blocker(
                "crossVolume",
                format!(
                    "{} is on a different volume from {}; the move only renames on one volume.",
                    m.from,
                    to_parent.display()
                ),
                true,
            );
        }
    }
    if dev_of(&cx.roots.support) != hdev && !cx.plan.stores.is_empty() {
        cx.blocker(
            "crossVolume",
            "Hollow's app support folder is on a different volume from the Hollow folder, where the backup goes.".into(),
            true,
        );
        same = false;
    }
    cx.plan.same_volume = same;
    // Space for the backup copies (clones cost nothing on APFS; a plain copy
    // costs their size).
    let free = free_bytes(&hollow);
    cx.plan.free_bytes = free;
    let db = sessions_db(cx.roots, &cx.plan.old_stem);
    let mut need: u64 = [sidecar(&db, ""), sidecar(&db, "-wal")]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    need += cx
        .plan
        .files
        .iter()
        .filter_map(|f| std::fs::metadata(&f.path).ok())
        .map(|m| m.len())
        .sum::<u64>();
    if free < need + need / 5 + 64 * 1024 * 1024 {
        cx.blocker(
            "space",
            format!(
                "{} free; the backup and the database copy need about {}.",
                size(free),
                size(need + need / 5)
            ),
            true,
        );
    }
}

/// Binds a confirm to the decisions (not sizes or counts, which change as
/// you work): moves, folders made, mappings, stores, ids, launch items and
/// scripts on the plan, and blocking reasons.
pub fn plan_hash(plan: &Plan, options: &Options) -> String {
    let decisions = serde_json::json!({
        "version": plan.version,
        "hollowHome": plan.hollow_home,
        "vault": [plan.vault_from, plan.vault_to, plan.old_stem, plan.new_stem],
        "moves": plan.moves.iter().map(|m| (m.from.clone(), m.to.clone(), m.kind, m.replaces_symlink.clone(), m.compat_symlink, m.method.clone())).collect::<Vec<_>>(),
        "mkdirs": plan.mkdirs,
        "mappings": plan.mappings,
        "stores": plan.stores.iter().map(|s| (s.from.clone(), s.to.clone())).collect::<Vec<_>>(),
        "ids": plan.ids.iter().map(|i| (i.old.clone(), i.new.clone())).collect::<Vec<_>>(),
        "launchAgents": plan.launch_agents.iter().filter(|l| l.on_plan).map(|l| l.path.clone()).collect::<Vec<_>>(),
        "scripts": plan.scripts.iter().filter(|l| l.on_plan).map(|l| l.path.clone()).collect::<Vec<_>>(),
        "blocking": plan.blockers.iter().filter(|b| b.blocking).map(|b| b.kind.clone()).collect::<Vec<_>>(),
        "options": Options { budget_secs: None, ..options.clone() },
    });
    format!("{:x}", Sha256::digest(decisions.to_string().as_bytes()))
}

// ── The readable report ───────────────────────────────────────────────────

fn kind_word(k: MoveKind) -> &'static str {
    match k {
        MoveKind::Vault => "vault",
        MoveKind::ProjectsRoot => "Projects folder",
        MoveKind::Project => "project",
        MoveKind::Personal => "personal",
        MoveKind::Archive => "archive",
    }
}

/// The plan as readable lines.
pub fn describe(plan: &Plan) -> Vec<String> {
    let mut l = Vec::new();
    l.push("Move Hollow home: dry run (nothing was changed).".to_string());
    l.push(format!("Hollow folder: {}", plan.hollow_home));
    if plan.old_stem != plan.new_stem {
        l.push(format!(
            "Vault: {} -> {} (stores re-keyed {} -> {})",
            plan.vault_from, plan.vault_to, plan.old_stem, plan.new_stem
        ));
    } else {
        l.push(format!(
            "Vault: {} (already in place, stem {})",
            plan.vault_from, plan.old_stem
        ));
    }
    if plan.moves.is_empty() {
        l.push("Moves: none.".into());
    } else {
        l.push(format!("Moves ({}):", plan.moves.len()));
        for m in &plan.moves {
            let mut extra = Vec::new();
            if let Some(t) = &m.replaces_symlink {
                extra.push(format!("replaces the link to {t}"));
            }
            if m.compat_symlink {
                extra.push("a link stays at the old path".to_string());
            }
            let dataless = if m.dataless > 0 {
                format!(", {} evicted ({})", m.dataless, size(m.dataless_bytes))
            } else {
                String::new()
            };
            if m.method == METHOD_COPY {
                extra.push("copied out of iCloud (clone, compare, rename into place, original removed last)".to_string());
            }
            l.push(format!(
                "  {}: {} -> {} ({}, {} files{dataless}){}",
                kind_word(m.kind),
                m.from,
                m.to,
                size(m.bytes),
                m.files,
                if extra.is_empty() {
                    String::new()
                } else {
                    format!("; {}", extra.join("; "))
                }
            ));
            l.push(format!("    {}", m.why));
        }
    }
    if !plan.mkdirs.is_empty() {
        l.push(format!("Folders made: {}", plan.mkdirs.join(", ")));
    }
    if !plan.beside_vault.is_empty() {
        let joined = plan
            .moves
            .iter()
            .any(|m| plan.beside_vault.contains(&m.from));
        l.push(format!(
            "Folders beside the vault ({}){}:",
            plan.beside_vault.len(),
            if joined {
                ""
            } else {
                ", not moved (--with-vault-siblings moves them into Projects)"
            }
        ));
        for b in &plan.beside_vault {
            l.push(format!("  {b}"));
        }
    }
    if !plan.stores.is_empty() {
        l.push(format!("Stores renamed ({}):", plan.stores.len()));
        for st in plan.stores.iter().take(40) {
            l.push(format!("  {}: {} -> {}", st.what, st.from, st.to));
        }
        if plan.stores.len() > 40 {
            l.push(format!("  ... and {} more", plan.stores.len() - 40));
        }
    }
    let cols: Vec<&ColumnRewrite> = plan.columns.iter().filter(|c| c.rows > 0).collect();
    if !cols.is_empty() {
        l.push("Sessions database rows rewritten:".into());
        for c in cols {
            l.push(format!("  {}.{}: {} rows", c.table, c.column, c.rows));
        }
    }
    if !plan.files.is_empty() {
        l.push(format!("Files rewritten ({}):", plan.files.len()));
        for f in plan.files.iter().take(60) {
            l.push(format!("  {}: {} ({} changes)", f.what, f.path, f.changes));
        }
        if plan.files.len() > 60 {
            l.push(format!("  ... and {} more", plan.files.len() - 60));
        }
    }
    if !plan.ids.is_empty() {
        l.push(format!("Workspace ids that change ({}):", plan.ids.len()));
        for i in plan.ids.iter().take(40) {
            l.push(format!(
                "  {} -> {} ({}{})",
                i.old,
                i.new,
                i.project_to,
                if i.has_workspace {
                    ", workspace folder renamed"
                } else {
                    ""
                }
            ));
        }
    }
    if !plan.orphans.is_empty() {
        l.push(format!(
            "Workspace folders with no connected project ({}): {}",
            plan.orphans.len(),
            plan.orphans.join(", ")
        ));
    }
    if !plan.collisions.is_empty() {
        l.push("Collisions:".into());
        for c in &plan.collisions {
            let w = c
                .winner
                .as_ref()
                .map(|w| format!(" Kept in Projects: {w}."))
                .unwrap_or_default();
            l.push(format!("  {}: {}.{w}", c.kind, c.detail));
        }
    }
    let d = &plan.dataless;
    l.push(format!(
        "iCloud-evicted (dataless) files: {}{}{}",
        d.total,
        if d.partial { " (partial count)" } else { "" },
        d.file_provider_domain
            .as_ref()
            .map(|f| format!("; File Provider domain {f}"))
            .unwrap_or_default()
    ));
    if d.total > 0 {
        l.push(format!(
            "  {} to download once ({} files in folders that move by copy)",
            size(d.bytes),
            d.to_download
        ));
    }
    if d.to_download > 0 {
        l.push(format!(
            "  Download first: {} files, {}, about {} (small files ~{} a second, large ~{} MB/s): run the command under Blocking",
            d.to_download,
            size(d.to_download_bytes),
            crate::move_home_icloud::duration_text(d.download_secs),
            crate::move_home_icloud::FILES_PER_SEC,
            (crate::move_home_icloud::BYTES_PER_SEC / 1048576.0).round()
        ));
    }
    if !d.copy_folders.is_empty() {
        l.push(format!(
            "  Folders in iCloud, moved by copy ({}), about {} for the copies: {}",
            d.copy_folders.len(),
            crate::move_home_icloud::duration_text(d.copy_secs),
            d.copy_folders.join(", ")
        ));
    }
    for r in &d.roots {
        let top: Vec<String> = r.top.iter().map(|(n, c)| format!("{n} {c}")).collect();
        l.push(format!(
            "  {}: {} ({}) ({})",
            r.root,
            r.count,
            size(r.bytes),
            top.join(", ")
        ));
        for sm in r.sample.iter().take(3) {
            l.push(format!("    e.g. {sm}"));
        }
    }
    l.push(format!(
        "Same volume: {}; {} moved; {} free.",
        if plan.same_volume {
            "yes (renames, and copies for folders in iCloud)"
        } else {
            "no"
        },
        size(plan.total_bytes),
        size(plan.free_bytes)
    ));
    if !plan.repos.is_empty() {
        l.push("Git worktree repairs (never prune):".into());
        for r in &plan.repos {
            l.push(format!(
                "  {} -> {}: {} worktrees, {} already broken{}",
                r.repo_from,
                r.repo_to,
                r.worktrees,
                r.broken,
                r.skipped
                    .as_ref()
                    .map(|s| format!(", skipped: {s}"))
                    .unwrap_or_default()
            ));
        }
    }
    if !plan.agents.is_empty() {
        l.push("Agent stores:".into());
        for a in &plan.agents {
            l.push(format!(
                "  {} ({}): {} changes, {} merges, {} dead keys reported",
                a.agent, a.path, a.changes, a.merges, a.dead
            ));
            for amb in a.ambiguous.iter().take(10) {
                l.push(format!("    left as it is: {amb}"));
            }
        }
    }
    for (title, items) in [
        ("LaunchAgents", &plan.launch_agents),
        ("Scripts", &plan.scripts),
    ] {
        if !items.is_empty() {
            l.push(format!("{title}:"));
            for i in items {
                l.push(format!(
                    "  {} ({}): {}{}",
                    i.label,
                    i.path,
                    i.mentions.join(", "),
                    if i.on_plan {
                        "; rewritten"
                    } else {
                        "; not on the plan"
                    }
                ));
            }
        }
    }
    if !plan.processes.is_empty() {
        l.push("Processes that may hold these folders:".into());
        for p in &plan.processes {
            l.push(format!("  {p}"));
        }
    }
    l.push(format!(
        "Backup: {} {}",
        match plan.backup.covered {
            Some(true) => "covered.",
            Some(false) => "not covered.",
            None => "unknown.",
        },
        plan.backup.detail
    ));
    let blocking: Vec<&Blocker> = plan.blockers.iter().filter(|b| b.blocking).collect();
    let warnings: Vec<&Blocker> = plan.blockers.iter().filter(|b| !b.blocking).collect();
    if !blocking.is_empty() {
        l.push("Blocking:".into());
        for b in blocking {
            l.push(format!("  {}: {}", b.kind, b.detail));
        }
    }
    if !warnings.is_empty() {
        l.push("Warnings:".into());
        for b in warnings {
            l.push(format!("  {}: {}", b.kind, b.detail));
        }
    }
    if !plan.notes.is_empty() {
        l.push("Notes:".into());
        for n in &plan.notes {
            l.push(format!("  {n}"));
        }
    }
    l.push(format!(
        "Ready: {}",
        if plan.is_noop() {
            "nothing to do.".to_string()
        } else if plan.ready {
            "yes.".to_string()
        } else {
            "no, see Blocking.".to_string()
        }
    ));
    l.push(format!("Plan hash: {}", plan.hash));
    l
}
