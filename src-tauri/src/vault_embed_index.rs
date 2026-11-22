//! Semantic-index cache foundation for future embeddings.
//!
//! This is deliberately the same shape as `vault_search_index`: a derived
//! SQLite cache kept OUTSIDE the vault, keyed by canonical vault path, with
//! best-effort reconciliation by `(mtime, size)`. This first slice stores the
//! section-aware chunks and citation spans only; the actual model runtime and
//! vector table land in a later phase. Keeping this layer pure/SQLite-only lets
//! us prove the lifecycle before adding ONNX/model signing risk.
//!
//! Nothing here is user-facing yet. The cache is rebuildable, and any failure
//! means callers can fall back to today's keyword search.

use crate::embed_chunk::{chunk_note, Chunk, ChunkOpts};
use crate::types::{Note, SearchResult};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;

// v2 adds the `vectors` table on top of the v1 chunk store. v3 adds
// frontmatter chunks (`chunks.in_frontmatter`, HF03): a task, project, person
// or goal card, and any note with an empty body, is embedded from its
// frontmatter fields too. A version bump drops + rebuilds the derived cache
// (re-chunk + re-embed), so existing installs pay a one-time backfill on first
// launch of the new build. A NEWER stored version (an older build installed
// over a newer one) is never dropped or written: `open` returns `None`, so
// semantic search stays off and the newer build finds its index intact (OF25).
pub(crate) const SCHEMA_VERSION: i64 = 3;
const DEFAULT_MAX_CHARS: usize = 1000;
const DEFAULT_OVERLAP_CHARS: usize = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedChunk {
    pub path: String,
    pub title: String,
    pub kind: String,
    pub ord: usize,
    pub heading: Option<String>,
    pub byte_start: usize,
    pub byte_end: usize,
    pub text: String,
    /// The span is the note's frontmatter (offsets into the whole file), not
    /// its body.
    pub in_frontmatter: bool,
}

/// One note's best semantic chunk for a query (see [`EmbedIndex::note_hits`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChunkHit {
    pub path: String,
    pub kind: String,
    pub mtime: String,
    /// Byte span into the note's body (frontmatter removed; HTML stripped
    /// for `.html` notes), as `embed_chunk::Chunk` records it — or, when
    /// `in_frontmatter`, into the whole file, inside its frontmatter block.
    pub byte_start: usize,
    pub byte_end: usize,
    pub in_frontmatter: bool,
    /// Cosine similarity (the vectors are L2-normalized).
    pub score: f32,
}

pub struct EmbedIndex {
    conn: Connection,
    /// rel-path -> (mtime string, size bytes, chunk count).
    meta: HashMap<String, (String, u64, usize)>,
    disabled: bool,
}

impl EmbedIndex {
    pub fn open(db_path: &Path) -> Option<EmbedIndex> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path).ok()?;
        if let Some(stored) = crate::vault_search_index::newer_schema(&conn, SCHEMA_VERSION) {
            // Same recovery as the keyword index's line in
            // `vault_search_index::SearchIndex::open` (OF25).
            eprintln!(
                "[embed index] semantic (embedding) index schema {stored} is newer than this \
                 build's {SCHEMA_VERSION}: the index is left as it is and semantic search stays \
                 off. If no newer build will run again (e.g. a branch build wrote it), quit Hollow \
                 and delete this vault's file in <app support>/EmbedIndex/ (<vault name>-<hash>.db, \
                 with any -wal/-shm beside it); the next launch rebuilds it (a full re-embed)"
            );
            return None;
        }
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(
            None,
            "journal_size_limit",
            crate::vault_search_index::WAL_LIMIT_BYTES,
        );
        let _ = conn.busy_timeout(std::time::Duration::from_millis(2000));
        let mut idx = EmbedIndex {
            conn,
            meta: HashMap::new(),
            disabled: false,
        };
        if !idx.ensure_schema() {
            return None;
        }
        idx.load_meta();
        Some(idx)
    }

    /// Empty the `-wal` after a sweep (`vault_search_index::checkpoint_truncate`).
    pub(crate) fn compact_wal(&self) -> bool {
        crate::vault_search_index::checkpoint_truncate(&self.conn)
    }

    fn ensure_schema(&self) -> bool {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap_or(0);
        if version > SCHEMA_VERSION {
            // `open` refuses a newer index first; never drop or downgrade one.
            return false;
        }
        if version != 0 && version != SCHEMA_VERSION {
            let _ = self.conn.execute_batch(
                "DROP TABLE IF EXISTS chunks; DROP TABLE IF EXISTS meta; \
                 DROP TABLE IF EXISTS vectors;",
            );
        }
        // `vectors`: one embedding per chunk, keyed to its (path,ord) row.
        // `dim = 0` is a TOMBSTONE — an embed was attempted and failed, recorded
        // so the chunk isn't retried forever (cleared when its content changes).
        // `vec` is little-endian f32[dim]. (No SQL `--` comments inside the
        // string: the `\` line-continuations strip the trailing newline, so a
        // comment would swallow the statement that follows it.)
        let ok = self
            .conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS meta(\
                     path TEXT PRIMARY KEY,\
                     mtime TEXT NOT NULL,\
                     size INTEGER NOT NULL,\
                     title TEXT NOT NULL,\
                     kind TEXT NOT NULL,\
                     chunk_count INTEGER NOT NULL);\
                 CREATE TABLE IF NOT EXISTS chunks(\
                     path TEXT NOT NULL,\
                     ord INTEGER NOT NULL,\
                     heading TEXT,\
                     byte_start INTEGER NOT NULL,\
                     byte_end INTEGER NOT NULL,\
                     text TEXT NOT NULL,\
                     in_frontmatter INTEGER NOT NULL DEFAULT 0,\
                     PRIMARY KEY(path, ord));\
                 CREATE INDEX IF NOT EXISTS chunks_path_idx ON chunks(path);\
                 CREATE TABLE IF NOT EXISTS vectors(\
                     path TEXT NOT NULL,\
                     ord INTEGER NOT NULL,\
                     dim INTEGER NOT NULL,\
                     vec BLOB NOT NULL,\
                     PRIMARY KEY(path, ord));\
                 CREATE INDEX IF NOT EXISTS vectors_path_idx ON vectors(path);",
            )
            .is_ok();
        if !ok {
            return false;
        }
        let _ = self
            .conn
            .pragma_update(None, "user_version", SCHEMA_VERSION);
        true
    }

    fn load_meta(&mut self) {
        self.meta.clear();
        let mut stmt = match self
            .conn
            .prepare("SELECT path, mtime, size, chunk_count FROM meta;")
        {
            Ok(s) => s,
            Err(_) => return,
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                if row.2 >= 0 && row.3 >= 0 {
                    self.meta
                        .insert(row.0, (row.1, row.2 as u64, row.3 as usize));
                }
            }
        }
    }

    pub fn reconcile_add_edit(&mut self, notes: &[Note], vault: &Path) -> bool {
        if self.disabled {
            return false;
        }
        let changed: Vec<&Note> = notes
            .iter()
            .filter(|n| {
                self.meta
                    .get(&n.path)
                    .map(|(m, sz, _)| m != &n.mtime || *sz != n.size)
                    .unwrap_or(true)
            })
            .collect();
        if changed.is_empty() {
            return true;
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return false;
        }
        for n in changed {
            let full = vault.join(&n.path);
            match std::fs::read_to_string(&full) {
                Ok(content) => {
                    if !self.upsert_locked(&n.path, &n.mtime, n.size, &content) {
                        let _ = self.conn.execute_batch("ROLLBACK;");
                        return false;
                    }
                }
                Err(_) => {
                    let _ = self.remove_locked(&n.path);
                }
            }
        }
        self.conn.execute_batch("COMMIT;").is_ok()
    }

    pub fn full_sweep(&mut self, notes: &[Note], vault: &Path) -> bool {
        if !self.reconcile_add_edit(notes, vault) {
            return false;
        }
        let current: HashSet<&String> = notes.iter().map(|n| &n.path).collect();
        let gone: Vec<String> = self
            .meta
            .keys()
            .filter(|p| !current.contains(*p))
            .cloned()
            .collect();
        if gone.is_empty() {
            return true;
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return false;
        }
        for p in gone {
            let _ = self.remove_locked(&p);
        }
        self.conn.execute_batch("COMMIT;").is_ok()
    }

    pub fn upsert(&mut self, path: &str, mtime: &str, size: u64, content: &str) {
        if self.disabled {
            return;
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return;
        }
        if self.upsert_locked(path, mtime, size, content) {
            let _ = self.conn.execute_batch("COMMIT;");
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
    }

    pub fn remove(&mut self, path: &str) {
        if self.disabled || !self.meta.contains_key(path) {
            return;
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return;
        }
        if self.remove_locked(path) {
            let _ = self.conn.execute_batch("COMMIT;");
        } else {
            let _ = self.conn.execute_batch("ROLLBACK;");
        }
    }

    fn upsert_locked(&mut self, path: &str, mtime: &str, size: u64, content: &str) -> bool {
        let prepared = prepare_chunks(path, content);
        if !self.remove_locked(path) {
            return false;
        }
        if self
            .conn
            .execute(
                "INSERT INTO meta(path, mtime, size, title, kind, chunk_count) VALUES(?1,?2,?3,?4,?5,?6);",
                rusqlite::params![
                    path,
                    mtime,
                    size as i64,
                    prepared.title,
                    prepared.kind,
                    prepared.chunks.len() as i64
                ],
            )
            .is_err()
        {
            return false;
        }
        for c in &prepared.chunks {
            if self
                .conn
                .execute(
                    "INSERT INTO chunks(path, ord, heading, byte_start, byte_end, text, \
                     in_frontmatter) VALUES(?1,?2,?3,?4,?5,?6,?7);",
                    rusqlite::params![
                        path,
                        c.ord as i64,
                        c.heading,
                        c.byte_start as i64,
                        c.byte_end as i64,
                        c.text,
                        c.in_frontmatter
                    ],
                )
                .is_err()
            {
                return false;
            }
        }
        self.meta.insert(
            path.to_string(),
            (mtime.to_string(), size, prepared.chunks.len()),
        );
        true
    }

    fn remove_locked(&mut self, path: &str) -> bool {
        if self
            .conn
            .execute("DELETE FROM chunks WHERE path=?1;", [path])
            .is_err()
        {
            return false;
        }
        // Drop the vectors alongside the chunks so an edited/removed note never
        // leaves orphaned (now-stale) embeddings; the rewritten chunks are
        // re-embedded by the next backfill pass.
        if self
            .conn
            .execute("DELETE FROM vectors WHERE path=?1;", [path])
            .is_err()
        {
            return false;
        }
        if self
            .conn
            .execute("DELETE FROM meta WHERE path=?1;", [path])
            .is_err()
        {
            return false;
        }
        self.meta.remove(path);
        true
    }

    #[cfg(test)]
    pub fn indexed_count(&self) -> usize {
        self.meta.len()
    }

    pub fn chunk_count(&self) -> usize {
        self.meta.values().map(|(_, _, n)| *n).sum()
    }

    pub fn chunks_for_path(&self, path: &str) -> Vec<IndexedChunk> {
        let mut stmt = match self.conn.prepare(
            "SELECT c.path, m.title, m.kind, c.ord, c.heading, c.byte_start, c.byte_end, c.text, \
             c.in_frontmatter \
             FROM chunks c JOIN meta m ON m.path=c.path \
             WHERE c.path=?1 ORDER BY c.ord ASC;",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([path], |r| {
            Ok(IndexedChunk {
                path: r.get(0)?,
                title: r.get(1)?,
                kind: r.get(2)?,
                ord: r.get::<_, i64>(3)?.max(0) as usize,
                heading: r.get(4)?,
                byte_start: r.get::<_, i64>(5)?.max(0) as usize,
                byte_end: r.get::<_, i64>(6)?.max(0) as usize,
                text: r.get(7)?,
                in_frontmatter: r.get(8)?,
            })
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Up to `limit` chunks with no `vectors` row yet (real OR tombstone),
    /// returned as `(path, ord, text-to-embed)`. A cheap indexed lookup so the
    /// backfill worker can pull a batch, release the lock, then embed unlocked.
    fn take_unembedded(&self, limit: usize) -> Vec<(String, usize, String)> {
        let mut stmt = match self.conn.prepare(
            "SELECT c.path, c.ord, c.text FROM chunks c \
             LEFT JOIN vectors v ON v.path=c.path AND v.ord=c.ord \
             WHERE v.path IS NULL LIMIT ?1;",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?.max(0) as usize,
                r.get::<_, String>(2)?,
            ))
        });
        match rows {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Persist embed results. Each item is `(path, ord, embedded_text, vector)`:
    /// `Some(v)` stores the f32 vector; `None` stores a TOMBSTONE (dim=0) so a
    /// chunk that fails to embed isn't retried every pass. The write is guarded
    /// on the chunk text STILL matching what was embedded, so a concurrent edit
    /// during the (unlocked) embed can't persist a stale vector — the rewritten
    /// chunk has no row and is re-embedded next pass.
    fn store_vectors(&self, items: &[(String, usize, String, Option<Vec<f32>>)]) {
        for (path, ord, text, maybe_vec) in items {
            let (dim, blob): (i64, Vec<u8>) = match maybe_vec {
                Some(v) => (
                    v.len() as i64,
                    v.iter().flat_map(|f| f.to_le_bytes()).collect(),
                ),
                None => (0, Vec::new()),
            };
            let _ = self.conn.execute(
                "INSERT OR REPLACE INTO vectors(path, ord, dim, vec) \
                 SELECT ?1, ?2, ?3, ?4 \
                 WHERE EXISTS(SELECT 1 FROM chunks WHERE path=?1 AND ord=?2 AND text=?5);",
                rusqlite::params![path, *ord as i64, dim, blob, text],
            );
        }
    }

    /// Brute-force cosine over every stored vector. Vectors are pre-normalized,
    /// so cosine == dot product. O(n) decode+dot per query — fine at a personal
    /// vault's chunk count; revisit (sqlite-vec / HNSW) only if it grows large.
    /// Returns `(path, ord, score)` sorted best-first, capped at `limit`. Only
    /// rows whose stored dim matches the query dim are scored (so tombstones,
    /// dim=0, are skipped).
    fn query(&self, qvec: &[f32], limit: usize) -> Vec<(String, usize, f32)> {
        let dim = qvec.len();
        let mut stmt = match self
            .conn
            .prepare("SELECT path, ord, vec FROM vectors WHERE dim=?1;")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([dim as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?.max(0) as usize,
                r.get::<_, Vec<u8>>(2)?,
            ))
        });
        let mut scored: Vec<(String, usize, f32)> = Vec::new();
        if let Ok(rows) = rows {
            for (path, ord, blob) in rows.flatten() {
                if blob.len() != dim * 4 {
                    continue;
                }
                let mut dot = 0.0f32;
                for (i, q) in qvec.iter().enumerate() {
                    let b = [
                        blob[i * 4],
                        blob[i * 4 + 1],
                        blob[i * 4 + 2],
                        blob[i * 4 + 3],
                    ];
                    dot += q * f32::from_le_bytes(b);
                }
                scored.push((path, ord, dot));
            }
        }
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// The best chunk per note for `qvec`, best first, at most `limit` notes
    /// (retrieval's semantic half, agent effectiveness F3). Cosine scores, as
    /// [`semantic_search`] computes them; the chunk's body offsets locate its
    /// citable lines.
    pub(crate) fn note_hits(&self, qvec: &[f32], limit: usize) -> Vec<ChunkHit> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut out = Vec::new();
        for (path, ord, score) in self.query(qvec, limit.saturating_mul(4).max(limit)) {
            if !seen.insert(path.clone()) {
                continue; // sorted best first: the first per path is its best
            }
            let chunks = self.chunks_for_path(&path);
            let Some(chunk) = chunks.iter().find(|c| c.ord == ord) else {
                continue;
            };
            let mtime = self
                .meta
                .get(&path)
                .map(|(m, _, _)| m.clone())
                .unwrap_or_default();
            out.push(ChunkHit {
                path,
                kind: chunk.kind.clone(),
                mtime,
                byte_start: chunk.byte_start,
                byte_end: chunk.byte_end,
                in_frontmatter: chunk.in_frontmatter,
                score,
            });
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    /// The best cosine of `qvec` with note `path`'s statement: its embedded
    /// body chunks before the first heading (meaning-ranked recall, OF26). A
    /// learning (`vault_learning::write_context`) puts its statement there
    /// and the "Why Hollow proposed this" and "Source excerpt" sections under
    /// headings; the excerpt is raw conversation, which can sit close to a
    /// request the learning has nothing to do with (OF26 review), so neither
    /// counts. A long statement split into several chunks counts its closest
    /// one. `None` when it has no such vector of `qvec`'s dimension (not
    /// embedded yet, a tombstone, another model, no text before a heading).
    pub(crate) fn statement_cosine(&self, qvec: &[f32], path: &str) -> Option<f32> {
        let dim = qvec.len();
        if dim == 0 {
            return None;
        }
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT v.vec FROM vectors v JOIN chunks c ON c.path=v.path AND c.ord=v.ord \
                 WHERE v.path=?1 AND v.dim=?2 AND c.heading IS NULL AND c.in_frontmatter=0;",
            )
            .ok()?;
        let rows = stmt
            .query_map(rusqlite::params![path, dim as i64], |r| {
                r.get::<_, Vec<u8>>(0)
            })
            .ok()?;
        rows.flatten()
            .filter(|blob| blob.len() == dim * 4)
            .map(|blob| {
                qvec.iter()
                    .zip(blob.chunks_exact(4))
                    .map(|(q, b)| q * f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .sum::<f32>()
            })
            .reduce(f32::max)
    }

    /// [`Self::statement_cosine`] of each of `paths`, keyed by path, leaving
    /// out a path with none: what recall reads for all its global learnings
    /// under one hold of the engaged index's lock (OF26 second review).
    pub(crate) fn statement_cosines(
        &self,
        qvec: &[f32],
        paths: &[String],
    ) -> BTreeMap<String, f32> {
        paths
            .iter()
            .filter_map(|path| Some((path.clone(), self.statement_cosine(qvec, path)?)))
            .collect()
    }

    /// Embed every pending chunk with `embed` (tests: a real or fake model
    /// over an isolated index).
    #[cfg(test)]
    pub(crate) fn embed_pending_for_test(&self, embed: impl Fn(&str) -> Vec<f32>) {
        loop {
            let batch = self.take_unembedded(64);
            if batch.is_empty() {
                return;
            }
            let out: Vec<EmbeddedChunk> = batch
                .into_iter()
                .map(|(path, ord, text)| {
                    let v = embed(&text);
                    (path, ord, text, Some(v))
                })
                .collect();
            self.store_vectors(&out);
        }
    }

    /// Count of chunks with a real (non-tombstone) embedding — the "indexed"
    /// progress for the Settings status row.
    fn embedded_count(&self) -> usize {
        self.conn
            .query_row("SELECT COUNT(*) FROM vectors WHERE dim>0;", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0)
            .max(0) as usize
    }
}

struct PreparedChunks {
    title: String,
    kind: String,
    chunks: Vec<Chunk>,
}

/// Kinds whose substance lives in their frontmatter (176 of 209 tasks keep
/// title, description, project and status nowhere else): they always get a
/// frontmatter chunk, after their body's chunks.
const FRONTMATTER_KINDS: [&str; 4] = [
    crate::vault_index::KIND_TASK,
    crate::vault_index::KIND_PROJECT,
    crate::vault_index::KIND_PERSON,
    crate::vault_index::KIND_GOAL,
];

/// The frontmatter fields a frontmatter chunk embeds, in order, with the
/// label each line gets. No other field is read.
const FRONTMATTER_FIELDS: [(&str, &str); 5] = [
    ("title", "Title"),
    ("description", "Description"),
    ("project", "Project"),
    ("status", "Status"),
    ("due", "Due"),
];

/// Whether `key` is one of the [`FRONTMATTER_FIELDS`] a frontmatter chunk
/// embeds: the broker quotes only these lines of its span.
pub(crate) fn is_frontmatter_field(key: &str) -> bool {
    FRONTMATTER_FIELDS.iter().any(|(k, _)| *k == key)
}

/// The body's section-aware chunks, then — for a note whose body has none,
/// or one of [`FRONTMATTER_KINDS`] — one chunk synthesized from its
/// frontmatter ([`frontmatter_chunk`]).
fn prepare_chunks(path: &str, content: &str) -> PreparedChunks {
    let (fm, body) = crate::frontmatter::parse_map(content);
    let item = crate::vault_index::classify(path, &fm);
    let body = if is_html_path(path) {
        strip_html(&body)
    } else {
        body
    };
    let opts = ChunkOpts {
        title: &item.title,
        kind: &item.kind,
        max_chars: DEFAULT_MAX_CHARS,
        overlap_chars: DEFAULT_OVERLAP_CHARS,
    };
    let mut chunks = chunk_note(&body, &opts);
    if chunks.is_empty() || FRONTMATTER_KINDS.contains(&item.kind.as_str()) {
        if let Some(mut chunk) = frontmatter_chunk(content, &fm, !chunks.is_empty()) {
            chunk.ord = chunks.len();
            chunks.push(chunk);
        }
    }
    PreparedChunks {
        title: item.title,
        kind: item.kind,
        chunks,
    }
}

/// The kind [`prepare_chunks`] gives note `path` and how many chunks it
/// makes of `content` (OF17's replay probe counts the notes with none).
#[cfg(test)]
pub(crate) fn chunk_count(path: &str, content: &str) -> (String, usize) {
    let prepared = prepare_chunks(path, content);
    (prepared.kind, prepared.chunks.len())
}

/// One chunk of plain `Label: value` lines from the [`FRONTMATTER_FIELDS`]
/// the note sets (blank values and YAML block scalars skipped). Its span runs
/// from the first to the last of those fields' lines in the file, inside the
/// frontmatter block, so a citation of it names a frontmatter line. `None`
/// without frontmatter or any of those fields, and for a note with a body
/// whose only such field is its title (every body chunk's header has it).
fn frontmatter_chunk(
    content: &str,
    fm: &BTreeMap<String, String>,
    has_body: bool,
) -> Option<Chunk> {
    let (start, end) = crate::frontmatter::block_bounds(content).ok()?;
    let kept: Vec<(&str, &str, &str)> = FRONTMATTER_FIELDS
        .iter()
        .filter_map(|&(key, label)| {
            let value = fm.get(key)?.trim();
            let usable = !value.is_empty() && !crate::frontmatter::is_block_scalar(content, key);
            usable.then_some((key, label, value))
        })
        .collect();
    if kept.is_empty() || (has_body && kept.iter().all(|(key, _, _)| *key == "title")) {
        return None;
    }
    let mut span: Option<(usize, usize)> = None;
    let mut offset = start;
    for line in content[start..end].split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let Some(key) = crate::frontmatter::top_level_key(line) else {
            continue;
        };
        if kept.iter().any(|(k, _, _)| *k == key) {
            let line_end = line_start + line.trim_end().len();
            span = Some(match span {
                Some((s, e)) => (s.min(line_start), e.max(line_end)),
                None => (line_start, line_end),
            });
        }
    }
    // Only an indented key (which `parse` reads too) leaves no span: the block.
    let (byte_start, byte_end) = span.unwrap_or((start, end));
    let text = kept
        .iter()
        .map(|(_, label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join("\n");
    Some(Chunk {
        ord: 0,
        heading: None,
        byte_start,
        byte_end,
        text,
        in_frontmatter: true,
    })
}

pub(crate) fn is_html_path(path: &str) -> bool {
    let p = path.to_lowercase();
    p.ends_with(".html") || p.ends_with(".htm")
}

/// Text a reader never sees: an HTML comment, a script or a style (OF25).
/// One regex scanned left to right, so whichever block OPENS first wins: a
/// commented-out `<script src=…>` stays a comment, and a `"<!--"` string
/// inside a script stays script, not a comment swallowing the visible text
/// after it (separate passes got one or the other wrong). A script or style
/// runs to its closing tag (`</script >` and `</script\n>` close too), or to
/// the end of the file when it has none, as a browser reads it: an unclosed
/// `<script>` used to leak its code into the text. The tag name must end at
/// whitespace, `/` or `>`, so `<script-loader>` or `<style-guide>` is just a
/// tag. An unclosed comment matches nothing (a stray `<!--` doesn't blank the
/// rest of the file).
static HIDDEN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?is)<!--.*?-->|<script(?:[\s/][^>]*)?>.*?(?:</script\s*>|\z)|<style(?:[\s/][^>]*)?>.*?(?:</style\s*>|\z)",
    )
    .unwrap()
});
static BLOCK_TAG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)</?(p|br|div|li|ul|ol|h[1-6]|tr|td|section|article|header|footer)\b[^>]*>")
        .unwrap()
});
static TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)<[^>]+>").unwrap());
static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t\r\n]+").unwrap());

fn strip_html(html: &str) -> String {
    let s = HIDDEN_RE.replace_all(html, " ");
    let s = BLOCK_TAG_RE.replace_all(&s, "\n");
    let s = TAG_RE.replace_all(&s, " ");
    let s = decode_basic_entities(&s);
    WS_RE.replace_all(&s, " ").trim().to_string()
}

/// [`strip_html`] that keeps the line structure: a removed comment, script,
/// style or tag leaves its own line breaks behind, so line N of the result is
/// the text of line N of `html` and retrieval's line evidence still cites the
/// file's line (HF05). Spaces collapse within each line.
pub(crate) fn strip_html_lines(html: &str) -> String {
    let s = HIDDEN_RE.replace_all(html, keep_breaks);
    let s = TAG_RE.replace_all(&s, keep_breaks);
    decode_basic_entities(&s)
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A removed match's replacement in [`strip_html_lines`]: its line breaks,
/// else one space.
fn keep_breaks(caps: &regex::Captures) -> String {
    match caps[0].matches('\n').count() {
        0 => " ".to_string(),
        n => "\n".repeat(n),
    }
}

fn decode_basic_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

// ── Global engagement + the semantic backfill worker (GUI process only) ──

static ENGAGED: Lazy<Mutex<Option<(PathBuf, EmbedIndex)>>> = Lazy::new(|| Mutex::new(None));

/// The database file `engage` opened (see `vault_search_index::ENGAGED_DB`):
/// never offered for removal. Its own lock, since `reconcile` holds
/// [`ENGAGED`] for long stretches.
static ENGAGED_DB: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

/// See [`ENGAGED_DB`].
pub(crate) fn engaged_db_path() -> Option<PathBuf> {
    ENGAGED_DB.lock().ok()?.clone()
}

/// Master kill switch (Settings → "Semantic search"). Default OFF: no model
/// loads or chunks are embedded, and `semantic_search` returns empty. Disabling
/// requests model release without waiting on an in-flight inference. The
/// derived index remains on disk for reuse. Synced from the frontend at boot.
static SEMANTIC_ENABLED: AtomicBool = AtomicBool::new(false);

/// Chunks embedded per batch before re-acquiring the lock to store + yield.
const EMBED_BATCH: usize = 16;
/// Pause between batches so a first-run backfill doesn't pin a core.
const EMBED_THROTTLE_MS: u64 = 25;
/// Worker idle/poll cadence when enabled (reconcile external edits + drain).
const WORKER_POLL_SECS: u64 = 30;

pub fn set_semantic_enabled(on: bool) {
    let was = SEMANTIC_ENABLED.swap(on, Ordering::AcqRel);
    if !on {
        crate::embed_model::request_unload();
    } else if !was {
        crate::embed_model::warm_on_enable();
    }
}

pub fn is_semantic_enabled() -> bool {
    SEMANTIC_ENABLED.load(Ordering::Acquire)
}

/// Where the derived embedding databases live. Test builds get a per-process
/// temp directory so a test can never write into the user's app data dir
/// (same isolation as `vault_search_index::index_root`).
#[cfg(not(test))]
fn index_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("EmbedIndex"))
}

#[cfg(test)]
fn index_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir("hollow-test-embedindex-"))
}

/// Its stem must stay `session_store::vault_file_stem` (see
/// `vault_search_index::db_path_for`).
pub(crate) fn db_path_for(vault: &Path) -> Option<PathBuf> {
    let canon = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canon.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    let name: String = canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "vault".to_string())
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Some(index_root()?.join(format!("{name}-{}.db", &hash[..16])))
}

pub fn engage(vault: &Path) {
    let Some(db_path) = db_path_for(vault) else {
        return;
    };
    if let Ok(mut g) = ENGAGED_DB.lock() {
        *g = Some(db_path.clone());
    }
    let Some(mut idx) = EmbedIndex::open(&db_path) else {
        return;
    };
    let notes = crate::vault::walk_notes(vault);
    let _ = idx.full_sweep(&notes, vault);
    let _ = idx.compact_wal();
    if let Ok(mut g) = ENGAGED.lock() {
        *g = Some((vault.to_path_buf(), idx));
    }
}

pub fn on_write(vault: &Path, rel: &str, content: &str, mtime: &str, size: u64) {
    if let Ok(mut g) = ENGAGED.lock() {
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                idx.upsert(rel, mtime, size, content);
            }
        }
    }
}

pub fn on_remove(vault: &Path, rel: &str) {
    if let Ok(mut g) = ENGAGED.lock() {
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                idx.remove(rel);
            }
        }
    }
}

/// Status for the Settings row: `(enabled, embedded_chunks, total_chunks)`.
pub fn semantic_status(vault: &Path) -> (bool, usize, usize) {
    let enabled = is_semantic_enabled();
    if let Ok(g) = ENGAGED.lock() {
        if let Some((v, idx)) = g.as_ref() {
            if v == vault {
                return (enabled, idx.embedded_count(), idx.chunk_count());
            }
        }
    }
    (enabled, 0, 0)
}

/// Whether this process can answer a semantic query for `vault` right now:
/// the switch is on, the index is engaged here for this vault and holds at
/// least one embedded chunk. `Err` carries the reason, in Hollow's words.
pub(crate) fn semantic_ready(vault: &Path) -> Result<(), &'static str> {
    if !is_semantic_enabled() {
        return Err("semantic search is off");
    }
    let g = ENGAGED
        .lock()
        .map_err(|_| "the semantic index is unavailable")?;
    match g.as_ref() {
        Some((v, idx)) if v == vault => {
            if idx.embedded_count() == 0 {
                Err("the semantic index has no embedded notes yet")
            } else {
                Ok(())
            }
        }
        _ => Err("the semantic index isn't open in this process"),
    }
}

/// [`EmbedIndex::note_hits`] on this process's engaged index for `vault`.
pub(crate) fn engaged_note_hits(
    vault: &Path,
    qvec: &[f32],
    limit: usize,
) -> Result<Vec<ChunkHit>, &'static str> {
    semantic_ready(vault)?;
    let g = ENGAGED
        .lock()
        .map_err(|_| "the semantic index is unavailable")?;
    match g.as_ref() {
        Some((v, idx)) if v == vault => Ok(idx.note_hits(qvec, limit)),
        _ => Err("the semantic index isn't open in this process"),
    }
}

/// How long a turn's recall waits for its global learnings' cosines (OF26
/// review, B1; second review). Recall is required local context that
/// preparation never cuts (`prep_deadline`), and it runs before
/// `runStarted` on the turn's async task, so an unbounded wait could hold
/// the turn (and a tokio worker) for a cold model load, behind the backfill
/// worker on the model's lock (it takes the lock per chunk but, the lock
/// being unfair, can re-take it through a whole batch of [`EMBED_BATCH`]
/// before its pause), or behind [`ENGAGED`], which `reconcile` holds while
/// it re-chunks every changed file and `on_write` while it re-chunks one. A
/// warm bge-small query takes a few tens of milliseconds; past this, recall
/// ranks by stems only (`RecallMeaning::TimedOut`) while the work runs on in
/// its own thread and leaves its vector in `embed_model`'s query cache,
/// where retrieval, which asks for the same text next, finds it.
///
/// J4 (overnight 11-08, `recall-measurement.md` in that night's board
/// folder): 100 ms, down from 250. On an idle Mac with an optimized build the
/// query takes 28 ms (p50) and 50 ms (p99), model resident or just loaded, so
/// the work itself never needs more; in 500 real runs recall timed out on
/// 289 and waited the whole 255 ms each time (mean 173 ms a turn), and 48 of
/// the 197 that got a vector had waited past 100 ms. That is contention, which
/// a longer wait doesn't cure. The 100 ms cap gives up meaning-ranking on those
/// 48 (10% of turns), where the probe's top 5 differs from the stem ranking by
/// 0.97 Jaccard and the first item moves in 4% of requests.
pub(crate) const RECALL_MEANING_WAIT: Duration = Duration::from_millis(100);

/// What recall waits once it has been late [`RECALL_LATE_LIMIT`] times running
/// (see [`RecallPace`]): long enough for an idle query (28 ms p50), short
/// enough that a contended stretch costs a turn almost nothing.
pub(crate) const RECALL_SLOW_WAIT: Duration = Duration::from_millis(25);

/// Consecutive late recalls after which the wait drops to
/// [`RECALL_SLOW_WAIT`].
pub(crate) const RECALL_LATE_LIMIT: u32 = 3;

/// How recent recalls fared against their wait (J4): after
/// [`RECALL_LATE_LIMIT`] in a row whose vector wasn't ready in time, the
/// model or the index is contended (the backfill worker's batch, a
/// reconcile), and each further turn waits only [`RECALL_SLOW_WAIT`] for it,
/// as Jev's circuit skips a failing service. One recall that gets its vector
/// in time, or an embedding thread that finishes within
/// [`RECALL_MEANING_WAIT`] of its own start (it runs on past the turn's wait
/// and reports how long it took), ends the stretch. Only the wait changes: a
/// recall that misses its vector still ranks by stems, as it always did.
pub(crate) struct RecallPace {
    late: AtomicU32,
}

impl RecallPace {
    pub(crate) const fn new() -> Self {
        Self {
            late: AtomicU32::new(0),
        }
    }

    /// Whether recalls are in a late stretch: [`RECALL_LATE_LIMIT`] in a
    /// row missed their vector.
    pub(crate) fn contended(&self) -> bool {
        self.late.load(Ordering::Relaxed) >= RECALL_LATE_LIMIT
    }

    /// What the next recall waits for its vector.
    pub(crate) fn wait(&self) -> Duration {
        if self.contended() {
            RECALL_SLOW_WAIT
        } else {
            RECALL_MEANING_WAIT
        }
    }

    /// A recall's wait ended: its vector `arrived`, or it was late (timed
    /// out, or an earlier embedding was still running).
    pub(crate) fn waited(&self, arrived: bool) {
        if arrived {
            self.late.store(0, Ordering::Relaxed);
        } else {
            let _ = self
                .late
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n.saturating_add(1))
                });
        }
    }

    /// An embedding thread finished `took` after it started, whether or not
    /// its turn still waited: a healthy one ends a slow stretch.
    pub(crate) fn finished(&self, took: Duration) {
        if took <= RECALL_MEANING_WAIT {
            self.late.store(0, Ordering::Relaxed);
        }
    }
}

/// The app's one pace: recalls share one model and one index.
static RECALL_PACE: RecallPace = RecallPace::new();

/// Whether the app's recalls are in a late stretch ([`RecallPace`]): a
/// turn then doesn't wait for its prefetched recall at all (J3a,
/// `recall_prefetch`).
pub(crate) fn recall_contended() -> bool {
    RECALL_PACE.contended()
}

/// Whether a `hollow-recall-embed` thread is still running
/// ([`meaning_within`]): at most one at a time.
static RECALL_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// Whether a turn's recall could rank by meaning at all, from what is cheap
/// and never waits on a lock: semantic search on and the bundled model
/// installed. Whether the index is engaged for the vault ([`semantic_ready`],
/// under [`ENGAGED`]) is read on the `hollow-recall-embed` thread, inside
/// the bounded wait.
pub(crate) fn meaning_available() -> bool {
    if !is_semantic_enabled() {
        return false;
    }
    let dir = crate::embed_model::default_model_dir();
    dir.join("model.safetensors").is_file()
}

/// Meaning-ranked recall's vectors in the app (OF26): the request embedded
/// as retrieval's semantic half embeds it (same model, same epoch guard, so
/// `embed_model`'s one-entry query cache hands retrieval the same vector),
/// then every global learning's statement cosine read from this process's
/// engaged index under one lock ([`EmbedIndex::statement_cosines`]), all on
/// a `hollow-recall-embed` thread waited for [`RECALL_MEANING_WAIT`] at most
/// ([`RECALL_SLOW_WAIT`] while recalls keep arriving late, [`RecallPace`]; second review: the turn's own thread takes no lock that can block).
/// Nothing when semantic search can't answer for `vault`
/// ([`meaning_available`], [`semantic_ready`]): recall then ranks as before.
pub(crate) struct EngagedMeaning<'a> {
    pub vault: &'a Path,
}

impl crate::vault_learning::MeaningSource for EngagedMeaning<'_> {
    fn available(&self) -> bool {
        meaning_available()
    }

    fn cosines(
        &self,
        text: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, f32>, crate::vault_learning::RecallMeaning> {
        use crate::vault_learning::RecallMeaning;
        if !self.available() {
            return Err(RecallMeaning::Off);
        }
        let dir = crate::embed_model::default_model_dir();
        // Captured before the thread starts, as retrieval does before its
        // blocking pool: a disable while it waits keeps it cancelled.
        let epoch = crate::embed_model::request_epoch();
        let vault = self.vault.to_path_buf();
        let text = text.to_string();
        let paths = paths.to_vec();
        let result = meaning_within(RECALL_PACE.wait(), &RECALL_IN_FLIGHT, move || {
            let started = std::time::Instant::now();
            let result = (|| {
                semantic_ready(&vault).map_err(|_| RecallMeaning::Off)?;
                let qvec = crate::embed_model::embed_query_for_request(
                    &dir,
                    &text,
                    epoch,
                    is_semantic_enabled,
                )
                .map_err(meaning_of)?;
                let g = ENGAGED.lock().map_err(|_| RecallMeaning::Off)?;
                match g.as_ref() {
                    Some((v, idx)) if *v == vault => Ok(idx.statement_cosines(&qvec, &paths)),
                    _ => Err(RecallMeaning::Off),
                }
            })();
            if result.is_ok() {
                RECALL_PACE.finished(started.elapsed());
            }
            result
        });
        match &result {
            Ok(_) => RECALL_PACE.waited(true),
            Err(RecallMeaning::TimedOut) => RECALL_PACE.waited(false),
            // Off and NoVector say nothing about how fast the model is.
            Err(_) => {}
        }
        result
    }
}

/// J4's probe (`recall_probe`): embed every pending chunk of the engaged
/// index for `vault` with the real model; how many.
#[cfg(test)]
pub(crate) fn embed_engaged_for_probe(vault: &Path, model_dir: &Path) -> usize {
    let Ok(g) = ENGAGED.lock() else {
        return 0;
    };
    let Some((v, idx)) = g.as_ref() else {
        return 0;
    };
    if v != vault {
        return 0;
    }
    let mut done = 0;
    loop {
        let batch = idx.take_unembedded(64);
        if batch.is_empty() {
            return done;
        }
        let out: Vec<EmbeddedChunk> = batch
            .into_iter()
            .map(|(path, ord, text)| {
                let vector = crate::embed_model::embed_passage(model_dir, &text).ok();
                (path, ord, text, vector)
            })
            .collect();
        done += out.len();
        idx.store_vectors(&out);
    }
}

/// Whether a recall's embedding thread still runs (`recall_probe`).
#[cfg(test)]
pub(crate) fn recall_in_flight_for_probe() -> bool {
    RECALL_IN_FLIGHT.load(Ordering::Acquire)
}

/// A failed query embedding as recall records it: cancelled (semantic
/// search switched off meanwhile) is [`RecallMeaning::Off`], any other
/// failure [`RecallMeaning::NoVector`].
///
/// [`RecallMeaning::Off`]: crate::vault_learning::RecallMeaning::Off
/// [`RecallMeaning::NoVector`]: crate::vault_learning::RecallMeaning::NoVector
fn meaning_of(err: crate::embed_model::EmbedError) -> crate::vault_learning::RecallMeaning {
    use crate::vault_learning::RecallMeaning;
    match err {
        crate::embed_model::EmbedError::Cancelled => RecallMeaning::Off,
        _ => RecallMeaning::NoVector,
    }
}

/// Clears its flag when dropped: on its `hollow-recall-embed` thread before
/// the answer is sent, or with the work if the thread never started.
struct InFlight(&'static AtomicBool);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// `work`'s answer, run on its own thread (`hollow-recall-embed`) and
/// waited for `wait` at most: past it, [`RecallMeaning::TimedOut`] and the
/// thread runs on to its end (what it embeds lands in the query cache).
/// While an earlier thread on `in_flight` still runs, also `TimedOut` at
/// once, without starting another (second review: a stuck index lock can't
/// pile up one thread per turn). No thread, or one that dies, is
/// [`RecallMeaning::NoVector`].
///
/// [`RecallMeaning::TimedOut`]: crate::vault_learning::RecallMeaning::TimedOut
/// [`RecallMeaning::NoVector`]: crate::vault_learning::RecallMeaning::NoVector
fn meaning_within<T, F>(
    wait: Duration,
    in_flight: &'static AtomicBool,
    work: F,
) -> Result<T, crate::vault_learning::RecallMeaning>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, crate::vault_learning::RecallMeaning> + Send + 'static,
{
    use crate::vault_learning::RecallMeaning;
    use std::sync::mpsc::RecvTimeoutError;
    if in_flight
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err(RecallMeaning::TimedOut);
    }
    let guard = InFlight(in_flight);
    let (tx, answer) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("hollow-recall-embed".into())
        .spawn(move || {
            let result = work();
            // Before the answer: whoever receives it can start the next.
            drop(guard);
            let _ = tx.send(result);
        })
        .map_err(|_| RecallMeaning::NoVector)?;
    match answer.recv_timeout(wait) {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(RecallMeaning::TimedOut),
        Err(RecvTimeoutError::Disconnected) => Err(RecallMeaning::NoVector),
    }
}

/// Semantic search: cosine over the embedded chunk vectors, deduped to the
/// best-scoring chunk per note. `qvec` is the already-embedded query (the
/// caller runs `embed_model::embed_query`). Returns `SearchResult`s shaped like
/// the keyword path so the Vault UI renders + routes them identically. Empty
/// when the feature is off or the vault isn't engaged — the keyword path stays
/// the fallback.
pub fn semantic_search(
    vault: &Path,
    qvec: &[f32],
    limit: usize,
    kinds: Option<&[String]>,
) -> Vec<SearchResult> {
    if !is_semantic_enabled() {
        return Vec::new();
    }
    let g = match ENGAGED.lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    let (engaged_vault, idx) = match g.as_ref() {
        Some(x) => x,
        None => return Vec::new(),
    };
    if engaged_vault != vault {
        return Vec::new();
    }
    let kind_filter: Option<HashSet<&str>> =
        kinds.map(|ks| ks.iter().map(String::as_str).collect());
    // Overfetch so the per-note dedupe still yields `limit` distinct notes.
    // Lens-scoped semantic search needs a wider candidate pool because many
    // high-scoring chunks may belong to other lenses.
    let fetch_limit = if kind_filter.is_some() {
        limit.saturating_mul(12).max(limit)
    } else {
        limit.saturating_mul(4).max(limit)
    };
    let hits = idx.query(qvec, fetch_limit);
    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<SearchResult> = Vec::new();
    for (path, ord, score) in hits {
        let chunks = idx.chunks_for_path(&path);
        let chunk = chunks
            .iter()
            .find(|c| c.ord == ord)
            .or_else(|| chunks.first());
        let (title, kind, snippet) = match chunk {
            Some(c) => (c.title.clone(), c.kind.clone(), snippet_of(&c.text)),
            None => (String::new(), String::new(), String::new()),
        };
        if let Some(filter) = &kind_filter {
            if !filter.contains(kind.as_str()) {
                continue;
            }
        }
        if !seen.insert(path.clone()) {
            continue; // hits are sorted desc, so the first per path is its best
        }
        let _ = title; // title currently unused in SearchResult; kept for clarity
        let (mtime, size) = idx
            .meta
            .get(&path)
            .map(|(m, s, _)| (m.clone(), *s))
            .unwrap_or_default();
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        results.push(SearchResult {
            path: path.clone(),
            name,
            mtime,
            size,
            snippet,
            score: (score * 1000.0) as i64,
            kind,
        });
        if results.len() >= limit {
            break;
        }
    }
    results
}

fn snippet_of(text: &str) -> String {
    let t = text.trim();
    const MAX: usize = 240;
    if t.chars().count() <= MAX {
        t.to_string()
    } else {
        let s: String = t.chars().take(MAX).collect();
        format!("{s}…")
    }
}

/// The resident backfill worker (one spawned thread, `cwd`-independent). Dormant
/// while the feature is off (just a poll-sleep). When on: engage the cache once,
/// reconcile external edits each tick, and embed any chunks lacking a vector —
/// OFF the SQLite write path so vault writes never stall on inference. Runs for
/// the process lifetime.
pub fn run_semantic_worker(vault: &Path) {
    let model_dir = crate::embed_model::default_model_dir();
    let mut engaged_once = false;
    loop {
        if is_semantic_enabled() {
            if !engaged_once {
                engage(vault); // open + full_sweep + stash (catches edits since last run)
                engaged_once = true;
            } else {
                reconcile(vault);
            }
            drain(vault, &model_dir);
        }
        // A completed index must not keep a model resident just because the
        // worker wakes to reconcile metadata. Queries reload it on demand.
        crate::embed_model::evict_if_idle();
        std::thread::sleep(Duration::from_secs(WORKER_POLL_SECS));
    }
}

/// Cheap re-sweep on the engaged handle: re-chunk files changed by `(mtime,size)`
/// and drop notes that vanished. SQLite-only (no embedding), so holding the lock
/// is brief; `walk_notes` is cached, so an idle tick is nearly free.
fn reconcile(vault: &Path) {
    let notes = crate::vault::walk_notes(vault);
    if let Ok(mut g) = ENGAGED.lock() {
        if let Some((v, idx)) = g.as_mut() {
            if v == vault {
                let _ = idx.full_sweep(&notes, vault);
            }
        }
    }
}

type PendingChunk = (String, usize, String);
type EmbeddedChunk = (String, usize, String, Option<Vec<f32>>);

/// Embed pending chunks without loading the model for a completed index.
/// Cancellation and model-load failures leave chunks pending; an actual
/// per-chunk inference failure retains the existing dim=0 tombstone behavior.
fn drain(vault: &Path, model_dir: &Path) {
    drain_batches(
        || {
            if !is_semantic_enabled() {
                return Vec::new();
            }
            match ENGAGED.lock() {
                Ok(g) => match g.as_ref() {
                    Some((v, idx)) if v == vault => idx.take_unembedded(EMBED_BATCH),
                    _ => Vec::new(),
                },
                Err(_) => Vec::new(),
            }
        },
        |text| crate::embed_model::embed_passage_if_enabled(model_dir, text, is_semantic_enabled),
        |out| {
            if let Ok(g) = ENGAGED.lock() {
                if let Some((v, idx)) = g.as_ref() {
                    if v == vault {
                        idx.store_vectors(out);
                    }
                }
            }
        },
        || std::thread::sleep(Duration::from_millis(EMBED_THROTTLE_MS)),
    );
}

/// Keep the batch loop independent of the process-global cache so cancellation
/// and retry behavior can be exercised against an isolated SQLite index.
fn drain_batches(
    mut take_pending: impl FnMut() -> Vec<PendingChunk>,
    mut embed: impl FnMut(&str) -> Result<Vec<f32>, crate::embed_model::EmbedError>,
    mut store: impl FnMut(&[EmbeddedChunk]),
    mut pause: impl FnMut(),
) {
    loop {
        let batch = take_pending();
        if batch.is_empty() {
            return;
        }
        // No SQLite lock is held during model loading or inference. Check the
        // feature gate inside each guarded embed, including queued operations.
        let mut out = Vec::with_capacity(batch.len());
        let mut interrupted = false;
        for (path, ord, text) in batch {
            let v = match embed(&text) {
                Ok(v) => Some(v),
                Err(crate::embed_model::EmbedError::Inference(_)) => None,
                Err(
                    crate::embed_model::EmbedError::Cancelled
                    | crate::embed_model::EmbedError::Load(_),
                ) => {
                    interrupted = true;
                    break;
                }
            };
            out.push((path, ord, text, v));
        }
        if !out.is_empty() {
            store(&out);
        }
        if interrupted {
            return;
        }
        pause();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, mtime: &str, size: u64) -> Note {
        Note {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            mtime: mtime.to_string(),
            size,
        }
    }

    fn write_note(vault: &Path, rel: &str, body: &str) -> Note {
        let full = vault.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, body).unwrap();
        note(rel, "2026-06-23T00:00:00+00:00", body.len() as u64)
    }

    fn index_at(dir: &Path) -> EmbedIndex {
        EmbedIndex::open(&dir.join("embed.db")).unwrap()
    }

    /// Same isolation guarantee as the search index: no test-run database may
    /// land in the user's real app data directory.
    #[test]
    fn db_path_stays_out_of_the_real_app_data_dir() {
        let db = db_path_for(Path::new("/tmp/some-vault")).expect("a db path");
        assert!(
            db.starts_with(std::env::temp_dir()),
            "test db path should live under the temp dir, got {db:?}"
        );
        if let Some(real) = dirs::data_dir() {
            assert!(
                !db.starts_with(real.join("Hollow")),
                "test db path must not sit under the live app data dir, got {db:?}"
            );
        }
    }

    /// OF25: an index a newer build wrote (an older build installed over it,
    /// a rollback) is neither dropped nor written. `open` gives `None`, so
    /// semantic search stays off as it does with no index, and the newer
    /// build's chunk and stored version are still there for it.
    #[test]
    fn a_newer_schema_index_is_left_alone() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let db = dir.path().join("embed.db");
        let newer = SCHEMA_VERSION + 1;
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE chunks(path TEXT NOT NULL, ord INTEGER NOT NULL, text TEXT NOT NULL, \
                                 PRIMARY KEY(path, ord));\
             INSERT INTO chunks VALUES('notes/a.md', 0, 'alpha');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", newer).unwrap();
        drop(conn);
        assert!(
            EmbedIndex::open(&db).is_none(),
            "a newer index opens disabled"
        );
        let conn = Connection::open(&db).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, newer, "the stored version is kept");
        let chunks: i64 = conn
            .query_row("SELECT COUNT(*) FROM chunks;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunks, 1, "the newer index's chunk is kept");
    }

    #[test]
    fn prepare_chunks_strips_frontmatter_and_carries_context() {
        let content = "---\ntitle: Roadmap\n---\n\nIntro\n\n## Alpha\nShip semantic search.";
        let prepared = prepare_chunks("notes/roadmap.md", content);
        assert_eq!(prepared.title, "Roadmap");
        assert_eq!(prepared.kind, crate::vault_index::KIND_NOTE);
        assert_eq!(prepared.chunks.len(), 2);
        assert!(prepared.chunks[1]
            .text
            .starts_with("Roadmap · note · Alpha\n\n"));
        assert!(!prepared.chunks[0].text.contains("title: Roadmap"));
    }

    #[test]
    fn html_notes_strip_scripts_styles_tags_and_entities() {
        let content =
            "<style>.x{}</style><h1>Brief</h1><script>alert(1)</script><p>A &amp; B&nbsp;win.</p>";
        let prepared = prepare_chunks("briefings/today.html", content);
        assert_eq!(prepared.kind, crate::vault_index::KIND_BRIEFING);
        assert_eq!(prepared.chunks.len(), 1);
        let text = &prepared.chunks[0].text;
        assert!(text.contains("Brief A & B win."));
        assert!(!text.contains("alert"));
        assert!(!text.contains("<p>"));
    }

    /// HF05: line-preserving stripping keeps every line where it was, so a
    /// line number cited from the text is the file's own.
    #[test]
    fn strip_html_lines_keeps_line_numbers() {
        let html = "<html>\n<style>\n.x { color: red }\n</style>\n<p class=\"a\">Gate &amp; \
                    relay</p>\n<script>\nalert(1)\n</script><div\n  id=\"b\">Elm St lot</div>\n";
        let text = strip_html_lines(html);
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines.len(), html.split('\n').count());
        assert_eq!(lines[4], "Gate & relay");
        assert_eq!(lines[8], "Elm St lot");
        assert!(!text.contains('<'));
        assert!(!text.contains("color") && !text.contains("alert"));
    }

    /// OF25: a comment goes whole, markup inside it too, and leaves its line
    /// breaks; a commented-out script doesn't blank the rest of the file.
    #[test]
    fn strip_html_lines_drops_comments() {
        assert_eq!(strip_html_lines("<!-- <b>x</b> -->\ntext"), "\ntext");
        let html = "<p>a</p><!-- one\n<script src=\"x.js\"> -->\n<p>Elm St lot</p>";
        assert_eq!(strip_html_lines(html), "a\n\nElm St lot");
        let prepared = prepare_chunks("briefings/c.html", "<p>Brief</p><!-- <i>draft</i> -->");
        assert_eq!(prepared.chunks.len(), 1);
        assert!(!prepared.chunks[0].text.contains("draft"));
    }

    /// OF25: an unclosed script or style runs to the end of the file, blanked
    /// with its line breaks kept, instead of leaking its code as text.
    #[test]
    fn strip_html_lines_blanks_an_unclosed_script_or_style() {
        let html = "<script>var a=1\nb";
        let text = strip_html_lines(html);
        assert_eq!(text, "\n");
        assert_eq!(text.split('\n').count(), html.split('\n').count());
        let html = "<p>Gate</p>\n<style>\n.x { color: red }";
        assert_eq!(strip_html_lines(html), "Gate\n\n");
        assert_eq!(strip_html_lines("<p>a</p><script>x()</script>\nb"), "a\nb");
    }

    /// OF25 review: whichever block opens first wins. A `"<!--"` string in a
    /// script is script, so it can't open a comment that runs to a later
    /// `-->` and swallows the visible text between.
    #[test]
    fn strip_html_lines_a_comment_opener_inside_a_script_is_script() {
        let html = "<script>var s = \"<!--\";</script>\n<p>Elm St lot</p>\n<!-- note -->";
        assert_eq!(strip_html_lines(html), "\nElm St lot\n");
        assert_eq!(strip_html(html), "Elm St lot");
    }

    /// OF25 review: `</script >` and `</script\n>` close a script, as in a
    /// browser, instead of leaving it unclosed to the end of the file.
    #[test]
    fn strip_html_lines_a_closing_tag_may_hold_whitespace() {
        assert_eq!(strip_html_lines("<script>x()</script >\nb"), "\nb");
        assert_eq!(strip_html_lines("<script>x()</script\n>b"), "\nb");
        assert_eq!(strip_html_lines("<style>.x{}</style\t>\nb"), "\nb");
        assert_eq!(strip_html("<script>x()</script >Elm St lot"), "Elm St lot");
    }

    /// OF25 review: a custom element whose name starts `script-`/`style-` is
    /// an ordinary tag; its text stays (`\b` used to match before the `-`,
    /// and with no `</script>` it would have blanked the rest of the file).
    #[test]
    fn strip_html_lines_a_custom_element_is_not_a_script() {
        let html = "<script-loader>Gate</script-loader>\n<style-guide>Elm</style-guide>\nb";
        assert_eq!(strip_html_lines(html), "Gate\nElm\nb");
        assert_eq!(strip_html("<script-loader>Gate</script-loader>"), "Gate");
        assert_eq!(
            strip_html_lines("<script src=\"a.js\">x()</script>\nb"),
            "\nb"
        );
        // `/` ends the name too; HTML has no self-closing script, so it runs on.
        assert_eq!(strip_html_lines("<script/>x\nb"), "\n");
    }

    /// OF25: [`strip_html_lines`] as it was before comments were dropped and
    /// unclosed scripts ran to the end, to count what it leaked.
    fn strip_html_lines_before_of25(html: &str) -> String {
        let script = Regex::new(r"(?is)<script\b[^>]*>.*?</script>").unwrap();
        let style = Regex::new(r"(?is)<style\b[^>]*>.*?</style>").unwrap();
        let s = script.replace_all(html, keep_breaks);
        let s = style.replace_all(&s, keep_breaks);
        let s = TAG_RE.replace_all(&s, keep_breaks);
        decode_basic_entities(&s)
            .split('\n')
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The words inside each comment, script or style ([`HIDDEN_RE`]) of
    /// `text`, each with its line: what a stripped copy shouldn't show on that
    /// line. Markup inside the match is dropped first, and a word needs a
    /// letter or digit.
    fn hidden_words(text: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for m in HIDDEN_RE.find_iter(text) {
            let first = text[..m.start()].matches('\n').count();
            let inner = TAG_RE.replace_all(m.as_str(), keep_breaks);
            for (i, line) in inner.split('\n').enumerate() {
                for word in line.split_whitespace() {
                    if word.chars().any(char::is_alphanumeric) {
                        out.push((first + i, word.to_string()));
                    }
                }
            }
        }
        out
    }

    /// Whether `stripped` shows any of `hidden` on its own line. A heuristic
    /// that over-counts: a hidden word that is ALSO visible on that line (a
    /// `<p>Gate</p><!-- Gate -->`) counts as a leak, so a file count built on
    /// it is an upper bound.
    fn leaks(stripped: &str, hidden: &[(usize, String)]) -> bool {
        let lines: Vec<&str> = stripped.split('\n').collect();
        hidden.iter().any(|(line, word)| {
            lines
                .get(*line)
                .is_some_and(|l| l.split_whitespace().any(|w| w == word))
        })
    }

    /// Every `.html`/`.htm` file under `dir`, never entering (or reading) a
    /// path component named `restricted`.
    fn html_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_name() == "restricted" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                html_files(&path, out);
            } else if kind.is_file() && is_html_path(&path.to_string_lossy()) {
                out.push(path);
            }
        }
    }

    /// OF25 on a real vault: how many `.html` files hold comments, scripts or
    /// styles, and how many leaked their text before and after. Counts only,
    /// never a path or any text. Skips (passes) when the variable is unset.
    /// The leak counts are a measurement and an upper bound (the same-line
    /// word heuristic of [`leaks`]), so they're reported, not asserted; only a
    /// changed line count fails it, since that is exact.
    #[test]
    #[ignore = "reads a vault copy: HOLLOW_HTML_LEAK_VAULT=<copy without restricted/>"]
    fn html_leak_counts_on_a_real_vault() {
        let Some(vault) = std::env::var("HOLLOW_HTML_LEAK_VAULT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
        else {
            eprintln!("html leak counts: HOLLOW_HTML_LEAK_VAULT is unset; skipped");
            return;
        };
        let vault = PathBuf::from(vault);
        assert!(vault.is_dir(), "HOLLOW_HTML_LEAK_VAULT is not a folder");
        assert!(
            !vault.components().any(|c| c.as_os_str() == "restricted"),
            "HOLLOW_HTML_LEAK_VAULT lies inside a restricted folder"
        );
        let mut files = Vec::new();
        html_files(&vault, &mut files);
        let total = files.len();
        let (mut unreadable, mut with_comments, mut with_scripts, mut with_styles) = (0, 0, 0, 0);
        let (mut leaks_before, mut leaks_after, mut lines_moved) = (0, 0, 0);
        for path in &files {
            let Ok(html) = std::fs::read_to_string(path) else {
                unreadable += 1;
                continue;
            };
            let (mut comment, mut script, mut style) = (false, false, false);
            for m in HIDDEN_RE.find_iter(&html) {
                // Every match opens `<!--`, `<script` or `<style` (ASCII, any case).
                let s = m.as_str();
                if s.starts_with("<!--") {
                    comment = true;
                } else if s
                    .get(..7)
                    .is_some_and(|h| h.eq_ignore_ascii_case("<script"))
                {
                    script = true;
                } else {
                    style = true;
                }
            }
            with_comments += usize::from(comment);
            with_scripts += usize::from(script);
            with_styles += usize::from(style);
            let hidden = hidden_words(&html);
            leaks_before += usize::from(leaks(&strip_html_lines_before_of25(&html), &hidden));
            let after = strip_html_lines(&html);
            leaks_after += usize::from(leaks(&after, &hidden));
            lines_moved += usize::from(after.split('\n').count() != html.split('\n').count());
        }
        println!(
            "html files {total}, unreadable {unreadable}, with comments {with_comments}, \
             with scripts {with_scripts}, with styles {with_styles}, \
             leaking before {leaks_before}, leaking after {leaks_after} \
             (leaks are an upper bound: same-line word heuristic), \
             line count changed {lines_moved}"
        );
        assert_eq!(lines_moved, 0, "files whose line count changed");
    }

    /// HF03: a task whose title, description, project and status live only
    /// in its frontmatter had no chunk at all, so semantic retrieval never
    /// found it. It gets one, labelled lines of just those fields, whose span
    /// lies inside the frontmatter block (a citation names no body line).
    #[test]
    fn frontmatter_only_task_gets_one_chunk_inside_its_frontmatter() {
        let content = "---\nid: t-17\ntitle: Renew parking permit\nstatus: open\n\
                       description: Call the city about the Elm St lot permit\n\
                       project: Elm St garage\ndue: 2026-11-01\ncreated: 2026-10-01\n---\n";
        let prepared = prepare_chunks("tasks/renew-permit.md", content);
        assert_eq!(prepared.kind, crate::vault_index::KIND_TASK);
        assert_eq!(prepared.chunks.len(), 1);
        let chunk = &prepared.chunks[0];
        assert!(chunk.in_frontmatter);
        assert_eq!(chunk.ord, 0);
        assert_eq!(
            chunk.text,
            "Title: Renew parking permit\n\
             Description: Call the city about the Elm St lot permit\n\
             Project: Elm St garage\nStatus: open\nDue: 2026-11-01"
        );
        let (start, end) = crate::frontmatter::block_bounds(content).unwrap();
        assert!(start <= chunk.byte_start && chunk.byte_end <= end);
        let span = &content[chunk.byte_start..chunk.byte_end];
        assert!(span.starts_with("title: Renew"), "{span:?}");
        assert!(span.ends_with("due: 2026-11-01"), "{span:?}");
        assert!(!span.contains("id: t-17") && !span.contains("created:"));
    }

    /// The kind choice: a task (project, person, goal) gets its frontmatter
    /// chunk even with a body, after the body's chunks, which are unchanged.
    #[test]
    fn task_with_a_body_gets_its_frontmatter_chunk_after_the_body() {
        let content = "---\ntitle: Ship the gate fix\nstatus: doing\nproject: Gate\n---\n\n\
                       Swap the relay, then test the loop.";
        let prepared = prepare_chunks("tasks/gate-fix.md", content);
        assert_eq!(prepared.chunks.len(), 2);
        let (body, fm) = (&prepared.chunks[0], &prepared.chunks[1]);
        assert!(!body.in_frontmatter);
        assert!(body.text.contains("Swap the relay"));
        assert!(fm.in_frontmatter);
        assert_eq!(fm.ord, 1);
        assert_eq!(
            fm.text,
            "Title: Ship the gate fix\nProject: Gate\nStatus: doing"
        );
        // A person with a body and only a title adds nothing new: no chunk.
        let jane = prepare_chunks(
            "people/jane.md",
            "---\ntitle: Jane\n---\n\nWorks on parking.",
        );
        assert_eq!(jane.chunks.len(), 1);
        assert!(!jane.chunks[0].in_frontmatter);
    }

    /// A note with neither a body nor any of the embedded fields stays at 0
    /// chunks; one with an empty body and a title gets a title chunk.
    #[test]
    fn frontmatter_chunk_needs_a_field_it_embeds() {
        let bare = "---\ncreated: 2026-01-01\ntags: x\n---\n";
        assert!(prepare_chunks("notes/empty.md", bare).chunks.is_empty());
        let blank = "---\nid: t-1\ndescription: \"\"\n---\n\n  \n";
        assert!(prepare_chunks("tasks/blank.md", blank).chunks.is_empty());
        let block = "---\ndescription: |\n  long text\n---\n";
        let block = prepare_chunks("tasks/block.md", block);
        assert!(block.chunks.is_empty(), "a block scalar");
        let titled = prepare_chunks("notes/stub.md", "---\ntitle: Parking stub\n---\n");
        assert_eq!(titled.chunks.len(), 1);
        assert_eq!(titled.chunks[0].text, "Title: Parking stub");
        assert!(titled.chunks[0].in_frontmatter);
    }

    #[test]
    fn frontmatter_chunks_round_trip_through_the_index() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let vault = dir.path().join("vault");
        let task = write_note(
            &vault,
            "tasks/permit.md",
            "---\ntitle: Renew permit\ndescription: Elm St lot\n---\n",
        );
        let mut idx = index_at(dir.path());
        assert!(idx.reconcile_add_edit(&[task], &vault));
        let chunks = idx.chunks_for_path("tasks/permit.md");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].in_frontmatter);
        assert_eq!(chunks[0].kind, crate::vault_index::KIND_TASK);
        let pending = idx.take_unembedded(10);
        idx.store_vectors(&[(
            pending[0].0.clone(),
            pending[0].1,
            pending[0].2.clone(),
            Some(unit(vec![1.0, 0.0])),
        )]);
        let hits = idx.note_hits(&unit(vec![1.0, 0.0]), 5);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].in_frontmatter);
        assert_eq!(
            (hits[0].byte_start, hits[0].byte_end),
            (chunks[0].byte_start, chunks[0].byte_end)
        );
    }

    #[test]
    fn reconcile_indexes_chunks_and_is_steady_state() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let vault = dir.path().join("vault");
        let n1 = write_note(
            &vault,
            "notes/a.md",
            "---\ntitle: A\n---\n\n## One\nalpha\n\n## Two\nbeta",
        );
        let n2 = write_note(
            &vault,
            "people/jane.md",
            "---\ntitle: Jane\n---\n\nWorks on parking.",
        );
        let notes = vec![n1, n2];
        let mut idx = index_at(dir.path());
        assert!(idx.reconcile_add_edit(&notes, &vault));
        assert_eq!(idx.indexed_count(), 2);
        assert_eq!(idx.chunk_count(), 3);
        let chunks = idx.chunks_for_path("notes/a.md");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].heading.as_deref(), Some("One"));
        assert_eq!(chunks[1].heading.as_deref(), Some("Two"));
        let count = idx.chunk_count();
        assert!(idx.reconcile_add_edit(&notes, &vault));
        assert_eq!(idx.chunk_count(), count);
    }

    #[test]
    fn upsert_replaces_old_chunks_when_size_or_mtime_changes() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let vault = dir.path().join("vault");
        let body1 = "## Old\nalpha";
        let mut note = write_note(&vault, "notes/a.md", body1);
        let mut idx = index_at(dir.path());
        assert!(idx.reconcile_add_edit(&[note.clone()], &vault));
        assert_eq!(
            idx.chunks_for_path("notes/a.md")[0].heading.as_deref(),
            Some("Old")
        );

        let body2 = "## New\nbravo";
        std::fs::write(vault.join("notes/a.md"), body2).unwrap();
        note.mtime = "2026-06-23T00:01:00+00:00".to_string();
        note.size = body2.len() as u64;
        assert!(idx.reconcile_add_edit(&[note], &vault));
        let chunks = idx.chunks_for_path("notes/a.md");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].heading.as_deref(), Some("New"));
        assert!(chunks[0].text.contains("bravo"));
        assert!(!chunks[0].text.contains("alpha"));
    }

    #[test]
    fn full_sweep_removes_deleted_paths() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let vault = dir.path().join("vault");
        let n1 = write_note(&vault, "notes/a.md", "alpha");
        let n2 = write_note(&vault, "notes/b.md", "bravo");
        let mut idx = index_at(dir.path());
        assert!(idx.full_sweep(&[n1.clone(), n2], &vault));
        assert_eq!(idx.indexed_count(), 2);
        assert!(idx.full_sweep(&[n1], &vault));
        assert_eq!(idx.indexed_count(), 1);
        assert!(idx.chunks_for_path("notes/b.md").is_empty());
    }

    // ── Vector layer (synthetic vectors; no model load, so CI-safe) ──

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    fn three_chunk_index(dir: &Path) -> EmbedIndex {
        let vault = dir.join("vault");
        let n1 = write_note(
            &vault,
            "notes/a.md",
            "## One\nalpha\n\n## Two\nbeta\n\n## Three\ngamma",
        );
        let mut idx = index_at(dir);
        assert!(idx.reconcile_add_edit(&[n1], &vault));
        assert_eq!(idx.chunk_count(), 3);
        idx
    }

    #[test]
    fn completed_index_never_calls_the_embedder() {
        let dir = tempfile::tempdir().unwrap();
        let idx = three_chunk_index(dir.path());
        let complete: Vec<EmbeddedChunk> = idx
            .take_unembedded(EMBED_BATCH)
            .into_iter()
            .map(|(path, ord, text)| (path, ord, text, Some(vec![1.0, 0.0])))
            .collect();
        idx.store_vectors(&complete);

        drain_batches(
            || idx.take_unembedded(EMBED_BATCH),
            |_| panic!("idle reconciliation must not load the model"),
            |_| panic!("a completed index needs no writes"),
            || panic!("a completed index needs no backfill delay"),
        );
        assert_eq!(idx.embedded_count(), 3);
    }

    #[test]
    fn cancellation_and_load_failure_preserve_pending_chunks_for_retry() {
        use crate::embed_model::EmbedError;
        let dir = tempfile::tempdir().unwrap();
        let idx = three_chunk_index(dir.path());
        let mut calls = 0;
        drain_batches(
            || idx.take_unembedded(EMBED_BATCH),
            |_| {
                calls += 1;
                if calls == 1 {
                    Ok(vec![1.0, 0.0])
                } else {
                    Err(EmbedError::Cancelled)
                }
            },
            |out| idx.store_vectors(out),
            || panic!("cancellation must stop this drain"),
        );
        assert_eq!(calls, 2, "stop before starting another passage");
        assert_eq!(idx.embedded_count(), 1, "keep completed work");
        assert_eq!(idx.take_unembedded(EMBED_BATCH).len(), 2);

        drain_batches(
            || idx.take_unembedded(EMBED_BATCH),
            |_| Err(EmbedError::Load("weights unavailable".into())),
            |_| panic!("load failure must not tombstone pending work"),
            || panic!("retry on the next worker tick, not in a busy loop"),
        );
        assert_eq!(idx.take_unembedded(EMBED_BATCH).len(), 2);

        drain_batches(
            || idx.take_unembedded(EMBED_BATCH),
            |_| Ok(vec![1.0, 0.0]),
            |out| idx.store_vectors(out),
            || {},
        );
        assert_eq!(idx.embedded_count(), 3);
        assert!(idx.take_unembedded(EMBED_BATCH).is_empty());
    }

    #[test]
    fn passage_failure_tombstones_only_that_chunk_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let idx = three_chunk_index(dir.path());
        let mut calls = 0;
        drain_batches(
            || idx.take_unembedded(EMBED_BATCH),
            |_| {
                calls += 1;
                if calls == 2 {
                    Err(crate::embed_model::EmbedError::Inference(
                        "invalid passage".into(),
                    ))
                } else {
                    Ok(vec![1.0, 0.0])
                }
            },
            |out| idx.store_vectors(out),
            || {},
        );
        assert_eq!(calls, 3);
        assert_eq!(idx.embedded_count(), 2);
        assert!(idx.take_unembedded(EMBED_BATCH).is_empty());
    }

    #[test]
    fn vectors_store_take_query_and_tombstone() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let idx = three_chunk_index(dir.path());

        // Every chunk is unembedded initially.
        let pending = idx.take_unembedded(100);
        assert_eq!(pending.len(), 3);
        assert_eq!(idx.embedded_count(), 0);

        // Embed two orthogonal directions; leave the third for the tombstone.
        let (c0, c1) = (&pending[0], &pending[1]);
        idx.store_vectors(&[
            (
                c0.0.clone(),
                c0.1,
                c0.2.clone(),
                Some(unit(vec![1.0, 0.0, 0.0])),
            ),
            (
                c1.0.clone(),
                c1.1,
                c1.2.clone(),
                Some(unit(vec![0.0, 1.0, 0.0])),
            ),
        ]);
        assert_eq!(idx.embedded_count(), 2);
        assert_eq!(idx.take_unembedded(100).len(), 1); // the third is still pending

        // A query near c0's direction ranks c0 first.
        let hits = idx.query(&unit(vec![0.9, 0.1, 0.0]), 10);
        assert_eq!((hits[0].0.as_str(), hits[0].1), (c0.0.as_str(), c0.1));

        // Tombstone the remaining chunk (embed failure) → no longer pending, not
        // in query results, and not counted as embedded.
        let c2 = idx.take_unembedded(1).into_iter().next().unwrap();
        idx.store_vectors(&[(c2.0.clone(), c2.1, c2.2.clone(), None)]);
        assert!(idx.take_unembedded(100).is_empty());
        assert!(idx
            .query(&unit(vec![0.0, 0.0, 1.0]), 10)
            .iter()
            .all(|h| !(h.0 == c2.0 && h.1 == c2.1)));
        assert_eq!(idx.embedded_count(), 2);
    }

    /// OF26, red first for review M1: recall read a learning's meaning as
    /// its closest chunk of any kind, so the "Source excerpt" (raw
    /// conversation) could carry an unrelated learning over the floor. Now
    /// it reads the statement, the text before the first heading: here the
    /// excerpt is closer to the request (0.8) and the statement's 0.6
    /// counts. A note with no text before a heading, another dimension or
    /// an unindexed note gives none.
    #[test]
    fn statement_cosine_reads_a_learnings_statement_not_its_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let rel = "learnings/ship-rule.md";
        let learning = write_note(
            &vault,
            rel,
            "Deploy only after the staging smoke check passes.\n\n\
             ## Why Hollow proposed this\n\nThe person asked for it twice.\n\n\
             ## Source excerpt\n\n> can we ship the release to production today?\n",
        );
        let headed = write_note(&vault, "notes/a.md", "## One\nalpha\n\n## Two\nbeta");
        let mut idx = index_at(dir.path());
        assert!(idx.reconcile_add_edit(&[learning, headed], &vault));
        let headings: Vec<Option<String>> = idx
            .chunks_for_path(rel)
            .into_iter()
            .map(|c| c.heading)
            .collect();
        let section = |h: &str| Some(h.to_string());
        assert_eq!(
            headings,
            vec![
                None,
                section("Why Hollow proposed this"),
                section("Source excerpt")
            ]
        );
        let axis = |i: usize| {
            let mut v = vec![0.0f32; 3];
            v[i.min(2)] = 1.0;
            v
        };
        let vectors: Vec<EmbeddedChunk> = idx
            .take_unembedded(100)
            .into_iter()
            .map(|(path, ord, text)| {
                let v = if path == rel { axis(ord) } else { axis(0) };
                (path, ord, text, Some(v))
            })
            .collect();
        idx.store_vectors(&vectors);
        let q: [f32; 3] = [0.6, 0.0, 0.8];
        let statement = idx.statement_cosine(&q, rel).unwrap();
        assert!((statement - 0.6).abs() < 1e-6, "{statement}");
        assert_eq!(idx.statement_cosine(&q, "notes/a.md"), None, "no statement");
        assert_eq!(idx.statement_cosine(&[0.6, 0.8], rel), None);
        assert_eq!(idx.statement_cosine(&[], rel), None);
        assert_eq!(idx.statement_cosine(&q, "notes/b.md"), None);

        // Recall reads them all at once (second review): a path with no
        // statement vector is left out.
        let asked = [rel.to_string(), "notes/a.md".into(), "notes/b.md".into()];
        let cosines = idx.statement_cosines(&q, &asked);
        assert_eq!(cosines.keys().collect::<Vec<_>>(), vec![rel]);
        assert!((cosines[rel] - 0.6).abs() < 1e-6);
        assert!(idx.statement_cosines(&q, &[]).is_empty());
    }

    /// OF26 second review: a task's frontmatter chunk (title, status...)
    /// has no heading either, but it isn't the statement: however close it
    /// is to the request (0.8 here), only the body's text counts (0.6), and
    /// a note with only frontmatter has no statement at all.
    #[test]
    fn statement_cosine_leaves_out_frontmatter_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let rel = "tasks/ship.md";
        let task = write_note(
            &vault,
            rel,
            "---\ntitle: Ship the release\nstatus: open\nproject: Launch\n---\n\n\
             Deploy only after the staging smoke check passes.",
        );
        let only = "tasks/renew.md";
        let bare = write_note(
            &vault,
            only,
            "---\ntitle: Renew the permit\nstatus: open\nproject: Launch\n---\n",
        );
        let mut idx = index_at(dir.path());
        assert!(idx.reconcile_add_edit(&[task, bare], &vault));
        let shape = |path: &str| -> Vec<(Option<String>, bool)> {
            idx.chunks_for_path(path)
                .into_iter()
                .map(|c| (c.heading, c.in_frontmatter))
                .collect()
        };
        assert_eq!(shape(rel), vec![(None, false), (None, true)]);
        assert_eq!(shape(only), vec![(None, true)]);
        let vectors: Vec<EmbeddedChunk> = idx
            .take_unembedded(100)
            .into_iter()
            .map(|(path, ord, text)| {
                // The body on one axis, every frontmatter chunk on another.
                let fm = path == only || ord == 1;
                let v = if fm {
                    vec![0.0f32, 0.0, 1.0]
                } else {
                    vec![1.0f32, 0.0, 0.0]
                };
                (path, ord, text, Some(v))
            })
            .collect();
        idx.store_vectors(&vectors);
        let q: [f32; 3] = [0.6, 0.0, 0.8];
        let statement = idx.statement_cosine(&q, rel).unwrap();
        assert!((statement - 0.6).abs() < 1e-6, "{statement}");
        assert_eq!(idx.statement_cosine(&q, only), None);
    }

    /// Red first (OF26 review, B1): a turn's recall waited for its query
    /// vector with no limit, through a cold model load or the backfill
    /// worker's batches. Now it waits `wait` at most and the work runs on
    /// to its end; an answer in time, a cancelled and a failed embedding
    /// are told apart. Second review: while one recall's work still runs,
    /// the next times out at once without starting another thread, and
    /// one can start again once it ends. The slow work is held on a
    /// channel, not a sleep, so a loaded machine can't make it finish in
    /// time.
    #[test]
    fn recall_waits_a_bounded_time_for_its_query_vector() {
        use crate::embed_model::EmbedError;
        use crate::vault_learning::RecallMeaning;
        static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
        static SECOND_RAN: AtomicBool = AtomicBool::new(false);
        let wait = Duration::from_millis(100);
        let in_time = Duration::from_secs(10);
        let (release_tx, release) = std::sync::mpsc::channel::<()>();
        let (done_tx, done) = std::sync::mpsc::channel();
        let slow = meaning_within(wait, &IN_FLIGHT, move || {
            // At most 30 s, so a failing test can't hang the run.
            let _ = release.recv_timeout(Duration::from_secs(30));
            let _ = done_tx.send(());
            Ok(vec![1.0f32])
        });
        assert_eq!(slow, Err(RecallMeaning::TimedOut));

        let queued = meaning_within(in_time, &IN_FLIGHT, || {
            SECOND_RAN.store(true, Ordering::SeqCst);
            Ok(vec![2.0f32])
        });
        assert_eq!(queued, Err(RecallMeaning::TimedOut));
        assert!(!SECOND_RAN.load(Ordering::SeqCst), "a second thread ran");

        release_tx.send(()).unwrap();
        done.recv_timeout(in_time)
            .expect("the work runs on after the wait");
        let ended = std::time::Instant::now();
        while IN_FLIGHT.load(Ordering::Acquire) && ended.elapsed() < in_time {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!IN_FLIGHT.load(Ordering::Acquire), "still in flight");

        let answered = meaning_within(in_time, &IN_FLIGHT, || Ok(vec![0.5f32]));
        assert_eq!(answered, Ok(vec![0.5]));
        let cancelled = meaning_within(in_time, &IN_FLIGHT, || {
            Err::<Vec<f32>, _>(meaning_of(EmbedError::Cancelled))
        });
        assert_eq!(cancelled, Err(RecallMeaning::Off));
        let failed = meaning_within(in_time, &IN_FLIGHT, || {
            Err::<Vec<f32>, _>(meaning_of(EmbedError::Load("gone".into())))
        });
        assert_eq!(failed, Err(RecallMeaning::NoVector));
        assert!(!IN_FLIGHT.load(Ordering::Acquire));
        assert_eq!(RECALL_MEANING_WAIT, Duration::from_millis(100));
    }

    /// J4 (red first): recall waited a flat 250 ms whatever the last waits
    /// had shown. Now the wait is 100 ms, falls to 25 ms after three late
    /// recalls running, and returns to 100 ms when a vector arrives in time or
    /// an embedding thread reports finishing within 100 ms of its start.
    #[test]
    fn recall_waits_less_while_the_model_is_contended() {
        let pace = RecallPace::new();
        assert_eq!(pace.wait(), RECALL_MEANING_WAIT);
        assert!(RECALL_MEANING_WAIT <= Duration::from_millis(100));
        pace.waited(false);
        pace.waited(false);
        assert_eq!(pace.wait(), RECALL_MEANING_WAIT, "two late are not three");
        assert!(!pace.contended());
        pace.waited(false);
        assert_eq!(pace.wait(), RECALL_SLOW_WAIT);
        // J3a: a turn then skips waiting for its prefetched recall.
        assert!(pace.contended());
        pace.waited(false);
        assert_eq!(pace.wait(), RECALL_SLOW_WAIT, "stays slow while late");
        // A vector that arrives in time ends the stretch.
        pace.waited(true);
        assert_eq!(pace.wait(), RECALL_MEANING_WAIT);
        // So does an embedding thread that was fast, though its turn left.
        for _ in 0..RECALL_LATE_LIMIT {
            pace.waited(false);
        }
        assert_eq!(pace.wait(), RECALL_SLOW_WAIT);
        pace.finished(RECALL_MEANING_WAIT + Duration::from_millis(1));
        assert_eq!(pace.wait(), RECALL_SLOW_WAIT, "a slow one changes nothing");
        pace.finished(RECALL_MEANING_WAIT);
        assert_eq!(pace.wait(), RECALL_MEANING_WAIT);
        // The count saturates instead of wrapping to a fast stretch.
        pace.late.store(u32::MAX, Ordering::Relaxed);
        pace.waited(false);
        assert_eq!(pace.wait(), RECALL_SLOW_WAIT);
    }

    #[test]
    fn store_vectors_guards_against_stale_text() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let idx = three_chunk_index(dir.path());
        let c = idx.take_unembedded(1).into_iter().next().unwrap();
        // Embedded text no longer matches the chunk (a concurrent edit) → the
        // EXISTS guard refuses the write, so the chunk stays pending for a fresh
        // re-embed rather than persisting a stale vector.
        idx.store_vectors(&[(
            c.0.clone(),
            c.1,
            "STALE TEXT".to_string(),
            Some(unit(vec![1.0, 0.0, 0.0])),
        )]);
        assert_eq!(idx.embedded_count(), 0);
        assert!(idx
            .take_unembedded(100)
            .iter()
            .any(|p| p.0 == c.0 && p.1 == c.1));
    }

    #[test]
    fn remove_clears_vectors() {
        let dir = tempfile::Builder::new()
            .prefix("embed-index-")
            .tempdir()
            .unwrap();
        let mut idx = three_chunk_index(dir.path());
        let c = idx.take_unembedded(1).into_iter().next().unwrap();
        idx.store_vectors(&[(
            c.0.clone(),
            c.1,
            c.2.clone(),
            Some(unit(vec![1.0, 0.0, 0.0])),
        )]);
        assert_eq!(idx.embedded_count(), 1);
        idx.remove("notes/a.md");
        assert_eq!(idx.embedded_count(), 0);
        assert!(idx.query(&unit(vec![1.0, 0.0, 0.0]), 10).is_empty());
    }

    /// R7a: the same `-wal` cap as the keyword index (the default, -1, kept
    /// a 30 MB high-water mark), and the post-sweep checkpoint empties it.
    #[test]
    fn embed_index_limits_its_wal() {
        let dir = tempfile::tempdir().unwrap();
        let mut idx = index_at(dir.path());
        let limit: i64 = idx
            .conn
            .query_row("PRAGMA journal_size_limit;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(limit, 16 * 1024 * 1024);
        idx.upsert("notes/a.md", "2026-10-18T00:00:00+00:00", 10, "## A\nalpha");
        let wal = dir.path().join("embed.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 0);
        assert!(idx.compact_wal());
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
    }
}
