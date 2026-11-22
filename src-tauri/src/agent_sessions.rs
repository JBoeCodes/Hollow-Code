//! Hollow-owned agent session identity and the live-phase record.
//!
//! Two jobs, both small and both about telling the truth:
//!
//! 1. **Identity.** Hollow mints the session key (`s-{YYYYMMDD}-{8 hex}`); the
//!    harness's own id is per-stint metadata that may be absent entirely.
//!    Providers without an explicit resume id continue from Hollow's retained
//!    transcript; they must never resume an unrelated most-recent CLI session.
//!    `Option<String>` is the correct value rather than a sentinel. Keys become filenames and
//!    command arguments, so `is_safe_session_key` is the one gate every path
//!    join goes through, mirroring `sessions::validate_session_components`.
//!
//! 2. **Phase.** The live record is chat-only phase DETAIL, never an
//!    exclusion signal. Whether the vault is held is answered only by the
//!    `automated_history` lease: `try_begin_run` is the admission point and
//!    `lease_holder` names the holder, whichever writer it is (chat, workflow,
//!    heartbeat, ingest, tracker sync, scheduler bookkeeping, history clear).
//!    [`describe_holder`] / [`busy_message`] / [`busy_reason`] word that
//!    holder for a person, adding this record's "a Claude turn is still
//!    preparing (since …)" only when the record's run id IS the holder's run.
//!    The record is set once chat holds the lease (`chat::admit_turn`, which
//!    the run-queue pump calls) and cleared inside `chat::finish_observed_run`
//!    after `finish_run`, the single funnel every chat exit path routes
//!    through, so its detail never outlives the lease. A stop shows as
//!    `Cancelling` for the whole termination (up to the grace, reap and verify
//!    bounds). A chat turn still waiting in the run queue (Paperclip PR 7b)
//!    has no record: `agent_session_live` reports its queue row as `queued` (or
//!    `preparing` when nothing truly holds it back, 2026-11-16)
//!    (`chat::queued_live_session`) instead. `chat::is_busy()` was removed in
//!    PR 7b: it read the chat stop registration, which a turn drops before
//!    the lease is released, so it was never a lease signal; the in-memory
//!    `chat_rows::CHAT_ROW_ACTIVE` gate is what says a chat turn is running.
//!
//!    `is_engaged()` was removed in Paperclip PR 2: it read only this record,
//!    so the heartbeat that yielded on it could not see a workflow, ingest,
//!    tracker sync, scheduler or history-clear holder, and charged its budget for a
//!    beat that then failed to take the lease.

use chrono::Utc;
use once_cell::sync::Lazy;
use rand::Rng;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::agent::AgentKind;
use crate::automated_history::LeaseHolder;
use crate::run_activity::{CardHold, RunActivity};
use crate::types::LiveAgentSession;

/// Upper bound on a session key. Generous next to the minted 19-char shape so
/// an adopted or migrated key still fits, tight enough to stay a sane filename.
pub const SESSION_KEY_MAX: usize = 64;

/// Where a live turn is in its lifecycle. `Idle` is never stored (an idle
/// session simply has no record); it exists so the wire type can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Tests only: its derived union lists every variant, so the TS mirror check
// in `ts_bindings` (`ts_bindings::SessionPhase`) can't miss one.
#[cfg_attr(test, derive(ts_rs::TS))]
pub enum SessionPhase {
    Idle,
    Queued,
    Preparing,
    Running,
    /// P1a: asked to pause; still running until its next safe point.
    Pausing,
    /// P1a: paused with its native session kept. Never a live record's
    /// phase: `chat_pause::live_session` lists a paused chat with it.
    Paused,
    Cancelling,
    #[cfg_attr(not(test), allow(dead_code))]
    Failed,
}

impl SessionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionPhase::Idle => "idle",
            SessionPhase::Queued => "queued",
            SessionPhase::Preparing => "preparing",
            SessionPhase::Running => "running",
            SessionPhase::Pausing => "pausing",
            SessionPhase::Paused => "paused",
            SessionPhase::Cancelling => "cancelling",
            SessionPhase::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
struct LiveSession {
    key: String,
    agent: AgentKind,
    run_id: String,
    phase: SessionPhase,
    phase_entered_at: String,
    failure_reason: Option<String>,
    /// The running agent's activity clock (Paperclip PR 15b), shared with
    /// `process_group::drive` and the turn's ticker.
    activity: Option<Arc<RunActivity>>,
}

/// Live chat turns by history run id. Several can be live since build
/// programme PR B4 (build workspaces' turns run in parallel); before it this
/// held at most one.
static LIVE: Lazy<Mutex<Vec<LiveSession>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// `LIVE` is a process-global, and cargo runs tests in parallel threads of
/// ONE process, so any test (in any module) that touches the record holds
/// this first, or it observes another test's begin/clear.
#[cfg(test)]
pub(crate) static LIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

/// `s-{YYYYMMDD}-{8 lowercase hex}`. The date prefix makes a directory of
/// records sort chronologically; the suffix is what makes it unique.
pub fn mint_session_key() -> String {
    let day = Utc::now().format("%Y%m%d");
    let suffix: u32 = rand::thread_rng().gen();
    format!("s-{day}-{suffix:08x}")
}

/// The single path-safety gate. Every caller that turns a key into a filename,
/// a SQL parameter, or a command argument runs it through here first.
///
/// The charset restriction is what does the work: with only `[a-z0-9-]`
/// allowed there is no `.`, no `/`, and no `\`, so `..` traversal and path
/// separators are unrepresentable rather than merely checked for.
pub fn is_safe_session_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= SESSION_KEY_MAX
        && key.starts_with("s-")
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Record a chat turn as starting once `chat::admit_turn` holds its lease
/// (`try_begin_owned` → `try_begin_run`), before provider preparation or
/// subprocess spawn.
pub fn begin(key: &str, agent: AgentKind, run_id: &str, phase: SessionPhase) {
    if let Ok(mut guard) = LIVE.lock() {
        // One record per conversation: a newer turn of the same key replaces
        // any leftover (a turn is admitted only once its predecessor ended).
        guard.retain(|live| live.key != key && live.run_id != run_id);
        guard.push(LiveSession {
            key: key.to_string(),
            agent,
            run_id: run_id.to_string(),
            phase,
            phase_entered_at: Utc::now().to_rfc3339(),
            failure_reason: None,
            activity: None,
        });
    }
}

fn enter(live: &mut LiveSession, phase: SessionPhase) {
    if live.phase != phase {
        live.phase = phase;
        live.phase_entered_at = Utc::now().to_rfc3339();
    }
}

/// Advance `run_id`'s phase, restamping `phase_entered_at`. A wedged run is
/// then observable from outside as elapsed-in-state, which is the whole
/// reason the timestamp rides along with the phase.
pub fn set_phase(run_id: &str, phase: SessionPhase) {
    if let Ok(mut guard) = LIVE.lock() {
        for live in guard.iter_mut().filter(|live| live.run_id == run_id) {
            enter(live, phase);
        }
    }
}

/// Advance the phase of `key`'s live turn (the stop button knows the key).
pub fn set_phase_for_key(key: &str, phase: SessionPhase) {
    if let Ok(mut guard) = LIVE.lock() {
        for live in guard.iter_mut().filter(|live| live.key == key) {
            enter(live, phase);
        }
    }
}

/// P1a: show `key`'s live turn as pausing, unless a Stop already shows it
/// cancelling (a Stop wins).
pub fn mark_pausing(key: &str) {
    if let Ok(mut guard) = LIVE.lock() {
        for live in guard
            .iter_mut()
            .filter(|live| live.key == key && live.phase != SessionPhase::Cancelling)
        {
            enter(live, SessionPhase::Pausing);
        }
    }
}

/// Mark `run_id`'s turn failed and keep the reason for `agent_session_live`.
/// The failed-but-unfinalized turn still holds its lease, so
/// [`describe_holder`] keeps naming it until `finish_observed_run` clears it.
#[cfg(test)]
pub fn fail(run_id: &str, reason: &str) {
    if let Ok(mut guard) = LIVE.lock() {
        for live in guard.iter_mut().filter(|live| live.run_id == run_id) {
            live.phase = SessionPhase::Failed;
            live.phase_entered_at = Utc::now().to_rfc3339();
            live.failure_reason = Some(reason.to_string());
        }
    }
}

/// Clear every record. Test-only since Paperclip PR 7b: production clears
/// go through [`clear_run`], so a late clear can never erase a newer turn's
/// record.
#[cfg(test)]
pub fn clear() {
    if let Ok(mut guard) = LIVE.lock() {
        guard.clear();
    }
}

/// Clear `run_id`'s record (Paperclip PR 7b). Chat's finish funnel and an
/// admitted turn dropped unexecuted use it, so a late clear can never erase
/// a newer turn's record.
pub fn clear_run(run_id: &str) {
    if let Ok(mut guard) = LIVE.lock() {
        guard.retain(|live| live.run_id != run_id);
    }
}

/// The live record's detail, when `matches` accepts it and it is not idle.
fn live_detail(matches: impl Fn(&LiveSession) -> bool) -> Option<String> {
    let guard = LIVE.lock().ok()?;
    let live = guard
        .iter()
        .find(|live| live.phase != SessionPhase::Idle && matches(live))?;
    Some(format!(
        "a {} turn is still {} (since {})",
        live.agent.label(),
        live.phase.as_str(),
        live.phase_entered_at
    ))
}

/// The live chat detail for `holder`, only when the record belongs to the very
/// run that holds the lease. Run ids are unique per process, so an unrelated
/// record (another test, a turn that already finished) never decorates it.
fn live_detail_for(holder: &LeaseHolder) -> Option<String> {
    let run_id = holder.run_id()?;
    live_detail(|live| live.run_id == run_id)
}

/// A single-line name for whoever holds the lease, with the live chat phase
/// when the holder is the recorded chat turn.
pub fn describe_holder(holder: &LeaseHolder) -> String {
    live_detail_for(holder).unwrap_or_else(|| holder.describe())
}

/// The refusal to show when `holder` blocked an admission.
pub fn busy_message(holder: &LeaseHolder) -> String {
    match live_detail_for(holder) {
        Some(detail) => format!("Hollow is busy: {detail}; try again when it finishes"),
        None => holder.busy_message(),
    }
}

/// Why `vault` is busy right now, naming ANY lease holder, or `None` when the
/// lease is free. Advisory (see `automated_history::lease_holder`): use it to
/// refuse early or word a message, never as permission to start work.
pub fn busy_reason(vault: &Path) -> Option<String> {
    crate::automated_history::lease_holder(vault).map(|holder| describe_holder(&holder))
}

/// Attach the running turn's activity clock (Paperclip PR 15b), only when the
/// record belongs to `run_id`, so a late attach never decorates a newer turn.
pub fn attach_activity(run_id: &str, activity: Arc<RunActivity>) {
    if let Ok(mut guard) = LIVE.lock() {
        if let Some(live) = guard.iter_mut().find(|live| live.run_id == run_id) {
            live.activity = Some(activity);
        }
    }
}

/// Pause the watchdog of `key`'s live turn while a Hollow card waits on the
/// user (merged plan slice 3, A·R2): one [`CardHold`] per matching turn with
/// an attached activity clock, released when the returned holds drop. A
/// card the broker could not bind to a conversation (`None`) holds every
/// live turn: pausing a watchdog only delays it, while a card left
/// unattributed must never let one fire on a run that is waiting.
pub(crate) fn hold_watchdog(key: Option<&str>) -> Vec<CardHold> {
    let Ok(guard) = LIVE.lock() else {
        return Vec::new();
    };
    guard
        .iter()
        .filter(|live| key.is_none_or(|key| live.key == key))
        .filter_map(|live| live.activity.as_ref())
        .map(|activity| CardHold::new(Arc::clone(activity)))
        .collect()
}

/// Snapshot for the `agent_session_live` command. `lastEventAt` and
/// `progress` come from the attached activity clock (Paperclip PR 15b); both
/// are `None` before one is attached.
pub fn live() -> Option<LiveAgentSession> {
    live_where(|_| true)
}

/// [`live`] for one conversation.
pub fn live_for_key(key: &str) -> Option<LiveAgentSession> {
    live_where(|live| live.key == key)
}

/// Every live turn, newest first (build programme PR B4: the rail marks each
/// working conversation).
pub fn live_all() -> Vec<LiveAgentSession> {
    let Ok(guard) = LIVE.lock() else {
        return Vec::new();
    };
    guard.iter().rev().map(snapshot).collect()
}

/// The newest live turn `matches` accepts.
fn live_where(matches: impl Fn(&LiveSession) -> bool) -> Option<LiveAgentSession> {
    let guard = LIVE.lock().ok()?;
    guard.iter().rev().find(|live| matches(live)).map(snapshot)
}

fn snapshot(live: &LiveSession) -> LiveAgentSession {
    let snapshot = live.activity.as_ref().map(|activity| activity.snapshot());
    LiveAgentSession {
        key: live.key.clone(),
        agent: live.agent.as_str().to_string(),
        run_id: live.run_id.clone(),
        phase: live.phase.as_str().to_string(),
        phase_entered_at: live.phase_entered_at.clone(),
        failure_reason: live.failure_reason.clone(),
        last_event_at: snapshot.as_ref().and_then(|s| s.last_event_at.clone()),
        progress: snapshot.and_then(|s| s.progress).map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Take the serialization lock ([`LIVE_TEST_LOCK`]) and start from a
    /// known-empty record. Tolerates a poisoned lock so one failing test
    /// doesn't cascade.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        let lock = LIVE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        lock
    }

    #[test]
    fn minted_keys_are_safe_and_unique() {
        let a = mint_session_key();
        let b = mint_session_key();
        assert!(is_safe_session_key(&a), "minted key should validate: {a}");
        assert!(is_safe_session_key(&b));
        assert_ne!(a, b, "two mints should not collide");
        assert!(a.starts_with("s-"));
        assert_eq!(a.len(), 19, "s- + 8 date + - + 8 hex, got {a}");
    }

    #[test]
    fn unsafe_keys_are_rejected() {
        // Traversal and separators are unrepresentable in the charset, which is
        // the point — these must all fail, and none of them may ever reach a
        // path join.
        for bad in [
            "",
            "..",
            "s-../../etc/passwd",
            "s-2026/08/11",
            "s-2026\\08",
            "s-2026 08",
            "s-UPPER123",
            "no-prefix-2026",
            "s-2026.08.11",
            "s-2026;drop",
        ] {
            assert!(
                !is_safe_session_key(bad),
                "should reject {bad:?} but it passed"
            );
        }
        let too_long = format!("s-{}", "a".repeat(SESSION_KEY_MAX));
        assert!(!is_safe_session_key(&too_long));
    }

    #[test]
    fn live_record_tracks_begin_phase_and_clear() {
        let _lock = guard();
        assert!(live().is_none(), "no live session means no record");

        begin(
            "s-20260811-deadbeef",
            AgentKind::Claude,
            "run-1",
            SessionPhase::Preparing,
        );
        assert_eq!(live().expect("a live snapshot").phase, "preparing");

        set_phase("run-1", SessionPhase::Running);
        let snapshot = live().expect("a live snapshot");
        assert_eq!(snapshot.phase, "running");
        assert_eq!(snapshot.run_id, "run-1");
        assert_eq!(snapshot.agent, "claude");

        clear();
        assert!(live().is_none(), "clear must release the record");
    }

    #[test]
    fn busy_reason_names_any_holder_and_adds_chat_detail_only_for_its_own_lease() {
        use crate::automated_history::{begin_run, finish_run, lease_holder};
        let _lock = guard();
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        assert!(busy_reason(v).is_none(), "a free lease has no reason");

        // A non-chat holder is named even though no live record exists; the
        // removed chat-only `is_engaged()` could never see it.
        let workflow = begin_run(v, "workflow", Some("x")).unwrap();
        let reason = busy_reason(v).expect("a workflow holds the lease");
        assert!(reason.contains("workflow `x`"), "got {reason}");
        assert!(live().is_none(), "no chat record exists for a workflow run");
        let holder = lease_holder(v).unwrap();
        assert!(busy_message(&holder).starts_with("Hollow is busy: workflow `x`"));
        finish_run(v, workflow, "success").unwrap();
        assert!(busy_reason(v).is_none());

        // The chat turn that holds the lease adds its live phase.
        let chat = begin_run(v, "chat", None).unwrap();
        begin(
            "s-20260925-0000000c",
            AgentKind::Claude,
            &chat.id,
            SessionPhase::Preparing,
        );
        let reason = busy_reason(v).expect("the chat turn holds the lease");
        assert!(reason.contains("Claude"), "got {reason}");
        assert!(reason.contains("preparing"), "got {reason}");
        let holder = lease_holder(v).unwrap();
        let message = busy_message(&holder);
        assert!(
            message.starts_with("Hollow is busy: a Claude turn"),
            "got {message}"
        );

        // A record for some other run never decorates this holder.
        clear_run(&chat.id);
        begin(
            "s-20260925-0000000d",
            AgentKind::Codex,
            "run-unrelated",
            SessionPhase::Running,
        );
        assert_eq!(busy_reason(v).as_deref(), Some("a chat turn"));
        assert_eq!(busy_message(&holder), holder.busy_message());

        clear();
        finish_run(v, chat, "success").unwrap();
        assert!(busy_reason(v).is_none());
    }

    /// Paperclip PR 15b: the live snapshot carries the attached activity's
    /// last event and progress; a foreign run id never attaches.
    #[test]
    fn live_snapshot_carries_last_event_and_progress() {
        use crate::run_activity::ProgressPhase;
        let _lock = guard();
        begin(
            "s-20260926-0000000e",
            AgentKind::Codex,
            "run-act",
            SessionPhase::Running,
        );
        let snapshot = live().expect("live");
        assert!(
            snapshot.last_event_at.is_none() && snapshot.progress.is_none(),
            "nothing attached"
        );

        let foreign = Arc::new(RunActivity::new());
        foreign.mark_spawned();
        attach_activity("run-other", Arc::clone(&foreign));
        assert!(
            live().unwrap().last_event_at.is_none(),
            "a foreign run id is ignored"
        );

        let activity = Arc::new(RunActivity::new());
        attach_activity("run-act", Arc::clone(&activity));
        assert!(live().unwrap().last_event_at.is_none(), "not spawned yet");
        activity.mark_spawned();
        activity.touch_output();
        activity.set_progress(ProgressPhase::Researching);
        let snapshot = live().expect("live");
        let at = snapshot.last_event_at.expect("a last event");
        assert!(chrono::DateTime::parse_from_rfc3339(&at).is_ok(), "{at}");
        assert_eq!(snapshot.progress.as_deref(), Some("researching"));
        clear();
    }

    /// Merged plan slice 3 (A·R2): a card bound to a conversation pauses
    /// only that turn's watchdog; an unbound one pauses every live turn;
    /// dropping the holds releases them.
    #[test]
    fn a_card_holds_its_conversation_or_every_turn_when_unbound() {
        let _lock = guard();
        let (key_a, key_b) = ("s-20261002-0000000a", "s-20261002-0000000b");
        begin(
            key_a,
            AgentKind::Claude,
            "run-hold-a",
            SessionPhase::Running,
        );
        begin(key_b, AgentKind::Grok, "run-hold-b", SessionPhase::Running);
        let a = Arc::new(RunActivity::new());
        let b = Arc::new(RunActivity::new());
        a.mark_spawned();
        b.mark_spawned();
        attach_activity("run-hold-a", Arc::clone(&a));
        attach_activity("run-hold-b", Arc::clone(&b));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let held = |x: &RunActivity| x.quiet_since_mono_ms() > x.last_activity_mono_ms();
        assert!(!held(&a) && !held(&b));

        let holds = hold_watchdog(Some(key_a));
        assert_eq!(holds.len(), 1);
        assert!(held(&a) && !held(&b), "only its own conversation");
        drop(holds);
        assert!(!held(&a), "released");

        std::thread::sleep(std::time::Duration::from_millis(20));
        let all = hold_watchdog(None);
        assert_eq!(all.len(), 2);
        assert!(held(&a) && held(&b), "an unbound card holds every turn");
        drop(all);
        assert!(hold_watchdog(Some("s-20261002-0000000c")).is_empty());
        clear();
    }

    #[test]
    fn phase_restamps_only_on_change() {
        let _lock = guard();
        begin(
            "s-20260811-0000000a",
            AgentKind::Codex,
            "run-2",
            SessionPhase::Running,
        );
        let first = live().expect("live").phase_entered_at;
        set_phase("run-2", SessionPhase::Running);
        let same = live().expect("live").phase_entered_at;
        assert_eq!(first, same, "re-entering the same phase should not restamp");
        clear();
    }

    /// Build programme PR B4: two conversations' turns are live at once, each
    /// with its own phase; clearing one leaves the other.
    #[test]
    fn two_conversations_have_their_own_live_records() {
        let _lock = guard();
        begin(
            "s-20260927-0000000a",
            AgentKind::Claude,
            "run-a",
            SessionPhase::Running,
        );
        begin(
            "s-20260927-0000000b",
            AgentKind::Codex,
            "run-b",
            SessionPhase::Preparing,
        );
        assert_eq!(live_all().len(), 2);
        set_phase_for_key("s-20260927-0000000a", SessionPhase::Cancelling);
        assert_eq!(
            live_for_key("s-20260927-0000000a").unwrap().phase,
            "cancelling"
        );
        assert_eq!(
            live_for_key("s-20260927-0000000b").unwrap().phase,
            "preparing"
        );
        clear_run("run-a");
        assert!(live_for_key("s-20260927-0000000a").is_none());
        assert_eq!(live().unwrap().run_id, "run-b");
        // A newer turn of the same conversation replaces its record.
        begin(
            "s-20260927-0000000b",
            AgentKind::Codex,
            "run-c",
            SessionPhase::Running,
        );
        assert_eq!(live_all().len(), 1);
        clear();
    }

    #[test]
    fn failure_reason_is_carried() {
        let _lock = guard();
        begin(
            "s-20260811-0000000b",
            AgentKind::Claude,
            "run-3",
            SessionPhase::Running,
        );
        fail("run-3", "spawn claude: No such file or directory");
        let snapshot = live().expect("live");
        assert_eq!(snapshot.phase, "failed");
        assert_eq!(
            snapshot.failure_reason.as_deref(),
            Some("spawn claude: No such file or directory")
        );
        assert!(
            live().is_some(),
            "a failed-but-unfinalized turn keeps its record until finish clears it"
        );
        clear();
    }
}
