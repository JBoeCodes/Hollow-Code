//! The chat-row gate (Paperclip PR 7b; split out of `chat.rs` by ST11,
//! 2026-10-14, no behaviour change): which chat rows the run-queue pump
//! holds per vault. `chat.rs` re-exports [`ChatRowGuard`] and
//! [`active_chat_rows`], so callers keep their `chat::` paths;
//! [`with_active_rows`] is how `chat::accept_turn` holds the gate across its
//! check, repair and enqueue.

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// The chat row the run-queue pump holds, per canonical vault: set before
/// the pump claims a chat row and cleared once that row is requeued or
/// terminal ([`ChatRowGuard`]). It is the authority for "a chat turn is
/// running": `chat::accept_turn` refuses a second turn while it is set, and
/// repairs a `claimed` or `running` chat row it does not name (left by a
/// terminal write that failed) to `interrupted`. Set BEFORE the claim, not
/// after the lease (the plan's wording), so the repair can never interrupt a
/// row the pump has claimed but not admitted yet.
///
/// Lock order: `CHAT_ROW_ACTIVE` → the session store's `WRITE_LOCK` (only
/// `chat::accept_turn` holds both, across its check, repair and enqueue). Never
/// held while taking the lease, finishing a run or emitting.
///
/// Since build programme PR B4 it holds a SET of rows per vault: several
/// conversations' turns can be admitted at once (workspaces' scoped runs),
/// and "a chat turn is running" is asked per conversation.
static CHAT_ROW_ACTIVE: Lazy<Mutex<HashMap<PathBuf, HashSet<i64>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn canonical(vault: &Path) -> PathBuf {
    std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf())
}

/// Holds [`CHAT_ROW_ACTIVE`] for one chat row; dropping it clears the gate
/// only if it still names this row. The pump drops it after the row's
/// requeue or terminal write, and before it emits the turn's `done`, so a
/// follow-up sent the moment `done` arrives is never refused as "already
/// queued".
///
/// Merged plan slice 3 (A·R4): a guard exists only for an id it inserted.
/// [`ChatRowGuard::hold`] refuses an id that is already active (a turn for
/// that row is live), so a second guard can never be built for it, and
/// dropping a refused hold (`None`) releases nothing.
#[derive(Debug)]
pub(crate) struct ChatRowGuard {
    vault: PathBuf,
    id: i64,
}

impl ChatRowGuard {
    /// Mark `id` active, or `None` when it already is: that row's turn is
    /// live elsewhere and must not be claimed or admitted a second time.
    pub(crate) fn hold(vault: &Path, id: i64) -> Option<ChatRowGuard> {
        let vault = canonical(vault);
        let inserted = CHAT_ROW_ACTIVE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(vault.clone())
            .or_default()
            .insert(id);
        // Never build the guard for a refused id: its `Drop` would release
        // the live holder's mark.
        if inserted {
            Some(ChatRowGuard { vault, id })
        } else {
            None
        }
    }
}

/// The chat rows the pump holds for `vault` (each a live or admitting turn).
/// The pump never releases one of these as a stray claim.
pub(crate) fn active_chat_rows(vault: &Path) -> Vec<i64> {
    CHAT_ROW_ACTIVE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&canonical(vault))
        .map(|rows| rows.iter().copied().collect())
        .unwrap_or_default()
}

impl Drop for ChatRowGuard {
    fn drop(&mut self) {
        let mut active = CHAT_ROW_ACTIVE
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(rows) = active.get_mut(&self.vault) {
            rows.remove(&self.id);
            if rows.is_empty() {
                active.remove(&self.vault);
            }
        }
    }
}

/// The chat row the pump holds for `vault`, when it holds exactly one.
#[cfg(test)]
pub(crate) fn active_chat_row(vault: &Path) -> Option<i64> {
    let active = CHAT_ROW_ACTIVE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let rows = active.get(&canonical(vault))?;
    (rows.len() == 1).then(|| *rows.iter().next().unwrap())
}

/// Run `f` with the rows the pump holds for `vault`, holding the gate for
/// the whole call. Lock order: `CHAT_ROW_ACTIVE` → the session store's
/// `WRITE_LOCK`; `chat::accept_turn` is the one caller that takes both
/// ([`reserve_if_idle`] also holds the gate, for a read).
/// `f` must not touch the gate again (the mutex isn't reentrant: no
/// [`active_chat_rows`], [`ChatRowGuard::hold`] or guard drop inside it),
/// take the run lease, call `interactions` or emit.
pub(crate) fn with_active_rows<R>(vault: &Path, f: impl FnOnce(&HashSet<i64>) -> R) -> R {
    let active = CHAT_ROW_ACTIVE
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let current = active.get(&canonical(vault)).cloned().unwrap_or_default();
    f(&current)
}

/// Conversations reserved against admission (R1, review of #1336): a fresh
/// follow-up's idle check and its start are not one step, so the keys it
/// found idle stay reserved until its own row is queued. Admission
/// (`chat::accept_turn`) refuses a reserved key as busy, under the gate, so
/// a send (or a held send drained) that lands in between waits behind
/// instead of running beside it. A leaf lock: taken inside the gate, never
/// around it.
static RESERVED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Keys reserved by [`reserve_if_idle`], released when dropped.
#[derive(Debug)]
pub(crate) struct Reservation {
    keys: Vec<String>,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut reserved = RESERVED.lock().unwrap_or_else(PoisonError::into_inner);
        for key in &self.keys {
            if let Some(at) = reserved.iter().position(|k| k == key) {
                reserved.swap_remove(at);
            }
        }
    }
}

/// Reserve `keys` when none is reserved already and `idle` says so, both
/// checked under the gate admission holds, so no turn for them is admitted
/// between the check and the reservation. `None` means busy.
pub(crate) fn reserve_if_idle(
    vault: &Path,
    keys: &[&str],
    idle: impl FnOnce() -> bool,
) -> Option<Reservation> {
    with_active_rows(vault, |_| {
        if keys.iter().any(|key| is_reserved(key)) || !idle() {
            return None;
        }
        let mut keys: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
        keys.sort();
        keys.dedup();
        let mut reserved = RESERVED.lock().unwrap_or_else(PoisonError::into_inner);
        reserved.extend(keys.iter().cloned());
        Some(Reservation { keys })
    })
}

/// Whether a [`Reservation`] holds `key` (admission asks under the gate).
pub(crate) fn is_reserved(key: &str) -> bool {
    RESERVED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|k| k == key)
}
