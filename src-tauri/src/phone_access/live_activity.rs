//! v1.2 Live Activities (2026-11-07, `docs/phone-api/live-activity.md`).
//!
//! The iPhone app starts an activity for a running turn itself and keeps it
//! current while it is open. This is the Mac's half, for when the app is
//! closed: the push tokens of the activities ([`Activities`]) and what
//! Apple is told ([`State`], [`payload`]).
//!
//! - **The tokens are in memory only.** A token names one running activity,
//!   so it is worth nothing after the turn; a restart of Hollow loses only a
//!   convenience, because the app registers again at its next launch while
//!   an activity still shows. One per phone and chat, at most [`PER_PHONE`]
//!   chats per phone, [`LIFE`] at most (an activity's longest life).
//! - **What Apple sees** is the activity's `content-state`: a phase and a
//!   step label built from a tool's NAME only ([`step_for_tool`]). A tool's
//!   detail (a shell command can hold a secret), the chat's text and its
//!   title never go; the title is in the activity's attributes, which the
//!   app set when it started it.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde_json::{json, Map, Value};

use super::wire;

/// Chats per phone with a token kept; a ninth drops the oldest.
pub(crate) const PER_PHONE: usize = 8;
/// An activity's longest life (12 hours), after which its token goes.
pub(crate) const LIFE: i64 = 12 * 60 * 60;
/// A pushed `update` goes stale this long after it was made (seconds): the
/// lock screen then says "Open Hollow to refresh".
pub(crate) const STALE_IN: i64 = 600;
/// An `end` push keeps the finished state on the lock screen this long.
pub(crate) const FINISHED_STAYS: i64 = 300;
/// A step label's longest.
pub(crate) const STEP_MAX: usize = 80;

pub(crate) const STARTING: &str = "Starting";
pub(crate) const WORKING: &str = "Working";
pub(crate) const WRITING: &str = "Writing the answer";
pub(crate) const WAITING_FOR_ANSWER: &str = "Waiting for your answer";

/// One activity's push token, as a phone registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub device_id: String,
    pub key: String,
    pub token: String,
    pub environment: String,
    /// When it was registered (Unix seconds).
    pub at: i64,
    /// Which registration it was (1, 2, ...): a turn's end is for the entries
    /// that existed when the turn ended, whatever the clock says.
    pub seq: u64,
}

/// The tokens phones registered (`POST /v1/live-activity`), oldest first.
#[derive(Default)]
pub(crate) struct Activities {
    entries: Mutex<Vec<Entry>>,
    /// Registrations so far.
    registered: AtomicU64,
}

impl Activities {
    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Keep `token` for `device_id`'s activity on chat `key`: a new one for
    /// the same phone and chat replaces the old, a token another entry held
    /// moves here (it names one activity), and a phone's ninth chat drops
    /// its oldest.
    pub(crate) fn register(
        &self,
        device_id: &str,
        key: &str,
        token: &str,
        environment: &str,
        now: i64,
    ) {
        let mut entries = self.lock();
        let seq = self.registered.fetch_add(1, Ordering::SeqCst) + 1;
        entries.retain(|e| e.token != token && !(e.device_id == device_id && e.key == key));
        entries.push(Entry {
            device_id: device_id.to_string(),
            key: key.to_string(),
            token: token.to_string(),
            environment: environment.to_string(),
            at: now,
            seq,
        });
        while entries.iter().filter(|e| e.device_id == device_id).count() > PER_PHONE {
            match entries.iter().position(|e| e.device_id == device_id) {
                Some(oldest) => {
                    entries.remove(oldest);
                }
                None => break,
            }
        }
    }

    /// Drop what `device_id` registered: for chat `key`, or all of it.
    pub(crate) fn forget(&self, device_id: &str, key: Option<&str>) -> usize {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|e| e.device_id != device_id || key.is_some_and(|k| e.key != k));
        before - entries.len()
    }

    /// Drop every phone's tokens for chat `key` (tests: the end drops only
    /// the tokens it told, by [`Self::drop_token`]).
    #[cfg(test)]
    pub(crate) fn forget_chat(&self, key: &str) -> usize {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|e| e.key != key);
        before - entries.len()
    }

    /// Drop a token Apple says is no longer valid.
    pub(crate) fn drop_token(&self, token: &str) -> usize {
        let mut entries = self.lock();
        let before = entries.len();
        entries.retain(|e| e.token != token);
        before - entries.len()
    }

    /// The registrations made so far: the entries with a `seq` up to this are
    /// the ones that exist now. A turn's end takes it when the turn ends, so an
    /// activity the app registers afterwards (the chat's next turn) is never
    /// told the previous turn is over.
    pub(crate) fn mark(&self) -> u64 {
        self.registered.load(Ordering::SeqCst)
    }

    /// Some phone has a token for chat `key` (cheap: the observer asks it
    /// for every event).
    pub(crate) fn has_chat(&self, key: &str) -> bool {
        self.lock().iter().any(|e| e.key == key)
    }

    /// The tokens for chat `key` that are less than [`LIFE`] old at `now`;
    /// older ones, of any chat, are dropped here.
    pub(crate) fn for_chat(&self, key: &str, now: i64) -> Vec<Entry> {
        let mut entries = self.lock();
        entries.retain(|e| now - e.at < LIFE);
        entries.iter().filter(|e| e.key == key).cloned().collect()
    }

    /// Keep only the tokens of phones that are still paired (a revoke).
    pub(crate) fn retain_devices(&self, paired: &HashSet<String>) {
        self.lock().retain(|e| paired.contains(&e.device_id));
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.lock().len()
    }
}

// ── What a push says ─────────────────────────────────────────────────────

/// Where a turn stands on the lock screen (`content-state.phase`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Running,
    Waiting,
    Finished,
    Failed,
    Stopped,
}

impl Phase {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Phase::Running => "running",
            Phase::Waiting => "waiting",
            Phase::Finished => "finished",
            Phase::Failed => "failed",
            Phase::Stopped => "stopped",
        }
    }

    /// The turn is over: its push is the `end` event.
    pub(crate) fn is_final(self) -> bool {
        matches!(self, Phase::Finished | Phase::Failed | Phase::Stopped)
    }
}

/// What the activity holds: exactly `aps.content-state`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct State {
    pub phase: Phase,
    /// None for a final phase.
    pub step: Option<String>,
}

impl State {
    /// A running turn at `step` (see [`step_of`]).
    pub(crate) fn running(step: String) -> Self {
        Self {
            phase: Phase::Running,
            step: Some(step),
        }
    }

    pub(crate) fn waiting() -> Self {
        Self {
            phase: Phase::Waiting,
            step: Some(WAITING_FOR_ANSWER.to_string()),
        }
    }

    pub(crate) fn ended(phase: Phase) -> Self {
        Self { phase, step: None }
    }
}

/// The step for a tool call by its name (fixture `step-labels.json`, the
/// table the app tests too): it never names a file, path or command.
pub(crate) fn step_for_tool(name: &str) -> &'static str {
    let name = name.to_ascii_lowercase();
    if name.starts_with("mcp__") {
        return "Using a tool";
    }
    match name.as_str() {
        "read" | "notebookread" => "Reading files",
        "write" | "edit" | "multiedit" | "notebookedit" | "apply_patch" => "Editing files",
        "bash" | "bashoutput" | "shell" | "exec_command" | "local_shell" => "Running a command",
        "grep" | "glob" | "ls" => "Searching files",
        "webfetch" | "websearch" => "Searching the web",
        "task" | "agent" => "Running a helper",
        "todowrite" | "update_plan" => "Planning",
        _ => WORKING,
    }
}

/// What a running turn is doing: the newest tool call still running (by its
/// name), else the answer being written, else `Working` once a tool call was
/// seen, else `Starting`.
pub(crate) fn step_of(newest_running: Option<&str>, has_text: bool, seen_tool: bool) -> String {
    if let Some(name) = newest_running {
        return step_for_tool(name).to_string();
    }
    if has_text {
        return WRITING.to_string();
    }
    if seen_tool {
        WORKING.to_string()
    } else {
        STARTING.to_string()
    }
}

/// [`step_of`] from the live buffer's view of a turn. That view keeps at most
/// [`super::live::TOOLS_CAP`] tool calls, so a long turn is read through a
/// [`ToolWatch`] instead once there is one.
pub(crate) fn current_step(live: &wire::Live) -> String {
    let running = live.tools.iter().rev().find(|t| t.status == "running");
    step_of(
        running.map(|t| t.name.as_str()),
        !live.text.is_empty(),
        !live.tools.is_empty(),
    )
}

/// The most tool calls a [`ToolWatch`] holds as running at once; a further
/// one drops the oldest.
const WATCHED_RUNNING: usize = 32;

/// A running turn's tool calls by id, kept for its Live Activity. The live
/// buffer stops adding tool calls at [`super::live::TOOLS_CAP`] (the phone's
/// `live` view is bounded), which froze a long turn's step; this is not
/// capped by how many calls a turn made, only by how many run at once.
#[derive(Default, Debug)]
pub(crate) struct ToolWatch {
    /// `(id, name)`, oldest first.
    running: Vec<(String, String)>,
    seen: bool,
}

impl ToolWatch {
    /// What the live buffer already holds of a turn, for a watch that starts
    /// partway through it.
    pub(crate) fn from_live(live: &wire::Live) -> Self {
        let mut watch = Self {
            running: Vec::new(),
            seen: !live.tools.is_empty(),
        };
        for tool in live.tools.iter().filter(|t| t.status == "running") {
            watch.used(&tool.id, &tool.name);
        }
        watch
    }

    /// A tool call started. The same id again (the buffer's copy and the
    /// event are both seen) is one call.
    pub(crate) fn used(&mut self, id: &str, name: &str) {
        self.seen = true;
        if !id.is_empty() {
            self.running.retain(|(known, _)| known != id);
        }
        self.running.push((id.to_string(), name.to_string()));
        while self.running.len() > WATCHED_RUNNING {
            self.running.remove(0);
        }
    }

    /// A tool call ended; without an id the newest running one did.
    pub(crate) fn resulted(&mut self, id: &str) {
        if id.is_empty() {
            self.running.pop();
        } else {
            self.running.retain(|(known, _)| known != id);
        }
    }

    pub(crate) fn newest_running(&self) -> Option<&str> {
        self.running.last().map(|(_, name)| name.as_str())
    }

    pub(crate) fn seen(&self) -> bool {
        self.seen
    }
}

/// The body of a Live Activity push for `state`, made at `now` (Unix
/// seconds): an `update` that goes stale in [`STALE_IN`] seconds, or for a
/// final phase an `end` that stays on the lock screen [`FINISHED_STAYS`]
/// seconds (fixtures `push-update.json`, `push-waiting.json`,
/// `push-end.json`).
pub(crate) fn payload(state: &State, now: i64) -> Value {
    let mut content = Map::new();
    content.insert("phase".into(), json!(state.phase.as_str()));
    if let Some(step) = state.step.as_deref().filter(|s| !s.is_empty()) {
        content.insert("step".into(), json!(super::live::cut_chars(step, STEP_MAX)));
    }
    let mut aps = Map::new();
    aps.insert("timestamp".into(), json!(now));
    if state.phase.is_final() {
        aps.insert("event".into(), json!("end"));
        aps.insert("dismissal-date".into(), json!(now + FINISHED_STAYS));
    } else {
        aps.insert("event".into(), json!("update"));
        aps.insert("stale-date".into(), json!(now + STALE_IN));
        aps.insert("relevance-score".into(), json!(100));
    }
    aps.insert("content-state".into(), Value::Object(content));
    json!({ "aps": aps })
}
