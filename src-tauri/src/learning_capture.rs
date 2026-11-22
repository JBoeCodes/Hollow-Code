//! Provider-neutral capture scheduling. Only the person's attended turns
//! enter this queue; task packets and unattended runs never teach preferences.
//! One retained finished turn, the turn in flight and one capture run per
//! chat, with no turn on the wait path. A turn that ends without success
//! ([`ended`]) leaves the retained turn waiting, never the chat in flight.

use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::Emitter;

pub(crate) const IDLE: Duration = Duration::from_secs(10 * 60);
const EVERY_TURNS: u32 = 4;
const CAPTURE_BUSY: &str = "A learning capture for this chat is already running.";
/// The extraction prompt reads at most this much of the reply, so no more
/// waits in memory.
const ASSISTANT_CHARS: usize = 8_000;

#[derive(Clone, Default)]
pub(crate) struct Input {
    pub user: String,
    pub references: Vec<String>,
    pub assistant: String,
    pub run_id: String,
    pub self_hosted: Option<String>,
    pub material_change: bool,
}

#[derive(Default)]
struct Pending {
    /// The newest finished turn not yet captured. A new turn never discards
    /// it: only a capture that read it, or a newer finished turn, does.
    completed: Option<Input>,
    /// The turn in flight, which only a forced boundary snapshot reads.
    current: Option<Input>,
    /// The turn in flight already had its boundary snapshot.
    snapped: bool,
    generation: u64,
    turns: u32,
    /// The finished turn asked to remember, corrected or decided something
    /// (`vault_learning::should_extract`): due at once, not after idle.
    explicit: bool,
    busy: bool,
    idle_at: Option<Instant>,
    timer: Option<tauri::async_runtime::JoinHandle<()>>,
}

impl Pending {
    fn begin(&mut self, input: Input) {
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
        self.current = Some(input);
        self.snapped = false;
    }

    /// Turn `run_id` ended without success: cancelled, failed, or never
    /// started. The chat is idle again, so the retained finished turn waits
    /// one more idle period. False when `run_id` was not the turn in flight.
    fn abandon(&mut self, run_id: &str, now: Instant) -> bool {
        if self.current.as_ref().is_none_or(|c| c.run_id != run_id) {
            return false;
        }
        self.current = None;
        self.snapped = false;
        if self.completed.is_some() {
            self.idle_at = Some(now + IDLE);
        }
        true
    }

    fn finish(&mut self, input: Input, explicit: bool, now: Instant) -> bool {
        // Its own done ends the turn in flight. A delayed done from the turn
        // before (the next one already began) keeps its input and leaves
        // the newer turn in flight.
        if self
            .current
            .as_ref()
            .is_some_and(|c| c.run_id == input.run_id)
        {
            self.current = None;
            self.snapped = false;
        }
        self.completed = Some(input);
        // A complete turn supersedes a forced mid-turn snapshot.
        self.generation = self.generation.wrapping_add(1);
        self.turns = self.turns.saturating_add(1);
        self.explicit |= explicit;
        self.idle_at = Some(now + IDLE);
        self.due(now)
    }

    fn due(&self, now: Instant) -> bool {
        self.explicit || self.turns >= EVERY_TURNS || self.idle_at.is_some_and(|at| now >= at)
    }

    fn take(&mut self, forced: bool, now: Instant) -> Option<(Input, u64)> {
        if self.busy {
            return None;
        }
        let input = if forced {
            self.current
                .clone()
                .filter(|_| !self.snapped)
                .or_else(|| self.completed.clone())
        } else if self.current.is_none() && self.due(now) {
            self.completed.clone()
        } else {
            None
        }?;
        self.busy = true;
        if let Some(timer) = self.timer.take() {
            timer.abort();
        }
        Some((input, self.generation))
    }

    fn settle(&mut self, generation: u64, run_id: &str) {
        self.busy = false;
        if self.current.as_ref().is_some_and(|c| c.run_id == run_id) {
            self.snapped = true;
        }
        // The capture read the retained finished turn, or a snapshot whose
        // four-turn window holds it. A newer finish keeps its own input.
        if generation == self.generation {
            self.completed = None;
            self.turns = 0;
            self.explicit = false;
            self.idle_at = None;
        }
    }

    fn retry(&mut self, now: Instant) {
        self.busy = false;
        self.idle_at = Some(now + Duration::from_secs(30));
        self.turns = 0;
        self.explicit = false;
    }

    /// Nothing waits and nothing runs: the chat's entry can go.
    fn spent(&self) -> bool {
        self.completed.is_none() && self.current.is_none() && !self.busy
    }
}

type Key = (PathBuf, String);
static PENDING: Lazy<Mutex<HashMap<Key, Pending>>> = Lazy::new(|| Mutex::new(HashMap::new()));

static CAPTURING: Lazy<Mutex<HashSet<Key>>> = Lazy::new(|| Mutex::new(HashSet::new()));

/// Admission shared by timers and private-model queue drains. Different
/// model entries can drain together, but one chat gets one extractor.
pub(crate) struct CaptureLease(Key);

impl CaptureLease {
    pub(crate) fn acquire(vault: &Path, session: &str) -> Result<Self, String> {
        let key = (vault.to_path_buf(), session.to_string());
        if !CAPTURING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.clone())
        {
            return Err(CAPTURE_BUSY.into());
        }
        Ok(Self(key))
    }
}

impl Drop for CaptureLease {
    fn drop(&mut self) {
        CAPTURING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

pub(crate) fn started(vault: &Path, session: &str, input: Input) {
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry((vault.into(), session.into()))
        .or_default()
        .begin(input);
}

/// A successful attended turn's input. An explicit remember, correction or
/// decision (`vault_learning::should_extract`, the gate the old post-turn
/// extraction used) is due at once through the same host capture, with its
/// lease, scope and proof rules; anything else waits for idle, the fourth
/// turn or a boundary.
pub(crate) fn finished(
    app: Option<&tauri::AppHandle>,
    vault: &Path,
    session: &str,
    mut input: Input,
) {
    let key = (vault.to_path_buf(), session.to_string());
    let now = Instant::now();
    input.assistant = cap_chars(&input.assistant, ASSISTANT_CHARS);
    let explicit = crate::vault_learning::should_extract(&input.user, input.material_change);
    let immediate = PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key.clone())
        .or_default()
        .finish(input, explicit, now);
    if immediate {
        launch(app.cloned(), key, false);
    } else {
        arm(app.cloned(), key);
    }
}

/// Every attended turn that ends without success (cancelled, failed, or
/// unable to start after [`started`]) comes here, so its chat is never left
/// in flight: the retained finished turn re-arms its idle timer.
pub(crate) fn ended(app: Option<&tauri::AppHandle>, vault: &Path, session: &str, run_id: &str) {
    let key = (vault.to_path_buf(), session.to_string());
    {
        let mut all = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pending) = all.get_mut(&key) else {
            return;
        };
        if !pending.abandon(run_id, Instant::now()) {
            return;
        }
        if pending.spent() {
            all.remove(&key);
            return;
        }
    }
    arm(app.cloned(), key);
}

fn arm(app: Option<tauri::AppHandle>, key: Key) {
    let mut all = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(pending) = all.get_mut(&key) else {
        return;
    };
    if pending.current.is_some() || pending.completed.is_none() {
        return;
    }
    if let Some(timer) = pending.timer.take() {
        timer.abort();
    }
    let now = Instant::now();
    let delay = if pending.due(now) {
        Duration::ZERO
    } else {
        pending
            .idle_at
            .unwrap_or(now)
            .saturating_duration_since(now)
    };
    let generation = pending.generation;
    let task = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        // Detach the timer's handle before launching: taking a job must
        // never abort the task that is about to execute it.
        if let Some(p) = PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(&key)
        {
            if p.generation != generation {
                return;
            }
            p.timer = None;
        }
        launch(app, key, false);
    });
    pending.timer = Some(task);
}

/// Called before handing off a chat, or near its compaction boundary. It
/// snapshots the current input before the provider loses its context;
/// the host's durable transcript supplies the grounding, even mid-turn.
pub(crate) fn boundary(app: Option<&tauri::AppHandle>, vault: &Path, session: &str) {
    launch(app.cloned(), (vault.into(), session.into()), true);
}

fn launch(app: Option<tauri::AppHandle>, key: Key, forced: bool) {
    let job = PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&key)
        .and_then(|p| p.take(forced, Instant::now()));
    let Some((input, generation)) = job else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        let result = crate::vault_learning::capture_from_chat(&key.0, &key.1, &input).await;
        let retry = matches!(&result, Err(error) if error == CAPTURE_BUSY);
        match result {
            Ok(items) => {
                if let Some(app) = &app {
                    let _ = app.emit(
                        "learning-proposals-changed",
                        serde_json::json!({ "pendingAdded": items.len() }),
                    );
                }
            }
            Err(_) => {
                eprintln!("[learning] automatic capture could not finish; no memory was accepted")
            }
        }
        {
            let mut all = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(p) = all.get_mut(&key) {
                if retry {
                    p.retry(Instant::now());
                } else {
                    p.settle(generation, &input.run_id);
                }
                if p.spent() {
                    all.remove(&key);
                    return;
                }
            }
        }
        arm(app, key);
    });
}

fn cap_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_string(),
        None => text.to_string(),
    }
}

/// A provider's known compaction limit, or the context ring's advisory,
/// with a fifth of the window left for the capture to finish.
pub(crate) fn approaching(tokens: u64, limit: Option<u64>) -> bool {
    limit.is_some_and(|limit| limit > 0 && tokens >= limit.saturating_mul(4) / 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(run: &str) -> Input {
        Input {
            run_id: run.into(),
            ..Input::default()
        }
    }

    #[test]
    fn capture_admission_covers_private_drains_and_releases_on_cancellation() {
        let vault = tempfile::tempdir().unwrap();
        let first = CaptureLease::acquire(vault.path(), "same-chat").unwrap();
        assert!(CaptureLease::acquire(vault.path(), "same-chat").is_err());
        let other_chat = CaptureLease::acquire(vault.path(), "other-chat").unwrap();
        drop(first);
        assert!(CaptureLease::acquire(vault.path(), "same-chat").is_ok());
        drop(other_chat);
    }

    #[test]
    fn a_finished_turn_survives_its_earlier_forced_snapshot_and_lease_collision() {
        let now = Instant::now();
        let mut p = Pending::default();
        p.begin(input("a"));
        let (_, generation) = p.take(true, now).unwrap();
        p.finish(input("a"), false, now);
        p.settle(generation, "a");
        assert!(p.completed.is_some());
        assert!(p.take(false, now + IDLE).is_some());
        p.retry(now + IDLE);
        assert!(p.completed.is_some());
        assert!(p
            .take(false, now + IDLE + Duration::from_secs(29))
            .is_none());
        assert_eq!(
            p.take(false, now + IDLE + Duration::from_secs(30))
                .unwrap()
                .0
                .run_id,
            "a"
        );
    }

    #[test]
    fn an_idle_chat_waits_ten_minutes_and_a_new_turn_coalesces_it() {
        let now = Instant::now();
        let mut p = Pending::default();
        p.begin(input("a"));
        assert!(!p.finish(input("a"), false, now));
        assert!(p.take(false, now + IDLE - Duration::from_secs(1)).is_none());
        p.begin(input("b"));
        assert!(p.take(false, now + IDLE).is_none());
        assert!(!p.finish(input("b"), false, now + IDLE));
        let (job, _) = p.take(false, now + IDLE * 2).unwrap();
        assert_eq!(job.run_id, "b");
    }

    #[test]
    fn every_four_turns_or_a_boundary_is_due_but_captures_never_overlap() {
        let now = Instant::now();
        let mut p = Pending::default();
        for n in 1..=4 {
            let job = input(&n.to_string());
            p.begin(job.clone());
            assert_eq!(p.finish(job, false, now), n == 4);
        }
        let (_, generation) = p.take(false, now).unwrap();
        assert!(p.take(true, now).is_none());
        p.begin(input("new"));
        p.settle(generation, "4");
        assert_eq!(p.take(true, now).unwrap().0.run_id, "new");
    }

    #[test]
    fn settled_input_is_not_recaptured_and_old_done_cannot_replace_a_new_turn() {
        let now = Instant::now();
        let mut p = Pending::default();
        p.begin(input("new"));
        assert!(!p.finish(input("old"), false, now));
        assert_eq!(p.current.as_ref().unwrap().run_id, "new");
        let (job, generation) = p.take(true, now).unwrap();
        assert_eq!(job.run_id, "new");
        p.settle(generation, "new");
        assert!(p.take(true, now).is_none());
        assert!(approaching(80, Some(100)));
        assert!(!approaching(79, Some(100)));
        assert!(!approaching(u64::MAX, None));
    }

    /// Review of #1237: a cancelled turn after a finished one used to leave
    /// the chat in flight for good, and the finished turn's input was gone.
    #[test]
    fn a_cancelled_turn_after_a_finished_one_keeps_it_and_rearms_idle() {
        let now = Instant::now();
        let mut p = Pending::default();
        p.begin(input("a"));
        assert!(!p.finish(input("a"), false, now));
        p.begin(input("b"));
        assert!(p.take(false, now + IDLE).is_none(), "b is in flight");
        let stopped = now + Duration::from_secs(60);
        assert!(!p.abandon("another-run", stopped));
        assert!(p.abandon("b", stopped));
        assert!(p.current.is_none());
        assert!(p
            .take(false, stopped + IDLE - Duration::from_secs(1))
            .is_none());
        let (job, generation) = p.take(false, stopped + IDLE).unwrap();
        assert_eq!(job.run_id, "a");
        p.settle(generation, "a");
        assert!(p.spent());
    }

    /// A failed turn, or one whose transcript row could not be written after
    /// it began, keeps the four-turn count of the turns that did finish.
    #[test]
    fn a_failed_turn_keeps_the_count_and_a_lone_failure_leaves_nothing() {
        let now = Instant::now();
        let mut p = Pending::default();
        for n in 1..=3 {
            let job = input(&n.to_string());
            p.begin(job.clone());
            assert!(!p.finish(job, false, now));
        }
        p.begin(input("failed"));
        assert!(p.abandon("failed", now));
        assert!(p.take(false, now).is_none());
        p.begin(input("4"));
        assert!(p.finish(input("4"), false, now));
        assert_eq!(p.take(false, now).unwrap().0.run_id, "4");
        let mut lone = Pending::default();
        lone.begin(input("unstarted"));
        assert!(lone.abandon("unstarted", now));
        assert!(lone.spent());
        assert!(lone.idle_at.is_none());
    }

    #[test]
    fn an_explicit_remember_is_due_at_once_and_only_once() {
        let now = Instant::now();
        let mut p = Pending::default();
        p.begin(input("remember"));
        assert!(p.finish(input("remember"), true, now));
        let (job, generation) = p.take(false, now).unwrap();
        assert_eq!(job.run_id, "remember");
        p.settle(generation, "remember");
        p.begin(input("plain"));
        assert!(!p.finish(input("plain"), false, now));
        assert!(p.take(false, now).is_none());
    }

    #[test]
    fn the_public_hooks_rearm_a_cancelled_chat_and_drop_a_spent_one() {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let key = (v.to_path_buf(), "s-capture-ended".to_string());
        started(v, &key.1, input("a"));
        let mut done = input("a");
        done.assistant = "x".repeat(ASSISTANT_CHARS + 50);
        finished(None, v, &key.1, done);
        started(v, &key.1, input("b"));
        ended(None, v, &key.1, "b");
        {
            let mut all = PENDING.lock().unwrap_or_else(|e| e.into_inner());
            let p = all.get_mut(&key).unwrap();
            assert!(p.current.is_none());
            let kept = p.completed.as_ref().unwrap();
            assert_eq!(kept.run_id, "a");
            assert_eq!(kept.assistant.chars().count(), ASSISTANT_CHARS);
            assert!(p.idle_at.is_some());
            p.timer.take().expect("idle timer re-armed").abort();
            all.remove(&key);
        }
        let lone = (v.to_path_buf(), "s-capture-lone".to_string());
        started(v, &lone.1, input("c"));
        ended(None, v, &lone.1, "c");
        assert!(!PENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&lone));
    }
}
