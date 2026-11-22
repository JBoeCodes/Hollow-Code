//! Lane M (2026-11-17): the routed turns this Mac's COO waits to see end,
//! kept across a restart.
//!
//! A dispatcher COO's `sessions.route` returns once the runner accepted the
//! text; Hollow then watches that chat through the runner's list stream for
//! one notification when its turn ends (`Hub::follow_routed`). That list of
//! turns was memory only: quit or crash the app and the notification was
//! never sent. This file keeps it.
//!
//! **What is kept.** Per turn: which Mac and chat, the words the
//! notification names ("the docs lead on Mac mini", never the chat's own
//! words), this Mac's vault the notification goes to, when it was routed
//! (wall clock, so it survives a restart) and its state. Nothing of the
//! conversation. One JSON file in the Peers folder, written whole through
//! `vault::atomic_write_private` (temp file, rename, `0600`).
//!
//! **States.** `pending` waits to end. `delivered` was claimed for its one
//! notification: the claim is written *before* the notification is posted,
//! so a crash between the two loses a notification rather than sending it
//! twice (a notification that fails to post is given back, [`Book::unclaim`]).
//! `expired` ran past the watch's limit and is forgotten without one. Done
//! entries stay [`DONE_KEPT_MS`] as tombstones (so a replayed list can't fire
//! one again), then are pruned.
//!
//! **Bounds.** At most [`ENTRIES_MAX`] entries (done ones go first, then the
//! oldest waiting one), [`FIELD_MAX`] characters a field, a file past
//! [`FILE_MAX`] bytes or that doesn't parse is ignored (and left for the next
//! save to replace). One routed turn per chat at a time: routing to a chat
//! that already has one waiting adds nothing (a chat takes one turn at a
//! time, so one notification covers it).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file in the Peers folder.
pub(crate) const FILE: &str = "routed-turns.json";
/// At most this many entries, done ones included.
pub(crate) const ENTRIES_MAX: usize = 64;
/// At most this many characters in a text field.
pub(crate) const FIELD_MAX: usize = 256;
/// A file larger than this is not read.
pub(crate) const FILE_MAX: u64 = 1024 * 1024;
/// A turn waits at most this long (`dispatcher_coo::ROUTED_FOLLOW_MAX`).
/// A notification that can't be posted this many times is given up on.
pub(crate) const POST_TRIES_MAX: u32 = 5;
pub(crate) const FOLLOW_MAX_MS: i64 = 12 * 3600 * 1000;
/// A delivered or expired entry is kept this long, then pruned.
pub(crate) const DONE_KEPT_MS: i64 = 24 * 3600 * 1000;
const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Pending,
    Delivered,
    Expired,
}

/// One routed turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Entry {
    /// `<machine>/<chat>@<routed at>`: one notification per id.
    pub id: String,
    pub machine_id: String,
    pub machine: String,
    pub key: String,
    pub what: String,
    /// This Mac's vault, where the notification goes.
    pub vault: String,
    /// Unix milliseconds the turn was routed.
    pub at_ms: i64,
    pub seen_busy: bool,
    pub state: State,
    /// Unix milliseconds it was delivered or expired.
    #[serde(default)]
    pub done_ms: Option<i64>,
    /// How many times its notification was claimed and could not be posted.
    #[serde(default)]
    pub tries: u32,
}

/// The id of a turn routed at `at_ms` to `key` on `machine_id`.
pub(crate) fn id_of(machine_id: &str, key: &str, at_ms: i64) -> String {
    format!("{machine_id}/{key}@{at_ms}")
}

fn clipped(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(FIELD_MAX)
        .collect()
}

impl Entry {
    /// Whether a loaded entry is usable: every identifying field there.
    fn usable(&self) -> bool {
        !self.id.is_empty()
            && !self.machine_id.is_empty()
            && !self.key.is_empty()
            && !self.vault.is_empty()
            && self.at_ms > 0
            && (self.state == State::Pending || self.done_ms.is_some())
    }

    fn clipped(mut self) -> Self {
        self.id = clipped(&self.id);
        self.machine_id = clipped(&self.machine_id);
        self.machine = clipped(&self.machine);
        self.key = clipped(&self.key);
        self.what = clipped(&self.what);
        self.vault = self.vault.chars().filter(|c| *c != '\0').collect();
        self
    }
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Book {
    version: u32,
    pub(crate) entries: Vec<Entry>,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

/// Set a file this Mac can't use aside as `<name>.json.bad` (replacing an
/// older one) so the next save doesn't overwrite what was in it.
fn quarantine(path: &Path) {
    if path.is_file() {
        let _ = std::fs::rename(path, path.with_extension("json.bad"));
    }
}

impl Book {
    /// The book in `path`: empty when there is no file, it is too large, or
    /// it doesn't parse (a note goes to the log; the next save replaces it).
    pub(crate) fn load(path: &Path) -> Self {
        let Ok(meta) = std::fs::metadata(path) else {
            return Self::default();
        };
        if !meta.is_file() || meta.len() > FILE_MAX {
            crate::launch::elog(&format!(
                "[routed turns] ignoring {}: not a file or too large",
                path.display()
            ));
            quarantine(path);
            return Self::default();
        }
        let parsed = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| serde_json::from_slice::<Book>(&bytes).map_err(|e| e.to_string()));
        match parsed {
            Ok(mut book) if book.version == VERSION => {
                book.entries = book
                    .entries
                    .into_iter()
                    .filter(Entry::usable)
                    .map(Entry::clipped)
                    .collect();
                book.entries.truncate(ENTRIES_MAX);
                book
            }
            Ok(_) => {
                quarantine(path);
                Self::default()
            }
            Err(e) => {
                crate::launch::elog(&format!("[routed turns] ignoring {}: {e}", path.display()));
                quarantine(path);
                Self::default()
            }
        }
    }

    /// Write the book whole, `0600`, atomically.
    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        crate::vault::atomic_write_private(path, &bytes)
    }

    /// Keep a turn that was routed, unless that chat has one waiting
    /// already or this very one is here. True when it was added.
    pub(crate) fn add(&mut self, entry: Entry, now_ms: i64) -> bool {
        let entry = entry.clipped();
        let dup = self.entries.iter().any(|e| {
            e.id == entry.id
                || (e.state == State::Pending
                    && e.machine_id == entry.machine_id
                    && e.key == entry.key)
        });
        if dup {
            return false;
        }
        self.entries.push(Entry {
            state: State::Pending,
            done_ms: None,
            ..entry
        });
        self.prune(now_ms);
        true
    }

    /// Expire the waiting turns past the limit, drop done entries past their
    /// keeping time, and hold the book to [`ENTRIES_MAX`]: done ones go
    /// first (oldest first), then the oldest waiting one.
    pub(crate) fn prune(&mut self, now_ms: i64) {
        for e in &mut self.entries {
            if e.state == State::Pending && now_ms.saturating_sub(e.at_ms) >= FOLLOW_MAX_MS {
                e.state = State::Expired;
                e.done_ms = Some(now_ms);
            }
        }
        self.entries.retain(|e| {
            e.done_ms
                .is_none_or(|d| now_ms.saturating_sub(d) < DONE_KEPT_MS)
        });
        while self.entries.len() > ENTRIES_MAX {
            let victim = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| (e.state == State::Pending, e.done_ms.unwrap_or(e.at_ms)))
                .map(|(i, _)| i);
            match victim {
                Some(i) => {
                    self.entries.remove(i);
                }
                None => break,
            }
        }
    }

    /// The turns still waiting to end, oldest first.
    pub(crate) fn pending(&self) -> Vec<&Entry> {
        let mut waiting: Vec<&Entry> = self
            .entries
            .iter()
            .filter(|e| e.state == State::Pending)
            .collect();
        waiting.sort_by_key(|e| e.at_ms);
        waiting
    }

    /// Take the one notification of turn `id`: true only the first time
    /// (it was waiting). A delivered, expired or unknown turn gives false,
    /// so a replayed or repeated list never fires it again.
    pub(crate) fn claim(&mut self, id: &str, now_ms: i64) -> bool {
        self.finish(id, State::Delivered, now_ms)
    }

    /// Forget turn `id` without a notification (it ran past the limit).
    pub(crate) fn expire(&mut self, id: &str, now_ms: i64) -> bool {
        self.finish(id, State::Expired, now_ms)
    }

    fn finish(&mut self, id: &str, state: State, now_ms: i64) -> bool {
        match self
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.state == State::Pending)
        {
            Some(e) => {
                e.state = state;
                e.done_ms = Some(now_ms);
                true
            }
            None => false,
        }
    }

    /// Give a claimed turn back (its notification could not be posted), so
    /// the next list tries again: true when it waits again. Only a delivered
    /// one. After [`POST_TRIES_MAX`] failed posts it is expired instead
    /// (false), so a post that never works doesn't repeat for the whole watch.
    pub(crate) fn unclaim(&mut self, id: &str, now_ms: i64) -> bool {
        match self
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.state == State::Delivered)
        {
            Some(e) => {
                e.tries += 1;
                if e.tries >= POST_TRIES_MAX {
                    e.state = State::Expired;
                    e.done_ms = Some(now_ms);
                    return false;
                }
                e.state = State::Pending;
                e.done_ms = None;
                true
            }
            None => false,
        }
    }

    /// Record that turn `id` was seen queued or running. True when that
    /// changed the book.
    pub(crate) fn note_busy(&mut self, id: &str) -> bool {
        match self
            .entries
            .iter_mut()
            .find(|e| e.id == id && e.state == State::Pending && !e.seen_busy)
        {
            Some(e) => {
                e.seen_busy = true;
                true
            }
            None => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn state_of(&self, id: &str) -> Option<State> {
        self.entries.iter().find(|e| e.id == id).map(|e| e.state)
    }
}

/// The app's file (`<support>/Peers/routed-turns.json`).
pub(crate) fn app_file() -> Option<PathBuf> {
    crate::instance::support_dir().map(|dir| dir.join("Peers").join(FILE))
}

/// Whether the file at `path` holds a turn still waiting (a cheap look, so
/// boot makes no hub or tunnel when there is none).
pub(crate) fn has_pending(path: &Path) -> bool {
    !Book::load(path).pending().is_empty()
}

#[cfg(test)]
#[path = "routed_store_tests.rs"]
mod tests;
