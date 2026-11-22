//! Lane M (2026-11-17): the continuations a moved chat is owed, and the
//! notifications already told about moves, kept across a restart.
//!
//! `chat_move_trigger` owes a chat that was running when it moved one
//! continuation turn at its new home (sent once the move lands), and tells
//! the person once about a move that is stuck, came home, or didn't
//! continue. Both lived in memory: quit the app between the stop and the
//! landing and the chat sat idle on the other Mac with nothing to say why, or
//! an old stuck move was announced again. This is the file that keeps them.
//!
//! **Owed.** `move id -> chat key`, with the time it was owed. Taking one
//! (the continuation sent, the move came home, the person released it)
//! removes it: that is its delivered state, so a continuation is never owed
//! twice. One older than [`OWED_KEPT_MS`] is dropped as expired: a "continue
//! from where it stopped" days later would be wrong. On boot the trigger's
//! retry replays each owed continuation whose move landed (`retry_with`);
//! `clientMessageId` is made from the move id, so a send that did land the
//! first time runs once there.
//!
//! **Reported.** `(move id, kind)` of every notification told, so a restart
//! doesn't tell it again. A mark is written before the notification is
//! posted and taken back when posting fails, so it fires at most once. Kept
//! [`REPORTED_KEPT_MS`], then pruned.
//!
//! One JSON file in the Peers folder, written whole with
//! `vault::atomic_write_private` (`0600`), at most [`OWED_MAX`] and
//! [`REPORTED_MAX`] entries (the oldest go first), ignored when too large or
//! unreadable. Nothing of a conversation is in it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The file in the Peers folder.
pub(crate) const FILE: &str = "chat-move-owed.json";
pub(crate) const OWED_MAX: usize = 256;
pub(crate) const REPORTED_MAX: usize = 512;
/// An owed continuation older than this is dropped.
pub(crate) const OWED_KEPT_MS: i64 = 24 * 3600 * 1000;
/// A told notification is remembered this long.
pub(crate) const REPORTED_KEPT_MS: i64 = 7 * 24 * 3600 * 1000;
/// A file larger than this is not read.
pub(crate) const FILE_MAX: u64 = 1024 * 1024;
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Owed {
    pub move_id: String,
    pub key: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Reported {
    pub move_id: String,
    pub kind: String,
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Book {
    version: u32,
    pub(crate) owed: Vec<Owed>,
    pub(crate) reported: Vec<Reported>,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            version: VERSION,
            owed: Vec::new(),
            reported: Vec::new(),
        }
    }
}

fn sane(text: &str) -> bool {
    !text.is_empty() && text.len() <= 200 && !text.chars().any(char::is_control)
}

/// Set a file this Mac can't use aside as `<name>.json.bad` (replacing an
/// older one) so the next save doesn't overwrite what was in it.
fn quarantine(path: &Path) {
    if path.is_file() {
        let _ = std::fs::rename(path, path.with_extension("json.bad"));
    }
}

impl Book {
    /// The book in `path`, empty when it is missing, too large or unreadable.
    pub(crate) fn load(path: &Path) -> Self {
        let Ok(meta) = std::fs::metadata(path) else {
            return Self::default();
        };
        if !meta.is_file() || meta.len() > FILE_MAX {
            crate::launch::elog(&format!(
                "[chat move] ignoring {}: too large",
                path.display()
            ));
            quarantine(path);
            return Self::default();
        }
        let parsed = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Book>(&b).map_err(|e| e.to_string()));
        match parsed {
            Ok(mut book) if book.version == VERSION => {
                book.owed
                    .retain(|o| sane(&o.move_id) && sane(&o.key) && o.at_ms > 0);
                book.reported
                    .retain(|r| sane(&r.move_id) && sane(&r.kind) && r.at_ms > 0);
                book.owed.truncate(OWED_MAX);
                book.reported.truncate(REPORTED_MAX);
                book
            }
            Ok(_) => {
                quarantine(path);
                Self::default()
            }
            Err(e) => {
                crate::launch::elog(&format!("[chat move] ignoring {}: {e}", path.display()));
                quarantine(path);
                Self::default()
            }
        }
    }

    pub(crate) fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        crate::vault::atomic_write_private(path, &bytes)
    }

    /// Owe chat `key` a continuation for move `move_id`. Owing the same
    /// move again keeps its first time (and takes the new key).
    pub(crate) fn owe(&mut self, move_id: &str, key: &str, now_ms: i64) {
        match self.owed.iter_mut().find(|o| o.move_id == move_id) {
            Some(o) => o.key = key.to_string(),
            None => self.owed.push(Owed {
                move_id: move_id.to_string(),
                key: key.to_string(),
                at_ms: now_ms,
            }),
        }
        self.prune(now_ms);
    }

    /// Take move `move_id`'s owed continuation: its chat key, once.
    pub(crate) fn take(&mut self, move_id: &str) -> Option<String> {
        let at = self.owed.iter().position(|o| o.move_id == move_id)?;
        Some(self.owed.remove(at).key)
    }

    /// Whether a continuation is owed to chat `key`.
    #[cfg(test)]
    pub(crate) fn owes(&self, key: &str) -> bool {
        self.owed.iter().any(|o| o.key == key)
    }

    /// Every continuation owed: `(move id, chat key)`, oldest first.
    pub(crate) fn owed_moves(&self) -> Vec<(String, String)> {
        self.owed
            .iter()
            .map(|o| (o.move_id.clone(), o.key.clone()))
            .collect()
    }

    /// Mark `kind` of move `move_id` as told. False when it was told
    /// already (do not tell it again).
    pub(crate) fn mark_reported(&mut self, move_id: &str, kind: &str, now_ms: i64) -> bool {
        if self
            .reported
            .iter()
            .any(|r| r.move_id == move_id && r.kind == kind)
        {
            return false;
        }
        self.reported.push(Reported {
            move_id: move_id.to_string(),
            kind: kind.to_string(),
            at_ms: now_ms,
        });
        self.prune(now_ms);
        true
    }

    /// Take a mark back (the notification could not be posted).
    pub(crate) fn unmark_reported(&mut self, move_id: &str, kind: &str) {
        self.reported
            .retain(|r| !(r.move_id == move_id && r.kind == kind));
    }

    /// Drop expired entries and hold both lists to their bounds (oldest
    /// first).
    pub(crate) fn prune(&mut self, now_ms: i64) {
        self.owed
            .retain(|o| now_ms.saturating_sub(o.at_ms) < OWED_KEPT_MS);
        self.reported
            .retain(|r| now_ms.saturating_sub(r.at_ms) < REPORTED_KEPT_MS);
        if self.owed.len() > OWED_MAX {
            let extra = self.owed.len() - OWED_MAX;
            self.owed.sort_by_key(|o| o.at_ms);
            self.owed.drain(..extra);
        }
        if self.reported.len() > REPORTED_MAX {
            let extra = self.reported.len() - REPORTED_MAX;
            self.reported.sort_by_key(|r| r.at_ms);
            self.reported.drain(..extra);
        }
    }
}

/// The app's file (`<support>/Peers/chat-move-owed.json`).
pub(crate) fn app_file() -> Option<PathBuf> {
    crate::instance::support_dir().map(|dir| dir.join("Peers").join(FILE))
}

#[cfg(test)]
#[path = "chat_move_owed_tests.rs"]
mod tests;
