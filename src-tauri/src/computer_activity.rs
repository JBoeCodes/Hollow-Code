//! Computer control's per-chat state for the chat UI (Cua CU8): whether the
//! person has paused or taken over the desktop, the recent actions, and the
//! latest frame. Held in memory by chat (the broker's `scope`), bounded, and
//! gone when Hollow quits or Stop all runs. Never persisted: the audit log
//! (CU3) is the record; this is what the card and the chip show.
//!
//! A frame is a screenshot the broker already passed to the model (capped);
//! it stays here so the person can see what the agent saw, and is read by
//! the chat's own card only.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Chats remembered at once; the least recently active is dropped past this.
const MAX_CHATS: usize = 32;
/// Actions kept per chat.
const MAX_ENTRIES: usize = 50;
/// Entries that keep their own frame (the rest keep only a mark).
const MAX_THUMBS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: u64,
    pub at_ms: u64,
    pub tool: String,
    pub tier: String,
    pub target: String,
    /// `ok`, `failed` or `refused`.
    pub outcome: String,
    /// The audit row's fixed category (never the Driver's words).
    pub detail: String,
    /// The app a This Mac call named, when it did.
    pub app: Option<String>,
    /// This entry still has its frame (only the newest few do).
    pub has_frame: bool,
}

/// What the card and chip read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatView {
    pub paused: bool,
    pub taken_over: bool,
    /// Newest first.
    pub entries: Vec<Entry>,
    /// The id of the newest entry that has a frame (what `frame` returns).
    pub frame_id: Option<u64>,
}

#[derive(Default)]
struct Chat {
    paused: bool,
    taken_over: bool,
    entries: VecDeque<Entry>,
    frames: HashMap<u64, String>,
    last_active: u64,
}

static STATE: Mutex<Option<(u64, HashMap<String, Chat>)>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut u64, &mut HashMap<String, Chat>) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let (next, chats) = guard.get_or_insert_with(|| (1, HashMap::new()));
    f(next, chats)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether the person has paused (or taken over) this chat's desktop.
pub(crate) fn pause_state(scope: &str) -> Option<bool> {
    with(|_, chats| chats.get(scope).filter(|c| c.paused).map(|c| c.taken_over))
}

/// Record one call. `frame` is a data URL of the first screenshot it
/// returned, if any.
pub(crate) fn record(scope: &str, mut entry: Entry, frame: Option<String>) {
    if scope.is_empty() {
        return;
    }
    with(|next, chats| {
        if chats.len() >= MAX_CHATS && !chats.contains_key(scope) {
            // Never drop a chat the person holds: forgetting its pause would
            // let the agent carry on.
            if let Some(oldest) = chats
                .iter()
                .filter(|(_, c)| !c.paused)
                .min_by_key(|(_, c)| c.last_active)
                .map(|(k, _)| k.clone())
            {
                chats.remove(&oldest);
            }
        }
        let chat = chats.entry(scope.to_string()).or_default();
        entry.id = *next;
        *next += 1;
        entry.at_ms = now_ms();
        entry.has_frame = frame.is_some();
        chat.last_active = entry.at_ms;
        if let Some(f) = frame {
            chat.frames.insert(entry.id, f);
        }
        chat.entries.push_back(entry);
        while chat.entries.len() > MAX_ENTRIES {
            if let Some(old) = chat.entries.pop_front() {
                chat.frames.remove(&old.id);
            }
        }
        // Only the newest few keep their frame.
        let mut with_frames: Vec<u64> = chat.frames.keys().copied().collect();
        with_frames.sort_unstable();
        while with_frames.len() > MAX_THUMBS {
            let drop = with_frames.remove(0);
            chat.frames.remove(&drop);
            if let Some(e) = chat.entries.iter_mut().find(|e| e.id == drop) {
                e.has_frame = false;
            }
        }
    });
}

pub(crate) fn view(scope: &str) -> ChatView {
    with(|_, chats| {
        let Some(chat) = chats.get(scope) else {
            return ChatView::default();
        };
        ChatView {
            paused: chat.paused,
            taken_over: chat.taken_over,
            entries: chat.entries.iter().rev().cloned().collect(),
            frame_id: chat
                .entries
                .iter()
                .rev()
                .find(|e| e.has_frame)
                .map(|e| e.id),
        }
    })
}

/// One frame: the newest when `id` is `None`.
pub(crate) fn frame(scope: &str, id: Option<u64>) -> Option<String> {
    with(|_, chats| {
        let chat = chats.get(scope)?;
        let id = id.or_else(|| {
            chat.entries
                .iter()
                .rev()
                .find(|e| e.has_frame)
                .map(|e| e.id)
        })?;
        chat.frames.get(&id).cloned()
    })
}

/// Pause or resume a chat's desktop. `taken_over` marks that the person
/// holds the controls (Take over), which Hand back releases.
pub(crate) fn set_paused(scope: &str, paused: bool, taken_over: bool) {
    if scope.is_empty() {
        return;
    }
    with(|_, chats| {
        let chat = chats.entry(scope.to_string()).or_default();
        chat.paused = paused;
        chat.taken_over = paused && taken_over;
        chat.last_active = now_ms();
    });
}

/// Forget one chat's state (Stop on that chat).
pub(crate) fn forget(scope: &str) {
    with(|_, chats| {
        chats.remove(scope);
    });
}

/// Forget everything (Stop all).
pub(crate) fn clear() {
    with(|_, chats| chats.clear());
}

/// A `data:` URL for the first image block of a (capped) tool result.
pub(crate) fn first_frame(result: &serde_json::Value) -> Option<String> {
    result
        .get("content")?
        .as_array()?
        .iter()
        .find(|i| i.get("type").and_then(serde_json::Value::as_str) == Some("image"))
        .and_then(|i| {
            let data = i.get("data")?.as_str()?;
            let mime = i
                .get("mimeType")
                .and_then(serde_json::Value::as_str)
                .filter(|m| matches!(*m, "image/png" | "image/jpeg" | "image/webp"))
                .unwrap_or("image/png");
            Some(format!("data:{mime};base64,{data}"))
        })
}

#[cfg(test)]
#[path = "computer_activity_tests.rs"]
mod tests;
