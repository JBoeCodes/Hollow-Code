//! M5 PR3 (2026-11-15): a chat that moved to another Mac, continued here
//! while that Mac was out of reach, syncs to it once it answers again.
//!
//! Design and decisions: the M5 plan's "PR3 design"
//! (`outputs/hollow-build-run-2026-11-14/m5-plan.md` in the Hollow vault
//! workspace); wire: `docs/phone-api/machines.md` (Offline sync).
//!
//! **One owner per chat, always.** Continuing chat K (`away` here, so the
//! other Mac owns it) offline makes a new chat O here ([`continue_offline`]):
//! `parentKey` K, `forkKind` [`FORK_KIND`], a copy of this Mac's replica of
//! K's turns, and a base, K's last turn here (`ord`, `runId`), kept in
//! `chat_offline`. O has no home record, so this Mac owns and runs it as any
//! of its chats; K stays `away` and never runs here.
//!
//! **A sync is a move of O whose bundle is an append** (version
//! [`chat_move::APPEND_VERSION`], [`Append`], only O's turns after the base),
//! sent by `chat_move_send`'s driver over the move routes with the same
//! proof rules: O is `leaving` (it runs nowhere) until the other Mac's answer
//! settles it. The receiving Mac ([`import_append`], one transaction) adds
//! the turns after K's last turn when K hasn't moved on since the base
//! (`appended`), and otherwise puts them, after K's turns up to the base, in
//! a new chat under O's key there (`forked`): K is never overwritten. Every
//! turn is keyed by its run id, so a second delivery adds nothing. This Mac
//! then folds the answer in ([`fold`]): appended, O's turns join K's replica
//! here and O goes; forked, O stays `away` like any moved chat.

use crate::chat_move::{self, Imported, MoveBundle, MoveError, MovedTurn, ARRIVED, AWAY, LEAVING};
use crate::chat_move_send::{self, MoveLink, Sent};
use crate::peer_client::MoveRoute;
use crate::session_store::{db_time, with_read};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// The `forkKind` of an offline continuation, here and there.
pub(crate) const FORK_KIND: &str = "offline";

/// `chat_offline.state`: O runs here and syncs on the next contact.
pub(crate) const OPEN: &str = "open";
/// Its turns were added to K on the other Mac (and to K's replica here).
/// Also `chat_appends.outcome`.
pub(crate) const APPENDED: &str = "appended";
/// It went to the other Mac as its own chat (`away` here). Also
/// `chat_appends.outcome`.
pub(crate) const FORKED: &str = "forked";
/// The other Mac refused it: an ordinary chat of this Mac's from then on.
pub(crate) const KEPT: &str = "kept";

/// The longest run id an append carries.
pub(crate) const RUN_ID_MAX: usize = 200;

/// What an offline sync's bundle adds to: chat `to` there, after `base`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Append {
    pub to: String,
    pub base: Base,
}

/// The last turn both Macs have: its `ord` (0 for none) and run id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Base {
    pub ord: i64,
    #[serde(default)]
    pub run_id: Option<String>,
}

/// An offline continuation (`chat_offline`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offline {
    pub offline_key: String,
    /// The chat it continues (K, `away` here).
    pub key: String,
    /// K's Mac (`machineId`).
    pub peer: String,
    pub base_ord: i64,
    pub base_run: Option<String>,
    pub state: String,
    /// Why it was kept here, in a sentence for the person (`kept` only).
    pub note: Option<String>,
}

fn offline_in(conn: &Connection, offline_key: &str) -> Result<Option<Offline>, String> {
    conn.query_row(
        "SELECT offline_key, session_key, peer, base_ord, base_run, state, note FROM chat_offline \
         WHERE offline_key = ?1",
        params![offline_key],
        |row| {
            Ok(Offline {
                offline_key: row.get(0)?,
                key: row.get(1)?,
                peer: row.get(2)?,
                base_ord: row.get(3)?,
                base_run: row.get(4)?,
                state: row.get(5)?,
                note: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("chat offline read: {e}"))
}

/// Every offline continuation here (the window's notices read them through
/// [`offline_rows`]: a `kept` one says why in `note`).
pub(crate) fn offline_chats(vault: &Path) -> Result<Vec<Offline>, String> {
    let keys: Vec<String> = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT offline_key FROM chat_offline ORDER BY at ASC, rowid ASC")
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("chat offline read: {e}"))
    })?;
    keys.iter()
        .filter_map(|k| offline_of(vault, k).transpose())
        .collect()
}

/// One offline continuation as the window shows it (`chat_offline_rows`):
/// the notice on the chat it continues and on the continuation itself, and
/// the offline badge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineRow {
    /// The continuation's own key (its chat goes here once `appended`; the
    /// record stays, for the notice on the chat it continued).
    pub offline_key: String,
    /// The chat it continues (moved to `peer`).
    pub key: String,
    /// That chat's Mac (`machineId`).
    pub peer: String,
    /// `open`, `appended`, `forked` or `kept`.
    pub state: String,
    /// Why it was kept here (`kept` only), one sentence.
    pub note: Option<String>,
    /// `open` with part of its turns synced already (its base moved past
    /// this Mac's copy of the chat it continues): a long stretch going in
    /// slices.
    pub partly: bool,
}

/// Every offline continuation here, for the window (`chat_offline_rows`).
pub(crate) fn offline_rows(vault: &Path) -> Result<Vec<OfflineRow>, String> {
    let rows = offline_chats(vault)?;
    with_read(vault, |conn| {
        rows.into_iter()
            .map(|o| {
                let last: i64 = conn
                    .query_row(
                        "SELECT COALESCE(MAX(ord), 0) FROM turns WHERE session_key = ?1",
                        params![o.key],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("chat offline read: {e}"))?;
                Ok(OfflineRow {
                    partly: o.state == OPEN && o.base_ord > last,
                    offline_key: o.offline_key,
                    key: o.key,
                    peer: o.peer,
                    state: o.state,
                    note: o.note,
                })
            })
            .collect()
    })
}

/// Chat `offline_key`'s offline record, `None` when it isn't one.
pub(crate) fn offline_of(vault: &Path, offline_key: &str) -> Result<Option<Offline>, String> {
    with_read(vault, |conn| offline_in(conn, offline_key))
}

fn conflict(message: impl Into<String>) -> MoveError {
    MoveError {
        code: "conflict",
        message: message.into(),
    }
}

fn bad(message: impl Into<String>) -> MoveError {
    MoveError {
        code: "bad_request",
        message: message.into(),
    }
}

/// The turn columns a move carries, in `INSERT ... SELECT` order.
const TURN_COLUMNS: &str =
    "ord, run_id, started_at, ended_at, status, asked, permission_mode, effort, asked_by";

// ── This Mac: continue offline ───────────────────────────────────────────

/// Continue chat `key` (moved to another Mac, `away` here) on this Mac while
/// that Mac can't be reached: a new chat of this Mac's with a copy of the
/// turns this Mac has of it, which syncs there on the next contact. Its key;
/// asked again while that chat hasn't synced, the same one.
pub(crate) fn continue_offline(
    vault: &Path,
    key: &str,
    now: DateTime<Utc>,
) -> Result<String, MoveError> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(bad("That isn't a chat's key."));
    }
    let at = db_time(now);
    chat_move::in_write(vault, "chat offline", |tx| {
        let home = chat_move::home_in(tx, key)?;
        let Some(home) = home.filter(|h| h.state == AWAY) else {
            return Err(conflict(
                "Only a chat that moved to another Mac continues offline; this one runs here.",
            ));
        };
        let open: Option<String> = tx
            .query_row(
                "SELECT offline_key FROM chat_offline WHERE session_key = ?1 AND state = ?2 \
                 ORDER BY at DESC LIMIT 1",
                params![key, OPEN],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("chat offline read: {e}"))?;
        if let Some(open) = open {
            return Ok(open);
        }
        let (title, agent, cwd) = tx
            .query_row(
                "SELECT title, agent, cwd FROM sessions WHERE key = ?1",
                params![key],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("chat offline read: {e}"))?
            .ok_or_else(|| MoveError {
                code: "not_found",
                message: "That chat isn't in Hollow.".into(),
            })?;
        let (base_ord, base_run): (i64, Option<String>) = tx
            .query_row(
                "SELECT ord, run_id FROM turns WHERE session_key = ?1 ORDER BY ord DESC LIMIT 1",
                params![key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("chat offline read: {e}"))?
            .unwrap_or((0, None));
        let offline = fresh_key(tx)?;
        let copied = copy_turns(tx, key, &offline, i64::MAX)?;
        tx.execute(
            "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count, \
             parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)",
            params![
                offline,
                title.map(|t| format!("{t} (offline)")),
                agent,
                cwd,
                at,
                copied,
                key,
                FORK_KIND
            ],
        )
        .map_err(|e| format!("chat offline session: {e}"))?;
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![offline, agent, at],
        )
        .map_err(|e| format!("chat offline stint: {e}"))?;
        tx.execute(
            "INSERT INTO chat_efforts(key, effort, updated_at) \
             SELECT ?2, effort, ?3 FROM chat_efforts WHERE key = ?1",
            params![key, offline, at],
        )
        .map_err(|e| format!("chat offline effort: {e}"))?;
        tx.execute(
            "INSERT INTO chat_offline(offline_key, session_key, peer, base_ord, base_run, state, at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![offline, key, home.peer, base_ord, base_run, OPEN, at],
        )
        .map_err(|e| format!("chat offline record: {e}"))?;
        Ok(offline)
    })
}

/// A session key no chat here has.
fn fresh_key(tx: &Connection) -> Result<String, MoveError> {
    for _ in 0..8 {
        let key = crate::agent_sessions::mint_session_key();
        let used: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .map_err(|e| format!("chat offline key: {e}"))?;
        if used == 0 {
            return Ok(key);
        }
    }
    Err(MoveError::from("no free chat key".to_string()))
}

/// Copy chat `from`'s turns up to `upto` (and their transcripts) to chat
/// `to`, ords kept. How many.
fn copy_turns(tx: &Connection, from: &str, to: &str, upto: i64) -> Result<i64, MoveError> {
    let copied = tx
        .execute(
            &format!(
                "INSERT INTO turns(session_key, {TURN_COLUMNS}) SELECT ?2, {TURN_COLUMNS} \
                 FROM turns WHERE session_key = ?1 AND ord <= ?3 ORDER BY ord"
            ),
            params![from, to, upto],
        )
        .map_err(|e| format!("chat offline turns: {e}"))?;
    tx.execute(
        "INSERT INTO transcripts(session_key, turn_ord, payload) \
         SELECT ?2, turn_ord, payload FROM transcripts WHERE session_key = ?1 AND turn_ord <= ?3",
        params![from, to, upto],
    )
    .map_err(|e| format!("chat offline transcripts: {e}"))?;
    Ok(copied as i64)
}

/// Shape `bundle` as an append when its chat is an offline continuation
/// still to sync: version 2, its base, and only its turns after the base,
/// each of which must carry a run id. A chat that isn't one is left as it
/// is (a plain move).
pub(crate) fn shape_append(tx: &Connection, bundle: &mut MoveBundle) -> Result<(), MoveError> {
    let Some(offline) = offline_in(tx, &bundle.key)? else {
        return Ok(());
    };
    if offline.state != OPEN {
        return Ok(());
    }
    // `chat_move::read_bundle` read only the turns after the base already
    // (`delta_base`), so the caps count the delta.
    bundle.turns.retain(|turn| turn.ord > offline.base_ord);
    if bundle.turns.iter().any(|turn| !valid_run_id(&turn.run_id)) {
        return Err(MoveError {
            code: UNSYNCABLE,
            message: "A turn of this offline chat has no run id, so it can't be synced; it stays on this Mac."
                .into(),
        });
    }
    bundle.version = chat_move::APPEND_VERSION;
    // The fresh stint there is the chat's; this Mac's native id is no use.
    bundle.native = None;
    bundle.append = Some(Append {
        to: offline.key,
        base: Base {
            ord: offline.base_ord,
            run_id: offline.base_run,
        },
    });
    Ok(())
}

/// The bundle bytes one slice's turns may take: one sync's cap less room
/// for the rest of the bundle (the chat's record, folder and base).
#[cfg(test)]
pub(crate) const SLICE_BYTES: usize = chat_move::Caps::SYNC.slice_bytes;

/// How much of an offline stretch one sync carries: its first turns, in
/// order, at most `chat_move::TURNS_MAX` of them and [`SLICE_BYTES`]. A
/// longer stretch goes in several syncs, each moving the base to the last
/// turn it carried ([`fold`]). The first turn is always taken: one turn too
/// large alone is refused by `chat_move::encode` and the chat is kept here.
/// It reads only the turns, which don't change while the chat is sent, so a
/// retry rebuilds the same slice under the same move id.
#[derive(Debug)]
pub(crate) struct Slice {
    turns: usize,
    bytes: usize,
    caps: chat_move::Caps,
}

impl Default for Slice {
    fn default() -> Self {
        Self::within(chat_move::Caps::SYNC)
    }
}

impl Slice {
    /// A slice under other caps than a sync's (a sliced move's, tests').
    pub(crate) fn within(caps: chat_move::Caps) -> Self {
        Self {
            turns: 0,
            bytes: 0,
            caps,
        }
    }

    /// Whether `turn`, the next one, still fits this sync (and count it).
    pub(crate) fn takes(&mut self, turn: &MovedTurn) -> bool {
        // The turn as the bundle's JSON carries it, and its comma.
        let size = serde_json::to_vec(turn).map_or(self.caps.slice_bytes, |b| b.len()) + 1;
        if self.turns > 0
            && (self.turns >= self.caps.turns
                || self.bytes.saturating_add(size) > self.caps.slice_bytes)
        {
            return false;
        }
        self.turns += 1;
        self.bytes = self.bytes.saturating_add(size);
        true
    }
}

/// The base of chat `key` when it is an offline continuation still to sync
/// (only its turns after it travel), `None` otherwise.
pub(crate) fn delta_base(tx: &Connection, key: &str) -> Result<Option<i64>, String> {
    Ok(offline_in(tx, key)?
        .filter(|o| o.state == OPEN)
        .map(|o| o.base_ord))
}

/// A sync refused on this Mac before anything was sent, for good (a turn
/// with no run id): the chat stays this Mac's.
pub(crate) const UNSYNCABLE: &str = "unsyncable";

fn valid_run_id(run_id: &Option<String>) -> bool {
    run_id.as_deref().is_some_and(|id| {
        !id.is_empty() && id.len() <= RUN_ID_MAX && !id.chars().any(char::is_control)
    })
}

/// The append's own checks, past `chat_move::check_bundle`'s: a chat key
/// other than the bundle's, a base, and every turn after it with a run id
/// of its own.
pub(crate) fn check_append(bundle: &MoveBundle, append: &Append) -> Result<(), MoveError> {
    if !crate::agent_sessions::is_safe_session_key(&append.to) || append.to == bundle.key {
        return Err(bad("append.to must be another chat's key."));
    }
    if append.base.ord < 0
        || append
            .base
            .run_id
            .as_ref()
            .is_some_and(|_| !valid_run_id(&append.base.run_id))
    {
        return Err(bad("append.base isn't a turn."));
    }
    let mut seen = HashSet::new();
    for turn in &bundle.turns {
        if turn.ord <= append.base.ord {
            return Err(bad("An append carries only the turns after its base."));
        }
        if !valid_run_id(&turn.run_id) {
            return Err(bad("Every appended turn needs its run id."));
        }
        if !seen.insert(turn.run_id.as_deref().unwrap_or_default()) {
            return Err(bad("Each appended turn must come once."));
        }
    }
    Ok(())
}

// ── The receiving Mac: append or fork ────────────────────────────────────

/// An offline sync this Mac took in: `(outcome, the key its turns are in,
/// the sending Mac)`.
pub(crate) fn outcome_in(
    conn: &Connection,
    move_id: &str,
) -> Result<Option<(String, String, String)>, String> {
    conn.query_row(
        "SELECT outcome, CASE outcome WHEN ?2 THEN session_key ELSE offline_key END, peer \
         FROM chat_appends WHERE move_id = ?1",
        params![move_id, APPENDED],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .map_err(|e| format!("chat sync read: {e}"))
}

/// Whether a `DELETE` of offline sync `move_id` must answer `started`: its
/// turns were added to a chat here (appended, or into the fork an earlier
/// sync made). A fork this sync made itself is undone as a move's.
pub(crate) fn sync_stands(conn: &Connection, move_id: &str) -> Result<bool, String> {
    let Some((outcome, _, _)) = outcome_in(conn, move_id)? else {
        return Ok(false);
    };
    if outcome == APPENDED {
        return Ok(true);
    }
    let own: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chat_moves WHERE move_id = ?1 AND direction = 'in' AND state = 'imported'",
            params![move_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("move log read: {e}"))?;
    Ok(own == 0)
}

/// A time the other Mac sent, as this Mac stores times (`db_time`, UTC):
/// `now` when it is later than `now` or isn't a time at all, so a sender
/// can't put a chat or a turn in the future.
fn not_after(sent: &str, now: DateTime<Utc>) -> String {
    let at = DateTime::parse_from_rfc3339(sent)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .filter(|t| *t <= now)
        .unwrap_or(now);
    db_time(at)
}

/// The common turns of `chat` here agree with the sync: its turn at the
/// base carries the base's run id, and no offline run id names one of its
/// turns up to the base. Otherwise refused for good (`conflict`).
fn check_common(
    tx: &Connection,
    chat: &str,
    append: &Append,
    sent: &HashSet<&str>,
) -> Result<(), MoveError> {
    if append.base.ord > 0 {
        let at_base: Option<Option<String>> = tx
            .query_row(
                "SELECT run_id FROM turns WHERE session_key = ?1 AND ord = ?2",
                params![chat, append.base.ord],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("chat sync read: {e}"))?;
        if at_base.as_ref() != Some(&append.base.run_id) {
            return Err(conflict(
                "That chat changed here before the offline turns, so they stay on the other Mac.",
            ));
        }
    }
    let common = run_ids_upto(tx, chat, append.base.ord)?;
    if common
        .iter()
        .flatten()
        .any(|run| sent.contains(run.as_str()))
    {
        return Err(conflict(
            "Those turns name turns this chat had before, so they stay on the other Mac.",
        ));
    }
    Ok(())
}

/// How many of `turns`, from the first, `have` (an earlier delivery of
/// them); a turn here after one missing is refused (`conflict`): the
/// offline turns arrive in their order or not at all.
fn present_prefix(turns: &[MovedTurn], have: &HashSet<&str>) -> Result<usize, MoveError> {
    let here = |t: &MovedTurn| t.run_id.as_deref().is_some_and(|r| have.contains(r));
    let prefix = turns.iter().take_while(|t| here(t)).count();
    if turns[prefix..].iter().any(here) {
        return Err(conflict(
            "Those turns are here out of their order, so they stay on the other Mac.",
        ));
    }
    Ok(prefix)
}

/// Whether chat `key` here is running or holds a turn: a turn queued or
/// running, or its agent process still warm (`agent_sessions`).
fn busy_here(tx: &Connection, key: &str) -> Result<bool, String> {
    Ok(chat_move::turn_in_flight(tx, key)? || crate::agent_sessions::live_for_key(key).is_some())
}

/// The fork an earlier sync of offline chat `offline_key` made here (from
/// `caller`, still here and still `arrived` from it).
fn earlier_fork(tx: &Connection, offline_key: &str, caller: &str) -> Result<bool, String> {
    let recorded: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM chat_appends WHERE offline_key = ?1 AND peer = ?2 AND outcome = ?3",
            params![offline_key, caller, FORKED],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat sync read: {e}"))?;
    if recorded == 0 {
        return Ok(false);
    }
    let here: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE key = ?1",
            params![offline_key],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat sync read: {e}"))?;
    let home = chat_move::home_in(tx, offline_key)?;
    Ok(here > 0 && home.is_some_and(|h| h.state == ARRIVED && h.peer == caller))
}

fn outcome_word(outcome: &str) -> Option<&'static str> {
    match outcome {
        APPENDED => Some(APPENDED),
        FORKED => Some(FORKED),
        _ => None,
    }
}

fn already_in(conn: &Connection, move_id: &str) -> Result<Option<Imported>, MoveError> {
    let found: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT outcome, CASE outcome WHEN ?2 THEN session_key ELSE offline_key END, turns \
             FROM chat_appends WHERE move_id = ?1",
            params![move_id, APPENDED],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| MoveError::from(format!("chat sync read: {e}")))?;
    Ok(found.map(|(outcome, key, turns)| Imported {
        move_id: move_id.to_string(),
        key,
        turns: turns as usize,
        already: true,
        outcome: outcome_word(&outcome),
    }))
}

/// The answer again for offline sync `move_id`, taken in already.
pub(crate) fn already(vault: &Path, move_id: &str) -> Result<Imported, MoveError> {
    with_read(vault, |conn| {
        already_in(conn, move_id).map_err(|e| e.message)
    })?
    .ok_or_else(|| MoveError::from(format!("no sync {move_id}")))
}

/// Take in offline sync `bundle` from the paired Mac `caller`, in one
/// transaction: appended to chat `append.to` when it hasn't moved on here
/// since the base, else forked into a new chat under the bundle's key. A
/// sync already taken in answers again with `already`; a called-off one is
/// refused. `bundle` passed `chat_move::check_bundle`.
pub(crate) fn import_append(
    vault: &Path,
    bundle: &MoveBundle,
    caller: &str,
    now: DateTime<Utc>,
) -> Result<Imported, MoveError> {
    let Some(append) = bundle.append.as_ref() else {
        return Err(bad("That isn't an offline sync."));
    };
    let at = db_time(now);
    let imported = chat_move::in_write(vault, "chat sync in", |tx| {
        if let Some((_, _, peer)) = outcome_in(tx, &bundle.move_id)? {
            if peer != caller {
                return Err(MoveError {
                    code: "forbidden",
                    message: "That move isn't this Mac's.".into(),
                });
            }
            return already_in(tx, &bundle.move_id)?
                .ok_or_else(|| MoveError::from("sync record vanished".to_string()));
        }
        let logged: Option<String> = tx
            .query_row(
                "SELECT state FROM chat_moves WHERE move_id = ?1 AND direction = 'in'",
                params![bundle.move_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("move log read: {e}"))?;
        if logged.is_some() {
            return Err(MoveError {
                code: "cancelled",
                message: "That move was called off; start it again.".into(),
            });
        }
        let sent: HashSet<&str> = bundle
            .turns
            .iter()
            .filter_map(|t| t.run_id.as_deref())
            .collect();
        // An earlier sync of this chat forked it here (the person released it
        // on the other Mac and it was sent again): its turns go to that fork,
        // each run id once, never into a second fork or the chat it left.
        if earlier_fork(tx, &bundle.key, caller)? {
            return into_earlier_fork(tx, bundle, append, &sent, caller, now);
        }
        let k = append.to.as_str();
        let gone = || {
            conflict("That chat doesn't live on this Mac any more, so the offline turns stay on the other Mac.")
        };
        let (cwd, archived): (String, bool) = tx
            .query_row(
                "SELECT cwd, archived FROM sessions WHERE key = ?1",
                params![k],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("chat sync read: {e}"))?
            .ok_or_else(gone)?;
        match chat_move::home_in(tx, k)? {
            Some(h) if h.state == ARRIVED && h.peer == caller => {}
            // Moving on from here, or living on another Mac now (it may come
            // back): no proof of anything, the sender tries again later.
            Some(h) if h.state == LEAVING || h.state == AWAY => {
                return Err(MoveError {
                    code: "busy",
                    message:
                        "That chat is moving or lives on another Mac now; the offline turns wait."
                            .into(),
                })
            }
            _ => return Err(gone()),
        }
        check_common(tx, k, append, &sent)?;
        let after = run_ids(tx, k, Some(append.base.ord))?;
        let have: HashSet<&str> = after.iter().filter_map(|r| r.as_deref()).collect();
        // The first offline turns may be here already (an earlier delivery).
        let present = present_prefix(&bundle.turns, &have)?;
        let missing = &bundle.turns[present..];
        // All of them here already (a sync sent again): nothing to add,
        // whatever the chat did since, and never a fork of copies.
        if missing.is_empty() {
            record(tx, bundle, append, caller, APPENDED, 0, &at)?;
            return Ok(Imported {
                move_id: bundle.move_id.clone(),
                key: k.to_string(),
                turns: 0,
                already: false,
                outcome: Some(APPENDED),
            });
        }
        let moved_on = after
            .iter()
            .any(|run| run.as_deref().is_none_or(|run| !sent.contains(run)));
        if archived || moved_on || busy_here(tx, k)? {
            // The fork starts where the two histories part: after the last
            // offline turn here already, else at the base. It holds only
            // the turns missing here, so nothing is in two chats.
            let upto = match present {
                0 => append.base.ord,
                n => tx
                    .query_row(
                        "SELECT ord FROM turns WHERE session_key = ?1 AND run_id = ?2",
                        params![k, bundle.turns[n - 1].run_id],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("chat sync read: {e}"))?,
            };
            fork_into(tx, bundle, append, missing, upto, &cwd, caller, now)
        } else {
            let last = not_after(&bundle.session.last_activity_at, now);
            let added = append_into(tx, k, missing, &last, now)?;
            record(tx, bundle, append, caller, APPENDED, added, &at)?;
            Ok(Imported {
                move_id: bundle.move_id.clone(),
                key: k.to_string(),
                turns: added,
                already: false,
                outcome: Some(APPENDED),
            })
        }
    })?;
    // A paused record names the chat's old native session, which never saw
    // the added turns: it goes, as a moved chat's does.
    if imported.outcome == Some(APPENDED) && imported.turns > 0 {
        if let Err(error) = crate::chat_pause::clear(vault, &imported.key) {
            crate::launch::elog(&format!("[chat offline] {}: {error}", imported.key));
        }
    }
    Ok(imported)
}

/// Sync `bundle` again into the fork an earlier sync of it made here: the
/// turns the fork lacks (by run id) are added when the fork hasn't moved on
/// since the base, nothing when it has them all, else refused (they stay on
/// the other Mac).
fn into_earlier_fork(
    tx: &Connection,
    bundle: &MoveBundle,
    append: &Append,
    sent: &HashSet<&str>,
    caller: &str,
    now: DateTime<Utc>,
) -> Result<Imported, MoveError> {
    let fork = bundle.key.as_str();
    check_common(tx, fork, append, sent)?;
    let after = run_ids(tx, fork, Some(append.base.ord))?;
    let have: HashSet<&str> = after.iter().filter_map(|r| r.as_deref()).collect();
    let present = present_prefix(&bundle.turns, &have)?;
    let missing = &bundle.turns[present..];
    let added = if missing.is_empty() {
        0
    } else {
        let moved_on = after
            .iter()
            .any(|run| run.as_deref().is_none_or(|run| !sent.contains(run)));
        if moved_on || busy_here(tx, fork)? {
            return Err(conflict(
                "This chat's offline copy went on here already, so these turns stay on the other Mac.",
            ));
        }
        let last = not_after(&bundle.session.last_activity_at, now);
        append_into(tx, fork, missing, &last, now)?
    };
    record(tx, bundle, append, caller, FORKED, added, &db_time(now))?;
    Ok(Imported {
        move_id: bundle.move_id.clone(),
        key: fork.to_string(),
        turns: added,
        already: false,
        outcome: Some(FORKED),
    })
}

/// Chat `key`'s run ids of the turns up to `upto`.
fn run_ids_upto(tx: &Connection, key: &str, upto: i64) -> Result<Vec<Option<String>>, String> {
    let mut stmt = tx
        .prepare("SELECT run_id FROM turns WHERE session_key = ?1 AND ord <= ?2 ORDER BY ord")
        .map_err(|e| format!("chat sync read: {e}"))?;
    let rows = stmt
        .query_map(params![key, upto], |row| row.get(0))
        .map_err(|e| format!("chat sync read: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("chat sync read: {e}"))
}

/// Chat `key`'s run ids, of the turns after `after` (all with `None`).
fn run_ids(tx: &Connection, key: &str, after: Option<i64>) -> Result<Vec<Option<String>>, String> {
    let mut stmt = tx
        .prepare("SELECT run_id FROM turns WHERE session_key = ?1 AND ord > ?2 ORDER BY ord")
        .map_err(|e| format!("chat sync read: {e}"))?;
    let rows = stmt
        .query_map(params![key, after.unwrap_or(i64::MIN)], |row| row.get(0))
        .map_err(|e| format!("chat sync read: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("chat sync read: {e}"))
}

fn insert_turn(
    tx: &Connection,
    key: &str,
    ord: i64,
    turn: &MovedTurn,
    now: DateTime<Utc>,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked, \
         permission_mode, effort, asked_by) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            key,
            ord,
            turn.run_id,
            not_after(&turn.started_at, now),
            turn.ended_at.as_deref().map(|t| not_after(t, now)),
            turn.status,
            turn.asked,
            turn.permission_mode,
            turn.effort.as_deref().and_then(chat_move::canonical_effort),
            turn.asked_by
        ],
    )
    .map_err(|e| format!("chat sync turn: {e}"))?;
    if let Some(transcript) = &turn.transcript {
        tx.execute(
            "INSERT INTO transcripts(session_key, turn_ord, payload) VALUES(?1, ?2, ?3)",
            params![key, ord, transcript.to_string()],
        )
        .map_err(|e| format!("chat sync transcript: {e}"))?;
    }
    Ok(())
}

/// Add `turns` that chat `key` lacks (by run id) after its last turn, in
/// order. How many were added. When any were, the chat's native session
/// (which never saw them) is closed as a moved chat's is, so its next turn
/// starts a fresh one with the whole conversation, and a send naming the
/// old id resets (`chat_resume::ResetReason::Moved`).
fn append_into(
    tx: &Connection,
    key: &str,
    turns: &[MovedTurn],
    last_activity: &str,
    now: DateTime<Utc>,
) -> Result<usize, String> {
    let at = db_time(now);
    let have: HashSet<String> = run_ids(tx, key, None)?.into_iter().flatten().collect();
    let mut ord: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ord), 0) FROM turns WHERE session_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat sync read: {e}"))?;
    let mut added = 0;
    for turn in turns {
        if turn.run_id.as_ref().is_some_and(|run| have.contains(run)) {
            continue;
        }
        ord += 1;
        insert_turn(tx, key, ord, turn, now)?;
        added += 1;
    }
    if added > 0 {
        // Its folded task state is behind the added turns: folded again.
        tx.execute(
            "DELETE FROM derived_state WHERE session_key = ?1",
            params![key],
        )
        .map_err(|e| format!("chat sync state: {e}"))?;
        tx.execute(
            "UPDATE stints SET ended_at = ?2, reason = ?3 WHERE session_key = ?1 AND ended_at IS NULL",
            params![key, at, crate::session_store::MOVED_REASON],
        )
        .map_err(|e| format!("chat sync stint: {e}"))?;
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) \
             SELECT key, agent, ?2 FROM sessions WHERE key = ?1",
            params![key, at],
        )
        .map_err(|e| format!("chat sync stint: {e}"))?;
    }
    touch(tx, key, last_activity)?;
    Ok(added)
}

/// Chat `key`'s turn count and last activity after turns were added.
fn touch(tx: &Connection, key: &str, last_activity: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE sessions SET turn_count = (SELECT COUNT(*) FROM turns WHERE session_key = ?1), \
         last_activity_at = MAX(last_activity_at, ?2) WHERE key = ?1",
        params![key, last_activity],
    )
    .map_err(|e| format!("chat sync session: {e}"))?;
    Ok(())
}

/// The chat moved on here since the base: a new chat under the bundle's key,
/// with this Mac's turns of `append.to` up to the base and then the
/// offline turns, `arrived` from `caller`. `append.to` is not touched.
#[allow(clippy::too_many_arguments)]
fn fork_into(
    tx: &Transaction<'_>,
    bundle: &MoveBundle,
    append: &Append,
    turns: &[MovedTurn],
    upto: i64,
    cwd: &str,
    caller: &str,
    now: DateTime<Utc>,
) -> Result<Imported, MoveError> {
    let at = &db_time(now);
    let key = bundle.key.as_str();
    let exists: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat sync read: {e}"))?;
    if exists > 0 {
        return Err(conflict("A chat with this key is already on this Mac."));
    }
    copy_turns(tx, &append.to, key, upto)?;
    for (n, turn) in turns.iter().enumerate() {
        insert_turn(tx, key, upto + 1 + n as i64, turn, now)?;
    }
    let s = &bundle.session;
    let total: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE session_key = ?1",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat sync read: {e}"))?;
    tx.execute(
        "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count, \
         parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            key,
            s.title,
            s.agent,
            cwd,
            not_after(&s.created_at, now),
            not_after(&s.last_activity_at, now),
            total,
            append.to,
            FORK_KIND
        ],
    )
    .map_err(|e| format!("chat sync session: {e}"))?;
    tx.execute(
        "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
        params![key, s.agent, at],
    )
    .map_err(|e| format!("chat sync stint: {e}"))?;
    if let Some(effort) = bundle
        .effort
        .as_deref()
        .and_then(chat_move::canonical_effort)
    {
        tx.execute(
            "INSERT INTO chat_efforts(key, effort, updated_at) VALUES(?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET effort = ?2, updated_at = ?3",
            params![key, effort, at],
        )
        .map_err(|e| format!("chat sync effort: {e}"))?;
    }
    tx.execute(
        "INSERT INTO chat_homes(session_key, state, peer, move_id, at) VALUES(?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(session_key) DO UPDATE SET state = ?2, peer = ?3, move_id = ?4, at = ?5",
        params![key, ARRIVED, caller, bundle.move_id, at],
    )
    .map_err(|e| format!("chat home write: {e}"))?;
    // A move in, so a `DELETE` of it undoes it as a move's while no turn
    // ran here (`chat_move::cancel_move_in`).
    tx.execute(
        "INSERT INTO chat_moves(move_id, session_key, direction, peer, state, turns, at) \
         VALUES(?1, ?2, 'in', ?3, 'imported', ?4, ?5)",
        params![bundle.move_id, key, caller, total, at],
    )
    .map_err(|e| format!("move log: {e}"))?;
    record(tx, bundle, append, caller, FORKED, turns.len(), at)?;
    Ok(Imported {
        move_id: bundle.move_id.clone(),
        key: key.to_string(),
        turns: turns.len(),
        already: false,
        outcome: Some(FORKED),
    })
}

fn record(
    tx: &Connection,
    bundle: &MoveBundle,
    append: &Append,
    caller: &str,
    outcome: &str,
    turns: usize,
    at: &str,
) -> Result<(), String> {
    tx.execute(
        "INSERT INTO chat_appends(move_id, session_key, offline_key, peer, outcome, turns, at) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            bundle.move_id,
            append.to,
            bundle.key,
            caller,
            outcome,
            turns as i64,
            at
        ],
    )
    .map_err(|e| format!("chat sync record: {e}"))?;
    Ok(())
}

// ── This Mac again: the answer folded in ─────────────────────────────────

/// Why chat `key` doesn't run here: it was an offline continuation whose
/// turns were added to the chat it continued, on the other Mac.
pub(crate) fn folded_refusal(vault: &Path, key: &str) -> Option<String> {
    let offline = offline_of(vault, key).ok().flatten()?;
    (offline.state == APPENDED).then(|| {
        "This chat's offline turns were added to the chat it continued, on the other Mac. Continue there."
            .to_string()
    })
}

/// Offline sync `move_id` came home for good (the other Mac refused it
/// for good, or the person chose Keep it here): the chat stays an ordinary
/// chat of this Mac's, never sent as a sync again (a Move to runner carries
/// it whole). Only while it is home (no home record): a sync that landed is
/// never kept.
pub(crate) fn keep_move(
    vault: &Path,
    move_id: &str,
    note: &str,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let at = db_time(now);
    crate::session_store::with_write(vault, "chat offline kept", |tx| {
        tx.execute(
            "UPDATE chat_offline SET state = ?2, at = ?3, note = ?5 WHERE state = ?4 AND offline_key = \
             (SELECT session_key FROM chat_moves WHERE move_id = ?1 AND direction = 'out') \
             AND NOT EXISTS (SELECT 1 FROM chat_homes h WHERE h.session_key = chat_offline.offline_key)",
            params![move_id, KEPT, at, OPEN, note],
        )
        .map(|_| ())
        .map_err(|e| format!("chat offline record: {e}"))
    })
    .map_err(MoveError::from)?;
    changed();
    Ok(())
}

/// Offline chat `offline_key` is the person's own again: a sync of it that
/// was refused before it was sent (too large, its folder gone).
fn keep(vault: &Path, offline_key: &str, note: &str, now: DateTime<Utc>) -> Result<(), String> {
    let at = db_time(now);
    crate::session_store::with_write(vault, "chat offline kept", |tx| {
        tx.execute(
            "UPDATE chat_offline SET state = ?2, at = ?3, note = ?5 WHERE offline_key = ?1 AND state = ?4 \
             AND NOT EXISTS (SELECT 1 FROM chat_homes h WHERE h.session_key = ?1)",
            params![offline_key, KEPT, at, OPEN, note],
        )
        .map(|_| ())
        .map_err(|e| format!("chat offline record: {e}"))
    })
}

/// Fold the other Mac's answer to offline sync `move_id` of chat
/// `offline_key` in, once the move finished (O `away` under that move):
/// `appended` (to `into`, the chat it continued): its turns after the base
/// join that chat's replica here (each run id once) and it goes; `forked`
/// (`into` is its own key): it stays `away`. Idempotent.
///
/// A sync that carried only a slice of the turns after the base ([`Slice`])
/// settles nothing yet: the base moves to the last turn it carried and the
/// chat comes home, open, so the next contact sends the rest (into the chat
/// it continued, or into the fork this slice made): [`Folded::Sliced`].
/// An answer to a slice folded already (read again, or racing the next
/// slice's send): [`Folded::Stale`], nothing changes.
pub(crate) fn fold(
    vault: &Path,
    offline_key: &str,
    move_id: &str,
    outcome: &str,
    into: &str,
    now: DateTime<Utc>,
) -> Result<Folded, MoveError> {
    let at = db_time(now);
    chat_move::in_write(vault, "chat offline fold", |tx| {
        let offline = offline_in(tx, offline_key)?
            .ok_or_else(|| bad("That chat isn't an offline continuation."))?;
        if offline.state == outcome {
            return Ok(Folded::Settled);
        }
        if offline.state != OPEN {
            return Err(conflict(format!(
                "That offline chat is {} already.",
                offline.state
            )));
        }
        let home = chat_move::home_in(tx, offline_key)?;
        if !home.is_some_and(|h| h.state == AWAY && h.move_id == move_id) {
            // A slice of O's folded already (the same answer read again, or
            // a retry after the fold): its move is done and O came home or
            // is out with a later slice. Nothing more: the base moved once,
            // and whoever sent the later slice settles it.
            if out_done(tx, offline_key, move_id)? {
                return Ok(Folded::Stale);
            }
            return Err(conflict("That sync hasn't landed yet."));
        }
        // From here O is `away` under this very move, so its base is still
        // the one the slice was cut from (a slice's fold moves the base and
        // drops the home record in one transaction).
        match outcome {
            FORKED if into == offline_key => {}
            APPENDED if into == offline.key => {}
            _ => return Err(bad("That answer doesn't match the sync.")),
        }
        if let Some(end) = slice_end(tx, &offline, move_id)? {
            // One slice landed: the rest follows it in the next sync. This
            // Mac's copy of the chat it continued is folded once, at the
            // end, from its own last turn (`fold_turns`).
            tx.execute(
                "UPDATE chat_offline SET base_ord = ?2, base_run = ?3, at = ?4 WHERE offline_key = ?1",
                params![offline_key, end.ord, end.run_id, at],
            )
            .map_err(|e| format!("chat offline record: {e}"))?;
            tx.execute(
                "DELETE FROM chat_homes WHERE session_key = ?1 AND move_id = ?2",
                params![offline_key, move_id],
            )
            .map_err(|e| format!("chat home write: {e}"))?;
            return Ok(Folded::Sliced);
        }
        if outcome == APPENDED {
            fold_turns(tx, &offline)?;
        }
        tx.execute(
            "UPDATE chat_offline SET state = ?2, at = ?3 WHERE offline_key = ?1",
            params![offline_key, outcome, at],
        )
        .map_err(|e| format!("chat offline record: {e}"))?;
        Ok(Folded::Settled)
    })
}

/// What [`fold`] did with an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Folded {
    /// The sync is settled (appended or forked), now or before.
    Settled,
    /// One slice landed; the chat is back here, open, with the rest.
    Sliced,
    /// The answer to a slice folded already: nothing changed.
    Stale,
}

/// Whether this Mac's move `move_id` of chat `key` out finished (landed
/// there).
fn out_done(tx: &Connection, key: &str, move_id: &str) -> Result<bool, String> {
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM chat_moves WHERE move_id = ?1 AND direction = 'out' \
             AND session_key = ?2",
            params![move_id, key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("move log read: {e}"))?;
    Ok(state.as_deref() == Some("done"))
}

/// The last turn offline sync `move_id` carried when it was a slice (more
/// of `offline`'s turns after its base stay here), `None` when it carried
/// them all. The move log holds how many it carried; the chat's turns
/// didn't change since (it ran nowhere while it was sent).
fn slice_end(tx: &Connection, offline: &Offline, move_id: &str) -> Result<Option<Base>, String> {
    let sent: i64 = tx
        .query_row(
            "SELECT turns FROM chat_moves WHERE move_id = ?1 AND direction = 'out'",
            params![move_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("move log read: {e}"))?
        .unwrap_or(0);
    if sent <= 0 {
        return Ok(None);
    }
    let mut stmt = tx
        .prepare(
            "SELECT ord, run_id FROM turns WHERE session_key = ?1 AND ord > ?2 ORDER BY ord LIMIT ?3",
        )
        .map_err(|e| format!("chat offline read: {e}"))?;
    let rows = stmt
        .query_map(
            params![offline.offline_key, offline.base_ord, sent + 1],
            |row| {
                Ok(Base {
                    ord: row.get(0)?,
                    run_id: row.get(1)?,
                })
            },
        )
        .map_err(|e| format!("chat offline read: {e}"))?;
    let after: Vec<Base> = rows
        .collect::<Result<_, _>>()
        .map_err(|e| format!("chat offline read: {e}"))?;
    Ok((after.len() as i64 > sent).then(|| after[sent as usize - 1].clone()))
}

/// The per-turn tables keyed by `(session_key, turn_ord)` whose rows follow
/// a turn folded into the chat it continued. What stays behind is O's own
/// and goes with it: its copies of K's turns, `derived_state` (folded again
/// from K's turns), and the rows of a turn K had already.
const TURN_TABLES: [&str; 5] = [
    "transcripts",
    "events",
    "steps",
    "interactions",
    "turn_feedback",
];

/// O's turns after its first base join K's replica after its last turn, as
/// the other Mac added them (a run id K has already is skipped); then O
/// goes. The first base is K's replica's last turn: O's base moved past it
/// when the stretch went in slices, and K's replica here is folded only
/// now, once.
fn fold_turns(tx: &Transaction<'_>, offline: &Offline) -> Result<(), MoveError> {
    let (o, k) = (offline.offline_key.as_str(), offline.key.as_str());
    let replica: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE key = ?1",
            params![k],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat offline read: {e}"))?;
    if replica == 0 {
        // This Mac's copy of K went (deleted here while O was open): the
        // turns live in K on the other Mac, which owns it; none are written
        // here under a key with no chat, and O goes.
        crate::launch::elog(&format!(
            "[chat offline] {o}: its turns went to {k} there; this Mac has no copy of {k}"
        ));
        return drop_offline_rows(tx, o);
    }
    let have: HashMap<String, i64> = {
        let mut stmt = tx
            .prepare("SELECT run_id, ord FROM turns WHERE session_key = ?1 AND run_id IS NOT NULL")
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![k], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("chat offline read: {e}"))?
    };
    let mut ord: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(ord), 0) FROM turns WHERE session_key = ?1",
            params![k],
            |row| row.get(0),
        )
        .map_err(|e| format!("chat offline read: {e}"))?;
    let from = offline.base_ord.min(ord);
    let rows: Vec<(i64, Option<String>)> = {
        let mut stmt = tx
            .prepare(
                "SELECT ord, run_id FROM turns WHERE session_key = ?1 AND ord > ?2 ORDER BY ord",
            )
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![o, from], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("chat offline read: {e}"))?
    };
    // Where each of O's turns after the base is in K: a run id K has
    // already keeps K's ord (that turn's own rows here go with O), the rest
    // follow K's last turn in order.
    let mut moved_to: HashMap<i64, i64> = HashMap::new();
    let mut moving: Vec<(i64, i64)> = Vec::new();
    for (old, run) in rows {
        match run.as_ref().and_then(|run| have.get(run)) {
            Some(there) => {
                moved_to.insert(old, *there);
            }
            None => {
                ord += 1;
                moved_to.insert(old, ord);
                moving.push((old, ord));
            }
        }
    }
    // A card answered after one of O's turns names that turn in K: set by
    // row id before any row moves, so no remap is applied twice.
    let answered: Vec<(String, i64)> = {
        let mut stmt = tx
            .prepare(
                "SELECT id, answered_after_ord FROM interactions \
                 WHERE session_key = ?1 AND answered_after_ord > ?2",
            )
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![o, from], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<_, _>>()
            .map_err(|e| format!("chat offline read: {e}"))?
    };
    for (id, after) in answered {
        if let Some(new) = moved_to.get(&after) {
            tx.execute(
                "UPDATE interactions SET answered_after_ord = ?2 WHERE id = ?1",
                params![id, new],
            )
            .map_err(|e| format!("chat offline fold interactions: {e}"))?;
        }
    }
    // New ords are past K's last and O's rows are moved one turn at a time
    // into K, so no row is moved twice.
    for (old, new) in moving {
        tx.execute(
            "UPDATE turns SET session_key = ?3, ord = ?4 WHERE session_key = ?1 AND ord = ?2",
            params![o, old, k, new],
        )
        .map_err(|e| format!("chat offline fold: {e}"))?;
        // The turn's own records go with it: its transcript, trace events,
        // step spine, question cards and the person's feedback vote.
        for table in TURN_TABLES {
            tx.execute(
                &format!(
                    "UPDATE {table} SET session_key = ?3, turn_ord = ?4 \
                     WHERE session_key = ?1 AND turn_ord = ?2"
                ),
                params![o, old, k, new],
            )
            .map_err(|e| format!("chat offline fold {table}: {e}"))?;
        }
    }
    // K's folded task state is behind the added turns: folded again.
    tx.execute(
        "DELETE FROM derived_state WHERE session_key = ?1",
        params![k],
    )
    .map_err(|e| format!("chat offline fold: {e}"))?;
    let last: String = tx
        .query_row(
            "SELECT last_activity_at FROM sessions WHERE key = ?1",
            params![o],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("chat offline read: {e}"))?
        .unwrap_or_default();
    touch(tx, k, &last)?;
    drop_offline_rows(tx, o)
}

/// O's own rows here go (its turns are in K now, there and here).
fn drop_offline_rows(tx: &Transaction<'_>, o: &str) -> Result<(), MoveError> {
    chat_move::clear_chat_rows(tx, o)?;
    tx.execute("DELETE FROM sessions WHERE key = ?1", params![o])
        .map_err(|e| format!("chat offline fold: {e}"))?;
    tx.execute("DELETE FROM chat_homes WHERE session_key = ?1", params![o])
        .map_err(|e| format!("chat offline fold: {e}"))?;
    Ok(())
}

// ── The sync driver ──────────────────────────────────────────────────────

/// How an offline chat's sync stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Synced {
    /// Its turns were added to the chat it continued there (that key).
    Appended(String),
    /// It went there as its own chat (the other Mac had moved on).
    Forked,
    /// The other Mac refused it: it stays this Mac's.
    Kept(String),
    /// Not settled yet (no answer, or no answer to `GET`): the next contact.
    Waiting(String),
    /// Being sent by another call right now, or a turn of it is running.
    Busy,
    /// One slice of its turns landed; it is back here, open, with the rest.
    Sliced,
}

/// The most slices one contact sends of one chat, the first included (the
/// next contact, a minute on, sends more).
pub(crate) const SLICES_PER_CONTACT: usize = 8;

/// Run a Sessions DB call off the async threads.
async fn off_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| Err(format!("chat offline: {e}")))
}

/// The offline chats to sync with `peer`: open, owned here (no home record),
/// the chat they continue still `away` at `peer` (one that came back here
/// waits, never kept: the person continues there or here), with a turn
/// after the base: `(offline_key)`, oldest first.
fn ready(vault: &Path, peer: &str) -> Result<Vec<String>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT o.offline_key FROM chat_offline o WHERE o.state = ?1 AND o.peer = ?2 \
                 AND NOT EXISTS (SELECT 1 FROM chat_homes h WHERE h.session_key = o.offline_key) \
                 AND EXISTS (SELECT 1 FROM chat_homes k WHERE k.session_key = o.session_key \
                             AND k.state = ?3 AND k.peer = o.peer) \
                 AND EXISTS (SELECT 1 FROM turns t WHERE t.session_key = o.offline_key \
                             AND t.ord > o.base_ord) \
                 ORDER BY o.at ASC, o.rowid ASC",
            )
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![OPEN, peer, AWAY], |row| row.get(0))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("chat offline read: {e}"))
    })
}

/// The offline chats whose sync to `peer` landed but isn't folded in yet:
/// `(offline_key, move_id)`.
fn unfolded(vault: &Path, peer: &str) -> Result<Vec<(String, String)>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT o.offline_key, h.move_id FROM chat_offline o \
                 JOIN chat_homes h ON h.session_key = o.offline_key \
                 WHERE o.state = ?1 AND h.state = ?2 AND h.peer = ?3 ORDER BY o.at ASC",
            )
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![OPEN, AWAY, peer], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("chat offline read: {e}"))
    })
}

/// The Macs with an offline chat to sync or fold in (the trigger's retry
/// contacts each).
pub(crate) fn pending_peers(vault: &Path) -> BTreeSet<String> {
    let mut peers = BTreeSet::new();
    let open: Vec<String> = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT DISTINCT peer FROM chat_offline WHERE state = ?1")
            .map_err(|e| format!("chat offline read: {e}"))?;
        let rows = stmt
            .query_map(params![OPEN], |row| row.get(0))
            .map_err(|e| format!("chat offline read: {e}"))?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("chat offline read: {e}"))
    })
    .unwrap_or_default();
    for peer in open {
        let due = ready(vault, &peer).is_ok_and(|r| !r.is_empty())
            || unfolded(vault, &peer).is_ok_and(|u| !u.is_empty());
        if due {
            peers.insert(peer);
        }
    }
    peers
}

/// Where a landed move's chat is, once its answer is folded in ([`landed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Landed {
    /// Under this key on the other Mac.
    There(String),
    /// An offline chat whose answer can't be had now: the next contact
    /// folds it.
    Unplaced,
    /// One slice of an offline chat's turns landed; the chat is back here,
    /// open, with the rest to send.
    Sliced,
    /// The answer to a slice folded already (read again, or racing a later
    /// slice): nothing to do; whoever sent the later slice goes on.
    Stale,
}

/// Once move `move_id` of chat `key` landed on `link`'s Mac: when `key` is
/// an offline chat, ask that Mac how it took it and fold the answer in.
/// [`Landed::There`] the key the chat lives under there (`key` itself
/// unless its turns were added to the chat it continued); `Unplaced` for an
/// offline chat whose answer can't be had now (a continuation owed to it is
/// dropped rather than sent to a key that may not be there); `Sliced` when
/// it carried only part of the turns (the caller sends the next slice).
pub(crate) async fn landed(vault: &Path, link: &dyn MoveLink, move_id: &str, key: &str) -> Landed {
    let (v, k) = (vault.to_path_buf(), key.to_string());
    let offline = off_thread(move || offline_of(&v, &k)).await.ok().flatten();
    let Some(offline) = offline else {
        return Landed::There(key.to_string());
    };
    match offline.state.as_str() {
        APPENDED => return Landed::There(offline.key),
        OPEN => {}
        _ => return Landed::There(key.to_string()),
    }
    let Ok(answer) = link.call(MoveRoute::Status { move_id }).await else {
        return Landed::Unplaced;
    };
    if answer.status != 200 || answer.body["state"] != "imported" {
        return Landed::Unplaced;
    }
    let (Some(outcome), Some(into)) =
        (answer.body["outcome"].as_str(), answer.body["key"].as_str())
    else {
        return Landed::Unplaced;
    };
    let (outcome, into) = (outcome.to_string(), into.to_string());
    let (v, k, id, i) = (
        vault.to_path_buf(),
        key.to_string(),
        move_id.to_string(),
        into.clone(),
    );
    match off_thread(move || fold(&v, &k, &id, &outcome, &i, Utc::now()).map_err(|e| e.message))
        .await
    {
        Ok(Folded::Settled) => {
            changed();
            Landed::There(into)
        }
        Ok(Folded::Sliced) => {
            changed();
            Landed::Sliced
        }
        Ok(Folded::Stale) => Landed::Stale,
        Err(error) => {
            crate::launch::elog(&format!("[chat offline] {key}: {error}"));
            Landed::Unplaced
        }
    }
}

/// The window's event when an offline chat's record changed (a sync folded
/// in, a chat kept here): it reads `chat_offline_rows` and the homes again.
pub(crate) const CHANGED_EVENT: &str = "chat-offline-changed";

fn changed() {
    if let Some(app) = crate::notifications::registered_app() {
        use tauri::Emitter;
        let _ = app.emit(CHANGED_EVENT, ());
    }
}

/// How offline chat `key` stands after a sync step; `landed` after one
/// that landed there (an open chat back home is then one slice further).
async fn standing(vault: &Path, key: &str, landed: bool, why: String) -> Synced {
    let (v, k) = (vault.to_path_buf(), key.to_string());
    let read = off_thread(move || Ok((offline_of(&v, &k)?, chat_move::home_of(&v, &k)?))).await;
    let (offline, home) = read.unwrap_or_default();
    match offline {
        Some(o) if landed && o.state == OPEN && home.is_none() => Synced::Sliced,
        Some(o) if o.state == APPENDED => Synced::Appended(o.key),
        Some(o) if o.state == FORKED => Synced::Forked,
        Some(o) if o.state == KEPT => Synced::Kept(why),
        _ => Synced::Waiting(why),
    }
}

/// On contact with `link`'s Mac (one that lists `appends`): fold in the
/// syncs that landed, then send each offline chat to it that has a turn
/// after its base and nothing queued or running. `from` is this Mac's
/// `machineId`. `(offline_key, how it stands)`.
pub(crate) async fn sync_with(
    vault: &Path,
    link: &dyn MoveLink,
    from: &str,
) -> Vec<(String, Synced)> {
    let mut out = Vec::new();
    let (v, peer) = (vault.to_path_buf(), link.peer().to_string());
    for (key, move_id) in off_thread(move || unfolded(&v, &peer))
        .await
        .unwrap_or_default()
    {
        landed(vault, link, &move_id, &key).await;
        out.push((
            key.clone(),
            standing(vault, &key, true, LANDED.into()).await,
        ));
    }
    let (v, peer) = (vault.to_path_buf(), link.peer().to_string());
    for key in off_thread(move || ready(&v, &peer))
        .await
        .unwrap_or_default()
    {
        // A stretch past one sync's caps goes in slices, one after another.
        let mut synced = Synced::Sliced;
        for _ in 0..SLICES_PER_CONTACT {
            synced = sync_one(vault, link, from, &key).await;
            if synced != Synced::Sliced {
                break;
            }
        }
        out.push((key, synced));
    }
    out
}

const LANDED: &str = "Its sync landed; the answer comes on the next contact.";

/// Send offline chat `key` (or its next slice) to `link`'s Mac once.
async fn sync_one(vault: &Path, link: &dyn MoveLink, from: &str, key: &str) -> Synced {
    let busy = crate::agent_sessions::live_for_key(key).is_some()
        || chat_move::turn_in_flight_now(vault, key);
    if busy {
        return Synced::Busy;
    }
    let move_id = chat_move::new_move_id();
    match chat_move_send::send_move(vault, link, key, from, &move_id).await {
        Ok(Sent::Moved) => {
            landed(vault, link, &move_id, key).await;
            standing(vault, key, true, LANDED.into()).await
        }
        Ok(Sent::Home(why)) => standing(vault, key, false, why).await,
        Ok(Sent::Waiting(why)) => Synced::Waiting(why),
        Ok(Sent::Busy) => Synced::Busy,
        // Refused here for good before anything was sent (a turn with no
        // run id, one turn larger than a sync carries): it stays this Mac's
        // chat. Anything else (archived, its folder gone, a race) may pass:
        // the next contact tries again.
        Err(error) if !matches!(error.code, UNSYNCABLE | "too_large") => {
            Synced::Waiting(error.message)
        }
        Err(error) => {
            let note = if error.code == "too_large" {
                "One of its offline turns is larger than a sync carries (16 MiB), so it stays on this Mac."
                    .to_string()
            } else {
                error.message.clone()
            };
            let (v, k, n) = (vault.to_path_buf(), key.to_string(), note.clone());
            if let Err(e) = off_thread(move || keep(&v, &k, &n, Utc::now())).await {
                crate::launch::elog(&format!("[chat offline] {key}: {e}"));
            }
            changed();
            Synced::Kept(note)
        }
    }
}

#[cfg(test)]
#[path = "chat_offline_tests.rs"]
mod tests;
