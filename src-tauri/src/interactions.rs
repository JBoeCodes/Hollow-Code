//! Structured question cards (Paperclip PR 13): the store.
//!
//! One row per card in the `interactions` table of the session store, which
//! lives OUTSIDE the vault (no history, receipt or `run_guard` involvement).
//! Like the usage ledger, the rows are control state, not a derived cache:
//! every read-then-write goes through `session_store::with_write` (IMMEDIATE,
//! because `hollow team-mcp` co-writes the database) and every compared
//! timestamp is `db_time`. Terminal rows are pruned after [`KEEP_DAYS`]; open
//! rows (`pending`, `stale`) never are.
//!
//! Lifecycle:
//! - [`record`] (from the chat turn's drive closure, once the completed tool
//!   event says the MCP posted the card) re-validates the request, resolves
//!   and hashes a bound target file, and inserts the row keyed by
//!   `(session_key, run_id, tool_use_id)`. Codex item ids restart at `item_0`
//!   on every exec, resumes included, so the run is part of the key. Recording
//!   supersedes the session's other open cards (`newer_interaction`).
//! - A later chat turn the person sent supersedes open cards flagged
//!   `supersedeOnUserMessage` ([`supersede_for_message`], called once the
//!   agent has SPAWNED, so a send that fails never destroys a card). A wake,
//!   check-in, pulse or scheduled turn does not. Archiving a conversation expires its
//!   open cards in the same transaction ([`expire_pending_in`]).
//! - [`resolve`] answers a pending card: the target re-hashed first, the
//!   answer validated, then a compare-and-set to `answered` that stores the
//!   formatted prompt and `answered_after_ord`, the session's newest turn
//!   ordinal. The frontend sends exactly that prompt, each question named in
//!   full: into the native session that asked when the conversation still
//!   holds it, else (2026-11-04) as an ordinary message
//!   ([`ResolvedInteraction`]'s `resumes`), so a filled-in card always goes.
//! - An answer whose turn never ran comes back: [`reopen`] while no later turn
//!   has started, [`reopen_unstarted`] when the answer turn itself failed to
//!   spawn (it is by construction the first turn after the answer).
//!
//! Lock order: only the session store's leaf `WRITE_LOCK` (through
//! `with_write`). Never call these while holding `chat_rows::CHAT_ROW_ACTIVE`, the
//! run queue's `WAITERS` or the chat stop slot.

use crate::agent::AgentKind;
use crate::agent_sessions::is_safe_session_key;
use crate::interaction_schema::{self, QuestionRequest};
use crate::session_store::{self, db_time};
use crate::types::{ChatInteraction, ResolvedInteraction};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Terminal rows are kept this long, then pruned by session-store retention.
pub const KEEP_DAYS: i64 = 90;
/// A bound target file is hashed only up to this size.
const TARGET_MAX_BYTES: u64 = 8 * 1024 * 1024;

pub const CLOSED: &str = "INTERACTION_CLOSED";
pub const STALE: &str = "INTERACTION_STALE";
pub const INVALID: &str = "INTERACTION_INVALID";

/// Statuses a card is still shown in.
const OPEN: &str = "('pending', 'stale')";

/// Where a card came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionSource {
    /// A completed `ask_user_questions` tool event.
    Mcp,
    /// Antigravity's fenced `hollow-questions` block.
    Fenced,
}

impl InteractionSource {
    fn as_str(self) -> &'static str {
        match self {
            InteractionSource::Mcp => "mcp",
            InteractionSource::Fenced => "fenced",
        }
    }
}

/// A card to record, as the chat turn saw it.
pub struct NewInteraction<'a> {
    pub session_key: &'a str,
    pub turn_ord: i64,
    pub run_id: &'a str,
    pub agent: AgentKind,
    pub native_session_id: Option<&'a str>,
    pub tool_use_id: &'a str,
    pub source: InteractionSource,
    /// The request, or why it could not be read.
    pub arguments: &'a Result<Value, String>,
    /// The run's working folder: relative target paths resolve against it.
    pub cwd: &'a Path,
    pub vault_root: &'a Path,
}

/// What [`record`] did: the card, and the open cards it superseded.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub interaction: ChatInteraction,
    pub superseded: Vec<ChatInteraction>,
}

const COLUMNS: &str = "id, session_key, turn_ord, run_id, agent, native_session_id, tool_use_id, \
    source, mode, request_json, target_path, supersede_on_user_message, status, status_reason, \
    superseded_by, answer_json, created_at, resolved_at";

fn row_to_interaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChatInteraction> {
    let request: String = row.get(9)?;
    let answer: Option<String> = row.get(15)?;
    Ok(ChatInteraction {
        id: row.get(0)?,
        session_key: row.get(1)?,
        turn_ord: row.get(2)?,
        run_id: row.get(3)?,
        agent: row.get(4)?,
        native_session_id: row.get(5)?,
        tool_use_id: row.get(6)?,
        source: row.get(7)?,
        mode: row.get(8)?,
        request: serde_json::from_str(&request).unwrap_or(Value::Null),
        target_path: row.get(10)?,
        supersede_on_user_message: row.get::<_, i64>(11)? != 0,
        status: row.get(12)?,
        status_reason: row.get(13)?,
        superseded_by: row.get(14)?,
        answer: answer.and_then(|a| serde_json::from_str(&a).ok()),
        created_at: row.get(16)?,
        resolved_at: row.get(17)?,
    })
}

fn get_in(conn: &Connection, id: &str) -> Result<Option<ChatInteraction>, String> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM interactions WHERE id = ?1"),
        params![id],
        row_to_interaction,
    )
    .optional()
    .map_err(|e| format!("interaction read: {e}"))
}

fn rows_in(conn: &Connection, ids: &[String]) -> Result<Vec<ChatInteraction>, String> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(row) = get_in(conn, id)? {
            out.push(row);
        }
    }
    Ok(out)
}

fn ids_where<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    args: P,
) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(sql)
        .map_err(|e| format!("interaction query: {e}"))?;
    let rows = statement
        .query_map(args, |row| row.get::<_, String>(0))
        .map_err(|e| format!("interaction query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("interaction rows: {e}"))
}

fn new_id() -> String {
    format!("qi-{:016x}", rand::random::<u64>())
}

fn now() -> String {
    db_time(Utc::now())
}

/// The newest turn ordinal of a session (0 when it has none).
fn max_turn_ord(conn: &Connection, key: &str) -> Result<i64, String> {
    conn.query_row(
        "SELECT COALESCE(MAX(ord), 0) FROM turns WHERE session_key = ?1",
        params![key],
        |row| row.get(0),
    )
    .map_err(|e| format!("turn ord: {e}"))
}

/// Resolve a card's target against the run's working folder (or take it as
/// absolute), canonicalize it, and require it inside the canonical working
/// folder or the canonical vault root, as a regular file of at most 8 MiB.
fn resolve_target(path: &str, cwd: &Path, vault_root: &Path) -> Result<PathBuf, String> {
    let given = Path::new(path);
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        cwd.join(given)
    };
    let canonical = std::fs::canonicalize(&joined).map_err(|_| "target not found".to_string())?;
    let roots = [
        Some(cwd),
        (!vault_root.as_os_str().is_empty()).then_some(vault_root),
    ];
    let inside = roots
        .into_iter()
        .flatten()
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .any(|root| canonical.starts_with(&root));
    if !inside {
        return Err("target is outside the working folder and the vault".into());
    }
    let meta = std::fs::metadata(&canonical).map_err(|_| "target not found".to_string())?;
    if !meta.is_file() || meta.len() > TARGET_MAX_BYTES {
        return Err("target is not a regular file of at most 8 MiB".into());
    }
    Ok(canonical)
}

/// The sha256 of a target file. `metadata()` is checked before any open, the
/// open is non-blocking (so an agent-made FIFO or device can never block the
/// chat turn, which holds the vault lease), the handle is re-checked, and the
/// read is bounded.
fn read_target_hash(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "target not found".to_string())?;
    if !meta.is_file() || meta.len() > TARGET_MAX_BYTES {
        return Err("target is not a regular file of at most 8 MiB".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| "target not found".to_string())?;
    let handle = file
        .metadata()
        .map_err(|_| "target not found".to_string())?;
    if !handle.is_file() || handle.len() > TARGET_MAX_BYTES {
        return Err("target is not a regular file of at most 8 MiB".into());
    }
    let mut hasher = Sha256::new();
    let mut reader = file.take(TARGET_MAX_BYTES + 1);
    let mut buffer = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader
            .read(&mut buffer)
            .map_err(|e| format!("target read: {e}"))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > TARGET_MAX_BYTES {
            return Err("target is larger than 8 MiB".into());
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Record a card. Validation runs again here (the MCP subprocess is not
/// trusted); a bound target is resolved and hashed OUTSIDE the transaction.
/// A target that cannot be read records the card as `stale` ("target not
/// found"). Re-recording the same `(session_key, run_id, tool_use_id)`
/// returns the existing row and supersedes nothing.
pub fn record(vault: &Path, new: &NewInteraction<'_>) -> Result<Recorded, String> {
    if !is_safe_session_key(new.session_key) {
        return Err(format!("unsafe session key: {}", new.session_key));
    }
    let arguments = new
        .arguments
        .as_ref()
        .map_err(|e| format!("its arguments could not be read ({e})"))?;
    let request: QuestionRequest = interaction_schema::validate_request(arguments)?;
    let request_json = serde_json::to_string(&request).map_err(|e| format!("request: {e}"))?;
    let target_path = request.target_path().map(str::to_string);
    let (status, reason, target_abs, target_hash) = match &target_path {
        None => ("pending", None, None, None),
        Some(path) => match resolve_target(path, new.cwd, new.vault_root)
            .and_then(|abs| read_target_hash(&abs).map(|hash| (abs, hash)))
        {
            Ok((abs, hash)) => (
                "pending",
                None,
                Some(abs.to_string_lossy().into_owned()),
                Some(hash),
            ),
            Err(_) => ("stale", Some("target not found"), None, None),
        },
    };
    let id = new_id();
    let created = now();
    session_store::with_write(vault, "interaction record", |tx| {
        let existing: Option<String> = tx
            .query_row(
                "SELECT id FROM interactions WHERE session_key = ?1 AND run_id = ?2 AND tool_use_id = ?3",
                params![new.session_key, new.run_id, new.tool_use_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("interaction lookup: {e}"))?;
        if let Some(existing) = existing {
            let interaction = get_in(tx, &existing)?.ok_or("interaction vanished")?;
            return Ok(Recorded {
                interaction,
                superseded: Vec::new(),
            });
        }
        tx.execute(
            "INSERT INTO interactions(id, session_key, turn_ord, run_id, agent, native_session_id, \
             tool_use_id, source, mode, request_json, target_path, target_abs, target_hash, \
             supersede_on_user_message, status, status_reason, created_at, resolved_at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, \
             CASE WHEN ?15 = 'stale' THEN ?17 ELSE NULL END)",
            params![
                id,
                new.session_key,
                new.turn_ord,
                new.run_id,
                new.agent.as_str(),
                new.native_session_id,
                new.tool_use_id,
                new.source.as_str(),
                request.mode.as_str(),
                request_json,
                target_path,
                target_abs,
                target_hash,
                request.supersede_on_user_message as i64,
                status,
                reason,
                created,
            ],
        )
        .map_err(|e| format!("interaction insert: {e}"))?;
        let older = ids_where(
            tx,
            &format!("SELECT id FROM interactions WHERE session_key = ?1 AND id != ?2 AND status IN {OPEN} ORDER BY created_at, rowid"),
            params![new.session_key, id],
        )?;
        tx.execute(
            &format!(
                "UPDATE interactions SET status = 'superseded', status_reason = 'newer_interaction', \
                 superseded_by = ?2, resolved_at = ?3 WHERE session_key = ?1 AND id != ?2 AND status IN {OPEN}"
            ),
            params![new.session_key, id, created],
        )
        .map_err(|e| format!("interaction supersede: {e}"))?;
        let interaction = get_in(tx, &id)?.ok_or("interaction vanished")?;
        Ok(Recorded {
            interaction,
            superseded: rows_in(tx, &older)?,
        })
    })
}

/// The `chat-event` payload for a card.
pub fn event_payload(interaction: &ChatInteraction) -> Value {
    crate::chat_event::build::interaction(interaction, &interaction.session_key)
}

/// Record a card and emit it (after the cards it superseded), or, when it
/// cannot be shown, append an `interactionRejected` trace event and emit a
/// chat `error` telling the user to reply in the composer: an agent must never
/// be left waiting on an invisible card. AppHandle-free so tests drive it.
pub fn persist_and_notify(vault: &Path, new: &NewInteraction<'_>, emit: &mut dyn FnMut(Value)) {
    match record(vault, new) {
        Ok(recorded) => {
            for older in &recorded.superseded {
                emit(event_payload(older));
            }
            emit(event_payload(&recorded.interaction));
        }
        Err(reason) => {
            let _ = session_store::append_event(
                vault,
                new.session_key,
                new.turn_ord,
                "interactionRejected",
                &json!({ "toolUseId": new.tool_use_id, "reason": reason }).to_string(),
            );
            emit(crate::chat_event::build::error(format!(
                "The agent asked questions Hollow could not show: {reason}. Reply in the composer instead."
            )));
        }
    }
}

/// Supersede the open cards a later chat turn answers in free text: those
/// flagged `supersedeOnUserMessage` and recorded in an earlier turn. Called
/// once the turn's agent has spawned. A card on an earlier conversation in
/// the same chain closes too: the dock shows it on the current chat.
/// Returns the superseded rows.
pub fn supersede_for_message(
    vault: &Path,
    key: &str,
    turn_ord: i64,
) -> Result<Vec<ChatInteraction>, String> {
    if !is_safe_session_key(key) || turn_ord <= 0 {
        return Ok(Vec::new());
    }
    // Read the chain before the session write lock. `chain_keys` can take
    // the run-queue lock for a Main agent conversation.
    let earlier = crate::chat_chain::chain_keys(vault, key);
    let at = now();
    session_store::with_write(vault, "interaction supersede", |tx| {
        let mut ids = ids_where(
            tx,
            &format!(
                "SELECT id FROM interactions WHERE session_key = ?1 AND status IN {OPEN} \
                 AND supersede_on_user_message = 1 AND turn_ord < ?2 ORDER BY created_at, rowid"
            ),
            params![key, turn_ord],
        )?;
        for earlier_key in &earlier {
            if earlier_key == key {
                continue;
            }
            ids.extend(ids_where(
                tx,
                &format!(
                    "SELECT id FROM interactions WHERE session_key = ?1 AND status IN {OPEN} \
                     AND supersede_on_user_message = 1 ORDER BY created_at, rowid"
                ),
                params![earlier_key],
            )?);
        }
        ids.sort();
        ids.dedup();
        for id in &ids {
            tx.execute(
                "UPDATE interactions SET status = 'superseded', status_reason = 'user_message', \
                 resolved_at = ?2 WHERE id = ?1",
                params![id, at],
            )
            .map_err(|e| format!("interaction supersede: {e}"))?;
        }
        rows_in(tx, &ids)
    })
}

/// How long an open question card, or an unanswered workspace ask-canvas,
/// holds a wake, check-in, pulse or scheduled turn.
pub const CARD_HOLD_MINUTES: i64 = 30;

fn card_release(opened: chrono::DateTime<Utc>) -> chrono::DateTime<Utc> {
    opened + chrono::Duration::minutes(CARD_HOLD_MINUTES)
}

/// The latest instant an open card in `key` still holds a non-person turn.
/// Each card releases at `created_at` plus [`CARD_HOLD_MINUTES`]. The hold is
/// the latest release still in the future, so the youngest card keeps its
/// full half hour. `None` when every open card is already past that, or when
/// the list cannot be read (the queue must not stall on a store error).
pub fn card_hold_until(
    vault: &Path,
    key: &str,
    now: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    let pending = match list_pending(vault, key) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("[interactions] card hold skipped: {error}");
            return None;
        }
    };
    let mut latest: Option<chrono::DateTime<Utc>> = None;
    for card in pending {
        let Some(created) = session_store::parse_iso(&card.created_at) else {
            continue;
        };
        let release = card_release(created);
        if release > now {
            latest = Some(latest.map_or(release, |at| at.max(release)));
        }
    }
    latest
}

/// An unanswered ask-canvas in this conversation's workspace, younger than
/// [`CARD_HOLD_MINUTES`]. App-data canvases (chats with no workspace) are
/// not scanned: that walk covers every session. Any read error skips the
/// ask hold and leaves the question-card hold alone.
fn ask_hold_until(
    vault: &Path,
    key: &str,
    now: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    let asks = match session_store::session_asks(vault, key, crate::next_step::CANVAS_ANSWER_LANG) {
        Ok(Some(asks)) => asks,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("[interactions] ask-canvas hold skipped: {error}");
            return None;
        }
    };
    if asks.archived {
        return None;
    }
    let cwd = PathBuf::from(&asks.cwd);
    let workspace = match crate::workspaces::workspace_for_cwd(vault, &cwd) {
        Ok(Some(workspace)) => workspace,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("[interactions] ask-canvas hold skipped: {error}");
            return None;
        }
    };
    let folder = match crate::workspaces::folder(vault, &workspace.id) {
        Ok(folder) => folder,
        Err(error) => {
            eprintln!("[interactions] ask-canvas hold skipped: {error}");
            return None;
        }
    };
    let heads = match crate::workspace_canvases::workspace_ask_heads(&folder) {
        Ok(heads) => heads,
        Err(error) => {
            eprintln!("[interactions] ask-canvas hold skipped: {error}");
            return None;
        }
    };
    let mut latest: Option<chrono::DateTime<Utc>> = None;
    for head in heads {
        if head.session.as_deref() != Some(key) {
            continue;
        }
        let answer = (head.id.clone(), head.revision);
        if asks
            .asked
            .iter()
            .any(|text| crate::canvas_answers::answered(text).contains(&answer))
        {
            continue;
        }
        let Some(saved) = session_store::parse_iso(&head.saved_at) else {
            continue;
        };
        let release = card_release(saved);
        if release > now {
            latest = Some(latest.map_or(release, |at| at.max(release)));
        }
    }
    latest
}

fn holds_for(vault: &Path, key: &str, now: chrono::DateTime<Utc>) -> Option<chrono::DateTime<Utc>> {
    match (
        card_hold_until(vault, key, now),
        ask_hold_until(vault, key, now),
    ) {
        (Some(card), Some(ask)) => Some(card.max(ask)),
        (card, ask) => card.or(ask),
    }
}

/// When a non-person turn for `key` may start. The later of the question-card
/// hold and the workspace ask-canvas hold, or `None` when neither is open
/// inside the half hour. A card on an earlier conversation in the same chain
/// counts: the dock shows it here. The pump sleeps until then instead of
/// writing `not_before`, so an answer before the cap lets the next step start.
pub fn open_hold_until(
    vault: &Path,
    key: &str,
    now: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    if !is_safe_session_key(key) {
        return None;
    }
    let mut latest = holds_for(vault, key, now);
    for earlier in crate::chat_chain::chain_keys(vault, key) {
        if earlier == key {
            continue;
        }
        let Some(at) = holds_for(vault, &earlier, now) else {
            continue;
        };
        latest = Some(latest.map_or(at, |have| have.max(at)));
    }
    latest
}

/// A session's open cards (pending and stale), oldest first.
pub fn list_pending(vault: &Path, key: &str) -> Result<Vec<ChatInteraction>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    session_store::with_read(vault, |conn| {
        let ids = ids_where(
            conn,
            &format!("SELECT id FROM interactions WHERE session_key = ?1 AND status IN {OPEN} ORDER BY created_at, rowid"),
            params![key],
        )?;
        rows_in(conn, &ids)
    })
}

/// A session's cards in every status (open and closed), oldest first: the
/// phone API shows each turn's card, answered or not (`phone_access`).
/// Closed cards are kept [`KEEP_DAYS`] after they close.
pub fn list_for_session(vault: &Path, key: &str) -> Result<Vec<ChatInteraction>, String> {
    if !is_safe_session_key(key) {
        return Err(format!("unsafe session key: {key}"));
    }
    session_store::with_read(vault, |conn| {
        let ids = ids_where(
            conn,
            "SELECT id FROM interactions WHERE session_key = ?1 ORDER BY created_at, rowid",
            params![key],
        )?;
        rows_in(conn, &ids)
    })
}

/// Every conversation's open cards (pending and stale), newest first, at most
/// `limit`: the activity view's "Waiting on you" (next-level P19).
pub fn list_open_all(vault: &Path, limit: usize) -> Result<Vec<ChatInteraction>, String> {
    session_store::with_read(vault, |conn| {
        let ids = ids_where(
            conn,
            &format!("SELECT id FROM interactions WHERE status IN {OPEN} ORDER BY created_at DESC, rowid DESC LIMIT ?1"),
            params![limit.clamp(1, 200) as i64],
        )?;
        rows_in(conn, &ids)
    })
}

/// One card by id, whatever its status (T3b reads how it ended).
pub fn get(vault: &Path, id: &str) -> Result<Option<ChatInteraction>, String> {
    session_store::with_read(vault, |conn| get_in(conn, id))
}

/// Every conversation's open cards (pending and stale), newest first, with
/// no cap: the "needs you" query pages them itself (B·F15).
pub fn list_open_every(vault: &Path) -> Result<Vec<ChatInteraction>, String> {
    session_store::with_read(vault, |conn| {
        let ids = ids_where(
            conn,
            &format!("SELECT id FROM interactions WHERE status IN {OPEN} ORDER BY created_at DESC, rowid DESC"),
            params![],
        )?;
        rows_in(conn, &ids)
    })
}

fn read_one(
    vault: &Path,
    id: &str,
) -> Result<Option<(ChatInteraction, Option<String>, Option<String>)>, String> {
    session_store::with_read(vault, |conn| {
        let Some(row) = get_in(conn, id)? else {
            return Ok(None);
        };
        let (abs, hash): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT target_abs, target_hash FROM interactions WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| format!("interaction read: {e}"))?;
        Ok(Some((row, abs, hash)))
    })
}

/// Mark a pending card stale (compare-and-set) and return the refusal.
fn mark_stale(vault: &Path, id: &str, reason: &str) -> String {
    let at = now();
    let _ = session_store::with_write(vault, "interaction stale", |tx| {
        tx.execute(
            "UPDATE interactions SET status = 'stale', status_reason = ?2, resolved_at = ?3 \
             WHERE id = ?1 AND status = 'pending'",
            params![id, reason, at],
        )
        .map_err(|e| format!("interaction stale: {e}"))
    });
    format!("{STALE}: the file this question is about changed; the card can no longer be answered")
}

/// Answer a pending card. `native` is the native session the conversation
/// resumes now: `resumes` says whether that is the card's own. Either way the
/// card is answered (2026-11-04: the turn after a Main check-in had started a
/// fresh session and Send stayed disabled), and the prompt names each
/// question in full ([`format_answers`]), so it reads on its own in whatever
/// session gets it, an ordinary message when not the asking one. Refusals
/// carry a prefix the UI keys on: `INTERACTION_CLOSED` (not pending any
/// more), `INTERACTION_STALE` (its target changed; the card goes stale and no
/// turn is sent), `INTERACTION_INVALID` (the answer does not fit). Reads the
/// target file, so the command runs it on a blocking worker.
///
/// [`format_answers`]: interaction_schema::format_answers
pub fn resolve(
    vault: &Path,
    id: &str,
    answer: &Value,
    native: Option<&str>,
) -> Result<ResolvedInteraction, String> {
    let Some((row, target_abs, target_hash)) = read_one(vault, id)? else {
        return Err(format!("{CLOSED}: this question card no longer exists"));
    };
    if row.status != "pending" {
        return Err(format!(
            "{CLOSED}: this question card is already {}",
            row.status
        ));
    }
    let resumes = matches!(
        (row.native_session_id.as_deref(), native),
        (Some(expected), Some(given)) if expected == given
    );
    if let (Some(abs), Some(hash)) = (target_abs.as_deref(), target_hash.as_deref()) {
        match read_target_hash(Path::new(abs)) {
            Ok(current) if current == hash => {}
            _ => return Err(mark_stale(vault, id, "target changed")),
        }
    }
    let request: QuestionRequest = serde_json::from_value(row.request.clone())
        .map_err(|e| format!("{INVALID}: the stored question could not be read: {e}"))?;
    let answer = interaction_schema::validate_answer(&request, answer)
        .map_err(|e| format!("{INVALID}: {e}"))?;
    let prompt = interaction_schema::format_answers(&request, &answer);
    let answer_json = serde_json::to_string(&answer).map_err(|e| format!("{INVALID}: {e}"))?;
    let at = now();
    let interaction = session_store::with_write(vault, "interaction resolve", |tx| {
        let after = max_turn_ord(tx, &row.session_key)?;
        let changed = tx
            .execute(
                "UPDATE interactions SET status = 'answered', status_reason = NULL, answer_json = ?2, \
                 answer_prompt = ?3, answered_after_ord = ?4, resolved_at = ?5 \
                 WHERE id = ?1 AND status = 'pending'",
                params![id, answer_json, prompt, after, at],
            )
            .map_err(|e| format!("interaction resolve: {e}"))?;
        if changed == 0 {
            return Err(format!("{CLOSED}: this question card was closed meanwhile"));
        }
        get_in(tx, id)?.ok_or_else(|| format!("{CLOSED}: this question card no longer exists"))
    })?;
    Ok(ResolvedInteraction {
        interaction,
        prompt,
        resumes,
    })
}

/// Dismiss an open card without sending anything.
pub fn dismiss(vault: &Path, id: &str) -> Result<ChatInteraction, String> {
    let at = now();
    session_store::with_write(vault, "interaction dismiss", |tx| {
        let changed = tx
            .execute(
                &format!(
                    "UPDATE interactions SET status = 'dismissed', status_reason = 'user', resolved_at = ?2 \
                     WHERE id = ?1 AND status IN {OPEN}"
                ),
                params![id, at],
            )
            .map_err(|e| format!("interaction dismiss: {e}"))?;
        if changed == 0 {
            return Err(format!("{CLOSED}: this question card is not open"));
        }
        get_in(tx, id)?.ok_or_else(|| format!("{CLOSED}: this question card no longer exists"))
    })
}

fn reopen_in(conn: &Connection, id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE interactions SET status = 'pending', status_reason = NULL, answer_json = NULL, \
         answer_prompt = NULL, answered_after_ord = NULL, resolved_at = NULL WHERE id = ?1",
        params![id],
    )
    .map_err(|e| format!("interaction reopen: {e}"))?;
    Ok(())
}

/// Put an answered card back to pending while no turn has started since it
/// was answered: its answer turn never ran (busy, already queued, withdrawn or
/// dropped from the queue). `INTERACTION_CLOSED` otherwise.
pub fn reopen(vault: &Path, id: &str) -> Result<ChatInteraction, String> {
    session_store::with_write(vault, "interaction reopen", |tx| {
        let Some(row) = get_in(tx, id)? else {
            return Err(format!("{CLOSED}: this question card no longer exists"));
        };
        if row.status != "answered" {
            return Err(format!("{CLOSED}: this question card is {}", row.status));
        }
        let after: Option<i64> = tx
            .query_row(
                "SELECT answered_after_ord FROM interactions WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .map_err(|e| format!("interaction read: {e}"))?;
        if after != Some(max_turn_ord(tx, &row.session_key)?) {
            return Err(format!(
                "{CLOSED}: a turn already started after this answer"
            ));
        }
        reopen_in(tx, id)?;
        get_in(tx, id)?.ok_or_else(|| format!("{CLOSED}: this question card no longer exists"))
    })
}

/// Put back the cards answered just before `turn_ord`, whose answer turn it
/// is: that turn started (so [`reopen`] refuses) but its agent never ran
/// (spawn failed, no stdout, stopped or failed before spawn). Chat runs one
/// turn at a time and Submit keeps the composer busy, so the first turn after
/// an answer is its answer turn.
pub fn reopen_unstarted(
    vault: &Path,
    key: &str,
    turn_ord: i64,
) -> Result<Vec<ChatInteraction>, String> {
    if !is_safe_session_key(key) || turn_ord <= 0 {
        return Ok(Vec::new());
    }
    session_store::with_write(vault, "interaction reopen", |tx| {
        let ids = ids_where(
            tx,
            "SELECT id FROM interactions WHERE session_key = ?1 AND status = 'answered' \
             AND answered_after_ord = ?2 ORDER BY created_at, rowid",
            params![key, turn_ord - 1],
        )?;
        for id in &ids {
            reopen_in(tx, id)?;
        }
        rows_in(tx, &ids)
    })
}

/// [`reopen_unstarted`] for a turn that had already moved the conversation
/// to a fresh native session (Paperclip PR 14b): expire the cards its answer
/// carried as `reason` instead of reopening them. Their answer stays
/// recorded, and the answer turn's own text stays in the transcript, which
/// the next fresh session's bundle carries, so a reopened card would only
/// ask again what was answered. (Until 2026-11-04 `resolve` also refused
/// their abandoned session.) Returns them for the caller to emit.
pub fn expire_unstarted(
    vault: &Path,
    key: &str,
    turn_ord: i64,
    reason: &str,
) -> Result<Vec<ChatInteraction>, String> {
    if !is_safe_session_key(key) || turn_ord <= 0 {
        return Ok(Vec::new());
    }
    session_store::with_write(vault, "interaction expire unstarted", |tx| {
        let ids = ids_where(
            tx,
            "SELECT id FROM interactions WHERE session_key = ?1 AND status = 'answered' \
             AND answered_after_ord = ?2 ORDER BY created_at, rowid",
            params![key, turn_ord - 1],
        )?;
        let at = db_time(Utc::now());
        for id in &ids {
            tx.execute(
                "UPDATE interactions SET status = 'expired', status_reason = ?2, resolved_at = ?3 WHERE id = ?1",
                params![id, reason, at],
            )
            .map_err(|e| format!("interaction expire: {e}"))?;
        }
        rows_in(tx, &ids)
    })
}

/// Expire a session's open cards (archiving the conversation). One UPDATE,
/// no row reads, inside the caller's IMMEDIATE transaction. Unarchiving does
/// not revive them.
pub(crate) fn expire_pending_in(
    conn: &Connection,
    key: &str,
    at: &str,
    reason: &str,
) -> Result<usize, String> {
    conn.execute(
        &format!(
            "UPDATE interactions SET status = 'expired', status_reason = ?3, resolved_at = ?2 \
             WHERE session_key = ?1 AND status IN {OPEN}"
        ),
        params![key, at, reason],
    )
    .map_err(|e| format!("interaction expire: {e}"))
}

/// [`expire_pending_in`], returning the cards it expired (oldest first), so
/// the caller can emit them. Paperclip PR 14b calls it from
/// `session_store::restart_stint`, in the same transaction: the old native
/// session that asked is gone, so its open cards expire as `session_reset`
/// (their questions stay in the transcript the fresh session's bundle
/// carries; since 2026-11-04 `resolve` would take an answer from any
/// session, but a reset is a fresh start).
pub(crate) fn expire_pending_rows_in(
    conn: &Connection,
    key: &str,
    at: &str,
    reason: &str,
) -> Result<Vec<ChatInteraction>, String> {
    let ids = ids_where(
        conn,
        &format!("SELECT id FROM interactions WHERE session_key = ?1 AND status IN {OPEN} ORDER BY created_at, rowid"),
        params![key],
    )?;
    expire_pending_in(conn, key, at, reason)?;
    rows_in(conn, &ids)
}

/// Prune terminal cards resolved before `cutoff` (a `db_time`). Open cards
/// are never pruned.
pub(crate) fn prune_in(conn: &Connection, cutoff: &str) -> Result<usize, String> {
    conn.execute(
        &format!("DELETE FROM interactions WHERE status NOT IN {OPEN} AND resolved_at IS NOT NULL AND resolved_at < ?1"),
        params![cutoff],
    )
    .map_err(|e| format!("interaction retention: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_schema::tests::{layout_request, replay, CLAUDE_ASK, CODEX_ASK};

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    const KEY: &str = "s-20260926-aaaa0001";

    fn start(vault: &Path, key: &str, run: &str) -> i64 {
        session_store::start_turn(vault, key, "claude", &vault.to_string_lossy(), run, "hello")
            .unwrap()
    }

    struct Ask {
        key: &'static str,
        run: String,
        ord: i64,
        tool: String,
        native: Option<String>,
        arguments: Result<Value, String>,
    }

    impl Ask {
        fn new(run: &str, ord: i64, tool: &str) -> Ask {
            Ask {
                key: KEY,
                run: run.into(),
                ord,
                tool: tool.into(),
                native: Some("native-1".into()),
                arguments: Ok(layout_request()),
            }
        }

        fn record(&self, vault: &Path) -> Result<Recorded, String> {
            record(vault, &self.new_interaction(vault))
        }

        fn new_interaction<'a>(&'a self, vault: &'a Path) -> NewInteraction<'a> {
            NewInteraction {
                session_key: self.key,
                turn_ord: self.ord,
                run_id: &self.run,
                agent: AgentKind::Claude,
                native_session_id: self.native.as_deref(),
                tool_use_id: &self.tool,
                source: InteractionSource::Mcp,
                arguments: &self.arguments,
                cwd: vault,
                vault_root: vault,
            }
        }
    }

    fn grid() -> Value {
        json!({ "answers": { "layout": { "selected": ["grid"] } } })
    }

    #[test]
    fn record_is_idempotent_within_a_run_and_supersedes_older_pending() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        let a = Ask::new("r1", ord, "toolu-a").record(v).unwrap();
        assert_eq!(a.interaction.status, "pending");
        assert!(a.superseded.is_empty());
        assert!(a.interaction.id.starts_with("qi-") && a.interaction.id.len() == 19);

        let other_key = "s-20260926-bbbb0002";
        start(v, other_key, "r9");
        let mut elsewhere = Ask::new("r9", 1, "toolu-z");
        elsewhere.key = other_key;
        let untouched = elsewhere.record(v).unwrap().interaction;

        let b = Ask::new("r1", ord, "toolu-b").record(v).unwrap();
        assert_eq!(b.superseded.len(), 1);
        assert_eq!(b.superseded[0].id, a.interaction.id);
        assert_eq!(b.superseded[0].status, "superseded");
        assert_eq!(
            b.superseded[0].status_reason.as_deref(),
            Some("newer_interaction")
        );
        assert_eq!(
            b.superseded[0].superseded_by.as_deref(),
            Some(b.interaction.id.as_str())
        );

        let again = Ask::new("r1", ord, "toolu-b").record(v).unwrap();
        assert_eq!(
            again.interaction, b.interaction,
            "same run and tool id: the same card"
        );
        assert!(again.superseded.is_empty());
        assert_eq!(
            list_pending(v, other_key).unwrap(),
            vec![untouched.clone()],
            "other sessions are untouched"
        );
        assert_eq!(list_pending(v, KEY).unwrap(), vec![b.interaction.clone()]);
        // Across sessions, newest first, open cards only (next-level P19).
        let all = list_open_all(v, 10).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|c| c.status == "pending"));
        assert!(
            all.iter().any(|c| c.id == b.interaction.id)
                && all.iter().any(|c| c.id == untouched.id)
        );
    }

    #[test]
    fn a_repeated_codex_item_id_in_a_later_run_records_a_new_card() {
        let dir = vault();
        let v = dir.path();
        let first = start(v, KEY, "r1");
        let one = Ask::new("r1", first, "item_0")
            .record(v)
            .unwrap()
            .interaction;
        resolve(v, &one.id, &grid(), Some("native-1")).unwrap();
        let second = start(v, KEY, "r2");
        let two = Ask::new("r2", second, "item_0")
            .record(v)
            .unwrap()
            .interaction;
        assert_ne!(two.id, one.id);
        assert_eq!(two.status, "pending");
        assert_eq!(list_pending(v, KEY).unwrap(), vec![two]);
    }

    #[test]
    fn fixture_turns_record_resolvable_interactions() {
        for (kind, text, native) in [
            (AgentKind::Claude, CLAUDE_ASK, "id-101"),
            (AgentKind::Codex, CODEX_ASK, "id-201"),
        ] {
            let dir = vault();
            let v = dir.path();
            let ord = start(v, KEY, "run-1");
            // A first turn: nothing to resume, so the id must come from the stream.
            let (calls, _) = replay(kind, text, None);
            assert_eq!(calls.len(), 1);
            let call = &calls[0];
            let mut emitted = Vec::new();
            persist_and_notify(
                v,
                &NewInteraction {
                    session_key: KEY,
                    turn_ord: ord,
                    run_id: "run-1",
                    agent: kind,
                    native_session_id: call.native_session_id.as_deref(),
                    tool_use_id: &call.tool_use_id,
                    source: InteractionSource::Mcp,
                    arguments: &call.arguments,
                    cwd: v,
                    vault_root: v,
                },
                &mut |payload| emitted.push(payload),
            );
            assert_eq!(emitted.len(), 1, "{emitted:?}");
            assert_eq!(emitted[0]["type"], json!("interaction"));
            assert_eq!(emitted[0]["sessionKey"], json!(KEY));
            let card: ChatInteraction =
                serde_json::from_value(emitted[0]["interaction"].clone()).unwrap();
            assert_eq!(card.native_session_id.as_deref(), Some(native));
            let resolved = resolve(v, &card.id, &grid(), Some(native)).unwrap();
            assert!(
                resolved
                    .prompt
                    .starts_with("Answered questions\n\nWhich layout?\nGrid"),
                "{}",
                resolved.prompt
            );
            assert_eq!(resolved.interaction.status, "answered");
        }
    }

    #[test]
    fn an_unreadable_request_is_rejected_visibly_and_recorded_nowhere() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        for arguments in [
            Err("the arguments are not a JSON object".to_string()),
            Ok(json!({ "questions": [] })),
        ] {
            let mut ask = Ask::new("r1", ord, "toolu-bad");
            ask.arguments = arguments;
            let mut emitted = Vec::new();
            persist_and_notify(v, &ask.new_interaction(v), &mut |p| emitted.push(p));
            assert_eq!(emitted.len(), 1);
            assert_eq!(emitted[0]["type"], json!("error"));
            let message = emitted[0]["message"].as_str().unwrap();
            assert!(
                message.starts_with("The agent asked questions Hollow could not show:"),
                "{message}"
            );
            assert!(message.ends_with("Reply in the composer instead."));
        }
        assert!(list_pending(v, KEY).unwrap().is_empty());
        let rejected: i64 = session_store::with_read(v, |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM events WHERE session_key = ?1 AND kind = 'interactionRejected'",
                params![KEY],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(rejected, 2, "each rejection is traced");
    }

    #[test]
    fn a_later_user_turn_supersedes_only_flagged_pending_cards() {
        let dir = vault();
        let v = dir.path();
        let first = start(v, KEY, "r1");
        let flagged = Ask::new("r1", first, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        // Recorded in the same session but not superseded by the next one:
        // insert it directly as a second open card that opts out.
        let mut sticky = Ask::new("r1", first, "toolu-b");
        let mut request = layout_request();
        request["supersedeOnUserMessage"] = json!(false);
        sticky.arguments = Ok(request);
        let sticky = sticky.record(v).unwrap().interaction;
        // Recording the sticky card superseded the flagged one already; put
        // the flagged one back so both are open.
        session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE interactions SET status = 'pending' WHERE id = ?1",
                params![flagged.id],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();

        // Same turn: nothing to supersede.
        assert!(supersede_for_message(v, KEY, first).unwrap().is_empty());
        let next = start(v, KEY, "r2");
        let gone = supersede_for_message(v, KEY, next).unwrap();
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].id, flagged.id);
        assert_eq!(gone[0].status_reason.as_deref(), Some("user_message"));
        let open: Vec<String> = list_pending(v, KEY)
            .unwrap()
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(open, vec![sticky.id]);
        assert!(supersede_for_message(v, KEY, 0).unwrap().is_empty());
    }

    #[test]
    fn e1_card_hold_is_thirty_minutes() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        let card = Ask::new("r1", ord, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        let now = Utc::now();
        let hold = card_hold_until(v, KEY, now).unwrap();
        let created = session_store::parse_iso(&card.created_at).unwrap();
        assert_eq!(hold, card_release(created));
        assert_eq!(open_hold_until(v, KEY, now), Some(hold));
        let aged = now - chrono::Duration::minutes(CARD_HOLD_MINUTES + 1);
        session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE interactions SET created_at = ?2 WHERE id = ?1",
                params![card.id, session_store::db_time(aged)],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(card_hold_until(v, KEY, now), None);
        assert_eq!(open_hold_until(v, KEY, now), None);
    }

    #[test]
    fn resolve_formats_the_prompt_once_and_refuses_a_second_time() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        let card = Ask::new("r1", ord, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        let invalid =
            resolve(v, &card.id, &json!({ "answers": {} }), Some("native-1")).unwrap_err();
        assert!(invalid.starts_with(INVALID), "{invalid}");
        let resolved = resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        assert_eq!(
            resolved.prompt,
            "Answered questions\n\nWhich layout?\nGrid\n\nAnything else?\nNo answer"
        );
        assert_eq!(
            resolved.interaction.answer,
            Some(json!({ "answers": { "layout": { "selected": ["grid"] }, "notes": {} } }))
        );
        let again = resolve(v, &card.id, &grid(), Some("native-1")).unwrap_err();
        assert!(again.starts_with(CLOSED), "{again}");
        assert!(resolve(v, "qi-0000000000000000", &grid(), Some("native-1"))
            .unwrap_err()
            .starts_with(CLOSED));
    }

    /// 2026-11-04 (Alex, a Main check-in's card): the conversation had
    /// moved on to a fresh native session, and Send stayed disabled. A card
    /// whose session the conversation no longer holds (another one, none,
    /// or a card that never had one) is answered all the same, `resumes`
    /// false; every answer names each question in full, its own session's
    /// too, since a resume can still become a fresh session.
    #[test]
    fn resolve_answers_as_a_message_when_the_session_moved_on() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        // A short header the card shows, and the whole question.
        let headed = || {
            json!({ "questions": [
                { "id": "layout", "header": "Layout", "prompt": "Which layout fits the launch page?", "options": [
                    { "id": "grid", "label": "Grid", "recommended": true },
                    { "id": "list", "label": "List" }
                ]},
                { "id": "notes", "prompt": "Anything else?", "answerMode": "text", "required": false }
            ]})
        };
        let ask = |tool: &str, native: Option<&str>| {
            let mut ask = Ask::new("r1", ord, tool);
            ask.native = native.map(str::to_string);
            ask.arguments = Ok(headed());
            ask.record(v).unwrap().interaction
        };
        let standalone =
            "Answered questions\n\nWhich layout fits the launch page?\nGrid\n\nAnything else?\nNo answer";
        for (tool, native) in [("toolu-a", None), ("toolu-b", Some("native-2"))] {
            let card = ask(tool, Some("native-1"));
            let resolved = resolve(v, &card.id, &grid(), native).unwrap();
            assert!(!resolved.resumes, "{native:?}");
            assert_eq!(resolved.prompt, standalone);
            assert_eq!(resolved.interaction.status, "answered");
        }
        let card = ask("toolu-c", None);
        let resolved = resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        assert!(
            !resolved.resumes,
            "a card with no native session never resumes"
        );
        assert_eq!(resolved.prompt, standalone);

        let card = ask("toolu-d", Some("native-1"));
        let resolved = resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        assert!(resolved.resumes);
        assert_eq!(
            resolved.prompt, standalone,
            "the same words for its own session"
        );
        assert!(
            list_pending(v, KEY).unwrap().is_empty(),
            "every card answered"
        );
    }

    fn confirmation(path: &str) -> Value {
        json!({ "mode": "confirmation", "confirmation": { "prompt": "Delete it?", "target": { "path": path } } })
    }

    #[test]
    fn a_changed_confirmation_target_resolves_as_stale() {
        let dir = vault();
        let v = dir.path();
        std::fs::create_dir_all(v.join("notes")).unwrap();
        std::fs::write(v.join("notes/a.md"), "one").unwrap();
        let ord = start(v, KEY, "r1");
        let mut ask = Ask::new("r1", ord, "toolu-a");
        ask.arguments = Ok(confirmation("notes/a.md"));
        let card = ask.record(v).unwrap().interaction;
        assert_eq!(card.status, "pending");
        assert_eq!(card.target_path.as_deref(), Some("notes/a.md"));

        std::fs::write(v.join("notes/a.md"), "two").unwrap();
        let refused =
            resolve(v, &card.id, &json!({ "accepted": true }), Some("native-1")).unwrap_err();
        assert!(refused.starts_with(STALE), "{refused}");
        let open = list_pending(v, KEY).unwrap();
        assert_eq!(open[0].status, "stale");
        assert_eq!(open[0].status_reason.as_deref(), Some("target changed"));

        // An unchanged target answers normally.
        std::fs::write(v.join("notes/b.md"), "same").unwrap();
        let mut ask = Ask::new("r1", ord, "toolu-b");
        ask.arguments = Ok(confirmation("notes/b.md"));
        let card = ask.record(v).unwrap().interaction;
        let resolved =
            resolve(v, &card.id, &json!({ "accepted": true }), Some("native-1")).unwrap();
        assert!(resolved.prompt.contains("Delete it?\nAccepted"));
    }

    #[test]
    fn an_unresolvable_or_special_target_records_stale_without_blocking() {
        let parent = tempfile::Builder::new().prefix("outer-").tempdir().unwrap();
        let v = parent.path().join("vault");
        std::fs::create_dir_all(v.join("dir")).unwrap();
        std::fs::write(parent.path().join("outside.md"), "x").unwrap();
        let ord = start(&v, KEY, "r1");
        let mut targets = vec![
            "../outside.md".to_string(),
            parent
                .path()
                .join("outside.md")
                .to_string_lossy()
                .into_owned(),
            "missing.md".to_string(),
            "dir".to_string(),
        ];
        #[cfg(unix)]
        {
            let fifo = v.join("pipe");
            let c_path = std::ffi::CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
            // SAFETY: a valid NUL-terminated path; mkfifo does not retain it.
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
            targets.push("pipe".to_string());
        }
        for (n, target) in targets.iter().enumerate() {
            let mut ask = Ask::new("r1", ord, &format!("toolu-{n}"));
            ask.arguments = Ok(confirmation(target));
            let started = std::time::Instant::now();
            let card = ask.record(&v).unwrap().interaction;
            assert!(
                started.elapsed() < std::time::Duration::from_secs(5),
                "{target} blocked"
            );
            assert_eq!(card.status, "stale", "{target}");
            assert_eq!(
                card.status_reason.as_deref(),
                Some("target not found"),
                "{target}"
            );
        }
    }

    #[test]
    fn archive_expires_pending_atomically_and_unarchive_does_not_revive() {
        let dir = vault();
        let v = dir.path();
        let first = start(v, KEY, "r1");
        let answered = Ask::new("r1", first, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        resolve(v, &answered.id, &grid(), Some("native-1")).unwrap();
        let open = Ask::new("r1", first, "toolu-b")
            .record(v)
            .unwrap()
            .interaction;

        let unknown = session_store::archive_session(v, "s-20260926-ffff0009", true).unwrap_err();
        assert!(unknown.contains("no session"), "{unknown}");
        assert_eq!(
            list_pending(v, KEY).unwrap().len(),
            1,
            "an unknown key expires nothing"
        );

        session_store::archive_session(v, KEY, true).unwrap();
        assert!(list_pending(v, KEY).unwrap().is_empty());
        let expired = session_store::with_read(v, |conn| get_in(conn, &open.id))
            .unwrap()
            .unwrap();
        assert_eq!(expired.status, "expired");
        assert_eq!(expired.status_reason.as_deref(), Some("archived"));
        let kept = session_store::with_read(v, |conn| get_in(conn, &answered.id))
            .unwrap()
            .unwrap();
        assert_eq!(kept.status, "answered", "answered cards are left alone");

        session_store::archive_session(v, KEY, false).unwrap();
        assert!(
            list_pending(v, KEY).unwrap().is_empty(),
            "unarchiving revives nothing"
        );
    }

    #[test]
    fn reopen_only_before_the_next_turn_starts() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        let card = Ask::new("r1", ord, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        assert!(
            reopen(v, &card.id).unwrap_err().starts_with(CLOSED),
            "a pending card has nothing to reopen"
        );
        resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        let back = reopen(v, &card.id).unwrap();
        assert_eq!(back.status, "pending");
        assert_eq!(back.answer, None);

        resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        start(v, KEY, "r2");
        assert!(reopen(v, &card.id).unwrap_err().starts_with(CLOSED));

        let other = Ask::new("r2", 2, "toolu-b").record(v).unwrap().interaction;
        let dismissed = dismiss(v, &other.id).unwrap();
        assert_eq!(dismissed.status, "dismissed");
        assert!(dismiss(v, &other.id).unwrap_err().starts_with(CLOSED));
        assert!(list_pending(v, KEY).unwrap().is_empty());
    }

    #[test]
    fn reopen_unstarted_restores_an_answer_whose_turn_failed_to_spawn() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        assert_eq!(ord, 1);
        let card = Ask::new("r1", ord, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        resolve(v, &card.id, &grid(), Some("native-1")).unwrap();
        let answer_turn = start(v, KEY, "r2");
        assert_eq!(answer_turn, 2);
        assert!(reopen(v, &card.id).unwrap_err().starts_with(CLOSED));
        assert!(
            reopen_unstarted(v, KEY, 3).unwrap().is_empty(),
            "a later turn touches nothing"
        );
        let back = reopen_unstarted(v, KEY, answer_turn).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, card.id);
        assert_eq!(back[0].status, "pending");
        assert_eq!(back[0].answer, None);
        assert!(reopen_unstarted(v, KEY, 0).unwrap().is_empty());
    }

    #[test]
    fn retention_prunes_old_terminal_rows_never_pending() {
        let dir = vault();
        let v = dir.path();
        let ord = start(v, KEY, "r1");
        let old = Ask::new("r1", ord, "toolu-a")
            .record(v)
            .unwrap()
            .interaction;
        dismiss(v, &old.id).unwrap();
        let open = Ask::new("r1", ord, "toolu-b")
            .record(v)
            .unwrap()
            .interaction;
        let ancient = db_time(Utc::now() - chrono::Duration::days(KEEP_DAYS + 1));
        session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE interactions SET resolved_at = ?2, created_at = ?2 WHERE id IN (?1, ?3)",
                params![old.id, ancient, open.id],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        session_store::retain(v).unwrap();
        let (old_row, open_row) = session_store::with_read(v, |conn| {
            Ok((get_in(conn, &old.id)?, get_in(conn, &open.id)?))
        })
        .unwrap();
        assert!(
            old_row.is_none(),
            "a terminal row resolved long ago is pruned"
        );
        assert_eq!(
            open_row.unwrap().status,
            "pending",
            "an open row is never pruned"
        );
    }
}
