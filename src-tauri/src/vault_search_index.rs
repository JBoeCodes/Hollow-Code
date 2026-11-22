//! FTS5 acceleration for [`crate::vault_search`]: a trigram table that is a
//! PURE PREFILTER for keyword search, and a word table that RANKS directory
//! retrieval's candidates (OB12, below).
//!
//! The trigram table answers "which notes *could* contain this term", one set
//! per query term, so keyword search can skip reading notes that provably
//! can't match. Matching, scoring and snippets are still produced by the same
//! code over the notes that are read, so **its results stay byte-identical**
//! to the unaccelerated scan.
//!
//! PARITY PROOF (per term; see `vault_search` for the matcher):
//!   (a) The `body` column holds `content.to_lowercase()`, the SAME text the
//!       matcher searches. The trigram tokenizer is `case_sensitive 1`, so it
//!       does no case folding of its own, and each term is queried as a quoted
//!       FTS5 literal of the same lowercased text. For a term of >= 3 chars,
//!       trigram matching finds exactly the documents where the term occurs as
//!       a codepoint substring, which for valid UTF-8 is a byte substring. A
//!       word-start match (the 1-3 letter rule) is still a substring match. So
//!       for every indexable term t: t ⊆ lower(content) ⇒ note ∈ FTS(t). No
//!       false negatives. Terms under 3 chars get `None` (unknown) and never
//!       justify a skip.
//!   (b) Search skips a note (no read) only if some indexable term t has
//!       FTS(t) ∌ note AND t matches none of the note's name variants. By (a)
//!       such a t is in neither the name nor the content, so the note could
//!       never be admitted (admission requires every term).
//!   (c) A note that is read is processed by the same code with or without the
//!       index, and its score and snippet depend only on its content and
//!       metadata. Results are sorted by a total order (score, mtime, path),
//!       so iteration order is irrelevant.
//!   (d) Typo recovery reads only name-matched notes and triggers on an empty
//!       literal result, which by (b)-(c) is identical in both paths.
//!   (e) Any SQLite failure makes the whole candidate list `None`, which means
//!       a full scan. One failed term query fails the call; partial sets are
//!       never trusted.
//!
//! RANKED CANDIDATES (OB12, schema 2). Retrieval's terms are up to 16 words
//! of a whole request, and most notes share one of them, so a per-term
//! prefilter let p50 81% of the vault through (~959 notes, 7.58 MB, 1.1 s
//! per chat turn). The `words` table holds the same rows by rowid, tokenized
//! `unicode61 remove_diacritics 2`, with the file's stem as a `name` column
//! (weight 2) beside the body. [`ranked_candidates`] ORs the terms (each an
//! FTS5 string, so no character is an operator; a prefix query from
//! `PREFIX_MIN_CHARS` (3) up, so `auth` finds `authentication`, as the
//! scan's word-start rule does) and returns the eligible notes best first by
//! `bm25()`, at most [`BM25_CANDIDATES`], with their scores. When it ranks
//! fewer than that, the trigram table's matches for every word term of 3+
//! characters join as `topup` (`auth` in `OAuth`; `api` in `会議のapi`,
//! one `unicode61` token), so a narrow query reads no fewer matching notes
//! than the per-term prefilter did, up to the rest of [`BM25_CANDIDATES`]
//! and `TOPUP_BYTES` (4 MB), best trigram `bm25()` first (RF-737). A term the word
//! tokenizer can't hold as one word (a CJK
//! run, punctuation) is answered by the trigram table instead and its notes
//! join as `extra`; one under 3 characters there means no candidates at all
//! (a linear scan). This is NOT a pure prefilter: the scan
//! (`vault_search::retrieval_scan_with`) reads only these candidates plus
//! name matches and the newest notes, so a note that matches a term only
//! inside a longer word, when 200 notes rank above it, is not read. A note
//! that is read is still scored by the same code, so its passages are the
//! linear scan's; only which notes are read, and their BM25 bands, differ.
//! The linear scan stays the cold-index fallback and the test oracle.
//!
//! The guarantee is for a vault that is not modified during the call: the
//! index is reconciled by mtime string, then notes are read, the same race the
//! pre-Paperclip single-needle prefilter had. The name dimension is evaluated
//! directly by the matcher (no read), never delegated to the index.
//!
//! The DB lives OUTSIDE the vault — `{data_dir}/Hollow/SearchIndex/{name}-{hash}.db`,
//! keyed by the canonical vault path. (hollow-mac first put it inside the vault;
//! the WAL DB collided with cloud sync / external indexers and busy-spun. It is a
//! DERIVED CACHE: deleting it only forces a rebuild.) A failure anywhere → the
//! caller falls back to the linear scan, so search NEVER fails or diverges.
//!
//! Engaged (opened read-write, reconciled, kept current by the write hooks)
//! only in the GUI process (`engage` at startup). Another process, such as the
//! `hollow_vault` MCP subprocess, opens the same file READ-ONLY per call
//! ([`term_candidates`] and [`ranked_candidates`] fall back to it) and never
//! writes or reconciles it: every note whose mtime differs from the index, or
//! that the index has never seen, joins every candidate set (unranked, for
//! retrieval) and is read, so a stale index only widens the read set, never
//! narrows it (2026-10-06, agent effectiveness F3). A missing file, another
//! schema or any SQLite error means a linear scan. A schema bump drops every
//! table, so the engaged index rebuilds once (the full sweep at startup). An
//! index at a NEWER schema than this build's (an older build installed over
//! a newer one) is never dropped or written: `open` returns `None` and search
//! linear-scans, so going back to the newer build doesn't rebuild it (OF25).

use crate::types::Note;
use crate::vault_search::RankedCandidates;
use once_cell::sync::Lazy;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 2 (OB12): the `words` table beside `fts`.
pub(crate) const SCHEMA_VERSION: i64 = 2;

/// Eligible notes [`ranked_candidates`] returns at most, best BM25 first.
pub const BM25_CANDIDATES: usize = 200;
/// A word term at least this long also matches the words it begins
/// (`auth` finds `authentication`, `roadmap` finds `roadmaps`); shorter ones
/// match whole words only.
const PREFIX_MIN_CHARS: usize = 3;
/// `bm25()` over `words`' columns (path, name, body): the stem counts twice.
const BM25_SQL: &str = "SELECT path, bm25(words, 0.0, 2.0, 1.0) AS s FROM words \
                        WHERE words MATCH ?1 ORDER BY s, path;";
/// The trigram top-up's matches, best `bm25()` first (RF-737).
const TOPUP_SQL: &str = "SELECT path FROM fts WHERE body MATCH ?1 ORDER BY bm25(fts), path;";
/// Bytes of top-up notes one retrieval reads at most (RF-737): they rank
/// last (band 32), so a mid-word match never costs more than this.
const TOPUP_BYTES: u64 = 4 * 1024 * 1024;
/// The most a quiet index's `-wal` keeps after a checkpoint reset (R7a):
/// SQLite otherwise leaves it at its high-water mark (90 MB measured on
/// 10-18). Applied to the embedding index too.
pub(crate) const WAL_LIMIT_BYTES: i64 = 16 * 1024 * 1024;

/// Copy the WAL into the database and truncate it to zero bytes (R7a). Best
/// effort: `false` while a reader (an agent's `hollow_vault` process, say)
/// keeps it busy; [`WAL_LIMIT_BYTES`] then trims it at the next reset.
pub(crate) fn checkpoint_truncate(conn: &Connection) -> bool {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|busy| busy == 0)
    .unwrap_or(false)
}

pub struct SearchIndex {
    conn: Connection,
    /// rel-path → (mtime string, fts rowid). The only resident whole-vault state.
    meta: HashMap<String, (String, i64)>,
    disabled: bool,
}

impl SearchIndex {
    /// Open (creating if absent) the index DB and load the `meta` mirror. `None`
    /// if the connection or schema can't be established (⇒ caller linear-scans),
    /// or if a newer build wrote it ([`newer_schema`]): then nothing is written.
    pub fn open(db_path: &Path) -> Option<SearchIndex> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(db_path).ok()?;
        if let Some(stored) = newer_schema(&conn, SCHEMA_VERSION) {
            // Schema versions are plain integers, so a branch build can claim a
            // number main reaches later with a different shape; main then keeps
            // the index off until someone deletes it. Say how (OF25).
            eprintln!(
                "[search index] keyword (FTS) index schema {stored} is newer than this build's \
                 {SCHEMA_VERSION}: the index is left as it is and search scans the vault. If no \
                 newer build will run again (e.g. a branch build wrote it), quit Hollow and delete \
                 this vault's file in <app support>/SearchIndex/ (<vault name>-<hash>.db, with any \
                 -wal/-shm beside it); the next launch rebuilds it"
            );
            return None;
        }
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        let _ = conn.pragma_update(None, "journal_size_limit", WAL_LIMIT_BYTES);
        let _ = conn.busy_timeout(std::time::Duration::from_millis(2000));
        let mut idx = SearchIndex {
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

    /// Empty the `-wal` after a sweep ([`checkpoint_truncate`]).
    pub(crate) fn compact_wal(&self) -> bool {
        checkpoint_truncate(&self.conn)
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
                "DROP TABLE IF EXISTS fts; DROP TABLE IF EXISTS words; DROP TABLE IF EXISTS meta;",
            );
        }
        // `body` is the only indexed column; `path` rides along UNINDEXED.
        // trigram + case_sensitive 1 ⇒ exact-codepoint substring over lowercased text.
        // `words` holds the same rows (same rowid) word-tokenized for BM25,
        // with the file's stem as `name`.
        let ok = self
            .conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(\
                     path UNINDEXED, body, tokenize='trigram case_sensitive 1');\
                 CREATE VIRTUAL TABLE IF NOT EXISTS words USING fts5(\
                     path UNINDEXED, name, body, tokenize='unicode61 remove_diacritics 2');\
                 CREATE TABLE IF NOT EXISTS meta(\
                     path TEXT PRIMARY KEY, mtime TEXT NOT NULL, fts_rowid INTEGER NOT NULL);",
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
            .prepare("SELECT path, mtime, fts_rowid FROM meta;")
        {
            Ok(s) => s,
            Err(_) => return,
        };
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                self.meta.insert(row.0, (row.1, row.2));
            }
        }
    }

    /// Reconcile add/edit once, then return one candidate set per term, aligned
    /// with `terms_lower`. A term under 3 chars can't be trigram-tokenized and
    /// gets `None` (unknown). `None` overall if the index is unavailable or
    /// any step fails: one failed term query fails the whole call.
    pub fn reconcile_and_query_terms(
        &mut self,
        notes: &[Note],
        terms_lower: &[String],
        vault: &Path,
    ) -> Option<Vec<Option<HashSet<String>>>> {
        if self.disabled {
            return None;
        }
        if !self.reconcile_add_edit(notes, vault) {
            return None;
        }
        let mut out = Vec::with_capacity(terms_lower.len());
        for term in terms_lower {
            if term.chars().count() < 3 {
                out.push(None);
            } else {
                out.push(Some(self.candidate_paths(term)?));
            }
        }
        Some(out)
    }

    /// Reconcile add/edit once, then retrieval's ranked candidates among
    /// `notes` ([`ranked_on`]). `None` if the index is unavailable, a query
    /// fails or a term is one neither table can answer.
    pub fn reconcile_and_rank(
        &mut self,
        notes: &[Note],
        terms_lower: &[String],
        vault: &Path,
    ) -> Option<RankedCandidates> {
        if !self.reconcile_add_edit(notes, vault) {
            return None;
        }
        match ranked_on(&self.conn, terms_lower, notes) {
            Ok(ranked) => ranked,
            Err(()) => {
                // As in `candidate_paths`: a broken schema or connection.
                self.disabled = true;
                None
            }
        }
    }

    /// Reconcile additions + edits (the per-search hot path): re-read + upsert any
    /// note whose mtime differs from the mirror or that the mirror never saw.
    /// Deletions are harmless to correctness (a gone path simply never appears in
    /// `notes`), swept by `full_sweep` + the remove hooks. `false` on a write error.
    pub fn reconcile_add_edit(&mut self, notes: &[Note], vault: &Path) -> bool {
        if self.disabled {
            return false;
        }
        let changed: Vec<&Note> = notes
            .iter()
            .filter(|n| {
                self.meta
                    .get(&n.path)
                    .map(|(m, _)| m != &n.mtime)
                    .unwrap_or(true)
            })
            .collect();
        if changed.is_empty() {
            return true; // steady state: nothing to write
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return false;
        }
        for n in changed {
            let full = vault.join(&n.path);
            match std::fs::read_to_string(&full) {
                Ok(content) => {
                    if !self.upsert_locked(&n.path, &n.mtime, &content.to_lowercase()) {
                        let _ = self.conn.execute_batch("ROLLBACK;");
                        return false;
                    }
                }
                Err(_) => {
                    // Unreadable as UTF-8 — the scan `continue`s on these too, so
                    // ensure no stale body lingers.
                    let _ = self.remove_locked(&n.path);
                }
            }
        }
        self.conn.execute_batch("COMMIT;").is_ok()
    }

    /// Full reconcile INCLUDING deletions — drops tombstones for files removed
    /// while the app was closed + backfills a fresh DB.
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

    /// FTS5 MATCH for `needle_lower`. `None` on any SQLite error.
    fn candidate_paths(&mut self, needle_lower: &str) -> Option<HashSet<String>> {
        match match_paths(&self.conn, needle_lower) {
            Ok(set) => set,
            Err(()) => {
                // A prepare failure means the schema/connection is broken — disable
                // so we stop probing a dead DB and linear-scan for good.
                self.disabled = true;
                None
            }
        }
    }

    // ── incremental hooks ──

    pub fn upsert(&mut self, path: &str, mtime: &str, content: &str) {
        if self.disabled {
            return;
        }
        if self.conn.execute_batch("BEGIN IMMEDIATE;").is_err() {
            return;
        }
        if self.upsert_locked(path, mtime, &content.to_lowercase()) {
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

    // ── locked primitives (caller holds a transaction) ──

    fn upsert_locked(&mut self, path: &str, mtime: &str, lower_body: &str) -> bool {
        if let Some((_, rowid)) = self.meta.get(path).cloned() {
            if self
                .conn
                .execute("DELETE FROM fts WHERE rowid=?1;", [rowid])
                .is_err()
            {
                return false;
            }
            if self
                .conn
                .execute(
                    "INSERT INTO fts(rowid, path, body) VALUES(?1,?2,?3);",
                    rusqlite::params![rowid, path, lower_body],
                )
                .is_err()
                || !self.put_words_locked(rowid, path, lower_body)
            {
                return false;
            }
            if self
                .conn
                .execute(
                    "UPDATE meta SET mtime=?1 WHERE path=?2;",
                    rusqlite::params![mtime, path],
                )
                .is_err()
            {
                return false;
            }
            self.meta
                .insert(path.to_string(), (mtime.to_string(), rowid));
            true
        } else {
            if self
                .conn
                .execute(
                    "INSERT INTO fts(path, body) VALUES(?1,?2);",
                    rusqlite::params![path, lower_body],
                )
                .is_err()
            {
                return false;
            }
            let rowid = self.conn.last_insert_rowid();
            if !self.put_words_locked(rowid, path, lower_body)
                || self
                    .conn
                    .execute(
                        "INSERT INTO meta(path, mtime, fts_rowid) VALUES(?1,?2,?3);",
                        rusqlite::params![path, mtime, rowid],
                    )
                    .is_err()
            {
                return false;
            }
            self.meta
                .insert(path.to_string(), (mtime.to_string(), rowid));
            true
        }
    }

    /// `words`' row `rowid` (the `fts` row's) replaced by `path`'s stem and
    /// `lower_body`, in the caller's transaction.
    fn put_words_locked(&self, rowid: i64, path: &str, lower_body: &str) -> bool {
        let name = crate::vault_search::stem(path.rsplit('/').next().unwrap_or(path));
        self.conn
            .execute("DELETE FROM words WHERE rowid=?1;", [rowid])
            .is_ok()
            && self
                .conn
                .execute(
                    "INSERT INTO words(rowid, path, name, body) VALUES(?1,?2,?3,?4);",
                    rusqlite::params![rowid, path, name.to_lowercase(), lower_body],
                )
                .is_ok()
    }

    fn remove_locked(&mut self, path: &str) -> bool {
        let Some((_, rowid)) = self.meta.get(path).cloned() else {
            return true;
        };
        if self
            .conn
            .execute("DELETE FROM fts WHERE rowid=?1;", [rowid])
            .is_err()
            || self
                .conn
                .execute("DELETE FROM words WHERE rowid=?1;", [rowid])
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
}

/// FTS5 MATCH for `needle_lower` on `conn`: `Ok(Some(set))`, `Ok(None)` when
/// the query or a row failed (a partial set is never trusted), `Err(())` when
/// the statement can't even be prepared (a broken schema or connection).
fn match_paths(conn: &Connection, needle_lower: &str) -> Result<Option<HashSet<String>>, ()> {
    // Wrap the needle as an FTS5 string literal (double-quoted, embedded `"`
    // doubled) so EVERY char — including FTS operators — is matched literally
    // as a substring by the trigram tokenizer.
    let query = format!("\"{}\"", needle_lower.replace('"', "\"\""));
    let mut stmt = conn
        .prepare("SELECT path FROM fts WHERE body MATCH ?1;")
        .map_err(|_| ())?;
    let rows = match stmt.query_map([&query], |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let mut out = HashSet::new();
    for row in rows {
        match row {
            Ok(p) => {
                out.insert(p);
            }
            Err(_) => return Ok(None), // partial set → don't trust it
        }
    }
    Ok(Some(out))
}

/// Whether the `words` table holds `term` as one word, as the scan matches
/// it: every character is a digit or a letter of a spaced script, which
/// `unicode61` keeps inside a token. Others (a CJK run, punctuation) are
/// answered by the trigram table.
fn word_term(term: &str) -> bool {
    !term.is_empty() && term.chars().all(crate::vault_search::joins_word)
}

/// The `words` MATCH expression for `terms_lower`: the word terms ORed, each
/// an FTS5 string (embedded `"` doubled, so no character is an operator),
/// with a prefix `*` from [`PREFIX_MIN_CHARS`]. `None` with no word term.
/// The project content index ranks with it too (`project_text_index`).
pub(crate) fn words_query(terms_lower: &[String]) -> Option<String> {
    let parts: Vec<String> = terms_lower
        .iter()
        .filter(|t| word_term(t))
        .map(|t| {
            let quoted = format!("\"{}\"", t.replace('"', "\"\""));
            if t.chars().count() >= PREFIX_MIN_CHARS {
                quoted + "*"
            } else {
                quoted
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" OR "))
}

/// The best `k` notes of `eligible` for `query` on `words`, best first, with
/// their BM25 score (positive, higher is better; `bm25()` is its negation).
/// Ties go by path. Errors as [`match_paths`].
fn rank_paths(
    conn: &Connection,
    query: &str,
    eligible: &HashSet<&str>,
    k: usize,
) -> Result<Option<Vec<(String, f64)>>, ()> {
    let mut stmt = conn.prepare(BM25_SQL).map_err(|_| ())?;
    let rows = match stmt.query_map([query], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
    }) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let mut out = Vec::new();
    for row in rows {
        match row {
            Ok((path, bm25)) => {
                if eligible.contains(path.as_str()) {
                    out.push((path, -bm25));
                    if out.len() == k {
                        break;
                    }
                }
            }
            Err(_) => return Ok(None), // a partial ranking is never trusted
        }
    }
    Ok(Some(out))
}

/// The trigram top-up (RF-737): eligible notes (keys of `sizes`) the `fts`
/// `query` matches, best `bm25()` first and ties by path, skipping `skip`
/// and notes the scan won't read (over 1 MB), until `k` are chosen or the
/// next would pass [`TOPUP_BYTES`]. Errors as [`match_paths`].
fn topup_paths(
    conn: &Connection,
    query: &str,
    sizes: &HashMap<&str, u64>,
    skip: &HashSet<&str>,
    k: usize,
) -> Result<Option<HashSet<String>>, ()> {
    let mut stmt = conn.prepare(TOPUP_SQL).map_err(|_| ())?;
    let rows = match stmt.query_map([query], |r| r.get::<_, String>(0)) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };
    let mut out = HashSet::new();
    let mut bytes = 0u64;
    for row in rows {
        let path = match row {
            Ok(p) => p,
            Err(_) => return Ok(None), // a partial set is never trusted
        };
        let Some(&size) = sizes.get(path.as_str()) else {
            continue;
        };
        if skip.contains(path.as_str()) || size > crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES {
            continue;
        }
        if out.len() == k || bytes + size > TOPUP_BYTES {
            break;
        }
        bytes += size;
        out.insert(path);
    }
    Ok(Some(out))
}

/// Retrieval's candidates among `notes` on `conn` (OB12): the best
/// [`BM25_CANDIDATES`] by BM25 over the word terms; every note the trigram
/// table matches for a term that isn't one word (`extra`); and, when fewer
/// than [`BM25_CANDIDATES`] rank, the notes it matches for a word term of 3+
/// characters (`topup`: a term inside a longer token), best `bm25()` first,
/// at most the rest of [`BM25_CANDIDATES`] and [`TOPUP_BYTES`]. `Ok(None)` when a
/// query fails or a term that isn't one word is under 3 characters (neither
/// table can rule a note out for it); `Err(())` as [`match_paths`].
fn ranked_on(
    conn: &Connection,
    terms_lower: &[String],
    notes: &[Note],
) -> Result<Option<RankedCandidates>, ()> {
    let mut extra = HashSet::new();
    for term in terms_lower.iter().filter(|t| !word_term(t)) {
        if term.chars().count() < 3 {
            return Ok(None);
        }
        match match_paths(conn, term)? {
            Some(set) => extra.extend(set),
            None => return Ok(None),
        }
    }
    let ranked = match words_query(terms_lower) {
        Some(query) => {
            let eligible: HashSet<&str> = notes.iter().map(|n| n.path.as_str()).collect();
            match rank_paths(conn, &query, &eligible, BM25_CANDIDATES)? {
                Some(ranked) => ranked,
                None => return Ok(None),
            }
        }
        None => Vec::new(),
    };
    let mut topup = HashSet::new();
    if ranked.len() < BM25_CANDIDATES {
        // Fewer than 200 rank: also read what the pre-OB12 per-term
        // prefilter read for these terms, mid-word matches included. Every
        // term, even a 1-3 letter one the scan matches only at a word start:
        // the scan's word start (after any non-spaced-script letter) is not
        // always a `unicode61` token start (`会議のapi` is one token, so
        // `api*` misses it). Capped at the rest of the candidate budget
        // (RF-737): uncapped, `rate` pulled in every `strategy` note.
        let parts: Vec<String> = terms_lower
            .iter()
            .filter(|t| word_term(t) && t.chars().count() >= 3)
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect();
        if !parts.is_empty() {
            let sizes: HashMap<&str, u64> =
                notes.iter().map(|n| (n.path.as_str(), n.size)).collect();
            let skip: HashSet<&str> = ranked
                .iter()
                .map(|(p, _)| p.as_str())
                .chain(extra.iter().map(String::as_str))
                .collect();
            let k = BM25_CANDIDATES - ranked.len();
            match topup_paths(conn, &parts.join(" OR "), &sizes, &skip, k)? {
                Some(set) => topup = set,
                None => return Ok(None),
            }
        }
    }
    Ok(Some(RankedCandidates {
        ranked,
        extra,
        topup,
    }))
}

/// The schema version stored in `conn`'s database when it is NEWER than
/// `ours`, else `None` (also when it can't be read). A newer build wrote that
/// index, so an older one (a rollback install) must neither drop nor write
/// it: dropping it made the next launch of the newer build rebuild it from
/// scratch (OF25). The embedding index uses this too.
pub(crate) fn newer_schema(conn: &Connection, ours: i64) -> Option<i64> {
    let stored: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .ok()?;
    (stored > ours).then_some(stored)
}

/// `db_path` opened read-only, if it exists at this schema version.
fn open_read_only(db_path: &Path) -> Option<Connection> {
    use rusqlite::OpenFlags;
    if !db_path.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(500));
    let version: i64 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .ok()?;
    if version != SCHEMA_VERSION {
        return None;
    }
    Some(conn)
}

/// The notes whose mtime the index's mirror lacks or holds otherwise: the
/// ones a read-only reader must read whatever the index says.
fn stale_paths(conn: &Connection, notes: &[Note]) -> Option<Vec<String>> {
    let mut mirror: HashMap<String, String> = HashMap::new();
    let mut stmt = conn.prepare("SELECT path, mtime FROM meta;").ok()?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()?;
    for row in rows {
        let (path, mtime) = row.ok()?;
        mirror.insert(path, mtime);
    }
    Some(
        notes
            .iter()
            .filter(|n| mirror.get(&n.path) != Some(&n.mtime))
            .map(|n| n.path.clone())
            .collect(),
    )
}

/// Per-term candidates from `db_path`, opened read-only and never written.
/// One read transaction covers the mtime mirror and every MATCH, so they
/// agree. A note whose mtime differs from the mirror, or that it lacks, is
/// added to every set: the caller reads it and the matcher decides. `None`
/// for a missing file, another schema or any SQLite error.
pub(crate) fn read_only_candidates(
    db_path: &Path,
    terms_lower: &[String],
    notes: &[Note],
) -> Option<Vec<Option<HashSet<String>>>> {
    let conn = open_read_only(db_path)?;
    conn.execute_batch("BEGIN;").ok()?;
    let answer = (|| {
        let stale = stale_paths(&conn, notes)?;
        let mut out = Vec::with_capacity(terms_lower.len());
        for term in terms_lower {
            if term.chars().count() < 3 {
                out.push(None);
                continue;
            }
            let mut set = match_paths(&conn, term).ok()??;
            set.extend(stale.iter().cloned());
            out.push(Some(set));
        }
        Some(out)
    })();
    let _ = conn.execute_batch("COMMIT;");
    answer
}

/// Retrieval's ranked candidates from `db_path`, opened read-only, in one
/// read transaction like [`read_only_candidates`]. A note the mirror doesn't
/// hold at its mtime joins `extra`, unranked: the caller reads it. `None`
/// for a missing file, another schema or any SQLite error.
pub(crate) fn read_only_ranked(
    db_path: &Path,
    terms_lower: &[String],
    notes: &[Note],
) -> Option<RankedCandidates> {
    let conn = open_read_only(db_path)?;
    conn.execute_batch("BEGIN;").ok()?;
    let answer = (|| {
        let stale = stale_paths(&conn, notes)?;
        let mut ranked = ranked_on(&conn, terms_lower, notes).ok()??;
        ranked.extra.extend(stale);
        Some(ranked)
    })();
    let _ = conn.execute_batch("COMMIT;");
    answer
}

// ── Global engagement (GUI process only) ─────────────────────────────────────

static ENGAGED: Lazy<Mutex<Option<(PathBuf, SearchIndex)>>> = Lazy::new(|| Mutex::new(None));

/// The database file `engage` opened, set before it opens it and never
/// cleared: `hollow_databases` never offers it for removal, whatever its
/// stem says (a symlinked vault whose target changed since launch hashes to
/// another stem). Its own lock, so asking never waits on the index.
static ENGAGED_DB: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

/// See [`ENGAGED_DB`].
pub(crate) fn engaged_db_path() -> Option<PathBuf> {
    ENGAGED_DB.lock().ok()?.clone()
}

/// Where the derived FTS5 databases live. Under `cfg(test)` this is a
/// per-process temp directory, never the real app data dir, so a test that
/// engages an index can't leave a database behind in the user's
/// `Application Support` folder. 246 such orphans accumulated once already.
/// The project content index (`project_text_index`, R14b) lives here too.
#[cfg(not(test))]
pub(crate) fn index_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("SearchIndex"))
}

/// `pub(crate)` so the real-vault retrieval eval can delete it at the end of
/// its run (the index holds note bodies).
#[cfg(test)]
pub(crate) fn index_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir("hollow-test-searchindex-"))
}

/// The DB path for `vault`: `{index_root}/{name}-{hash}.db`, keyed by the
/// canonical vault path so distinct vaults never share a DB. Its stem must
/// stay `session_store::vault_file_stem` (`hollow_databases` tells this
/// vault's files by it; a test there keeps them equal).
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

/// Open + register the search index for `vault` (called once at GUI startup).
/// Best-effort: a failure just means search runs unaccelerated. Runs a full
/// sweep so a fresh/stale DB is brought current on launch, then empties the
/// WAL the sweep grew.
pub fn engage(vault: &Path) {
    let Some(db_path) = db_path_for(vault) else {
        return;
    };
    if let Ok(mut g) = ENGAGED_DB.lock() {
        *g = Some(db_path.clone());
    }
    let Some(mut idx) = SearchIndex::open(&db_path) else {
        return;
    };
    let notes = crate::vault::walk_notes(vault);
    let _ = idx.full_sweep(&notes, vault);
    let _ = idx.compact_wal();
    if let Ok(mut g) = ENGAGED.lock() {
        *g = Some((vault.to_path_buf(), idx));
    }
}

/// Per-term candidate prefilter for `vault_search` (keyword search and the
/// vault half of directory retrieval). Returns `Some(sets)`, index-aligned with
/// `terms_lower`, when at least one term has >= 3 chars and either an index is
/// engaged for THIS vault (the GUI process) or, in any other process, the
/// GUI's database for this vault opens read-only ([`read_only_candidates`]).
/// `None` (⇒ full linear scan) otherwise or on any failure.
pub fn term_candidates(
    vault: &Path,
    terms_lower: &[String],
    notes: &[Note],
) -> Option<Vec<Option<HashSet<String>>>> {
    if terms_lower.iter().all(|t| t.chars().count() < 3) {
        return None;
    }
    {
        let mut g = ENGAGED.lock().ok()?;
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                return idx.reconcile_and_query_terms(notes, terms_lower, vault);
            }
        }
    }
    read_only_candidates(&db_path_for(vault)?, terms_lower, notes)
}

/// Retrieval's candidates for `vault` (OB12, the vault half of directory
/// retrieval): the engaged index for THIS vault (the GUI process), else the
/// GUI's database read-only ([`read_only_ranked`]), as [`term_candidates`].
/// `notes` are the eligible notes, the only ones ranked. `None` (⇒ the scan
/// reads every eligible note) with no terms, no index or on any failure.
pub fn ranked_candidates(
    vault: &Path,
    terms_lower: &[String],
    notes: &[Note],
) -> Option<RankedCandidates> {
    if terms_lower.is_empty() {
        return None;
    }
    {
        let mut g = ENGAGED.lock().ok()?;
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                return idx.reconcile_and_rank(notes, terms_lower, vault);
            }
        }
    }
    read_only_ranked(&db_path_for(vault)?, terms_lower, notes)
}

/// Which index [`term_candidates`] and [`ranked_candidates`] use for `vault`
/// in this process: `fts` (engaged here), `fts-read-only` (the GUI's
/// database, read-only) or `linear` (none). For the run's context manifest
/// only.
pub fn prefilter_kind(vault: &Path) -> &'static str {
    if ENGAGED
        .lock()
        .ok()
        .is_some_and(|g| g.as_ref().is_some_and(|(v, _)| v == vault))
    {
        "fts"
    } else if db_path_for(vault).is_some_and(|p| p.is_file()) {
        "fts-read-only"
    } else {
        "linear"
    }
}

/// Incremental hook: a note was written. No-op unless engaged for `vault`.
pub fn on_write(vault: &Path, rel: &str, content: &str, mtime: &str) {
    if let Ok(mut g) = ENGAGED.lock() {
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                idx.upsert(rel, mtime, content);
            }
        }
    }
}

/// Incremental hook: a note was removed. No-op unless engaged for `vault`.
pub fn on_remove(vault: &Path, rel: &str) {
    if let Ok(mut g) = ENGAGED.lock() {
        if let Some((engaged_vault, idx)) = g.as_mut() {
            if engaged_vault == vault {
                idx.remove(rel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(path: &str, mtime: &str) -> Note {
        Note {
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            mtime: mtime.to_string(),
            size: 0,
        }
    }

    /// A test run must never be able to write a database into the user's real
    /// app data directory. 246 orphans accumulated there once from a test that
    /// engaged an index directly.
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

    /// OB12 bumped the schema to 2: an index written at schema 1 (no `words`
    /// table) is never read by another process, and the app drops it and
    /// rebuilds from scratch.
    #[test]
    fn a_schema_1_index_is_rebuilt_and_never_read() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let db = dir.path().join("old.db");
        Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE VIRTUAL TABLE fts USING fts5(\
                     path UNINDEXED, body, tokenize='trigram case_sensitive 1');\
                 CREATE TABLE meta(\
                     path TEXT PRIMARY KEY, mtime TEXT NOT NULL, fts_rowid INTEGER NOT NULL);\
                 INSERT INTO fts(rowid, path, body) VALUES(1, 'a.md', 'quillfeather');\
                 INSERT INTO meta VALUES('a.md', 'then', 1);\
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        let terms = vec!["quillfeather".to_string()];
        let notes = vec![note("a.md", "then")];
        assert!(read_only_candidates(&db, &terms, &notes).is_none());
        assert!(read_only_ranked(&db, &terms, &notes).is_none());
        let idx = SearchIndex::open(&db).expect("reopened at schema 2");
        assert_eq!(idx.indexed_count(), 0, "schema 1 rows are dropped");
        let version: i64 = idx
            .conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(idx);
        let r = read_only_ranked(&db, &terms, &notes).expect("schema 2 is read");
        assert!(r.ranked.is_empty() && r.extra.contains("a.md"), "{r:?}");
    }

    /// OF25: an index a newer build wrote (an older build installed over it,
    /// a rollback) is neither dropped nor written. `open` gives `None`, so
    /// search scans the vault as it does with no index, and the rows and the
    /// stored version are still there for the newer build.
    #[test]
    fn a_newer_schema_index_is_left_alone() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let db = dir.path().join("newer.db");
        let newer = SCHEMA_VERSION + 1;
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE fts USING fts5(\
                 path UNINDEXED, body, tokenize='trigram case_sensitive 1');\
             CREATE TABLE meta(\
                 path TEXT PRIMARY KEY, mtime TEXT NOT NULL, fts_rowid INTEGER NOT NULL);\
             INSERT INTO fts(rowid, path, body) VALUES(1, 'a.md', 'quillfeather');\
             INSERT INTO meta VALUES('a.md', 'then', 1);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", newer).unwrap();
        drop(conn);
        assert!(
            SearchIndex::open(&db).is_none(),
            "a newer index opens disabled"
        );
        let terms = vec!["quillfeather".to_string()];
        let notes = vec![note("a.md", "then")];
        assert!(read_only_candidates(&db, &terms, &notes).is_none());
        let conn = Connection::open(&db).unwrap();
        let version: i64 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, newer, "the stored version is kept");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 1, "the newer index's rows are kept");
        let fts: i64 = conn
            .query_row("SELECT COUNT(*) FROM fts;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 1);
    }

    /// A throwaway vault + a temp-DB index over it (never the app-support path,
    /// so tests don't pollute it).
    fn corpus() -> (tempfile::TempDir, SearchIndex, Vec<Note>) {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let files: &[(&str, &str)] = &[
            (
                "notes/roadmap.md",
                "Discussed the Q3 ROADMAP and platform plan.",
            ),
            (
                "people/alice.md",
                "Alice owns the platform roadmap; likes terse updates.",
            ),
            (
                "notes/cafe.md",
                "Café meeting notes — résumé review with naïve façade.",
            ),
            ("notes/cjk.md", "会議の議事録：来週の計画。"),
            (
                "notes/punct.md",
                "Edge case: foo(bar):baz \"quoted\" & <html>.",
            ),
            ("notes/empty.md", "nothing relevant here"),
        ];
        let mut notes = Vec::new();
        for (i, (rel, body)) in files.iter().enumerate() {
            std::fs::create_dir_all(v.join(rel).parent().unwrap()).unwrap();
            std::fs::write(v.join(rel), body).unwrap();
            notes.push(note(rel, &format!("2026-06-2{i}T00:00:00+00:00")));
        }
        let db = dir.path().join("idx.db"); // inside the temp dir, NOT app support
        let idx = SearchIndex::open(&db).unwrap();
        (dir, idx, notes)
    }

    /// The reference: which paths' lowercased body contains the lowercased needle.
    fn linear_matches(v: &Path, notes: &[Note], needle: &str) -> HashSet<String> {
        let nl = needle.to_lowercase();
        notes
            .iter()
            .filter(|n| {
                std::fs::read_to_string(v.join(&n.path))
                    .map(|c| c.to_lowercase().contains(&nl))
                    .unwrap_or(false)
            })
            .map(|n| n.path.clone())
            .collect()
    }

    #[test]
    fn candidate_set_is_a_superset_of_true_matches_no_false_negatives() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        assert!(idx.reconcile_add_edit(&notes, v));
        // Needles across infix / suffix / accent / CJK / punctuation classes.
        // Only terms with >= 3 chars route to the index (trigram minimum):
        // `term_candidates` gives shorter terms `None`, so they never justify
        // a skip and stay on the scan.
        for needle in [
            "roadmap",
            "platform",
            "café",
            "résumé",
            "façade",
            "議事録",
            "来週の計画",
            "foo(bar):baz",
            "\"quoted\"",
            "<html>",
            "ROADMAP",
            "Platform Roadmap",
            "the platform plan",
            "zzz-no-match",
        ] {
            let nl = needle.to_lowercase();
            assert!(
                nl.chars().count() >= 3,
                "test needles must be >= 3 chars: {needle:?}"
            );
            let truth = linear_matches(v, &notes, needle);
            let cands = idx.candidate_paths(&nl).expect("candidate query ok");
            // No false negatives: every true match is a candidate.
            for t in &truth {
                assert!(
                    cands.contains(t),
                    "needle {needle:?}: missing true match {t}"
                );
            }
        }
    }

    #[test]
    fn per_term_candidates_align_and_short_terms_are_unknown() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        let terms: Vec<String> = ["roadmap", "ui", "café", "platform plan", "api"]
            .iter()
            .map(|t| t.to_string())
            .collect();
        let sets = idx
            .reconcile_and_query_terms(&notes, &terms, v)
            .expect("candidate query ok");
        assert_eq!(sets.len(), terms.len(), "one entry per term, index-aligned");
        assert!(sets[1].is_none(), "a 2-char term is unknown to the index");
        for (i, term) in terms.iter().enumerate() {
            if i == 1 {
                continue;
            }
            let cands = sets[i].as_ref().expect("indexed term");
            for truth in linear_matches(v, &notes, term) {
                assert!(
                    cands.contains(&truth),
                    "term {term:?}: missing true match {truth}"
                );
            }
        }
        // The whole-query guard: nothing indexable ⇒ no prefilter at all.
        assert!(term_candidates(v, &["ui".to_string(), "q3".to_string()], &notes).is_none());
    }

    #[test]
    fn incremental_upsert_and_remove_track_content() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        assert!(idx.reconcile_add_edit(&notes, v));
        let before = idx.indexed_count();
        // Add a new note via the hook.
        idx.upsert(
            "notes/new.md",
            "2026-06-30T00:00:00+00:00",
            "a brand new GIZMO appears",
        );
        assert_eq!(idx.indexed_count(), before + 1);
        assert!(idx
            .candidate_paths("gizmo")
            .unwrap()
            .contains("notes/new.md"));
        // Edit it (same path, new body) — old term gone, new term found.
        idx.upsert(
            "notes/new.md",
            "2026-06-30T01:00:00+00:00",
            "now it is a WIDGET",
        );
        assert!(idx.candidate_paths("gizmo").unwrap().is_empty());
        assert!(idx
            .candidate_paths("widget")
            .unwrap()
            .contains("notes/new.md"));
        // Remove it.
        idx.remove("notes/new.md");
        assert_eq!(idx.indexed_count(), before);
        assert!(idx.candidate_paths("widget").unwrap().is_empty());
    }

    /// Retrieval's ranked paths for `terms` among `notes`.
    fn ranked(idx: &mut SearchIndex, v: &Path, notes: &[Note], terms: &[&str]) -> Vec<String> {
        let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
        let r = idx.reconcile_and_rank(notes, &terms, v).expect("ranked");
        r.ranked.into_iter().map(|(path, _)| path).collect()
    }

    fn words_matching(idx: &SearchIndex, query: &str) -> i64 {
        idx.conn
            .query_row(
                "SELECT count(*) FROM words WHERE words MATCH ?1;",
                [query],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// OB12: the word table ranks by BM25 (the note naming both terms, one
    /// of them rare and in its file name, first); a term of 3+ characters
    /// matches the words it begins; accents fold; FTS5 keywords are plain
    /// words; and edits and removals reach it.
    #[test]
    fn words_rank_by_bm25_and_track_edits() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        let both = ranked(&mut idx, v, &notes, &["roadmap", "alice"]);
        assert_eq!(both.first().map(String::as_str), Some("people/alice.md"));
        assert!(both.iter().any(|p| p == "notes/roadmap.md"), "{both:?}");
        assert_eq!(ranked(&mut idx, v, &notes, &["roadma"]).len(), 2);
        assert_eq!(ranked(&mut idx, v, &notes, &["road"]).len(), 2);
        assert_eq!(
            ranked(&mut idx, v, &notes, &["resume"]),
            vec!["notes/cafe.md"]
        );
        assert!(ranked(&mut idx, v, &notes, &["near", "or"]).is_empty());
        // Edits and removals reach `words` in the same transaction as `fts`.
        let first = "2026-06-30T00:00:00+00:00";
        idx.upsert("notes/new.md", first, "a brand new GIZMO appears");
        let mut with_new = notes.clone();
        with_new.push(note("notes/new.md", first));
        assert_eq!(
            ranked(&mut idx, v, &with_new, &["gizmo"]),
            vec!["notes/new.md"]
        );
        let later = "2026-06-30T01:00:00+00:00";
        idx.upsert("notes/new.md", later, "now it is a WIDGET");
        with_new.last_mut().unwrap().mtime = later.to_string();
        assert!(ranked(&mut idx, v, &with_new, &["gizmo"]).is_empty());
        assert_eq!(
            ranked(&mut idx, v, &with_new, &["widget"]),
            vec!["notes/new.md"]
        );
        idx.remove("notes/new.md");
        assert_eq!(words_matching(&idx, "widget"), 0);
        let rows: i64 = idx
            .conn
            .query_row("SELECT count(*) FROM words;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows as usize, idx.indexed_count());
    }

    /// OB12: a term the word tokenizer can't hold as one word (a CJK run) is
    /// answered by the trigram table, unranked; one under 3 characters there
    /// leaves no candidates at all, so the scan reads every note.
    #[test]
    fn a_term_that_is_not_one_word_falls_back_to_the_trigram_table() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        let terms = vec!["議事録".to_string(), "platform".to_string()];
        let r = idx.reconcile_and_rank(&notes, &terms, v).expect("ranked");
        assert!(r.extra.contains("notes/cjk.md"), "{r:?}");
        assert!(r.ranked.iter().all(|(p, _)| p != "notes/cjk.md"));
        assert!(r.ranked.iter().any(|(p, _)| p == "people/alice.md"));
        assert!(idx
            .reconcile_and_rank(&notes, &["会議".to_string()], v)
            .is_none());
    }

    /// OB12 review: `auth` ranks `authentication` by prefix, and, as fewer
    /// than 200 notes rank, `OAuth` (the term inside a longer word) joins
    /// through the trigram top-up; a note without it joins neither.
    #[test]
    fn a_short_term_ranks_by_prefix_and_tops_up_mid_word_matches() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let mut notes = Vec::new();
        for (rel, body) in [
            ("login.md", "The authentication flow failed."),
            ("tokens.md", "OAuth tokens expire hourly."),
            ("lunch.md", "Lunch at noon."),
            ("motion.md", "Motion carried by a wide margin."),
            ("kaigi.md", "会議のapi"),
        ] {
            std::fs::write(v.join(rel), body).unwrap();
            notes.push(note(rel, "2026-06-20T00:00:00+00:00"));
        }
        let mut idx = SearchIndex::open(&v.join("idx.db")).unwrap();
        let r = idx
            .reconcile_and_rank(&notes, &["auth".to_string()], v)
            .expect("ranked");
        let ranked: Vec<&str> = r.ranked.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(ranked, vec!["login.md"]);
        assert!(r.topup.contains("tokens.md"), "{r:?}");
        assert!(!r.topup.contains("lunch.md") && r.extra.is_empty(), "{r:?}");
        // A 1-3 letter term ranks only where a word begins with it (`ion` is
        // no prefix of `motion`), but it is topped up like any other: the
        // scan's word start after a CJK letter is inside one `unicode61`
        // token (`会議のapi`), which `api*` can't find (OB12 round 3).
        let r = idx
            .reconcile_and_rank(&notes, &["ion".to_string()], v)
            .expect("ranked");
        assert!(r.ranked.is_empty(), "{r:?}");
        let r = idx
            .reconcile_and_rank(&notes, &["api".to_string()], v)
            .expect("ranked");
        assert!(r.ranked.is_empty() && r.topup.contains("kaigi.md"), "{r:?}");
    }

    #[test]
    fn reconcile_is_steady_state_after_first_pass() {
        let (dir, mut idx, notes) = corpus();
        let v = dir.path();
        assert!(idx.reconcile_add_edit(&notes, v));
        let n = idx.indexed_count();
        // Same notes, same mtimes → no-op, count unchanged.
        assert!(idx.reconcile_add_edit(&notes, v));
        assert_eq!(idx.indexed_count(), n);
    }

    /// OB12: another process ranks retrieval's candidates from the app's
    /// index read-only too: fresh notes ranked, a note changed since the
    /// index saw it unranked in `extra`, so it is still read.
    #[test]
    fn another_process_ranks_from_the_index_read_only() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::write(v.join("a.md"), "quillfeather supplier").unwrap();
        std::fs::write(v.join("b.md"), "nothing relevant").unwrap();
        let terms = vec!["quillfeather".to_string()];
        let notes = crate::vault::walk_notes(v);
        assert!(
            ranked_candidates(v, &terms, &notes).is_none(),
            "no database"
        );
        let db = db_path_for(v).unwrap();
        {
            let mut idx = SearchIndex::open(&db).expect("index");
            assert!(idx.full_sweep(&notes, v));
        }
        let r = ranked_candidates(v, &terms, &notes).expect("read-only ranking");
        let paths: Vec<&str> = r.ranked.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["a.md"]);
        assert!(r.ranked[0].1 > 0.0 && r.extra.is_empty(), "{r:?}");
        let mut stale = notes.clone();
        for n in stale.iter_mut().filter(|n| n.path == "b.md") {
            n.mtime = "2099-01-01T00:00:00Z".into();
        }
        let r = read_only_ranked(&db, &terms, &stale).expect("read-only ranking");
        assert!(r.extra.contains("b.md"), "{r:?}");
    }

    /// Another process (the `hollow_vault` subprocess) reads the app's index
    /// read-only: a fresh note is narrowed exactly as the engaged index
    /// narrows it; a note changed since the index saw it, or one it never
    /// saw, joins every set, so it is read and nothing is missed; nothing is
    /// written; a missing database means a linear scan.
    #[test]
    fn another_process_reads_the_index_read_only_and_never_trusts_a_stale_row() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::write(v.join("a.md"), "quillfeather supplier").unwrap();
        std::fs::write(v.join("b.md"), "nothing relevant").unwrap();
        let terms = vec!["quillfeather".to_string()];
        let notes = crate::vault::walk_notes(v);
        assert!(
            term_candidates(v, &terms, &notes).is_none(),
            "no database: linear"
        );
        assert_eq!(prefilter_kind(v), "linear");
        let db = db_path_for(v).unwrap();
        {
            let mut idx = SearchIndex::open(&db).expect("index");
            assert!(idx.full_sweep(&notes, v));
        }
        assert_eq!(prefilter_kind(v), "fts-read-only");
        let sets = term_candidates(v, &terms, &notes).expect("read-only sets");
        let set = sets[0].as_ref().unwrap();
        assert!(set.contains("a.md") && !set.contains("b.md"), "{set:?}");
        // b.md changes (another mtime) and c.md is new: both join the set.
        let stale: Vec<Note> = notes
            .iter()
            .map(|n| Note {
                mtime: if n.path == "b.md" {
                    "2099-01-01T00:00:00Z".into()
                } else {
                    n.mtime.clone()
                },
                ..n.clone()
            })
            .chain(std::iter::once(note("c.md", "2099-01-01T00:00:00Z")))
            .collect();
        let before = std::fs::metadata(&db).unwrap().modified().unwrap();
        let sets = read_only_candidates(&db, &terms, &stale).expect("read-only sets");
        let set = sets[0].as_ref().unwrap();
        assert!(set.contains("a.md") && set.contains("b.md") && set.contains("c.md"));
        assert_eq!(std::fs::metadata(&db).unwrap().modified().unwrap(), before);
        // Short terms stay unknown, as in the engaged index.
        let sets = read_only_candidates(&db, &["ui".to_string()], &notes).unwrap();
        assert!(sets[0].is_none());
    }

    /// R7a: the index caps what its `-wal` keeps after a checkpoint reset
    /// (SQLite's default, -1, kept a 90 MB high-water mark).
    #[test]
    fn search_index_limits_its_wal() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let idx = SearchIndex::open(&dir.path().join("s.db")).expect("index");
        let limit: i64 = idx
            .conn
            .query_row("PRAGMA journal_size_limit;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(limit, 16 * 1024 * 1024);
    }

    /// R7a: the post-sweep checkpoint leaves a zero-byte `-wal`.
    #[test]
    fn compact_wal_empties_the_wal() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let db = dir.path().join("s.db");
        let mut idx = SearchIndex::open(&db).expect("index");
        let body = "quillfeather supplier ledger ".repeat(1_800); // ~52 KB
        for i in 0..40 {
            idx.upsert(&format!("n{i}.md"), "2026-10-18T00:00:00Z", &body);
        }
        let wal = dir.path().join("s.db-wal");
        assert!(
            std::fs::metadata(&wal).unwrap().len() > 0,
            "rows went to the WAL"
        );
        assert!(idx.compact_wal());
        assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
    }
}
