//! Self-hosted models Q10 (2026-11-03): Hollow's own background model calls
//! on a self-hosted model, chosen per job kind, so private-data jobs (HR,
//! payroll, claims, recordings with personal data) can go to Qwen while the
//! rest stay where they are.
//!
//! - **The setting.** `self-hosted-jobs.json` in app data
//!   (`{"version":1,"jobs":{kind: entryId}}`): each [`JobKind`] runs "As now"
//!   (no line) or on one registry entry. A missing file is "As now" for
//!   every kind. A file that exists but can't be read or parsed, or has
//!   another version (or an app-data folder Hollow can't find), fails
//!   closed (review): every kind is skipped with a note until it is fixed,
//!   Settings says so and shows what it can still read of the routes, and
//!   every change is refused, so Hollow never overwrites it. Settings,
//!   Advanced → Background jobs changes it (main window
//!   only; the COO's `settings.agents.set_self_hosted_job` asks on its card). Both files are written
//!   under an flock on a `.lock` file beside them, so two Hollow processes
//!   never interleave a change.
//! - **Routing** ([`route`], [`decide`]): a kind with no entry runs exactly
//!   as before. A kind routed to an entry that is missing, switched off,
//!   of another API style (Q9: an `openai` entry runs on Codex, in chats
//!   only, so background jobs take `anthropic` entries alone), or in a list
//!   that can't be read is skipped with
//!   a note in the job's own status or log; it never runs on Anthropic
//!   instead. A routed job runs on the Claude harness on this Mac whatever
//!   the default agent is, with a key minted for it
//!   (`self_hosted::run_for`, [`JOB_KEY_TTL`]) and revoked when it ends.
//! - **Wake and batch.** One-shot timeouts (45 to 300 s) are shorter than a
//!   cold start (4 to 6 minutes), so a routed job runs at once only when its
//!   entry answered within [`WARM_FOR`] (`self_hosted_forwarder::last_answered`).
//!   Otherwise it waits in a per-entry queue (`self-hosted-job-queue.json` in
//!   app data, private, so a quit doesn't lose it; each job is a [`Job`]
//!   holding what reruns it). A drain ([`drain_with`]) first drops jobs no
//!   longer wanted (a cheap check per kind: [`still_wanted`]), wakes the
//!   entry once with its health path (the forwarder waits up to 15 minutes)
//!   only if any are left, then runs them back to back while the entry
//!   counts as awake. A job leaves the queue only when its rerun succeeds;
//!   a failure keeps it with its attempts and last error and a backoff
//!   ([`backoff`]: no drain tries it again before then), and after
//!   [`MAX_ATTEMPTS`] it waits for the person (Settings: Try again or
//!   Discard); only a person's own request for it again starts it over, an
//!   automatic one never does ([`Asked`]). A drain starts when the oldest job has waited
//!   [`DRAIN_AFTER`], when the person presses Run them now, or right after
//!   the entry answers for any other reason (a chat turn on it: the
//!   forwarder's [`self_hosted_forwarder::answered_signal`]). There is no
//!   keep-warm loop and no wake without queued work.
//! - **One rerun at a time** (review round 2). A drain holds a flock on its
//!   entry's drain lock, so two Hollow processes never drain one entry
//!   together. Each rerun first claims its job in the queue file
//!   ([`Claim`]: read again, still waiting, not running in any process),
//!   so Discard, Run as now and a drain never run or drop a job another
//!   one is running. Each rerun is also pinned ([`Pinned`]) to the entry
//!   being drained (Run as now: to As now): its entry point decides again
//!   from the setting, and a kind that no longer goes there is held, never
//!   run As now by a drain, and that is not a failed attempt (it backs off
//!   30 minutes, so no hold makes a drain loop).
//! - **A kind moved.** Sending a kind to another entry moves its waiting
//!   jobs there; sending it back to As now holds them (never sent to
//!   Anthropic by a drain): Settings lists them with Run as now and
//!   Discard.
//! - **A self-hosted chat's own** retitle and learning extraction (Q5 skips
//!   them) run on that chat's entry when Chat titles or Learning extraction
//!   routes to that same entry (`only_on`), and stay skipped otherwise; a
//!   queued one never moves to another model ([`place`]).
use crate::self_hosted::{TurnRun, SETTINGS_PLACE};
use crate::self_hosted_forwarder as forwarder;
use crate::self_hosted_registry::{self as registry, SelfHostedEntry};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// The setting's file name inside the app-data folder.
pub const JOBS_FILE: &str = "self-hosted-jobs.json";
/// The queue's file name inside the app-data folder.
pub const QUEUE_FILE: &str = "self-hosted-job-queue.json";
/// The schema version of both files.
pub const VERSION: u64 = 1;
/// How recently an entry must have answered for a job to run on it at once.
pub const WARM_FOR: Duration = Duration::from_secs(8 * 60);
/// How long the oldest queued job waits before its entry is woken.
pub const DRAIN_AFTER: Duration = Duration::from_secs(30 * 60);
/// How long a job's run key lives at most (it is revoked when the job ends).
pub const JOB_KEY_TTL: Duration = Duration::from_secs(60 * 60);
/// The most jobs the queue holds, for every entry together.
pub const MAX_QUEUED: usize = 200;
/// Failed reruns after which a job waits for the person.
pub const MAX_ATTEMPTS: u32 = 3;
/// How long a claim on a job counts at most, whatever its process says
/// (a rerun's key lives an hour at most).
const CLAIM_STALE: Duration = Duration::from_secs(2 * 60 * 60);
/// What a queued learning extraction keeps of each side of the exchange
/// (the extractor reads no more: `vault_learning::extraction_prompt`).
pub const TEXT_KEPT: usize = 8_000;
/// Emitted when the queue or a drain changes (Settings reads the view again).
pub const CHANGED_EVENT: &str = "self-hosted-jobs-changed";
/// The drain loop's tick, and its first one after launch.
const TICK: Duration = Duration::from_secs(60);
const FIRST_TICK: Duration = Duration::from_secs(90);
/// Most of a health answer read.
const HEALTH_BYTES: usize = 4 * 1024;
/// Where the person chooses a job kind's model, named in notes.
const JOBS_PLACE: &str = "Settings → Advanced → Background jobs";

/// A kind of Hollow's own model call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "SelfHostedJobKind"))]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// A kept recording's (or a chat range's) proposals (`extract.rs`,
    /// Keep transcript and `extract.run`).
    Extraction,
    /// A chat's model retitle (`chat_title.rs`) and the session list's
    /// titles (`sessions.rs`).
    Titles,
    /// The learning extraction after a chat turn (`vault_learning.rs`).
    Learning,
    /// Meeting recaps and prep packs (`meeting_recap.rs`, `prep_pack.rs`:
    /// Prepare, Recap and a check-in's prep pack).
    Meetings,
    /// The About-you scan (`profile_builder.rs`).
    Profile,
    /// The heartbeat (`heartbeat.rs`).
    Heartbeat,
}

impl JobKind {
    pub const ALL: [JobKind; 6] = [
        JobKind::Extraction,
        JobKind::Titles,
        JobKind::Learning,
        JobKind::Meetings,
        JobKind::Profile,
        JobKind::Heartbeat,
    ];

    /// The key in `self-hosted-jobs.json`.
    pub fn id(self) -> &'static str {
        match self {
            JobKind::Extraction => "extraction",
            JobKind::Titles => "titles",
            JobKind::Learning => "learning",
            JobKind::Meetings => "meetings",
            JobKind::Profile => "profile",
            JobKind::Heartbeat => "heartbeat",
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_id(id: &str) -> Option<JobKind> {
        JobKind::ALL.into_iter().find(|kind| kind.id() == id)
    }

    /// What Settings calls it.
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Extraction => "Recording extraction",
            JobKind::Titles => "Chat and session titles",
            JobKind::Learning => "Learning extraction",
            JobKind::Meetings => "Meeting recaps and prep packs",
            JobKind::Profile => "About-you scan",
            JobKind::Heartbeat => "Heartbeat",
        }
    }

    /// What it covers, one line.
    pub fn about(self) -> &'static str {
        match self {
            JobKind::Extraction => "Promises, tasks, decisions and facts found in a kept recording transcript or a chat range.",
            JobKind::Titles => "A new chat's model title after its first answer, and titles in the session list.",
            JobKind::Learning => "The learnings Hollow proposes after your own chat turns.",
            JobKind::Meetings => "Prepare, Recap and the prep pack a check-in writes before a meeting.",
            JobKind::Profile => "The About-you scan that reads what your agents know about you.",
            JobKind::Heartbeat => "The background check of your vault every few minutes.",
        }
    }
}

// ── The setting ─────────────────────────────────────────────────────────

fn app_file(name: &str) -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return Ok(dir.join(name));
    }
    Ok(crate::agent_settings::app_data_dir()?.join(name))
}

#[cfg(test)]
thread_local! {
    /// A test's own folder for both files, on its own thread only, so a
    /// test of a call site never touches another test's (or the app's).
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Point this thread's [`app_file`] at `dir` until the guard drops (tests).
#[cfg(test)]
pub(crate) fn test_dir(dir: &Path) -> TestDir {
    TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.to_path_buf()));
    TestDir
}

/// Resets [`test_dir`] when dropped.
#[cfg(test)]
pub(crate) struct TestDir;

#[cfg(test)]
impl Drop for TestDir {
    fn drop(&mut self) {
        TEST_DIR.with(|d| *d.borrow_mut() = None);
    }
}

/// An exclusive flock on `<path>.lock`, held until dropped, so another
/// Hollow process never changes the file meanwhile (the file itself is
/// replaced by each atomic write, so the lock lives beside it).
struct FileLock(#[allow(dead_code)] crate::flock::HeldLock);

fn file_lock(path: &Path) -> Result<FileLock, String> {
    let lock = PathBuf::from(format!("{}.lock", path.display()));
    lock_at(&lock, true)?.ok_or_else(|| format!("Lock {}: busy", lock.display()))
}

/// An exclusive flock on `lock` (created when missing): waiting for it when
/// `wait`, else `None` when another descriptor holds it.
fn lock_at(lock: &Path, wait: bool) -> Result<Option<FileLock>, String> {
    if let Some(dir) = lock.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Lock {}: {e}", lock.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock)
        .map_err(|e| format!("Lock {}: {e}", lock.display()))?;
    // `HeldLock` unlocks explicitly on drop: a bare file would keep the lock
    // while a child another thread is spawning still holds a copy of its
    // descriptor, and the next drain read "busy" and did nothing (the flaky
    // `a_drain_that_cant_wake_or_run_its_entry_leaves_the_jobs`).
    if wait {
        return crate::flock::HeldLock::take(file)
            .map(|held| Some(FileLock(held)))
            .map_err(|e| format!("Lock {}: {e}", lock.display()));
    }
    match crate::flock::HeldLock::try_take(file) {
        Ok(held) => Ok(Some(FileLock(held))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(format!("Lock {}: {e}", lock.display())),
    }
}

/// Review round 2 (M3): a drain's own lock, one per entry beside the queue
/// (`<queue>.drain-<hash of the entry id>.lock`), taken without waiting:
/// `None` while another drain of that entry runs, in this Hollow process or
/// another.
fn try_drain_lock(queue: &Path, entry: &str) -> Result<Option<FileLock>, String> {
    // Review round 3 (N5): named by a hash of the id, so two ids never share
    // one lock.
    use sha2::{Digest, Sha256};
    let name = format!("{:x}", Sha256::digest(entry.as_bytes()));
    lock_at(
        &PathBuf::from(format!("{}.drain-{}.lock", queue.display(), &name[..16])),
        false,
    )
}

/// Whether a drain of `entry` runs now, in any Hollow process.
pub(crate) fn drain_busy_at(queue: &Path, entry: &str) -> Result<bool, String> {
    Ok(try_drain_lock(queue, entry)?.is_none())
}

/// Reads and writes of the setting, one at a time.
static JOBS_LOCK: Mutex<()> = Mutex::new(());

fn jobs_locked() -> std::sync::MutexGuard<'static, ()> {
    JOBS_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

#[derive(Deserialize)]
struct JobsIn {
    #[serde(default)]
    jobs: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct JobsOut<'a> {
    version: u64,
    jobs: &'a BTreeMap<String, String>,
}

/// Check a file's `version`: this one, or why Hollow won't read it.
fn versioned(name: &str, bytes: &[u8]) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("{name} isn't valid JSON ({e}). Fix it or move it aside."))?;
    match value.get("version").and_then(serde_json::Value::as_u64) {
        Some(VERSION) => Ok(value),
        Some(other) => Err(format!(
            "{name} is version {other}; this Hollow reads version {VERSION}. Update Hollow, or move the file aside."
        )),
        None => Err(format!(
            "{name} has no version number, so Hollow won't read or change it. Fix it or move it aside."
        )),
    }
}

/// The saved setting, kind id to entry id (unknown keys kept as they are):
/// empty when the file is missing; an error naming the problem otherwise.
pub(crate) fn read_jobs_at(path: &Path) -> Result<BTreeMap<String, String>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{JOBS_FILE} can't be read: {e}")),
    };
    let value = versioned(JOBS_FILE, &bytes)?;
    let file: JobsIn = serde_json::from_value(value).map_err(|e| {
        format!("{JOBS_FILE} doesn't match version {VERSION} ({e}). Fix it or move it aside.")
    })?;
    Ok(file.jobs)
}

/// What a setting that can't be read still says, for Settings only (review:
/// a damaged file holds every kind, without hiding what was routed): the
/// `jobs` map of a file that parses as JSON whatever its version, else
/// nothing. Never used to route a job.
fn peek_jobs_at(path: &Path) -> BTreeMap<String, String> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("jobs").and_then(|j| j.as_object()).cloned())
        .map(|jobs| {
            jobs.into_iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k, v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// The entry `kind` goes to in `jobs`, if any.
fn entry_for(jobs: &BTreeMap<String, String>, kind: JobKind) -> Option<&str> {
    jobs.get(kind.id())
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
}

/// Send `kind` to `entry` (`None`: As now). The entry must be in the list,
/// switched on and `anthropic` (background jobs run on Claude; Q9). A
/// setting that can't be read refuses every change and is never
/// overwritten.
pub(crate) fn set_at(
    path: &Path,
    entries: Result<Vec<SelfHostedEntry>, String>,
    kind: JobKind,
    entry: Option<&str>,
) -> Result<(), String> {
    let entry = entry.map(str::trim).filter(|id| !id.is_empty());
    if let Some(id) = entry {
        if let Some(why) = reason(id, &entries) {
            return Err(format!(
                "{} can't go there: {why}. Nothing changed.",
                kind.label()
            ));
        }
    }
    let _held = jobs_locked();
    let _file = file_lock(path)?;
    let mut jobs = read_jobs_at(path)?;
    match entry {
        Some(id) => jobs.insert(kind.id().to_string(), id.to_string()),
        None => jobs.remove(kind.id()),
    };
    let mut bytes = serde_json::to_vec_pretty(&JobsOut {
        version: VERSION,
        jobs: &jobs,
    })
    .map_err(|e| format!("Encode {JOBS_FILE}: {e}"))?;
    bytes.push(b'\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Save {JOBS_FILE}: {e}"))?;
    }
    crate::vault::atomic_write_private(path, &bytes).map_err(|e| format!("Save {JOBS_FILE}: {e}"))
}

/// Why jobs sent to entry `id` can't run on it now, or `None` when they
/// can: the list can't be read, there is no such entry, it is switched
/// off, or it uses an API style background jobs can't run (Q9: an `openai`
/// entry runs on Codex, in chats only).
pub(crate) fn reason(id: &str, entries: &Result<Vec<SelfHostedEntry>, String>) -> Option<String> {
    let entries = match entries {
        Ok(entries) => entries,
        Err(why) => {
            return Some(format!(
                "the self-hosted models list can't be read ({})",
                why.trim().trim_end_matches('.')
            ))
        }
    };
    let Some(entry) = entries.iter().find(|e| e.id == id) else {
        return Some(format!(
            "there is no self-hosted model \"{id}\" in {SETTINGS_PLACE}"
        ));
    };
    if !entry.enabled {
        return Some(format!("{} is switched off", entry.name));
    }
    // Self-hosted models Q9: an `openai` entry runs on Codex, in chats
    // only; background jobs are Claude one-shots, so they take
    // `anthropic` entries alone, and a kind whose entry became `openai` is
    // held (skipped with this note), never run As now.
    match crate::self_hosted::harness(&entry.api_style) {
        Some(crate::agent::AgentKind::Claude) => None,
        Some(crate::agent::AgentKind::Codex) => Some(format!(
            "{} uses the openai API style (OpenAI's Responses API), which runs on Codex in chats only; background jobs run on Claude, so choose a model with Anthropic's Messages API",
            entry.name
        )),
        _ => Some(format!(
            "{} uses the {} API style, which Hollow can't run",
            entry.name, entry.api_style
        )),
    }
}

// ── Routing ─────────────────────────────────────────────────────────────

/// Why a routed job didn't run now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// It waits in its entry's queue (`added` false: the same job was
    /// already waiting).
    Queued { message: String, added: bool },
    /// It was skipped, never run on another model.
    Skipped(String),
}

impl Held {
    pub fn message(&self) -> &str {
        match self {
            Held::Queued { message, .. } | Held::Skipped(message) => message,
        }
    }

    pub fn into_message(self) -> String {
        match self {
            Held::Queued { message, .. } | Held::Skipped(message) => message,
        }
    }
}

/// A routed job whose entry answered recently: run it now
/// ([`attach`]). `since` is the entry's stop count read before the
/// registry, so a switch-off meanwhile refuses the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warm {
    pub entry: SelfHostedEntry,
    pub since: u64,
}

/// What a job of a kind runs on now, decided without starting anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The job's own path, exactly as before Q10.
    AsNow,
    Warm(Warm),
    Held(Held),
}

/// What a job of a kind runs on now, with its run when it runs on an entry.
#[derive(Debug)]
pub enum Routed {
    AsNow,
    /// Its endpoint and the key minted for it; dropping it revokes the key.
    Run(TurnRun),
    Held(Held),
}

/// Where [`decide_with`] reads and writes, and what it asks.
pub(crate) struct Env<'a> {
    pub jobs: &'a Path,
    pub queue: &'a Path,
    pub entries: &'a dyn Fn() -> Result<Vec<SelfHostedEntry>, String>,
    pub warm: &'a dyn Fn(&str) -> bool,
    pub now: DateTime<Utc>,
    pub asked: Asked,
}

/// Who asked for a job (review round 4, R1). Only a person's request
/// (Prepare, Recap, Keep transcript, an About-you scan they started; the
/// `extract_run` command counts too, though no screen calls it; Try again
/// is its own way) starts over a queued job that gave up. An automatic one
/// (a check-in's or agent's call, the end of a chat turn, the session
/// list's title backfill, the About-you and heartbeat timers, the
/// heartbeat's Run now) merges into it ([`Job::merged`]) and leaves it for
/// Try again, so a job a model can never do doesn't wake it three times
/// every few hours forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    Person,
    Auto,
}

/// Whether `entry` answered within [`WARM_FOR`] of `now` (wall clock).
pub(crate) fn warm_at(entry: &str, now: SystemTime) -> bool {
    forwarder::last_answered(entry)
        .is_some_and(|at| now.duration_since(at).map_or(true, |age| age < WARM_FOR))
}

/// Review: a setting that exists but can't be read holds every kind.
fn damaged_note(kind: JobKind, why: &str) -> String {
    format!(
        "{} was skipped: background jobs are held while {JOBS_FILE} can't be read ({}). Nothing ran on another model.",
        kind.label(),
        why.trim().trim_end_matches('.')
    )
}

fn own_note(kind: JobKind, own: &str) -> String {
    format!(
        "{} for this chat stays skipped: the chat runs on the self-hosted model \"{own}\", and {} doesn't go to it in {JOBS_PLACE}.",
        kind.label(),
        kind.label()
    )
}

fn skipped_note(kind: JobKind, why: &str) -> String {
    format!(
        "{} was skipped: it goes to a self-hosted model in {JOBS_PLACE}, and {why}. It never runs on another model instead.",
        kind.label()
    )
}

fn queued_note(kind: JobKind, name: &str) -> String {
    format!(
        "{} waits for {name}, which is asleep: it runs when Hollow wakes it (within {} minutes), or when you press Run them now in {JOBS_PLACE}.",
        kind.label(),
        DRAIN_AFTER.as_secs() / 60
    )
}

/// What a request that waits says, from the job it joined (review round 4,
/// R2): one backing off after a failure names the time Hollow tries it
/// again, and one that gave up says only Try again runs it. Q10 follow-up:
/// a drain of the model that stopped with an error holds every drain of it
/// until its own retry time (`drain_retry`, [`DrainState::finish`]), so the
/// later of the two times is named, with its day when it isn't today
/// ([`local_when`]).
fn waiting_note(
    kind: JobKind,
    name: &str,
    q: &Queued,
    drain_retry: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> String {
    if q.gave_up() {
        return format!(
            "{} waits for {name}: it failed there {} times, so it runs again only when you press Try again in {JOBS_PLACE}.",
            kind.label(),
            q.attempts
        );
    }
    let own = q.retry_at.filter(|at| *at > now);
    let drain = drain_retry.filter(|at| *at > now);
    match (own, drain) {
        (Some(at), drain) if drain.is_none_or(|d| d <= at) => format!(
            "{} waits for {name}: it failed there before, so Hollow tries it again {} (or when you press Run them now in {JOBS_PLACE}).",
            kind.label(),
            local_when(at, now)
        ),
        (_, Some(at)) => format!(
            "{} waits for {name}: Hollow's last run of its waiting jobs stopped with an error, so Hollow tries again {} (or when you press Run them now in {JOBS_PLACE}).",
            kind.label(),
            local_when(at, now)
        ),
        _ => queued_note(kind, name),
    }
}

/// `at` in local time for a sentence, said from `now`: "at 14:05" today,
/// "tomorrow at 00:30", else "on 5 Nov at 09:00" (Q10 follow-up: a bare
/// time after midnight read as earlier today).
fn local_when(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let at = at.with_timezone(&chrono::Local);
    let today = now.with_timezone(&chrono::Local).date_naive();
    let time = at.format("%H:%M");
    if at.date_naive() == today {
        format!("at {time}")
    } else if today.succ_opt() == Some(at.date_naive()) {
        format!("tomorrow at {time}")
    } else {
        format!("on {} at {time}", at.format("%-d %b"))
    }
}

/// Review round 2 (B1): a pinned rerun whose kind no longer goes where it
/// was sent.
fn pinned_note(kind: JobKind, to: Option<&str>, now: Option<&str>) -> String {
    let label = kind.label();
    match (to, now) {
        (Some(to), None) => format!(
            "{label} was held: it waited for \"{to}\", and {label} runs As now since, so it waits for you in {JOBS_PLACE} (Run as now or Discard). Nothing ran on another model."
        ),
        (Some(to), Some(now)) => format!(
            "{label} was held: it waited for \"{to}\", and {label} goes to \"{now}\" now, so it moves there. Nothing ran on another model."
        ),
        (None, Some(now)) => format!(
            "{label} didn't run as now: {label} goes to \"{now}\" now, so it runs there."
        ),
        (None, None) => format!("{label} was held."),
    }
}

// ── Pinned reruns ───────────────────────────────────────────────────────

/// Where a queued job's rerun may run (review round 2, B1): the entry a
/// drain is draining (`Some`), or As now for the person's Run as now
/// (`None`). A rerun goes through the job's own entry point, which decides
/// again from the setting, and a cold start can take 15 minutes: a pinned
/// decision runs the job only where it was sent and holds it otherwise.
struct Pin {
    kind: JobKind,
    to: Option<String>,
    /// Why routing held a decision for this job, once it did.
    held: Mutex<Option<String>>,
    /// A decision for this job ran it (As now or on the entry): review
    /// round 3, N3, so another call's hold never marks a rerun that ran.
    ran: std::sync::atomic::AtomicBool,
}

/// A pin's key: the queue file and the job's [`Job::key`].
type PinKey = (PathBuf, String);

fn pins() -> std::sync::MutexGuard<'static, HashMap<PinKey, Arc<Pin>>> {
    static PINS: OnceLock<Mutex<HashMap<PinKey, Arc<Pin>>>> = OnceLock::new();
    PINS.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// One job's rerun pinned until dropped. The pin is found by the job's key
/// in [`decide_with`], not carried by the task, so it holds through every
/// entry point, those that decide on a blocking thread (Prepare, Recap)
/// included. A fresh call of the same job meanwhile is held the same way.
pub(crate) struct Pinned {
    key: PinKey,
    pin: Arc<Pin>,
}

impl Pinned {
    /// Pin reruns of `job` in queue `queue` to `to` (`None`: As now);
    /// `None` when the same job is pinned already (another rerun of it).
    pub(crate) fn new(queue: &Path, job: &Job, to: Option<&str>) -> Option<Pinned> {
        let key = (queue.to_path_buf(), job.key());
        let mut pins = pins();
        if pins.contains_key(&key) {
            return None;
        }
        let pin = Arc::new(Pin {
            kind: job.kind(),
            to: to.map(str::to_string),
            held: Mutex::new(None),
            ran: Default::default(),
        });
        pins.insert(key.clone(), pin.clone());
        Some(Pinned { key, pin })
    }

    /// Why routing held a decision for the job, if it did.
    pub(crate) fn held(&self) -> Option<String> {
        self.pin
            .held
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Whether a decision for the job ran it.
    pub(crate) fn ran(&self) -> bool {
        self.pin.ran.load(Ordering::SeqCst)
    }
}

impl Drop for Pinned {
    fn drop(&mut self) {
        let mut pins = pins();
        if pins
            .get(&self.key)
            .is_some_and(|p| Arc::ptr_eq(p, &self.pin))
        {
            pins.remove(&self.key);
        }
    }
}

/// What a job of `kind` runs on, deciding and queuing but starting nothing.
/// `only_on` is a self-hosted chat's own entry (its retitle or learning):
/// the job runs there or not at all. `pending` is the job as the queue
/// keeps it, asked for only when it is queued (`None`: it can't wait), or
/// up front while a rerun of this kind on this queue is pinned
/// ([`Pinned`]), to find its pin by the job's key. A pinned job runs only
/// where its pin says, and each of its held outcomes is noted on the pin.
/// The registry is read only for a routed kind.
pub(crate) fn decide_with(
    kind: JobKind,
    only_on: Option<&str>,
    pending: impl FnOnce() -> Option<Job>,
    env: &Env<'_>,
) -> Decision {
    let mut pending = Some(pending);
    let pinned_kind = pins()
        .iter()
        .any(|((queue, _), pin)| pin.kind == kind && queue.as_path() == env.queue);
    let early = if pinned_kind {
        Some(pending.take().and_then(|job| job()))
    } else {
        None
    };
    let pin = early
        .as_ref()
        .and_then(Option::as_ref)
        .and_then(|job| pins().get(&(env.queue.to_path_buf(), job.key())).cloned());
    let pending = move || match early {
        Some(job) => job,
        None => pending.and_then(|job| job()),
    };
    let decision = decide_pinned(kind, only_on, pending, env, pin.as_deref());
    if let Some(pin) = &pin {
        match &decision {
            Decision::Held(held) => {
                *pin.held.lock().unwrap_or_else(|p| p.into_inner()) =
                    Some(held.message().to_string());
            }
            Decision::AsNow | Decision::Warm(_) => pin.ran.store(true, Ordering::SeqCst),
        }
    }
    decision
}

/// [`decide_with`] once the pin is known.
fn decide_pinned(
    kind: JobKind,
    only_on: Option<&str>,
    pending: impl FnOnce() -> Option<Job>,
    env: &Env<'_>,
    pin: Option<&Pin>,
) -> Decision {
    // Review: fail closed. Only a missing file is As now; one that exists
    // but can't be read could be sending this kind to a private model.
    let jobs = match read_jobs_at(env.jobs) {
        Ok(jobs) => jobs,
        Err(why) => return Decision::Held(Held::Skipped(damaged_note(kind, &why))),
    };
    // Review round 2 (B1): a rerun runs only where it was sent. A drain's
    // rerun of a kind sent back to As now (or elsewhere) meanwhile is held,
    // never run As now; Run as now's of a kind routed meanwhile, the same.
    if let Some(pin) = pin {
        let now = entry_for(&jobs, kind);
        if now != pin.to.as_deref() {
            return Decision::Held(Held::Skipped(pinned_note(kind, pin.to.as_deref(), now)));
        }
    }
    let Some(id) = entry_for(&jobs, kind) else {
        return match only_on {
            Some(own) => Decision::Held(Held::Skipped(own_note(kind, own))),
            None => Decision::AsNow,
        };
    };
    if only_on.is_some_and(|own| own != id) {
        return Decision::Held(Held::Skipped(own_note(kind, only_on.unwrap_or_default())));
    }
    let since = forwarder::stops(id);
    let entries = (env.entries)();
    if let Some(why) = reason(id, &entries) {
        return Decision::Held(Held::Skipped(skipped_note(kind, &why)));
    }
    let Some(entry) = entries
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.id == id))
    else {
        return Decision::Held(Held::Skipped(skipped_note(kind, "it isn't in the list")));
    };
    if (env.warm)(id) {
        return Decision::Warm(Warm { entry, since });
    }
    // A pinned rerun never queues itself again (it is in the queue, and a
    // merge would clear its attempts): held, it stays where it is.
    if pin.is_some() {
        return Decision::Held(Held::Skipped(skipped_note(
            kind,
            &format!("{} went back to sleep", entry.name),
        )));
    }
    let Some(job) = pending() else {
        return Decision::Held(Held::Skipped(skipped_note(
            kind,
            &format!(
                "{} is asleep and this job can't wait in its queue",
                entry.name
            ),
        )));
    };
    match enqueue_as_at(env.queue, &entry.id, job, env.asked, env.now) {
        Ok(Enqueued { added, queued }) => {
            if added {
                changed();
            }
            let drain_retry = drains().retry_at.get(&entry.id).copied();
            Decision::Held(Held::Queued {
                message: waiting_note(kind, &entry.name, &queued, drain_retry, env.now),
                added,
            })
        }
        Err(why) => Decision::Held(Held::Skipped(skipped_note(
            kind,
            &format!(
                "{} is asleep and the job couldn't be queued ({why})",
                entry.name
            ),
        ))),
    }
}

/// [`decide_with`] on the app's files, the registry and the forwarders,
/// for a request `asked` by a person or automatically.
pub fn decide(
    kind: JobKind,
    only_on: Option<&str>,
    asked: Asked,
    pending: impl FnOnce() -> Option<Job>,
) -> Decision {
    // Review: without its folder Hollow can't know where a kind goes, so
    // every kind is held, as for a damaged file.
    let (jobs, queue) = match (app_file(JOBS_FILE), app_file(QUEUE_FILE)) {
        (Ok(jobs), Ok(queue)) => (jobs, queue),
        (Err(why), _) | (_, Err(why)) => {
            return Decision::Held(Held::Skipped(damaged_note(kind, &why)))
        }
    };
    let now = SystemTime::now();
    decide_with(
        kind,
        only_on,
        pending,
        &Env {
            jobs: &jobs,
            queue: &queue,
            entries: &registry::entries,
            warm: &|id: &str| warm_at(id, now) || awake(id),
            now: Utc::now(),
            asked,
        },
    )
}

/// A warm job's run: the entry's forwarder (started if it isn't running;
/// refused when a stop came after `since`), the registry read again (an
/// entry now off or gone stops its forwarder and refuses), and a key
/// minted for [`JOB_KEY_TTL`].
pub(crate) async fn attach_with(
    warm: Warm,
    entries: impl Fn() -> Result<Vec<SelfHostedEntry>, String>,
) -> Result<TurnRun, String> {
    let Warm { entry, since } = warm;
    let fwd = forwarder::ensure_since(entry.upstream(), since)
        .await
        .map_err(|e| {
            format!(
                "Hollow's local forwarder for {} didn't start, so the job didn't run: {e}",
                entry.name
            )
        })?;
    let now = entries();
    if let Some(why) = reason(&entry.id, &now) {
        forwarder::stop(&entry.id);
        return Err(format!("The job didn't run: {why}."));
    }
    let Some(now) = now
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.id == entry.id))
    else {
        return Err("The job didn't run: its model left the list.".into());
    };
    if now.upstream().normalized() != entry.upstream().normalized() {
        return Err(format!(
            "{} changed in {SETTINGS_PLACE} while the job was starting, so it didn't run.",
            now.name
        ));
    }
    Ok(crate::self_hosted::run_for(&now, fwd, JOB_KEY_TTL))
}

/// [`attach_with`] on the registry.
pub async fn attach(warm: Warm) -> Result<TurnRun, String> {
    attach_with(warm, registry::entries).await
}

/// What a job of `kind` runs on now ([`decide`], then [`attach`] for a warm
/// entry; a failed start is a skip with its reason).
pub async fn route(
    kind: JobKind,
    only_on: Option<&str>,
    asked: Asked,
    pending: impl FnOnce() -> Option<Job>,
) -> Routed {
    match decide(kind, only_on, asked, pending) {
        Decision::AsNow => Routed::AsNow,
        Decision::Held(held) => Routed::Held(held),
        Decision::Warm(warm) => match attach(warm).await {
            Ok(run) => Routed::Run(run),
            Err(why) => Routed::Held(Held::Skipped(why)),
        },
    }
}

/// A one-shot on a routed job's run: Claude Code on this Mac, the served
/// model, the forwarder and the job's key, nothing else of `opts` changed.
pub async fn run_on(
    run: &TurnRun,
    prompt: &str,
    opts: crate::claude_runner::RunOpts<'_>,
) -> Result<String, String> {
    let (agent, model, opts) = on_run(run, opts);
    crate::claude_runner::run_oneshot_selected(agent, model, prompt, opts).await
}

/// What [`run_on`] hands the one-shot runner: Claude, the served model, and
/// `opts` with the run's target.
pub(crate) fn on_run<'a>(
    run: &TurnRun,
    opts: crate::claude_runner::RunOpts<'a>,
) -> (
    crate::agent::AgentKind,
    Option<String>,
    crate::claude_runner::RunOpts<'a>,
) {
    (
        crate::agent::AgentKind::Claude,
        Some(run.target.served_model.clone()),
        crate::claude_runner::RunOpts {
            self_hosted: Some(run.target.clone()),
            ..opts
        },
    )
}

/// Whether `kind` goes to `entry` now (a self-hosted chat's own retitle and
/// learning, Q10's decision 5). A setting that can't be read says no.
pub fn routes_to(kind: JobKind, entry: &str) -> bool {
    app_file(JOBS_FILE).is_ok_and(|path| routes_to_at(&path, kind, entry))
}

pub(crate) fn routes_to_at(path: &Path, kind: JobKind, entry: &str) -> bool {
    read_jobs_at(path).is_ok_and(|jobs| entry_for(&jobs, kind) == Some(entry))
}

// ── The queue ───────────────────────────────────────────────────────────

/// A queued job: what reruns it once its entry is awake. Each reruns the
/// job's own entry point, which routes again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Job {
    /// `extract::run` on a recording transcript or a chat range (the
    /// extraction ledger keeps a rerun from repeating).
    Extraction {
        vault: PathBuf,
        source: crate::extract::Source,
    },
    /// A chat's model retitle after its first turn.
    ChatTitle {
        vault: PathBuf,
        session_key: String,
        turn_ord: i64,
        run_id: String,
        #[serde(default)]
        only_on: Option<String>,
    },
    /// A session-list title (`sessions::generate`).
    SessionTitle {
        projects: PathBuf,
        project_dir: String,
        session_id: String,
    },
    /// A chat turn's learning extraction, each side kept to [`TEXT_KEPT`].
    Learning {
        vault: PathBuf,
        user: String,
        #[serde(default)]
        references: Vec<String>,
        assistant: String,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        changed: bool,
        #[serde(default)]
        only_on: Option<String>,
        /// Host capture retains its source chat while a private model sleeps.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capture_session: Option<String>,
    },
    /// Prepare (the button, or a check-in's `agenda.prepare_meeting`).
    PrepPack {
        vault: PathBuf,
        meeting_id: String,
        #[serde(default)]
        regenerate: bool,
        #[serde(default)]
        expected_hash: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
    },
    /// Recap (`meeting_recap`).
    Recap {
        vault: PathBuf,
        meeting_id: String,
        around: DateTime<Utc>,
        #[serde(default)]
        regenerate: bool,
        #[serde(default)]
        recording_id: Option<String>,
        #[serde(default)]
        expected_hash: Option<String>,
    },
    /// An About-you scan.
    Profile { vault: PathBuf },
    /// A heartbeat beat, run inside the drain that woke its model.
    /// `manual_at`: when Settings' Run now asked for it (Q10 follow-up; none
    /// for a scheduled beat). A Run now skips the scheduled gates when it
    /// reruns, as it does when it runs at once, and is wanted only while no
    /// beat was admitted after it and no budget holds it
    /// (`heartbeat::manual_beat_unwanted`).
    Heartbeat {
        vault: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manual_at: Option<DateTime<Utc>>,
    },
}

fn kept(text: &str) -> String {
    text.chars().take(TEXT_KEPT).collect()
}

impl Job {
    /// A learning extraction, each side kept to [`TEXT_KEPT`].
    pub fn learning(
        vault: &Path,
        user: &str,
        references: &[String],
        assistant: &str,
        run_id: Option<&str>,
        changed: bool,
        only_on: Option<&str>,
    ) -> Job {
        Job::Learning {
            vault: vault.to_path_buf(),
            user: kept(user),
            references: references.iter().take(8).cloned().collect(),
            assistant: kept(assistant),
            run_id: run_id.map(str::to_string),
            changed,
            only_on: only_on.map(str::to_string),
            capture_session: None,
        }
    }

    pub fn kind(&self) -> JobKind {
        match self {
            Job::Extraction { .. } => JobKind::Extraction,
            Job::ChatTitle { .. } | Job::SessionTitle { .. } => JobKind::Titles,
            Job::Learning { .. } => JobKind::Learning,
            Job::PrepPack { .. } | Job::Recap { .. } => JobKind::Meetings,
            Job::Profile { .. } => JobKind::Profile,
            Job::Heartbeat { .. } => JobKind::Heartbeat,
        }
    }

    /// The same job waits once: a second one with this key replaces the
    /// first one's details and keeps its place.
    pub fn key(&self) -> String {
        let at = |vault: &Path| vault.to_string_lossy().into_owned();
        match self {
            Job::Extraction { vault, source } => format!(
                "extraction\u{0}{}\u{0}{}",
                at(vault),
                serde_json::to_string(source).unwrap_or_default()
            ),
            Job::ChatTitle {
                vault, session_key, ..
            } => format!("title\u{0}{}\u{0}{session_key}", at(vault)),
            Job::SessionTitle {
                project_dir,
                session_id,
                ..
            } => format!("session-title\u{0}{project_dir}\u{0}{session_id}"),
            Job::Learning {
                vault,
                run_id,
                user,
                capture_session,
                ..
            } => match (capture_session, run_id) {
                (Some(session), _) => format!("learning-capture\u{0}{}\u{0}{session}", at(vault)),
                (None, Some(run)) => format!("learning\u{0}{run}"),
                (None, None) => format!("learning\u{0}{}\u{0}{user}", at(vault)),
            },
            Job::PrepPack {
                vault, meeting_id, ..
            } => format!("prep\u{0}{}\u{0}{meeting_id}", at(vault)),
            Job::Recap {
                vault, meeting_id, ..
            } => format!("recap\u{0}{}\u{0}{meeting_id}", at(vault)),
            Job::Profile { vault } => format!("profile\u{0}{}", at(vault)),
            Job::Heartbeat { vault, .. } => format!("heartbeat\u{0}{}", at(vault)),
        }
    }

    /// What a waiting job becomes when the same job ([`Job::key`]) is asked
    /// for again (`newer`, `asked` by a person or automatically): the newer
    /// copy's details, except that nothing automatic weakens it (Q10
    /// follow-up). An automatic request merged into a Prepare or Recap
    /// rewrite keeps that rewrite whole (`regenerate` and the person's
    /// `expected_hash`), so a check-in's `agenda.prepare_meeting` never turns
    /// the person's rewrite into a request a kept pack answers, which a drain
    /// drops as no longer needed. (Every Recap is a person's today, from
    /// `meeting_recap`, so its half of the arm is defensive: it holds the
    /// same rule if an automatic Recap is ever added.) A heartbeat keeps the
    /// later Run now's time when either copy is one.
    pub fn merged(self, newer: Job, asked: Asked) -> Job {
        match (self, newer) {
            (
                older @ (Job::PrepPack {
                    regenerate: true, ..
                }
                | Job::Recap {
                    regenerate: true, ..
                }),
                _,
            ) if asked == Asked::Auto => older,
            (
                Job::Heartbeat { manual_at, .. },
                Job::Heartbeat {
                    vault,
                    manual_at: now,
                },
            ) => Job::Heartbeat {
                vault,
                manual_at: manual_at.max(now),
            },
            (_, newer) => newer,
        }
    }

    /// A self-hosted chat's own entry, for its retitle or learning: the
    /// job runs there or not at all (so never as now).
    pub fn only_on(&self) -> Option<&str> {
        match self {
            Job::ChatTitle { only_on, .. } | Job::Learning { only_on, .. } => only_on.as_deref(),
            _ => None,
        }
    }
}

/// One job in the queue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub id: String,
    /// The registry entry it waits for.
    pub entry: String,
    pub queued_at: DateTime<Utc>,
    pub job: Job,
    /// Failed reruns so far (review: a job leaves only when it succeeds).
    #[serde(default)]
    pub attempts: u32,
    /// The last failed rerun's error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Review round 2 (M2): after a failed rerun, the earliest a drain
    /// tries it again ([`backoff`]); Run them now doesn't wait for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<DateTime<Utc>>,
    /// Review round 2 (M3): a rerun of it is running, in this Hollow
    /// process or another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<Claim>,
}

/// Who is rerunning a queued job, and since when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub pid: u32,
    pub at: DateTime<Utc>,
    /// Review round 3 (N4): when that process started
    /// (`proc_info::own_start_micros`), so a pid reused by another process
    /// never counts as the claim's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
}

impl Queued {
    /// It failed [`MAX_ATTEMPTS`] times and waits for the person.
    pub fn gave_up(&self) -> bool {
        self.attempts >= MAX_ATTEMPTS
    }

    /// A failed rerun waits for its backoff until `retry_at`.
    pub fn backing_off(&self, now: DateTime<Utc>) -> bool {
        self.retry_at.is_some_and(|at| now < at)
    }

    /// A rerun of it is running now: its claim is younger than
    /// [`CLAIM_STALE`], and the process that made it is still there (this
    /// one: the claim is still held here). A claim left by a crash or a
    /// quit doesn't count, nor one whose pid another process reuses.
    pub fn running(&self, now: DateTime<Utc>) -> bool {
        let Some(claim) = &self.claim else {
            return false;
        };
        let stale = chrono::Duration::from_std(CLAIM_STALE).unwrap_or_default();
        if now.signed_duration_since(claim.at) > stale {
            return false;
        }
        claim_alive(claim, &self.id)
    }
}

/// Whether `claim`'s process made it and is still there (review round 3,
/// N4): this process while it holds the claim; another one while its pid
/// names a process that started when the claim says. A claim without a
/// start time (a platform Hollow can't read it on) counts while its pid
/// exists and answers this user's signal; a permission error doesn't count.
fn claim_alive(claim: &Claim, id: &str) -> bool {
    if claim.pid == std::process::id() {
        return claim.started == crate::proc_info::own_start_micros()
            && claimed_here().contains(id);
    }
    let Ok(pid) = i32::try_from(claim.pid) else {
        return false;
    };
    match (claim.started, crate::proc_info::process_start_micros(pid)) {
        (Some(was), Some(is)) => was == is,
        (Some(_), None) => false,
        (None, _) => crate::proc_info::pid_alive(pid) == Some(true),
    }
}

/// How long a job waits after its `attempts`th failed rerun before a drain
/// tries it again: [`DRAIN_AFTER`] times the attempts (30, then 60 minutes).
pub fn backoff(attempts: u32) -> chrono::Duration {
    chrono::Duration::from_std(DRAIN_AFTER).unwrap_or_default() * attempts.max(1) as i32
}

/// The jobs this process is rerunning now (their claims count as running).
fn claimed_here() -> std::sync::MutexGuard<'static, HashSet<String>> {
    static CLAIMED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CLAIMED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// This process's claim on a queued job: while it lives, the job's claim
/// in the file counts as running. Its end in the file is the rerun's
/// ([`take_at`], [`fail_at`], [`release_at`]).
pub(crate) struct Claimed(String);

impl Drop for Claimed {
    fn drop(&mut self) {
        claimed_here().remove(&self.0);
    }
}

/// Where a queued job stands under the setting `jobs`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    /// Its kind still goes to the entry it waits for.
    Here,
    /// Its kind goes to another entry now: a drain moves it there.
    Elsewhere(String),
    /// Its kind runs As now: it waits for the person (Run as now, Discard),
    /// never sent anywhere by a drain.
    Held,
}

pub(crate) fn place(jobs: &BTreeMap<String, String>, q: &Queued) -> Place {
    let now = entry_for(jobs, q.job.kind());
    // Review round 3 (N1): a self-hosted chat's own job runs on its chat's
    // model or not at all. Its kind sent to another model holds it for the
    // person (Discard), never moves it there.
    if let Some(own) = q.job.only_on() {
        return match now {
            Some(id) if id == own && id == q.entry => Place::Here,
            Some(id) if id == own => Place::Elsewhere(own.to_string()),
            _ => Place::Held,
        };
    }
    match now {
        Some(id) if id == q.entry => Place::Here,
        Some(id) => Place::Elsewhere(id.to_string()),
        None => Place::Held,
    }
}

#[derive(Deserialize)]
struct QueueIn {
    #[serde(default)]
    jobs: Vec<Queued>,
}

#[derive(Serialize)]
struct QueueOut<'a> {
    version: u64,
    jobs: &'a [Queued],
}

/// Reads and writes of the queue, one at a time.
static QUEUE_LOCK: Mutex<()> = Mutex::new(());

fn queue_locked() -> std::sync::MutexGuard<'static, ()> {
    QUEUE_LOCK.lock().unwrap_or_else(|p| p.into_inner())
}

/// The queued jobs, oldest first: empty when the file is missing; an error
/// naming the problem otherwise (never reset: that would lose the jobs).
pub(crate) fn read_queue_at(path: &Path) -> Result<Vec<Queued>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{QUEUE_FILE} can't be read: {e}")),
    };
    let value = versioned(QUEUE_FILE, &bytes)?;
    let file: QueueIn = serde_json::from_value(value).map_err(|e| {
        format!("{QUEUE_FILE} doesn't match version {VERSION} ({e}). Fix it or move it aside.")
    })?;
    Ok(file.jobs)
}

/// Written private: a job can hold a chat's words.
fn write_queue_at(path: &Path, jobs: &[Queued]) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(&QueueOut {
        version: VERSION,
        jobs,
    })
    .map_err(|e| format!("Encode {QUEUE_FILE}: {e}"))?;
    bytes.push(b'\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Save {QUEUE_FILE}: {e}"))?;
    }
    crate::vault::atomic_write_private(path, &bytes).map_err(|e| format!("Save {QUEUE_FILE}: {e}"))
}

fn new_id(now: DateTime<Utc>) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "j{}-{}",
        now.timestamp_millis(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// What queuing a job did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Enqueued {
    /// It was added (`false`: the same job was waiting already).
    pub added: bool,
    /// The job as it waits now.
    pub queued: Queued,
}

/// Queue `job` for `entry`, asked for by a person or automatically
/// (`asked`). When the same job ([`Job::key`]) waits for it already, its
/// details are replaced ([`Job::merged`]: nothing automatic weakens a
/// rewrite or a manual beat) and its attempts, error and backoff kept
/// (review round 3, N2: a new request never cuts a failed job's backoff
/// short, nor stops it giving up). A job that gave up starts over only on a person's
/// request (review round 4, R1): attempts and error cleared, and it waits
/// from `now` as a new job would; an automatic request leaves it given up,
/// for Try again. Refused when the queue can't be read or is full.
pub(crate) fn enqueue_as_at(
    path: &Path,
    entry: &str,
    job: Job,
    asked: Asked,
    now: DateTime<Utc>,
) -> Result<Enqueued, String> {
    change_queue_at(path, |jobs| {
        let key = job.key();
        if let Some(same) = jobs
            .iter_mut()
            .find(|q| q.entry == entry && q.job.key() == key)
        {
            same.job = same.job.clone().merged(job, asked);
            if same.gave_up() && asked == Asked::Person {
                same.attempts = 0;
                same.last_error = None;
                same.retry_at = None;
                same.queued_at = now;
            }
            let queued = same.clone();
            return (
                true,
                Ok(Enqueued {
                    added: false,
                    queued,
                }),
            );
        }
        if jobs.len() >= MAX_QUEUED {
            return (false, Err(format!("{MAX_QUEUED} jobs are waiting already")));
        }
        let queued = Queued {
            id: new_id(now),
            entry: entry.to_string(),
            queued_at: now,
            job,
            attempts: 0,
            last_error: None,
            retry_at: None,
            claim: None,
        };
        jobs.push(queued.clone());
        (
            true,
            Ok(Enqueued {
                added: true,
                queued,
            }),
        )
    })?
}

/// [`enqueue_as_at`] for a person's request: `true` when it was added.
#[cfg(test)]
pub(crate) fn enqueue_at(
    path: &Path,
    entry: &str,
    job: Job,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    enqueue_as_at(path, entry, job, Asked::Person, now).map(|e| e.added)
}

/// Change the queue under both locks (this process's and the file's):
/// `change` gets the jobs and says whether to write them back.
fn change_queue_at<T>(
    path: &Path,
    change: impl FnOnce(&mut Vec<Queued>) -> (bool, T),
) -> Result<T, String> {
    let _held = queue_locked();
    let _file = file_lock(path)?;
    let mut jobs = read_queue_at(path)?;
    let (write, out) = change(&mut jobs);
    if write {
        write_queue_at(path, &jobs)?;
    }
    Ok(out)
}

/// Take job `id` off the queue (it succeeded, was discarded or is no longer
/// wanted), if it is still there.
pub(crate) fn take_at(path: &Path, id: &str) -> Result<Option<Queued>, String> {
    change_queue_at(path, |jobs| match jobs.iter().position(|q| q.id == id) {
        Some(at) => (true, Some(jobs.remove(at))),
        None => (false, None),
    })
}

/// Note a failed rerun of job `id`: one more attempt, its error and its
/// [`backoff`] from `now` (review round 2, M2), and its claim ended. The
/// job stays; after [`MAX_ATTEMPTS`] it waits for the person. A heartbeat
/// Run now's time moves to `now` (Q10 follow-up, delta review): its own
/// failed rerun was let in as a beat, and that beat must never count as one
/// "after the press" that drops it.
pub(crate) fn fail_at(
    path: &Path,
    id: &str,
    error: &str,
    now: DateTime<Utc>,
) -> Result<(), String> {
    // The saved token commands' text hidden before it is kept (L13 review):
    // a kept error outlives an auth change, when the old command would no
    // longer be known to hide.
    let error = registry::without_saved_token_commands(error);
    change_queue_at(path, |jobs| match jobs.iter_mut().find(|q| q.id == id) {
        Some(q) => {
            q.attempts = q.attempts.saturating_add(1);
            q.last_error = Some(error.chars().take(400).collect());
            q.retry_at = Some(now + backoff(q.attempts));
            q.claim = None;
            if let Job::Heartbeat {
                manual_at: Some(at),
                ..
            } = &mut q.job
            {
                *at = (*at).max(now);
            }
            (true, ())
        }
        None => (false, ()),
    })
}

/// Note a rerun of job `id` that routing held (review round 3, N1): not an
/// attempt, but a backoff of [`DRAIN_AFTER`] from `now`, so no hold can
/// make a drain loop; its claim ended.
pub(crate) fn hold_at(path: &Path, id: &str, now: DateTime<Utc>) -> Result<(), String> {
    change_queue_at(path, |jobs| match jobs.iter_mut().find(|q| q.id == id) {
        Some(q) => {
            q.retry_at = Some(now + backoff(1));
            q.claim = None;
            (true, ())
        }
        None => (false, ()),
    })
}

/// End the claim on job `id` and leave it as it was (its rerun never
/// started).
pub(crate) fn release_at(path: &Path, id: &str) -> Result<(), String> {
    change_queue_at(path, |jobs| match jobs.iter_mut().find(|q| q.id == id) {
        Some(q) if q.claim.is_some() => {
            q.claim = None;
            (true, ())
        }
        _ => (false, ()),
    })
}

/// Claim job `id` for a rerun in this process (review round 2, M3): read
/// again under the queue's locks, it must still be waiting, not running in
/// any Hollow process, and pass `check`. Returns the job as it is now (a
/// newer copy of it may have replaced its details) and this process's
/// claim.
pub(crate) fn claim_at(
    path: &Path,
    id: &str,
    now: DateTime<Utc>,
    check: impl FnOnce(&Queued) -> Result<(), String>,
) -> Result<(Queued, Claimed), String> {
    change_queue_at(path, |jobs| {
        let Some(q) = jobs.iter_mut().find(|q| q.id == id) else {
            return (false, Err("That job isn't waiting any more.".to_string()));
        };
        if q.running(now) {
            return (false, Err("Hollow is running it now.".to_string()));
        }
        if let Err(why) = check(q) {
            return (false, Err(why));
        }
        q.claim = Some(Claim {
            pid: std::process::id(),
            at: now,
            started: crate::proc_info::own_start_micros(),
        });
        claimed_here().insert(id.to_string());
        (true, Ok((q.clone(), Claimed(id.to_string()))))
    })?
}

/// Discard: take job `id` off without running it, unless a rerun of it is
/// running (review round 2, M3: in any process).
pub(crate) fn discard_at(path: &Path, id: &str, now: DateTime<Utc>) -> Result<Queued, String> {
    change_queue_at(path, |jobs| match jobs.iter().position(|q| q.id == id) {
        None => (false, Err("That job isn't waiting any more.".to_string())),
        Some(at) if jobs[at].running(now) => (false, Err("Hollow is running it now.".to_string())),
        Some(at) => (true, Ok(jobs.remove(at))),
    })?
}

/// Try again (review round 2, M4): job `id`, which gave up, waits again
/// with its attempts, error and backoff cleared, on the entry its kind goes
/// to now (a copy waiting there already stands for it). Returns that
/// entry. Refused while the setting can't be read, while its kind runs As
/// now (Run as now is its way then), and while it runs.
pub(crate) fn try_again_at(
    queue: &Path,
    jobs: &Path,
    id: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let routes = read_jobs_at(jobs)?;
    change_queue_at(queue, |all| {
        let Some(at) = all.iter().position(|q| q.id == id) else {
            return (false, Err("That job isn't waiting any more.".to_string()));
        };
        let q = &all[at];
        if q.running(now) {
            return (false, Err("Hollow is running it now.".to_string()));
        }
        if !q.gave_up() {
            return (
                false,
                Err("It hasn't given up: it waits for its model.".to_string()),
            );
        }
        let to = match place(&routes, q) {
            Place::Here => q.entry.clone(),
            Place::Elsewhere(to) => to,
            Place::Held => {
                let label = q.job.kind().label();
                let why = match q.job.only_on() {
                    Some(own) => format!(
                        "This {label} belongs to a chat on \"{own}\", so it runs only there: send {label} to that model again, or Discard it."
                    ),
                    None => format!("{label} runs As now since, so Run as now sends it there."),
                };
                return (false, Err(why));
            }
        };
        let key = q.job.key();
        if to != q.entry
            && all
                .iter()
                .any(|other| other.entry == to && other.job.key() == key)
        {
            all.remove(at);
            return (true, Ok(to));
        }
        let q = &mut all[at];
        q.entry = to.clone();
        q.attempts = 0;
        q.last_error = None;
        q.retry_at = None;
        (true, Ok(to))
    })?
}

/// Q10 follow-up: switching the heartbeat off drops its beats that wait for
/// a self-hosted model, a Run now included, unless a rerun of one is
/// running. Returns how many went.
pub(crate) fn drop_heartbeats_at(path: &Path, now: DateTime<Utc>) -> Result<usize, String> {
    change_queue_at(path, |jobs| {
        let before = jobs.len();
        jobs.retain(|q| !matches!(q.job, Job::Heartbeat { .. }) || q.running(now));
        let dropped = before - jobs.len();
        (dropped > 0, dropped)
    })
}

/// [`drop_heartbeats_at`] on the app's queue (the heartbeat's switch going
/// off in Settings); Settings reads the queue again.
pub fn drop_heartbeats() {
    let Ok(path) = app_file(QUEUE_FILE) else {
        return;
    };
    match drop_heartbeats_at(&path, Utc::now()) {
        Ok(0) => {}
        Ok(n) => {
            eprintln!("[self-hosted jobs] the heartbeat is off: {n} waiting beat(s) dropped");
            changed();
        }
        Err(why) => {
            eprintln!("[self-hosted jobs] the heartbeat is off, but its waiting beats stay: {why}")
        }
    }
}

/// Move every waiting job of `kind` to entry `to` (its kind goes there
/// now), keeping its place; one the target holds already is dropped as a
/// repeat. A self-hosted chat's own job moves only to its own chat's model
/// (review round 3, N1). Returns how many moved.
pub(crate) fn move_kind_at(path: &Path, kind: JobKind, to: &str) -> Result<usize, String> {
    change_queue_at(path, |jobs| {
        let mut moved = 0;
        let mut on_target: HashSet<String> = jobs
            .iter()
            .filter(|q| q.entry == to)
            .map(|q| q.job.key())
            .collect();
        let mut out: Vec<Queued> = Vec::with_capacity(jobs.len());
        for mut q in std::mem::take(jobs) {
            if q.job.kind() == kind && q.entry != to && q.job.only_on().is_none_or(|own| own == to)
            {
                moved += 1;
                if !on_target.insert(q.job.key()) {
                    continue;
                }
                q.entry = to.to_string();
            }
            out.push(q);
        }
        *jobs = out;
        (moved > 0, moved)
    })
}

// ── Draining ────────────────────────────────────────────────────────────

/// Why a drain starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// The oldest job waited [`DRAIN_AFTER`].
    Waited,
    /// The person pressed Run them now.
    RunNow,
    /// The entry answered for something else (it is awake).
    Answered,
}

/// Whether `entry`'s queue drains now and why. Only jobs a drain would act
/// on count: their kind goes to this entry (or moved to another, which a
/// drain carries over), they haven't given up, and a failed one's backoff
/// is over (review round 2, M2: so a model that just answered doesn't
/// rerun it within the minute); a held job (its kind As now) never starts
/// one. Nothing to act on: no drain, no wake. At once when the entry is
/// `warm`; when the oldest has waited [`DRAIN_AFTER`]. A drain that
/// stopped with an error sets `retry_at`, and neither starts one again
/// before then.
pub(crate) fn due(
    queue: &[Queued],
    jobs: &BTreeMap<String, String>,
    entry: &str,
    now: DateTime<Utc>,
    warm: bool,
    retry_at: Option<DateTime<Utc>>,
) -> Option<Trigger> {
    let oldest = queue
        .iter()
        .filter(|q| {
            q.entry == entry
                && !q.gave_up()
                && !q.backing_off(now)
                && !q.running(now)
                && place(jobs, q) != Place::Held
        })
        .map(|q| q.queued_at)
        .min()?;
    if retry_at.is_some_and(|at| now < at) {
        return None;
    }
    if warm {
        return Some(Trigger::Answered);
    }
    let waited = now
        .signed_duration_since(oldest)
        .to_std()
        .unwrap_or_default();
    (waited >= DRAIN_AFTER).then_some(Trigger::Waited)
}

/// How a drain ends.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DrainOutcome {
    /// It woke the entry (one health call).
    pub woke: bool,
    /// Jobs rerun, and how many of those failed (and stayed queued).
    pub ran: usize,
    pub failed: usize,
    /// Jobs dropped as no longer wanted, before any wake.
    pub dropped: usize,
    /// Jobs moved to the entry their kind goes to now.
    pub moved: usize,
    /// Jobs whose kind runs As now when it ended, left for the person.
    pub held: usize,
    /// Jobs still waiting for the entry (not held, not given up).
    pub left: usize,
    /// Another drain of the entry was running (in any Hollow process), so
    /// this one did nothing.
    pub busy: bool,
    /// Why it stopped before running the entry's jobs.
    pub error: Option<String>,
}

/// What a drain does to the world: ask whether a job is still wanted (cheap,
/// no model call), wake an entry, rerun a job.
pub(crate) trait Drainer {
    fn wanted(&self, q: &Queued) -> bool;
    fn wake(&self, entry: &SelfHostedEntry) -> impl Future<Output = Result<(), String>> + Send;
    fn replay(&self, job: Job) -> impl Future<Output = Result<(), String>> + Send;
}

// ── Entries a drain woke ────────────────────────────────────────────────

fn awake_set() -> std::sync::MutexGuard<'static, HashSet<String>> {
    static AWAKE: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    AWAKE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Whether a drain woke `entry` and is running its jobs: a job routed there
/// meanwhile runs at once, so a rerun never finds the model "asleep" and
/// queues itself again (review).
pub(crate) fn awake(entry: &str) -> bool {
    awake_set().contains(entry)
}

/// Marks an entry awake for a drain's reruns, until dropped.
struct Awake(String);

impl Awake {
    fn mark(entry: &str) -> Awake {
        awake_set().insert(entry.to_string());
        Awake(entry.to_string())
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        awake_set().remove(&self.0);
    }
}

/// Drain `entry_id`'s queue, holding the entry's drain lock (review round
/// 2, M3: another drain of it, in this Hollow process or another, makes
/// this one do nothing, `busy`). First, under the setting as it is now: a
/// job whose kind goes to another entry moves there (no rerun, no wake);
/// one whose kind runs As now is held for the person (review: a drain never
/// sends a queued job to Anthropic); one that gave up waits for the
/// person's Try again; one backing off after a failure waits for its time
/// (`trigger` [`Trigger::RunNow`] doesn't wait for it). Then each job left
/// is asked whether it is still wanted ([`Drainer::wanted`]: the heartbeat
/// still due, the retitle still asked for, the source not extracted, the
/// learning gate), and one that isn't is dropped. Only if any are left, and
/// the entry can run them, one wake; then each reruns back to back while
/// the entry counts as awake ([`Rerun`]: claimed in the queue file after
/// reading it again, so one discarded, moved, held or running meanwhile is
/// left alone, and pinned to this entry, so one whose kind went elsewhere
/// or back to As now during the wake is held, never run As now). It leaves
/// the queue only when it succeeds; a failure counts an attempt and keeps
/// its error with a backoff; a rerun routing held keeps it as it was. A
/// setting that can't be read, an entry that can't run (missing, off) or a
/// wake that fails leaves the jobs, with why.
pub(crate) async fn drain_with<D: Drainer>(
    entry_id: &str,
    trigger: Trigger,
    jobs_path: &Path,
    queue_path: &Path,
    entries: impl Fn() -> Result<Vec<SelfHostedEntry>, String>,
    drainer: &D,
) -> DrainOutcome {
    let mut out = DrainOutcome::default();
    let _drain = match try_drain_lock(queue_path, entry_id) {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            out.busy = true;
            return out;
        }
        Err(why) => {
            out.error = Some(why);
            return out;
        }
    };
    let now = Utc::now();
    let snapshot: Vec<Queued> = match read_queue_at(queue_path) {
        Ok(queue) => queue.into_iter().filter(|q| q.entry == entry_id).collect(),
        Err(why) => {
            out.error = Some(why);
            return out;
        }
    };
    if snapshot.is_empty() {
        return out;
    }
    let jobs = match read_jobs_at(jobs_path) {
        Ok(jobs) => jobs,
        Err(why) => {
            out.error = Some(format!("Queued jobs wait: {why}"));
            out.left = snapshot.len();
            return out;
        }
    };
    let mut candidates = Vec::new();
    for q in snapshot {
        match place(&jobs, &q) {
            Place::Elsewhere(to) => {
                out.moved += move_kind_at(queue_path, q.job.kind(), &to).unwrap_or(0);
            }
            Place::Held => {}
            Place::Here if q.gave_up() || q.running(now) => {}
            Place::Here if q.backing_off(now) && trigger != Trigger::RunNow => {}
            Place::Here => candidates.push(q),
        }
    }
    let mut wanted = Vec::new();
    for q in candidates {
        if drainer.wanted(&q) {
            wanted.push(q);
        } else if take_at(queue_path, &q.id).is_ok_and(|t| t.is_some()) {
            eprintln!(
                "[self-hosted jobs] {}: no longer needed, dropped",
                q.job.kind().label()
            );
            out.dropped += 1;
        }
    }
    // What is left when the drain ends, under the setting then.
    let left = |out: &mut DrainOutcome| {
        let routes = read_jobs_at(jobs_path).unwrap_or_else(|_| jobs.clone());
        let mine: Vec<Queued> = read_queue_at(queue_path)
            .unwrap_or_default()
            .into_iter()
            .filter(|q| q.entry == entry_id)
            .collect();
        out.held = mine
            .iter()
            .filter(|q| place(&routes, q) == Place::Held)
            .count();
        out.left = mine
            .iter()
            .filter(|q| !q.gave_up() && place(&routes, q) == Place::Here)
            .count();
    };
    if wanted.is_empty() {
        left(&mut out);
        return out;
    }
    let entries = entries();
    if let Some(why) = reason(entry_id, &entries) {
        out.error = Some(format!("Queued jobs wait: {why}."));
        left(&mut out);
        return out;
    }
    let Some(entry) = entries
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.id == entry_id))
    else {
        left(&mut out);
        return out;
    };
    if let Err(why) = drainer.wake(&entry).await {
        out.error = Some(format!("{} didn't wake: {why}", entry.name));
        left(&mut out);
        return out;
    }
    out.woke = true;
    let _awake = Awake::mark(entry_id);
    for q in wanted {
        let label = q.job.kind().label();
        let check = |now_q: &Queued| drain_check(now_q, entry_id, jobs_path, trigger);
        let rerun = match Rerun::claim(queue_path, &q.id, Some(entry_id), check) {
            Ok(rerun) => rerun,
            Err(why) => {
                eprintln!("[self-hosted jobs] {label}: not rerun ({why})");
                continue;
            }
        };
        match rerun.run(|job| drainer.replay(job)).await {
            RerunEnd::Done => out.ran += 1,
            RerunEnd::Failed(why) => {
                out.ran += 1;
                out.failed += 1;
                eprintln!("[self-hosted jobs] {label}: {why}");
            }
            RerunEnd::Held(why) => eprintln!("[self-hosted jobs] {label}: {why}"),
        }
    }
    left(&mut out);
    out
}

/// Review round 2 (M3, B1): whether a drain of `entry` may rerun queued
/// job `q` as it is now, right before its rerun: still waiting for this
/// entry, its kind still going here under the setting read again, not
/// given up, and its backoff over (Run them now doesn't wait for it). Q10
/// follow-up: a heartbeat Run now not after a beat admitted since it was
/// asked (during the wake, say), which would be a second beat back to back;
/// the next look drops it.
fn drain_check(q: &Queued, entry: &str, jobs: &Path, trigger: Trigger) -> Result<(), String> {
    if q.entry != entry {
        return Err(format!("it waits for \"{}\" now", q.entry));
    }
    if let Some(own) = q.job.only_on().filter(|own| *own != entry) {
        return Err(format!("it belongs to a chat on \"{own}\""));
    }
    match place(&read_jobs_at(jobs)?, q) {
        Place::Here => {}
        Place::Elsewhere(to) => return Err(format!("its kind goes to \"{to}\" now")),
        Place::Held => return Err("its kind runs As now since".into()),
    }
    if q.gave_up() {
        return Err("it gave up".into());
    }
    if trigger != Trigger::RunNow && q.backing_off(Utc::now()) {
        return Err("it waits for its backoff".into());
    }
    if let Job::Heartbeat {
        vault,
        manual_at: Some(at),
    } = &q.job
    {
        if crate::heartbeat::beat_admitted_since(vault, *at) {
            return Err("a beat ran after its Run now".into());
        }
    }
    Ok(())
}

/// How a [`Rerun`] ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RerunEnd {
    /// It ran: off the queue.
    Done,
    /// It failed: an attempt counted, with its error and a backoff.
    Failed(String),
    /// Routing held it (its kind went elsewhere or back to As now, its
    /// model went off): kept, not an attempt, with a backoff.
    Held(String),
}

/// One queued job's rerun (review round 2): claimed in the queue file
/// ([`claim_at`]) and pinned to where it may run ([`Pinned`]) until it
/// ends.
pub(crate) struct Rerun {
    queue: PathBuf,
    q: Queued,
    pinned: Pinned,
    _claimed: Claimed,
}

impl Rerun {
    /// Claim job `id` (it must still wait, run nowhere and pass `check`)
    /// and pin it to `to` (`None`: As now).
    pub(crate) fn claim(
        queue: &Path,
        id: &str,
        to: Option<&str>,
        check: impl FnOnce(&Queued) -> Result<(), String>,
    ) -> Result<Rerun, String> {
        let (q, claimed) = claim_at(queue, id, Utc::now(), check)?;
        let Some(pinned) = Pinned::new(queue, &q.job, to) else {
            let _ = release_at(queue, id);
            return Err("Hollow is running the same job now.".into());
        };
        Ok(Rerun {
            queue: queue.to_path_buf(),
            q,
            pinned,
            _claimed: claimed,
        })
    }

    /// The job as it was claimed.
    pub(crate) fn queued(&self) -> &Queued {
        &self.q
    }

    /// Rerun it with `replay` (the job's own entry point) and note how it
    /// went in the queue: success takes it off, a failure counts an attempt
    /// with a backoff, a rerun routing held is kept with a backoff and no
    /// attempt ([`hold_at`]). Success wins unless no decision for the job
    /// ran it and one held it (review round 3, N3: another call of the
    /// same job, held, never marks a rerun that ran); an error with a hold
    /// is the hold.
    pub(crate) async fn run<F, Fut>(self, replay: F) -> RerunEnd
    where
        F: FnOnce(Job) -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let result = replay(self.q.job.clone()).await;
        let now = Utc::now();
        match (result, self.pinned.held()) {
            (Ok(()), Some(why)) if !self.pinned.ran() => {
                let _ = hold_at(&self.queue, &self.q.id, now);
                RerunEnd::Held(why)
            }
            (Ok(()), _) => {
                let _ = take_at(&self.queue, &self.q.id);
                RerunEnd::Done
            }
            (Err(_), Some(why)) => {
                let _ = hold_at(&self.queue, &self.q.id, now);
                RerunEnd::Held(why)
            }
            (Err(why), None) => {
                let _ = fail_at(&self.queue, &self.q.id, &why, now);
                RerunEnd::Failed(why)
            }
        }
    }
}

/// Whether queued job `q` would still do anything, without a model call
/// (review: a drain never pays for a cold start with nothing to run).
pub(crate) fn still_wanted(q: &Queued) -> bool {
    match &q.job {
        Job::Extraction { vault, source } => crate::extract::still_wanted(vault, source),
        Job::ChatTitle {
            vault,
            session_key,
            turn_ord,
            ..
        } => crate::chat_title::still_wanted(vault, session_key, *turn_ord),
        Job::SessionTitle { session_id, .. } => crate::sessions::title_still_wanted(session_id),
        Job::Learning {
            vault,
            user,
            run_id,
            changed,
            capture_session,
            ..
        } => match capture_session {
            Some(session) => {
                crate::vault_learning::capture_still_wanted(vault, session, run_id.as_deref())
            }
            None => crate::vault_learning::still_wanted(vault, user, run_id.as_deref(), *changed),
        },
        Job::PrepPack {
            vault,
            meeting_id,
            regenerate,
            ..
        } => crate::prep_pack::still_wanted(vault, meeting_id, "prep", *regenerate),
        Job::Recap {
            vault,
            meeting_id,
            regenerate,
            ..
        } => crate::prep_pack::still_wanted(vault, meeting_id, "recaps", *regenerate),
        Job::Profile { vault } => crate::profile_builder::still_wanted(vault, q.queued_at),
        // Q10 follow-up: a Run now is wanted whatever the schedule says
        // (the person asked), unless a beat was admitted since or a budget
        // holds it, which drops it with a note before any wake; a scheduled
        // beat while its gates pass.
        Job::Heartbeat {
            vault,
            manual_at: Some(at),
        } => match crate::heartbeat::manual_beat_unwanted(vault, *at, Utc::now()) {
            Some(why) => {
                crate::heartbeat::note_dropped_run_now(vault, &why);
                false
            }
            None => true,
        },
        Job::Heartbeat {
            vault,
            manual_at: None,
        } => crate::heartbeat::still_due(vault),
    }
}

/// The app's drains: which entries are draining in this process, when a
/// failed drain may try again, and what it said. (A job being rerun is
/// its claim in the queue file: [`Queued::running`].)
#[derive(Default)]
pub(crate) struct DrainState {
    pub running: HashSet<String>,
    pub retry_at: HashMap<String, DateTime<Utc>>,
    pub last_error: HashMap<String, String>,
}

impl DrainState {
    /// Claim `entry` for a drain: `false` when one runs already.
    pub(crate) fn begin(&mut self, entry: &str) -> bool {
        self.running.insert(entry.to_string())
    }

    /// A drain of `entry` ended: an error sets its retry time
    /// ([`DRAIN_AFTER`] from `now`) and is kept for Settings; a clean end
    /// clears both; one that found another drain running changes neither.
    /// (A failed rerun's backoff is the job's own: [`fail_at`].)
    pub(crate) fn finish(&mut self, entry: &str, outcome: &DrainOutcome, now: DateTime<Utc>) {
        self.running.remove(entry);
        if outcome.busy {
            return;
        }
        match &outcome.error {
            Some(why) => {
                let retry = now + chrono::Duration::from_std(DRAIN_AFTER).unwrap_or_default();
                self.retry_at.insert(entry.to_string(), retry);
                self.last_error.insert(entry.to_string(), why.clone());
            }
            None => {
                self.retry_at.remove(entry);
                self.last_error.remove(entry);
            }
        }
    }

    /// The drains a look at the queue starts: each entry with jobs that is
    /// not draining already and is [`due`], with why.
    pub(crate) fn due_drains(
        &self,
        queue: &[Queued],
        jobs: &BTreeMap<String, String>,
        now: DateTime<Utc>,
        warm: impl Fn(&str) -> bool,
    ) -> Vec<(String, Trigger)> {
        let entries: BTreeSet<&str> = queue.iter().map(|q| q.entry.as_str()).collect();
        entries
            .into_iter()
            .filter(|entry| !self.running.contains(*entry))
            .filter_map(|entry| {
                due(
                    queue,
                    jobs,
                    entry,
                    now,
                    warm(entry),
                    self.retry_at.get(entry).copied(),
                )
                .map(|trigger| (entry.to_string(), trigger))
            })
            .collect()
    }
}

fn drains() -> std::sync::MutexGuard<'static, DrainState> {
    static STATE: OnceLock<Mutex<DrainState>> = OnceLock::new();
    STATE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// The core host (registered at startup), for events, notifications and a
/// queued heartbeat; `None` in tests and before startup.
fn app() -> Option<&'static crate::core_host::Core> {
    crate::notifications::registered()
}

/// Tell Settings the queue or a drain changed.
fn changed() {
    if let Some(app) = app() {
        let _ = app.emit(CHANGED_EVENT, ());
    }
}

/// The app's [`Drainer`]: [`still_wanted`], the forwarder's health call,
/// and each job's own entry point ([`replay`]).
pub(crate) struct AppDrainer;

impl Drainer for AppDrainer {
    fn wanted(&self, q: &Queued) -> bool {
        still_wanted(q)
    }

    async fn wake(&self, entry: &SelfHostedEntry) -> Result<(), String> {
        let since = forwarder::stops(&entry.id);
        let fwd = forwarder::ensure_since(entry.upstream(), since).await?;
        let (status, _) = fwd.get(&entry.health_path, HEALTH_BYTES).await?;
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(format!("its health check answered {status}"))
        }
    }

    async fn replay(&self, job: Job) -> Result<(), String> {
        replay(job).await
    }
}

/// Rerun `job` through its own entry point, which routes it again: on the
/// awake entry during a drain; As now for a held job the person runs as
/// now. Its error (a skip, a failed call) is the rerun's.
pub(crate) async fn replay(job: Job) -> Result<(), String> {
    match job {
        Job::Extraction { vault, source } => {
            let me = crate::extract::me(&vault);
            let model = crate::extract::ConfiguredModel::new(
                None,
                Some(Job::Extraction {
                    vault: vault.clone(),
                    source: source.clone(),
                }),
            );
            let report = crate::extract::run(&vault, &source, &model, &me, Utc::now()).await?;
            if let (crate::extract::Source::Recording { path }, Some(app)) = (&source, app()) {
                crate::extract::notify_found(app, &vault, &report, path);
            }
            Ok(())
        }
        Job::ChatTitle {
            vault,
            session_key,
            turn_ord,
            run_id,
            only_on,
        } => {
            crate::chat_title::retitle_now(app(), &vault, &session_key, turn_ord, &run_id, only_on)
                .await
        }
        Job::SessionTitle {
            projects,
            project_dir,
            session_id,
        } => crate::sessions::generate(&projects, &project_dir, &session_id)
            .await
            .map(drop),
        Job::Learning {
            vault,
            user,
            references,
            assistant,
            run_id,
            changed,
            only_on,
            capture_session,
        } => {
            let items = if let Some(session) = capture_session {
                crate::vault_learning::capture_from_chat(
                    &vault,
                    &session,
                    &crate::learning_capture::Input {
                        user,
                        references,
                        assistant,
                        run_id: run_id
                            .ok_or_else(|| "capture source run is missing".to_string())?,
                        self_hosted: only_on,
                        material_change: changed,
                    },
                )
                .await?
            } else {
                crate::vault_learning::propose_from_exchange(
                    &vault,
                    &user,
                    &references,
                    &assistant,
                    run_id.as_deref(),
                    changed,
                    only_on.as_deref(),
                )
                .await?
            };
            if let (false, Some(app)) = (items.is_empty(), app()) {
                let _ = app.emit(
                    "learning-proposals-changed",
                    serde_json::json!({ "pendingAdded": items.len() }),
                );
            }
            Ok(())
        }
        Job::PrepPack {
            vault,
            meeting_id,
            regenerate,
            expected_hash,
            run_id,
        } => {
            crate::prep_pack::run_queued(vault, meeting_id, regenerate, expected_hash, run_id).await
        }
        Job::Recap {
            vault,
            meeting_id,
            around,
            regenerate,
            recording_id,
            expected_hash,
        } => {
            crate::meeting_recap::run_queued(
                vault,
                meeting_id,
                around,
                regenerate,
                recording_id,
                expected_hash,
            )
            .await
        }
        Job::Profile { vault } => {
            let tell = || {
                if let Some(app) = app() {
                    let _ = app.emit(crate::commands::profile_builder_api::EVENT, ());
                }
            };
            tell();
            let scanned = crate::profile_builder::scan(&vault).await.map(drop);
            tell();
            scanned
        }
        Job::Heartbeat { vault, manual_at } => {
            // Review: inside the drain, so the beat can't find the model
            // asleep again by the time the run queue would have claimed it.
            let app = app().ok_or("Hollow's window isn't running, so the beat waits.")?;
            crate::heartbeat::run_queued_beat(app, &vault, manual_at.is_some()).await
        }
    }
}

/// Start a drain of `entry` unless one is running in this process: `false`
/// when one is. (One in another process makes it end `busy`.)
fn spawn_drain(entry: &str, trigger: Trigger) -> bool {
    if !drains().begin(entry) {
        return false;
    }
    changed();
    let entry = entry.to_string();
    tauri::async_runtime::spawn(async move {
        let outcome = match (app_file(JOBS_FILE), app_file(QUEUE_FILE)) {
            (Ok(jobs), Ok(queue)) => {
                drain_with(
                    &entry,
                    trigger,
                    &jobs,
                    &queue,
                    registry::entries,
                    &AppDrainer,
                )
                .await
            }
            (Err(why), _) | (_, Err(why)) => DrainOutcome {
                error: Some(why),
                ..Default::default()
            },
        };
        if outcome.busy {
            eprintln!("[self-hosted jobs] {entry}: another drain of it is running");
        } else {
            eprintln!(
                "[self-hosted jobs] {entry}: drain ({trigger:?}) woke {}, ran {} ({} failed), dropped {}, moved {}, held {}, {} left{}",
                outcome.woke,
                outcome.ran,
                outcome.failed,
                outcome.dropped,
                outcome.moved,
                outcome.held,
                outcome.left,
                outcome
                    .error
                    .as_deref()
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            );
        }
        // Kept for Settings: the saved token commands' text hidden first, as
        // a failed rerun's is (`fail_at`).
        let mut outcome = outcome;
        outcome.error = outcome
            .error
            .map(|why| registry::without_saved_token_commands(&why));
        drains().finish(&entry, &outcome, Utc::now());
        changed();
    });
    true
}

/// The drains one look at the queue starts ([`tick`]): each due entry
/// ([`DrainState::due_drains`]); none while the queue or the setting can't
/// be read.
pub(crate) fn due_now(
    queue: &Path,
    jobs: &Path,
    state: &DrainState,
    now: DateTime<Utc>,
    warm: impl Fn(&str) -> bool,
) -> Vec<(String, Trigger)> {
    let (Ok(queue), Ok(jobs)) = (read_queue_at(queue), read_jobs_at(jobs)) else {
        return Vec::new();
    };
    state.due_drains(&queue, &jobs, now, warm)
}

/// One look at the queue: drain each entry that is due ([`due_now`]).
fn tick() {
    let (Ok(queue), Ok(jobs)) = (app_file(QUEUE_FILE), app_file(JOBS_FILE)) else {
        return;
    };
    let now = SystemTime::now();
    let due = {
        let state = drains();
        due_now(&queue, &jobs, &state, Utc::now(), |entry| {
            warm_at(entry, now)
        })
    };
    for (entry, trigger) in due {
        spawn_drain(&entry, trigger);
    }
}

/// The drain loop (queue owner only, `lib.rs`): a look every minute, and
/// right after any entry answers. It never wakes an entry by itself: only a
/// due drain with jobs still wanted for it does.
pub fn start(_app: impl Into<crate::core_host::Core>) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_TICK).await;
        loop {
            tick();
            tokio::select! {
                _ = tokio::time::sleep(TICK) => {}
                _ = forwarder::answered_signal().notified() => {}
            }
        }
    });
}

/// Why Run them now has nothing to run for `entry`: no job waits for it
/// whose kind goes there (or moved on) and that hasn't given up (those
/// have their own Try again), or the setting can't be read.
pub(crate) fn run_now_problem(
    queue: &[Queued],
    jobs: &Result<BTreeMap<String, String>, String>,
    entry: &str,
) -> Option<String> {
    let jobs = match jobs {
        Ok(jobs) => jobs,
        Err(why) => {
            return Some(format!(
                "Background jobs are held while {JOBS_FILE} can't be read ({}).",
                why.trim().trim_end_matches('.')
            ))
        }
    };
    let waiting = queue
        .iter()
        .any(|q| q.entry == entry && !q.gave_up() && place(jobs, q) != Place::Held);
    (!waiting).then(|| "Nothing is waiting for this model.".to_string())
}

/// Run them now: drain `entry`'s queue at once (a wake included, and the
/// jobs backing off after a failure run too). Refused while a drain of it
/// runs in any Hollow process (review round 2, M3: Run them now in a
/// window of another process never starts a second one).
pub fn run_now(entry: &str) -> Result<(), String> {
    let queue_path = app_file(QUEUE_FILE)?;
    let queue = read_queue_at(&queue_path)?;
    let jobs = app_file(JOBS_FILE).and_then(|path| read_jobs_at(&path));
    if let Some(why) = run_now_problem(&queue, &jobs, entry) {
        return Err(why);
    }
    if drain_busy_at(&queue_path, entry)? || !spawn_drain(entry, Trigger::RunNow) {
        return Err("Hollow is running them already.".into());
    }
    Ok(())
}

/// Whether held job `q` may run as now: its kind runs As now under a
/// setting that can be read (review: only the person sends a held job
/// there), and it isn't a self-hosted chat's own job, which runs only on
/// that chat's model (review round 2).
pub(crate) fn as_now_problem(
    jobs: &Result<BTreeMap<String, String>, String>,
    q: &Queued,
) -> Option<String> {
    if let Some(own) = q.job.only_on() {
        return Some(format!(
            "This {} belongs to a chat on \"{own}\", so it runs only there.",
            q.job.kind().label()
        ));
    }
    match jobs {
        Err(why) => Some(format!(
            "Background jobs are held while {JOBS_FILE} can't be read ({}).",
            why.trim().trim_end_matches('.')
        )),
        Ok(jobs) => match place(jobs, q) {
            Place::Held => None,
            Place::Here => Some(format!(
                "{} goes to this model now, so it runs there.",
                q.job.kind().label()
            )),
            Place::Elsewhere(to) => Some(format!(
                "{} goes to \"{to}\" now, so it runs there.",
                q.job.kind().label()
            )),
        },
    }
}

/// Run as now's checks and claim (review round 2): the job read again
/// under the queue's locks, running nowhere, and [`as_now_problem`] clear
/// under the setting read again; pinned to As now, so a kind sent to a
/// model meanwhile holds it instead.
pub(crate) fn start_as_now_at(queue: &Path, jobs: &Path, id: &str) -> Result<Rerun, String> {
    Rerun::claim(queue, id, None, |q| {
        as_now_problem(&read_jobs_at(jobs), q).map_or(Ok(()), Err)
    })
}

/// Run as now: the person sends held job `id` (its kind runs As now) to
/// where its kind runs today. It leaves the queue only when it succeeds.
pub fn run_as_now(id: &str) -> Result<(), String> {
    let rerun = start_as_now_at(&app_file(QUEUE_FILE)?, &app_file(JOBS_FILE)?, id)?;
    changed();
    tauri::async_runtime::spawn(async move {
        let label = rerun.queued().job.kind().label();
        match rerun.run(replay).await {
            RerunEnd::Done => {}
            RerunEnd::Failed(why) | RerunEnd::Held(why) => {
                eprintln!("[self-hosted jobs] {label} (run as now): {why}");
            }
        }
        changed();
    });
    Ok(())
}

/// Discard: the person drops queued job `id` without running it (refused
/// while a rerun of it runs, in any Hollow process).
pub fn discard(id: &str) -> Result<(), String> {
    discard_at(&app_file(QUEUE_FILE)?, id, Utc::now())?;
    changed();
    Ok(())
}

/// Try again (review round 2, M4): job `id`, which gave up, waits again
/// ([`try_again_at`]). A drain of its entry starts at once (Run them now),
/// unless one already runs in this or another Hollow process (review round
/// 3, N6): then the next drain runs it, since it waits like any other job.
/// Once the job is back in line nothing fails the request: a drain lock
/// that can't be probed starts no drain (review round 4, R3), and the
/// answer says so ([`retried`], Q10 follow-up).
pub fn try_again(id: &str) -> Result<SelfHostedRetry, String> {
    let queue = app_file(QUEUE_FILE)?;
    let jobs = app_file(JOBS_FILE)?;
    let entry = try_again_at(&queue, &jobs, id, Utc::now())?;
    changed();
    let busy = drain_busy_at(&queue, &entry);
    Ok(retried(
        entry,
        registry::entries(),
        busy,
        |entry| spawn_drain(entry, Trigger::RunNow),
        |entries| view_with(&jobs, &queue, entries),
    ))
}

/// What Try again says once the job is back in line for `entry` (the model
/// its kind goes to now, which [`try_again_at`] may have moved it to; Q10
/// follow-up), named from `entries`: a drain of it started now (`start`,
/// only when the drain lock was free), one runs already (the lock held, or
/// `start` found one in this process), or neither is known (the lock
/// couldn't be probed), so the next drain runs it. `view` reads Settings'
/// view after that.
pub(crate) fn retried(
    entry: String,
    entries: Result<Vec<SelfHostedEntry>, String>,
    busy: Result<bool, String>,
    start: impl FnOnce(&str) -> bool,
    view: impl FnOnce(Result<Vec<SelfHostedEntry>, String>) -> SelfHostedJobs,
) -> SelfHostedRetry {
    let started = busy == Ok(false) && start(&entry);
    let name = entries
        .as_ref()
        .ok()
        .and_then(|list| list.iter().find(|e| e.id == entry))
        .map(|e| e.name.clone());
    SelfHostedRetry {
        started,
        under_way: !started && busy.is_ok(),
        entry,
        name,
        jobs: view(entries),
    }
}

// ── Settings' view ──────────────────────────────────────────────────────

/// One job kind's row in Settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedJobRow {
    pub kind: JobKind,
    pub label: String,
    pub about: String,
    /// The entry it goes to; `None`: As now. With a setting that can't be
    /// read, what can still be read of it (for showing only).
    pub entry: Option<String>,
    /// Why its jobs are skipped now, when they are.
    pub problem: Option<String>,
}

/// Jobs waiting for one entry (their kind goes there, not given up; those
/// backing off after a failure included).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedWaiting {
    pub entry: String,
    /// Its name, when it is in the list.
    pub name: Option<String>,
    pub count: u32,
    /// The oldest job's time (RFC 3339).
    pub oldest_at: String,
    /// A drain is running for it now.
    pub draining: bool,
    /// Why the last drain stopped before running its jobs.
    pub last_error: Option<String>,
}

/// A queued job that waits for the person: its kind runs As now (Run as
/// now or Discard), or it failed [`MAX_ATTEMPTS`] times (Try again or
/// Discard).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedHeldJob {
    pub id: String,
    pub kind: JobKind,
    pub label: String,
    /// The entry it waited for, and its name when it is in the list.
    pub entry: String,
    pub name: Option<String>,
    /// When it was queued (RFC 3339).
    pub queued_at: String,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub gave_up: bool,
    /// A rerun of it is running (Run as now, Try again or a drain), in
    /// any Hollow process.
    pub running: bool,
    /// Run as now can send it: its kind runs As now under a setting that
    /// reads, and it isn't a self-hosted chat's own job.
    pub can_run_as_now: bool,
    /// Try again can run it: it gave up, and its kind still goes to a
    /// self-hosted model.
    pub can_try_again: bool,
    /// A self-hosted chat's own retitle or learning: the chat's entry, the
    /// only one it runs on.
    pub only_on: Option<String>,
}

/// What Try again did (review round 4, R3): `started`, a drain of the
/// job's model started now; `under_way`, one runs already and the next
/// runs it; neither, the drain lock couldn't be probed and the next drain
/// runs it (Q10 follow-up). `entry` and `name`: the model the job waits for
/// now, which may not be the one it gave up on.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedRetry {
    pub started: bool,
    pub under_way: bool,
    pub entry: String,
    /// Its name, when it is in the list.
    pub name: Option<String>,
    pub jobs: SelfHostedJobs,
}

/// Settings, Advanced, Background jobs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfHostedJobs {
    pub kinds: Vec<SelfHostedJobRow>,
    pub waiting: Vec<SelfHostedWaiting>,
    pub held: Vec<SelfHostedHeldJob>,
    /// The setting can't be read: every kind is held (skipped) and nothing
    /// can change until it is fixed or moved aside.
    pub error: Option<String>,
    /// The queue can't be read: routed jobs that would wait are skipped.
    pub queue_error: Option<String>,
}

pub(crate) fn view_with(
    jobs_path: &Path,
    queue_path: &Path,
    entries: Result<Vec<SelfHostedEntry>, String>,
) -> SelfHostedJobs {
    let read = read_jobs_at(jobs_path);
    let (shown, error) = match &read {
        Ok(jobs) => (jobs.clone(), None),
        Err(why) => (peek_jobs_at(jobs_path), Some(why.clone())),
    };
    let kinds = JobKind::ALL
        .into_iter()
        .map(|kind| {
            let entry = entry_for(&shown, kind).map(str::to_string);
            let problem = if error.is_some() {
                Some(format!(
                    "Held: {JOBS_FILE} can't be read, so no job of this kind runs until it is fixed or moved aside."
                ))
            } else {
                entry
                    .as_deref()
                    .and_then(|id| reason(id, &entries))
                    .map(|why| {
                        format!(
                            "Skipped while {why}: these jobs never run on another model instead."
                        )
                    })
            };
            SelfHostedJobRow {
                kind,
                label: kind.label().into(),
                about: kind.about().into(),
                entry,
                problem,
            }
        })
        .collect();
    let (queue, queue_error) = match read_queue_at(queue_path) {
        Ok(queue) => (queue, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let names: HashMap<&str, &str> = entries
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|e| (e.id.as_str(), e.name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    let name = |id: &str| names.get(id).map(|n| n.to_string());
    let now = Utc::now();
    let state = drains();
    let mut by_entry: BTreeMap<&str, (u32, DateTime<Utc>)> = BTreeMap::new();
    let mut held = Vec::new();
    for q in &queue {
        // With a setting that can't be read every job waits where it is.
        let at = read.as_ref().ok().map(|jobs| place(jobs, q));
        let waits = !q.gave_up() && at != Some(Place::Held);
        if waits {
            let slot = by_entry.entry(q.entry.as_str()).or_insert((0, q.queued_at));
            slot.0 += 1;
            slot.1 = slot.1.min(q.queued_at);
        } else {
            held.push(SelfHostedHeldJob {
                id: q.id.clone(),
                kind: q.job.kind(),
                label: q.job.kind().label().into(),
                entry: q.entry.clone(),
                name: name(&q.entry),
                queued_at: q.queued_at.to_rfc3339(),
                attempts: q.attempts,
                last_error: q.last_error.clone(),
                gave_up: q.gave_up(),
                running: q.running(now),
                can_run_as_now: as_now_problem(&read, q).is_none(),
                can_try_again: q.gave_up() && matches!(at, Some(Place::Here | Place::Elsewhere(_))),
                only_on: q.job.only_on().map(str::to_string),
            });
        }
    }
    let waiting = by_entry
        .into_iter()
        .map(|(entry, (count, oldest))| SelfHostedWaiting {
            entry: entry.to_string(),
            name: name(entry),
            count,
            oldest_at: oldest.to_rfc3339(),
            draining: state.running.contains(entry),
            last_error: state.last_error.get(entry).cloned(),
        })
        .collect();
    SelfHostedJobs {
        kinds,
        waiting,
        held,
        error,
        queue_error,
    }
}

/// The view on the app's files and the registry. Without an app-data
/// folder every kind is held, as for a setting that can't be read.
pub fn view() -> SelfHostedJobs {
    match (app_file(JOBS_FILE), app_file(QUEUE_FILE)) {
        (Ok(jobs), Ok(queue)) => view_with(&jobs, &queue, registry::entries()),
        (Err(why), _) | (_, Err(why)) => SelfHostedJobs {
            kinds: Vec::new(),
            waiting: Vec::new(),
            held: Vec::new(),
            error: Some(why),
            queue_error: None,
        },
    }
}

/// Send `kind` to `entry` (`None`: As now) and return the view. Its waiting
/// jobs move to the new entry; sent back to As now, they are held for the
/// person (Run as now, Discard).
pub fn set(kind: JobKind, entry: Option<&str>) -> Result<SelfHostedJobs, String> {
    set_at(&app_file(JOBS_FILE)?, registry::entries(), kind, entry)?;
    if let Some(to) = entry.map(str::trim).filter(|id| !id.is_empty()) {
        if let Err(why) = move_kind_at(&app_file(QUEUE_FILE)?, kind, to) {
            eprintln!(
                "[self-hosted jobs] moving {}'s waiting jobs: {why}",
                kind.label()
            );
        }
    }
    changed();
    Ok(view())
}

#[cfg(test)]
#[path = "self_hosted_jobs_tests.rs"]
mod tests;
