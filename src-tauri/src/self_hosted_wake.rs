//! Self-hosted models, Q6 (2026-11-03): waking a self-hosted model from its
//! chat. A model on Cloud Run scales to zero after about 10 idle minutes,
//! and the first request after that waits 4 to 6 minutes for a GPU (Qwen:
//! 272 s in Q0) and costs money while it is up, so Hollow wakes one only on
//! a person's action: the first keystroke or a send in a chat on it, or the
//! chip's Start now (`components/SelfHostedStatus.tsx`).
//!
//! - **Resolution** ([`resolve_in`]): the entry must be saved, switched on
//!   and valid; each refusal names Settings, Models, Self-hosted models.
//! - **The wake** ([`wake_once`]): the entry's forwarder
//!   (`self_hosted_forwarder::ensure`), then a GET of its health path with
//!   the endpoint's token and no run key (`Forwarder::get`), waiting up to
//!   [`WAKE_TIMEOUT`] for a cold start.
//! - **Throttle and join** ([`take_turn`]): at most one wake per entry per
//!   [`WAKE_EVERY`], counted from when it started. A call while one runs
//!   waits for that one's answer instead of sending another, however long
//!   it takes; a call within 2 minutes of the last start gets that start's
//!   answer. Either way `shared` is true and nothing is sent, so no caller
//!   can make the endpoint loop. The wake runs on its own task, so a caller
//!   that goes away (a reloaded window) doesn't cut it short. Each answer
//!   says how long ago its wake started (`started_ms_ago`), so the chip
//!   anchors Ready, an error and the 2-minute wait to that start.
//! - **Status** ([`status`], `self_hosted_wake_status`): the last wake's
//!   age and answer (none while it runs), read from memory with no call to
//!   the model, so a reloaded window's chip shows a start already running.
//! - **Forget** ([`forget`]): saving, switching or removing the entry in
//!   Settings drops its last wake, so the next keystroke wakes the entry as
//!   it is now (Q6 review).
//! - **No secret** in the result ([`SelfHostedWake`]: status, times, error)
//!   or the log line (status and time only).
use crate::self_hosted_forwarder as forwarder;
use crate::self_hosted_registry::{self as registry, SelfHostedEntry, SelfHostedModels};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// One wake per entry this often, at most.
pub const WAKE_EVERY: Duration = Duration::from_secs(2 * 60);
/// The longest a wake waits: the forwarder's own 15 minutes for a cold
/// start's response headers, plus a margin.
pub const WAKE_TIMEOUT: Duration = Duration::from_secs(16 * 60);
/// Where the person adds, fixes or switches on an entry.
pub const SETTINGS_PATH: &str = "Settings → Models → Self-hosted models";
/// Most of a health answer Hollow reads.
const HEALTH_BYTES: usize = 4 * 1024;

/// What a wake found. Holds neither the endpoint's token nor a run key.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedWake {
    /// The health check's status, once it answered.
    pub status: Option<u16>,
    /// How long the wake took, from its start to the answer.
    pub ms: u32,
    /// What went wrong, in plain words.
    pub error: Option<String>,
    /// This answer is another call's wake (running, or started under
    /// 2 minutes ago): nothing was sent for this call.
    pub shared: bool,
    /// How long before this answer came back its wake started (another
    /// call's start when `shared`).
    pub started_ms_ago: u32,
}

/// An entry's last wake, as `self_hosted_wake_status` reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedWakeStatus {
    /// How long ago it started.
    pub started_ms_ago: u32,
    /// Its answer, or none while it runs.
    pub result: Option<SelfHostedWake>,
}

impl SelfHostedWake {
    /// The health check answered 2xx.
    pub fn ok(&self) -> bool {
        self.error.is_none() && self.status.is_some_and(|s| (200..300).contains(&s))
    }
}

/// The entry `id` from the registry's view, ready to wake, or why not.
pub(crate) fn resolve_in(view: SelfHostedModels, id: &str) -> Result<SelfHostedEntry, String> {
    if let Some(why) = view.error {
        return Err(format!("{why} ({SETTINGS_PATH})"));
    }
    let Some(entry) = view.entries.into_iter().find(|e| e.id == id) else {
        return Err(format!(
            "There is no self-hosted model \"{id}\". Add it in {SETTINGS_PATH}."
        ));
    };
    if !entry.enabled {
        return Err(format!(
            "{} is switched off. Switch it on in {SETTINGS_PATH}.",
            entry.name
        ));
    }
    let name = entry.name.clone();
    registry::normalize(entry)
        .map_err(|why| format!("{name} can't start until it is fixed in {SETTINGS_PATH}: {why}"))
}

fn ms(since: Instant) -> u32 {
    ms_between(since, Instant::now())
}

fn ms_between(since: Instant, now: Instant) -> u32 {
    u32::try_from(now.saturating_duration_since(since).as_millis()).unwrap_or(u32::MAX)
}

fn wait_text(limit: Duration) -> String {
    if limit.as_secs() >= 60 {
        format!("{} minutes", limit.as_secs() / 60)
    } else {
        format!("{} ms", limit.as_millis())
    }
}

/// One wake: the forwarder, then the health path through it.
pub(crate) async fn wake_once(entry: &SelfHostedEntry, timeout: Duration) -> SelfHostedWake {
    let started = Instant::now();
    let mut out = SelfHostedWake::default();
    match forwarder::ensure(entry.upstream()).await {
        Err(why) => out.error = Some(why),
        Ok(fwd) => {
            match tokio::time::timeout(timeout, fwd.get(&entry.health_path, HEALTH_BYTES)).await {
                Err(_) => {
                    out.error = Some(format!(
                        "The model didn't answer within {}.",
                        wait_text(timeout)
                    ))
                }
                Ok(Err(why)) => out.error = Some(why),
                Ok(Ok((status, _))) => {
                    out.status = Some(status);
                    if !(200..300).contains(&status) {
                        out.error = Some(format!(
                            "The health check ({}) answered {status}.",
                            entry.health_path
                        ));
                    }
                }
            }
        }
    }
    out.ms = ms(started);
    eprintln!(
        "[self-hosted] {}: wake {} ({:?}) in {} ms",
        entry.id,
        if out.ok() { "answered" } else { "failed" },
        out.status,
        out.ms
    );
    out
}

// ── Throttle and join ──────────────────────────────────────────────────

/// A wake's answer: `None` while it runs.
type Answer = watch::Receiver<Option<SelfHostedWake>>;

/// The last wake of one entry.
pub(crate) struct Slot {
    started: Instant,
    answer: Answer,
}

/// Whether a call sends its own wake or shares another's.
pub(crate) enum Turn {
    /// Send it, and put the answer here.
    Lead(watch::Sender<Option<SelfHostedWake>>),
    /// Wait for the answer of this wake, started then.
    Join(Answer, Instant),
}

/// Every entry's last wake. A leaf lock: held only to read or replace one
/// slot, never across an await or while taking another lock.
fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A call to wake `id` at `now`: join the wake that is running (however
/// old) or that started under `every` ago, else lead a new one. A wake whose
/// task ended without an answer is never joined.
pub(crate) fn take_turn(
    slots: &mut HashMap<String, Slot>,
    id: &str,
    now: Instant,
    every: Duration,
) -> Turn {
    if let Some(slot) = slots.get(id) {
        let answered = slot.answer.borrow().is_some();
        // `has_changed` fails once the sender is gone.
        let running = !answered && slot.answer.has_changed().is_ok();
        let recent = answered && now.saturating_duration_since(slot.started) < every;
        if running || recent {
            return Turn::Join(slot.answer.clone(), slot.started);
        }
    }
    let (tx, rx) = watch::channel(None);
    slots.insert(
        id.to_string(),
        Slot {
            started: now,
            answer: rx,
        },
    );
    Turn::Lead(tx)
}

/// Wake `entry`, or share the wake running or started under `every` ago.
pub(crate) async fn wake_entry(
    entry: SelfHostedEntry,
    every: Duration,
    timeout: Duration,
) -> SelfHostedWake {
    let now = Instant::now();
    let turn = {
        let mut map = slots()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        take_turn(&mut map, &entry.id, now, every)
    };
    let (mut answer, shared, started) = match turn {
        Turn::Join(answer, started) => (answer, true, started),
        Turn::Lead(tx) => {
            let answer = tx.subscribe();
            tokio::spawn(async move {
                let result = wake_once(&entry, timeout).await;
                tx.send_replace(Some(result));
            });
            (answer, false, now)
        }
    };
    let got = answer.wait_for(Option::is_some).await.map(|v| v.clone());
    let mut result = match got {
        Ok(Some(result)) => result,
        _ => SelfHostedWake {
            error: Some("The wake stopped before the model answered.".into()),
            ..SelfHostedWake::default()
        },
    };
    result.shared = shared;
    result.started_ms_ago = ms(started);
    result
}

/// The last wake of `id` in `slots` at `now`: its age and answer. None when
/// there is none, or its task ended without an answer.
pub(crate) fn status_in(
    slots: &HashMap<String, Slot>,
    id: &str,
    now: Instant,
) -> Option<SelfHostedWakeStatus> {
    let slot = slots.get(id)?;
    let result = slot.answer.borrow().clone();
    if result.is_none() && slot.answer.has_changed().is_err() {
        return None;
    }
    Some(SelfHostedWakeStatus {
        started_ms_ago: ms_between(slot.started, now),
        result,
    })
}

/// The last wake of entry `id`: no call to the model.
pub fn status(id: &str) -> Option<SelfHostedWakeStatus> {
    let map = slots()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    status_in(&map, id, Instant::now())
}

/// Drop entry `id`'s last wake (Settings saved, switched or removed it). A
/// wake still running finishes for the calls already waiting on it; the
/// next call starts a new one.
pub fn forget(id: &str) {
    slots()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(id);
}

/// A chat's wake of the saved entry `id` (`wake_self_hosted_model`): the
/// person typed or sent in a chat on it, or pressed Start now.
pub async fn wake(id: String) -> Result<SelfHostedWake, String> {
    let entry = tokio::task::spawn_blocking(move || resolve_in(registry::list(), &id))
        .await
        .map_err(|e| e.to_string())??;
    Ok(wake_entry(entry, WAKE_EVERY, WAKE_TIMEOUT).await)
}

#[cfg(test)]
#[path = "self_hosted_wake_tests.rs"]
mod tests;
