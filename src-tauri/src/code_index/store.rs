//! The index file (CC1 design 2.1): `files`, `names`, `nodes`, `edges`,
//! `sym_fts` (FTS5 over split identifiers) and `meta`. Written once into a
//! new file, then only read.
//!
//! CC3: the same file shape holds a repository's base index (built from a
//! commit's blobs) and a worktree's overlay, which adds `hidden` (the base
//! paths it replaces or deletes: a deleted one is a tombstone) and
//! `edge_fix` (base edges whose annotation the overlay's files change). An
//! overlay's node ids continue after the base's, so one id names one
//! definition across both. [`Index::open_overlay`] reads the pair as one
//! index: overlay rows where a path is in the overlay, else base rows.

use super::resolve::{node_ids, Res};
use super::FileParse;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

/// [`Index::open_overlay`]'s refusal of an overlay built on another base.
pub(crate) const OVERLAY_MISMATCH: &str = "the overlay was built on another base file";

/// Bumped when the tables change; part of the rebuild stamp.
pub(crate) const SCHEMA_VERSION: u32 = 1;

const SCHEMA: &str = "
CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE files(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE, lang TEXT NOT NULL,
  sha TEXT NOT NULL, bytes INTEGER NOT NULL, is_test INTEGER NOT NULL);
CREATE TABLE names(id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE);
CREATE TABLE nodes(id INTEGER PRIMARY KEY, file_id INTEGER NOT NULL, kind TEXT NOT NULL,
  name_id INTEGER NOT NULL, container TEXT NOT NULL, modpath TEXT NOT NULL,
  l0 INTEGER NOT NULL, l1 INTEGER NOT NULL, is_test INTEGER NOT NULL, command INTEGER NOT NULL);
CREATE TABLE edges(file_id INTEGER NOT NULL, src INTEGER, line INTEGER NOT NULL,
  kind TEXT NOT NULL, name_id INTEGER NOT NULL, qual TEXT NOT NULL, macro_args INTEGER NOT NULL,
  resolved INTEGER NOT NULL, target INTEGER, how TEXT NOT NULL);
CREATE VIRTUAL TABLE sym_fts USING fts5(words, node_id UNINDEXED);
";

/// The overlay's own tables (CC3).
const OVERLAY_SCHEMA: &str = "
CREATE TABLE hidden(path TEXT PRIMARY KEY);
CREATE TABLE edge_fix(rid INTEGER PRIMARY KEY, target INTEGER, how TEXT NOT NULL);
CREATE INDEX edge_fix_target ON edge_fix(target);
";

const INDEXES: &str = "
CREATE INDEX edges_name ON edges(name_id);
CREATE INDEX edges_src ON edges(src);
CREATE INDEX edges_target ON edges(target);
CREATE INDEX nodes_name ON nodes(name_id);
CREATE INDEX nodes_file ON nodes(file_id);
";

/// What a build records beside the rows.
#[derive(Debug, Clone, Default)]
pub(crate) struct Meta {
    pub head: String,
    pub stamp: String,
    pub indexed_at: String,
    pub skipped: Vec<String>,
    /// Paths that resolve outside the root (skipped).
    pub outside: Vec<String>,
    /// The first node id is this plus one (an overlay's continue after its
    /// base's).
    pub node_offset: i64,
    /// Replaces the computed fingerprint (an overlay's covers base and
    /// overlay files together).
    pub fingerprint: Option<String>,
    /// More `meta` rows; one named like a standard row replaces it.
    pub extra: Vec<(&'static str, String)>,
    /// An overlay's `hidden` paths and `edge_fix` rows.
    pub overlay: Option<OverlayRows>,
}

/// What an overlay records beside its own files' rows.
#[derive(Debug, Clone, Default)]
pub(crate) struct OverlayRows {
    /// Base paths the overlay replaces or deletes.
    pub hidden: Vec<String>,
    /// `(base edge rowid, target, how)` for base edges re-resolved against
    /// the merged definitions whose annotation changed.
    pub fixes: Vec<(i64, Option<i64>, String)>,
}

/// A build's counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Built {
    pub files: usize,
    pub nodes: usize,
    pub edges: usize,
    pub skipped: Vec<String>,
    pub timed_out: Vec<String>,
}

/// `valid_skill_name` is `valid skill name valid_skill_name`;
/// `AttachedCaps` is `attached caps attachedcaps`.
pub(crate) fn split_ident(name: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let boundary = c == '_'
            || (c.is_uppercase()
                && i > 0
                && (chars[i - 1].is_lowercase()
                    || chars.get(i + 1).is_some_and(|n| n.is_lowercase())));
        if boundary && !cur.is_empty() {
            words.push(std::mem::take(&mut cur));
        }
        if c != '_' {
            cur.extend(c.to_lowercase());
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    let whole = name.to_lowercase().replace('_', "");
    if words.len() > 1 || words.first() != Some(&whole) {
        words.push(whole);
    }
    words.join(" ")
}

/// Write `files` and their annotations into the new file `out`.
pub(crate) fn write(
    out: &Path,
    files: &[FileParse],
    resolved: &[Vec<Res>],
    meta: &Meta,
) -> Result<Built, String> {
    let fail = |e: rusqlite::Error| format!("could not write the code index: {e}");
    let mut conn = Connection::open(out).map_err(fail)?;
    conn.execute_batch("PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF;")
        .map_err(fail)?;
    conn.execute_batch(SCHEMA).map_err(fail)?;
    if meta.overlay.is_some() {
        conn.execute_batch(OVERLAY_SCHEMA).map_err(fail)?;
    }
    let tx = conn.transaction().map_err(fail)?;
    let mut fingerprint = Vec::new();
    let mut built = Built {
        files: files.len(),
        nodes: 0,
        edges: 0,
        skipped: meta.skipped.clone(),
        timed_out: Vec::new(),
    };
    {
        let mut names: HashMap<String, i64> = HashMap::new();
        let mut name_stmt = tx
            .prepare("INSERT INTO names(id, text) VALUES (?1, ?2)")
            .map_err(fail)?;
        let mut intern = |text: &str| -> Result<i64, rusqlite::Error> {
            if let Some(id) = names.get(text) {
                return Ok(*id);
            }
            let id = names.len() as i64 + 1;
            name_stmt.execute(params![id, text])?;
            names.insert(text.to_string(), id);
            Ok(id)
        };
        let mut file_stmt = tx
            .prepare(
                "INSERT INTO files(id, path, lang, sha, bytes, is_test) VALUES (?1,?2,?3,?4,?5,?6)",
            )
            .map_err(fail)?;
        let mut node_stmt = tx
            .prepare("INSERT INTO nodes(id, file_id, kind, name_id, container, modpath, l0, l1, is_test, command) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")
            .map_err(fail)?;
        let mut fts_stmt = tx
            .prepare("INSERT INTO sym_fts(words, node_id) VALUES (?1, ?2)")
            .map_err(fail)?;
        let mut edge_stmt = tx
            .prepare("INSERT INTO edges(file_id, src, line, kind, name_id, qual, macro_args, resolved, target, how) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")
            .map_err(fail)?;
        let firsts: Vec<i64> = node_ids(files)
            .into_iter()
            .map(|f| f + meta.node_offset)
            .collect();
        for (fi, f) in files.iter().enumerate() {
            let file_id = fi as i64 + 1;
            fingerprint.push(format!("{}\0{}\n", f.path, f.sha));
            if f.parsed.timed_out {
                built.timed_out.push(f.path.clone());
            }
            file_stmt
                .execute(params![
                    file_id,
                    f.path,
                    f.lang.as_str(),
                    f.sha,
                    f.bytes as i64,
                    f.test
                ])
                .map_err(fail)?;
            for (di, d) in f.parsed.defs.iter().enumerate() {
                let id = firsts[fi] + di as i64;
                let name_id = intern(&d.name).map_err(fail)?;
                node_stmt
                    .execute(params![
                        id,
                        file_id,
                        d.kind,
                        name_id,
                        d.container,
                        d.modpath,
                        d.l0,
                        d.l1,
                        d.test || f.test,
                        d.command
                    ])
                    .map_err(fail)?;
                fts_stmt
                    .execute(params![split_ident(&d.name), id])
                    .map_err(fail)?;
                built.nodes += 1;
            }
            for (ei, e) in f.parsed.edges.iter().enumerate() {
                let res = &resolved[fi][ei];
                let name_id = intern(&e.name).map_err(fail)?;
                let src = e.src.map(|s| firsts[fi] + s as i64);
                edge_stmt
                    .execute(params![
                        file_id,
                        src,
                        e.line,
                        e.kind,
                        name_id,
                        e.qual,
                        e.macro_args,
                        res.target.is_some(),
                        res.target,
                        res.how
                    ])
                    .map_err(fail)?;
                built.edges += 1;
            }
        }
        let mut meta_stmt = tx
            .prepare("INSERT INTO meta(key, value) VALUES (?1, ?2)")
            .map_err(fail)?;
        let fp = meta
            .fingerprint
            .clone()
            .unwrap_or_else(|| super::short_sha(fingerprint.concat().as_bytes()));
        let mut rows: Vec<(&str, String)> = vec![
            ("head", meta.head.clone()),
            ("fingerprint", fp),
            ("stamp", meta.stamp.clone()),
            ("indexedAt", meta.indexed_at.clone()),
            ("parsers", super::parse::PARSERS.to_string()),
            ("schema", SCHEMA_VERSION.to_string()),
            ("skipped", serde_json::json!(meta.skipped).to_string()),
            ("timedOut", serde_json::json!(built.timed_out).to_string()),
            ("outsideRoot", serde_json::json!(meta.outside).to_string()),
        ];
        for (k, v) in &meta.extra {
            match rows.iter_mut().find(|(key, _)| key == k) {
                Some(row) => row.1 = v.clone(),
                None => rows.push((k, v.clone())),
            }
        }
        for (k, v) in rows {
            meta_stmt.execute(params![k, v]).map_err(fail)?;
        }
        if let Some(ov) = &meta.overlay {
            let mut hide = tx
                .prepare("INSERT INTO hidden(path) VALUES (?1)")
                .map_err(fail)?;
            for p in &ov.hidden {
                hide.execute(params![p]).map_err(fail)?;
            }
            let mut fix = tx
                .prepare("INSERT INTO edge_fix(rid, target, how) VALUES (?1, ?2, ?3)")
                .map_err(fail)?;
            for (rid, target, how) in &ov.fixes {
                fix.execute(params![rid, target, how]).map_err(fail)?;
            }
        }
    }
    tx.commit().map_err(fail)?;
    conn.execute_batch(INDEXES).map_err(fail)?;
    Ok(built)
}

/// SQLite's `quick_check` of the file at `path` (CC3: an imported base's
/// private copy must pass it). Read-write: FTS5's own check refuses a
/// read-only connection.
pub(crate) fn quick_check(path: &Path) -> Result<(), String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| format!("it is not an index file: {e}"))?;
    untrusted(&conn).map_err(|e| format!("it is not an index file: {e}"))?;
    let v: String = conn
        .query_row("PRAGMA main.quick_check", [], |r| r.get(0))
        .map_err(|e| format!("it is not an index file: {e}"))?;
    if v == "ok" {
        Ok(())
    } else {
        Err(format!("the file failed SQLite's check: {v}"))
    }
}

/// CC3 review: a cache file's schema never runs a function or virtual
/// table it names from a view or trigger.
fn untrusted(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA trusted_schema=OFF;")
}

/// `(type, name, tbl_name, sql)` of every schema object, sorted.
fn schema_rows(conn: &Connection, db: &str) -> rusqlite::Result<Vec<[String; 4]>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT type, name, tbl_name, coalesce(sql, '') FROM {db}.sqlite_master ORDER BY type, name"
    ))?;
    let rows = stmt.query_map([], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?]))?;
    rows.collect()
}

/// CC3 review: the structural checks of a base built elsewhere (an
/// import's private copy). Its schema objects are exactly the ones
/// [`write`] makes for a base (no view, no trigger, no other table or
/// index, each with the same SQL); definition ids run 1..=n with each
/// file's contiguous; every edge's file, source and target and every
/// definition's file and name exist. Overlay arithmetic on its ids then
/// stays in range.
pub(crate) fn check_base_tables(path: &Path) -> Result<(), String> {
    let bad = |e: rusqlite::Error| format!("it is not an index file: {e}");
    let want = {
        let mem = Connection::open_in_memory().map_err(bad)?;
        mem.execute_batch(SCHEMA).map_err(bad)?;
        mem.execute_batch(INDEXES).map_err(bad)?;
        schema_rows(&mem, "main").map_err(bad)?
    };
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(bad)?;
    untrusted(&conn).map_err(bad)?;
    let got = schema_rows(&conn, "main").map_err(bad)?;
    if got != want {
        let extra: Vec<String> = got
            .iter()
            .filter(|g| !want.contains(g))
            .map(|g| format!("{} `{}`", g[0], g[1]))
            .collect();
        return Err(format!(
            "its tables are not a base index's (unexpected or changed: {})",
            if extra.is_empty() {
                "a table is missing".to_string()
            } else {
                extra.join(", ")
            }
        ));
    }
    let count =
        |sql: &str| -> Result<i64, String> { conn.query_row(sql, [], |r| r.get(0)).map_err(bad) };
    let n = count("SELECT count(*) FROM nodes")?;
    let (lo, hi) = conn
        .query_row(
            "SELECT coalesce(min(id), 1), coalesce(max(id), 0) FROM nodes",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
        )
        .map_err(bad)?;
    if lo != 1 || hi != n {
        return Err(format!(
            "its definition ids are not 1 to {n} (they run {lo} to {hi})"
        ));
    }
    let checks = [
        (
            "SELECT count(*) FROM (SELECT min(id) lo, max(id) hi, count(*) c FROM nodes GROUP BY file_id) WHERE hi - lo + 1 != c",
            "a file's definitions are not contiguous",
        ),
        (
            "SELECT count(*) FROM nodes WHERE file_id NOT IN (SELECT id FROM files) OR name_id NOT IN (SELECT id FROM names)",
            "a definition names a missing file or name",
        ),
        (
            "SELECT count(*) FROM edges WHERE file_id NOT IN (SELECT id FROM files) OR name_id NOT IN (SELECT id FROM names)",
            "an edge names a missing file or name",
        ),
        (
            "SELECT count(*) FROM edges e WHERE (e.src IS NOT NULL AND (typeof(e.src) != 'integer' OR NOT EXISTS (SELECT 1 FROM nodes n WHERE n.id = e.src AND n.file_id = e.file_id))) OR (e.target IS NOT NULL AND (typeof(e.target) != 'integer' OR e.target < 1 OR e.target > (SELECT count(*) FROM nodes)))",
            "an edge's source or target is not a definition",
        ),
    ];
    for (sql, why) in checks {
        if count(sql)? != 0 {
            return Err(format!("its rows are inconsistent: {why}"));
        }
    }
    Ok(())
}

/// One `meta` value of the index at `path`, if it opens.
pub(crate) fn read_meta(path: &Path, key: &str) -> Option<String> {
    Index::open(path).ok()?.meta(key)
}

/// A definition row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NodeRow {
    pub id: i64,
    pub path: String,
    pub lang: String,
    pub sha: String,
    pub kind: String,
    pub name: String,
    pub container: String,
    pub modpath: String,
    pub l0: i64,
    pub l1: i64,
    pub test: bool,
    pub command: bool,
}

/// An edge row with its file and the definition it sits in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EdgeRow {
    pub path: String,
    pub lang: String,
    pub sha: String,
    pub file_test: bool,
    pub src: Option<NodeRow>,
    pub line: i64,
    pub kind: String,
    pub name: String,
    pub qual: String,
    pub target: Option<i64>,
    pub how: String,
    /// Its row in its file (the base's or the overlay's): the order of
    /// edges on one line.
    pub rid: i64,
}

impl EdgeRow {
    pub(crate) fn test(&self) -> bool {
        self.file_test || self.src.as_ref().is_some_and(|s| s.test)
    }
}

/// A file row with its definition count.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FileRow {
    pub path: String,
    pub lang: String,
    pub sha: String,
    pub bytes: i64,
    pub test: bool,
    pub defs: i64,
}

const NODE_COLS: &str = "n.id, f.path, f.lang, f.sha, n.kind, nm.text, n.container, n.modpath, n.l0, n.l1, n.is_test, n.command";

fn node_row(r: &rusqlite::Row, at: usize) -> rusqlite::Result<NodeRow> {
    Ok(NodeRow {
        id: r.get(at)?,
        path: r.get(at + 1)?,
        lang: r.get(at + 2)?,
        sha: r.get(at + 3)?,
        kind: r.get(at + 4)?,
        name: r.get(at + 5)?,
        container: r.get(at + 6)?,
        modpath: r.get(at + 7)?,
        l0: r.get(at + 8)?,
        l1: r.get(at + 9)?,
        test: r.get(at + 10)?,
        command: r.get(at + 11)?,
    })
}

/// A stored edge as the overlay build re-resolves it (CC3).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawEdge {
    pub rid: i64,
    pub path: String,
    pub src: Option<i64>,
    pub line: i64,
    pub kind: String,
    pub name: String,
    pub qual: String,
    pub macro_args: bool,
    pub target: Option<i64>,
    pub how: String,
}

/// One side of a query: a plain index, or the base or overlay half of an
/// overlaid one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arm {
    Plain,
    Base,
    Over,
}

impl Arm {
    /// The schema its tables are in.
    fn s(self) -> &'static str {
        match self {
            Arm::Over => "ov",
            _ => "main",
        }
    }

    /// The condition that drops the base files the overlay replaces.
    fn hide(self, files: &str) -> String {
        match self {
            Arm::Base => format!(" AND {files}.path NOT IN (SELECT path FROM ov.hidden)"),
            _ => String::new(),
        }
    }
}

/// An open index, read only: one file, or a base with a worktree's overlay
/// attached (CC3).
pub(crate) struct Index {
    conn: Connection,
    overlay: bool,
}

fn qerr(e: rusqlite::Error) -> String {
    format!("the code index could not be read: {e}")
}

impl Index {
    pub(crate) fn open(path: &Path) -> Result<Index, String> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(qerr)?;
        untrusted(&conn).map_err(qerr)?;
        Ok(Index {
            conn,
            overlay: false,
        })
    }

    /// The base index `base` read through the overlay `overlay` (both read
    /// only): overlay rows where a path is in the overlay, else base rows.
    ///
    /// CC3 review: the overlay must have been built on this very base (its
    /// `baseIdentity` meta equals [`Index::base_identity`] of the file just
    /// opened), so a base swapped between the overlay check and this attach
    /// (a prune and rebuild, an import) is refused rather than read with
    /// node offsets that belong to another file.
    pub(crate) fn open_overlay(base: &Path, overlay: &Path) -> Result<Index, String> {
        let mut index = Index::open(base)?;
        index
            .conn
            .execute(
                "ATTACH DATABASE ?1 AS ov",
                [overlay.to_string_lossy().as_ref()],
            )
            .map_err(qerr)?;
        let built_on: Option<String> = index
            .conn
            .query_row(
                "SELECT value FROM ov.meta WHERE key = 'baseIdentity'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(qerr)?;
        if built_on != Some(index.base_identity()?) {
            return Err(OVERLAY_MISMATCH.into());
        }
        index.overlay = true;
        Ok(index)
    }

    /// CC3 review: what identifies this base's contents to an overlay built
    /// on it: its highest definition id and edge rowid (the offsets an
    /// overlay's rows depend on) and its stamp, fingerprint and timed-out
    /// files. Cheap: two index lookups and three meta rows, all from the
    /// file this connection has open.
    pub(crate) fn base_identity(&self) -> Result<String, String> {
        let (nodes, edges): (i64, i64) = self
            .conn
            .query_row(
                "SELECT (SELECT coalesce(max(id), 0) FROM main.nodes),
                        (SELECT coalesce(max(rowid), 0) FROM main.edges)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(qerr)?;
        let meta = |k: &str| {
            self.conn
                .query_row("SELECT value FROM main.meta WHERE key = ?1", [k], |r| {
                    r.get::<_, String>(0)
                })
                .optional()
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let text = format!(
            "{nodes}\0{edges}\0{}\0{}\0{}",
            meta("stamp"),
            meta("fingerprint"),
            meta("timedOut")
        );
        Ok(super::short_sha(text.as_bytes()))
    }

    /// Whether this reads a base through an overlay.
    pub(crate) fn overlaid(&self) -> bool {
        self.overlay
    }

    fn arms(&self) -> &'static [Arm] {
        if self.overlay {
            &[Arm::Base, Arm::Over]
        } else {
            &[Arm::Plain]
        }
    }

    /// One `meta` value (an overlay's first, then its base's).
    pub(crate) fn meta(&self, key: &str) -> Option<String> {
        let one = |s: &str| {
            self.conn
                .query_row(
                    &format!("SELECT value FROM {s}.meta WHERE key = ?1"),
                    [key],
                    |r| r.get(0),
                )
                .optional()
                .ok()
                .flatten()
        };
        if self.overlay {
            one("ov").or_else(|| one("main"))
        } else {
            one("main")
        }
    }

    /// Definitions matching `cond` (`{s}` is the arm's schema) in `tail`'s
    /// order, from each arm, the arms concatenated.
    fn nodes(
        &self,
        cond: &str,
        tail: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<NodeRow>, String> {
        let mut out = Vec::new();
        for arm in self.arms() {
            let s = arm.s();
            let sql = format!(
                "SELECT {NODE_COLS} FROM {s}.nodes n JOIN {s}.files f ON f.id = n.file_id JOIN {s}.names nm ON nm.id = n.name_id WHERE ({}){} {tail}",
                cond.replace("{s}", s),
                arm.hide("f")
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(qerr)?;
            let rows = stmt
                .query_map(args, |r| node_row(r, 0))
                .map_err(qerr)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(qerr)?;
            out.extend(rows);
        }
        Ok(out)
    }

    /// Definitions named `name`, by path and line.
    pub(crate) fn defs_named(&self, name: &str) -> Result<Vec<NodeRow>, String> {
        let mut rows = self.nodes("nm.text = ?1", "ORDER BY f.path, n.l0, n.id", &[&name])?;
        if self.overlay {
            rows.sort_by(|a, b| (&a.path, a.l0, a.id).cmp(&(&b.path, b.l0, b.id)));
        }
        Ok(rows)
    }

    /// Definitions of kind `kind`, by path and line (CC5: the `mod`
    /// declarations `affected_tests` reads `#[path]` attributes above).
    pub(crate) fn defs_of_kind(&self, kind: &str) -> Result<Vec<NodeRow>, String> {
        let mut rows = self.nodes("n.kind = ?1", "ORDER BY f.path, n.l0, n.id", &[&kind])?;
        if self.overlay {
            rows.sort_by(|a, b| (&a.path, a.l0, a.id).cmp(&(&b.path, b.l0, b.id)));
        }
        Ok(rows)
    }

    pub(crate) fn node(&self, id: i64) -> Result<Option<NodeRow>, String> {
        Ok(self.nodes("n.id = ?1", "", &[&id])?.pop())
    }

    /// Definitions of one file, by line.
    pub(crate) fn defs_in(&self, path: &str) -> Result<Vec<NodeRow>, String> {
        let mut rows = self.nodes("f.path = ?1", "ORDER BY n.l0, n.id", &[&path])?;
        if self.overlay {
            rows.sort_by_key(|r| (r.l0, r.id));
        }
        Ok(rows)
    }

    /// Definitions whose split name matches every word of `query` (each a
    /// prefix), best first.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Result<Vec<NodeRow>, String> {
        let words: Vec<String> = split_ident(query)
            .split_whitespace()
            .map(|w| {
                w.chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>()
            })
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{w}\"*"))
            .collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let fts = words.join(" AND ");
        let mut rows = self.nodes(
            "n.id IN (SELECT node_id FROM {s}.sym_fts WHERE sym_fts MATCH ?1)",
            &format!("ORDER BY (nm.text = ?2) DESC, length(nm.text), f.path, n.l0 LIMIT {limit}"),
            &[&fts, &query],
        )?;
        if self.overlay {
            rows.sort_by(|a, b| {
                (a.name != query, a.name.chars().count(), &a.path, a.l0, a.id).cmp(&(
                    b.name != query,
                    b.name.chars().count(),
                    &b.path,
                    b.l0,
                    b.id,
                ))
            });
            rows.truncate(limit);
        }
        Ok(rows)
    }

    /// Every distinct definition name (for did-you-mean).
    pub(crate) fn def_names(&self) -> Result<Vec<String>, String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for arm in self.arms() {
            let s = arm.s();
            let sql = format!(
                "SELECT DISTINCT nm.text FROM {s}.nodes n JOIN {s}.names nm ON nm.id = n.name_id JOIN {s}.files f ON f.id = n.file_id WHERE 1{}",
                arm.hide("f")
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(qerr)?;
            let rows = stmt
                .query_map([], |r| r.get(0))
                .map_err(qerr)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(qerr)?;
            for name in rows {
                if seen.insert(name.clone()) {
                    out.push(name);
                }
            }
        }
        Ok(out)
    }

    /// Edges matching `cond` on each arm in `arms`, each in path, line and
    /// row order. On the base arm of an overlaid index, `x` is the
    /// overlay's `edge_fix` row for the edge (if any), and the annotation
    /// is the fixed one.
    fn edges_on(
        &self,
        arms: &[(Arm, &str)],
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<EdgeRow>, String> {
        let mut out = Vec::new();
        for (arm, cond) in arms {
            let s = arm.s();
            let (fix, target, how) = if *arm == Arm::Base {
                (
                    " LEFT JOIN ov.edge_fix x ON x.rid = e.rowid",
                    "CASE WHEN x.rid IS NULL THEN e.target ELSE x.target END",
                    "CASE WHEN x.rid IS NULL THEN e.how ELSE x.how END",
                )
            } else {
                ("", "e.target", "e.how")
            };
            let sql = format!(
                "SELECT f.path, f.lang, f.sha, f.is_test, e.line, e.kind, en.text, e.qual, {target}, {how},
                        s.id, sf.path, sf.lang, sf.sha, s.kind, sn.text, s.container, s.modpath, s.l0, s.l1, s.is_test, s.command, e.rowid
                 FROM {s}.edges e JOIN {s}.files f ON f.id = e.file_id JOIN {s}.names en ON en.id = e.name_id{fix}
                 LEFT JOIN {s}.nodes s ON s.id = e.src LEFT JOIN {s}.files sf ON sf.id = s.file_id
                 LEFT JOIN {s}.names sn ON sn.id = s.name_id
                 WHERE ({cond}){} ORDER BY f.path, e.line, e.rowid",
                arm.hide("f")
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(qerr)?;
            let rows = stmt
                .query_map(args, |r| {
                    let src_id: Option<i64> = r.get(10)?;
                    Ok(EdgeRow {
                        path: r.get(0)?,
                        lang: r.get(1)?,
                        sha: r.get(2)?,
                        file_test: r.get(3)?,
                        line: r.get(4)?,
                        kind: r.get(5)?,
                        name: r.get(6)?,
                        qual: r.get(7)?,
                        target: r.get(8)?,
                        how: r.get(9)?,
                        rid: r.get(22)?,
                        src: match src_id {
                            Some(_) => Some(node_row(r, 10)?),
                            None => None,
                        },
                    })
                })
                .map_err(qerr)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(qerr)?;
            out.extend(rows);
        }
        if arms.len() > 1 {
            // One path is in one arm's file only, so its rows compare.
            out.sort_by(|a, b| (&a.path, a.line, a.rid).cmp(&(&b.path, b.line, b.rid)));
        }
        Ok(out)
    }

    /// `cond` on every arm.
    fn edges(&self, cond: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<EdgeRow>, String> {
        let arms: Vec<(Arm, &str)> = self.arms().iter().map(|a| (*a, cond)).collect();
        self.edges_on(&arms, args)
    }

    /// Every edge named `name` (the name scan).
    pub(crate) fn edges_named(&self, name: &str) -> Result<Vec<EdgeRow>, String> {
        self.edges("en.text = ?1", &[&name])
    }

    /// Edges inside definition `src`.
    pub(crate) fn edges_from(&self, src: i64) -> Result<Vec<EdgeRow>, String> {
        self.edges("e.src = ?1", &[&src])
    }

    /// Edges resolved to definition `target` (on an overlaid index: base
    /// edges by their fixed annotation, so each query keeps an index).
    pub(crate) fn edges_to(&self, target: i64) -> Result<Vec<EdgeRow>, String> {
        if !self.overlay {
            return self.edges("e.target = ?1", &[&target]);
        }
        self.edges_on(
            &[
                (Arm::Base, "e.target = ?1 AND x.rid IS NULL"),
                (Arm::Base, "x.target = ?1"),
                (Arm::Over, "e.target = ?1"),
            ],
            &[&target],
        )
    }

    /// One file by path.
    pub(crate) fn file(&self, path: &str) -> Result<Option<FileRow>, String> {
        Ok(self.files_where("f.path = ?1", &[&path])?.pop())
    }

    /// Files under folder `prefix` (`""`: all), by path.
    pub(crate) fn files_under(&self, prefix: &str) -> Result<Vec<FileRow>, String> {
        if prefix.is_empty() {
            return self.files_where("1", &[]);
        }
        let like = format!("{}/%", prefix.replace('%', "\\%").replace('_', "\\_"));
        self.files_where("f.path LIKE ?1 ESCAPE '\\'", &[&like])
    }

    fn files_where(
        &self,
        filter: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<FileRow>, String> {
        let mut out = Vec::new();
        for arm in self.arms() {
            let s = arm.s();
            let sql = format!(
                "SELECT f.path, f.lang, f.sha, f.bytes, f.is_test,
                        (SELECT count(*) FROM {s}.nodes n WHERE n.file_id = f.id)
                 FROM {s}.files f WHERE ({filter}){} ORDER BY f.path",
                arm.hide("f")
            );
            let mut stmt = self.conn.prepare_cached(&sql).map_err(qerr)?;
            let rows = stmt
                .query_map(args, |r| {
                    Ok(FileRow {
                        path: r.get(0)?,
                        lang: r.get(1)?,
                        sha: r.get(2)?,
                        bytes: r.get(3)?,
                        test: r.get(4)?,
                        defs: r.get(5)?,
                    })
                })
                .map_err(qerr)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(qerr)?;
            out.extend(rows);
        }
        if self.overlay {
            out.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Ok(out)
    }

    // ── What an overlay build reads from its base (CC3) ─────────────────

    /// The highest node id (0 when there is none).
    pub(crate) fn max_node_id(&self) -> Result<i64, String> {
        self.conn
            .query_row("SELECT coalesce(max(id), 0) FROM main.nodes", [], |r| {
                r.get(0)
            })
            .map_err(qerr)
    }

    /// Every definition, by id (each file's are contiguous, in order).
    pub(crate) fn all_defs(&self) -> Result<Vec<NodeRow>, String> {
        self.nodes("1", "ORDER BY n.id", &[])
    }

    /// Stored edges matching `cond` (`e`, `f` and `en` in scope), by row.
    pub(crate) fn raw_edges(
        &self,
        cond: &str,
        args: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<RawEdge>, String> {
        let sql = format!(
            "SELECT e.rowid, f.path, e.src, e.line, e.kind, en.text, e.qual, e.macro_args, e.target, e.how
             FROM main.edges e JOIN main.files f ON f.id = e.file_id JOIN main.names en ON en.id = e.name_id
             WHERE {cond} ORDER BY e.rowid"
        );
        let mut stmt = self.conn.prepare_cached(&sql).map_err(qerr)?;
        let rows = stmt
            .query_map(args, |r| {
                Ok(RawEdge {
                    rid: r.get(0)?,
                    path: r.get(1)?,
                    src: r.get(2)?,
                    line: r.get(3)?,
                    kind: r.get(4)?,
                    name: r.get(5)?,
                    qual: r.get(6)?,
                    macro_args: r.get(7)?,
                    target: r.get(8)?,
                    how: r.get(9)?,
                })
            })
            .map_err(qerr)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(qerr)?;
        Ok(rows)
    }
}
