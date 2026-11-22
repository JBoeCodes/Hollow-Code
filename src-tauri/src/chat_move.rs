//! M5 PR1 (2026-11-15): a chat moves to another Mac, its history sent once.
//!
//! The peer link (M2) never copies a transcript, with one narrow exception
//! (S32): when a chat moves to another Mac (the MacBook closing, PR2's
//! trigger), its history goes there once, so it keeps going there and the
//! person opens it on either Mac. Design, trigger and the offline path:
//! `outputs/hollow-build-run-2026-11-14/m5-plan.md` in the Hollow vault
//! workspace; wire contract: `docs/phone-api/machines.md` (Moving a chat).
//!
//! **Who owns a chat.** Each Mac's own Sessions DB keeps a home record
//! ([`SCHEMA`], never the vault, never synced). No row: this Mac owns the
//! chat. `leaving`: a move began; this Mac still owns it but runs nothing in
//! it. `away`: the other Mac owns it (`peer` is `movedTo`). `arrived`: this
//! Mac owns it and it came from `peer` (`movedFrom`). The source writes
//! `leaving` before anything is sent ([`begin_move_out`]); the target writes
//! the rows and `arrived` in one transaction ([`import_move`]); only then does
//! the source turn `leaving` into `away` ([`finish_move_out`]). A move that
//! fails rolls back ([`abort_move_out`], and on the target
//! [`cancel_move_in`], whose tombstone refuses a late commit), so a chat is
//! never owned by two Macs or by none. The run queue never runs a `leaving`
//! or `away` chat's turn here ([`admission_refusal`]).
//!
//! **What moves.** The `sessions` row, every turn with its stored
//! transcript, the chat's effort, and the newest native session id, kept in
//! a closed stint with reason `moved` beside a fresh open stint: the next
//! turn on the other Mac starts a fresh native session seeded with the
//! conversation bundle (`context_compiler::Session::fresh`), and a send that
//! still names the old id resets (`chat_resume::ResetReason::Moved`).
//! Neither CLI's own session files move (`~/.claude/projects`,
//! `~/.codex/sessions`): they belong to the other account and folder. Trace
//! events, usage, queue rows, question cards and attachments' files stay.
//!
//! **Transport.** The phone API reads at most 256 KiB a request, so the
//! bundle travels in parts ([`Staging`], memory only and bounded) and is
//! joined, checked against its SHA-256 and imported on the commit.

use crate::session_store::{with_read, with_write};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The home record and the move log, in each Mac's own Sessions DB
/// (`session_store::apply_schema`). PR3 (offline sync, `chat_offline`):
/// `chat_offline` (this Mac's offline continuations of chats it moved away)
/// and `chat_appends` (the syncs this Mac took in, by move id).
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS chat_homes( \
   session_key TEXT PRIMARY KEY, \
   state TEXT NOT NULL, \
   peer TEXT NOT NULL, \
   move_id TEXT NOT NULL, \
   at TEXT NOT NULL \
 ); \
 CREATE TABLE IF NOT EXISTS chat_moves( \
   move_id TEXT PRIMARY KEY, \
   session_key TEXT NOT NULL, \
   direction TEXT NOT NULL, \
   peer TEXT NOT NULL, \
   state TEXT NOT NULL, \
   turns INTEGER NOT NULL DEFAULT 0, \
   prior TEXT, \
   at TEXT NOT NULL \
 ); \
 CREATE INDEX IF NOT EXISTS idx_chat_moves_key ON chat_moves(session_key); \
 CREATE TABLE IF NOT EXISTS chat_offline( \
   offline_key TEXT PRIMARY KEY, \
   session_key TEXT NOT NULL, \
   peer TEXT NOT NULL, \
   base_ord INTEGER NOT NULL, \
   base_run TEXT, \
   state TEXT NOT NULL, \
   note TEXT, \
   at TEXT NOT NULL \
 ); \
 CREATE INDEX IF NOT EXISTS idx_chat_offline_key ON chat_offline(session_key); \
 CREATE TABLE IF NOT EXISTS chat_appends( \
   move_id TEXT PRIMARY KEY, \
   session_key TEXT NOT NULL, \
   offline_key TEXT NOT NULL, \
   peer TEXT NOT NULL, \
   outcome TEXT NOT NULL, \
   turns INTEGER NOT NULL DEFAULT 0, \
   at TEXT NOT NULL \
 ); \
 CREATE TABLE IF NOT EXISTS chat_slices_out( \
   root TEXT NOT NULL, \
   no INTEGER NOT NULL, \
   slice_id TEXT NOT NULL, \
   last_ord INTEGER NOT NULL, \
   turns INTEGER NOT NULL, \
   state TEXT NOT NULL, \
   at TEXT NOT NULL, \
   PRIMARY KEY(root, no) \
 ); \
 CREATE TABLE IF NOT EXISTS chat_slices_in( \
   root TEXT NOT NULL, \
   no INTEGER NOT NULL, \
   slice_id TEXT NOT NULL, \
   session_key TEXT NOT NULL, \
   peer TEXT NOT NULL, \
   last_ord INTEGER NOT NULL, \
   turns INTEGER NOT NULL, \
   payload TEXT NOT NULL, \
   at TEXT NOT NULL, \
   PRIMARY KEY(root, no) \
 );";

/// The bundle's format version.
pub(crate) const BUNDLE_VERSION: u32 = 1;
/// PR3: an offline sync's bundle (`append` set). A receiver from before
/// PR3 refuses it as another version (400), which is no proof, so the
/// sending Mac asks and calls it off as any unanswered move.
pub(crate) const APPEND_VERSION: u32 = 2;
/// A chat too long for one sync moves in slices (`slices` set): earlier
/// slices go ahead under ids of their own and wait on the other Mac; the
/// move's own bundle (its last slice) joins them and imports the whole chat
/// in one transaction. A receiver from before refuses it as another version
/// (400), which is no proof, so the sending Mac asks and calls it off.
pub(crate) const SLICED_VERSION: u32 = 3;
/// At most this many earlier slices wait for one move (so a move carries at
/// most `SLICES_MAX + 1` syncs of turns).
pub(crate) const SLICES_MAX: usize = 16;
/// Earlier slices nobody joined are dropped this long after the last one.
pub(crate) const SLICES_KEPT_FOR: Duration = Duration::from_secs(24 * 3600);
/// At most this many earlier slices wait on one Mac, all moves together.
pub(crate) const SLICES_WAITING_MAX: i64 = 64;
/// At most this many bytes of earlier slices wait on one Mac, all moves
/// together (room for one full move of `SLICES_MAX` slices at one sync's
/// size, with a little over).
pub(crate) const SLICES_WAITING_BYTES: i64 = 20 * BUNDLE_MAX as i64;
/// At most this many turns move; a longer chat gets a handover (PR2).
pub(crate) const TURNS_MAX: usize = 2_000;
/// At most this many bytes of bundle JSON.
pub(crate) const BUNDLE_MAX: usize = 16 * 1024 * 1024;
/// At most this many bundle bytes per part (base64 keeps the request under
/// the phone API's 256 KiB).
pub(crate) const PART_MAX: usize = 160 * 1024;
/// At most this many parts (`BUNDLE_MAX / PART_MAX`, rounded up).
pub(crate) const PARTS_MAX: usize = BUNDLE_MAX.div_ceil(PART_MAX);
/// At most this many moves staged at once on the target.
pub(crate) const STAGED_MAX: usize = 4;
/// At most this many of them from one Mac.
pub(crate) const STAGED_PER_MAC: usize = 2;
/// A staged move nobody committed is dropped after this.
pub(crate) const STAGED_FOR: Duration = Duration::from_secs(600);

/// The home states (`chat_homes.state`).
pub(crate) const LEAVING: &str = "leaving";
pub(crate) const AWAY: &str = "away";
pub(crate) const ARRIVED: &str = "arrived";

/// The move log's states (`chat_moves.state`).
const OUT_LEAVING: &str = "leaving";
const OUT_DONE: &str = "done";
/// Called off while the target's answer is unknown (Keep it here with the
/// other Mac out of reach): the chat still doesn't run here until the
/// target acknowledges the `DELETE` ([`settle_abort`]).
const OUT_ABORTING: &str = "aborting";
/// Released by the person ([`force_release`]): the other Mac is gone.
const OUT_RELEASED: &str = "released";
const OUT_ABORTED: &str = "aborted";
const IN_IMPORTED: &str = "imported";
const IN_CANCELLED: &str = "cancelled";

/// Why a move was refused: a code the wire carries and one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MoveError {
    pub code: &'static str,
    pub message: String,
}

impl MoveError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn bad(message: impl Into<String>) -> Self {
        Self::new("bad_request", message)
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
    /// The HTTP status the phone API answers it with.
    pub(crate) fn status(&self) -> u16 {
        match self.code {
            "bad_request" => 400,
            "forbidden" => 403,
            "not_found" => 404,
            "too_large" => 413,
            "internal" => 500,
            // busy, conflict, not_here, cancelled, started, moved
            _ => 409,
        }
    }
}

impl From<String> for MoveError {
    fn from(error: String) -> Self {
        MoveError::internal(error)
    }
}

/// A move's id: `mv-` and 16 lower-case hex digits.
pub(crate) fn valid_move_id(id: &str) -> bool {
    id.len() == 19
        && id.starts_with("mv-")
        && id[3..]
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// A fresh move id.
pub(crate) fn new_move_id() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("mv-{}", hex_of(&bytes))
}

fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// The SHA-256 of `bytes`, lower-case hex.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_of(&hasher.finalize())
}

// ── The bundle ───────────────────────────────────────────────────────────

/// What a move sends once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoveBundle {
    pub version: u32,
    pub move_id: String,
    pub key: String,
    /// The sending Mac's `machineId`.
    pub from: String,
    pub session: MovedSession,
    pub place: Place,
    /// The newest native session the chat ran in, so a stale id resets.
    #[serde(default)]
    pub native: Option<Native>,
    /// The chat's own effort (the composer's Effort list), if any.
    #[serde(default)]
    pub effort: Option<String>,
    pub turns: Vec<MovedTurn>,
    /// PR3: an offline sync ([`APPEND_VERSION`]): `turns` are the offline
    /// chat's turns after `base`, for the chat `to` there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append: Option<crate::chat_offline::Append>,
    /// A chat moved in slices ([`SLICED_VERSION`]): which slice this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slices: Option<Slices>,
}

/// Which slice of a sliced move a bundle is. An earlier slice (`last` false)
/// has an id of its own and names the move (`root`) it waits for; the move's
/// own bundle (`last`, `root` is its id) says what it expects to find
/// waiting, so a missing or doubled slice refuses the commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Slices {
    pub root: String,
    /// An earlier slice: its place (0 first). The last one: how many
    /// earlier slices wait.
    pub no: u32,
    pub last: bool,
    /// The last one only: the turns in the earlier slices, and the `ord` of
    /// the last of them.
    #[serde(default)]
    pub before_turns: u64,
    #[serde(default)]
    pub before_ord: i64,
}

/// What one sync carries: its turns, the bundle's bytes, and the room for
/// turns in a slice (the rest is the chat's record, folder and base).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caps {
    pub turns: usize,
    pub bundle: usize,
    pub slice_bytes: usize,
}

impl Caps {
    pub(crate) const SYNC: Caps = Caps {
        turns: TURNS_MAX,
        bundle: BUNDLE_MAX,
        slice_bytes: BUNDLE_MAX - 1024 * 1024,
    };
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MovedSession {
    #[serde(default)]
    pub title: Option<String>,
    pub agent: String,
    pub created_at: String,
    pub last_activity_at: String,
    #[serde(default)]
    pub parent_key: Option<String>,
    #[serde(default)]
    pub fork_kind: Option<String>,
}

/// Where the chat runs, as the vault says it (each Mac has its own clone):
/// a workspace by id, else a folder relative to the vault root (empty is the
/// root itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Place {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// The chat ran in the workspace's build worktree (outside the vault):
    /// the receiving Mac must have a live worktree bound to that workspace,
    /// or the chat would run in the vault folder instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Native {
    pub agent: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MovedTurn {
    pub ord: i64,
    #[serde(default)]
    pub run_id: Option<String>,
    pub started_at: String,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub asked: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub asked_by: Option<String>,
    /// The stored transcript as the sending Mac keeps it (an object).
    #[serde(default)]
    pub transcript: Option<Value>,
}

// ── The home record ──────────────────────────────────────────────────────

/// A chat's home record ([`SCHEMA`]'s `chat_homes`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Home {
    pub state: String,
    pub peer: String,
    pub move_id: String,
    pub at: String,
}

pub(crate) fn home_in(conn: &rusqlite::Connection, key: &str) -> Result<Option<Home>, String> {
    conn.query_row(
        "SELECT state, peer, move_id, at FROM chat_homes WHERE session_key = ?1",
        params![key],
        |row| {
            Ok(Home {
                state: row.get(0)?,
                peer: row.get(1)?,
                move_id: row.get(2)?,
                at: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("chat home read: {e}"))
}

/// Chat `key`'s home record, `None` when it never moved.
pub(crate) fn home_of(vault: &Path, key: &str) -> Result<Option<Home>, String> {
    with_read(vault, |conn| home_in(conn, key))
}

/// Every home record (the list rows' `movedTo` / `movedFrom`, and the
/// window's `chat_move_homes`), by key.
pub(crate) fn homes(vault: &Path) -> Result<HashMap<String, Home>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT session_key, state, peer, move_id, at FROM chat_homes")
            .map_err(|e| format!("chat homes read: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Home {
                        state: row.get(1)?,
                        peer: row.get(2)?,
                        move_id: row.get(3)?,
                        at: row.get(4)?,
                    },
                ))
            })
            .map_err(|e| format!("chat homes read: {e}"))?;
        rows.collect::<Result<HashMap<_, _>, _>>()
            .map_err(|e| format!("chat homes read: {e}"))
    })
}

/// One chat's home as the window shows it (`chat_move_homes`): its notice
/// in the chat and the moved badge in the lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct HomeRow {
    pub key: String,
    /// `leaving`, `away` or `arrived` ([`Home::state`]).
    pub state: String,
    /// The other Mac's machine id.
    pub peer: String,
    /// When the record last changed (RFC 3339).
    pub at: String,
    /// `leaving` after Keep it here, waiting for the other Mac's answer
    /// (the move `aborting`).
    pub waiting: bool,
}

/// Every chat with a home record, for the window (`chat_move_homes`).
pub(crate) fn home_rows(vault: &Path) -> Result<Vec<HomeRow>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT h.session_key, h.state, h.peer, h.at, m.state FROM chat_homes h \
                 LEFT JOIN chat_moves m ON m.move_id = h.move_id AND m.direction = 'out' \
                 ORDER BY h.at DESC, h.session_key ASC",
            )
            .map_err(|e| format!("chat homes read: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                let out: Option<String> = row.get(4)?;
                Ok(HomeRow {
                    key: row.get(0)?,
                    state: row.get(1)?,
                    peer: row.get(2)?,
                    at: row.get(3)?,
                    waiting: out.as_deref() == Some(OUT_ABORTING),
                })
            })
            .map_err(|e| format!("chat homes read: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("chat homes read: {e}"))
    })
}

/// Why the run queue won't run chat `key`'s turn here: it is moving to, or
/// now lives on, another Mac. `None` when this Mac owns it (or the record
/// can't be read: a read failure never blocks a chat that never moved).
pub(crate) fn admission_refusal(vault: &Path, key: &str) -> Option<String> {
    let Some(home) = home_of(vault, key).ok().flatten() else {
        // PR3: an offline chat whose turns were added to the chat there.
        return crate::chat_offline::folded_refusal(vault, key);
    };
    match home.state.as_str() {
        LEAVING => Some(
            "This chat is moving to another Mac, so it doesn't run here. If that Mac doesn't answer, choose Keep it here on the chat."
                .into(),
        ),
        AWAY => Some(
            "This chat moved to another Mac and keeps going there. Open it from that Mac.".into(),
        ),
        _ => None,
    }
}

/// Run `f` in one write transaction (`session_store::with_write`): an `Err`
/// rolls it back, and its refusal comes back as it was, not as a string.
pub(crate) fn in_write<T>(
    vault: &Path,
    what: &str,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, MoveError>,
) -> Result<T, MoveError> {
    let mut refusal = None;
    let out = with_write(vault, what, |tx| {
        f(tx).map_err(|error| {
            let message = error.message.clone();
            refusal = Some(error);
            message
        })
    });
    out.map_err(|error| refusal.unwrap_or_else(|| MoveError::internal(error)))
}

// ── The source: begin, finish, abort ─────────────────────────────────────

pub(crate) fn turn_in_flight(tx: &rusqlite::Connection, key: &str) -> Result<bool, String> {
    let queued: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM run_queue WHERE source = 'chat' AND source_id = ?1 \
             AND status IN ('queued', 'claimed', 'running')",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("move queue check: {e}"))?;
    let running: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM turns WHERE session_key = ?1 AND status = 'running'",
            params![key],
            |row| row.get(0),
        )
        .map_err(|e| format!("move turn check: {e}"))?;
    Ok(queued > 0 || running > 0)
}

/// Whether chat `key` has a turn queued, claimed or running here (its run
/// queue rows and turns). A read failure counts as one: never move on it.
pub(crate) fn turn_in_flight_now(vault: &Path, key: &str) -> bool {
    with_read(vault, |conn| turn_in_flight(conn, key)).unwrap_or(true)
}

/// Where `cwd` is, as the vault says it ([`Place`]); refuses the Main
/// agent's folder and a folder outside the vault.
fn place_of(vault: &Path, cwd: &Path) -> Result<Place, MoveError> {
    if let Ok(Some(workspace)) = crate::workspaces::workspace_for_cwd(vault, cwd) {
        if crate::workspaces::is_main_agent(&workspace.id) {
            return Err(MoveError::new(
                "conflict",
                "The Main agent's chat stays on its Mac; each Mac has its own.",
            ));
        }
        // A build workspace's chat runs in its worktree, outside the vault.
        let inside = vault
            .canonicalize()
            .ok()
            .zip(cwd.canonicalize().ok())
            .is_some_and(|(root, here)| here.starts_with(root));
        let worktree = !inside && workspace.build.as_ref().is_some_and(|b| b.exists);
        return Ok(Place {
            workspace_id: Some(workspace.id),
            vault_path: None,
            worktree,
        });
    }
    let root = vault
        .canonicalize()
        .map_err(|e| MoveError::internal(format!("vault path: {e}")))?;
    let here = cwd
        .canonicalize()
        .map_err(|_| MoveError::new("not_here", "This chat's folder is gone, so it can't move."))?;
    let rel = here.strip_prefix(&root).map_err(|_| {
        MoveError::new(
            "not_here",
            "This chat runs outside the vault, so the other Mac has no such folder.",
        )
    })?;
    reserved_folder(rel)?;
    Ok(Place {
        workspace_id: None,
        vault_path: Some(rel.to_string_lossy().into_owned()),
        worktree: false,
    })
}

fn main_refused() -> MoveError {
    MoveError::new(
        "conflict",
        "The Main agent's chat stays on its Mac; each Mac has its own.",
    )
}

/// A vault-relative folder no moved chat may run in: the Main agent's (each
/// Mac has its own) and any dot-folder (`.git`, `.hollow`, ...).
fn reserved_folder(rel: &Path) -> Result<(), MoveError> {
    if rel.starts_with(Path::new("workspaces").join(crate::workspaces::MAIN_AGENT_ID)) {
        return Err(main_refused());
    }
    let dotted = rel.components().any(|c| match c {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    });
    if dotted {
        return Err(MoveError::bad(
            "place.vaultPath can't name a hidden folder.",
        ));
    }
    Ok(())
}

/// The folder `place` names in this Mac's vault, which must exist: a
/// workspace's (its live build worktree when the chat ran in one), or a
/// folder inside the vault, resolved with every link followed
/// (`vault::safe_join`), so a synced symlink can't lead outside it.
fn cwd_for(vault: &Path, place: &Place) -> Result<PathBuf, MoveError> {
    let not_here = |what: &str| {
        MoveError::new(
            "not_here",
            format!("{what} isn't on this Mac yet. Let the vault sync, then move the chat again."),
        )
    };
    let dir = match (&place.workspace_id, &place.vault_path) {
        (Some(id), _) => {
            if crate::workspaces::is_main_agent(id) {
                return Err(main_refused());
            }
            if place.worktree {
                // A build workspace with no live worktree here would fall
                // back to its vault folder: refused instead.
                let bound = crate::workspaces::summary(vault, id)
                    .ok()
                    .and_then(|w| w.build)
                    .is_some_and(|b| b.exists);
                if !bound {
                    return Err(not_here("That workspace's worktree"));
                }
            }
            crate::workspaces::session_dir_for(vault, id)
                .map_err(|_| not_here("That chat's workspace"))?
        }
        (None, Some(rel)) => {
            let rel_path = Path::new(rel);
            if rel_path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(MoveError::bad(
                    "place.vaultPath must be a folder inside the vault.",
                ));
            }
            reserved_folder(rel_path)?;
            let joined = crate::vault::safe_join(vault, rel).map_err(|_| {
                MoveError::bad("place.vaultPath must be a folder inside the vault.")
            })?;
            // safe_join resolves links of an existing path; check the
            // resolved folder is still not a reserved one.
            if let (Ok(root), Ok(real)) = (vault.canonicalize(), joined.canonicalize()) {
                match real.strip_prefix(&root) {
                    Ok(inside) => reserved_folder(inside)?,
                    Err(_) => {
                        return Err(MoveError::bad(
                            "place.vaultPath must be a folder inside the vault.",
                        ))
                    }
                }
            }
            joined
        }
        (None, None) => vault.to_path_buf(),
    };
    if !dir.is_dir() {
        return Err(not_here("That chat's folder"));
    }
    Ok(dir)
}

fn read_bundle(
    tx: &Transaction<'_>,
    vault: &Path,
    key: &str,
    from: &str,
    move_id: &str,
) -> Result<MoveBundle, MoveError> {
    read_cut(tx, vault, key, from, move_id, &Cut::whole(Caps::SYNC)).map(|(bundle, _)| bundle)
}

/// How much of the chat a bundle is read to carry.
pub(crate) struct Cut {
    pub caps: Caps,
    /// Carry only the turns that fit one sync (a chat that doesn't fit
    /// whole), after `after` when set; the rest is for the next slice.
    pub sliced: bool,
    pub after: Option<i64>,
}

impl Cut {
    pub(crate) fn whole(caps: Caps) -> Self {
        Self {
            caps,
            sliced: false,
            after: None,
        }
    }
}

/// [`read_bundle`] under `cut`, and whether turns were left for a later
/// slice (never for a whole read).
fn read_cut(
    tx: &Transaction<'_>,
    vault: &Path,
    key: &str,
    from: &str,
    move_id: &str,
    cut: &Cut,
) -> Result<(MoveBundle, bool), MoveError> {
    let (session, cwd, archived) = tx
        .query_row(
            "SELECT title, agent, created_at, last_activity_at, parent_key, fork_kind, cwd, archived \
             FROM sessions WHERE key = ?1",
            params![key],
            |row| {
                Ok((
                    MovedSession {
                        title: row.get(0)?,
                        agent: row.get(1)?,
                        created_at: row.get(2)?,
                        last_activity_at: row.get(3)?,
                        parent_key: row.get(4)?,
                        fork_kind: row.get(5)?,
                    },
                    row.get::<_, String>(6)?,
                    row.get::<_, bool>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|e| MoveError::internal(format!("move read: {e}")))?
        .ok_or_else(|| MoveError::new("not_found", "That chat isn't in Hollow."))?;
    if archived {
        return Err(MoveError::new(
            "conflict",
            "That chat is archived; unarchive it to move it.",
        ));
    }
    let place = place_of(vault, Path::new(&cwd))?;
    let native = tx
        .query_row(
            "SELECT agent, harness_session_id FROM stints WHERE session_key = ?1 \
             AND harness_session_id IS NOT NULL ORDER BY id DESC LIMIT 1",
            params![key],
            |row| {
                Ok(Native {
                    agent: row.get(0)?,
                    id: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|e| MoveError::internal(format!("move native read: {e}")))?;
    let effort = tx
        .query_row(
            "SELECT effort FROM chat_efforts WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| MoveError::internal(format!("move effort read: {e}")))?
        .as_deref()
        .and_then(canonical_effort);
    // PR3: an offline chat still to sync carries only its turns after its
    // base, so the caps count those; past one sync's caps it goes in slices
    // (`chat_offline::Slice`), each a sync of its own.
    let base = crate::chat_offline::delta_base(tx, key)?;
    let mut slice =
        (base.is_some() || cut.sliced).then(|| crate::chat_offline::Slice::within(cut.caps));
    let after = cut.after.or(base).unwrap_or(i64::MIN);
    let mut more = false;
    let mut stmt = tx
        .prepare(
            "SELECT t.ord, t.run_id, t.started_at, t.ended_at, t.status, t.asked, \
             t.permission_mode, t.effort, t.asked_by, x.payload \
             FROM turns t LEFT JOIN transcripts x ON x.session_key = t.session_key AND x.turn_ord = t.ord \
             WHERE t.session_key = ?1 AND t.ord > ?2 ORDER BY t.ord ASC",
        )
        .map_err(|e| MoveError::internal(format!("move turns read: {e}")))?;
    let rows = stmt
        .query_map(params![key, after], |row| {
            Ok((
                MovedTurn {
                    ord: row.get(0)?,
                    run_id: row.get(1)?,
                    started_at: row.get(2)?,
                    ended_at: row.get(3)?,
                    status: row.get(4)?,
                    asked: row.get(5)?,
                    permission_mode: row.get(6)?,
                    effort: row
                        .get::<_, Option<String>>(7)?
                        .as_deref()
                        .and_then(canonical_effort),
                    asked_by: row.get(8)?,
                    transcript: None,
                },
                row.get::<_, Option<String>>(9)?,
            ))
        })
        .map_err(|e| MoveError::internal(format!("move turns read: {e}")))?;
    let mut turns = Vec::new();
    for row in rows {
        let (mut turn, payload) =
            row.map_err(|e| MoveError::internal(format!("move turn row: {e}")))?;
        turn.transcript = payload.and_then(|p| serde_json::from_str::<Value>(&p).ok());
        if slice.as_mut().is_some_and(|slice| !slice.takes(&turn)) {
            more = true;
            break;
        }
        turns.push(turn);
        if turns.len() > cut.caps.turns {
            return Err(too_large());
        }
    }
    let mut bundle = MoveBundle {
        version: BUNDLE_VERSION,
        move_id: move_id.to_string(),
        key: key.to_string(),
        from: from.to_string(),
        session,
        place,
        native,
        effort,
        turns,
        append: None,
        slices: None,
    };
    // PR3: an offline chat still to sync goes as an append.
    crate::chat_offline::shape_append(tx, &mut bundle)?;
    Ok((bundle, more))
}

pub(crate) fn too_large() -> MoveError {
    MoveError::new(
        "too_large",
        format!(
            "This chat is longer than a move carries ({TURNS_MAX} turns or 16 MiB); it stays on this Mac."
        ),
    )
}

/// The bundle as JSON, refused past [`BUNDLE_MAX`].
pub(crate) fn encode(bundle: &MoveBundle) -> Result<Vec<u8>, MoveError> {
    encode_within(bundle, &Caps::SYNC)
}

/// [`encode`] under `caps` (a sliced move's, tests').
fn encode_within(bundle: &MoveBundle, caps: &Caps) -> Result<Vec<u8>, MoveError> {
    let bytes =
        serde_json::to_vec(bundle).map_err(|e| MoveError::internal(format!("move encode: {e}")))?;
    if bytes.len() > caps.bundle {
        return Err(too_large());
    }
    Ok(bytes)
}

/// Begin moving chat `key` to the Mac `to` (its `machineId`): mark it
/// `leaving` and return what to send, in one transaction (a refusal or a
/// bundle too large changes nothing). `from` is this Mac's `machineId`.
/// Called again with the same `move_id` while it is still leaving, it
/// returns the bundle again (a retry). Refuses a chat with a turn queued or
/// running: the trigger stops it at a safe point first (PR2).
#[cfg(test)]
pub(crate) fn begin_move_out(
    vault: &Path,
    key: &str,
    from: &str,
    to: &str,
    move_id: &str,
    now: DateTime<Utc>,
) -> Result<(MoveBundle, Vec<u8>), MoveError> {
    let (bundle, bytes, _) = begin_inner(vault, key, from, to, move_id, now, None)?;
    Ok((bundle, bytes))
}

/// What a sliced move sends next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Next {
    /// The move's own bundle: the whole chat, or its last slice.
    Move,
    /// An earlier slice, under its own id; the move waits for it to land
    /// ([`slice_landed`]), then asks again.
    Slice(String),
}

/// [`begin_move_out`] for a chat that may be longer than one sync carries
/// (`caps`): one that fits is the same bundle, byte for byte. A longer one
/// goes in slices, each of at most one sync's caps, in order: the first call
/// marks the chat `leaving` and returns the first slice, and every later
/// call (after [`slice_landed`], or again after a restart) returns the next
/// slice not yet landed, then the move's own bundle with the last turns.
/// The progress is kept in `chat_slices_out`, so a restart sends the slice
/// that was out again under its id, never one that landed. The chat stays
/// `leaving` throughout and its turns can't change, so the slices are
/// the same ones however often they are asked for.
pub(crate) fn begin_move_out_sliced(
    vault: &Path,
    key: &str,
    from: &str,
    to: &str,
    move_id: &str,
    caps: Caps,
    now: DateTime<Utc>,
) -> Result<(MoveBundle, Vec<u8>, Next), MoveError> {
    let (bundle, bytes, slice) = begin_inner(vault, key, from, to, move_id, now, Some(caps))?;
    let next = slice.map_or(Next::Move, Next::Slice);
    Ok((bundle, bytes, next))
}

/// The slices of move `root` this Mac sent: `(no, slice id, last ord,
/// turns, landed)`, in order.
fn slices_out(
    tx: &Transaction<'_>,
    root: &str,
) -> Result<Vec<(u32, String, i64, i64, bool)>, String> {
    let mut stmt = tx
        .prepare(
            "SELECT no, slice_id, last_ord, turns, state FROM chat_slices_out WHERE root = ?1 \
             ORDER BY no ASC",
        )
        .map_err(|e| format!("slice log read: {e}"))?;
    let rows = stmt
        .query_map(params![root], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)? == SLICE_LANDED,
            ))
        })
        .map_err(|e| format!("slice log read: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("slice log read: {e}"))
}

const SLICE_SENDING: &str = "sending";
const SLICE_LANDED: &str = "landed";

/// The other Mac took the earlier slice `slice_id` (its commit answered 200).
/// Idempotent.
pub(crate) fn slice_landed(
    vault: &Path,
    slice_id: &str,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let at = crate::session_store::db_time(now);
    with_write(vault, "chat slice landed", |tx| {
        tx.execute(
            "UPDATE chat_slices_out SET state = ?2, at = ?3 WHERE slice_id = ?1",
            params![slice_id, SLICE_LANDED, at],
        )
        .map_err(|e| format!("slice log: {e}"))?;
        Ok(())
    })
    .map_err(MoveError::from)
}

fn begin_inner(
    vault: &Path,
    key: &str,
    from: &str,
    to: &str,
    move_id: &str,
    now: DateTime<Utc>,
    sliced: Option<Caps>,
) -> Result<(MoveBundle, Vec<u8>, Option<String>), MoveError> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(MoveError::bad("That isn't a chat's key."));
    }
    if !valid_move_id(move_id) {
        return Err(MoveError::bad(
            "moveId must be mv- and 16 lower-case hex digits.",
        ));
    }
    if from == to {
        return Err(MoveError::bad("A chat can't move to the Mac it is on."));
    }
    if crate::agent_sessions::live_for_key(key).is_some() {
        return Err(busy());
    }
    let at = crate::session_store::db_time(now);
    in_write(vault, "chat move out", |tx| {
        let home = home_in(tx, key)?;
        let used: Option<String> = tx
            .query_row(
                "SELECT state FROM chat_moves WHERE move_id = ?1",
                params![move_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("move log read: {e}"))?;
        let retry = used.as_deref() == Some(OUT_LEAVING)
            && home
                .as_ref()
                .is_some_and(|h| h.state == LEAVING && h.move_id == move_id);
        if used.is_some() && !retry {
            return Err(MoveError::new(
                "conflict",
                "That move id was used already; start a new move.",
            ));
        }
        match home.as_ref().map(|h| h.state.as_str()) {
            Some(LEAVING) if retry => {}
            Some(LEAVING) => return Err(MoveError::new("busy", "This chat is already moving.")),
            Some(AWAY) => {
                return Err(MoveError::new(
                    "moved",
                    "This chat already moved to another Mac.",
                ))
            }
            _ => {
                if turn_in_flight(tx, key)? {
                    return Err(busy());
                }
            }
        }
        let caps = sliced.unwrap_or(Caps::SYNC);
        // A chat that fits one sync goes whole, as it always did.
        let earlier = slices_out(tx, move_id)?;
        let mut whole = None;
        if earlier.is_empty() {
            let cut = Cut::whole(caps);
            let read = read_cut(tx, vault, key, from, move_id, &cut)
                .and_then(|(bundle, _)| encode_within(&bundle, &caps).map(|bytes| (bundle, bytes)));
            match read {
                Ok(found) => whole = Some(found),
                Err(e)
                    if e.code == "too_large"
                        && sliced.is_some()
                        && crate::chat_offline::delta_base(tx, key)?.is_none() => {}
                Err(e) => return Err(e),
            }
        }
        let (bundle, bytes, slice_id, root_turns) = match whole {
            Some((bundle, bytes)) => {
                let n = bundle.turns.len() as i64;
                (bundle, bytes, None, n)
            }
            None => {
                let landed: Vec<&(u32, String, i64, i64, bool)> =
                    earlier.iter().filter(|s| s.4).collect();
                let out = earlier.iter().find(|s| !s.4);
                let cut = Cut {
                    caps,
                    sliced: true,
                    after: landed.last().map(|s| s.2),
                };
                let (mut bundle, more) = read_cut(tx, vault, key, from, move_id, &cut)?;
                let total: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM turns WHERE session_key = ?1",
                        params![key],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("move turns count: {e}"))?;
                bundle.version = SLICED_VERSION;
                if more {
                    if landed.len() >= SLICES_MAX {
                        return Err(too_large());
                    }
                    let no = landed.len() as u32;
                    let last_ord = bundle.turns.last().map_or(0, |t| t.ord);
                    let slice_id = match out {
                        Some(row) if row.2 == last_ord => row.1.clone(),
                        _ => new_move_id(),
                    };
                    bundle.move_id = slice_id.clone();
                    bundle.native = None;
                    bundle.slices = Some(Slices {
                        root: move_id.to_string(),
                        no,
                        last: false,
                        before_turns: 0,
                        before_ord: 0,
                    });
                    let bytes = encode_within(&bundle, &caps)?;
                    tx.execute(
                        "DELETE FROM chat_slices_out WHERE root = ?1 AND no >= ?2",
                        params![move_id, no],
                    )
                    .map_err(|e| format!("slice log: {e}"))?;
                    tx.execute(
                        "INSERT INTO chat_slices_out(root, no, slice_id, last_ord, turns, state, at) \
                         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            move_id,
                            no,
                            slice_id,
                            last_ord,
                            bundle.turns.len() as i64,
                            SLICE_SENDING,
                            at
                        ],
                    )
                    .map_err(|e| format!("slice log: {e}"))?;
                    (bundle, bytes, Some(slice_id), total)
                } else if landed.is_empty() {
                    // No earlier slice and the whole chat still doesn't
                    // fit: nothing to slice (a turn larger than a sync).
                    return Err(too_large());
                } else {
                    bundle.slices = Some(Slices {
                        root: move_id.to_string(),
                        no: landed.len() as u32,
                        last: true,
                        before_turns: landed.iter().map(|s| s.3 as u64).sum(),
                        before_ord: landed.last().map_or(0, |s| s.2),
                    });
                    let bytes = encode_within(&bundle, &caps)?;
                    (bundle, bytes, None, total)
                }
            }
        };
        if home.as_ref().is_none_or(|h| h.state != LEAVING) {
            let prior = home.as_ref().and_then(|h| serde_json::to_string(h).ok());
            tx.execute(
                    "INSERT INTO chat_moves(move_id, session_key, direction, peer, state, turns, prior, at) \
                     VALUES(?1, ?2, 'out', ?3, ?4, ?5, ?6, ?7)",
                    params![move_id, key, to, OUT_LEAVING, root_turns, prior, at],
                )
                .map_err(|e| format!("move log: {e}"))?;
            tx.execute(
                    "INSERT INTO chat_homes(session_key, state, peer, move_id, at) VALUES(?1, ?2, ?3, ?4, ?5) \
                     ON CONFLICT(session_key) DO UPDATE SET state = ?2, peer = ?3, move_id = ?4, at = ?5",
                    params![key, LEAVING, to, move_id, at],
                )
                .map_err(|e| format!("chat home write: {e}"))?;
        }
        Ok((bundle, bytes, slice_id))
    })
}

fn busy() -> MoveError {
    MoveError::new(
        "busy",
        "This chat has a turn queued or running. It moves once the turn stops.",
    )
}

/// The move `move_id` landed on the other Mac: the chat is `away` here.
/// Idempotent; refuses a move that was aborted.
pub(crate) fn finish_move_out(
    vault: &Path,
    move_id: &str,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let at = crate::session_store::db_time(now);
    with_write(vault, "chat move finish", |tx| {
        let found = out_move(tx, move_id)?;
        let Some((key, state)) = found else {
            return Err(format!("no move {move_id}"));
        };
        match state.as_str() {
            OUT_DONE => Ok(()),
            OUT_LEAVING | OUT_ABORTING => {
                tx.execute(
                    "UPDATE chat_moves SET state = ?2, at = ?3 WHERE move_id = ?1",
                    params![move_id, OUT_DONE, at],
                )
                .map_err(|e| format!("move log: {e}"))?;
                tx.execute(
                    "UPDATE chat_homes SET state = ?3, at = ?4 WHERE session_key = ?1 AND move_id = ?2",
                    params![key, move_id, AWAY, at],
                )
                .map_err(|e| format!("chat home write: {e}"))?;
                drop_slices_out(tx, move_id)?;
                Ok(())
            }
            other => Err(format!("move {move_id} is {other}")),
        }
    })
    .map_err(|e| MoveError::new("conflict", e))
}

/// Forget the slices a settled move sent.
fn drop_slices_out(tx: &Transaction<'_>, root: &str) -> Result<(), String> {
    tx.execute("DELETE FROM chat_slices_out WHERE root = ?1", params![root])
        .map_err(|e| format!("slice log: {e}"))?;
    Ok(())
}

fn out_move(tx: &Transaction<'_>, move_id: &str) -> Result<Option<(String, String)>, String> {
    tx.query_row(
        "SELECT session_key, state FROM chat_moves WHERE move_id = ?1 AND direction = 'out'",
        params![move_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(|e| format!("move log read: {e}"))
}

/// What the other Mac actually answered, the proof an abort needs.
///
/// **Contract for PR2's caller** (the sending side's move driver, never a
/// route handler of this Mac's own server): a `TargetSaid` is built only
/// from a response the peer client received over that Mac's tunnel for this
/// move id, and is passed with that Mac's `machineId` (the paired record the
/// client called), which must be the move's own peer. `CommitRefused` only
/// for a 4xx answer to this move's `commit` (`conflict`, `not_here`,
/// `too_large`, `cancelled`; never `bad_request`, which the staging area
/// also answers when a second delivery's parts are incomplete, so it isn't
/// proof that nothing landed), and only while the move is `leaving`; `Cancelled` only from a 200 answer to this move's `DELETE`;
/// `Kept` only from its 409 `started`. A status of `unknown`, a timeout or
/// a transport error is no answer: send the `DELETE` (its tombstone refuses
/// a late commit), or [`begin_abort`] when the Mac is out of reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetSaid {
    /// The commit itself was answered with a refusal: nothing landed.
    CommitRefused,
    /// The `DELETE` was answered: nothing of the move is there now, and its
    /// tombstone refuses a late commit.
    Cancelled(Cancelled),
    /// The `DELETE` was answered 409 `started`: a turn ran there, the move
    /// stands.
    Kept,
}

/// The move's other Mac, as the move log has it.
fn out_peer(vault: &Path, move_id: &str) -> Result<Option<(String, String)>, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT state, peer FROM chat_moves WHERE move_id = ?1 AND direction = 'out'",
            params![move_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("move log read: {e}"))
    })
}

/// Where a move this Mac sent stands, for what replays a continuation
/// owed to it (`chat_move_trigger::replay_owed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    /// Still `leaving` or `aborting`: the retry settles it.
    Open,
    /// Landed on the other Mac.
    Done,
    /// Called off or released: the chat is this Mac's.
    Gone,
}

/// The state of move `move_id` this Mac sent and the Mac it went to; `None`
/// when this Mac has no such move.
pub(crate) fn settled_out(
    vault: &Path,
    move_id: &str,
) -> Result<Option<(Settled, String)>, String> {
    Ok(out_peer(vault, move_id)?.map(|(state, peer)| {
        let settled = match state.as_str() {
            OUT_DONE => Settled::Done,
            OUT_LEAVING | OUT_ABORTING => Settled::Open,
            _ => Settled::Gone,
        };
        (settled, peer)
    }))
}

/// The answer came from `peer`, the move's own other Mac, or it is refused.
fn answered_by(vault: &Path, move_id: &str, peer: &str) -> Result<String, MoveError> {
    let (state, theirs) = out_peer(vault, move_id)?
        .ok_or_else(|| MoveError::new("not_found", format!("no move {move_id}")))?;
    if theirs != peer {
        return Err(MoveError::new(
            "forbidden",
            "That answer came from another Mac than the move's.",
        ));
    }
    Ok(state)
}

/// The move `move_id` didn't land and its other Mac, `peer`, said so
/// (`said`, see [`TargetSaid`] for when each is true): the chat runs here
/// again, its home record as it was before. `CommitRefused` only from
/// `leaving`; `Cancelled` from `leaving` or `aborting`; `Kept` finishes the
/// move instead. Never on a timeout (a commit that landed would leave two
/// owners). Idempotent.
pub(crate) fn abort_move_out(
    vault: &Path,
    move_id: &str,
    peer: &str,
    said: TargetSaid,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let state = answered_by(vault, move_id, peer)?;
    let from: &[&str] = match said {
        TargetSaid::Kept => return finish_move_out(vault, move_id, now),
        TargetSaid::CommitRefused => &[OUT_LEAVING],
        TargetSaid::Cancelled(_) => &[OUT_LEAVING, OUT_ABORTING],
    };
    if state != OUT_ABORTED && !from.contains(&state.as_str()) {
        return Err(MoveError::new(
            "conflict",
            format!("move {move_id} is {state}; that answer doesn't settle it"),
        ));
    }
    release(vault, move_id, from, OUT_ABORTED, now)?;
    if state == OUT_ABORTING && matches!(said, TargetSaid::Cancelled(_)) {
        // PR3: the person chose Keep it here on an offline chat's sync:
        // it stays this Mac's, never sent as a sync again.
        crate::chat_offline::keep_move(
            vault,
            move_id,
            "Kept on this Mac: you chose Keep it here.",
            now,
        )?;
    }
    Ok(())
}

/// Close move `move_id` from one of `from` as `to` and give the chat its
/// home record back as it was before the move. The home record must still
/// be this move's: if it isn't (changed by hand or by a newer move), the
/// release is refused and nothing changes, rather than closing the move
/// over a home it no longer describes.
fn release(
    vault: &Path,
    move_id: &str,
    from: &[&str],
    to: &str,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let at = crate::session_store::db_time(now);
    with_write(vault, "chat move release", |tx| {
        let Some((key, state)) = out_move(tx, move_id)? else {
            return Err(format!("no move {move_id}"));
        };
        if state == to {
            return Ok(());
        }
        if !from.contains(&state.as_str()) {
            return Err(format!("move {move_id} is {state}"));
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT prior FROM chat_moves WHERE move_id = ?1",
                params![move_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("move log read: {e}"))?;
        tx.execute(
            "UPDATE chat_moves SET state = ?2, at = ?3 WHERE move_id = ?1",
            params![move_id, to, at],
        )
        .map_err(|e| format!("move log: {e}"))?;
        let changed = match prior.and_then(|p| serde_json::from_str::<Home>(&p).ok()) {
            Some(home) => tx.execute(
                "UPDATE chat_homes SET state = ?2, peer = ?3, move_id = ?4, at = ?5 \
                 WHERE session_key = ?1 AND move_id = ?6",
                params![key, home.state, home.peer, home.move_id, home.at, move_id],
            ),
            None => tx.execute(
                "DELETE FROM chat_homes WHERE session_key = ?1 AND move_id = ?2",
                params![key, move_id],
            ),
        }
        .map_err(|e| format!("chat home write: {e}"))?;
        if changed == 0 {
            // The transaction rolls back: the move stays as it was.
            return Err(format!(
                "the chat's home record isn't move {move_id}'s any more; nothing was released"
            ));
        }
        drop_slices_out(tx, move_id)?;
        Ok(())
    })
    .map_err(|e| MoveError::new("conflict", e))
}

/// What [`begin_abort`] left the move as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Aborting {
    /// Being called off: waiting for the other Mac's answer to the `DELETE`.
    Waiting,
    /// Already called off with an answer: the chat is this Mac's.
    Aborted,
}

/// Keep it here while the other Mac can't be reached: the move is marked
/// `aborting`, and the chat still runs nowhere (its home stays `leaving`, so
/// admission refuses it) until the other Mac answers the `DELETE`
/// ([`settle_abort`]). Without this, a commit that landed with its answer
/// lost would leave the chat owned by both Macs. Idempotent for a move
/// aborting or aborted (it says which); a move that finished or was
/// released is refused (`conflict`), so it is never taken for one being
/// called off.
pub(crate) fn begin_abort(
    vault: &Path,
    move_id: &str,
    now: DateTime<Utc>,
) -> Result<Aborting, MoveError> {
    let at = crate::session_store::db_time(now);
    with_write(vault, "chat move aborting", |tx| {
        let Some((_, state)) = out_move(tx, move_id)? else {
            return Err(format!("no move {move_id}"));
        };
        match state.as_str() {
            OUT_LEAVING => tx
                .execute(
                    "UPDATE chat_moves SET state = ?2, at = ?3 WHERE move_id = ?1",
                    params![move_id, OUT_ABORTING, at],
                )
                .map(|_| Aborting::Waiting)
                .map_err(|e| format!("move log: {e}")),
            OUT_ABORTING => Ok(Aborting::Waiting),
            OUT_ABORTED => Ok(Aborting::Aborted),
            other => Err(format!("move {move_id} is {other}; nothing to call off")),
        }
    })
    .map_err(|e| MoveError::new("conflict", e))
}

/// The other Mac, `peer`, answered the `DELETE` of an `aborting` move
/// (`said` is that answer, never a commit refusal): `Kept` finishes the
/// move, the chat `away`; `Cancelled` gives the chat back to this Mac.
/// Refuses a move that isn't aborting (or already settled the same way) and
/// an answer from any other Mac.
pub(crate) fn settle_abort(
    vault: &Path,
    move_id: &str,
    peer: &str,
    said: TargetSaid,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    let state = answered_by(vault, move_id, peer)?;
    let settled = matches!(
        (said, state.as_str()),
        (TargetSaid::Kept, OUT_DONE) | (TargetSaid::Cancelled(_), OUT_ABORTED)
    );
    if settled {
        return Ok(());
    }
    if said == TargetSaid::CommitRefused {
        return Err(MoveError::bad(
            "An aborting move settles on the other Mac's answer to its DELETE.",
        ));
    }
    if state != OUT_ABORTING {
        return Err(MoveError::new(
            "conflict",
            format!("move {move_id} isn't being called off"),
        ));
    }
    match said {
        TargetSaid::Kept => finish_move_out(vault, move_id, now),
        _ => abort_move_out(vault, move_id, peer, said, now),
    }
}

/// Proof that a person confirmed a release ([`force_release`]). Its field is
/// private, so it is made only by [`UserConfirmed::by_the_person`].
///
/// WIRING (PR2): only the Tauri command behind the person's own click
/// (Forget this Mac, and Release here on a stuck chat, each after a confirm
/// dialog) may call `by_the_person`. Never an app action, an agent tool, a
/// phone or peer route, or `hollow serve`; a test fails if any of those
/// modules names it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UserConfirmed(());

impl UserConfirmed {
    pub(crate) fn by_the_person() -> Self {
        UserConfirmed(())
    }
}

/// The person's own release of a chat stuck on a move whose other Mac is
/// gone (forgotten, unpaired, or never coming back): the move is closed as
/// `released` and the chat runs here again, whatever state the move was in
/// (`leaving`, `aborting`, or `done` with the chat `away`). It is the one
/// call that can leave a copy on the other Mac too, so it takes
/// [`UserConfirmed`]. Idempotent.
pub(crate) fn force_release(
    vault: &Path,
    move_id: &str,
    _confirmed: UserConfirmed,
    now: DateTime<Utc>,
) -> Result<(), MoveError> {
    release(
        vault,
        move_id,
        &[OUT_LEAVING, OUT_ABORTING, OUT_DONE],
        OUT_RELEASED,
        now,
    )
}

/// The moves called off while the other Mac was out of reach, waiting for
/// its answer to the `DELETE`: `(move_id, key, peer machineId)`, oldest first.
pub(crate) fn pending_aborts(vault: &Path) -> Result<Vec<(String, String, String)>, String> {
    out_moves_in(vault, OUT_ABORTING)
}

/// The moves still `leaving` (sent, or being sent, with no answer that
/// settled them): `(move_id, key, peer machineId)`, oldest first. PR2's
/// trigger sends each again (the same move id, so a commit that landed
/// answers `already`).
pub(crate) fn leaving_moves(vault: &Path) -> Result<Vec<(String, String, String)>, String> {
    out_moves_in(vault, OUT_LEAVING)
}

/// The moves `leaving` or `aborting` since before `before` (stuck: the
/// other Mac hasn't settled them): `(move_id, key, peer)`, oldest first.
pub(crate) fn open_since(
    vault: &Path,
    before: DateTime<Utc>,
) -> Result<Vec<(String, String, String)>, String> {
    let before = crate::session_store::db_time(before);
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT move_id, session_key, peer FROM chat_moves \
                 WHERE direction = 'out' AND state IN (?1, ?2) AND at < ?3 ORDER BY at ASC, rowid ASC",
            )
            .map_err(|e| format!("move log read: {e}"))?;
        let rows = stmt
            .query_map(params![OUT_LEAVING, OUT_ABORTING, before], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("move log read: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("move log read: {e}"))
    })
}

fn out_moves_in(vault: &Path, state: &str) -> Result<Vec<(String, String, String)>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT move_id, session_key, peer FROM chat_moves \
                 WHERE direction = 'out' AND state = ?1 ORDER BY at ASC, rowid ASC",
            )
            .map_err(|e| format!("move log read: {e}"))?;
        let rows = stmt
            .query_map(params![state], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("move log read: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("move log read: {e}"))
    })
}

/// Every move to `peer` still open: `leaving` or `aborting`, with the
/// chat's home record still that move's. Forget this Mac releases each
/// ([`force_release`]): `(move_id, key)`. A `done` move is not listed: its
/// chat lives and runs on that Mac, so releasing it here would give it two
/// owners; it stays `away` ("moved to a Mac you forgot").
pub(crate) fn open_moves_to(vault: &Path, peer: &str) -> Result<Vec<(String, String)>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT m.move_id, m.session_key FROM chat_moves m \
                 JOIN chat_homes h ON h.session_key = m.session_key AND h.move_id = m.move_id \
                 WHERE m.direction = 'out' AND m.peer = ?1 AND m.state IN (?2, ?3) \
                 ORDER BY m.at ASC, m.rowid ASC",
            )
            .map_err(|e| format!("move log read: {e}"))?;
        let rows = stmt
            .query_map(params![peer, OUT_LEAVING, OUT_ABORTING], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| format!("move log read: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("move log read: {e}"))
    })
}

/// Whether chat `key` could move from this Mac (`from`, its `machineId`)
/// once its turn has stopped: every refusal of [`begin_move_out`] but the
/// turn in flight, checked without writing anything. The trigger asks this
/// before it stops a running turn, so a chat that can't move (the Main
/// agent's, one outside the vault, one too long) keeps running here.
pub(crate) fn movable(vault: &Path, key: &str, from: &str) -> Result<(), MoveError> {
    if !crate::agent_sessions::is_safe_session_key(key) {
        return Err(MoveError::bad("That isn't a chat's key."));
    }
    in_write(vault, "chat move check", |tx| {
        match home_in(tx, key)?.as_ref().map(|h| h.state.as_str()) {
            Some(LEAVING) => return Err(MoveError::new("busy", "This chat is already moving.")),
            Some(AWAY) => {
                return Err(MoveError::new(
                    "moved",
                    "This chat already moved to another Mac.",
                ))
            }
            _ => {}
        }
        // Any well-formed id: nothing is written.
        let id = "mv-0000000000000000";
        let whole = read_bundle(tx, vault, key, from, id).and_then(|bundle| encode(&bundle));
        match whole {
            Ok(_) => Ok(()),
            // Longer than one sync carries: it moves in slices, if its
            // first slice can be built.
            Err(e)
                if e.code == "too_large" && crate::chat_offline::delta_base(tx, key)?.is_none() =>
            {
                let cut = Cut {
                    caps: Caps::SYNC,
                    sliced: true,
                    after: None,
                };
                let (bundle, _) = read_cut(tx, vault, key, from, id, &cut)?;
                encode(&bundle).map(|_| ())
            }
            Err(e) => Err(e),
        }
    })
}

// ── The target: import, cancel, status ──────────────────────────────────

/// What [`import_move`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Imported {
    pub move_id: String,
    /// Where the turns are here (an appended offline sync: the chat it
    /// was added to).
    pub key: String,
    pub turns: usize,
    /// True when this move had already been imported (a retried commit):
    /// nothing changed.
    pub already: bool,
    /// PR3, an offline sync only: `appended` or `forked`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
}

fn in_move(tx: &Transaction<'_>, move_id: &str) -> Result<Option<(String, String, i64)>, String> {
    tx.query_row(
        "SELECT session_key, state, turns FROM chat_moves WHERE move_id = ?1 AND direction = 'in'",
        params![move_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )
    .optional()
    .map_err(|e| format!("move log read: {e}"))
}

/// Turn statuses that aren't finished.
const UNFINISHED: [&str; 5] = ["running", "queued", "claimed", "pending", "starting"];

/// An effort as Hollow names it (`effort::canonical`: a level, `minimal`
/// read as `low`, or ultracode); `None` for a value it doesn't know (a
/// legacy `ultra`, a hand edit). Turns store efforts unvalidated, so a move
/// never refuses a chat for one: the sender normalises and the receiver
/// drops what it can't read.
pub(crate) fn canonical_effort(effort: &str) -> Option<String> {
    crate::effort::canonical(effort).map(str::to_string)
}

fn check_bundle(bundle: &MoveBundle, caller: &str) -> Result<(), MoveError> {
    let version_ok = match (&bundle.append, &bundle.slices) {
        (None, None) => bundle.version == BUNDLE_VERSION,
        (Some(_), None) => bundle.version == APPEND_VERSION,
        (None, Some(_)) => bundle.version == SLICED_VERSION,
        (Some(_), Some(_)) => false,
    };
    if !version_ok {
        return Err(MoveError::new(
            "bad_request",
            "That move comes from another Hollow version; update both Macs.",
        ));
    }
    if !valid_move_id(&bundle.move_id) {
        return Err(MoveError::bad(
            "moveId must be mv- and 16 lower-case hex digits.",
        ));
    }
    if bundle.from != caller {
        return Err(MoveError::new(
            "forbidden",
            "A Mac can only move its own chats here.",
        ));
    }
    if !crate::agent_sessions::is_safe_session_key(&bundle.key) {
        return Err(MoveError::bad("That isn't a chat's key."));
    }
    if crate::agent::AgentKind::from_opt(&bundle.session.agent).is_none() {
        return Err(MoveError::bad("That chat's agent isn't one Hollow runs."));
    }
    if bundle.turns.len() > TURNS_MAX {
        return Err(too_large());
    }
    let mut last = 0;
    for turn in &bundle.turns {
        if turn.ord <= last {
            return Err(MoveError::bad("The turns must be in order, each once."));
        }
        last = turn.ord;
        if turn.transcript.as_ref().is_some_and(|t| !t.is_object()) {
            return Err(MoveError::bad("A turn's transcript must be an object."));
        }
        // Only finished turns move: a turn still running (or a made-up
        // one) would make the chat unmovable and its cancel `started`
        // forever.
        let finished = turn.status.as_deref().is_some_and(|status| {
            !status.is_empty()
                && status.len() <= 32
                && status.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                && !UNFINISHED.contains(&status)
        });
        if !finished {
            return Err(MoveError::bad("Only finished turns move."));
        }
        if turn
            .permission_mode
            .as_deref()
            .is_some_and(|mode| !crate::app_mcp::PERMISSION_MODES.contains(&mode))
        {
            return Err(MoveError::bad(
                "A turn's permission mode isn't one Hollow has.",
            ));
        }
    }
    if let Some(native) = &bundle.native {
        if !crate::agent::is_safe_resume_id(&native.id) {
            return Err(MoveError::bad(
                "That native session id isn't one an agent hands out.",
            ));
        }
    }
    if let Some(append) = &bundle.append {
        crate::chat_offline::check_append(bundle, append)?;
    }
    if let Some(slices) = &bundle.slices {
        check_slices(bundle, slices)?;
    }
    Ok(())
}

/// A sliced move's own checks: the root is a move id (an earlier slice has
/// an id of its own, the last one is the root), the place in the series is
/// in range, and the last one counts what it expects to find waiting.
fn check_slices(bundle: &MoveBundle, slices: &Slices) -> Result<(), MoveError> {
    if !valid_move_id(&slices.root) {
        return Err(MoveError::bad(
            "slices.root must be mv- and 16 lower-case hex digits.",
        ));
    }
    if slices.last {
        if slices.root != bundle.move_id {
            return Err(MoveError::bad("The last slice is the move itself."));
        }
        if slices.no as usize > SLICES_MAX || (slices.no == 0) != (slices.before_turns == 0) {
            return Err(MoveError::bad(
                "slices doesn't describe the earlier slices.",
            ));
        }
    } else {
        if slices.root == bundle.move_id {
            return Err(MoveError::bad("An earlier slice has an id of its own."));
        }
        if slices.no as usize >= SLICES_MAX || bundle.turns.is_empty() {
            return Err(MoveError::bad("slices.no is past the last slice."));
        }
    }
    Ok(())
}

/// Import `bundle`, sent by the paired Mac whose `machineId` is `caller`, in
/// one transaction. A move already imported answers again with `already`
/// and changes nothing; a cancelled one is refused. The key must be new
/// here, or a chat this Mac moved to `caller` earlier (it comes back, and
/// its rows are replaced).
pub(crate) fn import_move(
    vault: &Path,
    bundle: &MoveBundle,
    caller: &str,
    now: DateTime<Utc>,
) -> Result<Imported, MoveError> {
    check_bundle(bundle, caller)?;
    if bundle.append.is_some() {
        // PR3: an offline sync appends to the chat here or forks it.
        return crate::chat_offline::import_append(vault, bundle, caller, now);
    }
    if bundle.slices.as_ref().is_some_and(|s| !s.last) {
        // An earlier slice of a sliced move: kept until the move's own.
        return store_slice(vault, bundle, caller, now, SLICES_WAITING_BYTES);
    }
    // A repeat answers before the place is looked at again (the vault may
    // have changed since the first commit landed).
    let earlier = with_read(vault, |conn| {
        conn.query_row(
            "SELECT session_key, state, turns FROM chat_moves WHERE move_id = ?1 AND direction = 'in'",
            params![bundle.move_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?)),
        )
        .optional()
        .map_err(|e| format!("move log read: {e}"))
    })?;
    match earlier {
        Some((key, state, turns)) if state == IN_IMPORTED => {
            return Ok(Imported {
                move_id: bundle.move_id.clone(),
                key,
                turns: turns as usize,
                already: true,
                outcome: None,
            })
        }
        Some(_) => {
            return Err(MoveError::new(
                "cancelled",
                "That move was called off; start it again.",
            ))
        }
        None => {}
    }
    let cwd = cwd_for(vault, &bundle.place)?;
    let cwd = cwd.to_string_lossy().into_owned();
    let at = crate::session_store::db_time(now);
    in_write(vault, "chat move in", |tx| -> Result<Imported, MoveError> {
        match in_move(tx, &bundle.move_id)? {
            Some((key, state, turns)) if state == IN_IMPORTED => {
                return Ok(Imported {
                    move_id: bundle.move_id.clone(),
                    key,
                    turns: turns as usize,
                    already: true,
                    outcome: None,
                })
            }
            Some(_) => {
                return Err(MoveError::new(
                    "cancelled",
                    "That move was called off; start it again.",
                ))
            }
            None => {}
        }
        // A sliced move: the earlier slices waiting here come first.
        let earlier_turns = match &bundle.slices {
            Some(slices) if slices.last => load_slices(tx, bundle, slices, caller)?,
            _ => Vec::new(),
        };
        let total = earlier_turns.len() + bundle.turns.len();
        let key = bundle.key.as_str();
        let exists: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE key = ?1",
                params![key],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| format!("move exists check: {e}"))?
            > 0;
        if exists {
            let home = home_in(tx, key)?;
            let returning = home
                .as_ref()
                .is_some_and(|h| h.state == AWAY && h.peer == caller);
            if !returning {
                return Err(MoveError::new(
                    "conflict",
                    "A chat with this key is already on this Mac.",
                ));
            }
            clear_chat_rows(tx, key)?;
        }
        let s = &bundle.session;
        tx.execute(
                "INSERT INTO sessions(key, title, agent, cwd, created_at, last_activity_at, turn_count, \
                 parent_key, fork_kind) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(key) DO UPDATE SET title = ?2, agent = ?3, cwd = ?4, created_at = ?5, \
                 last_activity_at = ?6, turn_count = ?7, parent_key = ?8, fork_kind = ?9, \
                 ended_at = NULL, archived = 0",
                params![
                    key,
                    s.title,
                    s.agent,
                    cwd,
                    s.created_at,
                    s.last_activity_at,
                    total as i64,
                    s.parent_key,
                    s.fork_kind
                ],
            )
            .map_err(|e| format!("move session write: {e}"))?;
        for turn in earlier_turns.iter().chain(bundle.turns.iter()) {
            tx.execute(
                    "INSERT INTO turns(session_key, ord, run_id, started_at, ended_at, status, asked, \
                     permission_mode, effort, asked_by) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        key,
                        turn.ord,
                        turn.run_id,
                        turn.started_at,
                        turn.ended_at,
                        turn.status,
                        turn.asked,
                        turn.permission_mode,
                        turn.effort.as_deref().and_then(canonical_effort),
                        turn.asked_by
                    ],
                )
                .map_err(|e| format!("move turn write: {e}"))?;
            if let Some(transcript) = &turn.transcript {
                tx.execute(
                    "INSERT INTO transcripts(session_key, turn_ord, payload) VALUES(?1, ?2, ?3)",
                    params![key, turn.ord, transcript.to_string()],
                )
                .map_err(|e| format!("move transcript write: {e}"))?;
            }
        }
        if let Some(native) = &bundle.native {
            tx.execute(
                    "INSERT INTO stints(session_key, agent, harness_session_id, started_at, ended_at, reason) \
                     VALUES(?1, ?2, ?3, ?4, ?4, ?5)",
                    params![key, native.agent, native.id, at, crate::session_store::MOVED_REASON],
                )
                .map_err(|e| format!("move stint write: {e}"))?;
        }
        tx.execute(
            "INSERT INTO stints(session_key, agent, started_at) VALUES(?1, ?2, ?3)",
            params![key, s.agent, at],
        )
        .map_err(|e| format!("move stint write: {e}"))?;
        match bundle.effort.as_deref().and_then(canonical_effort) {
            Some(effort) => tx.execute(
                "INSERT INTO chat_efforts(key, effort, updated_at) VALUES(?1, ?2, ?3) \
                     ON CONFLICT(key) DO UPDATE SET effort = ?2, updated_at = ?3",
                params![key, effort, at],
            ),
            None => tx.execute("DELETE FROM chat_efforts WHERE key = ?1", params![key]),
        }
        .map_err(|e| format!("move effort write: {e}"))?;
        tx.execute(
                "INSERT INTO chat_homes(session_key, state, peer, move_id, at) VALUES(?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(session_key) DO UPDATE SET state = ?2, peer = ?3, move_id = ?4, at = ?5",
                params![key, ARRIVED, caller, bundle.move_id, at],
            )
            .map_err(|e| format!("chat home write: {e}"))?;
        tx.execute(
            "INSERT INTO chat_moves(move_id, session_key, direction, peer, state, turns, at) \
                 VALUES(?1, ?2, 'in', ?3, ?4, ?5, ?6)",
            params![bundle.move_id, key, caller, IN_IMPORTED, total as i64, at],
        )
        .map_err(|e| format!("move log: {e}"))?;
        tx.execute(
            "DELETE FROM chat_slices_in WHERE root = ?1",
            params![bundle.move_id],
        )
        .map_err(|e| format!("slice drop: {e}"))?;
        Ok(Imported {
            move_id: bundle.move_id.clone(),
            key: key.to_string(),
            turns: total,
            already: false,
            outcome: None,
        })
    })
}

// ── Earlier slices of a sliced move, waiting on the target ──────────────

/// Drop earlier slices nobody joined within [`SLICES_KEPT_FOR`] (the retry
/// loop calls it, so a stash left by a move that never finished goes even
/// when no other slice arrives). How many went.
pub(crate) fn prune_slices_in(vault: &Path, now: DateTime<Utc>) -> Result<usize, String> {
    let kept = chrono::Duration::from_std(SLICES_KEPT_FOR).unwrap_or_default();
    let cutoff = crate::session_store::db_time(now - kept);
    with_write(vault, "chat slice prune", |tx| {
        tx.execute("DELETE FROM chat_slices_in WHERE at < ?1", params![cutoff])
            .map_err(|e| format!("slice prune: {e}"))
    })
}

/// At most this many turns a sliced move imports in all.
pub(crate) const SLICED_TURNS_MAX: usize = (SLICES_MAX + 1) * TURNS_MAX;

/// Keep an earlier slice of move `slices.root` until the move's own bundle
/// comes. Idempotent for the same slice id; the slices must come in order,
/// each after the last turn of the one before it. Nothing of the chat is
/// visible here until the move imports: its rows are written only then.
fn store_slice(
    vault: &Path,
    bundle: &MoveBundle,
    caller: &str,
    now: DateTime<Utc>,
    bytes_max: i64,
) -> Result<Imported, MoveError> {
    let Some(slices) = bundle.slices.as_ref() else {
        return Err(MoveError::bad("That isn't a slice."));
    };
    let at = crate::session_store::db_time(now);
    let kept = chrono::Duration::from_std(SLICES_KEPT_FOR).unwrap_or_default();
    let cutoff = crate::session_store::db_time(now - kept);
    let staged = |already: bool| Imported {
        move_id: bundle.move_id.clone(),
        key: bundle.key.clone(),
        turns: bundle.turns.len(),
        already,
        outcome: Some("staged"),
    };
    in_write(
        vault,
        "chat slice in",
        |tx| -> Result<Imported, MoveError> {
            if let Some((_, state, _)) = in_move(tx, &slices.root)? {
                return Err(if state == IN_IMPORTED {
                    MoveError::new("conflict", "That move landed already.")
                } else {
                    MoveError::new("cancelled", "That move was called off; start it again.")
                });
            }
            let key = bundle.key.as_str();
            let exists = tx
                .query_row(
                    "SELECT COUNT(*) FROM sessions WHERE key = ?1",
                    params![key],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| format!("move exists check: {e}"))?
                > 0;
            if exists
                && !home_in(tx, key)?
                    .as_ref()
                    .is_some_and(|h| h.state == AWAY && h.peer == caller)
            {
                return Err(MoveError::new(
                    "conflict",
                    "A chat with this key is already on this Mac.",
                ));
            }
            tx.execute("DELETE FROM chat_slices_in WHERE at < ?1", params![cutoff])
                .map_err(|e| format!("slice prune: {e}"))?;
            let rows: Vec<(u32, String, String, String, i64)> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT no, slice_id, peer, session_key, last_ord FROM chat_slices_in \
                     WHERE root = ?1 ORDER BY no ASC",
                    )
                    .map_err(|e| format!("slice read: {e}"))?;
                let found = stmt
                    .query_map(params![slices.root], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    })
                    .map_err(|e| format!("slice read: {e}"))?;
                found
                    .collect::<Result<_, _>>()
                    .map_err(|e| format!("slice read: {e}"))?
            };
            if rows.iter().any(|r| r.2 != caller) {
                return Err(MoveError::new("forbidden", "That move isn't this Mac's."));
            }
            if rows
                .iter()
                .any(|r| r.0 == slices.no && r.1 == bundle.move_id)
            {
                return Ok(staged(true));
            }
            // Another slice under this place (the sender began again): it and
            // the ones after it go, and this one takes its place.
            let before: Vec<&(u32, String, String, String, i64)> =
                rows.iter().filter(|r| r.0 < slices.no).collect();
            if before.len() != slices.no as usize {
                return Err(MoveError::new(
                    "conflict",
                    "An earlier slice of this move isn't here, so it can't go on.",
                ));
            }
            let first = bundle.turns.first().map_or(0, |t| t.ord);
            if before.last().is_some_and(|r| first <= r.4) || before.iter().any(|r| r.3 != key) {
                return Err(MoveError::new(
                    "conflict",
                    "This slice doesn't follow the earlier one.",
                ));
            }
            tx.execute(
                "DELETE FROM chat_slices_in WHERE root = ?1 AND no >= ?2",
                params![slices.root, slices.no],
            )
            .map_err(|e| format!("slice drop: {e}"))?;
            let waiting: i64 = tx
                .query_row("SELECT COUNT(*) FROM chat_slices_in", [], |row| row.get(0))
                .map_err(|e| format!("slice read: {e}"))?;
            let payload = serde_json::to_string(&bundle.turns)
                .map_err(|e| MoveError::internal(format!("slice encode: {e}")))?;
            let held: i64 = tx
                .query_row(
                    "SELECT COALESCE(SUM(LENGTH(payload)), 0) FROM chat_slices_in",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("slice read: {e}"))?;
            if waiting >= SLICES_WAITING_MAX || held + payload.len() as i64 > bytes_max {
                return Err(MoveError::new(
                    "busy",
                    "This Mac is taking other chats. Try again in a moment.",
                ));
            }
            tx.execute(
            "INSERT INTO chat_slices_in(root, no, slice_id, session_key, peer, last_ord, turns, payload, at) \
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                slices.root,
                slices.no,
                bundle.move_id,
                key,
                caller,
                bundle.turns.last().map_or(0, |t| t.ord),
                bundle.turns.len() as i64,
                payload,
                at
            ],
        )
        .map_err(|e| format!("slice write: {e}"))?;
            Ok(staged(false))
        },
    )
}

/// The earlier slices waiting for the sliced move `bundle` (its last
/// slice), in order and checked: exactly the ones it counts, from the same
/// Mac and chat, their turns in order with no turn twice, as many turns as
/// it expects and its own after them. Otherwise refused (`conflict`) and
/// nothing is written: the sender calls the move off.
fn load_slices(
    tx: &Transaction<'_>,
    bundle: &MoveBundle,
    slices: &Slices,
    caller: &str,
) -> Result<Vec<MovedTurn>, MoveError> {
    let mut stmt = tx
        .prepare(
            "SELECT no, session_key, peer, last_ord, turns, payload FROM chat_slices_in \
             WHERE root = ?1 ORDER BY no ASC",
        )
        .map_err(|e| format!("slice read: {e}"))?;
    let found = stmt
        .query_map(params![bundle.move_id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("slice read: {e}"))?;
    let rows = found
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("slice read: {e}"))?;
    let missing = || {
        MoveError::new(
            "conflict",
            "The earlier slices of this move aren't all here, so it can't land.",
        )
    };
    if rows.len() != slices.no as usize {
        return Err(missing());
    }
    let mut turns: Vec<MovedTurn> = Vec::new();
    for (i, (no, key, peer, last_ord, count, payload)) in rows.iter().enumerate() {
        if *no as usize != i || peer != caller || *key != bundle.key {
            return Err(missing());
        }
        let part: Vec<MovedTurn> = serde_json::from_str(payload).map_err(|_| missing())?;
        let ordered = part.windows(2).all(|w| w[0].ord < w[1].ord)
            && part
                .first()
                .is_some_and(|t| turns.last().is_none_or(|before| before.ord < t.ord));
        if part.len() as i64 != *count || part.last().map(|t| t.ord) != Some(*last_ord) || !ordered
        {
            return Err(missing());
        }
        turns.extend(part);
    }
    let follows = bundle
        .turns
        .first()
        .is_none_or(|t| turns.last().is_none_or(|before| before.ord < t.ord));
    if turns.len() as u64 != slices.before_turns
        || turns.last().map_or(0, |t| t.ord) != slices.before_ord
        || !follows
    {
        return Err(missing());
    }
    if turns.len() + bundle.turns.len() > SLICED_TURNS_MAX {
        return Err(too_large());
    }
    Ok(turns)
}

/// The per-chat rows keyed by session key that a chat coming back (or an
/// import called off) replaces: its turns, transcripts, stints, trace
/// events, question cards, derived task state, step spine and feedback.
/// Its effort goes too ([`clear_chat_rows`]). The usage ledger and the audit
/// record are history of what this Mac ran and stay, as they do when a chat
/// is archived. `delegations` are keyed by run id, not chat: a chat that comes back keeps
/// its old ones, which name runs of turns it still has (harmless, and the
/// Subagents tab reads them by run).
pub(crate) const CHAT_TABLES: [&str; 8] = [
    "turns",
    "transcripts",
    "stints",
    "events",
    "interactions",
    "derived_state",
    "steps",
    "turn_feedback",
];

pub(crate) fn clear_chat_rows(tx: &Transaction<'_>, key: &str) -> Result<(), String> {
    for table in CHAT_TABLES {
        tx.execute(
            &format!("DELETE FROM {table} WHERE session_key = ?1"),
            params![key],
        )
        .map_err(|e| format!("move clear {table}: {e}"))?;
    }
    // Keyed by `key`, not `session_key`.
    tx.execute("DELETE FROM chat_efforts WHERE key = ?1", params![key])
        .map_err(|e| format!("move clear chat_efforts: {e}"))?;
    Ok(())
}

/// At most this many tombstones (a `DELETE` of a move that never landed)
/// are kept per Mac; older ones go. A commit that late is long expired
/// from staging ([`STAGED_FOR`]) anyway.
pub(crate) const TOMBSTONES_MAX: i64 = 200;

/// What [`cancel_move_in`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Cancelled {
    /// Nothing had landed; a late commit is refused from now on.
    NotImported,
    /// The import is undone: the chat is the sending Mac's alone again.
    Removed,
    /// Already called off.
    Already,
}

/// Call off move `move_id` from `caller`: before its commit, a tombstone
/// refuses the commit; after it, the imported chat goes again, unless a turn
/// already ran or waits here (`started`: the sending Mac must finish
/// instead, so the chat is never owned by none). A chat that came back keeps
/// its rows and is `away` again (they are the same history and more).
pub(crate) fn cancel_move_in(
    vault: &Path,
    move_id: &str,
    caller: &str,
    now: DateTime<Utc>,
) -> Result<Cancelled, MoveError> {
    if !valid_move_id(move_id) {
        return Err(MoveError::bad(
            "moveId must be mv- and 16 lower-case hex digits.",
        ));
    }
    staging(vault).drop_move(move_id);
    let at = crate::session_store::db_time(now);
    in_write(
        vault,
        "chat move cancel",
        |tx| -> Result<Cancelled, MoveError> {
            // Earlier slices of this move waiting here go with it.
            tx.execute(
                "DELETE FROM chat_slices_in WHERE root = ?1",
                params![move_id],
            )
            .map_err(|e| format!("slice drop: {e}"))?;
            // PR3: turns an offline sync added to a chat here stand (an
            // append, or an answer naming a fork an earlier sync made).
            if crate::chat_offline::sync_stands(tx, move_id)? {
                return Err(MoveError::new(
                    "started",
                    "Those turns were added to the chat here, so the sync stands.",
                ));
            }
            let found: Option<(String, String, String, i64, Option<String>)> = tx
                .query_row(
                    "SELECT session_key, state, peer, turns, prior FROM chat_moves \
                     WHERE move_id = ?1 AND direction = 'in'",
                    params![move_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| format!("move log read: {e}"))?;
            let Some((key, state, peer, turns, _)) = found else {
                tx.execute(
                    "INSERT INTO chat_moves(move_id, session_key, direction, peer, state, at) \
                     VALUES(?1, '', 'in', ?2, ?3, ?4)",
                    params![move_id, caller, IN_CANCELLED, at],
                )
                .map_err(|e| format!("move log: {e}"))?;
                tx.execute(
                    "DELETE FROM chat_moves WHERE direction = 'in' AND state = ?2 AND session_key = '' \
                     AND peer = ?1 AND move_id NOT IN (SELECT move_id FROM chat_moves \
                     WHERE direction = 'in' AND state = ?2 AND session_key = '' AND peer = ?1 \
                     ORDER BY at DESC, rowid DESC LIMIT ?3)",
                    params![caller, IN_CANCELLED, TOMBSTONES_MAX],
                )
                .map_err(|e| format!("move log prune: {e}"))?;
                return Ok(Cancelled::NotImported);
            };
            if peer != caller {
                return Err(MoveError::new("forbidden", "That move isn't this Mac's."));
            }
            if state == IN_CANCELLED {
                return Ok(Cancelled::Already);
            }
            let now_turns: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM turns WHERE session_key = ?1",
                    params![key],
                    |row| row.get(0),
                )
                .map_err(|e| format!("move cancel read: {e}"))?;
            if now_turns > turns || turn_in_flight(tx, &key)? {
                return Err(MoveError::new(
                    "started",
                    "The chat already ran here, so the move stands.",
                ));
            }
            let home = home_in(tx, &key)?;
            if !home
                .as_ref()
                .is_some_and(|h| h.move_id == move_id && h.state == ARRIVED)
            {
                return Err(MoveError::new(
                    "started",
                    "The chat moved on from here, so the move stands.",
                ));
            }
            // Was it a chat coming back? Then this Mac had sent it to the
            // caller before: it keeps the rows and is away again.
            let returned: Option<String> = tx
                .query_row(
                    "SELECT move_id FROM chat_moves WHERE session_key = ?1 AND direction = 'out' \
                     AND state = ?2 AND peer = ?3 ORDER BY at DESC LIMIT 1",
                    params![key, OUT_DONE, caller],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("move log read: {e}"))?;
            match returned {
                Some(out_id) => {
                    tx.execute(
                        "UPDATE chat_homes SET state = ?2, peer = ?3, move_id = ?4, at = ?5 \
                         WHERE session_key = ?1",
                        params![key, AWAY, caller, out_id, at],
                    )
                    .map_err(|e| format!("chat home write: {e}"))?;
                }
                None => {
                    clear_chat_rows(tx, &key)?;
                    tx.execute("DELETE FROM sessions WHERE key = ?1", params![key])
                        .map_err(|e| format!("move cancel session: {e}"))?;
                    tx.execute(
                        "DELETE FROM chat_homes WHERE session_key = ?1",
                        params![key],
                    )
                    .map_err(|e| format!("move cancel home: {e}"))?;
                }
            }
            tx.execute(
                "UPDATE chat_moves SET state = ?2, at = ?3 WHERE move_id = ?1",
                params![move_id, IN_CANCELLED, at],
            )
            .map_err(|e| format!("move log: {e}"))?;
            // A forked offline sync undone: its record goes with it.
            tx.execute(
                "DELETE FROM chat_appends WHERE move_id = ?1",
                params![move_id],
            )
            .map_err(|e| format!("move log: {e}"))?;
            Ok(Cancelled::Removed)
        },
    )
}

/// A move as the target sees it (`GET /v1/machines/moves/{id}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MoveStatus {
    pub move_id: String,
    /// `receiving` (parts staged, no commit yet), `imported`, `cancelled`, or
    /// `unknown` (nothing here: nothing landed).
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The parts staged so far while receiving.
    pub parts: usize,
    /// PR3, an offline sync taken in: `appended` or `forked` (`key` is
    /// where its turns are).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// Move `move_id` from `caller` as this Mac has it.
pub(crate) fn move_status(
    vault: &Path,
    move_id: &str,
    caller: &str,
) -> Result<MoveStatus, MoveError> {
    if !valid_move_id(move_id) {
        return Err(MoveError::bad(
            "moveId must be mv- and 16 lower-case hex digits.",
        ));
    }
    let found: Option<(String, String, String)> = with_read(vault, |conn| {
        conn.query_row(
            "SELECT session_key, state, peer FROM chat_moves WHERE move_id = ?1 AND direction = 'in'",
            params![move_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("move log read: {e}"))
    })?;
    let mut status = MoveStatus {
        move_id: move_id.to_string(),
        state: "unknown",
        key: None,
        parts: 0,
        outcome: None,
    };
    // PR3: an offline sync this Mac took in.
    let synced = with_read(vault, |conn| crate::chat_offline::outcome_in(conn, move_id))?;
    if let Some((outcome, key, peer)) = synced {
        if peer != caller {
            return Err(MoveError::new("not_found", "Hollow has no such move."));
        }
        status.state = "imported";
        status.key = Some(key);
        status.outcome = Some(outcome);
        return Ok(status);
    }
    match found {
        Some((_, _, peer)) if peer != caller => {
            return Err(MoveError::new("not_found", "Hollow has no such move."))
        }
        Some((key, state, _)) => {
            status.state = if state == IN_IMPORTED {
                "imported"
            } else {
                "cancelled"
            };
            status.key = Some(key).filter(|k| !k.is_empty());
        }
        None => {
            let parts = staging(vault).staged_parts(move_id, caller);
            if parts > 0 {
                status.state = "receiving";
                status.parts = parts;
            }
        }
    }
    Ok(status)
}

// ── Staging the parts ────────────────────────────────────────────────────

struct Staged {
    caller: String,
    of: usize,
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
    since: Instant,
}

/// The parts of the moves a vault is receiving ([`staging`]), in memory
/// only: at most [`STAGED_MAX`] moves ([`STAGED_PER_MAC`] from one Mac) of at most
/// [`BUNDLE_MAX`] bytes each, dropped [`STAGED_FOR`] after their last part.
/// Expired moves are swept on every part, commit, status and cancel rather
/// than on a timer: the caps bound the memory a forgotten move holds (at
/// most 64 MiB) until the next call.
#[derive(Default)]
pub(crate) struct Staging {
    moves: Mutex<HashMap<String, Staged>>,
}

/// The staging area of the vault `vault` receives moves into: one per
/// vault this process serves. The app serves one vault, so its caps hold
/// for the whole process as before; each test's rig has a vault (and so a
/// staging area) of its own, so moves in tests running alongside never take
/// each other's places ("busy"). An area nobody holds that has nothing
/// staged is let go here, so the map keeps only areas in use. Parts staged
/// under a vault path the app has since left (Move Hollow home) aren't
/// found by a commit at the new path; that `not_found` proves nothing to
/// the sender, so the move waits and the next contact sends it again, as
/// after a restart.
pub(crate) fn staging(vault: &Path) -> Arc<Staging> {
    static AREAS: OnceLock<Mutex<HashMap<PathBuf, Arc<Staging>>>> = OnceLock::new();
    let mut areas = AREAS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    areas.retain(|at, area| at == vault || Arc::strong_count(area) > 1 || !area.is_empty());
    areas.entry(vault.to_path_buf()).or_default().clone()
}

/// A part's request body: `{"of": N, "data": base64}`.
#[derive(Debug, Deserialize)]
pub(crate) struct PartBody {
    pub of: Option<usize>,
    pub data: Option<String>,
}

/// A commit's request body: `{"parts": N, "sha256": hex}`.
#[derive(Debug, Deserialize)]
pub(crate) struct CommitBody {
    pub parts: Option<usize>,
    pub sha256: Option<String>,
}

/// Split `bytes` into the parts' request bodies (the sending Mac, PR2).
pub(crate) fn part_bodies(bytes: &[u8]) -> Vec<Value> {
    let chunks: Vec<&[u8]> = if bytes.is_empty() {
        vec![bytes]
    } else {
        bytes.chunks(PART_MAX).collect()
    };
    let of = chunks.len();
    chunks
        .into_iter()
        .map(|chunk| {
            serde_json::json!({
                "of": of,
                "data": base64::engine::general_purpose::STANDARD.encode(chunk),
            })
        })
        .collect()
}

impl Staging {
    fn sweep(moves: &mut HashMap<String, Staged>, now: Instant) {
        moves.retain(|_, staged| now.duration_since(staged.since) < STAGED_FOR);
    }

    /// Keep part `n` (0-based) of `of` for move `move_id` from `caller`.
    pub(crate) fn put(
        &self,
        move_id: &str,
        caller: &str,
        n: usize,
        body: &PartBody,
        now: Instant,
    ) -> Result<usize, MoveError> {
        if !valid_move_id(move_id) {
            return Err(MoveError::bad(
                "moveId must be mv- and 16 lower-case hex digits.",
            ));
        }
        let of = body
            .of
            .filter(|of| (1..=PARTS_MAX).contains(of))
            .ok_or_else(|| {
                MoveError::bad(format!("of must be a whole number from 1 to {PARTS_MAX}."))
            })?;
        if n >= of {
            return Err(MoveError::bad(
                "The part number is past the move's last part.",
            ));
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(body.data.as_deref().unwrap_or("").as_bytes())
            .map_err(|_| MoveError::bad("data must be base64."))?;
        if data.len() > PART_MAX {
            return Err(MoveError::new(
                "too_large",
                "A part carries at most 160 KiB.",
            ));
        }
        let mut moves = self.moves.lock().unwrap_or_else(|e| e.into_inner());
        Self::sweep(&mut moves, now);
        if !moves.contains_key(move_id) {
            let mine = moves.values().filter(|m| m.caller == caller).count();
            if moves.len() >= STAGED_MAX || mine >= STAGED_PER_MAC {
                return Err(MoveError::new(
                    "busy",
                    "This Mac is taking other chats. Try again in a moment.",
                ));
            }
        }
        let staged = moves.entry(move_id.to_string()).or_insert_with(|| Staged {
            caller: caller.to_string(),
            of,
            parts: vec![None; of],
            bytes: 0,
            since: now,
        });
        if staged.caller != caller {
            return Err(MoveError::new("forbidden", "That move isn't this Mac's."));
        }
        if staged.of != of {
            return Err(MoveError::bad("of changed during the move."));
        }
        let old = staged.parts[n].as_ref().map_or(0, Vec::len);
        let bytes = staged.bytes - old + data.len();
        if bytes > BUNDLE_MAX {
            moves.remove(move_id);
            return Err(too_large());
        }
        staged.bytes = bytes;
        staged.parts[n] = Some(data);
        // A move still sending is kept: the clock runs from its last part.
        staged.since = now;
        Ok(staged.parts.iter().filter(|p| p.is_some()).count())
    }

    /// Join move `move_id`'s parts, checked against `sha256` (lower-case
    /// hex) and the part count, and let them go. A count or digest that
    /// doesn't match keeps the parts (the sender may resend one).
    pub(crate) fn take(
        &self,
        move_id: &str,
        caller: &str,
        body: &CommitBody,
    ) -> Result<Vec<u8>, MoveError> {
        let mut moves = self.moves.lock().unwrap_or_else(|e| e.into_inner());
        Self::sweep(&mut moves, Instant::now());
        let staged = moves
            .get(move_id)
            .filter(|s| s.caller == caller)
            .ok_or_else(|| MoveError::new("not_found", "No parts of that move are here."))?;
        if body.parts != Some(staged.of) {
            return Err(MoveError::bad("parts doesn't match the parts sent."));
        }
        if staged.parts.iter().any(Option::is_none) {
            return Err(MoveError::bad("Some parts of the move haven't arrived."));
        }
        let joined: Vec<u8> = staged.parts.iter().flatten().flatten().copied().collect();
        let want = body
            .sha256
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if sha256_hex(&joined) != want {
            return Err(MoveError::bad(
                "The move's SHA-256 doesn't match its parts.",
            ));
        }
        moves.remove(move_id);
        Ok(joined)
    }

    /// How many parts of `move_id` from `caller` are staged.
    pub(crate) fn staged_parts(&self, move_id: &str, caller: &str) -> usize {
        let mut moves = self.moves.lock().unwrap_or_else(|e| e.into_inner());
        Self::sweep(&mut moves, Instant::now());
        moves
            .get(move_id)
            .filter(|s| s.caller == caller)
            .map_or(0, |s| s.parts.iter().filter(|p| p.is_some()).count())
    }

    /// Whether no move is staged here (expired ones count as gone). An
    /// area busy right now (a `take` joining a large move) counts as in
    /// use, so [`staging`] never waits on it while holding every vault's map.
    fn is_empty(&self) -> bool {
        let mut moves = match self.moves.try_lock() {
            Ok(moves) => moves,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return false,
        };
        Self::sweep(&mut moves, Instant::now());
        moves.is_empty()
    }

    /// Forget move `move_id`'s parts.
    pub(crate) fn drop_move(&self, move_id: &str) {
        let mut moves = self.moves.lock().unwrap_or_else(|e| e.into_inner());
        moves.remove(move_id);
        Self::sweep(&mut moves, Instant::now());
    }
}

/// The commit: join the staged parts, read the bundle and import it.
pub(crate) fn commit(
    vault: &Path,
    move_id: &str,
    caller: &str,
    body: &CommitBody,
    now: DateTime<Utc>,
) -> Result<Imported, MoveError> {
    if !valid_move_id(move_id) {
        return Err(MoveError::bad(
            "moveId must be mv- and 16 lower-case hex digits.",
        ));
    }
    // A retried commit whose parts are gone: the first one may have landed.
    if staging(vault).staged_parts(move_id, caller) == 0 {
        let status = move_status(vault, move_id, caller)?;
        if status.state == "imported" {
            if status.outcome.is_some() {
                // PR3: an offline sync taken in already.
                return crate::chat_offline::already(vault, move_id);
            }
            return Ok(Imported {
                move_id: move_id.to_string(),
                key: status.key.unwrap_or_default(),
                turns: imported_turns(vault, move_id)?,
                already: true,
                outcome: None,
            });
        }
    }
    let bytes = staging(vault).take(move_id, caller, body)?;
    let bundle: MoveBundle = serde_json::from_slice(&bytes)
        .map_err(|_| MoveError::bad("The move isn't the JSON Hollow expects."))?;
    if bundle.move_id != move_id {
        return Err(MoveError::bad("The move's id doesn't match its route."));
    }
    import_move(vault, &bundle, caller, now)
}

fn imported_turns(vault: &Path, move_id: &str) -> Result<usize, String> {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT turns FROM chat_moves WHERE move_id = ?1 AND direction = 'in'",
            params![move_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|n| n as usize)
        .map_err(|e| format!("move log read: {e}"))
    })
}

#[cfg(test)]
#[path = "chat_move_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "chat_move_slices_tests.rs"]
mod slices_tests;
