//! Pause a running chat (plan row P1a, Linear PER-15; PR 1 of 2, backend).
//!
//! AppHandle-free, so tests drive it. Three pieces:
//!
//! - **The request registry.** [`request_pause`] marks a conversation whose
//!   turn is running as pausing; `chat::request_pause_for` calls it under the
//!   chat stop-slot lock, so only a registered running turn can be asked, and
//!   the turn's stop registration clears the request when it is dropped (the
//!   turn ended, paused or not), so a stale request never reaches a later
//!   turn. The turn reads [`pause_requested`] at each safe point
//!   (`chat_resume::SafePoints`: its next main model request with no tool call
//!   open), never inside a tool call. A turn that finishes before any safe
//!   point just finishes: nothing is paused.
//! - **The paused record.** A turn that paused at a safe point keeps its
//!   native session and is recorded here ([`record`]), in the session store's
//!   `chat_pauses` table ([`SCHEMA`]), so a paused chat survives a restart.
//!   Each record names the native session, the agent, when, a one-line
//!   [`note`] of what the chat was doing, why ([`PauseReason`]: `user`,
//!   `all` from Pause all (P1b), `usage_limit` when every account ran out
//!   (P1d)) and `resume_at`, when it resumes on its own (P1c). Any turn the
//!   conversation starts clears it ([`on_turn_start`]).
//! - **Pause all (P1b).** [`begin_pause_all`] notes which chats a Pause all
//!   asked; each settles once, paused with its note or ended first, and the
//!   last one posts one summary notification ([`summary_notification`]).
//! - **Resume at a time (P1c).** [`set_resume_at`] sets or clears the time;
//!   the scheduler tick resumes [`due`] chats through the same claim as the
//!   Resume button, so a chat never resumes twice.
//! - **Pause at a usage limit (P1d).** [`limit_pause`] decides whether a
//!   turn that ran out pauses instead of failing, with its note and
//!   `resume_at` (the provider's reset plus [`RESET_MARGIN_SECS`]).
//! - **Resume.** [`resume_input`] builds the ordinary queued send that
//!   continues the same native session with [`RESUME_PROMPT`], as the
//!   composer's Continue after a Stop does, refusing a chat that is not
//!   paused or already has a turn.

use crate::session_store;
use once_cell::sync::Lazy;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The whole prompt of the turn that resumes a paused chat.
pub(crate) const RESUME_PROMPT: &str = "Continue where you left off; the chat was paused.";

/// The paused note's length cap, in characters.
const NOTE_CHARS: usize = 160;

/// The note when the turn left no text and used no tool.
const NOTE_FALLBACK: &str = "Paused between steps.";

// ── The request registry ───────────────────────────────────────────────────

/// Conversations whose running turn was asked to pause, by session key, and
/// why (the first ask's reason holds: a Pause all after the person's own
/// Pause keeps `user`).
static REQUESTS: Lazy<Mutex<HashMap<String, PauseReason>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Ask `key`'s running turn to pause at its next safe point. `running` is
/// whether the conversation has a registered running turn (the caller checks
/// it under its own lock); false asks nothing and returns false.
pub(crate) fn request_pause(key: &str, running: bool) -> bool {
    request_pause_as(key, running, PauseReason::User)
}

/// [`request_pause`] for `reason` (P1b: Pause all asks as `all`).
pub(crate) fn request_pause_as(key: &str, running: bool, reason: PauseReason) -> bool {
    if !running {
        return false;
    }
    REQUESTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(key.to_string())
        .or_insert(reason);
    true
}

/// Why `key`'s running turn was asked to pause, if it was.
pub(crate) fn requested_reason(key: &str) -> Option<PauseReason> {
    REQUESTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .copied()
}

/// Whether `key`'s running turn was asked to pause.
pub(crate) fn pause_requested(key: &str) -> bool {
    REQUESTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(key)
}

/// Forget `key`'s request (its turn ended, or a new one registered).
pub(crate) fn clear_request(key: &str) {
    REQUESTS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(key);
}

// ── The paused record ──────────────────────────────────────────────────────

/// Why a chat is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PauseReason {
    /// The person (or an agent through `sessions.pause`) paused it.
    User,
    /// Every account of its agent ran out of usage (P1d).
    UsageLimit,
    /// Pause all (P1b).
    All,
}

impl PauseReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PauseReason::User => "user",
            PauseReason::UsageLimit => "usage_limit",
            PauseReason::All => "all",
        }
    }

    fn parse(text: &str) -> PauseReason {
        match text {
            "usage_limit" => PauseReason::UsageLimit,
            "all" => PauseReason::All,
            _ => PauseReason::User,
        }
    }
}

/// One paused chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PausedChat {
    pub session_key: String,
    /// The native session the resume continues.
    pub native_session_id: String,
    /// The agent's wire name (`claude`, `codex`, ...).
    pub agent: String,
    /// Unix seconds.
    pub paused_at: i64,
    /// What the chat was doing, one line ([`note`]).
    pub note: String,
    pub reason: PauseReason,
    /// When to resume on its own (unix seconds, P1c); `None` waits for a
    /// Resume.
    pub resume_at: Option<i64>,
    /// Review fix: how many automatic resumes (scheduled or at a limit's
    /// reset) were tried in a row without the chat getting past its first
    /// model request ([`MAX_AUTO_ATTEMPTS`] caps them).
    #[serde(default)]
    pub auto_attempts: i64,
}

/// The `chat_pauses` table, created with the session store's schema.
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS chat_pauses( \
   session_key TEXT PRIMARY KEY, \
   native_session_id TEXT NOT NULL, \
   agent TEXT NOT NULL, \
   paused_at INTEGER NOT NULL, \
   note TEXT NOT NULL, \
   reason TEXT NOT NULL, \
   resume_at INTEGER, \
   claimed_at INTEGER, \
   claim_token TEXT, \
   auto_attempts INTEGER NOT NULL DEFAULT 0 \
 )";

const COLUMNS: &str =
    "session_key, native_session_id, agent, paused_at, note, reason, resume_at, auto_attempts";

fn row_to_chat(row: &rusqlite::Row<'_>) -> rusqlite::Result<PausedChat> {
    Ok(PausedChat {
        session_key: row.get(0)?,
        native_session_id: row.get(1)?,
        agent: row.get(2)?,
        paused_at: row.get(3)?,
        note: row.get(4)?,
        reason: PauseReason::parse(&row.get::<_, String>(5)?),
        resume_at: row.get(6)?,
        auto_attempts: row.get::<_, Option<i64>>(7)?.unwrap_or(0),
    })
}

fn safe_key(key: &str) -> Result<(), String> {
    if crate::agent_sessions::is_safe_session_key(key) {
        Ok(())
    } else {
        Err(format!("unsafe session key: {key}"))
    }
}

/// Record `chat` as paused, replacing any earlier record of the chat.
pub(crate) fn record(vault: &Path, chat: &PausedChat) -> Result<(), String> {
    safe_key(&chat.session_key)?;
    session_store::with_write(vault, "chat pause", |tx| {
        tx.execute(
            &format!(
                "INSERT OR REPLACE INTO chat_pauses({COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
            ),
            params![
                chat.session_key,
                chat.native_session_id,
                chat.agent,
                chat.paused_at,
                chat.note,
                chat.reason.as_str(),
                chat.resume_at,
                chat.auto_attempts,
            ],
        )
        .map(|_| ())
        .map_err(|e| format!("chat pause: {e}"))
    })
}

/// `key`'s paused record, if it is paused.
pub(crate) fn get(vault: &Path, key: &str) -> Result<Option<PausedChat>, String> {
    safe_key(key)?;
    session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!("SELECT {COLUMNS} FROM chat_pauses WHERE session_key = ?1"),
            params![key],
            row_to_chat,
        )
        .optional()
        .map_err(|e| format!("chat pause: {e}"))
    })
}

/// Every paused chat, newest first.
pub(crate) fn list(vault: &Path) -> Result<Vec<PausedChat>, String> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM chat_pauses ORDER BY paused_at DESC, session_key"
            ))
            .map_err(|e| format!("paused chats: {e}"))?;
        let rows = stmt
            .query_map([], row_to_chat)
            .map_err(|e| format!("paused chats: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("paused chats: {e}"))
    })
}

/// Forget `key`'s paused record. True when there was one.
pub(crate) fn clear(vault: &Path, key: &str) -> Result<bool, String> {
    safe_key(key)?;
    session_store::with_write(vault, "chat unpause", |tx| {
        tx.execute(
            "DELETE FROM chat_pauses WHERE session_key = ?1",
            params![key],
        )
        .map(|n| n > 0)
        .map_err(|e| format!("chat unpause: {e}"))
    })
}

/// A turn of `key` paused at a safe point (`chat::execute_turn`): record it,
/// on the native session its stream named (`native`; a turn pauses only once
/// it named one, so `None` records nothing). Logged, never fatal. The live
/// paths call [`record_turn_pause_as`] with the request's reason.
#[cfg(test)]
pub(crate) fn record_turn_pause(
    vault: &Path,
    key: &str,
    agent: &str,
    native: Option<&str>,
    note: &str,
    now: i64,
) -> bool {
    record_turn_pause_as(
        vault,
        key,
        agent,
        native,
        note,
        now,
        PauseReason::User,
        None,
        0,
    )
}

/// [`record_turn_pause`] with its reason and resume time (P1b, P1d).
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_turn_pause_as(
    vault: &Path,
    key: &str,
    agent: &str,
    native: Option<&str>,
    note: &str,
    now: i64,
    reason: PauseReason,
    resume_at: Option<i64>,
    auto_attempts: i64,
) -> bool {
    let Some(native) = native.filter(|id| !id.trim().is_empty()) else {
        return false;
    };
    let chat = PausedChat {
        session_key: key.to_string(),
        native_session_id: native.to_string(),
        agent: agent.to_string(),
        paused_at: now,
        note: note.to_string(),
        reason,
        resume_at,
        auto_attempts,
    };
    match record(vault, &chat) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("[chat pause] {key}: {error}");
            false
        }
    }
}

/// A turn of `key` spawned its agent: the chat is no longer paused, whatever
/// started it (the resume, or the person's own message). A turn that never
/// spawned leaves the record (review fix: it used to clear at admission, so
/// a resume that failed or was stopped before its spawn lost the pause).
/// Not keyed by the claim's token on purpose: a resume's own turn holds it
/// (checked at admission, one turn per chat at a time), and any other turn
/// that spawns supersedes the pause, so a still-queued resume then ends as a
/// no-op ([`claim_held`]). Logged, never fatal.
pub(crate) fn on_turn_start(vault: &Path, key: &str) {
    if let Err(error) = clear(vault, key) {
        eprintln!("[chat pause] {key}: {error}");
    }
}

/// The one-line note of what a pausing turn was doing: the last line of its
/// reply so far, else the last tool it used, else [`NOTE_FALLBACK`]. Kept to
/// one line of plain words (markdown markers and dashes taken out) and
/// [`NOTE_CHARS`] characters.
pub(crate) fn note(assistant: &str, last_tool: Option<&str>) -> String {
    let line = assistant
        .lines()
        .map(|line| line.trim().trim_start_matches(['#', '*', '-', '>', ' ']))
        .rfind(|line| !line.trim().is_empty() && !line.starts_with("```"))
        .map(str::to_string)
        .or_else(|| {
            last_tool
                .filter(|tool| !tool.trim().is_empty())
                .map(|tool| format!("Was using {}.", tool.trim()))
        })
        .unwrap_or_else(|| NOTE_FALLBACK.to_string());
    let plain: String = line
        .replace(['\u{2014}', '\u{2013}'], ", ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if plain.chars().count() <= NOTE_CHARS {
        return plain;
    }
    let cut: String = plain.chars().take(NOTE_CHARS - 3).collect();
    format!("{}...", cut.trim_end())
}

/// A paused chat as the live list shows it: phase `paused`, no run id.
pub(crate) fn live_session(chat: &PausedChat) -> crate::types::LiveAgentSession {
    let at = chrono::DateTime::from_timestamp(chat.paused_at, 0).unwrap_or_default();
    crate::types::LiveAgentSession {
        key: chat.session_key.clone(),
        agent: chat.agent.clone(),
        run_id: String::new(),
        phase: crate::agent_sessions::SessionPhase::Paused
            .as_str()
            .to_string(),
        phase_entered_at: at.to_rfc3339(),
        failure_reason: None,
        last_event_at: None,
        progress: None,
    }
}

/// Every paused chat as [`live_session`] rows; empty when the store can't be
/// read (logged).
pub(crate) fn live_sessions(vault: &Path) -> Vec<crate::types::LiveAgentSession> {
    match list(vault) {
        Ok(chats) => chats.iter().map(live_session).collect(),
        Err(error) => {
            eprintln!("[chat pause] paused chats: {error}");
            Vec::new()
        }
    }
}

// ── Resume ─────────────────────────────────────────────────────────────────

/// How long a resume's claim on a paused chat holds against another resume
/// (seconds). A claim is released when its send fails or its turn ends
/// without spawning; one Hollow lost (a quit, a refused admission) lapses
/// after this, and the chat can be resumed again.
const CLAIM_LAPSES_SECS: i64 = 120;

/// The resume's claim token prefix: a resume's send carries
/// `<prefix><16 hex>` as its `clientTurnId`, which names the claim's owner
/// (delta review: a claim must have an owner). Only Hollow mints it.
pub(crate) const RESUME_TOKEN_PREFIX: &str = "pause-resume-";

/// A fresh claim token ([`RESUME_TOKEN_PREFIX`]).
pub(crate) fn new_resume_token() -> String {
    use rand::Rng;
    let n: u64 = rand::thread_rng().gen();
    format!("{RESUME_TOKEN_PREFIX}{n:016x}")
}

/// Review fix: an automatic resume's token (the scheduler's, P1c, or a limit
/// pause's reset, P1d) names its attempt: `pause-resume-auto-<n>-<hex>`.
const AUTO_TOKEN_PREFIX: &str = "pause-resume-auto-";

/// At most this many automatic resumes in a row that never got past their
/// first model request; then the time is cleared, the chat stays paused and
/// a notification says so.
pub(crate) const MAX_AUTO_ATTEMPTS: i64 = 3;

/// A fresh token for automatic attempt `n` (from 1).
pub(crate) fn new_auto_token(n: i64) -> String {
    use rand::Rng;
    let hex: u64 = rand::thread_rng().gen();
    format!("{AUTO_TOKEN_PREFIX}{n}-{hex:016x}")
}

/// The attempt an automatic resume's token names, `None` for any other.
pub(crate) fn auto_attempt(token: &str) -> Option<i64> {
    let rest = token.strip_prefix(AUTO_TOKEN_PREFIX)?;
    rest.split_once('-')?.0.parse().ok()
}

/// Whether a turn's `clientTurnId` is a resume's claim token.
pub(crate) fn is_resume_token(client_turn_id: Option<&str>) -> bool {
    client_turn_id.is_some_and(|id| id.starts_with(RESUME_TOKEN_PREFIX))
}

/// Claim `key`'s paused record for one resume, owned by `token` (review
/// fix: two resumes at once queued two turns). One `BEGIN IMMEDIATE`
/// transaction marks the row `claimed_at` and `claim_token` only while no
/// live claim holds it, so exactly one caller wins; the row stays until a
/// turn of the chat spawns ([`on_turn_start`]).
pub(crate) fn claim_resume(
    vault: &Path,
    key: &str,
    now: i64,
    token: &str,
) -> Result<PausedChat, String> {
    safe_key(key)?;
    session_store::with_write(vault, "chat resume claim", |tx| {
        let found = tx
            .query_row(
                &format!("SELECT {COLUMNS}, claimed_at FROM chat_pauses WHERE session_key = ?1"),
                params![key],
                |row| Ok((row_to_chat(row)?, row.get::<_, Option<i64>>(8)?)),
            )
            .optional()
            .map_err(|e| format!("chat resume claim: {e}"))?;
        let Some((chat, claimed)) = found else {
            return Err("This chat isn't paused. Nothing started.".into());
        };
        if claimed.is_some_and(|at| now - at < CLAIM_LAPSES_SECS) {
            return Err("This chat is already being resumed. Nothing started.".into());
        }
        tx.execute(
            "UPDATE chat_pauses SET claimed_at = ?2, claim_token = ?3 WHERE session_key = ?1",
            params![key, now, token],
        )
        .map_err(|e| format!("chat resume claim: {e}"))?;
        // Review fix: an automatic resume's claim counts its attempt.
        if let Some(n) = auto_attempt(token) {
            tx.execute(
                "UPDATE chat_pauses SET auto_attempts = ?2 WHERE session_key = ?1",
                params![key, n],
            )
            .map_err(|e| format!("chat resume claim: {e}"))?;
        }
        Ok(chat)
    })
}

/// Whether `key` is still paused under `token`'s claim. A queued resume
/// checks this at admission (`chat_queue::admit_checked_turn`): when another
/// turn of the chat started meanwhile (the record is gone) or its claim
/// lapsed and was taken, it ends as a cancelled queued row instead of
/// running [`RESUME_PROMPT`] as a second turn.
pub(crate) fn claim_held(vault: &Path, key: &str, token: &str) -> bool {
    if safe_key(key).is_err() {
        return false;
    }
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT 1 FROM chat_pauses WHERE session_key = ?1 AND claim_token = ?2",
            params![key, token],
            |_| Ok(()),
        )
        .optional()
        .map(|found| found.is_some())
        .map_err(|e| format!("chat resume claim: {e}"))
    })
    .unwrap_or(false)
}

/// Give back the claim `token` holds on `key`: its resume's send failed, or
/// its queued row or turn ended without spawning, so the chat stays paused
/// and can be resumed again. Only the owner's token releases it, so another
/// turn's end (a normal message that failed before its spawn) never frees a
/// resume's claim. `None` or a token that isn't a resume's releases nothing.
/// Logged, never fatal.
pub(crate) fn release_claim(vault: &Path, key: &str, token: Option<&str>) {
    release_claim_as(vault, key, token, true);
}

/// [`release_claim`]; `warn` is false when the person ended it (a Stop or a
/// cancel from the queue, delta review): the time still goes, quietly.
pub(crate) fn release_claim_as(vault: &Path, key: &str, token: Option<&str>, warn: bool) {
    let Some(token) = token.filter(|t| is_resume_token(Some(t))) else {
        return;
    };
    if safe_key(key).is_err() {
        return;
    }
    // Review fix: an automatic resume released here (withdrawn, refused at
    // admission, failed or stopped before its spawn) also drops the time,
    // so a past time never re-fires every minute; the chat stays paused and
    // a notification says so.
    let auto = auto_attempt(token).is_some();
    let released = session_store::with_write(vault, "chat resume release", |tx| {
        let sql = if auto {
            "UPDATE chat_pauses SET claimed_at = NULL, claim_token = NULL, resume_at = NULL \
             WHERE session_key = ?1 AND claim_token = ?2"
        } else {
            "UPDATE chat_pauses SET claimed_at = NULL, claim_token = NULL \
             WHERE session_key = ?1 AND claim_token = ?2"
        };
        tx.execute(sql, params![key, token])
            .map_err(|e| format!("chat resume release: {e}"))
    });
    match released {
        Ok(n) if n > 0 && auto && warn => notify_stays_paused(
            vault,
            key,
            "its resume on its own did not start, so its resume time is cleared",
        ),
        Ok(_) => {}
        Err(error) => eprintln!("[chat pause] {key}: {error}"),
    }
}

/// Review fix: an automatic resume whose send failed (a transient error;
/// [`definitive_refusal`] is not): its claim goes and the time stays for the
/// next tick, until [`MAX_AUTO_ATTEMPTS`]; then the time goes too, with a
/// notification. True when the time was dropped.
pub(crate) fn release_for_retry(vault: &Path, key: &str, token: &str) -> bool {
    let give_up = auto_attempt(token).is_some_and(|n| n >= MAX_AUTO_ATTEMPTS);
    let released = session_store::with_write(vault, "chat resume retry", |tx| {
        tx.execute(
            "UPDATE chat_pauses SET claimed_at = NULL, claim_token = NULL, \
             resume_at = CASE WHEN ?3 THEN NULL ELSE resume_at END \
             WHERE session_key = ?1 AND claim_token = ?2",
            params![key, token, give_up],
        )
        .map_err(|e| format!("chat resume retry: {e}"))
    });
    match released {
        Ok(n) if n > 0 && give_up => {
            notify_gave_up(vault, key);
            true
        }
        Ok(_) => false,
        Err(error) => {
            eprintln!("[chat pause] {key}: {error}");
            false
        }
    }
}

/// The chat's title for a notification, else "A paused chat".
fn title_of(vault: &Path, key: &str) -> String {
    session_store::session_by_key(vault, key)
        .ok()
        .flatten()
        .and_then(|s| s.title)
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "A paused chat".to_string())
}

/// A notification about a paused chat, opening Running. A no-op without
/// the GUI (tests, the CLI).
fn notify(vault: &Path, title: String, body: String, kind: &str) {
    let input = crate::notifications::NotificationInput {
        title,
        body,
        kind: kind.into(),
        source: Some("chat-pause".into()),
        action_label: Some("Running".into()),
        action_url: Some(MISSION_CONTROL_ACTION.into()),
        notify_native: false,
    };
    if let Some(Err(error)) = crate::notifications::create_registered(vault, input) {
        eprintln!("[chat pause] notification: {error}");
    }
}

/// The text of [`notify_stays_paused`]: title and body.
pub(crate) fn stays_paused_text(title: &str, why: &str) -> (String, String) {
    (
        format!("“{title}” stays paused"),
        format!("“{title}” is still paused: {why}. Resume it from Running."),
    )
}

/// The chat stays paused because `why`; never silent.
pub(crate) fn notify_stays_paused(vault: &Path, key: &str, why: &str) {
    let (title, body) = stays_paused_text(&title_of(vault, key), why);
    notify(vault, title, body, "warning");
}

/// Why the automatic resumes stopped (`MAX_AUTO_ATTEMPTS`).
pub(crate) fn gave_up_why() -> String {
    format!(
        "Hollow tried to resume it on its own {MAX_AUTO_ATTEMPTS} times without it getting going, and cleared its resume time"
    )
}

/// Automatic resumes stopped after [`MAX_AUTO_ATTEMPTS`].
pub(crate) fn notify_gave_up(vault: &Path, key: &str) {
    notify_stays_paused(vault, key, &gave_up_why());
}

/// An automatic resume started (P1c's time, or P1d's reset).
pub(crate) fn notify_resumed(vault: &Path, chat: &PausedChat) {
    let title = title_of(vault, &chat.session_key);
    let why = match chat.reason {
        PauseReason::UsageLimit => "its usage limit reset",
        _ => "its resume time came",
    };
    notify(
        vault,
        format!("Resumed “{title}”"),
        format!("“{title}” continues where it left off: {why}."),
        "info",
    );
}

/// A chat queue row that ended without its turn spawning (cancelled from
/// the queue, refused at admission, skipped or failed before start): give
/// back the claim its resume took, if it was one. Every such end builds its
/// events through `chat::queued_end_events`; its callers call this beside it.
pub(crate) fn release_for_row(vault: &Path, row: &crate::run_queue::QueueRow, warn: bool) {
    if row.source != crate::run_queue::RunSource::Chat {
        return;
    }
    let token = row
        .payload
        .get("clientTurnId")
        .and_then(serde_json::Value::as_str);
    if let Some(key) = row.source_id.as_deref() {
        release_claim_as(vault, key, token, warn);
    }
}

/// The send that resumes paused chat `key`: [`RESUME_PROMPT`] on its stored
/// native session, with the conversation's own agent, folder, last model and
/// last permission mode, at depth 0 (as the composer's Continue). Refused
/// when `busy` (a turn of it runs or waits in the queue), it is not paused,
/// or another resume holds its claim ([`claim_resume`], taken last, so a
/// refusal here never leaves a claim behind). The caller releases the claim
/// ([`release_claim`]) if its send then fails.
pub(crate) fn resume_input(
    vault: &Path,
    key: &str,
    busy: bool,
    now: i64,
    token: &str,
) -> Result<crate::commands::ChatSendInput, String> {
    safe_key(key)?;
    if busy {
        return Err("This chat already has a turn running or queued. Nothing started.".into());
    }
    if get(vault, key)?.is_none() {
        return Err("This chat isn't paused. Nothing started.".into());
    }
    let summary = session_store::session_by_key(vault, key)?
        .ok_or_else(|| "This chat no longer exists. Nothing started.".to_string())?;
    let model = session_store::last_requested_model(vault, key)?.and_then(|(model, _)| model);
    let permission_mode = session_store::last_permission(vault, key)?;
    let paused = claim_resume(vault, key, now, token)?;
    Ok(crate::commands::ChatSendInput {
        prompt: RESUME_PROMPT.to_string(),
        session_id: Some(paused.native_session_id),
        session_key: Some(key.to_string()),
        cwd: Some(summary.cwd),
        model,
        permission_mode,
        agent: Some(paused.agent),
        // The claim's owner, echoed on the turn's events like any nonce.
        client_turn_id: Some(token.to_string()),
        ..Default::default()
    })
}

// ── Pause all (P1b) ────────────────────────────────────────────────────────

/// How one chat of a Pause all settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Settled {
    /// It paused at a safe point; what it was doing.
    Paused(String),
    /// Its turn ended first (finished, failed or stopped): not paused.
    Ended,
}

/// The chats one Pause all asked that haven't settled, and how the others
/// did. A second Pause all while one is open joins it, so one summary covers
/// both. Lost with the process: the per-chat records still say everything.
struct PauseAllBatch {
    vault: PathBuf,
    pending: BTreeSet<String>,
    settled: Vec<(String, Settled)>,
}

/// Process-global: one open Pause all at a time in this Hollow process (the
/// queue owner runs every chat turn), joined by a second Pause all while it
/// is open; another process, or a restart, never sees it.
static BATCH: Lazy<Mutex<Option<PauseAllBatch>>> = Lazy::new(|| Mutex::new(None));

/// A finished Pause all: every chat it asked, how each settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PauseAllSummary {
    pub vault: PathBuf,
    pub settled: Vec<(String, Settled)>,
}

/// Note that a Pause all asked `keys` (the running chats
/// `chat::request_pause_all` asked as `all`). Nothing when `keys` is empty.
pub(crate) fn begin_pause_all(vault: &Path, keys: &[String]) {
    if keys.is_empty() {
        return;
    }
    let mut batch = BATCH.lock().unwrap_or_else(PoisonError::into_inner);
    let open = batch.get_or_insert_with(|| PauseAllBatch {
        vault: vault.to_path_buf(),
        pending: BTreeSet::new(),
        settled: Vec::new(),
    });
    open.pending.extend(keys.iter().cloned());
}

/// `key` settled (`chat::settle_pause` when it paused or a Stop won, its
/// stop registration's drop when its turn ended): when it was the open
/// batch's last chat, the batch closes and its summary is returned. A key
/// the batch isn't waiting on changes nothing, so the drop after a settled
/// pause is a no-op.
pub(crate) fn settle_in_batch(key: &str, how: Settled) -> Option<PauseAllSummary> {
    let mut batch = BATCH.lock().unwrap_or_else(PoisonError::into_inner);
    let open = batch.as_mut()?;
    if !open.pending.remove(key) {
        return None;
    }
    open.settled.push((key.to_string(), how));
    if !open.pending.is_empty() {
        return None;
    }
    batch.take().map(|done| PauseAllSummary {
        vault: done.vault,
        settled: done.settled,
    })
}

/// [`settle_in_batch`], posting the summary when the batch closed. Logged,
/// never fatal; a no-op without the GUI (tests, the CLI).
pub(crate) fn settle_and_report(key: &str, how: Settled) {
    if let Some(summary) = settle_in_batch(key, how) {
        let input = summary_notification(&summary);
        if let Some(Err(error)) = crate::notifications::create_registered(&summary.vault, input) {
            eprintln!("[chat pause] pause all summary: {error}");
        }
    }
}

/// Tests: forget any open batch.
#[cfg(test)]
pub(crate) fn reset_batch() {
    *BATCH.lock().unwrap_or_else(PoisonError::into_inner) = None;
}

/// The notification a finished Pause all leaves in Hollow: how many paused,
/// then one line per chat (its title, and what it was doing or that it
/// finished first). Opens Running, where Resume all is.
pub(crate) fn summary_notification(
    summary: &PauseAllSummary,
) -> crate::notifications::NotificationInput {
    let title_of = |key: &str| {
        session_store::session_by_key(&summary.vault, key)
            .ok()
            .flatten()
            .and_then(|s| s.title)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "A chat".to_string())
    };
    let paused = summary
        .settled
        .iter()
        .filter(|(_, how)| matches!(how, Settled::Paused(_)))
        .count();
    let ended = summary.settled.len() - paused;
    let chats = |n: usize| if n == 1 { "chat" } else { "chats" };
    let title = match (paused, ended) {
        (p, 0) => format!("Paused {p} {}", chats(p)),
        (0, e) => format!("Pause all: {e} {} finished first", chats(e)),
        (p, e) => format!("Paused {p} {}; {e} finished first", chats(p)),
    };
    let body = summary
        .settled
        .iter()
        .map(|(key, how)| match how {
            Settled::Paused(note) => format!("{}: {}", title_of(key), note),
            Settled::Ended => format!(
                "{}: finished before it reached a safe point.",
                title_of(key)
            ),
        })
        .collect::<Vec<_>>()
        .join("\n");
    crate::notifications::NotificationInput {
        title,
        body,
        kind: "info".into(),
        source: Some("chat-pause".into()),
        action_label: Some("Running".into()),
        action_url: Some(MISSION_CONTROL_ACTION.into()),
        notify_native: false,
    }
}

/// The notification action that opens Running (Mission Control).
pub(crate) const MISSION_CONTROL_ACTION: &str = "mission-control:";

// ── Resume at a chosen time (P1c) ──────────────────────────────────────────

/// Set or clear (`None`) when paused chat `key` resumes on its own. Returns
/// the record, or `None` when the chat isn't paused (nothing is written).
pub(crate) fn set_resume_at(
    vault: &Path,
    key: &str,
    at: Option<i64>,
) -> Result<Option<PausedChat>, String> {
    safe_key(key)?;
    let changed = session_store::with_write(vault, "chat resume time", |tx| {
        tx.execute(
            // Delta review: a new time starts the automatic attempts again.
            "UPDATE chat_pauses SET resume_at = ?2, \
             auto_attempts = CASE WHEN ?2 IS NULL THEN auto_attempts ELSE 0 END \
             WHERE session_key = ?1",
            params![key, at],
        )
        .map_err(|e| format!("chat resume time: {e}"))
    })?;
    if changed == 0 {
        return Ok(None);
    }
    get(vault, key)
}

/// Paused chats whose time has come (`resume_at <= now`) and that no live
/// resume claim holds, earliest time first. The scheduler tick resumes each
/// through [`resume_input`]'s claim, so one already being resumed (by the
/// person, or an earlier tick) is skipped, never resumed twice.
pub(crate) fn due(vault: &Path, now: i64) -> Result<Vec<PausedChat>, String> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {COLUMNS} FROM chat_pauses \
                 WHERE resume_at IS NOT NULL AND resume_at <= ?1 \
                   AND (claimed_at IS NULL OR ?1 - claimed_at >= ?2) \
                 ORDER BY resume_at, session_key"
            ))
            .map_err(|e| format!("due paused chats: {e}"))?;
        let rows = stmt
            .query_map(params![now, CLAIM_LAPSES_SECS], row_to_chat)
            .map_err(|e| format!("due paused chats: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("due paused chats: {e}"))
    })
}

/// Review fix: whether a scheduled resume's refusal is definitive (the chat
/// isn't paused any more, or is gone), so its time is dropped at once. Any
/// other error is transient and retries under [`MAX_AUTO_ATTEMPTS`].
pub(crate) fn definitive_refusal(error: &str) -> bool {
    error.contains("isn't paused") || error.contains("no longer exists")
}

// ── Pause at a usage limit (P1d) ───────────────────────────────────────────

/// Seconds after the provider's reset time a limit-paused chat resumes, so
/// its first request lands after the window really reopened.
pub(crate) const RESET_MARGIN_SECS: i64 = 60;

/// What `chat::execute_turn` knows about a turn that ended at a usage limit.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LimitFacts<'a> {
    /// Every account it could use ran out (R1's stop), or the provider said
    /// its quota is spent (`ProviderQuota`, never a self-hosted endpoint).
    pub ran_out: bool,
    /// R1 stopped: every added Claude account was tried or had no room.
    pub every_account: bool,
    /// The turn ended as an error.
    pub failed: bool,
    /// A Stop was pressed.
    pub stopped: bool,
    /// The person's "when one runs out" moved the turn to another agent.
    pub continued_elsewhere: bool,
    /// A night or budget guard ended it, not the limit.
    pub guard_stop: bool,
    /// A person's own chat (depth 0, attended, not bare). A child task turn
    /// or an unattended run keeps failing as before, so nothing resumes it
    /// at another depth or permission.
    pub own_chat: bool,
    /// The native session the stream named; none means nothing to resume.
    pub native: Option<&'a str>,
    /// The agent's label (`Claude`, `Codex`).
    pub agent_label: &'a str,
    /// Review fix: credit or billing exhaustion ("credit balance is too
    /// low"), which no window reset ends: never a limit pause.
    pub credit: bool,
    /// When the limit resets (unix seconds), if the provider said.
    pub reset_at: Option<i64>,
    /// That time as the stop message shows it (`chat_resume::reset_label`).
    pub reset_label: Option<&'a str>,
}

/// A turn that pauses at its limit: the record's note and `resume_at`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LimitPause {
    pub note: String,
    pub resume_at: Option<i64>,
}

/// P1d: whether a turn that ran out pauses instead of failing. It does when
/// it ran out and failed, nothing else ended it (a Stop, a guard), it did
/// not move to another agent, it is the person's own chat and it named a
/// native session to continue. `resume_at` is the reset plus
/// [`RESET_MARGIN_SECS`]; no reset time means paused with no time.
pub(crate) fn limit_pause(facts: &LimitFacts<'_>) -> Option<LimitPause> {
    let pauses = facts.ran_out
        && !facts.credit
        && facts.failed
        && !facts.stopped
        && !facts.continued_elsewhere
        && !facts.guard_stop
        && facts.own_chat
        && facts.native.is_some_and(|id| !id.trim().is_empty());
    pauses.then(|| LimitPause {
        note: limit_note(facts.agent_label, facts.every_account, facts.reset_label),
        resume_at: facts.reset_at.map(|at| at + RESET_MARGIN_SECS),
    })
}

/// Review fix (P1d): a limit hit by a turn an automatic resume started
/// (`token`, [`auto_attempt`]) that never `progressed` (no reply, no tool)
/// counts that attempt, so a reset that did not reopen the window cannot
/// loop each minute; any other turn starts again from 0. At
/// [`MAX_AUTO_ATTEMPTS`] the pause keeps no time and its note says to
/// resume by hand. Returns the pause, its attempt count and whether it gave
/// up.
pub(crate) fn count_auto_attempt(
    limit: LimitPause,
    agent: &str,
    token: Option<&str>,
    progressed: bool,
) -> (LimitPause, i64, bool) {
    let attempts = if progressed {
        0
    } else {
        token.and_then(auto_attempt).unwrap_or(0)
    };
    if attempts < MAX_AUTO_ATTEMPTS {
        return (limit, attempts, false);
    }
    let gave_up = LimitPause {
        note: format!(
            "{agent} hit its usage limit again right after resuming {MAX_AUTO_ATTEMPTS} times. Resume it when there is room."
        ),
        resume_at: None,
    };
    (gave_up, attempts, true)
}

/// The limit pause's note: which limit, and when it resets.
pub(crate) fn limit_note(agent: &str, every_account: bool, resets: Option<&str>) -> String {
    let who = if every_account {
        format!("Every {agent} account hit its usage limit")
    } else {
        format!("{agent} hit its usage limit")
    };
    match resets {
        Some(at) => format!("{who}. It resets at {at}; the chat resumes then."),
        None => format!("{who} and gave no reset time. Resume it when there is room."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn paused(key: &str, at: i64) -> PausedChat {
        PausedChat {
            auto_attempts: 0,
            session_key: key.to_string(),
            native_session_id: format!("native-{key}"),
            agent: "claude".to_string(),
            paused_at: at,
            note: "Reading the plan.".to_string(),
            reason: PauseReason::User,
            resume_at: None,
        }
    }

    #[test]
    fn a_pause_is_asked_only_of_a_running_turn_and_clears() {
        let key = "s-20261115-pa000001";
        assert!(!request_pause(key, false), "no running turn: nothing asked");
        assert!(!pause_requested(key));
        assert!(request_pause(key, true));
        assert!(pause_requested(key));
        assert!(!pause_requested("s-20261115-pa000002"), "only that chat");
        clear_request(key);
        assert!(!pause_requested(key));
    }

    #[test]
    fn the_store_records_lists_gets_and_clears_paused_chats() {
        let dir = vault();
        let v = dir.path();
        let a = paused("s-20261115-pa000011", 100);
        let mut b = paused("s-20261115-pa000012", 200);
        b.reason = PauseReason::UsageLimit;
        b.resume_at = Some(900);
        record(v, &a).unwrap();
        record(v, &b).unwrap();
        assert_eq!(get(v, &a.session_key).unwrap(), Some(a.clone()));
        assert_eq!(get(v, &b.session_key).unwrap(), Some(b.clone()));
        assert_eq!(list(v).unwrap(), vec![b.clone(), a.clone()], "newest first");
        // Recording again replaces the chat's one record.
        let mut again = a.clone();
        again.note = "Writing the report.".into();
        again.paused_at = 300;
        record(v, &again).unwrap();
        assert_eq!(list(v).unwrap(), vec![again.clone(), b.clone()]);
        assert!(clear(v, &a.session_key).unwrap());
        assert!(!clear(v, &a.session_key).unwrap(), "cleared once");
        assert_eq!(get(v, &a.session_key).unwrap(), None);
        on_turn_start(v, &b.session_key);
        assert!(list(v).unwrap().is_empty(), "a new turn unpauses");
        assert!(get(v, "../x").is_err(), "unsafe keys refused");
    }

    #[test]
    fn a_record_serializes_every_field_for_the_later_rows() {
        let mut chat = paused("s-20261115-pa000021", 5);
        chat.reason = PauseReason::All;
        chat.resume_at = Some(77);
        let json = serde_json::to_value(&chat).unwrap();
        assert_eq!(json["sessionKey"], "s-20261115-pa000021");
        assert_eq!(json["nativeSessionId"], "native-s-20261115-pa000021");
        assert_eq!(json["pausedAt"], 5);
        assert_eq!(json["reason"], "all");
        assert_eq!(json["resumeAt"], 77);
        assert_eq!(
            serde_json::to_value(PauseReason::UsageLimit).unwrap(),
            "usage_limit"
        );
        for reason in [PauseReason::User, PauseReason::UsageLimit, PauseReason::All] {
            assert_eq!(PauseReason::parse(reason.as_str()), reason);
        }
    }

    #[test]
    fn the_note_is_one_plain_line() {
        assert_eq!(
            note("First.\n\n## Checking the tests\n", Some("Bash")),
            "Checking the tests"
        );
        assert_eq!(note("  \n", Some("Read")), "Was using Read.");
        assert_eq!(note("", None), NOTE_FALLBACK);
        assert_eq!(note("A \u{2014} B \u{2013} C", None), "A , B , C");
        let long = "word ".repeat(80);
        let cut = note(&long, None);
        assert!(
            cut.chars().count() <= NOTE_CHARS && cut.ends_with("..."),
            "{cut}"
        );
        assert!(!cut.contains('\n'));
    }

    #[test]
    fn a_paused_chat_is_listed_as_paused_not_running() {
        let dir = vault();
        let chat = paused("s-20261115-pa000031", 1_763_200_000);
        record(dir.path(), &chat).unwrap();
        let rows = live_sessions(dir.path());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].key, chat.session_key);
        assert_eq!(rows[0].phase, "paused");
        assert_eq!(rows[0].run_id, "");
        let at = chrono::DateTime::parse_from_rfc3339(&rows[0].phase_entered_at).unwrap();
        assert_eq!(at.timestamp(), chat.paused_at);
    }

    #[test]
    fn resume_refuses_a_busy_or_unpaused_chat_and_continues_the_native_session() {
        let dir = vault();
        let v = dir.path();
        let key = "s-20261115-pa000041";
        let err = resume_input(v, key, false, 50, "pause-resume-t1").unwrap_err();
        assert!(err.contains("isn't paused"), "{err}");
        record(v, &paused(key, 10)).unwrap();
        let err = resume_input(v, key, true, 50, "pause-resume-t1").unwrap_err();
        assert!(err.contains("already has a turn"), "{err}");
        // No conversation row: refused in words.
        let err = resume_input(v, key, false, 50, "pause-resume-t1").unwrap_err();
        assert!(err.contains("no longer exists"), "{err}");
        // With its conversation: the stored native session, the chat's own
        // agent, folder and last permission mode, at depth 0.
        let cwd = v.to_string_lossy().to_string();
        let ord = session_store::start_turn(v, key, "claude", &cwd, "run-p1", "Plan it.").unwrap();
        session_store::finish_turn(v, key, ord, "cancelled").unwrap();
        let input = resume_input(v, key, false, 50, "pause-resume-t1").unwrap();
        assert_eq!(input.prompt, RESUME_PROMPT);
        assert_eq!(
            input.session_id.as_deref(),
            Some("native-s-20261115-pa000041")
        );
        assert_eq!(input.session_key.as_deref(), Some(key));
        assert_eq!(input.cwd.as_deref(), Some(cwd.as_str()));
        assert_eq!(input.agent.as_deref(), Some("claude"));
        assert_eq!(input.app_depth, 0);
        assert!(!input.unattended);
    }
}
