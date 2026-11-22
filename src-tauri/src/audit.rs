//! The audit record (COO rebuild X2, decided in the X0 note
//! `outputs/coo-rebuild-2026-11-10/linear-tracking-audit.md`, section 5).
//!
//! One append-only table in the sessions database, `audit_log`: one row per
//! agent run start and end, issue link and tracker write (person or agent).
//! `BEFORE UPDATE` and `BEFORE DELETE` triggers abort, and each row carries
//! `prev_hash` and `hash` (SHA-256 of the previous hash and the row's
//! canonical JSON, a fixed field order), so an edited row or a gap shows in
//! [`verify`].
//!
//! Hollow's host writes it, never an agent. An audit write never blocks or
//! fails the run or the tracker write it describes: [`record`] logs and
//! counts an error ([`errors`]) and the caller goes on.
//!
//! Who asked (`asked_by`) is `person`, a session key or `automation:<id>`.
//! Account is its Hollow label only (`usage_ledger.account_ref`), never a
//! credential. Provider-neutral: nothing here names a provider.

use crate::session_store;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// The table, its indexes, the append-only triggers and `audit_head` (the
/// newest row's id and hash and the row count, written in the same
/// transaction, so `verify` sees a deleted tail). `apply_schema` runs it.
/// New in this release, so no column migrations.
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS audit_log( \
   id TEXT PRIMARY KEY, \
   at TEXT NOT NULL, \
   kind TEXT NOT NULL, \
   session_key TEXT, \
   parent_key TEXT, \
   root_key TEXT, \
   asked_by TEXT, \
   agent TEXT, \
   model TEXT, \
   effort TEXT, \
   account TEXT, \
   workspace_id TEXT, \
   issue_id TEXT, \
   issue_key TEXT, \
   action TEXT, \
   fields TEXT, \
   outcome TEXT, \
   detail TEXT, \
   cost_usd REAL, \
   tokens_in INTEGER, \
   tokens_out INTEGER, \
   working_ms INTEGER, \
   request_key TEXT, \
   prev_hash TEXT NOT NULL, \
   hash TEXT NOT NULL \
 ); \
 CREATE INDEX IF NOT EXISTS idx_audit_session ON audit_log(session_key, kind); \
 CREATE INDEX IF NOT EXISTS idx_audit_issue ON audit_log(issue_id); \
 CREATE INDEX IF NOT EXISTS idx_audit_issue_key ON audit_log(issue_key COLLATE NOCASE); \
 CREATE INDEX IF NOT EXISTS idx_audit_request ON audit_log(request_key, kind, action, outcome); \
 CREATE TABLE IF NOT EXISTS audit_head( \
   id INTEGER PRIMARY KEY CHECK (id = 1), \
   head_id TEXT NOT NULL, \
   head_hash TEXT NOT NULL, \
   rows INTEGER NOT NULL \
 ); \
 CREATE TRIGGER IF NOT EXISTS audit_log_no_update BEFORE UPDATE ON audit_log \
   BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END; \
 CREATE TRIGGER IF NOT EXISTS audit_log_no_delete BEFORE DELETE ON audit_log \
   BEGIN SELECT RAISE(ABORT, 'audit_log is append-only'); END;";

/// The event kinds (`audit_log.kind`).
pub(crate) mod kind {
    pub const RUN_STARTED: &str = "run_started";
    pub const RUN_ENDED: &str = "run_ended";
    pub const TRACKER_WRITE: &str = "tracker_write";
    pub const TRACKER_WRITE_FAILED: &str = "tracker_write_failed";
    pub const ISSUE_LINKED: &str = "issue_linked";
    pub const ALL: [&str; 5] = [
        RUN_STARTED,
        RUN_ENDED,
        TRACKER_WRITE,
        TRACKER_WRITE_FAILED,
        ISSUE_LINKED,
    ];
}

/// One audit row. `id`, `at`, `prev_hash` and `hash` are filled by
/// [`append`]; everything else is the caller's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Time-ordered and unique (`a` + 12 hex digits of milliseconds, kept
    /// strictly increasing in a process, + 16 random hex digits).
    pub id: String,
    /// RFC 3339.
    pub at: String,
    /// `run_started`, `run_ended`, `tracker_write`, `tracker_write_failed`
    /// or `issue_linked`.
    pub kind: String,
    pub session_key: Option<String>,
    pub parent_key: Option<String>,
    pub root_key: Option<String>,
    /// `person`, a session key or `automation:<id>`.
    pub asked_by: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The account's Hollow label, never a credential.
    pub account: Option<String>,
    pub workspace_id: Option<String>,
    pub issue_id: Option<String>,
    pub issue_key: Option<String>,
    /// For writes: `create`, `update`, `comment`, `archive` or `link`.
    pub action: Option<String>,
    /// For writes: the changed fields as `before` and `after`; for
    /// `run_ended`: `turns`, `costPartial` and `startRow` (the start it
    /// ended). Redacted and bounded at append.
    #[cfg_attr(test, ts(type = "Record<string, unknown> | null"))]
    pub fields: Option<Value>,
    /// `ok`, `failed`, `stopped` or `skipped`.
    pub outcome: Option<String>,
    /// A short reason.
    pub detail: Option<String>,
    pub cost_usd: Option<f64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub tokens_in: Option<i64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub tokens_out: Option<i64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub working_ms: Option<i64>,
    /// A write's idempotency key: a repeat with the same kind, action and
    /// outcome adds no row.
    pub request_key: Option<String>,
    pub prev_hash: String,
    pub hash: String,
}

const COLUMNS: &str = "id, at, kind, session_key, parent_key, root_key, asked_by, agent, \
     model, effort, account, workspace_id, issue_id, issue_key, action, fields, outcome, \
     detail, cost_usd, tokens_in, tokens_out, working_ms, request_key, prev_hash, hash";

/// Audit writes that failed in this process (logged; the caller went on).
static ERRORS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn errors() -> u64 {
    ERRORS.load(Ordering::Relaxed)
}

/// The last id's millisecond, so ids made in one process strictly increase
/// (monotonic, as a ULID's): two rows in the same millisecond still sort.
static LAST_MS: std::sync::Mutex<i64> = std::sync::Mutex::new(0);

fn new_id(now: DateTime<Utc>) -> String {
    let ms = {
        let mut last = LAST_MS.lock().unwrap_or_else(|e| e.into_inner());
        *last = now.timestamp_millis().max(*last + 1).max(0);
        *last
    };
    format!("a{ms:012x}{:016x}", rand::random::<u64>())
}

/// A stored number: finite, and `0.0` for `-0.0` (both hash the same way).
fn clean(value: Option<f64>) -> Option<f64> {
    value
        .filter(|v| v.is_finite())
        .map(|v| if v == 0.0 { 0.0 } else { v })
}

/// The bounds of one row (Codex review b): its canonical JSON in bytes, a
/// text column, a `fields` string, key and request key, nesting and items.
pub(crate) const MAX_ROW_BYTES: usize = 8 * 1024;
const MAX_TEXT: usize = 300;
const MAX_VALUE: usize = 200;
const MAX_KEY: usize = 64;
const MAX_REQUEST_KEY: usize = 200;
const MAX_DEPTH: usize = 4;
const MAX_ITEMS: usize = 32;

fn clip(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_string();
    }
    text.chars().take(chars).collect::<String>() + "…"
}

/// Free text without credentials (the redaction helper the tracker's
/// comment bodies use), then clipped.
fn scrub(text: &str, chars: usize) -> String {
    clip(&crate::claude_runner::redact(text), chars)
}

/// Q-R35: whether an audit field's key names a credential (`token`,
/// `apiKey`, `client_secret`, `password`, `Authorization`, `cookie`, ...).
/// Case, `_`, `-` and `.` are ignored. A key ending in `tokens` is a count
/// (`input_tokens`), so it stays, and a bare `key` may be an issue key.
pub(crate) fn secret_key(key: &str) -> bool {
    let k: String = key
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    k.ends_with("token")
        || [
            "password",
            "passwd",
            "secret",
            "apikey",
            "privatekey",
            "accesskey",
            "credential",
            "authorization",
            "cookie",
            "bearer",
        ]
        .iter()
        .any(|word| k.contains(word))
}

/// `fields` redacted and bounded: strings and keys clipped, at most
/// `MAX_ITEMS` entries a collection, nesting below `MAX_DEPTH` folded, and
/// `-0.0` as `0`.
fn bound(value: &Value, depth: usize) -> Value {
    let nested = || Value::String("(nested)".into());
    match value {
        Value::String(text) => Value::String(scrub(text, MAX_VALUE)),
        Value::Number(n) if n.is_f64() && n.as_f64() == Some(0.0) => json!(0),
        Value::Array(_) | Value::Object(_) if depth >= MAX_DEPTH => nested(),
        Value::Array(items) => {
            let mut out: Vec<Value> = items
                .iter()
                .take(MAX_ITEMS)
                .map(|v| bound(v, depth + 1))
                .collect();
            if items.len() > MAX_ITEMS {
                out.push(json!(format!("(+{} more)", items.len() - MAX_ITEMS)));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            let mut out: serde_json::Map<String, Value> = map
                .iter()
                .take(MAX_ITEMS)
                .map(|(k, v)| {
                    let value = match v {
                        // Q-R35: a secret-named key's text goes whole, as
                        // the redaction pass alone would keep a plain value.
                        Value::String(_) if secret_key(k) => json!("[redacted]"),
                        _ => bound(v, depth + 1),
                    };
                    (scrub(k, MAX_KEY), value)
                })
                .collect();
            if map.len() > MAX_ITEMS {
                out.insert("(more)".into(), json!(map.len() - MAX_ITEMS));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// Make `entry` storable: free text redacted, every column and `fields`
/// bounded, numbers normalized, and the whole row under `MAX_ROW_BYTES`
/// (`fields` gives way first, then the detail, then every text column).
fn sanitize(entry: &mut AuditEntry) {
    let ids = |v: &mut Option<String>| {
        if let Some(text) = v.as_mut() {
            *text = clip(text, MAX_TEXT);
        }
    };
    for column in [
        &mut entry.session_key,
        &mut entry.parent_key,
        &mut entry.root_key,
        &mut entry.asked_by,
        &mut entry.agent,
        &mut entry.model,
        &mut entry.effort,
        &mut entry.account,
        &mut entry.workspace_id,
        &mut entry.issue_id,
        &mut entry.issue_key,
        &mut entry.action,
        &mut entry.outcome,
    ] {
        ids(column);
    }
    entry.kind = clip(&entry.kind, MAX_KEY);
    entry.detail = entry.detail.as_deref().map(|d| scrub(d, MAX_TEXT));
    entry.request_key = entry
        .request_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|k| clip(k, MAX_REQUEST_KEY));
    entry.fields = entry.fields.as_ref().map(|f| bound(f, 0));
    entry.cost_usd = clean(entry.cost_usd);
    let size =
        |e: &AuditEntry| canonical(e, e.fields.as_ref().map(Value::to_string).as_deref()).len();
    if size(entry) > MAX_ROW_BYTES {
        entry.fields = Some(json!({"truncated": true}));
    }
    if size(entry) > MAX_ROW_BYTES {
        entry.detail = entry.detail.as_deref().map(|d| clip(d, 64));
    }
    if size(entry) > MAX_ROW_BYTES {
        for column in [
            &mut entry.session_key,
            &mut entry.parent_key,
            &mut entry.root_key,
            &mut entry.asked_by,
            &mut entry.agent,
            &mut entry.model,
            &mut entry.effort,
            &mut entry.account,
            &mut entry.workspace_id,
            &mut entry.issue_id,
            &mut entry.issue_key,
            &mut entry.request_key,
        ] {
            if let Some(text) = column.as_mut() {
                *text = clip(text, 64);
            }
        }
    }
}

/// The row's canonical JSON: every field but the hashes, in a fixed order
/// (a JSON array, so no map ordering applies). `fields` is its stored text.
fn canonical(e: &AuditEntry, fields: Option<&str>) -> String {
    json!([
        e.id,
        e.at,
        e.kind,
        e.session_key,
        e.parent_key,
        e.root_key,
        e.asked_by,
        e.agent,
        e.model,
        e.effort,
        e.account,
        e.workspace_id,
        e.issue_id,
        e.issue_key,
        e.action,
        fields,
        e.outcome,
        e.detail,
        e.cost_usd,
        e.tokens_in,
        e.tokens_out,
        e.working_ms,
        e.request_key,
    ])
    .to_string()
}

fn chain_hash(prev: &str, canonical: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(prev.as_bytes());
    digest.update(canonical.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Append `entry` in the caller's transaction: sanitize it, read the last
/// hash, compute this row's, insert, and move `audit_head`. A write with a
/// `request_key` is recorded once per kind, action and outcome (an indexed
/// lookup): a retry that changes nothing adds no row. Returns the stored
/// row, or none when it was such a repeat.
pub(crate) fn append(
    conn: &Connection,
    mut entry: AuditEntry,
) -> Result<Option<AuditEntry>, String> {
    sanitize(&mut entry);
    if let Some(key) = &entry.request_key {
        let seen: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_log WHERE request_key = ?1 AND kind = ?2 \
                 AND action IS ?3 AND outcome IS ?4)",
                params![key, entry.kind, entry.action, entry.outcome],
                |row| row.get(0),
            )
            .map_err(|e| format!("audit repeat check: {e}"))?;
        if seen {
            return Ok(None);
        }
    }
    let now = Utc::now();
    entry.id = new_id(now);
    if entry.at.trim().is_empty() {
        entry.at = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    }
    let prev: String = conn
        .query_row(
            "SELECT hash FROM audit_log ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("audit last hash: {e}"))?
        .unwrap_or_default();
    let fields = entry.fields.as_ref().map(Value::to_string);
    entry.prev_hash = prev;
    entry.hash = chain_hash(&entry.prev_hash, &canonical(&entry, fields.as_deref()));
    conn.execute(
        &format!(
            "INSERT INTO audit_log({COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, \
             ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)"
        ),
        params![
            entry.id,
            entry.at,
            entry.kind,
            entry.session_key,
            entry.parent_key,
            entry.root_key,
            entry.asked_by,
            entry.agent,
            entry.model,
            entry.effort,
            entry.account,
            entry.workspace_id,
            entry.issue_id,
            entry.issue_key,
            entry.action,
            fields,
            entry.outcome,
            entry.detail,
            entry.cost_usd,
            entry.tokens_in,
            entry.tokens_out,
            entry.working_ms,
            entry.request_key,
            entry.prev_hash,
            entry.hash,
        ],
    )
    .map_err(|e| format!("audit insert: {e}"))?;
    conn.execute(
        "INSERT INTO audit_head(id, head_id, head_hash, rows) VALUES(1, ?1, ?2, 1) \
         ON CONFLICT(id) DO UPDATE SET head_id = excluded.head_id, \
         head_hash = excluded.head_hash, rows = rows + 1",
        params![entry.id, entry.hash],
    )
    .map_err(|e| format!("audit head: {e}"))?;
    Ok(Some(entry))
}

/// Append `entry` in its own transaction. Never fails the caller: an error
/// is logged and counted, and the run or write it describes goes on.
pub(crate) fn record(vault: &Path, entry: AuditEntry) -> Option<AuditEntry> {
    let kind = entry.kind.clone();
    match session_store::with_write(vault, "audit", |tx| append(tx, entry)) {
        Ok(row) => row,
        Err(error) => {
            ERRORS.fetch_add(1, Ordering::Relaxed);
            eprintln!("[audit] {kind} not recorded: {error}");
            None
        }
    }
}

fn entry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(AuditEntry, Option<String>)> {
    let fields: Option<String> = row.get(15)?;
    Ok((
        AuditEntry {
            id: row.get(0)?,
            at: row.get(1)?,
            kind: row.get(2)?,
            session_key: row.get(3)?,
            parent_key: row.get(4)?,
            root_key: row.get(5)?,
            asked_by: row.get(6)?,
            agent: row.get(7)?,
            model: row.get(8)?,
            effort: row.get(9)?,
            account: row.get(10)?,
            workspace_id: row.get(11)?,
            issue_id: row.get(12)?,
            issue_key: row.get(13)?,
            action: row.get(14)?,
            fields: fields.as_deref().and_then(|f| serde_json::from_str(f).ok()),
            outcome: row.get(16)?,
            detail: row.get(17)?,
            cost_usd: row.get(18)?,
            tokens_in: row.get(19)?,
            tokens_out: row.get(20)?,
            working_ms: row.get(21)?,
            request_key: row.get(22)?,
            prev_hash: row.get(23)?,
            hash: row.get(24)?,
        },
        fields,
    ))
}

// ── Reads ───────────────────────────────────────────────────────────────────

/// `tracker.audit`'s filters. Every one is optional; `limit` defaults to 50
/// (at most 500).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct AuditFilter {
    /// An issue id or key.
    pub issue: Option<String>,
    pub session: Option<String>,
    pub workspace: Option<String>,
    /// RFC 3339: rows at or after it.
    pub since: Option<String>,
    pub kind: Option<String>,
    pub limit: Option<usize>,
}

pub(crate) const DEFAULT_LIMIT: usize = 50;
pub(crate) const MAX_LIMIT: usize = 500;

/// The rows matching `filter`, newest first.
pub(crate) fn query(vault: &Path, filter: &AuditFilter) -> Result<Vec<AuditEntry>, String> {
    if let Some(kind) = filter.kind.as_deref() {
        if !kind::ALL.contains(&kind) {
            return Err(format!(
                "Unknown audit kind {kind}: use one of {}.",
                kind::ALL.join(", ")
            ));
        }
    }
    let since = match filter
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => Some(
            DateTime::parse_from_rfc3339(s)
                .map_err(|_| "since must be an RFC 3339 time.".to_string())?
                .with_timezone(&Utc),
        ),
        None => None,
    };
    let mut clauses: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let text = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(issue) = text(&filter.issue) {
        clauses.push("(issue_id = ? OR issue_key = ? COLLATE NOCASE)".into());
        args.push(issue.clone());
        args.push(issue);
    }
    if let Some(session) = text(&filter.session) {
        clauses.push("session_key = ?".into());
        args.push(session);
    }
    let workspace = text(&filter.workspace);
    if let Some(kind) = text(&filter.kind) {
        clauses.push("kind = ?".into());
        args.push(kind);
    }
    let limit = filter.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    session_store::with_read(vault, |conn| {
        // A workspace asks for every id it has had: rows keep the id they
        // were written with (append-only), and a moved project's id changed
        // (`workspace_aliases`).
        let mut clauses = clauses;
        let mut args = args;
        if let Some(workspace) = &workspace {
            let ids = crate::workspace_aliases::Aliases::load(conn).ids_for(workspace);
            clauses.push(format!(
                "workspace_id IN ({})",
                crate::workspace_aliases::placeholders(ids.len())
            ));
            args.extend(ids);
        }
        let sql = format!(
            "SELECT {COLUMNS} FROM audit_log{} ORDER BY rowid DESC",
            if clauses.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", clauses.join(" AND "))
            }
        );
        let mut stmt = conn.prepare(&sql).map_err(|e| format!("audit read: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), entry_row)
            .map_err(|e| format!("audit read: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            let (entry, _) = row.map_err(|e| format!("audit read: {e}"))?;
            // `at` is RFC 3339 from different writers: compare as times.
            if let Some(since) = since {
                let at = DateTime::parse_from_rfc3339(&entry.at).map(|t| t.with_timezone(&Utc));
                if !at.is_ok_and(|at| at >= since) {
                    continue;
                }
            }
            out.push(entry);
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    })
}

/// What [`verify`] found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditVerify {
    pub ok: bool,
    pub rows: u64,
    /// The first row whose hash or link to the row before it is wrong.
    pub first_bad: Option<String>,
    /// Why: `edited` (its hash doesn't match its content, or it holds a
    /// number no append stores: infinite or not a number), `gap` (its
    /// `prev_hash` isn't the row before it: a row is missing or moved) or
    /// `tail` (the chain ends before `audit_head`'s newest row or count: the
    /// newest rows are gone; `first_bad` is the head's id).
    pub problem: Option<String>,
    /// Audit writes that failed in this process (logged; nothing stopped).
    pub write_errors: u64,
}

/// Walk the chain in insertion order and report the first bad row.
pub(crate) fn verify(vault: &Path) -> Result<AuditVerify, String> {
    // One read transaction for the rows and the head, so an append between
    // the two reads is never reported as a deleted tail.
    session_store::with_read(vault, |conn| {
        conn.execute_batch("BEGIN DEFERRED")
            .map_err(|e| format!("audit verify: {e}"))?;
        let result = verify_in(conn);
        let _ = conn.execute_batch("ROLLBACK");
        result
    })
}

pub(crate) fn verify_in(conn: &rusqlite::Connection) -> Result<AuditVerify, String> {
    {
        let mut stmt = conn
            .prepare(&format!("SELECT {COLUMNS} FROM audit_log ORDER BY rowid"))
            .map_err(|e| format!("audit verify: {e}"))?;
        let rows = stmt
            .query_map([], entry_row)
            .map_err(|e| format!("audit verify: {e}"))?;
        let mut out = AuditVerify {
            ok: true,
            write_errors: errors(),
            ..AuditVerify::default()
        };
        let mut prev = String::new();
        let mut last_id = String::new();
        for row in rows {
            let (entry, fields) = row.map_err(|e| format!("audit verify: {e}"))?;
            out.rows += 1;
            if out.first_bad.is_none() {
                let stored = entry.cost_usd.is_none_or(|c| c.is_finite());
                let problem = if entry.prev_hash != prev {
                    Some("gap")
                } else if !stored
                    || chain_hash(&prev, &canonical(&entry, fields.as_deref())) != entry.hash
                {
                    Some("edited")
                } else {
                    None
                };
                if let Some(problem) = problem {
                    out.ok = false;
                    out.first_bad = Some(entry.id.clone());
                    out.problem = Some(problem.into());
                }
            }
            prev = entry.hash;
            last_id = entry.id;
        }
        let head: Option<(String, String, i64)> = conn
            .query_row(
                "SELECT head_id, head_hash, rows FROM audit_head WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|e| format!("audit verify head: {e}"))?;
        let tail = match &head {
            None => out.rows > 0,
            Some((id, hash, rows)) => {
                *id != last_id || *hash != prev || u64::try_from(*rows).ok() != Some(out.rows)
            }
        };
        if tail && out.first_bad.is_none() {
            out.ok = false;
            out.first_bad = Some(head.map(|(id, _, _)| id).unwrap_or(last_id));
            out.problem = Some("tail".into());
        }
        Ok(out)
    }
}

/// `tracker.audit`: the rows, and the chain check when asked.
pub(crate) fn read_action(vault: &Path, filter: AuditFilter, check: bool) -> Result<Value, String> {
    let rows = query(vault, &filter)?;
    // A row written before Move Hollow home keeps its old workspace id; say
    // which workspace that is now, beside it (the row itself never changes).
    let aliases = session_store::with_read(vault, |conn| {
        Ok(crate::workspace_aliases::Aliases::load(conn))
    })
    .unwrap_or_default();
    let mut entries = serde_json::to_value(&rows).map_err(|e| e.to_string())?;
    if let Some(list) = entries.as_array_mut() {
        for (entry, row) in list.iter_mut().zip(&rows) {
            if let Some(now) = row.workspace_id.as_deref().and_then(|w| aliases.moved(w)) {
                entry["currentWorkspaceId"] = json!(now);
            }
        }
    }
    let mut out = json!({
        "entries": entries,
        "count": rows.len(),
        "writeErrors": errors(),
    });
    if check {
        out["verify"] = serde_json::to_value(verify(vault)?).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

// ── Who asked ───────────────────────────────────────────────────────────────

tokio::task_local! {
    static ASKER: String;
}

thread_local! {
    static SYNC_ASKER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Run `f` with `who` as the asker of every tracker write it makes (an agent
/// action's call). Writes outside such a scope are the person's.
pub(crate) async fn asking<F: std::future::Future>(who: String, f: F) -> F::Output {
    ASKER.scope(who, f).await
}

/// [`asking`] for synchronous work on this thread (the task broker's edits).
pub(crate) fn asking_sync<T>(who: &str, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            SYNC_ASKER.with(|cell| *cell.borrow_mut() = prev);
        }
    }
    let prev = SYNC_ASKER.with(|cell| cell.borrow_mut().replace(who.to_string()));
    let _restore = Restore(prev);
    f()
}

/// Who asked for the tracker write being made now: the enclosing
/// [`asking_sync`] or [`asking`] scope, else `person`.
pub(crate) fn asker() -> String {
    SYNC_ASKER
        .with(|cell| cell.borrow().clone())
        .or_else(|| ASKER.try_with(Clone::clone).ok())
        .unwrap_or_else(|| "person".to_string())
}

/// The asker an agent action's caller stands for: its chat session key,
/// `automation:<id>` for an automation's run, and `<surface>:<scope>` for
/// any other run (a delegate, a one-off run such as `ask`).
pub(crate) fn caller_asker(surface: &str, scope: &str) -> String {
    match surface.trim() {
        "chat" => scope.trim().to_string(),
        "workflow" | "automation" => format!("automation:{}", scope.trim()),
        other => format!("{other}:{}", scope.trim()),
    }
}

/// Whether `asked_by` is a session key (not the person and not a
/// `<surface>:<id>` label).
pub(crate) fn is_session(asked_by: &str) -> bool {
    asked_by != "person" && !asked_by.contains(':') && !asked_by.trim().is_empty()
}

// ── Tracker writes ──────────────────────────────────────────────────────────

/// One tracker write Hollow made or refused, for [`tracker_write`].
#[derive(Debug, Clone, Default)]
pub(crate) struct Write<'a> {
    /// `create`, `update`, `comment`, `archive` or `link`.
    pub action: &'a str,
    pub session_key: Option<&'a str>,
    pub asked_by: String,
    pub workspace_id: Option<&'a str>,
    pub issue_id: Option<&'a str>,
    pub issue_key: Option<&'a str>,
    /// Changed fields: `{"before": {...}, "after": {...}}`.
    pub before: Option<Value>,
    pub after: Option<Value>,
    /// The write's idempotency key: a repeat with the same outcome adds no row.
    pub request_key: Option<&'a str>,
    /// `ok`, `failed` or `skipped`.
    pub outcome: &'a str,
    pub detail: Option<String>,
}

/// Record one tracker write: `tracker_write` when it went (or was settled
/// as skipped), `tracker_write_failed` when it was refused or failed.
pub(crate) fn tracker_write(vault: &Path, write: Write<'_>) {
    let mut fields = serde_json::Map::new();
    if let Some(before) = write.before {
        fields.insert("before".into(), before);
    }
    if let Some(after) = write.after {
        fields.insert("after".into(), after);
    }
    let failed = !matches!(write.outcome, "ok" | "skipped");
    let session = write.session_key.filter(|s| !s.trim().is_empty());
    let (parent_key, root_key) = session.map(|s| lineage(vault, s)).unwrap_or_default();
    record(
        vault,
        AuditEntry {
            kind: if failed {
                kind::TRACKER_WRITE_FAILED
            } else {
                kind::TRACKER_WRITE
            }
            .into(),
            session_key: session.map(str::to_string),
            parent_key,
            root_key,
            asked_by: Some(write.asked_by),
            workspace_id: write.workspace_id.map(str::to_string),
            issue_id: write.issue_id.filter(|s| !s.is_empty()).map(str::to_string),
            issue_key: write
                .issue_key
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            action: Some(write.action.into()),
            fields: (!fields.is_empty()).then_some(Value::Object(fields)),
            outcome: Some(write.outcome.into()),
            detail: write.detail,
            request_key: write.request_key.map(str::to_string),
            ..AuditEntry::default()
        },
    );
}

// ── Runs ────────────────────────────────────────────────────────────────────

/// A session's parent and the conversation at the top of its chain (the
/// COO or lead), from `sessions.parent_key`. Both none for a top-level one.
pub(crate) fn lineage(vault: &Path, session_key: &str) -> (Option<String>, Option<String>) {
    let parent = |conn: &Connection, key: &str| -> Option<String> {
        conn.query_row(
            "SELECT parent_key FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .ok()
        .flatten()
        .flatten()
        .filter(|p| !p.trim().is_empty() && p != key)
    };
    session_store::with_read(vault, |conn| {
        let first = parent(conn, session_key);
        Ok((first.clone(), first.map(|p| root_from(conn, &p, parent))))
    })
    .unwrap_or_default()
}

/// The top of `key`'s chain (itself when it has no parent), at most 16 hops.
fn root_from(
    conn: &Connection,
    key: &str,
    parent: impl Fn(&Connection, &str) -> Option<String>,
) -> String {
    let mut at = key.to_string();
    for _ in 0..16 {
        match parent(conn, &at) {
            Some(up) if up != at => at = up,
            _ => break,
        }
    }
    at
}

/// The root above `parent_key` (a start's parent may have no row of its own
/// as a child yet, but its parents do).
pub(crate) fn root_of(vault: &Path, parent_key: &str) -> String {
    let (_, root) = lineage(vault, parent_key);
    root.unwrap_or_else(|| parent_key.to_string())
}

/// What a run used so far, read as the Subagents tab reads it
/// (`subagents::stats`: usage joined through its turns' run ids, the model
/// and account of its newest chat turn), so the Outcome equals the UI.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RunUsage {
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: Option<f64>,
    /// Some usage rows had no price: the cost is a floor.
    pub cost_partial: bool,
    pub working_ms: i64,
    pub turns: i64,
    pub model: Option<String>,
    pub account: Option<String>,
}

pub(crate) fn run_usage(vault: &Path, session_key: &str, now: DateTime<Utc>) -> RunUsage {
    session_store::with_read(vault, |conn| {
        let stats = crate::subagents::stats(conn, &[session_key.to_string()])?;
        let spans: Vec<crate::subagents::Span> =
            stats.turns.iter().map(|t| (t.started, t.ended)).collect();
        let (working_ms, _, _) = crate::subagents::spans(&spans, now);
        Ok(RunUsage {
            tokens_in: stats.input,
            tokens_out: stats.output,
            cost_usd: clean(stats.cost),
            cost_partial: stats.priced > 0 && stats.rows > stats.priced,
            working_ms,
            turns: spans.len() as i64,
            model: stats.model,
            account: stats.account,
        })
    })
    .unwrap_or_default()
}

/// The newest `kind` row of `session_key`.
pub(crate) fn latest(vault: &Path, session_key: &str, kind: &str) -> Option<AuditEntry> {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!(
                "SELECT {COLUMNS} FROM audit_log WHERE session_key = ?1 AND kind = ?2 \
                 ORDER BY rowid DESC LIMIT 1"
            ),
            params![session_key, kind],
            entry_row,
        )
        .optional()
        .map_err(|e| format!("audit read: {e}"))
    })
    .ok()
    .flatten()
    .map(|(entry, _)| entry)
}

/// Every `kind` row of `session_key`, oldest first.
pub(crate) fn rows_of(vault: &Path, session_key: &str, kind: &str) -> Vec<AuditEntry> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM audit_log WHERE session_key = ?1 AND kind = ?2 ORDER BY rowid"
            ))
            .map_err(|e| format!("audit read: {e}"))?;
        let rows = stmt
            .query_map(params![session_key, kind], entry_row)
            .map_err(|e| format!("audit read: {e}"))?
            .filter_map(Result::ok)
            .map(|(entry, _)| entry)
            .collect();
        Ok(rows)
    })
    .unwrap_or_default()
}

/// A run's totals as its audit has them: the newest `run_ended` row of each
/// of `keys` (each row holds its run's totals so far), summed. None when
/// the first key (the run itself) has no such row yet.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RunSums {
    pub working_ms: i64,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    pub cost_partial: bool,
}

pub(crate) fn run_sums(vault: &Path, keys: &[String]) -> Option<RunSums> {
    let first = keys.first()?;
    latest(vault, first, kind::RUN_ENDED)?;
    let mut sums = RunSums::default();
    for key in keys {
        let Some(row) = latest(vault, key, kind::RUN_ENDED) else {
            continue;
        };
        sums.working_ms += row.working_ms.unwrap_or(0);
        sums.tokens += row.tokens_in.unwrap_or(0) + row.tokens_out.unwrap_or(0);
        if let Some(cost) = row.cost_usd {
            sums.cost_usd = Some(sums.cost_usd.unwrap_or(0.0) + cost);
        }
        sums.cost_partial |= row
            .fields
            .as_ref()
            .and_then(|f| f.get("costPartial"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
    }
    Some(sums)
}

/// Test-only: change the table behind the triggers' back (they are dropped
/// first), as an attacker with file access could.
#[cfg(test)]
pub(crate) fn tamper(vault: &Path, sql: &str) {
    session_store::with_write(vault, "audit tamper", |tx| {
        tx.execute_batch(&format!(
            "DROP TRIGGER IF EXISTS audit_log_no_update; DROP TRIGGER IF EXISTS audit_log_no_delete; {sql}"
        ))
        .map_err(|e| e.to_string())
    })
    .unwrap();
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
