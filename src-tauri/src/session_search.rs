//! Rebuildable, secret-scrubbed full-text documents for Hollow's own chats:
//! one document per chat title (ordinal [`TITLE_ORD`]) and one per turn.
//! Source tables remain authoritative. SQLite triggers mark what changed in
//! the same transaction as the change: a turn (`chat_search_dirty_turns`) or,
//! for a deleted chat and the first-use backfill, a whole chat
//! (`chat_search_dirty`). [`refresh`] reindexes at most a budget of marked
//! documents per call, so a search never indexes the whole vault, and a
//! streaming chat costs one turn per search, not the chat.

use super::*;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchContext {
    pub request: String,
    pub outcome: String,
    pub messages: Vec<SearchMessage>,
    pub replaced_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMessage {
    pub turn: i64,
    pub text: String,
}

/// The derived index's layout version. A store with another version drops
/// its documents and marks every chat for the backfill.
const INDEX_VERSION: i64 = 4;
/// The ordinal of a chat's title document. Imported turns may use zero.
pub(super) const TITLE_ORD: i64 = -1;
/// Documents one search reindexes before it reads, at most. Expanding a
/// whole-chat mark into turn marks counts as one. The rest stay marked for
/// the next search or [`super::drain_search_index`].
pub(super) const REFRESH_UNITS: usize = 128;
/// Documents one write transaction reindexes, at most, so the store's writer
/// is released between them.
const UNITS_PER_WRITE: usize = 32;
/// A tool result summary's length in the index, before its truncation mark.
const TOOL_SUMMARY_CHARS: usize = 300;

/// The v3 triggers, dropped by the v4 migration.
const V3_TRIGGER_TABLES: [&str; 4] = ["sessions", "turns", "transcripts", "events"];

pub(super) fn schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS chat_search_meta(version INTEGER PRIMARY KEY);")
        .map_err(|e| format!("chat search schema: {e}"))?;
    let current = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_search_meta WHERE version=?1)",
            [INDEX_VERSION],
            |r| r.get::<_, bool>(0),
        )
        .map_err(|e| format!("chat search version: {e}"))?;
    if !current {
        // An older layout (v3 kept display text in FTS and had no per-turn
        // marks): drop it and backfill every chat, newest first.
        reset(conn).map_err(|e| format!("chat search migration: {e}"))?;
    }
    conn.execute_batch(TABLES)
        .map_err(|e| format!("chat search schema: {e}"))?;
    for trigger in triggers() {
        conn.execute_batch(&trigger)
            .map_err(|e| format!("chat search trigger: {e}"))?;
    }
    Ok(())
}

/// Drops every derived table and trigger of the index and marks every chat
/// for the backfill, newest first, in one transaction. Source tables are
/// never touched. The v4 migration and [`verify`]'s repair both run it.
fn reset(conn: &Connection) -> Result<(), rusqlite::Error> {
    let mut batch = String::from("BEGIN IMMEDIATE;");
    for table in V3_TRIGGER_TABLES {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            batch.push_str(&format!(
                "DROP TRIGGER IF EXISTS chat_search_{table}_{event};"
            ));
        }
    }
    batch.push_str(
        "DROP TABLE IF EXISTS chat_search_fts;\
         DROP TABLE IF EXISTS chat_search_context;\
         DROP TABLE IF EXISTS chat_search_docs;\
         DROP TABLE IF EXISTS chat_search_dirty;\
         DROP TABLE IF EXISTS chat_search_dirty_turns;\
         DELETE FROM chat_search_meta;",
    );
    batch.push_str(TABLES);
    batch.push_str(&format!(
        "INSERT INTO chat_search_dirty SELECT key FROM sessions;\
         INSERT INTO chat_search_meta VALUES({INDEX_VERSION});\
         COMMIT;"
    ));
    if let Err(e) = conn.execute_batch(&batch) {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(e);
    }
    Ok(())
}

/// Whether `e` says the file or the index is damaged (`SQLITE_CORRUPT`,
/// which includes `CORRUPT_VTAB`, or FTS5's "malformed" text). Busy, locked
/// and I/O errors are contention, not damage.
fn is_corruption(e: &rusqlite::Error) -> bool {
    match e {
        rusqlite::Error::SqliteFailure(f, msg) => {
            f.code == rusqlite::ErrorCode::DatabaseCorrupt
                || msg.as_deref().is_some_and(|m| m.contains("malformed"))
        }
        _ => false,
    }
}

/// What a cheap, read-only look at the index found.
#[derive(Debug, PartialEq, Eq)]
enum Probe {
    Sound,
    Corrupt,
    /// Contention or an I/O error: say nothing about damage.
    Unknown(String),
}

/// The startup probe: no write lock, bounded reads. It reads the newest
/// [`PROBE_DOCS`] document rows and, for each, matches its first word in the
/// FTS table (the inverted index) and reads its content row. A damaged
/// page the probe doesn't touch goes unseen: every launch reads the same
/// newest rows, so such damage surfaces only as a search error, never as a
/// startup repair.
fn probe(conn: &Connection) -> Probe {
    let read = || -> Result<(), rusqlite::Error> {
        let docs: Vec<(i64, String)> = conn
            .prepare(
                "SELECT id, substr(display, 1, 60) FROM chat_search_docs ORDER BY id DESC LIMIT ?1",
            )?
            .query_map([PROBE_DOCS], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, text) in docs {
            conn.query_row(
                "SELECT body FROM chat_search_fts WHERE rowid=?1",
                [id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?;
            if let Some(word) = text
                .split(|c: char| !c.is_alphanumeric())
                .find(|w| w.chars().count() >= 3)
            {
                let phrase = format!("\"{}\"", word.replace('"', "\"\""));
                conn.query_row(
                    "SELECT rowid FROM chat_search_fts WHERE chat_search_fts MATCH ?1 LIMIT 1",
                    [phrase],
                    |r| r.get::<_, i64>(0),
                )
                .optional()?;
            }
        }
        Ok(())
    };
    match read() {
        Ok(()) => Probe::Sound,
        Err(e) if is_corruption(&e) => Probe::Corrupt,
        Err(e) => Probe::Unknown(e.to_string()),
    }
}

/// Document rows [`probe`] reads.
const PROBE_DOCS: i64 = 64;

/// FTS5's full `integrity-check` (the inverted index against its content
/// table). It is an INSERT, so it holds the store's write lock for the whole
/// read of a large index: only [`verify`] runs it, after the probe found
/// damage, inside [`write_lock`].
fn full_check(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO chat_search_fts(chat_search_fts) VALUES('integrity-check')",
        [],
    )
    .map(|_| ())
}

/// Removes a damaged FTS table that refuses its own DROP: its shadow tables
/// and schema row by hand. Only after a corruption error.
fn force_drop(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch("PRAGMA writable_schema=ON;")?;
    // The documents table too, when its own page is the damaged one: try an
    // ordinary DROP first; a refusal leaves its schema rows (exact names)
    // to be deleted below, and any pages it held orphaned until a VACUUM.
    let _ = conn.execute_batch("DROP TABLE IF EXISTS chat_search_docs;");
    let dropped = conn.execute_batch(
        "BEGIN IMMEDIATE;\
         DROP TABLE IF EXISTS chat_search_fts_data;\
         DROP TABLE IF EXISTS chat_search_fts_idx;\
         DROP TABLE IF EXISTS chat_search_fts_content;\
         DROP TABLE IF EXISTS chat_search_fts_docsize;\
         DROP TABLE IF EXISTS chat_search_fts_config;\
         DELETE FROM sqlite_schema WHERE name IN ('chat_search_fts', \
           'chat_search_docs', 'sqlite_autoindex_chat_search_docs_1');\
         COMMIT;",
    );
    if dropped.is_err() {
        let _ = conn.execute_batch("ROLLBACK");
    }
    let _ = conn.execute_batch("PRAGMA writable_schema=OFF;");
    dropped
}

/// The startup check (PER-23): a damaged index is rebuilt from the stored
/// chats, which are authoritative and untouched; `true` when it was. The
/// rebuild is the same lazy backfill a new layout gets, so search and the
/// background drain refill it in bounded passes.
///
/// A clean index costs the [`probe`]'s bounded reads and no write lock. Only
/// a corruption error from it (never contention) leads to the repair, inside
/// the store's write lock. The probe's word stands: the FTS `integrity-check`
/// runs first only to log what it says (it doesn't cover
/// `chat_search_docs`), and the derived tables are rebuilt either way, so a
/// damaged index can't pass the probe and be left holding the lock every
/// launch. Contention or an I/O error returns `Err` and changes nothing.
pub(super) fn verify(vault: &Path) -> Result<bool, String> {
    repair(vault, reset)
}

/// [`verify`] with the first reset replaceable, so a test can make the
/// damaged table refuse its DROP.
fn repair(
    vault: &Path,
    first_reset: impl Fn(&Connection) -> Result<(), rusqlite::Error>,
) -> Result<bool, String> {
    let conn = open(vault)?;
    match probe(&conn) {
        Probe::Sound => return Ok(false),
        Probe::Unknown(e) => return Err(format!("chat search probe skipped: {e}")),
        Probe::Corrupt => {}
    }
    let _write = write_lock();
    match full_check(&conn) {
        Err(e) if !is_corruption(&e) => return Err(format!("chat search check skipped: {e}")),
        Ok(()) => eprintln!(
            "[session_store] chat search probe found damage the FTS integrity-check does not"
        ),
        Err(e) => eprintln!("[session_store] chat search integrity-check: {e}"),
    }
    match first_reset(&conn) {
        Ok(()) => {}
        Err(e) if is_corruption(&e) => {
            force_drop(&conn).map_err(|e| format!("chat search repair: {e}"))?;
            // A new connection reads the schema the hand edit left.
            drop(conn);
            let fresh = open(vault)?;
            reset(&fresh).map_err(|e| format!("chat search repair: {e}"))?;
        }
        Err(e) => return Err(format!("chat search repair skipped: {e}")),
    }
    eprintln!("[session_store] chat search index was damaged: rebuilding from the stored chats");
    Ok(true)
}

/// The index's tables. `chat_search_docs` holds each document's display
/// text, keyed by chat and ordinal; its `id` is the document's FTS rowid, so
/// replacing one document never scans the FTS table.
const TABLES: &str = "\
    CREATE VIRTUAL TABLE IF NOT EXISTS chat_search_fts USING fts5(\
      session_key UNINDEXED, body, tokenize='trigram');\
    CREATE TABLE IF NOT EXISTS chat_search_docs(id INTEGER PRIMARY KEY, \
      session_key TEXT NOT NULL, ord INTEGER NOT NULL, display TEXT NOT NULL, \
      UNIQUE(session_key, ord));\
    CREATE TABLE IF NOT EXISTS chat_search_dirty(session_key TEXT PRIMARY KEY);\
    CREATE TABLE IF NOT EXISTS chat_search_dirty_turns(session_key TEXT NOT NULL, \
      ord INTEGER NOT NULL, PRIMARY KEY(session_key, ord)) WITHOUT ROWID;";

/// Marks a whole chat. Trigger inserts test for an existing row instead of
/// relying on OR IGNORE, because an outer upsert can override a trigger's
/// conflict policy.
fn mark_chat(key: &str) -> String {
    format!(
        "INSERT INTO chat_search_dirty SELECT {key} WHERE NOT EXISTS \
         (SELECT 1 FROM chat_search_dirty WHERE session_key={key});"
    )
}

/// Marks one document of a chat (a turn, or the title at [`TITLE_ORD`]).
/// An event with no turn belongs to no turn's transcript and marks nothing.
fn mark_doc(key: &str, ord: &str) -> String {
    format!(
        "INSERT INTO chat_search_dirty_turns SELECT {key},{ord} WHERE {ord} IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM chat_search_dirty_turns WHERE session_key={key} AND ord={ord});"
    )
}

/// What marks what. A title change marks the title document; a turn's
/// request, its stored transcript and its trace events mark that turn only;
/// a deleted or re-keyed chat is marked whole.
fn triggers() -> Vec<String> {
    let title = TITLE_ORD.to_string();
    let mut out = vec![
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_sessions_insert AFTER INSERT ON sessions \
             BEGIN {} END;",
            mark_doc("NEW.key", &title)
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_sessions_title AFTER UPDATE OF title ON sessions \
             BEGIN {} END;",
            mark_doc("NEW.key", &title)
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_sessions_key AFTER UPDATE OF key ON sessions \
             WHEN OLD.key IS NOT NEW.key BEGIN {} {} END;",
            mark_chat("OLD.key"),
            mark_chat("NEW.key")
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_sessions_delete AFTER DELETE ON sessions \
             BEGIN {} END;",
            mark_chat("OLD.key")
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_turns_insert AFTER INSERT ON turns \
             BEGIN {} END;",
            mark_doc("NEW.session_key", "NEW.ord")
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_turns_update \
             AFTER UPDATE OF session_key, ord, asked, asked_by ON turns \
             BEGIN {} {} END;",
            mark_doc("OLD.session_key", "OLD.ord"),
            mark_doc("NEW.session_key", "NEW.ord")
        ),
        format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_turns_delete AFTER DELETE ON turns \
             BEGIN {} END;",
            mark_doc("OLD.session_key", "OLD.ord")
        ),
    ];
    for table in ["transcripts", "events"] {
        out.push(format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_{table}_insert AFTER INSERT ON {table} \
             BEGIN {} END;",
            mark_doc("NEW.session_key", "NEW.turn_ord")
        ));
        out.push(format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_{table}_update AFTER UPDATE ON {table} \
             BEGIN {} {} END;",
            mark_doc("OLD.session_key", "OLD.turn_ord"),
            mark_doc("NEW.session_key", "NEW.turn_ord")
        ));
        out.push(format!(
            "CREATE TRIGGER IF NOT EXISTS chat_search4_{table}_delete AFTER DELETE ON {table} \
             BEGIN {} END;",
            mark_doc("OLD.session_key", "OLD.turn_ord")
        ));
    }
    out
}

fn scrub(text: &str) -> String {
    crate::claude_runner::redact(text)
}

/// Redacted text as the index matches it: every redaction placeholder
/// blanked, so a placeholder holds no word. Copied only when it holds one
/// (each opens with `[`): the context reads every turn of each hit through
/// this, and seven copies of each made up most of that read (2026-11-08).
pub(super) fn searchable(text: &str) -> std::borrow::Cow<'_, str> {
    let mut text = std::borrow::Cow::Borrowed(text);
    if !text.contains('[') {
        return text;
    }
    for placeholder in crate::claude_runner::REDACT_PLACEHOLDERS {
        if text.contains(placeholder) {
            text = text
                .replace(placeholder, &" ".repeat(placeholder.len()))
                .into();
        }
    }
    text
}

/// [`cap`] for redacted text: a cut inside a redaction placeholder drops the
/// placeholder's start too, so no fragment of one (`[redac`) is left for
/// [`searchable`] to miss and a query to match.
pub(super) fn cap_scrubbed(text: &str, limit: usize) -> String {
    let Some(end) = crate::claude_runner::char_start(text, limit) else {
        return text.to_string();
    };
    let (mut kept, rest) = text.split_at(end);
    'placeholders: for placeholder in crate::claude_runner::REDACT_PLACEHOLDERS {
        // Placeholders are ASCII, so a byte split of one is a char split.
        for at in 1..placeholder.len() {
            let (head, tail) = placeholder.split_at(at);
            if kept.ends_with(head) && rest.starts_with(tail) {
                kept = &kept[..kept.len() - head.len()];
                break 'placeholders;
            }
        }
    }
    format!("{kept}… (truncated)")
}

/// The text of document `ord` of chat `key` as the source tables hold it
/// now, before scrubbing; `None` when the chat or turn no longer exists.
fn document(conn: &Connection, key: &str, ord: i64) -> Result<Option<String>, String> {
    let session = conn
        .query_row(
            "SELECT coalesce(title,'') FROM sessions WHERE key=?1",
            [key],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(title) = session else {
        return Ok(None);
    };
    if ord == TITLE_ORD {
        return Ok(Some(title));
    }
    let asked = conn
        .query_row(
            &format!(
                "SELECT CASE WHEN {SEARCH_PERSON_TURN} THEN t.asked ELSE '' END \
                 FROM turns t WHERE t.session_key=?1 AND t.ord=?2"
            ),
            params![key, ord],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(asked) = asked else {
        return Ok(None);
    };
    // A transcript that no longer decodes indexes what was asked: the index
    // is derived, and one damaged turn must not stop every search.
    let transcript = load_transcript(conn, key, ord).unwrap_or_default();
    let mut body = String::new();
    if !asked.is_empty() {
        body.push_str(&format!("{asked}\n"));
    }
    if !transcript.text.is_empty() {
        body.push_str(&format!("Assistant: {}\n", transcript.text));
    }
    if let Some(error) = transcript.error {
        body.push_str(&format!("Error: {error}\n"));
    }
    for tool in transcript.tools {
        // Tool inputs can hold credentials and pasted source files. Only a
        // short, redacted result summary belongs in search, cut so that no
        // fragment of a placeholder remains.
        let result = scrub(tool.result.as_deref().unwrap_or_default());
        body.push_str(&format!(
            "Tool: {} [{}] {}\n",
            tool.name,
            tool.status,
            cap_scrubbed(&result, TOOL_SUMMARY_CHARS)
        ));
    }
    Ok(Some(body))
}

/// Replace document `ord` of chat `key` with `text` (scrubbed here), or
/// remove it when `text` is `None`. The old and new text change in the
/// caller's transaction, so a reader sees one or the other.
fn replace(conn: &Connection, key: &str, ord: i64, text: Option<&str>) -> Result<(), String> {
    let old = conn
        .query_row(
            "SELECT id FROM chat_search_docs WHERE session_key=?1 AND ord=?2",
            params![key, ord],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(id) = old {
        conn.execute("DELETE FROM chat_search_fts WHERE rowid=?1", [id])
            .map_err(|e| format!("chat search document: {e}"))?;
        conn.execute("DELETE FROM chat_search_docs WHERE id=?1", [id])
            .map_err(|e| format!("chat search document: {e}"))?;
    }
    let Some(text) = text else {
        return Ok(());
    };
    let display = scrub(text);
    conn.execute(
        "INSERT INTO chat_search_docs(session_key,ord,display) VALUES(?1,?2,?3)",
        params![key, ord, display],
    )
    .map_err(|e| format!("chat search document: {e}"))?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO chat_search_fts(rowid,session_key,body) VALUES(?1,?2,?3)",
        params![id, key, searchable(&display)],
    )
    .map_err(|e| format!("chat search document: {e}"))?;
    Ok(())
}

/// Reindex the documents `ords` of chat `key` that are still marked (another
/// search may have reindexed some meanwhile), in one write transaction.
fn index_marked(vault: &Path, key: &str, ords: &[i64]) -> Result<(), String> {
    with_write(vault, "chat search index", |conn| {
        for &ord in ords {
            let marked = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM chat_search_dirty_turns WHERE session_key=?1 AND ord=?2)",
                    params![key, ord],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?;
            if !marked {
                continue;
            }
            let text = document(conn, key, ord)?;
            replace(conn, key, ord, text.as_deref())?;
            conn.execute(
                "DELETE FROM chat_search_dirty_turns WHERE session_key=?1 AND ord=?2",
                params![key, ord],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

/// Turn a whole-chat mark into document marks: a chat that no longer exists
/// loses its documents at once; otherwise documents of turns that no longer
/// exist go, and the title and every turn are marked. The chat's current
/// documents stay searchable until each is replaced.
fn expand(vault: &Path, key: &str) -> Result<(), String> {
    with_write(vault, "chat search index", |conn| {
        let marked = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_search_dirty WHERE session_key=?1)",
                [key],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?;
        if !marked {
            return Ok(());
        }
        let exists = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE key=?1)",
                [key],
                |r| r.get::<_, bool>(0),
            )
            .map_err(|e| e.to_string())?;
        // Every document of a chat that is gone; else those of its turns
        // that are gone (never the title's).
        let gone = if exists {
            ords(
                conn,
                "SELECT ord FROM chat_search_docs d WHERE d.session_key=?1 AND d.ord<>?2 \
                 AND NOT EXISTS(SELECT 1 FROM turns t WHERE t.session_key=d.session_key AND t.ord=d.ord)",
                params![key, TITLE_ORD],
            )?
        } else {
            ords(
                conn,
                "SELECT ord FROM chat_search_docs WHERE session_key=?1",
                [key],
            )?
        };
        for ord in gone {
            replace(conn, key, ord, None)?;
        }
        if exists {
            conn.execute(
                "INSERT OR IGNORE INTO chat_search_dirty_turns VALUES(?1,?2)",
                params![key, TITLE_ORD],
            )
            .map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO chat_search_dirty_turns \
                 SELECT session_key, ord FROM turns WHERE session_key=?1",
                [key],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "DELETE FROM chat_search_dirty_turns WHERE session_key=?1",
                [key],
            )
            .map_err(|e| e.to_string())?;
        }
        conn.execute("DELETE FROM chat_search_dirty WHERE session_key=?1", [key])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
}

fn ords(conn: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<Vec<i64>, String> {
    conn.prepare(sql)
        .map_err(|e| e.to_string())?
        .query_map(args, |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Reindex `marks` (chat key and ordinal, grouped by chat), at most
/// [`UNITS_PER_WRITE`] per write transaction.
fn index_marks(vault: &Path, marks: &[(String, i64)]) -> Result<(), String> {
    for chat in marks.chunk_by(|a, b| a.0 == b.0) {
        for part in chat.chunks(UNITS_PER_WRITE) {
            let ords: Vec<i64> = part.iter().map(|(_, ord)| *ord).collect();
            index_marked(vault, &part[0].0, &ords)?;
        }
    }
    Ok(())
}

/// Up to `limit` document marks, of chat `key` or of every chat, newest
/// chat key and turn first (keys open with their date).
fn marks(vault: &Path, key: Option<&str>, limit: usize) -> Result<Vec<(String, i64)>, String> {
    let filter = if key.is_some() {
        "WHERE session_key=?1"
    } else {
        "WHERE ?1 IS NULL"
    };
    with_read(vault, |conn| {
        conn.prepare(&format!(
            "SELECT session_key, ord FROM chat_search_dirty_turns {filter} \
             ORDER BY session_key DESC, ord DESC LIMIT ?2"
        ))
        .map_err(|e| e.to_string())?
        .query_map(params![key, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
}

/// Reindex at most `units` marked documents and say whether marks remain.
/// Changed turns go first, then whole chats by newest activity, each
/// expanded (one unit) and its documents indexed while units last. Each
/// mark is read once per call, so a chat that keeps streaming costs its live
/// turn once per search. A write transaction covers at most
/// [`UNITS_PER_WRITE`] documents of one chat, so the global writer is
/// released between them.
pub(super) fn refresh(vault: &Path, units: usize) -> Result<bool, String> {
    let changed = marks(vault, None, units)?;
    index_marks(vault, &changed)?;
    let mut left = units - changed.len();
    if left > 0 {
        let whole = with_read(vault, |conn| {
            conn.prepare(
                "SELECT d.session_key FROM chat_search_dirty d LEFT JOIN sessions s ON s.key=d.session_key \
                 ORDER BY s.last_activity_at DESC, d.session_key LIMIT ?1",
            )
            .map_err(|e| e.to_string())?
            .query_map([left as i64], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })?;
        for key in whole {
            if left == 0 {
                break;
            }
            expand(vault, &key)?;
            left -= 1;
            let chat = marks(vault, Some(&key), left)?;
            index_marks(vault, &chat)?;
            left -= chat.len();
        }
    }
    with_read(vault, pending)
}

/// Whether any document is marked for reindexing.
pub(super) fn pending(conn: &Connection) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_search_dirty) OR EXISTS(SELECT 1 FROM chat_search_dirty_turns)",
        [],
        |r| r.get::<_, bool>(0),
    )
    .map_err(|e| e.to_string())
}

/// The FTS phrase for one word of at least three characters (the trigram
/// tokenizer has no entries for shorter ones): the word as literal text.
pub(super) fn phrase(word: &SearchWord) -> String {
    format!("\"{}\"", word.full.replace('"', "\"\""))
}

/// A chat's turn documents, oldest first.
const TURN_DOCS_SQL: &str =
    "SELECT ord,display FROM chat_search_docs WHERE session_key=?1 AND ord>=0 ORDER BY ord";

pub(super) fn message(
    conn: &Connection,
    key: &str,
    word: &SearchWord,
) -> Result<Option<Holding>, String> {
    let mut stmt = conn
        .prepare_cached(TURN_DOCS_SQL)
        .map_err(|e| e.to_string())?;
    for row in stmt
        .query_map([key], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
    {
        let display = row.map_err(|e| e.to_string())?;
        if let Some(at) = find_folded(&searchable(&display), &word.full) {
            return Ok(Some(Holding { text: display, at }));
        }
    }
    Ok(None)
}

/// The characters a context's request and outcome keep ([`cap_scrubbed`]).
const CONTEXT_CHARS: usize = 2000;

/// `text` as a context shows it: redacted, then cut to [`CONTEXT_CHARS`].
/// Only a head past the cut is redacted (`claude_runner::redact_prefix`), long
/// enough that the cut and the placeholder test after it read what they would
/// read in the whole redacted text, so the result is the same (2026-11-08:
/// redacting each hit's whole outcome was most of a first page's time).
fn context_text(text: &str) -> String {
    let longest = crate::claude_runner::REDACT_PLACEHOLDERS
        .iter()
        .map(|p| p.len())
        .max()
        .unwrap_or(0);
    cap_scrubbed(
        &crate::claude_runner::redact_prefix(text, CONTEXT_CHARS + longest),
        CONTEXT_CHARS,
    )
}

pub(super) fn context(
    conn: &Connection,
    key: &str,
    words: &[SearchWord],
) -> Result<SearchContext, String> {
    // A page asks each statement once per hit: prepared once per search.
    let mut stmt = conn
        .prepare_cached(TURN_DOCS_SQL)
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([key], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    // Each turn's first word, where it starts and its width, read once and
    // only up to the first turn that holds one.
    let held = |body: &str| {
        let body = searchable(body);
        words.iter().find_map(|w| {
            find_folded(&body, &w.full).map(|offset| (offset, w.full.chars().count()))
        })
    };
    let mut found = Vec::with_capacity(rows.len());
    for (_, body) in &rows {
        let hit = held(body);
        found.push(hit);
        if hit.is_some() {
            break;
        }
    }
    let at = found.iter().position(Option::is_some).unwrap_or(0);
    let messages = rows
        .iter()
        .enumerate()
        .filter(|(i, _)| i.abs_diff(at) <= 1)
        .map(|(i, (turn, body))| {
            let matched = found.get(i).copied().unwrap_or_else(|| held(body));
            SearchMessage {
                turn: *turn,
                text: matched
                    .map(|(offset, width)| search_snippet(body, offset, width))
                    .unwrap_or_else(|| cap_scrubbed(body, 1000)),
            }
        })
        .collect();
    let mut replacements=conn.prepare_cached("SELECT DISTINCT CASE WHEN json_valid(d.record_json) THEN json_extract(d.record_json,'$.childSession') END FROM delegations d JOIN turns t ON t.run_id=d.run_id WHERE t.session_key=?1 AND CASE WHEN json_valid(d.record_json) THEN json_extract(d.record_json,'$.tool') END='handover'").map_err(|e|e.to_string())?;
    let replaced_by = replacements
        .query_map([key], |r| r.get::<_, Option<String>>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
        .into_iter()
        .flatten()
        .filter(|key| is_safe_session_key(key))
        .collect();
    let request=conn.prepare_cached(&format!("SELECT t.asked FROM turns t WHERE t.session_key=?1 AND {SEARCH_PERSON_TURN} ORDER BY t.ord LIMIT 1")).and_then(|mut s|s.query_row([key],|r|r.get::<_,String>(0)).optional()).map_err(|e|e.to_string())?.unwrap_or_default();
    let outcome = rows
        .last()
        .map(|(ord, _)| load_transcript(conn, key, *ord))
        .transpose()?
        .map(|t| {
            if t.text.is_empty() {
                t.error.unwrap_or_default()
            } else {
                t.text
            }
        })
        .unwrap_or_default();
    Ok(SearchContext {
        request: context_text(&request),
        outcome: context_text(&outcome),
        messages,
        replaced_by,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("chat-search-")
            .tempdir()
            .unwrap()
    }

    fn turn(vault: &Path, key: &str, asked: &str, answer: &str) -> i64 {
        let ord = start_turn(
            vault,
            key,
            "codex",
            "/fixture",
            &format!("{key}-{asked}"),
            asked,
        )
        .unwrap();
        append_event(vault, key, ord, "assistant", answer).unwrap();
        finish_turn(vault, key, ord, "completed").unwrap();
        ord
    }

    #[test]
    fn fulltext_finds_answers_errors_and_short_tool_summaries_with_context() {
        let tmp = fixture();
        let v = tmp.path();
        let key = "s-20261108-ft-context";
        turn(
            v,
            key,
            "Choose the export format",
            "CSV preserves the delimiter",
        );
        let ord = turn(
            v,
            key,
            "Check the implementation",
            "The final repair passed",
        );
        append_event(v, key, ord, "error", "Parser delimiter mismatch").unwrap();
        append_event(
            v,
            key,
            ord,
            "toolUse",
            r#"{"id":"check","name":"Typecheck","input":{"secret":"never-index-input"}}"#,
        )
        .unwrap();
        append_event(
            v,
            key,
            ord,
            "toolResult",
            r#"{"id":"check","content":"Verification summary frobnicator passed"}"#,
        )
        .unwrap();
        let (hits, _) = search_sessions(
            v,
            "CSV delimiter",
            |_| true,
            None,
            20,
            MessageText::Redacted,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].context.request, "Choose the export format");
        assert_eq!(hits[0].context.outcome, "The final repair passed");
        assert_eq!(hits[0].context.messages.len(), 2);
        assert!(
            search_sessions(v, "mismatch", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len()
                == 1
        );
        assert!(
            search_sessions(v, "frobnicator", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len()
                == 1
        );
        assert!(search_sessions(
            v,
            "never-index-input",
            |_| true,
            None,
            20,
            MessageText::Redacted
        )
        .unwrap()
        .0
        .is_empty());
    }

    #[test]
    fn index_is_scrubbed_and_updates_after_edits_and_renames() {
        let tmp = fixture();
        let v = tmp.path();
        let key = "s-20261108-ft-edit";
        let secret = "sk-1234567890abcdefghijklmnopqrstuv";
        let ord = turn(
            v,
            key,
            "A harmless request",
            &format!("initialword token {secret}"),
        );
        assert_eq!(
            search_sessions(v, "initialword", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len(),
            1
        );
        let conn = open(v).unwrap();
        let bodies: String = conn
            .query_row(
                "SELECT (SELECT group_concat(body) FROM chat_search_fts) || \
                 (SELECT group_concat(display) FROM chat_search_docs)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!bodies.contains(secret));
        assert!(
            search_sessions(v, secret, |_| true, None, 20, MessageText::AsWritten)
                .unwrap()
                .0
                .is_empty()
        );
        append_event(v, key, ord, "assistant", "replacementword is current").unwrap();
        assert!(
            search_sessions(v, "initialword", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .is_empty()
        );
        assert_eq!(
            search_sessions(
                v,
                "replacementword",
                |_| true,
                None,
                20,
                MessageText::Redacted
            )
            .unwrap()
            .0
            .len(),
            1
        );
        rename_session(v, key, "Renamedquartz").unwrap();
        assert_eq!(
            search_sessions(
                v,
                "renamedquartz",
                |_| true,
                None,
                20,
                MessageText::Redacted
            )
            .unwrap()
            .0
            .len(),
            1
        );
    }

    #[test]
    fn cloned_store_backfills_and_archived_chats_stay_excluded() {
        let source = fixture();
        let key = "s-20261108-ft-clone";
        turn(
            source.path(),
            key,
            "Explain the plan",
            "Answeronlysearchvalue",
        );
        // Copy through SQLite's own snapshot to preserve WAL content.
        let cloned = fixture();
        let target = db_path_for(cloned.path()).unwrap();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        open(source.path())
            .unwrap()
            .execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()])
            .unwrap();
        assert_eq!(
            search_sessions(
                cloned.path(),
                "Answeronlysearchvalue",
                |_| true,
                None,
                20,
                MessageText::Redacted
            )
            .unwrap()
            .0
            .len(),
            1
        );
        assert!(
            search_sessions(
                source.path(),
                "Answeronlysearchvalue",
                |_| true,
                None,
                20,
                MessageText::Redacted
            )
            .unwrap()
            .0
            .len()
                == 1
        );
        open(cloned.path())
            .unwrap()
            .execute("UPDATE sessions SET archived=1 WHERE key=?1", [key])
            .unwrap();
        assert!(search_sessions(
            cloned.path(),
            "Answeronlysearchvalue",
            |_| true,
            None,
            20,
            MessageText::Redacted
        )
        .unwrap()
        .0
        .is_empty());
        assert_eq!(
            search_sessions(
                source.path(),
                "Answeronlysearchvalue",
                |_| true,
                None,
                20,
                MessageText::Redacted
            )
            .unwrap()
            .0
            .len(),
            1
        );
    }

    #[test]
    fn workspace_priority_survives_pagination_before_recency() {
        let tmp = fixture();
        let v = tmp.path();
        for (key, cwd, at) in [
            ("s-20261108-ft-same", "/same", "2026-11-01T00:00:00Z"),
            ("s-20261108-ft-work", "/work", "2026-11-02T00:00:00Z"),
            ("s-20261108-ft-other", "/other", "2026-11-03T00:00:00Z"),
        ] {
            start_turn(v, key, "codex", cwd, key, "rankquery").unwrap();
            open(v)
                .unwrap()
                .execute(
                    "UPDATE sessions SET last_activity_at=?2 WHERE key=?1",
                    params![key, at],
                )
                .unwrap();
        }
        let rank = |cwd: &str| {
            Some(match cwd {
                "/same" => 0,
                "/work" => 1,
                _ => 2,
            })
        };
        let mut cursor = None;
        let mut keys = Vec::new();
        loop {
            let (page, more) = search_sessions_ranked(
                v,
                "rankquery",
                |_| true,
                cursor.as_ref(),
                1,
                MessageText::Redacted,
                rank,
            )
            .unwrap();
            let hit = &page[0];
            keys.push(hit.summary.key.clone());
            if !more {
                break;
            }
            cursor = Some(crate::page_cursor::Keyset::new(
                Some(&format!("{}|{}", hit.rank, hit.summary.last_activity_at)),
                &hit.summary.key,
            ));
        }
        assert_eq!(
            keys,
            [
                "s-20261108-ft-same",
                "s-20261108-ft-work",
                "s-20261108-ft-other"
            ]
        );
        assert_eq!(
            search_sessions_ranked(
                v,
                "rankquery",
                |s| s.cwd != "/other",
                None,
                20,
                MessageText::Redacted,
                rank
            )
            .unwrap()
            .0
            .len(),
            2
        );
    }

    /// Second 2026-11-08 review: a broad query with no preferred workspace
    /// (priorities 1 and 2 only) read every matching chat, on every page. Each
    /// priority is now its own query with the cursor and a `LIMIT` in SQL, so
    /// a page reads about a page of rows per priority it reaches, and the
    /// pages still come in priority order, newest first, each chat once.
    #[test]
    fn a_ranked_page_reads_about_a_page_of_rows() {
        let tmp = fixture();
        let v = tmp.path();
        // 3 chats in the work folder (priority 1), 40 in another (priority 2),
        // and 5 in a folder `rank` leaves out; all match.
        let mut expected = Vec::new();
        for (folder, count) in [("/work", 3), ("/other", 40), ("/hidden", 5)] {
            for i in 0..count {
                let key = format!("s-20261108-ft-{}{i:03}", &folder[1..]);
                start_turn(v, &key, "codex", folder, &key, "boundword").unwrap();
                open(v)
                    .unwrap()
                    .execute(
                        "UPDATE sessions SET last_activity_at=?2 WHERE key=?1",
                        params![key, format!("2026-11-08T{:02}:{:02}:00Z", 10, 59 - i)],
                    )
                    .unwrap();
                if folder != "/hidden" {
                    expected.push(key);
                }
            }
        }
        drain_search_index(v, std::time::Duration::ZERO).unwrap();
        let rank = |cwd: &str| match cwd {
            "/work" => Some(1),
            "/other" => Some(2),
            _ => None,
        };
        let limit = 5;
        let mut cursor = None;
        let mut seen = Vec::new();
        let mut reads = Vec::new();
        loop {
            let before = search_rows_read();
            let (page, more) = search_sessions_ranked(
                v,
                "boundword",
                |_| true,
                cursor.as_ref(),
                limit,
                MessageText::Redacted,
                rank,
            )
            .unwrap();
            reads.push(search_rows_read() - before);
            assert!(page.len() <= limit);
            seen.extend(page.iter().map(|h| h.summary.key.clone()));
            if !more {
                break;
            }
            let last = page.last().unwrap();
            cursor = Some(crate::page_cursor::Keyset::new(
                Some(&format!("{}|{}", last.rank, last.summary.last_activity_at)),
                &last.summary.key,
            ));
        }
        assert_eq!(
            seen, expected,
            "priority 1 first, then 2, each newest first"
        );
        // Page 1: the 3 work chats, then 3 of the rest (a page plus one).
        // Every later page: its own priority's next page plus one. Before the
        // fix every page read all 43.
        assert_eq!(reads[0], limit + 1, "{reads:?}");
        assert!(reads.iter().all(|n| *n <= limit + 1), "{reads:?}");
        assert_eq!(reads.len(), 9, "{reads:?}");
        // A cursor with no priority (the format before ranking, and the
        // phone's, where every chat is priority 0) reads as priority 0.
        let old =
            crate::page_cursor::Keyset::new(Some("2026-11-08T10:58:00Z"), "s-20261108-ft-work001");
        let before = search_rows_read();
        let (page, more) = search_sessions_ranked(
            v,
            "boundword",
            |_| true,
            Some(&old),
            limit,
            MessageText::Redacted,
            |cwd| (cwd != "/hidden").then_some(0),
        )
        .unwrap();
        assert_eq!(page[0].summary.key, "s-20261108-ft-other002");
        assert!(more);
        assert_eq!(search_rows_read() - before, limit + 1);
    }

    /// (ordinal, id) of every document of chat `key`, by ordinal.
    fn docs(v: &Path, key: &str) -> Vec<(i64, i64)> {
        open(v)
            .unwrap()
            .prepare("SELECT ord,id FROM chat_search_docs WHERE session_key=?1 ORDER BY ord")
            .unwrap()
            .query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn count(v: &Path, sql: &str) -> i64 {
        open(v).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// 2026-11-08 review: one refresh reindexes at most its budget, the rest
    /// stay marked for the next; a new event marks its turn only, so a
    /// streaming chat costs one document per search; a deleted chat loses
    /// every document.
    #[test]
    fn refresh_is_bounded_and_reindexes_only_the_changed_turn() {
        let tmp = fixture();
        let v = tmp.path();
        for i in 0..4 {
            let key = format!("s-20261108-ft-bound{i}");
            for j in 0..3 {
                turn(
                    v,
                    &key,
                    &format!("ask {j}"),
                    &format!("boundedanswer {i} {j}"),
                );
            }
        }
        // Four chats, each a title and three turns: 16 documents marked.
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty"), 0);
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty_turns"), 16);
        assert!(refresh(v, 5).unwrap());
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_docs"), 5);
        assert!(refresh(v, 5).unwrap());
        assert!(refresh(v, 5).unwrap());
        assert!(!refresh(v, 5).unwrap());
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_docs"), 16);
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_fts"), 16);

        let key = "s-20261108-ft-bound0";
        let before = docs(v, key);
        assert_eq!(before.len(), 4);
        append_event(v, key, 2, "assistant", "streamedword arrives").unwrap();
        let marks: Vec<(String, i64)> = open(v)
            .unwrap()
            .prepare("SELECT session_key,ord FROM chat_search_dirty_turns")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(marks, [(key.to_string(), 2)]);
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty"), 0);
        assert_eq!(
            search_sessions(v, "streamedword", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len(),
            1
        );
        // Only turn 2's document was replaced; the others kept their rows.
        let after = docs(v, key);
        assert_eq!(after.len(), before.len());
        for ((ord, old), (_, new)) in before.iter().zip(&after) {
            assert_eq!(old == new, *ord != 2, "turn {ord}: {before:?} {after:?}");
        }

        let conn = open(v).unwrap();
        conn.execute("DELETE FROM turns WHERE session_key=?1", [key])
            .unwrap();
        conn.execute("DELETE FROM sessions WHERE key=?1", [key])
            .unwrap();
        assert!(!refresh(v, REFRESH_UNITS).unwrap());
        assert!(docs(v, key).is_empty());
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_fts"), 12);
    }

    /// A store from before v4 (no layout version 4) drops the old index and
    /// marks every chat whole; the backfill expands each, newest activity
    /// first, and indexes it within the same budget.
    #[test]
    fn an_older_layout_is_rebuilt_newest_chat_first() {
        let tmp = fixture();
        let v = tmp.path();
        turn(v, "s-20261108-ft-old1", "Older chat", "layoutword one");
        turn(v, "s-20261108-ft-old2", "Newer chat", "layoutword two");
        let conn = open(v).unwrap();
        conn.execute(
            "UPDATE sessions SET last_activity_at=?2 WHERE key=?1",
            params!["s-20261108-ft-old1", "2026-11-08T01:00:00Z"],
        )
        .unwrap();
        conn.execute(
            "UPDATE sessions SET last_activity_at=?2 WHERE key=?1",
            params!["s-20261108-ft-old2", "2026-11-08T02:00:00Z"],
        )
        .unwrap();
        conn.execute("DELETE FROM chat_search_meta", []).unwrap();
        conn.execute("INSERT INTO chat_search_meta VALUES(3)", [])
            .unwrap();
        // The migration runs once per store per process; run it as a new
        // process would.
        schema(&conn).unwrap();
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty"), 2);
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty_turns"), 0);
        assert_eq!(
            count(v, "SELECT version FROM chat_search_meta"),
            INDEX_VERSION
        );
        // One unit expands the newest chat; two more index its turn and title.
        assert!(refresh(v, 3).unwrap());
        assert_eq!(docs(v, "s-20261108-ft-old2").len(), 2);
        assert!(docs(v, "s-20261108-ft-old1").is_empty());
        assert!(!refresh(v, REFRESH_UNITS).unwrap());
        assert_eq!(
            search_sessions(v, "layoutword", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len(),
            2
        );
    }

    fn plant_corruption(v: &Path) {
        // The fault PER-23 found: a malformed inverted index.
        open(v)
            .unwrap()
            .execute(
                "UPDATE chat_search_fts_data SET block=randomblob(64) WHERE id>10",
                [],
            )
            .unwrap();
    }

    fn two_indexed_chats(v: &Path) {
        turn(v, "s-20261115-ft-bad1", "First chat", "corruptword alpha");
        turn(v, "s-20261115-ft-bad2", "Second chat", "corruptword beta");
        drain_search_index(v, std::time::Duration::ZERO).unwrap();
    }

    fn hits(v: &Path) -> usize {
        search_sessions(v, "corruptword", |_| true, None, 20, MessageText::Redacted)
            .unwrap()
            .0
            .len()
    }

    /// PER-23: a damaged full-text index is found at startup and rebuilt
    /// from the stored chats, which stay as they were.
    #[test]
    fn a_corrupted_index_is_rebuilt_from_the_stored_chats() {
        let tmp = fixture();
        let v = tmp.path();
        two_indexed_chats(v);
        assert_eq!(hits(v), 2);
        assert!(!verify(v).unwrap(), "a sound index is left alone");
        assert_eq!(probe(&open(v).unwrap()), Probe::Sound);
        plant_corruption(v);
        assert_eq!(probe(&open(v).unwrap()), Probe::Corrupt);
        assert!(verify(v).unwrap(), "a damaged index is rebuilt");
        assert_eq!(probe(&open(v).unwrap()), Probe::Sound);
        assert!(full_check(&open(v).unwrap()).is_ok());
        assert_eq!(verify(v), Ok(false));
        // The chats are untouched; the index refills from them.
        assert_eq!(count(v, "SELECT count(*) FROM sessions"), 2);
        assert_eq!(count(v, "SELECT count(*) FROM turns"), 2);
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty"), 2);
        drain_search_index(v, std::time::Duration::ZERO).unwrap();
        assert_eq!(hits(v), 2);
        assert_eq!(check_search_index(v), Ok(false));
    }

    /// Review: when the damaged table refuses its DROP, the hand removal
    /// runs, a fresh connection rebuilds, and search works again.
    #[test]
    fn a_table_that_refuses_its_drop_is_removed_by_hand() {
        let tmp = fixture();
        let v = tmp.path();
        two_indexed_chats(v);
        plant_corruption(v);
        let refused = |_: &Connection| -> Result<(), rusqlite::Error> {
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT_VTAB),
                Some("malformed inverted index for FTS5 table main.chat_search_fts".into()),
            ))
        };
        assert_eq!(repair(v, refused), Ok(true));
        assert_eq!(verify(v), Ok(false));
        assert_eq!(count(v, "SELECT count(*) FROM sessions"), 2);
        drain_search_index(v, std::time::Duration::ZERO).unwrap();
        assert_eq!(hits(v), 2);
    }

    /// Review: contention is not damage. With the store's writer held by
    /// another connection, a damaged index is neither rebuilt nor touched.
    #[test]
    fn contention_never_drops_an_index() {
        let tmp = fixture();
        let v = tmp.path();
        two_indexed_chats(v);
        plant_corruption(v);
        let docs_before = count(v, "SELECT count(*) FROM chat_search_docs");
        let other = open(v).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let skipped = verify(v);
        other.execute_batch("ROLLBACK").unwrap();
        assert!(skipped.is_err(), "{skipped:?}");
        assert_eq!(
            count(v, "SELECT count(*) FROM chat_search_docs"),
            docs_before
        );
        assert_eq!(count(v, "SELECT count(*) FROM chat_search_dirty"), 0);
        assert!(!is_corruption(&rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None
        )));
    }

    /// Review: a damaged `chat_search_docs` page, which the FTS check does
    /// not cover, is found by the probe and repaired, chats untouched.
    #[test]
    fn a_damaged_documents_table_is_rebuilt() {
        use std::io::{Seek, SeekFrom, Write};
        let tmp = fixture();
        let v = tmp.path();
        two_indexed_chats(v);
        let (page, root, path) = {
            let conn = open(v).unwrap();
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .unwrap();
            let page: u64 = conn
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .unwrap();
            let root: u64 = conn
                .query_row(
                    "SELECT rootpage FROM sqlite_schema WHERE name='chat_search_docs'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            (page, root, db_path_for(v).unwrap())
        };
        let mut file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.seek(SeekFrom::Start((root - 1) * page)).unwrap();
        file.write_all(&vec![0xFFu8; page as usize]).unwrap();
        drop(file);
        assert_eq!(probe(&open(v).unwrap()), Probe::Corrupt);
        assert_eq!(verify(v), Ok(true));
        assert_eq!(probe(&open(v).unwrap()), Probe::Sound);
        assert_eq!(count(v, "SELECT count(*) FROM sessions"), 2);
        drain_search_index(v, std::time::Duration::ZERO).unwrap();
        assert_eq!(hits(v), 2);
        assert_eq!(verify(v), Ok(false));
    }

    /// 2026-11-08 review: a tool summary cut inside a redaction placeholder
    /// leaves no fragment of it ("[api-k") for a query to match.
    #[test]
    fn a_cut_redaction_placeholder_is_not_searchable() {
        assert_eq!(cap_scrubbed("ab[redacted]cd", 6), "ab… (truncated)");
        assert_eq!(cap_scrubbed("ab[redacted]cd", 3), "ab… (truncated)");
        assert_eq!(
            cap_scrubbed("ab[redacted]cd", 12),
            "ab[redacted]… (truncated)"
        );
        assert_eq!(cap_scrubbed("ab[x]cdefgh", 4), "ab[x… (truncated)");
        assert_eq!(cap_scrubbed("abc", 3), "abc");
        let tmp = fixture();
        let v = tmp.path();
        let key = "s-20261108-ft-cut";
        let ord = turn(v, key, "Run the probe", "Probe done");
        let secret = "sk-1234567890abcdefghijklmnopqrstuv";
        // Redacted, the summary's cut falls after "[api-k".
        let result = format!("{} {secret} after", "x".repeat(TOOL_SUMMARY_CHARS - 7));
        append_event(
            v,
            key,
            ord,
            "toolUse",
            r#"{"id":"probe","name":"Probe","input":{}}"#,
        )
        .unwrap();
        append_event(
            v,
            key,
            ord,
            "toolResult",
            &serde_json::json!({"id": "probe", "content": result}).to_string(),
        )
        .unwrap();
        assert_eq!(
            search_sessions(v, "probe", |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .len(),
            1
        );
        let display: String = open(v)
            .unwrap()
            .query_row(
                "SELECT display FROM chat_search_docs WHERE session_key=?1 AND ord=?2",
                params![key, ord],
                |r| r.get(0),
            )
            .unwrap();
        assert!(display.contains("x … (truncated)"), "{display}");
        assert!(!display.contains("[api"), "{display}");
        for query in ["api-k", "[api", "pi-k"] {
            assert!(
                search_sessions(v, query, |_| true, None, 20, MessageText::Redacted)
                    .unwrap()
                    .0
                    .is_empty(),
                "{query}"
            );
        }
    }

    /// 2026-11-08 review: a query of only one- or two-character words reads
    /// titles (no document scan); in a longer query a short word is checked
    /// in the chats the longer words found.
    #[test]
    fn short_words_alone_read_titles_only() {
        let tmp = fixture();
        let v = tmp.path();
        let (titled, body) = ("s-20261108-ft-short1", "s-20261108-ft-short2");
        turn(v, titled, "Plan the Q3 review", "Ok");
        rename_session(v, titled, "Q3 plan").unwrap();
        turn(v, body, "Shortbody Q3 numbers", "Ok");
        rename_session(v, body, "Numbers").unwrap();
        let keys = |query: &str| {
            search_sessions(v, query, |_| true, None, 20, MessageText::Redacted)
                .unwrap()
                .0
                .into_iter()
                .map(|h| h.summary.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys("q3"), [titled]);
        assert_eq!(keys("Q3 shortbody"), [body]);
        assert!(keys("zz").is_empty());
    }

    /// A context's request and outcome redact only a head of a long text and
    /// read as redacting all of it would (2026-11-08): a key or a bearer
    /// token at the cut, a PEM block past the head, wide characters, and a
    /// head that redacts too short to show.
    #[test]
    fn context_text_reads_as_the_whole_text_redacted() {
        let words = "Reviewed the layout and changed the spacing tokens. ".repeat(200);
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789";
        let pem = format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----",
            "MIIDdzCCAl".repeat(600)
        );
        let mut texts = vec![
            words.clone(),
            format!("{}{words}", "\u{e9}".repeat(2500)),
            format!("{} {words}", key.repeat(60)),
        ];
        for at in [1990, 1995, 2000, 2005, 2010, 2490, 2520, 3000] {
            let lead = &words[..at];
            texts.push(format!("{lead} {key} {words}"));
            texts.push(format!("{lead} Authorization: Bearer {key} {words}"));
            texts.push(format!("{lead} {pem} {words}"));
        }
        for text in texts {
            assert_eq!(
                context_text(&text),
                cap_scrubbed(&scrub(&text), CONTEXT_CHARS),
                "{}",
                &text[..80]
            );
        }
    }
}
