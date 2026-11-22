//! A wake-up an agent asks for (overnight C3): `sessions.wake_me {minutes,
//! reason}` queues a continue turn for the calling chat later, so an agent
//! can come back to slow work without a turn that waits. It replaces the
//! CLIs' own ScheduleWakeup/CronCreate inside Hollow, which never resume a
//! Hollow chat (A3's receipt flags those).
//!
//! **Bounded.** 1 to [`MAX_MINUTES`] minutes ahead; one pending wake per
//! conversation (asking again replaces it); at most [`MAX_PER_DAY`] in a
//! rolling day per conversation; only from a chat outside a build workspace
//! (never an automation or an unattended turn: `main_agent::unattended::SAFE`
//! doesn't list it, so a woken turn can't wake itself again), and only where
//! the timer runs (refused in a candidate build). The woken turn resumes the
//! same conversation **unattended** (`ChatSendInput::unattended`: the Main
//! agent's unattended limits apply, so only the safe Hollow actions, no
//! teammates, computer control, services or shared connectors) at the
//! asking turn's depth, in the conversation's current mode and never above
//! the asking turn's ([`woken_mode`]; until 2026-11-12 also capped at
//! `acceptEdits`), like the Main agent's own wakes.
//! A stopped asking turn cancels its wake, and so does the person's next
//! message in that conversation.
//!
//! **Visible and cancellable.** The asking turn's receipt says when it wakes
//! and why ([`receipt_line`]); `sessions.wakes` lists the pending ones and
//! `sessions.cancel_wake` cancels one (the conversation's own by default).
//! A wake whose conversation is busy at its time waits for the next minute;
//! one [`LATE_MINUTES`] late is dropped as missed, never stacked, and so is
//! one due past tonight's overnight stop. A woken turn's cost counts in the
//! overnight scope, and a woken turn of a durable child task joins its
//! delegation, so the parent hears when it ends ([`link_to_delegator`]).
//!
//! **When children end** (overnight OB10b). `sessions.wake_me
//! {onSessionsEnd: {keys, mode}}` wakes the conversation when child tasks it
//! started end instead of at a time ([`schedule_on_end`]): `any`, the first
//! of them; `all` (the default), every one. Each key must be a conversation
//! the caller started (`list_sessions` `mine`), and a chain that hands over
//! ends with its last successor. The chat-end hook
//! (`main_agent::after_chat_finished`), or the minute timer for a child
//! that ended without a `finished` event, marks it due once
//! ([`arm_ended`]); from then on it is an ordinary wake (same caps, same
//! unattended turn, the person's next message cancels it), whose message
//! carries each ended child's line and summary
//! (`descendants::child_summary`), reported once like the Main agent's wake
//! (the next note doesn't repeat them). It waits at most
//! [`SESSIONS_END_HOURS`], and never in a Main agent thread, which is woken
//! for its chats already. This is the only wake an orchestrator gets: a
//! woken parent runs at acceptEdits or lower with the safe actions, and
//! widening that is S19 (the person's call).
//!
//! Storage: `<support>/Wakes/<vault>.json`, the newest [`KEPT`].

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const MAX_MINUTES: i64 = 120;
pub const MAX_PER_DAY: usize = 24;
pub const LATE_MINUTES: i64 = 30;
const KEPT: usize = 300;
const REASON_MAX: usize = 300;
/// How long a wake on children's ends waits for them, at most (one lane
/// chat ran 11.6 h): past it, it is missed.
pub const SESSIONS_END_HOURS: i64 = 24;
/// Conversations one wake on children's ends watches, at most.
pub const MAX_WATCHED: usize = crate::descendants::WAIT_MAX_KEYS;
/// A wake on children's ends opens with this when the agent gave no reason.
const END_REASON: &str = "the chats you were waiting for";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Wake {
    pub id: String,
    pub session_key: String,
    /// The run that asked (its receipt shows the wake).
    pub run_id: String,
    pub created_at: String,
    /// When it is due. A wake on children's ends: the latest it waits for
    /// them until they end ([`arm_ended`]), then when they did.
    pub at: String,
    pub reason: String,
    pub mode: String,
    pub depth: u32,
    /// `pending`, `queueing` (claimed, being sent), `queued`, `cancelled`
    /// or `missed`.
    pub status: String,
    #[serde(default)]
    pub queue_id: Option<i64>,
    /// When it was claimed for sending (`queueing`, a `db_time`): a send
    /// that never settled is recovered from the turns requested since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    /// A wake on children's ends instead of at a time (overnight OB10b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_sessions_end: Option<SessionsEnd>,
    /// A project lead's batch report (L8, [`join_batch`]): Hollow asked for
    /// it, not the agent. The person's next message and a handover leave it
    /// (it reports to the lead's current conversation), and a budget stop
    /// drops it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub batch: bool,
    /// The last minute it was due while its conversation was busy (RFC
    /// 3339; PER-12, 11-14). Lateness counts from then, so a wake that comes
    /// due during a long turn runs once the turn ends instead of being
    /// missed, and the person's message no longer cancels it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_at: Option<String>,
    /// The Mac it was asked on (`runner_coo::machine_owner`, M3b): another
    /// Mac never fires it, so a wake file copied with an app data folder
    /// can't wake a conversation twice. None (older wakes) fires anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

/// What a wake on children's ends waits for (overnight OB10b).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsEnd {
    /// `any` (the first to end wakes it) or `all` (every one ended).
    pub mode: String,
    pub watched: Vec<Watched>,
    /// When they ended as `mode` asks (RFC 3339), set once by
    /// [`arm_ended`]; the wake's `at` is then that time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<String>,
    /// Also fire as soon as one of them fails (a lead's batch, L8).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_failure: bool,
}

/// One conversation a wake on children's ends watches: one the asking
/// conversation started, and the turn of it that it caused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watched {
    pub key: String,
    pub agent: String,
    /// That turn's run-queue row.
    pub queue_id: i64,
    /// A child task of the asking conversation's own (not a handover
    /// successor down one's chain).
    pub direct: bool,
}

impl SessionsEnd {
    /// `when s-a ends`, `when the first of s-a, s-b ends`, `once s-a, s-b
    /// have all ended`.
    fn when(&self) -> String {
        let keys: Vec<&str> = self.watched.iter().map(|w| w.key.as_str()).collect();
        match (keys.as_slice(), self.mode.as_str()) {
            ([one], _) => format!("when {one} ends"),
            (_, "any") => format!("when the first of {} ends", keys.join(", ")),
            _ => format!("once {} have all ended", keys.join(", ")),
        }
    }
}

fn path(vault: &Path) -> Option<PathBuf> {
    let stem = crate::session_store::vault_file_stem(vault);
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-wakes-").join(format!("{stem}.json")))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|r| r.join("Wakes").join(format!("{stem}.json")))
    }
}

static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The tail of every new wake's id ([`new_wake`]).
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Every subagent that ever joined a lead's batch report (L8), apart from
/// the wakes file, which keeps only the newest [`KEPT`]: orphan adoption
/// ([`adopt_orphans`]) skips these, so a child whose batch was evicted is
/// never adopted and reported twice. The newest [`MEMBERS_KEPT`].
fn members_path(vault: &Path) -> Option<PathBuf> {
    path(vault).map(|p| p.with_extension("batch-members.json"))
}

const MEMBERS_KEPT: usize = 5000;

fn load_members(vault: &Path) -> Vec<String> {
    members_path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Record `key` as a batch member (under [`LOCK`]).
fn remember_member(vault: &Path, key: &str) -> Result<(), String> {
    let mut all = load_members(vault);
    if all.iter().any(|k| k == key) {
        return Ok(());
    }
    all.push(key.to_string());
    let keep = &all[all.len().saturating_sub(MEMBERS_KEPT)..];
    let p = members_path(vault).ok_or("No app data folder.")?;
    let text = serde_json::to_vec(keep).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)
}

/// Wakes this process is sending now (claimed, not yet settled). Any other
/// `queueing` entry is left from a send that never settled: Hollow quit or
/// crashed mid-send ([`recover`], morning M2).
static SENDING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn load(vault: &Path) -> Vec<Wake> {
    path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save(vault: &Path, all: &[Wake]) -> Result<(), String> {
    let p = path(vault).ok_or("No app data folder.")?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let keep = &all[all.len().saturating_sub(KEPT)..];
    let text = serde_json::to_vec_pretty(keep).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&p, &text)
}

fn parse(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Check and build a wake for the calling chat (nothing saved).
pub fn plan(
    all: &[Wake],
    caller: &crate::app_actions::Caller,
    minutes: i64,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Wake, String> {
    chat_only(caller)?;
    if !(1..=MAX_MINUTES).contains(&minutes) {
        return Err(format!(
            "Wake in 1 to {MAX_MINUTES} minutes. Nothing scheduled."
        ));
    }
    let reason = reason_within_cap(all, caller, reason, now)?;
    Ok(new_wake(
        caller,
        reason,
        now + Duration::minutes(minutes),
        now,
    ))
}

/// Check and build a wake for the calling chat on its children's ends
/// (overnight OB10b; nothing saved). `watched` is already checked to be the
/// caller's own ([`schedule_on_end`]); an empty `reason` reads
/// [`END_REASON`].
pub fn plan_on_end(
    all: &[Wake],
    caller: &crate::app_actions::Caller,
    watched: Vec<Watched>,
    mode: &str,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Wake, String> {
    chat_only(caller)?;
    let reason = if reason.trim().is_empty() {
        END_REASON
    } else {
        reason
    };
    let reason = reason_within_cap(all, caller, reason, now)?;
    let mut wake = new_wake(
        caller,
        reason,
        now + Duration::hours(SESSIONS_END_HOURS),
        now,
    );
    wake.on_sessions_end = Some(SessionsEnd {
        mode: mode.to_string(),
        watched,
        fired_at: None,
        on_failure: false,
    });
    Ok(wake)
}

/// Only a chat asks for a wake, and only where the timer runs.
fn chat_only(caller: &crate::app_actions::Caller) -> Result<(), String> {
    if caller.surface != "chat" || !crate::agent_sessions::is_safe_session_key(caller.scope.trim())
    {
        return Err("Only a chat can ask to be woken (an automation runs on its own schedule). Nothing scheduled.".into());
    }
    if !crate::instance::background_agents_allowed() {
        return Err("This Hollow doesn't run background work (a candidate build), so a wake-up would never come. Nothing scheduled.".into());
    }
    Ok(())
}

/// `reason` on one line, checked, while the conversation is under the daily
/// cap.
fn reason_within_cap(
    all: &[Wake],
    caller: &crate::app_actions::Caller,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let reason = reason.split_whitespace().collect::<Vec<_>>().join(" ");
    if reason.is_empty() || reason.chars().count() > REASON_MAX {
        return Err(format!(
            "Say why in 1 to {REASON_MAX} characters (it opens the woken turn). Nothing scheduled."
        ));
    }
    let today = now - Duration::hours(24);
    let used = all
        .iter()
        // A lead's batch reports have their own cap (L8, [`BATCH_MAX_PER_DAY`]).
        .filter(|w| w.session_key == caller.scope && w.status != "cancelled" && !w.batch)
        .filter(|w| parse(&w.created_at).is_some_and(|t| t >= today))
        .count();
    if used >= MAX_PER_DAY {
        return Err(format!(
            "This conversation asked for {MAX_PER_DAY} wake-ups in the last day, the most Hollow allows; tell the person where things stand instead. Nothing scheduled."
        ));
    }
    Ok(reason)
}

/// A pending wake of the calling chat, due `at`.
fn new_wake(
    caller: &crate::app_actions::Caller,
    reason: String,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Wake {
    Wake {
        // A per-process counter keeps ids apart for wakes made at the same
        // instant (L8: orphan adoption makes several with one `now`).
        id: format!(
            "wake-{}-{:04x}-{:x}",
            now.format("%Y%m%d-%H%M%S"),
            now.timestamp_subsec_nanos() % 0x10000,
            NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ),
        session_key: caller.scope.clone(),
        run_id: caller.run_id.clone(),
        created_at: now.to_rfc3339(),
        at: at.to_rfc3339(),
        reason,
        mode: crate::app_mcp::normalize_perm(&caller.perm).to_string(),
        depth: caller.depth,
        status: "pending".into(),
        queue_id: None,
        claimed_at: None,
        on_sessions_end: None,
        batch: false,
        busy_at: None,
        owner: crate::runner_coo::machine_owner(),
    }
}

/// Whether `cwd` is a build workspace's worktree: a wake is refused there
/// when it is asked for and again when it fires (overnight S5: a workspace
/// merge or a chat move can carry a conversation into one meanwhile).
fn in_build_workspace(vault: &Path, cwd: &Path) -> bool {
    crate::build_workspaces::workspace_for_worktree(vault, cwd).is_some()
}

/// Why a due wake of conversation `key` (now in `cwd`) is dropped instead
/// of sent: it moved into a build workspace, or it handed its work over
/// since (a wake claimed just before the handover escapes its cancel). A
/// lead's batch report (`batch`) survives a handover: its send goes to the
/// successor (`chat_chain::route_send`).
fn dropped_at_fire_of(vault: &Path, key: &str, cwd: &Path, batch: bool) -> Option<&'static str> {
    if in_build_workspace(vault, cwd) {
        Some("now in a build workspace")
    } else if !batch && crate::app_sessions::successor_of(vault, key).is_some() {
        Some("handed over since")
    } else {
        None
    }
}

/// `sessions.wake_me`: save the wake, replacing the conversation's pending
/// one.
pub fn schedule(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    minutes: i64,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    let (wake, replaced) =
        save_replacing(vault, caller, |all| plan(all, caller, minutes, reason, now))?;
    let local = local_time(&wake.at);
    Ok(json!({
        "id": wake.id,
        "at": wake.at,
        "replaced": replaced,
        "note": format!("Hollow wakes this conversation at {local} with a continue turn. It runs unattended: in this conversation's own mode, only the safe Hollow actions, and it can't schedule another wake. End your turn now; cancel it with sessions.cancel_wake if you finish first."),
    }))
}

const BOTH: &str = "Give minutes or onSessionsEnd, not both. Nothing scheduled.";

/// `sessions.wake_me` with its arguments as given: `minutes` (with
/// `reason`) or `onSessionsEnd` (overnight OB10b), not both.
pub fn ask(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    minutes: Option<i64>,
    on_end: Option<&Value>,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    match (minutes, on_end) {
        (Some(_), Some(_)) => Err(BOTH.into()),
        (None, Some(on_end)) => schedule_on_end(vault, caller, on_end, reason, now),
        (Some(minutes), None) => schedule(vault, caller, minutes, reason, now),
        (None, None) => Err(format!(
            "Say when: minutes (1 to {MAX_MINUTES}, with a reason) or onSessionsEnd {{keys: [conversations you started]}} to be woken when they end. Nothing scheduled."
        )),
    }
}

/// `onSessionsEnd`'s shape as agents see it (`sessions.wake_me`).
pub(crate) fn on_sessions_end_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "keys": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "maxItems": MAX_WATCHED,
            },
            "mode": { "type": "string", "enum": ["all", "any"] },
        },
        "required": ["keys"],
        "additionalProperties": false,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OnSessionsEnd {
    keys: Vec<String>,
    #[serde(default)]
    mode: Option<String>,
}

/// `onSessionsEnd {keys, mode?}`: its keys (trimmed, each once, 1 to
/// [`MAX_WATCHED`]) and its mode (`all` by default).
fn parse_on_end(value: &Value) -> Result<(Vec<String>, &'static str), String> {
    let asked: OnSessionsEnd = serde_json::from_value(value.clone()).map_err(|e| {
        format!("onSessionsEnd is {{keys: [session keys], mode?: \"all\" | \"any\"}} ({e}). Nothing scheduled.")
    })?;
    let mode = match asked.mode.as_deref().map(str::trim) {
        None | Some("") | Some("all") => "all",
        Some("any") => "any",
        Some(other) => {
            return Err(format!(
                "onSessionsEnd mode is all or any, not {other:?}. Nothing scheduled."
            ));
        }
    };
    let mut keys: Vec<String> = Vec::new();
    for key in asked.keys {
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err("onSessionsEnd: a key is empty (list_sessions mine lists yours). Nothing scheduled.".into());
        }
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.is_empty() || keys.len() > MAX_WATCHED {
        return Err(format!(
            "onSessionsEnd names 1 to {MAX_WATCHED} sessions. Nothing scheduled."
        ));
    }
    Ok((keys, mode))
}

/// Whether conversation `key` is a Main agent thread: the Main agent is
/// woken once when a chat it started ends already (`main_agent::decide`),
/// so a wake on children's ends there would wake it twice (and a wait's
/// advice there doesn't offer one, `descendants::answer`).
pub(crate) fn in_main_agent(vault: &Path, key: &str) -> bool {
    crate::main_agent::main_cwd(vault).is_some_and(|cwd| {
        crate::session_store::session_cwd(vault, key).as_deref() == Some(cwd.as_str())
    })
}

/// Where watched conversation `w`'s chain ended, once it has
/// (`descendants::chain_end`).
fn ended(vault: &Path, w: &Watched) -> Option<crate::descendants::ChainEnd> {
    crate::descendants::chain_end(vault, &w.key, &w.agent, w.queue_id)
        .filter(|end| end.row.status.is_terminal())
}

/// Whether `end`'s children ended as its mode asks (or, for a lead's
/// batch, one of them failed).
fn children_ended(vault: &Path, end: &SessionsEnd) -> bool {
    if end.on_failure
        && end.watched.iter().any(|w| {
            ended(vault, w).is_some_and(|e| e.row.status == crate::run_queue::RowStatus::Failed)
        })
    {
        return true;
    }
    let mut each = end.watched.iter().map(|w| ended(vault, w).is_some());
    if end.mode == "any" {
        each.any(|e| e)
    } else {
        each.all(|e| e)
    }
}

/// `sessions.wake_me {onSessionsEnd: {keys, mode?}, reason?}` (overnight
/// OB10b): save a wake for when the caller's children end, replacing the
/// conversation's pending one. Each key must be a conversation the caller
/// started (`descendants::my_turns`); refused in a Main agent thread, and
/// when they already ended as `mode` asks.
pub fn schedule_on_end(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    on_end: &Value,
    reason: &str,
    now: DateTime<Utc>,
) -> Result<Value, String> {
    chat_only(caller)?;
    let (keys, mode) = parse_on_end(on_end)?;
    if in_main_agent(vault, &caller.scope) {
        return Err("The COO is already woken once when a chat it started ends, so it needs no wake for that. Nothing scheduled.".into());
    }
    let mut watched = Vec::new();
    for (key, turn) in keys
        .iter()
        .zip(crate::descendants::my_turns(vault, caller, &keys))
    {
        let Some((agent, queue_id, direct)) = turn else {
            return Err(format!("{key} isn't a conversation you started (list_sessions mine lists yours), so its end can't wake you. Nothing scheduled."));
        };
        watched.push(Watched {
            key: key.clone(),
            agent,
            queue_id,
            direct,
        });
    }
    let done: Vec<&str> = watched
        .iter()
        .filter(|w| ended(vault, w).is_some())
        .map(|w| w.key.as_str())
        .collect();
    let met = if mode == "any" {
        !done.is_empty()
    } else {
        done.len() == watched.len()
    };
    if met {
        return Err(format!(
            "{} already ended: read_session has the answer, so there is nothing to be woken for. Nothing scheduled.",
            done.join(", ")
        ));
    }
    let (wake, replaced) = save_replacing(vault, caller, |all| {
        plan_on_end(all, caller, watched, mode, reason, now)
    })?;
    let when = wake
        .on_sessions_end
        .as_ref()
        .map(SessionsEnd::when)
        .unwrap_or_default();
    let until = local_time(&wake.at);
    Ok(json!({
        "id": wake.id,
        "onSessionsEnd": wake.on_sessions_end,
        "until": wake.at,
        "replaced": replaced,
        "note": format!("Hollow wakes this conversation {when} (waiting until {until} at most) with a continue turn carrying their answers. It runs unattended: in this conversation's own mode, only the safe Hollow actions, and it can't schedule another wake. End your turn now; don't wait for them. Cancel it with sessions.cancel_wake if you no longer need it."),
    }))
}

/// Save the wake `build` makes from the saved ones, replacing the calling
/// conversation's pending one (whose id is returned). Refused in a build
/// workspace and after a handover.
fn save_replacing(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    build: impl FnOnce(&[Wake]) -> Result<Wake, String>,
) -> Result<(Wake, Option<String>), String> {
    if let Ok(detail) = crate::session_store::read_session(vault, &caller.scope) {
        if in_build_workspace(vault, Path::new(&detail.summary.cwd)) {
            return Err("A build workspace's chat can't be woken unattended: nobody watching never edits a person's branch. Nothing scheduled.".into());
        }
    }
    if let Some(next) = crate::app_sessions::successor_of(vault, &caller.scope) {
        return Err(format!("This conversation handed its work to {next}; the successor carries it on, so it isn't woken. Nothing scheduled."));
    }
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    let wake = build(&all)?;
    let mut replaced = None;
    for w in all.iter_mut() {
        // A lead's batch report (L8) is Hollow's, not the agent's: an
        // agent's own wake never replaces it. Another Mac's pending wake
        // for the same conversation is replaced too (M3b): replacing only
        // cancels, it never fires one, and that Mac never fires it here.
        if w.session_key == wake.session_key && w.status == "pending" && !w.batch {
            w.status = "cancelled".into();
            replaced = Some(w.id.clone());
        }
    }
    all.push(wake.clone());
    save(vault, &all)?;
    Ok((wake, replaced))
}

/// An RFC 3339 time as local `HH:MM` (empty when unreadable).
fn local_time(at: &str) -> String {
    parse(at)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}

/// The chat-end hook (`main_agent::after_chat_finished`, overnight OB10b)
/// and the minute timer (`main_agent::start_timer`, for children that end
/// without a `finished` event: cancelled while queued, interrupted at
/// startup): mark each pending wake on children's ends whose children
/// ended as its mode asks as due now, once (its `firedAt`); one past its
/// wait is left for [`claim_due`] to drop. Returns how many such wakes are
/// due and unsent, so the hook sends them at once ([`run_due`]).
pub(crate) fn arm_ended(vault: &Path, now: DateTime<Utc>) -> usize {
    adopt_orphans(vault, now);
    // Only this Mac's (M3b): another Mac's children aren't in this DB.
    let waiting = |w: &Wake| {
        w.status == "pending"
            && crate::runner_coo::owned_here(w.owner.as_deref())
            && w.on_sessions_end
                .as_ref()
                .is_some_and(|end| end.fired_at.is_none())
            && parse(&w.at).is_some_and(|until| now <= until)
    };
    // The queue is read outside the lock (a person's send takes it).
    let saved = load(vault);
    let ready: Vec<(String, String)> = saved
        .iter()
        .filter(|&w| waiting(w))
        // A lead's batch is complete only once the turn that started it has
        // ended (a turn may start more): it waits while the lead works.
        .filter(|w| !w.batch || !lead_busy(vault, &w.session_key))
        .filter(|w| {
            w.on_sessions_end
                .as_ref()
                .is_some_and(|end| children_ended(vault, end))
        })
        .map(|w| (w.id.clone(), w.session_key.clone()))
        .collect();
    let fired_before = saved
        .iter()
        .filter(|w| w.status == "pending" && crate::runner_coo::owned_here(w.owner.as_deref()))
        .filter(|w| {
            w.on_sessions_end
                .as_ref()
                .is_some_and(|end| end.fired_at.is_some())
        })
        .count();
    if ready.is_empty() {
        return fired_before;
    }
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    let mut armed = 0;
    for w in all.iter_mut() {
        // By id and conversation (a lead's root for a batch).
        if !ready
            .iter()
            .any(|(id, key)| *id == w.id && *key == w.session_key)
            || !waiting(&*w)
        {
            continue;
        }
        if let Some(end) = w.on_sessions_end.as_mut() {
            end.fired_at = Some(now.to_rfc3339());
            w.at = now.to_rfc3339();
            armed += 1;
        }
    }
    if armed > 0 {
        let _ = save(vault, &all);
    }
    armed + fired_before
}

/// Whether lead conversation `key`'s chain has a turn queued or running.
fn lead_busy(vault: &Path, key: &str) -> bool {
    crate::chat_chain::is_live(vault, &crate::chat_chain::handover_tail(vault, key))
}

/// The batch report's reason (L8): what the woken lead turn opens with.
pub(crate) const BATCH_REASON: &str = "the subagents you started have ended or one failed; report to the person in two or three lines and record decisions and outcomes in your notes";

/// Batch reports one lead may get in a rolling day (L8). Their own cap,
/// apart from the lead's own `wake_me` ones ([`MAX_PER_DAY`]), so a lead that
/// delegates again from its report turn keeps being reported to, up to a
/// generous bound; past it a report is dropped with a log line, and the
/// children still reach the lead's next note.
pub const BATCH_MAX_PER_DAY: usize = 96;

/// The conversations a lead's batch spans: its fixed conversation (`root`)
/// and every successor its handovers queued, in order.
fn lead_chain(vault: &Path, root: &str) -> Vec<String> {
    let next = crate::chat_chain::handover_map(vault);
    let mut out = vec![root.to_string()];
    while let Some(succ) = next.get(out.last().unwrap()) {
        if out.contains(succ) || out.len() > 64 {
            break;
        }
        out.push(succ.clone());
    }
    out
}

/// The conversations a wake's turn may have been queued in: a lead's batch
/// report goes to its chain's current conversation, so all of it; any other
/// wake, its own.
fn wake_chain(vault: &Path, w: &Wake) -> Vec<String> {
    if w.batch {
        lead_chain(vault, &w.session_key)
    } else {
        vec![w.session_key.clone()]
    }
}

/// L8 (2026-11-10): a project lead started subagent `child` (its turn
/// `queue_id`, on `agent`): add it to the lead's batch report, a wake on
/// its children's ends Hollow keeps for it. The batch is keyed by the lead's
/// fixed conversation (`chat_chain::lead_root`), so one batch spans a
/// handover and reports once, to the chain's current conversation. One
/// batch is open at a time: a start while one waits joins it, so it fires
/// once, when every subagent in it has ended (or one fails) and the lead is
/// idle. Batch reports have their own daily cap ([`BATCH_MAX_PER_DAY`]); a
/// start past it, or past [`MAX_WATCHED`], is logged and its end reaches the
/// lead's next note. The COO isn't batched: it is woken for its chats already
/// (`main_agent::decide`). Returns whether it was recorded.
pub(crate) fn join_batch(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    child: &str,
    agent: &str,
    queue_id: i64,
    now: DateTime<Utc>,
) -> bool {
    if chat_only(caller).is_err() || in_main_agent(vault, &caller.scope) {
        return false;
    }
    let Some(root) = crate::chat_chain::lead_root(vault, &caller.scope) else {
        return false;
    };
    let watched = Watched {
        key: child.to_string(),
        agent: agent.to_string(),
        queue_id,
        direct: true,
    };
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    if load_members(vault).contains(&watched.key)
        || all.iter().any(|w| {
            w.batch
                && w.on_sessions_end
                    .as_ref()
                    .is_some_and(|end| end.watched.iter().any(|x| x.key == watched.key))
        })
    {
        return true;
    }
    // Membership is recorded only once the batch holding the child is saved
    // (L8 delta review): recorded first, a failed save would leave a member
    // no batch watches, which adoption then skips and nothing reports.
    let member = watched.key.clone();
    let saved = |all: &[Wake]| -> bool {
        if save(vault, all).is_err() {
            return false;
        }
        if let Err(e) = remember_member(vault, &member) {
            eprintln!("[wake_me] {child}: its batch membership wasn't recorded: {e}");
        }
        true
    };
    let open = all.iter_mut().rev().find(|w| {
        w.batch
            && w.status == "pending"
            && w.session_key == root
            && w.on_sessions_end
                .as_ref()
                .is_some_and(|end| end.fired_at.is_none())
    });
    if let Some(w) = open {
        if let Some(end) = w.on_sessions_end.as_mut() {
            if end.watched.len() >= MAX_WATCHED {
                eprintln!(
                    "[wake_me] {}: batch report for {root} is full ({MAX_WATCHED}); {child} reaches the lead's next note instead",
                    w.id
                );
                return false;
            }
            end.watched.push(watched);
        }
        w.at = (now + Duration::hours(SESSIONS_END_HOURS)).to_rfc3339();
        return saved(&all);
    }
    let today = now - Duration::hours(24);
    let used = all
        .iter()
        .filter(|w| w.batch && w.session_key == root && w.status != "cancelled")
        .filter(|w| parse(&w.created_at).is_some_and(|t| t >= today))
        .count();
    if used >= BATCH_MAX_PER_DAY {
        eprintln!(
            "[wake_me] {root}: {BATCH_MAX_PER_DAY} batch reports in the last day; no report for {child}, its end reaches the lead's next note"
        );
        return false;
    }
    let mut wake = new_wake(
        caller,
        BATCH_REASON.to_string(),
        now + Duration::hours(SESSIONS_END_HOURS),
        now,
    );
    wake.session_key = root;
    wake.batch = true;
    wake.on_sessions_end = Some(SessionsEnd {
        mode: "all".into(),
        watched: vec![watched],
        fired_at: None,
        on_failure: true,
    });
    all.push(wake);
    saved(&all)
}

/// L8 crash recovery: a lead's subagent whose start was queued but whose
/// batch registration never ran ([`join_batch`] runs just after the queue
/// commits) is adopted into its lead's batch. Read from the live queue rows
/// that name a lead conversation as their subagent parent and that no batch
/// watches yet; a child that already ended reaches the lead's next note.
fn adopt_orphans(vault: &Path, now: DateTime<Utc>) {
    let live = crate::run_queue::live_rows(vault).unwrap_or_default();
    let mut watched: std::collections::HashSet<String> = load(vault)
        .iter()
        .filter(|w| w.batch)
        .filter_map(|w| w.on_sessions_end.as_ref())
        .flat_map(|end| end.watched.iter().map(|x| x.key.clone()))
        .collect();
    // Members of batches the wakes file no longer keeps, too.
    watched.extend(load_members(vault));
    for row in &live {
        let (Some(parent), Some(child)) = (
            crate::run_pump::subagent_parent_of(row),
            row.source_id.as_deref(),
        ) else {
            continue;
        };
        if watched.contains(child) || !crate::chat_chain::is_lead_conversation(vault, parent) {
            continue;
        }
        let agent = row
            .payload
            .get("agent")
            .and_then(Value::as_str)
            .unwrap_or("claude")
            .to_string();
        let caller = crate::app_actions::Caller {
            run_id: String::new(),
            surface: "chat".into(),
            scope: parent.to_string(),
            access: crate::app_mcp::AppAccess::Write,
            depth: 0,
            perm: "acceptEdits".into(),
        };
        if join_batch(vault, &caller, child, &agent, row.id, now) {
            eprintln!("[wake_me] adopted {child} into {parent}'s batch report (its registration was lost)");
        }
    }
}

/// Whether a hard-stop budget would hold lead batch report `w`'s turn now
/// (L8: a budget stop means no report; its children still reach the lead's
/// next note). Read-only: no pause, notice or save.
fn batch_budget_stopped(vault: &Path, w: &Wake, now: DateTime<Utc>) -> bool {
    let agent = crate::session_store::session_by_key(vault, &w.session_key)
        .ok()
        .flatten()
        .and_then(|s| crate::agent::AgentKind::from_opt(&s.agent));
    let intent = crate::budgets::RunIntent::chat(agent);
    matches!(
        crate::budgets::preflight_readonly(
            &crate::budgets::Store::for_app(vault),
            Some(vault),
            &intent,
            now
        ),
        crate::budgets::Preflight::Blocked(_)
    )
}

/// `sessions.cancel_wake`: cancel wake `id`, or the calling conversation's
/// pending one.
pub fn cancel(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    id: Option<&str>,
) -> Result<Value, String> {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    // A conversation cancels its own wakes only.
    let target = all.iter_mut().rev().find(|w| {
        w.status == "pending"
            && w.session_key == caller.scope
            && id.is_none_or(|id| w.id == id.trim())
    });
    let Some(w) = target else {
        return Err("No pending wake-up to cancel. Nothing changed.".into());
    };
    w.status = "cancelled".into();
    let id = w.id.clone();
    save(vault, &all)?;
    Ok(json!({ "cancelled": id }))
}

/// Cancel the pending wakes of run `run_id` (its turn was stopped).
pub fn cancel_run(vault: &Path, run_id: &str) {
    cancel_where(vault, |w| w.run_id == run_id);
}

/// Cancel conversation `key`'s pending wake (the person wrote to it, or it
/// handed over). A lead's batch report stays (L8): the person talking to
/// the lead while its subagents work is the point of delegating, and after
/// a handover the report goes to the successor (`chat_chain::route_send`).
pub fn cancel_session(vault: &Path, key: &str) {
    cancel_where(vault, |w| w.session_key == key && !w.batch);
}

/// The person wrote to conversation `key` (overnight C3): its pending wake is
/// cancelled, unless it has already come due (PER-12, 11-14). A wake that is
/// due, or came due while a turn ran, only waits for the conversation to be
/// free, and the person's message must not swallow it: it runs once that
/// turn ends. A lead's batch report stays, as in [`cancel_session`].
pub fn cancel_session_on_send(vault: &Path, key: &str, now: DateTime<Utc>) {
    cancel_where(vault, |w| {
        w.session_key == key && !w.batch && w.busy_at.is_none() && due(w, now) != Due::Now
    });
}

/// Whatever Mac a matching wake was asked on (M3b): cancelling only stops a
/// wake, never fires one, and the person's message, a stopped run or a
/// cancel names this conversation, which is the same here as there.
fn cancel_where(vault: &Path, matches: impl Fn(&Wake) -> bool) {
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    let mut changed = false;
    for w in all
        .iter_mut()
        .filter(|w| w.status == "pending" && matches(w))
    {
        w.status = "cancelled".into();
        changed = true;
    }
    if changed {
        let _ = save(vault, &all);
    }
}

/// `sessions.wakes`: the pending wake-ups, soonest first.
pub fn pending(vault: &Path) -> Value {
    let mut all: Vec<Wake> = load(vault)
        .into_iter()
        .filter(|w| w.status == "pending")
        .collect();
    all.sort_by(|a, b| a.at.cmp(&b.at));
    json!({ "wakes": all })
}

/// What a wake does at `now`: `Due` (queue it), `Missed` (too late, drop),
/// or nothing yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    Wait,
    Now,
    Missed,
}

pub fn due(w: &Wake, now: DateTime<Utc>) -> Due {
    if w.status != "pending" {
        return Due::Wait;
    }
    // Waiting for children's ends: never due on its own, missed past its wait.
    if w.on_sessions_end
        .as_ref()
        .is_some_and(|end| end.fired_at.is_none())
    {
        return match parse(&w.at) {
            Some(until) if now <= until => Due::Wait,
            _ => Due::Missed,
        };
    }
    // Late counts from the last minute a busy conversation held it (PER-12).
    let held = w.busy_at.as_deref().and_then(parse);
    let late = |at: DateTime<Utc>| {
        let from = held.map_or(at, |busy| busy.max(at));
        now - from > Duration::minutes(LATE_MINUTES)
    };
    match parse(&w.at) {
        Some(at) if now < at => Due::Wait,
        Some(at) if late(at) => Due::Missed,
        Some(_) => Due::Now,
        None => Due::Missed,
    }
}

/// The receipt line for run `run_id`'s wake, if it asked for one still
/// pending.
pub fn receipt_line(vault: &Path, run_id: &str) -> Option<String> {
    let w = load(vault)
        .into_iter()
        .rev()
        .find(|w| w.run_id == run_id && w.status == "pending")?;
    let local = parse(&w.at)?
        .with_timezone(&chrono::Local)
        .format("%H:%M")
        .to_string();
    Some(match &w.on_sessions_end {
        Some(end) if end.fired_at.is_none() => format!(
            "Hollow wakes this chat {} (until {local}): {}",
            end.when(),
            w.reason
        ),
        _ => format!("Hollow wakes this chat at {local}: {}", w.reason),
    })
}

/// How a woken turn's message ends.
const WAKE_TAIL: &str = "Continue where you left off; if nothing is left to do, say so and end the turn. This turn runs unattended (in this conversation's own mode, only the safe Hollow actions) and can't schedule another wake: if work remains, write down where it stands.";

/// The first message of a woken turn.
pub fn wake_prompt(w: &Wake) -> String {
    format!("{WAKE_OPEN} ({}). {WAKE_TAIL}", w.reason)
}

/// What a woken turn on children's ends reports: each ended conversation's
/// turns, recorded as reported once the turn is queued ([`deliver`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Reported {
    /// `(queue row, conversation)` of each direct child's turn it reports.
    pub children: Vec<(i64, String)>,
    /// Turns further down a chain it reports (a successor's).
    pub chain: Vec<i64>,
}

/// The first message of woken turn `w` (overnight OB10b: for a wake on
/// children's ends, [`wake_prompt`] plus a line per ended child with its
/// summary, as the Main agent's wake carries, and which still run), and
/// what it reports.
pub(crate) fn prompt_for(vault: &Path, w: &Wake) -> (String, Reported) {
    let Some(end) = &w.on_sessions_end else {
        return (wake_prompt(w), Reported::default());
    };
    let mut done = Vec::new();
    let mut running = Vec::new();
    for watched in &end.watched {
        match ended(vault, watched) {
            Some(chain) => done.push((watched, chain)),
            None => running.push(watched.key.as_str()),
        }
    }
    // The first `MAX_NOTES` carry a summary, sharing one answer budget.
    let summarized = done.len().min(crate::descendants::MAX_NOTES);
    let answer_chars = crate::descendants::answer_budget(summarized);
    let mut reported = Reported::default();
    let mut lines = Vec::new();
    for (at, (watched, chain)) in done.iter().enumerate() {
        lines.push(crate::descendants::ended_line(
            vault,
            chain,
            &watched.key,
            at < summarized,
            answer_chars,
        ));
        if watched.direct {
            reported
                .children
                .push((watched.queue_id, watched.key.clone()));
        } else {
            reported.chain.push(watched.queue_id);
        }
        reported.chain.extend(&chain.successors);
    }
    let lead = match done.len() {
        1 => "A chat you were waiting for has ended",
        _ => "Chats you were waiting for have ended",
    };
    let mut text = format!(
        "{WAKE_OPEN} ({}). {lead}; each is reported here once, with the end of its answer, its files and pull requests (read_session has more):\n{}\n",
        w.reason,
        lines.join("\n")
    );
    if !running.is_empty() {
        text.push_str(&format!(
            "Still running: {}. Hollow reports each in a later turn's note when it ends.\n",
            running.join(", ")
        ));
    }
    text.push_str(WAKE_TAIL);
    (text, reported)
}

/// Record what woken turn's message reported to conversation `parent`, so
/// its next note and a wait don't report them again (as `wake`, like the
/// Main agent's wake, `main_agent::wake_queued`). Best effort.
pub(crate) fn deliver(vault: &Path, parent: &str, reported: &Reported) {
    for (queue_id, key) in &reported.children {
        let _ = crate::session_store::deliver_child(vault, *queue_id, parent, key, "wake", None);
    }
    let _ = crate::session_store::deliver_to_root(vault, parent, "wake", -1, &reported.chain);
}

/// Forget what claimed wake `w`'s send recorded as reported ([`deliver`],
/// since it was claimed) when that send failed or never settled, so its
/// retry, or the conversation's next note when it is then cancelled or
/// missed, reports those children. Best effort.
fn undeliver(vault: &Path, w: &Wake) {
    let Some(end) = &w.on_sessions_end else {
        return;
    };
    let Some(since) = w
        .claimed_at
        .clone()
        .or_else(|| parse(&w.at).map(crate::session_store::db_time))
    else {
        return;
    };
    let keys: Vec<String> = end.watched.iter().map(|w| w.key.clone()).collect();
    for parent in wake_chain(vault, w) {
        let _ = crate::session_store::undeliver(vault, &parent, &keys, "wake", &since);
    }
}

/// The mode woken turn `w` runs in: the conversation's current mode
/// (`main_agent::wake_mode`, bypass and auto kept), never above the mode of
/// the turn that asked for the wake (a weaker asking turn keeps its wake
/// weaker; a conversation lowered since lowers it). Until 2026-11-12 it was
/// also capped at `acceptEdits`.
fn woken_mode(vault: &Path, w: &Wake) -> String {
    let current = crate::session_store::last_permission(vault, &w.session_key)
        .ok()
        .flatten();
    let current = crate::main_agent::wake_mode(Some(current.as_deref().unwrap_or(&w.mode)));
    crate::app_mcp::lowered(&current, Some(w.mode.as_str())).to_string()
}

/// A woken turn continues the turn that asked for it. When that turn was a
/// durable child task (`start_session`, `send_to_session`, a handover), the
/// woken one joins the same delegation: a record of tool `wake` for the
/// delegating run, so the parent hears when it ends (its next turn's
/// `<hollow-child-finished>` note, the Main agent's report wake, every
/// conversation up the chain) and its cost rolls up there (morning M1,
/// from S5's review). Best effort: a failure leaves the turn unlinked.
pub(crate) fn link_to_delegator(vault: &Path, w: &Wake, queue_id: i64, now: DateTime<Utc>) {
    let Some(asked) = crate::run_queue::id_for_run(vault, &w.run_id)
        .ok()
        .flatten()
    else {
        return;
    };
    let Some((parent_run, json)) = crate::session_store::delegation_for_child_turn(vault, asked)
        .ok()
        .flatten()
    else {
        return;
    };
    let Ok(asking) = serde_json::from_str::<crate::run_manifest::DelegationRecord>(&json) else {
        return;
    };
    // The same conversation only: a handover's record names its successor.
    if asking.child_session.as_deref() != Some(w.session_key.as_str()) {
        return;
    }
    let record = crate::run_manifest::DelegationRecord {
        kind: asking.kind,
        tool: WAKE_TOOL.into(),
        agent: asking.agent,
        child_session: asking.child_session,
        child_queue_id: Some(queue_id),
        at: crate::session_store::db_time(now),
        ..Default::default()
    };
    crate::delegation::store(vault, &parent_run, &record);
}

/// The history runs of the turns wakes queued in the last two days, and of
/// the child tasks they started, down their chains (the overnight scope
/// counts them: `budgets::unattended_spend`, morning M1). A turn still
/// queued has no run yet; its cost counts once it runs. One query per level.
pub(crate) fn woken_runs(vault: &Path) -> std::collections::HashSet<String> {
    let since = Utc::now() - Duration::hours(48);
    let ids: Vec<i64> = load(vault)
        .into_iter()
        .filter(|w| w.status == "queued")
        .filter(|w| parse(&w.created_at).is_some_and(|t| t >= since))
        .filter_map(|w| w.queue_id)
        .collect();
    let mut runs = std::collections::HashSet::new();
    let mut frontier = crate::run_queue::history_runs(vault, &ids).unwrap_or_default();
    for _ in 0..WOKEN_CHAIN_LEVELS {
        frontier.retain(|run| runs.insert(run.clone()));
        if frontier.is_empty() {
            break;
        }
        let children: Vec<i64> =
            crate::session_store::child_delegations_for_runs(vault, &frontier, since, 512)
                .unwrap_or_default()
                .into_iter()
                .filter_map(|(_, json)| {
                    serde_json::from_str::<crate::run_manifest::DelegationRecord>(&json).ok()
                })
                .filter_map(|record| record.child_queue_id)
                .collect();
        frontier = crate::run_queue::history_runs(vault, &children).unwrap_or_default();
    }
    runs
}

/// How far [`woken_runs`] follows child tasks (agent-started chains are at
/// most 2 deep; handovers add successors at the same depth).
const WOKEN_CHAIN_LEVELS: usize = 16;

/// Whether tonight's overnight stop is reached at `now`, as the night queue
/// weighs it (its own turns, automations, woken turns; a night turn's
/// unreported cost counts as reached). `Some(false)` outside the `tonight`
/// window; `None` when the spend couldn't be read.
fn night_stopped(vault: &Path, now: DateTime<Utc>) -> Option<bool> {
    let settings = crate::schedule_windows::settings();
    let Some(night) = settings
        .window("tonight")
        .and_then(|w| crate::schedule_windows::current(w, &chrono::Local, now))
    else {
        return Some(false);
    };
    crate::night_queue::budget_read(
        vault,
        &night.key,
        night.start,
        now,
        settings.overnight_cap_usd,
    )
    .ok()
    .map(|(spent, cap)| spent >= cap)
}

/// The delegation record's tool for a woken turn ([`link_to_delegator`]).
pub(crate) const WAKE_TOOL: &str = "wake";

/// Queue every due wake as a continue turn of its conversation (each minute,
/// from the Main agent's timer). A conversation still busy waits; a wake
/// [`LATE_MINUTES`] late is dropped.
pub(crate) async fn run_due(app: &crate::core_host::Core, config: &crate::config::Config) {
    let vault = config.vault_path.clone();
    let now = Utc::now();
    // Tonight's stop: read only when something is due, off the async
    // workers and before the lock (a person's send takes it to cancel).
    let night = if load(&vault).iter().any(|w| due(w, now) == Due::Now) {
        let v = vault.clone();
        tauri::async_runtime::spawn_blocking(move || night_stopped(&v, now))
            .await
            .unwrap_or(None)
    } else {
        Some(false)
    };
    for w in claim_due(&vault, now, night) {
        // A lead's batch report goes to its chain's current conversation.
        let target = if w.batch {
            crate::chat_chain::handover_tail(&vault, &w.session_key)
        } else {
            w.session_key.clone()
        };
        let outcome = match crate::session_store::read_session(&vault, &target) {
            // Moved into a build workspace since it was asked for: dropped,
            // never run on a person's branch.
            Ok(detail)
                if dropped_at_fire_of(&vault, &target, Path::new(&detail.summary.cwd), w.batch)
                    .is_some() =>
            {
                Ok(None)
            }
            Ok(detail) => {
                // The conversation's current mode, never above the asker's.
                let mode = woken_mode(&vault, &w);
                let (prompt, reported) = prompt_for(&vault, &w);
                // The children's ends it carries are recorded as reported
                // before the send, so the woken turn's own note (read as it
                // starts) doesn't repeat them; a send that fails or never
                // settles forgets them again ([`undeliver`]).
                deliver(&vault, &target, &reported);
                let input = crate::commands::ChatSendInput {
                    prompt,
                    session_id: detail.summary.harness_session_id.clone(),
                    session_key: Some(target.clone()),
                    cwd: Some(detail.summary.cwd.clone()),
                    model: wake_model(&vault, &target, &detail.summary.agent),
                    permission_mode: Some(mode),
                    agent: Some(detail.summary.agent.clone()),
                    client_turn_id: None,
                    app_depth: w.depth,
                    ultracode: false,
                    canvases: None,
                    continued_from: Vec::new(),
                    unattended: true,
                    typed: false,
                    // Hollow's turn, not the person's or an agent's (OB20).
                    host_asker: Some(crate::main_agent::ASKED_BY_WAKE_ME),
                    canned: None,
                    worker: false,
                };
                crate::commands::send_chat_on(Some(app), config, input, None, None)
                    .await
                    .map(|a| Some(a.queue_id))
            }
            Err(e) => Err(e),
        };
        settle(&vault, &w, outcome);
    }
}

/// The model a woken turn asks for: the one the conversation's newest turn
/// ran with, when that turn ran the same agent, so the woken turn keeps the
/// person's current choice (morning M2, from C3's review). A turn that ran
/// the CLI's own default (the composer's "Default": no model recorded) gives
/// `Some("")`, which keeps it, not the saved model. Else `None`: the agent's
/// saved default. (Its native session resets anyway: an unattended turn's
/// mode and tools differ.) The Main agent's wakes and check-ins ask for it
/// too (`main_agent::thread_basis`).
pub(crate) fn wake_model(vault: &Path, key: &str, agent: &str) -> Option<String> {
    match crate::session_store::last_requested_model(vault, key) {
        Ok(Some((model, used))) if used.as_deref().is_none_or(|u| u == agent) => {
            Some(model.unwrap_or_default())
        }
        _ => None,
    }
}

/// Claim what is due at `now` under the lock (status `queueing`, saved), so
/// a cancel or a replacement can't race the send. `night` is whether
/// tonight's overnight stop is reached ([`night_stopped`]): past it a due
/// wake is dropped as missed (morning M1); when it couldn't be read, due
/// wakes wait for the next minute ([`LATE_MINUTES`] still bounds them).
fn claim_due(vault: &Path, now: DateTime<Utc>, night: Option<bool>) -> Vec<Wake> {
    claim_due_within(vault, now, night, |w| {
        w.batch && batch_budget_stopped(vault, w, now)
    })
}

/// [`claim_due`] with the budget check a lead's batch report gets
/// (`stopped`, L8): a due one it holds is dropped as missed, never sent.
fn claim_due_within(
    vault: &Path,
    now: DateTime<Utc>,
    night: Option<bool>,
    stopped: impl Fn(&Wake) -> bool,
) -> Vec<Wake> {
    // Budgets are read before the lock (a person's send takes it).
    let held: Vec<String> = load(vault)
        .iter()
        .filter(|w| w.batch && due(w, now) == Due::Now && stopped(w))
        .map(|w| w.id.clone())
        .collect();
    let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = load(vault);
    let mut claimed = Vec::new();
    let mut changed = false;
    let mut sending = SENDING.lock().unwrap_or_else(|e| e.into_inner());
    let mut recovered = Vec::new();
    let mut unsent = Vec::new();
    for w in all.iter_mut() {
        // Another Mac's wake (M3b): never fired here, and left as it is
        // until it is late past the grace that Mac had to drop it itself
        // (`runner_coo::FOREIGN_GRACE_MINUTES`); then dropped as missed, so
        // a Mac whose id changed doesn't keep a dead wake pending forever.
        if !crate::runner_coo::owned_here(w.owner.as_deref()) {
            let grace = Duration::minutes(crate::runner_coo::FOREIGN_GRACE_MINUTES);
            if due(w, now - grace) == Due::Missed {
                eprintln!("[wake_me] {}: missed, another Mac's and long past", w.id);
                w.status = "missed".into();
                changed = true;
            }
            continue;
        }
        if w.status == "queueing" && !sending.contains(&w.id) {
            let claimed_at = w.claimed_at.clone();
            if let Some(queue_id) = recover(vault, w) {
                recovered.push((w.clone(), queue_id));
            } else if w.status != "queueing" {
                // Never sent: what it carried is forgotten, outside the lock.
                unsent.push(Wake {
                    claimed_at,
                    ..w.clone()
                });
            }
            changed |= w.status != "queueing";
        }
        match due(w, now) {
            Due::Wait => {}
            Due::Missed => {
                w.status = "missed".into();
                changed = true;
            }
            Due::Now if night == Some(true) => {
                eprintln!("[wake_me] {}: missed, the overnight stop is reached", w.id);
                w.status = "missed".into();
                changed = true;
            }
            Due::Now if night.is_none() => {}
            Due::Now if held.contains(&w.id) => {
                eprintln!("[wake_me] {}: missed, a budget stop holds it", w.id);
                w.status = "missed".into();
                changed = true;
            }
            Due::Now => {
                let busy = crate::run_queue::newest_chat_row(vault, &w.session_key)
                    .ok()
                    .flatten()
                    .is_some_and(|row| row.status.is_live());
                if !busy {
                    w.status = "queueing".into();
                    w.claimed_at = Some(crate::session_store::db_time(now));
                    sending.push(w.id.clone());
                    claimed.push(w.clone());
                    changed = true;
                } else {
                    // Held by the turn (PER-12): it waits for the turn's end,
                    // never counted late meanwhile.
                    w.busy_at = Some(now.to_rfc3339());
                    changed = true;
                }
            }
        }
    }
    if changed {
        let _ = save(vault, &all);
    }
    drop(sending);
    drop(_one);
    for (w, queue_id) in recovered {
        link_to_delegator(vault, &w, queue_id, now);
    }
    for w in unsent {
        undeliver(vault, &w);
    }
    claimed
}

/// What became of wake `w`, left `queueing` by a send that never settled:
/// the woken turn it queued (`queued`; its row id is returned so it can be
/// linked), or, with none, `cancelled` when the conversation got another turn
/// since (the person wrote) and `pending` otherwise (sent again when due, or
/// missed when late). A read error leaves it for the next minute.
fn recover(vault: &Path, w: &mut Wake) -> Option<i64> {
    let since = w
        .claimed_at
        .clone()
        .or_else(|| parse(&w.at).map(crate::session_store::db_time))?;
    // A lead's batch report may have gone to any conversation of its chain.
    let mut rows = Vec::new();
    for key in wake_chain(vault, w) {
        rows.extend(crate::run_queue::chat_rows_since(vault, &key, &since).ok()?);
    }
    let ours = rows
        .iter()
        .find(|row| text_of(vault, row).is_some_and(|t| is_wake_text(&t, w)));
    let queue_id = match ours {
        Some(row) => {
            w.status = "queued".into();
            w.queue_id = Some(row.id);
            Some(row.id)
        }
        None => {
            // Hollow's own turns since (a check-in, a report) aren't the
            // person writing; a row whose text is gone counts as one
            // (unknown: never sent twice).
            let person = rows.iter().any(|row| {
                text_of(vault, row).is_none_or(|t| !t.trim_start().starts_with("⟲ Hollow:"))
            });
            // A batch report outlives the person's messages (L8).
            w.status = if person && !w.batch {
                "cancelled"
            } else {
                "pending"
            }
            .into();
            w.claimed_at = None;
            None
        }
    };
    eprintln!(
        "[wake_me] {}: a send that never settled, recovered as {}",
        w.id, w.status
    );
    queue_id
}

/// A chat row's message: its payload's prompt while live, its kept text, or
/// its turn's asked text once it ran (a finished row's payload is cleared).
fn text_of(vault: &Path, row: &crate::run_queue::QueueRow) -> Option<String> {
    row.payload
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| row.detail.clone())
        .or_else(|| {
            let run = row.history_run_id.as_deref()?;
            crate::session_store::asked_for_run(vault, run)
                .ok()
                .flatten()
                .map(|(asked, _)| asked)
        })
}

/// Whether `text` is wake `w`'s woken turn ([`wake_prompt`]).
fn is_wake_text(text: &str, w: &Wake) -> bool {
    // A routed send leads with its `<hollow-forwarded>` note.
    let text = crate::sessions::strip_injected_context(text);
    text.trim_start().starts_with(WAKE_OPEN) && text.contains(&w.reason)
}

/// How a woken turn's message opens.
pub(crate) const WAKE_OPEN: &str = "⟲ Hollow: the wake-up you asked for";

/// Record what sending claimed wake `w` came to: its queued turn (then
/// linked to the asking turn's delegation, outside the lock), cancelled
/// (`Ok(None)`: moved or handed over), or back to pending after a passing
/// failure (it retries next minute, until it is late; what it carried is
/// forgotten, [`undeliver`], and recorded again by the retry).
fn settle(vault: &Path, w: &Wake, outcome: Result<Option<i64>, String>) {
    if outcome.is_err() {
        undeliver(vault, w);
    }
    let queued = {
        let _one = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        SENDING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|id| id != &w.id);
        let mut latest = load(vault);
        let mut queued = None;
        if let Some(entry) = latest.iter_mut().find(|e| e.id == w.id) {
            match outcome {
                Ok(Some(queue_id)) => {
                    entry.status = "queued".into();
                    entry.queue_id = Some(queue_id);
                    queued = Some(queue_id);
                }
                Ok(None) => {
                    eprintln!(
                        "[wake_me] {}: cancelled when due (moved or handed over)",
                        w.id
                    );
                    entry.status = "cancelled".into();
                }
                Err(e) => {
                    eprintln!("[wake_me] {}: {e}", w.id);
                    entry.status = "pending".into();
                    entry.claimed_at = None;
                }
            }
        }
        let _ = save(vault, &latest);
        queued
    };
    if let Some(queue_id) = queued {
        link_to_delegator(vault, w, queue_id, Utc::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_actions::Caller;
    use crate::app_mcp::AppAccess;

    fn caller(scope: &str, run: &str) -> Caller {
        Caller {
            run_id: run.into(),
            surface: "chat".into(),
            scope: scope.into(),
            access: AppAccess::Write,
            depth: 1,
            perm: "bypassPermissions".into(),
        }
    }

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn a_wake_is_bounded_and_chat_only() {
        let now = t("2026-10-10T06:00:00Z");
        let me = caller("s-20261010-wake", "run-w1");
        let w = plan(&[], &me, 30, "  check the gate  ", now).unwrap();
        assert_eq!(w.at, t("2026-10-10T06:30:00Z").to_rfc3339());
        assert_eq!(w.reason, "check the gate");
        assert_eq!(w.mode, "bypassPermissions");
        assert_eq!(w.depth, 1);
        for bad in [0, 121, -5] {
            assert!(plan(&[], &me, bad, "x", now).is_err(), "{bad}");
        }
        assert!(plan(&[], &me, 5, "   ", now).is_err());
        let automation = Caller {
            surface: "workflow".into(),
            scope: "nightly".into(),
            ..me.clone()
        };
        assert!(plan(&[], &automation, 5, "x", now)
            .unwrap_err()
            .contains("Only a chat"));
        // The daily cap counts this conversation's wakes, cancelled ones aside.
        let mut many: Vec<Wake> = (0..MAX_PER_DAY).map(|_| w.clone()).collect();
        assert!(plan(&many, &me, 5, "x", now)
            .unwrap_err()
            .contains("the most Hollow allows"));
        many[0].status = "cancelled".into();
        assert!(plan(&many, &me, 5, "x", now).is_ok());
        let other = caller("s-20261010-other", "run-o");
        assert!(
            plan(&many, &other, 5, "x", now).is_ok(),
            "another conversation's are its own"
        );
    }

    #[test]
    fn due_waits_fires_and_drops_late_wakes_on_a_fake_clock() {
        let me = caller("s-20261010-wake", "run-w1");
        let w = plan(&[], &me, 10, "x", t("2026-10-10T06:00:00Z")).unwrap();
        assert_eq!(due(&w, t("2026-10-10T06:09:59Z")), Due::Wait);
        assert_eq!(due(&w, t("2026-10-10T06:10:00Z")), Due::Now);
        assert_eq!(due(&w, t("2026-10-10T06:39:00Z")), Due::Now);
        assert_eq!(due(&w, t("2026-10-10T06:41:00Z")), Due::Missed);
        let done = Wake {
            status: "queued".into(),
            ..w
        };
        assert_eq!(due(&done, t("2026-10-10T06:10:00Z")), Due::Wait);
    }

    #[test]
    fn schedule_replaces_cancel_cancels_and_the_receipt_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let me = caller("s-20261010-wake", "run-w1");
        let first = schedule(v, &me, 15, "first", Utc::now()).unwrap();
        assert!(receipt_line(v, "run-w1").unwrap().contains("first"));
        let me2 = caller("s-20261010-wake", "run-w2");
        let second = schedule(v, &me2, 20, "second", Utc::now()).unwrap();
        assert_eq!(
            second["replaced"], first["id"],
            "one pending per conversation"
        );
        assert_eq!(pending(v)["wakes"].as_array().unwrap().len(), 1);
        assert!(
            receipt_line(v, "run-w1").is_none(),
            "the replaced one is gone"
        );
        cancel(v, &me2, None).unwrap();
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        assert!(cancel(v, &me2, None).unwrap_err().contains("No pending"));
        assert!(receipt_line(v, "run-w2").is_none());
        // A conversation cancels its own wakes only; a stopped turn and the
        // person's next message cancel too.
        let me3 = caller("s-20261010-wake", "run-w3");
        let ours = schedule(v, &me3, 5, "third", Utc::now()).unwrap();
        let other = caller("s-20261010-other", "run-o");
        assert!(cancel(v, &other, ours["id"].as_str()).is_err());
        cancel_run(v, "run-w3");
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        schedule(
            v,
            &caller("s-20261010-wake", "run-w4"),
            5,
            "fourth",
            Utc::now(),
        )
        .unwrap();
        cancel_session(v, "s-20261010-wake");
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        // A handover cancels the handing-over chat's wake, and it can't ask
        // for another (overnight S5).
        let from = caller("s-20261010-handing", "run-h1");
        schedule(v, &from, 30, "come back", Utc::now()).unwrap();
        crate::app_sessions::record_handover(v, &from.scope, "s-20261010-heir", 1).unwrap();
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        let again = caller("s-20261010-handing", "run-h2");
        assert!(schedule(v, &again, 30, "again", Utc::now())
            .unwrap_err()
            .contains("handed its work to s-20261010-heir"));
        // The woken turn's message says what it's for.
        let w = plan(&[], &me, 5, "re-run the gate", Utc::now()).unwrap();
        assert!(wake_prompt(&w).contains("re-run the gate"));
    }

    /// Overnight OB10b: a wake on a child's end is due only once that child
    /// ends, queues the parent's turn once (a second chat end doesn't queue
    /// it again), carries the child's line, and reports it once.
    #[test]
    fn a_wake_on_a_childs_end_queues_the_parent_once_when_it_ends() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-ob10b-parent";
        let ord =
            crate::session_store::start_turn(v, parent, "claude", "/v", "R-ob10b-p", "go").unwrap();
        let me = caller(parent, "R-ob10b-p");
        let kid = child_turn(v, "s-ob10b-kid", "R-ob10b-kid", RowStatus::Running);
        record_child(v, "R-ob10b-p", "s-ob10b-kid", kid);
        let now = Utc::now();
        let asked = schedule_on_end(v, &me, &json!({ "keys": ["s-ob10b-kid"] }), "", now).unwrap();
        let note = asked["note"].as_str().unwrap();
        assert!(note.contains("when s-ob10b-kid ends"), "{note}");
        assert_eq!(asked["onSessionsEnd"]["mode"], json!("all"));
        let receipt = receipt_line(v, "R-ob10b-p").unwrap();
        assert!(receipt.contains("when s-ob10b-kid ends"), "{receipt}");
        // Not due while the child runs, however long the timer ticks.
        assert_eq!(arm_ended(v, now), 0);
        assert!(claim_due(v, now + Duration::hours(2), Some(false)).is_empty());
        assert_eq!(load(v)[0].status, "pending");
        crate::run_queue::finish(v, kid, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let claimed = claim_due(v, now, Some(false));
        assert_eq!(claimed.len(), 1);
        let (prompt, reported) = prompt_for(v, &claimed[0]);
        assert!(prompt.starts_with(WAKE_OPEN), "{prompt}");
        assert!(prompt.contains("- s-ob10b-kid (claude): done"), "{prompt}");
        assert!(
            prompt.contains("runs unattended (in this conversation's own mode"),
            "{prompt}"
        );
        assert!(
            is_wake_text(&prompt, &claimed[0]),
            "a crash mid-send recovers it"
        );
        assert_eq!(reported.children, vec![(kid, "s-ob10b-kid".to_string())]);
        assert!(reported.chain.is_empty());
        // `run_due` records it reported, sends it, then settles it queued.
        deliver(v, parent, &reported);
        let woken = child_turn(v, parent, "R-ob10b-woken", RowStatus::Running);
        settle(v, &claimed[0], Ok(Some(woken)));
        assert_eq!(load(v)[0].status, "queued");
        // Another chat's end (the hook runs on every one) queues nothing more.
        assert_eq!(arm_ended(v, now), 0);
        assert!(claim_due(v, now, Some(false)).is_empty());
        // And the parent's next note doesn't report the child again.
        let delivery = crate::session_store::child_delivery(v, kid)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.via, "wake");
        assert_eq!(
            crate::descendants::notes_for_turn(v, parent, ord + 1),
            crate::descendants::Notes::default()
        );
    }

    /// Overnight OB10b: `all` waits for every key, `any` fires on the first
    /// (naming the ones still running); a wake already met is refused.
    #[test]
    fn a_wake_on_all_waits_for_every_child_and_on_any_for_the_first() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let me = caller("s-ob10b-fan", "R-ob10b-fan");
        let a = child_turn(v, "s-ob10b-a", "R-ob10b-a", RowStatus::Running);
        let b = child_turn(v, "s-ob10b-b", "R-ob10b-b", RowStatus::Running);
        record_child(v, "R-ob10b-fan", "s-ob10b-a", a);
        record_child(v, "R-ob10b-fan", "s-ob10b-b", b);
        let now = Utc::now();
        let any = json!({ "keys": ["s-ob10b-a", "s-ob10b-b"], "mode": "any" });
        let first = schedule_on_end(v, &me, &any, "first lane back", now).unwrap();
        assert!(first["note"]
            .as_str()
            .unwrap()
            .contains("when the first of s-ob10b-a, s-ob10b-b ends"));
        crate::run_queue::finish(v, a, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1, "any: the first to end");
        let fired = load(v).into_iter().find(|w| w.status == "pending").unwrap();
        let (prompt, _) = prompt_for(v, &fired);
        assert!(prompt.contains("- s-ob10b-a (claude): done"), "{prompt}");
        assert!(prompt.contains("Still running: s-ob10b-b."), "{prompt}");
        // Already met: nothing to wait for.
        let met = json!({ "keys": ["s-ob10b-a"], "mode": "any" });
        assert!(schedule_on_end(v, &me, &met, "", now)
            .unwrap_err()
            .contains("s-ob10b-a already ended"));
        // `all` (the default), keys trimmed and named once: replaces it.
        let both = json!({ "keys": ["s-ob10b-a", " s-ob10b-b ", "s-ob10b-a"] });
        let all = schedule_on_end(v, &me, &both, "both lanes", now).unwrap();
        assert_eq!(all["replaced"], first["id"]);
        assert_eq!(all["onSessionsEnd"]["watched"].as_array().unwrap().len(), 2);
        assert_eq!(arm_ended(v, now), 0, "all: s-ob10b-b still runs");
        crate::run_queue::finish(v, b, RowStatus::Failed, Some("boom"), now).unwrap();
        assert_eq!(arm_ended(v, now), 1, "all: every one ended");
        let fired = load(v).into_iter().find(|w| w.status == "pending").unwrap();
        let (prompt, reported) = prompt_for(v, &fired);
        assert!(prompt.contains("- s-ob10b-a (claude): done"), "{prompt}");
        assert!(prompt.contains("- s-ob10b-b (claude): failed"), "{prompt}");
        assert!(!prompt.contains("Still running"), "{prompt}");
        assert_eq!(reported.children.len(), 2);
        let bad = json!({ "keys": ["s-ob10b-b"], "mode": "some" });
        assert!(schedule_on_end(v, &me, &bad, "", now)
            .unwrap_err()
            .contains("all or any"));
    }

    /// Overnight OB10b: a lane that hands over wakes its parent when its
    /// last successor ends, not at the handover.
    #[test]
    fn a_wake_on_a_lane_that_hands_over_waits_for_its_last_successor() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let me = caller("s-ob10b-chain", "R-ob10b-chain");
        let lane = child_turn(v, "s-ob10b-lane", "R-ob10b-lane", RowStatus::Running);
        record_child(v, "R-ob10b-chain", "s-ob10b-lane", lane);
        let now = Utc::now();
        let keys = json!({ "keys": ["s-ob10b-lane"] });
        schedule_on_end(v, &me, &keys, "the lane", now).unwrap();
        let heir = child_turn(v, "s-ob10b-heir", "R-ob10b-heir", RowStatus::Running);
        let handover = crate::run_manifest::DelegationRecord {
            kind: "childTask".into(),
            tool: crate::app_sessions::HANDOVER_TOOL.into(),
            agent: "claude".into(),
            child_session: Some("s-ob10b-heir".into()),
            child_queue_id: Some(heir),
            at: crate::session_store::db_time(now),
            ..Default::default()
        };
        crate::delegation::store(v, "R-ob10b-lane", &handover);
        crate::run_queue::finish(v, lane, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 0, "its successor still runs");
        crate::run_queue::finish(v, heir, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let fired = load(v).into_iter().find(|w| w.status == "pending").unwrap();
        let (prompt, reported) = prompt_for(v, &fired);
        assert!(
            prompt.contains("- s-ob10b-heir (claude): done")
                && prompt.contains("(where s-ob10b-lane's handovers ended)"),
            "{prompt}"
        );
        assert_eq!(reported.children, vec![(lane, "s-ob10b-lane".to_string())]);
        assert_eq!(reported.chain, vec![heir]);
    }

    /// Overnight OB10b: the person's next message in the parent cancels a
    /// wake on children's ends, before or after they ended.
    #[test]
    fn a_person_message_cancels_a_wake_on_children_ends() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-ob10b-person";
        let me = caller(parent, "R-ob10b-person");
        let kid = child_turn(v, "s-ob10b-k1", "R-ob10b-k1", RowStatus::Running);
        let kid2 = child_turn(v, "s-ob10b-k2", "R-ob10b-k2", RowStatus::Running);
        record_child(v, "R-ob10b-person", "s-ob10b-k1", kid);
        record_child(v, "R-ob10b-person", "s-ob10b-k2", kid2);
        let now = Utc::now();
        schedule_on_end(v, &me, &json!({ "keys": ["s-ob10b-k1"] }), "", now).unwrap();
        // What `commands::send_chat` does for the person's own message.
        cancel_session(v, parent);
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        crate::run_queue::finish(v, kid, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 0);
        assert!(claim_due(v, now, Some(false)).is_empty());
        // Fired but not yet sent (the parent was busy): cancelled too.
        schedule_on_end(v, &me, &json!({ "keys": ["s-ob10b-k2"] }), "", now).unwrap();
        crate::run_queue::finish(v, kid2, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        cancel_session(v, parent);
        assert_eq!(arm_ended(v, now), 0);
        assert!(claim_due(v, now, Some(false)).is_empty());
    }

    /// Overnight OB10b (delta review): a woken turn's children are recorded
    /// as reported before its send. A send that fails forgets them (the
    /// retry records them again), and so does one that never settled, so
    /// when the wake then never runs (here the person wrote) the parent's
    /// next note reports them.
    #[test]
    fn an_unsent_wake_leaves_its_children_to_the_next_note() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-ob10b-unsent";
        let ord =
            crate::session_store::start_turn(v, parent, "claude", "/v", "R-ob10b-u", "go").unwrap();
        let me = caller(parent, "R-ob10b-u");
        let kid = child_turn(v, "s-ob10b-uk", "R-ob10b-uk", RowStatus::Running);
        record_child(v, "R-ob10b-u", "s-ob10b-uk", kid);
        let now = Utc::now();
        schedule_on_end(v, &me, &json!({ "keys": ["s-ob10b-uk"] }), "", now).unwrap();
        crate::run_queue::finish(v, kid, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let delivered = || crate::session_store::child_delivery(v, kid).unwrap();
        // `run_due`: claimed, recorded as reported, then the send fails.
        let claimed = claim_due(v, now, Some(false));
        assert_eq!(claimed.len(), 1);
        let (_, reported) = prompt_for(v, &claimed[0]);
        deliver(v, parent, &reported);
        assert!(delivered().is_some());
        settle(v, &claimed[0], Err("busy".into()));
        assert_eq!(load(v)[0].status, "pending");
        assert_eq!(delivered(), None, "a failed send forgets it");
        // The retry records it again, then never settles (a crash), and the
        // person writes before it is recovered: cancelled, and forgotten.
        let claimed = claim_due(v, now, Some(false));
        assert_eq!(claimed.len(), 1);
        let (_, reported) = prompt_for(v, &claimed[0]);
        deliver(v, parent, &reported);
        assert!(delivered().is_some());
        SENDING.lock().unwrap().retain(|id| id != &claimed[0].id);
        let person = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some(parent),
            "chat",
            "t",
        )
        .with_payload(json!({ "prompt": "Actually, stop that" }));
        crate::run_queue::enqueue(v, person, now + Duration::seconds(1)).unwrap();
        assert!(claim_due(v, now + Duration::seconds(1), Some(false)).is_empty());
        assert_eq!(load(v)[0].status, "cancelled");
        assert_eq!(delivered(), None, "a send that never settled forgets it");
        // So the parent's next note reports the child.
        let notes = crate::descendants::notes_for_turn(v, parent, ord + 1);
        assert_eq!(notes.children, 1, "{}", notes.text);
        assert!(notes.text.contains("s-ob10b-uk"), "{}", notes.text);
    }

    /// Overnight OB10b review: a child that ends without a `finished` event
    /// (cancelled while queued, interrupted by startup recovery) never runs
    /// the chat-end hook; the minute timer's [`arm_ended`] marks the wake
    /// due all the same.
    #[test]
    fn a_wake_on_children_cancelled_or_interrupted_is_armed_by_the_timer() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let me = caller("s-ob10b-quiet", "R-ob10b-quiet");
        let queued = child_turn(v, "s-ob10b-q", "R-ob10b-q", RowStatus::Queued);
        let running = child_turn(v, "s-ob10b-r", "R-ob10b-r", RowStatus::Running);
        record_child(v, "R-ob10b-quiet", "s-ob10b-q", queued);
        record_child(v, "R-ob10b-quiet", "s-ob10b-r", running);
        let now = Utc::now();
        let keys = json!({ "keys": ["s-ob10b-q", "s-ob10b-r"] });
        schedule_on_end(v, &me, &keys, "", now).unwrap();
        // Cancelled while queued: `emit_row` says `cancelled`, not `finished`.
        crate::run_queue::cancel(v, queued, now).unwrap();
        assert_eq!(arm_ended(v, now), 0, "all: s-ob10b-r still runs");
        // Interrupted, as startup recovery marks it: no event at all.
        crate::run_queue::finish(v, running, RowStatus::Interrupted, None, now).unwrap();
        // What the minute timer runs before `run_due`.
        assert_eq!(arm_ended(v, now), 1);
        let claimed = claim_due(v, now, Some(false));
        assert_eq!(claimed.len(), 1);
        let (prompt, reported) = prompt_for(v, &claimed[0]);
        assert!(
            prompt.contains("- s-ob10b-q (claude): cancelled"),
            "{prompt}"
        );
        assert!(
            prompt.contains("- s-ob10b-r (claude): interrupted"),
            "{prompt}"
        );
        assert_eq!(reported.children.len(), 2);
    }

    /// Overnight OB10b: only the caller's own children; never from an
    /// automation or a Main agent thread (woken for its chats already);
    /// minutes or onSessionsEnd, not both.
    #[test]
    fn a_wake_on_a_conversation_that_isnt_the_callers_child_is_refused() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let me = caller("s-ob10b-own", "R-ob10b-own");
        let mine = child_turn(v, "s-ob10b-mine", "R-ob10b-mine", RowStatus::Running);
        record_child(v, "R-ob10b-own", "s-ob10b-mine", mine);
        let theirs = child_turn(v, "s-ob10b-theirs", "R-ob10b-theirs", RowStatus::Running);
        record_child(v, "R-ob10b-other", "s-ob10b-theirs", theirs);
        let now = Utc::now();
        for key in ["s-ob10b-theirs", "s-ob10b-own", "s-ob10b-nobody"] {
            let asked = json!({ "keys": ["s-ob10b-mine", key] });
            let refused = schedule_on_end(v, &me, &asked, "", now).unwrap_err();
            assert!(
                refused.contains(&format!("{key} isn't a conversation you started")),
                "{refused}"
            );
        }
        assert!(pending(v)["wakes"].as_array().unwrap().is_empty());
        let automation = Caller {
            surface: "workflow".into(),
            scope: "nightly".into(),
            ..me.clone()
        };
        let keys = json!({ "keys": ["s-ob10b-mine"] });
        assert!(schedule_on_end(v, &automation, &keys, "", now)
            .unwrap_err()
            .contains("Only a chat"));
        assert!(ask(v, &me, Some(5), Some(&keys), "x", now)
            .unwrap_err()
            .contains("not both"));
        assert!(ask(v, &me, None, Some(&json!({ "keys": [] })), "", now).is_err());
        // Neither: the refusal names both ways to say when.
        let neither = ask(v, &me, None, None, "x", now).unwrap_err();
        assert!(
            neither.contains("minutes") && neither.contains("onSessionsEnd"),
            "{neither}"
        );
        // A Main agent thread is woken by its own mechanism.
        crate::workspaces::ensure_main_agent(v).unwrap();
        let main = crate::workspaces::main_agent_cwd(v).unwrap();
        crate::session_store::start_turn(v, "s-ob10b-main", "claude", &main, "R-ob10b-m", "go")
            .unwrap();
        let lane = child_turn(v, "s-ob10b-mlane", "R-ob10b-ml", RowStatus::Running);
        record_child(v, "R-ob10b-m", "s-ob10b-mlane", lane);
        let thread = caller("s-ob10b-main", "R-ob10b-m");
        assert!(
            schedule_on_end(v, &thread, &json!({ "keys": ["s-ob10b-mlane"] }), "", now)
                .unwrap_err()
                .contains("COO is already woken")
        );
        // The caller's own child is accepted.
        assert!(schedule_on_end(v, &me, &keys, "", now).is_ok());
    }

    /// Overnight OB10b, changed 2026-11-12: a woken parent runs in the
    /// asking turn's mode, bypass and auto kept (before, both fell to
    /// acceptEdits), and never above it or the conversation's current mode
    /// (plan stays plan).
    #[test]
    fn a_woken_parent_keeps_the_asking_mode() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let bypass = caller("s-ob10b-mode", "R-ob10b-mode");
        let watched = vec![Watched {
            key: "s-ob10b-x".into(),
            agent: "claude".into(),
            queue_id: 1,
            direct: true,
        }];
        let w = plan_on_end(&[], &bypass, watched.clone(), "all", "", now).unwrap();
        assert_eq!(w.mode, "bypassPermissions");
        assert_eq!(w.reason, END_REASON);
        assert_eq!(woken_mode(v, &w), "bypassPermissions");
        let auto = Caller {
            perm: "auto".into(),
            ..bypass.clone()
        };
        let w = plan_on_end(&[], &auto, watched.clone(), "any", "x", now).unwrap();
        assert_eq!(woken_mode(v, &w), "auto");
        let default = Caller {
            perm: "default".into(),
            ..bypass.clone()
        };
        let w = plan_on_end(&[], &default, watched.clone(), "any", "x", now).unwrap();
        assert_eq!(woken_mode(v, &w), "default", "never above the asking mode");
        // A timed wake keeps its mode the same way.
        let timed = plan(&[], &bypass, 5, "x", now).unwrap();
        assert_eq!(woken_mode(v, &timed), "bypassPermissions");
        // Waiting for children: never due on its own; missed past its wait.
        let w = plan_on_end(&[], &bypass, Vec::new(), "all", "", now).unwrap();
        assert_eq!(
            due(&w, now + Duration::hours(SESSIONS_END_HOURS)),
            Due::Wait
        );
        assert_eq!(
            due(&w, now + Duration::hours(SESSIONS_END_HOURS + 1)),
            Due::Missed
        );
        // Lowered since it asked: the conversation's last mode (plan) wins.
        let ord = crate::session_store::start_turn(v, "s-ob10b-mode", "claude", "/v", "R-m2", "go")
            .unwrap();
        crate::session_store::record_turn_permission(v, "s-ob10b-mode", ord, "plan").unwrap();
        let w = plan_on_end(&[], &bypass, watched.clone(), "all", "x", now).unwrap();
        assert_eq!(woken_mode(v, &w), "plan", "plan stays plan");
        // Raised since it asked: never above the asking turn's mode.
        let ord = crate::session_store::start_turn(v, "s-ob10b-mode", "claude", "/v", "R-m3", "go")
            .unwrap();
        crate::session_store::record_turn_permission(v, "s-ob10b-mode", ord, "bypassPermissions")
            .unwrap();
        let w = plan_on_end(&[], &default, watched.clone(), "all", "x", now).unwrap();
        assert_eq!(woken_mode(v, &w), "default", "a weaker asker stays weaker");
        let w = plan_on_end(&[], &bypass, watched, "all", "x", now).unwrap();
        assert_eq!(woken_mode(v, &w), "bypassPermissions", "bypass kept");
    }

    /// Morning M1: a woken turn of a durable child task joins its delegation,
    /// so the delegating parent hears once that it ended; a wake nobody
    /// delegated links nothing.
    #[test]
    fn a_woken_child_turn_reaches_the_delegating_parent_once() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::descendants::{notes_for_turn, Notes};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-m1-parent";
        let ord = crate::session_store::start_turn(v, parent, "claude", "/v", "R-m1-parent", "go")
            .unwrap();
        let asked = child_turn(v, "s-m1-child", "R-m1-child", RowStatus::Done);
        record_child(v, "R-m1-parent", "s-m1-child", asked);
        assert_eq!(notes_for_turn(v, parent, ord + 1).children, 1);
        // The child's turn asked to be woken; Hollow queued the woken turn.
        let now = Utc::now();
        let w = plan(&[], &caller("s-m1-child", "R-m1-child"), 30, "check", now).unwrap();
        let woken = child_turn(v, "s-m1-child", "R-m1-woken", RowStatus::Running);
        // `run_due` settles a sent wake: queued, then linked.
        let mut sent = w.clone();
        sent.status = "queueing".into();
        save(v, &[sent]).unwrap();
        settle(v, &w, Ok(Some(woken)));
        assert_eq!(load(v)[0].status, "queued");
        assert_eq!(load(v)[0].queue_id, Some(woken));
        let linked = crate::delegation::for_run(v, "R-m1-parent");
        assert_eq!(linked.len(), 2);
        assert_eq!(linked[1].tool, WAKE_TOOL);
        assert_eq!(linked[1].child_queue_id, Some(woken));
        assert_eq!(linked[1].packet_bytes, 0, "no packet was sent");
        assert_eq!(
            notes_for_turn(v, parent, ord + 2),
            Notes::default(),
            "not while it runs"
        );
        crate::run_queue::finish(v, woken, RowStatus::Done, None, now).unwrap();
        let later = notes_for_turn(v, parent, ord + 3);
        assert_eq!(later.children, 1);
        assert!(
            later.text.contains("- s-m1-child (claude): done"),
            "{}",
            later.text
        );
        assert_eq!(notes_for_turn(v, parent, ord + 4), Notes::default(), "once");
        // A person's own chat (no delegation) links nothing, and neither does
        // a record naming another conversation.
        child_turn(v, "s-m1-own", "R-m1-own", RowStatus::Done);
        let own = plan(&[], &caller("s-m1-own", "R-m1-own"), 5, "x", now).unwrap();
        let own_woken = child_turn(v, "s-m1-own", "R-m1-own-woken", RowStatus::Done);
        link_to_delegator(v, &own, own_woken, now);
        assert!(
            crate::session_store::delegation_for_child_turn(v, own_woken)
                .unwrap()
                .is_none()
        );
        let odd = child_turn(v, "s-m1-odd", "R-m1-odd", RowStatus::Done);
        record_child(v, "R-m1-other-parent", "s-m1-elsewhere", odd);
        let w = plan(&[], &caller("s-m1-odd", "R-m1-odd"), 5, "x", now).unwrap();
        let odd_woken = child_turn(v, "s-m1-odd", "R-m1-odd-woken", RowStatus::Done);
        link_to_delegator(v, &w, odd_woken, now);
        assert_eq!(crate::delegation::for_run(v, "R-m1-other-parent").len(), 1);
    }

    /// Morning M1: a woken turn is unattended, so its cost counts in the
    /// overnight scope (a person's own chat still doesn't).
    #[test]
    fn a_woken_turn_counts_in_the_overnight_scope() {
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let row = crate::descendants::tests::child_turn(
            v,
            "s-m1-night",
            "R-m1-night-woken",
            RowStatus::Done,
        );
        // A child task the woken turn started counts too.
        let kid = crate::descendants::tests::child_turn(
            v,
            "s-m1-night-kid",
            "R-m1-night-kid",
            RowStatus::Done,
        );
        crate::descendants::tests::record_child(v, "R-m1-night-woken", "s-m1-night-kid", kid);
        for (run, cost) in [
            ("R-m1-night-woken", 7.0),
            ("R-m1-night-kid", 2.0),
            ("R-m1-person", 30.0),
        ] {
            let at = crate::session_store::db_time(now);
            let mut r = crate::session_store::tests::usage_row_fixture(run, &at, "success");
            r.run_id = Some(run.into());
            r.cost_usd = Some(cost);
            crate::session_store::upsert_usage(v, &r).unwrap();
        }
        let spend = || {
            crate::budgets::unattended_spend(v, now - Duration::hours(1), now + Duration::hours(1))
                .unwrap()
                .0
        };
        assert_eq!(spend(), 0.0, "chat rows aren't unattended");
        let mut w = plan(&[], &caller("s-m1-night", "R-m1-night"), 5, "x", now).unwrap();
        w.status = "queued".into();
        w.queue_id = Some(row);
        save(v, &[w]).unwrap();
        assert_eq!(
            spend(),
            9.0,
            "the woken turn and its child count; the person's doesn't"
        );
        // The night queue weighs it as well, and a read is an answer.
        let read = crate::night_queue::budget_read(v, "m1", now - Duration::hours(1), now, 40.0);
        assert_eq!(read.unwrap(), (9.0, 40.0));
        let (spent, cap) =
            crate::night_queue::budget_read(v, "m1", now - Duration::hours(1), now, 5.0).unwrap();
        assert!(spent >= cap, "past a $5 cap");
    }

    /// Morning M1 (review): past tonight's stop a due wake is dropped as
    /// missed; when the spend couldn't be read it waits; otherwise it is
    /// claimed. A wake not yet due is never touched.
    #[test]
    fn claim_due_drops_past_the_stop_waits_when_unread_and_claims_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let me = caller("s-m1-claim", "R-m1-claim");
        let due_now = plan(&[], &me, 5, "x", now - Duration::minutes(6)).unwrap();
        let later = Wake {
            id: "wake-later".into(),
            session_key: "s-m1-later".into(),
            ..plan(&[], &me, 30, "y", now).unwrap()
        };
        let fresh = || save(v, &[due_now.clone(), later.clone()]).unwrap();
        let status = |id: &str| load(v).into_iter().find(|w| w.id == id).unwrap().status;
        fresh();
        assert!(claim_due(v, now, None).is_empty());
        assert_eq!(status(&due_now.id), "pending", "unread: it waits");
        assert!(claim_due(v, now, Some(true)).is_empty());
        assert_eq!(status(&due_now.id), "missed", "past the stop");
        fresh();
        let claimed = claim_due(v, now, Some(false));
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, due_now.id);
        assert_eq!(status(&due_now.id), "queueing");
        assert_eq!(status("wake-later"), "pending");
        // A failed send goes back to pending; a moved chat's is cancelled.
        settle(v, &due_now, Err("busy".into()));
        assert_eq!(status(&due_now.id), "pending");
        settle(v, &due_now, Ok(None));
        assert_eq!(status(&due_now.id), "cancelled");
    }

    /// M3b review: a wake carries the Mac it was asked on, and only that Mac
    /// fires it. Another Mac's due wake (a wake file copied with an app data
    /// folder) is left pending and untouched; an older wake with no owner
    /// fires anywhere.
    #[test]
    fn only_the_mac_a_wake_was_asked_on_fires_it() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let me = caller("s-m3b-own", "R-m3b-own");
        let ours = plan(&[], &me, 5, "x", now - Duration::minutes(6)).unwrap();
        assert_eq!(ours.owner, crate::runner_coo::machine_owner());
        assert!(ours.owner.is_some(), "this Mac has an id");
        let theirs = Wake {
            id: "wake-theirs".into(),
            session_key: "s-m3b-theirs".into(),
            owner: Some("m-0000000000000000".into()),
            ..ours.clone()
        };
        let legacy = Wake {
            id: "wake-legacy".into(),
            session_key: "s-m3b-legacy".into(),
            owner: None,
            ..ours.clone()
        };
        save(v, &[ours.clone(), theirs.clone(), legacy.clone()]).unwrap();
        let mut claimed: Vec<String> = claim_due(v, now, Some(false))
            .into_iter()
            .map(|w| w.id)
            .collect();
        claimed.sort();
        let mut want = vec![ours.id.clone(), legacy.id.clone()];
        want.sort();
        assert_eq!(claimed, want);
        let left = load(v).into_iter().find(|w| w.id == "wake-theirs").unwrap();
        assert_eq!(left, theirs, "another Mac's wake is left as it was");
        // Late, but within the grace that Mac had: still left as it was.
        let status = |id: &str| load(v).into_iter().find(|w| w.id == id).unwrap().status;
        let within = now + Duration::minutes(LATE_MINUTES + 50);
        assert!(claim_due(v, within, Some(false)).is_empty());
        assert_eq!(
            load(v).into_iter().find(|w| w.id == "wake-theirs").unwrap(),
            theirs
        );
        // Long past it (a Mac whose id changed): dropped as missed, never sent.
        assert!(claim_due(v, now + Duration::hours(3), Some(false)).is_empty());
        assert_eq!(status("wake-theirs"), "missed");
        // `sessions.wakes` shows whose a wake is.
        save(v, std::slice::from_ref(&theirs)).unwrap();
        assert_eq!(pending(v)["wakes"][0]["owner"], "m-0000000000000000");
        // The rule itself.
        use crate::runner_coo::owned_by;
        assert!(owned_by(Some("m-a"), Some("m-a")));
        assert!(!owned_by(Some("m-a"), Some("m-b")));
        assert!(owned_by(None, Some("m-b")) && owned_by(Some("m-a"), None));
    }

    /// M3b review 2: another Mac's wake on children's ends is never armed
    /// here, even when children of the same keys ended in this DB; this
    /// Mac's own is.
    #[test]
    fn another_macs_wake_on_childrens_ends_is_never_armed_here() {
        use crate::descendants::tests::{child_turn, record_child};
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-m3b-arm";
        crate::session_store::start_turn(v, parent, "claude", "/v", "R-m3b-arm", "go").unwrap();
        let me = caller(parent, "R-m3b-arm");
        let kid = child_turn(v, "s-m3b-kid", "R-m3b-kid", RowStatus::Running);
        record_child(v, "R-m3b-arm", "s-m3b-kid", kid);
        let now = Utc::now();
        schedule_on_end(v, &me, &json!({ "keys": ["s-m3b-kid"] }), "", now).unwrap();
        let mut all = load(v);
        all[0].owner = Some("m-0000000000000000".into());
        let theirs = all[0].clone();
        save(v, &all).unwrap();
        crate::run_queue::finish(v, kid, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 0, "not this Mac's");
        assert_eq!(load(v)[0], theirs, "left as it was");
        // The same wake as this Mac's is armed.
        let mut all = load(v);
        all[0].owner = crate::runner_coo::machine_owner();
        save(v, &all).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        assert!(load(v)[0]
            .on_sessions_end
            .as_ref()
            .unwrap()
            .fired_at
            .is_some());
    }

    /// Morning M2: a wake left `queueing` by a send that never settled (a
    /// crash mid-send) is recovered when wakes are next claimed: its woken
    /// turn found means queued, the person's turn means cancelled, nothing
    /// means pending (sent again when due). A send in flight is left alone.
    #[test]
    fn a_send_that_never_settled_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let stuck = |id: &str, key: &str| Wake {
            id: id.into(),
            status: "queueing".into(),
            claimed_at: Some(crate::session_store::db_time(now - Duration::minutes(1))),
            ..plan(
                &[],
                &caller(key, "R-m2"),
                5,
                "look again",
                now - Duration::minutes(6),
            )
            .unwrap()
        };
        let row = |key: &str, prompt: &str| {
            let row = crate::run_queue::NewRow::new(
                crate::run_queue::RunSource::Chat,
                Some(key),
                "chat",
                "t",
            )
            .with_payload(json!({ "prompt": prompt, "unattended": true }));
            crate::run_queue::enqueue(v, row, now).unwrap().id()
        };
        let sent = stuck("wake-m2-sent", "s-m2-sent");
        let ours = row("s-m2-sent", &wake_prompt(&sent));
        let wrote = stuck("wake-m2-wrote", "s-m2-wrote");
        row("s-m2-wrote", "Actually, stop that");
        // A woken turn that already ran: its payload and kept text are
        // cleared, so only its turn's asked text names it.
        let ran = stuck("wake-m2-ran", "s-m2-ran");
        let ran_row = crate::descendants::tests::child_turn(
            v,
            "s-m2-ran",
            "R-m2-ran",
            crate::run_queue::RowStatus::Done,
        );
        crate::session_store::start_turn(
            v,
            "s-m2-ran",
            "claude",
            "/v",
            "R-m2-ran",
            &wake_prompt(&ran),
        )
        .unwrap();
        // Hollow's own turn since isn't the person writing.
        let hollow = stuck("wake-m2-hollow", "s-m2-hollow");
        row("s-m2-hollow", "⟲ Hollow: check-in 10:00");
        let lost = stuck("wake-m2-lost", "s-m2-lost");
        let flying = stuck("wake-m2-flying", "s-m2-flying");
        SENDING.lock().unwrap().push("wake-m2-flying".to_string());
        save(v, &[sent, wrote, ran, hollow, lost, flying]).unwrap();
        let claimed = claim_due(v, now, Some(false));
        let by_id = |id: &str| load(v).into_iter().find(|w| w.id == id).unwrap();
        assert_eq!(by_id("wake-m2-sent").status, "queued");
        assert_eq!(by_id("wake-m2-sent").queue_id, Some(ours));
        assert_eq!(
            by_id("wake-m2-wrote").status,
            "cancelled",
            "the person wrote"
        );
        assert_eq!(
            claimed.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["wake-m2-lost"],
            "nothing was queued: sent again"
        );
        assert_eq!(
            by_id("wake-m2-hollow").status,
            "pending",
            "Hollow's own turn isn't the person's: it waits while that turn is live"
        );
        assert_eq!(by_id("wake-m2-ran").status, "queued");
        assert_eq!(by_id("wake-m2-ran").queue_id, Some(ran_row));
        assert_eq!(by_id("wake-m2-flying").status, "queueing", "in flight");
        for w in [by_id("wake-m2-lost"), by_id("wake-m2-flying")] {
            settle(v, &w, Err("test".into()));
        }
        let left = SENDING.lock().unwrap().clone();
        assert!(!left.iter().any(|id| id.starts_with("wake-m2-")), "settled");
    }

    /// Morning M2: a woken turn asks for the model the conversation's newest
    /// turn asked for (its agent's default when that turn asked for none, or
    /// ran another agent).
    #[test]
    fn a_woken_turn_keeps_the_conversations_model() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let key = "s-m2-model";
        assert_eq!(wake_model(v, key, "claude"), None, "no turn yet");
        let turn = |run: &str, agent: &str, model: Option<&str>| {
            crate::session_store::start_turn(v, key, agent, "/v", run, "go").unwrap();
            let at = crate::session_store::db_time(Utc::now());
            let mut r = crate::session_store::tests::usage_row_fixture(run, &at, "success");
            r.run_id = Some(run.into());
            r.agent = Some(agent.into());
            r.requested_model = model.map(str::to_string);
            crate::session_store::upsert_usage(v, &r).unwrap();
        };
        turn("R-m2-1", "claude", Some("claude-sonnet-5-5"));
        assert_eq!(
            wake_model(v, key, "claude").as_deref(),
            Some("claude-sonnet-5-5")
        );
        assert_eq!(wake_model(v, key, "codex"), None, "another agent's model");
        turn("R-m2-2", "claude", None);
        assert_eq!(
            wake_model(v, key, "claude").as_deref(),
            Some(""),
            "the newest ran the CLI default: kept, not the saved model"
        );
        turn("R-m2-3", "claude", Some("claude-haiku-5-5"));
        assert_eq!(
            wake_model(v, key, "claude").as_deref(),
            Some("claude-haiku-5-5")
        );
    }

    #[test]
    fn a_wake_is_refused_in_a_build_workspace_and_dropped_when_due_after_a_move_or_handover() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let tree = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(!in_build_workspace(v, tree.path()));
        // A merge or a chat move binds the folder after the wake was asked for.
        crate::build_workspaces::bind_for_test(v, "w-build", tree.path());
        assert!(in_build_workspace(v, tree.path()));
        assert!(!in_build_workspace(v, elsewhere.path()));
        // When due: dropped there, or after a handover; sent otherwise.
        let key = "s-20261010-fire";
        assert_eq!(
            dropped_at_fire_of(v, key, tree.path(), false),
            Some("now in a build workspace")
        );
        assert_eq!(dropped_at_fire_of(v, key, elsewhere.path(), false), None);
        crate::app_sessions::record_handover(v, key, "s-20261010-fire-heir", 1).unwrap();
        assert_eq!(
            dropped_at_fire_of(v, key, elsewhere.path(), false),
            Some("handed over since")
        );
    }

    /// L8: a lead's subagents join one batch report that fires once, when
    /// every one has ended and the lead is idle; it outlives the person's
    /// message and the lead's own wakes, and goes to a handover's successor.
    #[test]
    fn a_leads_batch_report_fires_once_when_every_subagent_has_ended() {
        use crate::descendants::tests::child_turn;
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-wake").unwrap();
        let me = caller(&lead, "R-l8-lead");
        let now = Utc::now();
        let a = child_turn(v, "s-20261110-l8wa", "R-l8-a", RowStatus::Running);
        let b = child_turn(v, "s-20261110-l8wb", "R-l8-b", RowStatus::Running);
        assert!(join_batch(v, &me, "s-20261110-l8wa", "claude", a, now));
        assert!(join_batch(v, &me, "s-20261110-l8wb", "codex", b, now));
        assert!(
            join_batch(v, &me, "s-20261110-l8wb", "codex", b, now),
            "named once"
        );
        let saved = load(v);
        assert_eq!(saved.len(), 1, "one batch: {saved:?}");
        let end = saved[0].on_sessions_end.as_ref().unwrap();
        assert!(saved[0].batch && end.on_failure);
        assert_eq!(end.watched.len(), 2);
        // Not a lead: no batch (an ordinary chat asks with wake_me itself).
        assert!(!join_batch(
            v,
            &caller("s-20261110-plain", "R-l8-p"),
            "s-20261110-l8wa",
            "claude",
            a,
            now
        ));
        // The person's message and an agent's own wake leave it.
        cancel_session(v, &lead);
        schedule(v, &me, 30, "look again", now).unwrap();
        let batch = || load(v).into_iter().find(|w| w.batch).unwrap();
        assert_eq!(batch().status, "pending");
        crate::run_queue::finish(v, a, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 0, "b still runs");
        crate::run_queue::finish(v, b, RowStatus::Done, None, now).unwrap();
        // The lead working (its own turn may start more) holds it.
        let busy = child_turn(v, &lead, "R-l8-busy", RowStatus::Running);
        assert_eq!(arm_ended(v, now), 0, "the lead is working");
        crate::run_queue::finish(v, busy, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let claimed: Vec<Wake> = claim_due_within(v, now, Some(false), |_| false)
            .into_iter()
            .filter(|w| w.batch)
            .collect();
        assert_eq!(claimed.len(), 1);
        let (prompt, reported) = prompt_for(v, &claimed[0]);
        assert!(prompt.contains(BATCH_REASON), "{prompt}");
        assert!(
            prompt.contains("- s-20261110-l8wa (claude): done"),
            "{prompt}"
        );
        assert!(
            prompt.contains("- s-20261110-l8wb (codex): done"),
            "{prompt}"
        );
        assert_eq!(reported.children.len(), 2);
        let woken = child_turn(v, &lead, "R-l8-woken", RowStatus::Done);
        settle(v, &claimed[0], Ok(Some(woken)));
        // Once: nothing more fires for this batch.
        assert_eq!(arm_ended(v, now), 0);
        assert!(claim_due_within(v, now, Some(false), |_| false)
            .iter()
            .all(|w| !w.batch));
        // A handover keeps a new batch (its report goes to the successor).
        let c = child_turn(v, "s-20261110-l8wc", "R-l8-c", RowStatus::Running);
        assert!(join_batch(v, &me, "s-20261110-l8wc", "claude", c, now));
        crate::app_sessions::record_handover(v, &lead, "s-20261110-l8lead2", 1).unwrap();
        assert_eq!(batch_pending(v).len(), 1);
        assert_eq!(dropped_at_fire_of(v, &lead, dir.path(), true), None);
    }

    fn batch_pending(v: &Path) -> Vec<Wake> {
        load(v)
            .into_iter()
            .filter(|w| w.batch && w.status == "pending")
            .collect()
    }

    /// L8: one failing subagent fires the batch report at once, and a
    /// budget stop drops a due one unsent.
    #[test]
    fn a_leads_batch_report_fires_on_a_failure_and_a_budget_stop_drops_it() {
        use crate::descendants::tests::child_turn;
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-fail").unwrap();
        let me = caller(&lead, "R-l8f-lead");
        let now = Utc::now();
        let a = child_turn(v, "s-20261110-l8fa", "R-l8f-a", RowStatus::Running);
        let b = child_turn(v, "s-20261110-l8fb", "R-l8f-b", RowStatus::Running);
        assert!(join_batch(v, &me, "s-20261110-l8fa", "claude", a, now));
        assert!(join_batch(v, &me, "s-20261110-l8fb", "claude", b, now));
        crate::run_queue::finish(v, a, RowStatus::Failed, Some("boom"), now).unwrap();
        assert_eq!(arm_ended(v, now), 1, "one failed: report now");
        let fired = batch_pending(v);
        let (prompt, _) = prompt_for(v, &fired[0]);
        assert!(
            prompt.contains("- s-20261110-l8fa (claude): failed"),
            "{prompt}"
        );
        assert!(
            prompt.contains("Still running: s-20261110-l8fb."),
            "{prompt}"
        );
        // A budget stop: missed, never claimed.
        assert!(claim_due_within(v, now, Some(false), |w| w.batch).is_empty());
        let after = load(v).into_iter().find(|w| w.batch).unwrap();
        assert_eq!(after.status, "missed");
        // A later start opens a new batch (the stopped one doesn't swallow it).
        let c = child_turn(v, "s-20261110-l8fc", "R-l8f-c", RowStatus::Running);
        assert!(join_batch(v, &me, "s-20261110-l8fc", "claude", c, now));
        assert_eq!(batch_pending(v).len(), 1);
    }

    /// L8 review: a lead's batch is keyed by its fixed conversation, so a
    /// start before and after a handover is one batch with one report, and
    /// a crash between the report's enqueue (to the successor) and its
    /// settle is recovered from the successor's rows, not sent again.
    #[test]
    fn a_leads_batch_spans_a_handover_and_recovers_from_the_successor() {
        use crate::descendants::tests::child_turn;
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-chain").unwrap();
        let b = "s-20261110-l8chainb";
        let now = Utc::now();
        let one = child_turn(v, "s-20261110-l8ch1", "R-l8ch-1", RowStatus::Running);
        assert!(join_batch(
            v,
            &caller(&lead, "R-l8ch-a"),
            "s-20261110-l8ch1",
            "claude",
            one,
            now
        ));
        crate::app_sessions::record_handover(v, &lead, b, 1).unwrap();
        let two = child_turn(v, "s-20261110-l8ch2", "R-l8ch-2", RowStatus::Running);
        assert!(join_batch(
            v,
            &caller(b, "R-l8ch-b"),
            "s-20261110-l8ch2",
            "claude",
            two,
            now
        ));
        let batches: Vec<Wake> = load(v).into_iter().filter(|w| w.batch).collect();
        assert_eq!(
            batches.len(),
            1,
            "one batch across the handover: {batches:?}"
        );
        assert_eq!(
            batches[0].session_key, lead,
            "keyed by the lead's fixed conversation"
        );
        assert_eq!(
            batches[0].on_sessions_end.as_ref().unwrap().watched.len(),
            2
        );
        // Both end: armed once; the report was queued to B, then a crash.
        crate::run_queue::finish(v, one, RowStatus::Done, None, now).unwrap();
        crate::run_queue::finish(v, two, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let claimed: Vec<Wake> = claim_due_within(v, now, Some(false), |_| false)
            .into_iter()
            .filter(|w| w.batch)
            .collect();
        assert_eq!(claimed.len(), 1);
        let (prompt, _) = prompt_for(v, &claimed[0]);
        let routed = crate::chat_chain::Routed {
            from: Some(lead.clone()),
            to: b.into(),
            to_title: None,
            session_id: None,
            cwd: "/v".into(),
            agent: "claude".into(),
        };
        let text = crate::chat_chain::with_forward_block(
            &prompt,
            &routed,
            crate::chat_chain::Sender::Hollow,
        );
        let row =
            crate::run_queue::NewRow::new(crate::run_queue::RunSource::Chat, Some(b), "chat", "t")
                .with_payload(json!({ "prompt": text, "unattended": true }));
        let report = crate::run_queue::enqueue(v, row, now).unwrap().id();
        SENDING.lock().unwrap().retain(|id| id != &claimed[0].id);
        assert!(
            claim_due_within(v, now, Some(false), |_| false)
                .iter()
                .all(|w| !w.batch),
            "never sent twice"
        );
        let after = load(v).into_iter().find(|w| w.batch).unwrap();
        assert_eq!(
            (after.status.as_str(), after.queue_id),
            ("queued", Some(report))
        );
    }

    /// L8 review: a subagent whose batch registration was lost (a crash just
    /// after its start was queued) is adopted into its lead's batch.
    #[test]
    fn a_leads_subagent_with_no_batch_record_is_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-orphan").unwrap();
        let now = Utc::now();
        let row = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some("s-20261110-l8orph"),
            "chat",
            "t",
        )
        .with_payload(json!({ "parent": lead, "agent": "codex", "prompt": "do it" }));
        let id = crate::run_queue::enqueue(v, row, now).unwrap().id();
        // An ordinary chat's subagent isn't a lead's: not adopted.
        let other = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some("s-20261110-l8orph2"),
            "chat",
            "t",
        )
        .with_payload(json!({ "parent": "s-20261110-plainparent", "prompt": "x" }));
        crate::run_queue::enqueue(v, other, now).unwrap();
        assert_eq!(arm_ended(v, now), 0);
        let batches: Vec<Wake> = load(v).into_iter().filter(|w| w.batch).collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].session_key, lead);
        let end = batches[0].on_sessions_end.as_ref().unwrap();
        assert_eq!(end.watched.len(), 1);
        assert_eq!(
            (
                end.watched[0].key.as_str(),
                end.watched[0].agent.as_str(),
                end.watched[0].queue_id
            ),
            ("s-20261110-l8orph", "codex", id)
        );
        // Adopted once.
        arm_ended(v, now);
        assert_eq!(load(v).iter().filter(|w| w.batch).count(), 1);
    }

    /// L8 review: batch reports have their own daily cap, apart from the
    /// lead's own wakes, so neither eats the other's; past it no batch.
    #[test]
    fn batch_reports_and_a_leads_own_wakes_have_separate_caps() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-cap").unwrap();
        let me = caller(&lead, "R-l8cap");
        let now = Utc::now();
        let done = |batch: bool, i: usize| Wake {
            id: format!("wake-cap-{batch}-{i}"),
            status: "queued".into(),
            batch,
            ..plan(&[], &me, 5, "x", now).unwrap()
        };
        let own: Vec<Wake> = (0..MAX_PER_DAY).map(|i| done(false, i)).collect();
        save(v, &own).unwrap();
        assert!(
            schedule(v, &me, 5, "again", now).is_err(),
            "own cap reached"
        );
        assert!(
            join_batch(v, &me, "s-20261110-l8cap1", "claude", 1, now),
            "a report still opens"
        );
        let mut reports: Vec<Wake> = (0..BATCH_MAX_PER_DAY).map(|i| done(true, i)).collect();
        for w in &mut reports {
            w.session_key = lead.clone();
        }
        save(v, &reports).unwrap();
        assert!(
            schedule(v, &me, 5, "again", now).is_ok(),
            "reports don't count against its own"
        );
        assert!(
            !join_batch(v, &me, "s-20261110-l8cap2", "claude", 2, now),
            "report cap reached"
        );
    }

    fn lead_child_row(v: &Path, lead: &str, child: &str, now: DateTime<Utc>) -> i64 {
        let row = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some(child),
            "chat",
            "t",
        )
        .with_payload(json!({ "parent": lead, "agent": "claude", "prompt": "do it" }));
        crate::run_queue::enqueue(v, row, now).unwrap().id()
    }

    /// L8 delta review: orphans of two leads adopted at one instant get two
    /// batches with distinct ids, and one lead's child ending arms only its
    /// own batch.
    #[test]
    fn orphans_of_two_leads_get_distinct_batches_that_arm_apart() {
        use crate::run_queue::RowStatus;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (a, b) = (
            crate::project_agents::key("proj-l8-twin-a").unwrap(),
            crate::project_agents::key("proj-l8-twin-b").unwrap(),
        );
        let now = Utc::now();
        let ka = lead_child_row(v, &a, "s-20261110-l8twa", now);
        lead_child_row(v, &b, "s-20261110-l8twb", now);
        assert_eq!(arm_ended(v, now), 0);
        let batches: Vec<Wake> = load(v).into_iter().filter(|w| w.batch).collect();
        assert_eq!(batches.len(), 2);
        assert_ne!(batches[0].id, batches[1].id, "distinct ids");
        crate::run_queue::claim(v, ka, now).unwrap();
        crate::run_queue::mark_running(v, ka, "R-l8twa", now).unwrap();
        crate::run_queue::finish(v, ka, RowStatus::Done, None, now).unwrap();
        assert_eq!(arm_ended(v, now), 1);
        let fired: Vec<String> = load(v)
            .into_iter()
            .filter(|w| {
                w.batch
                    && w.on_sessions_end
                        .as_ref()
                        .is_some_and(|e| e.fired_at.is_some())
            })
            .map(|w| w.session_key)
            .collect();
        assert_eq!(fired, vec![a]);
    }

    /// L8 delta review: a child that joined a batch is never adopted again,
    /// even after its batch left the wakes file (it keeps the newest 300).
    #[test]
    fn a_batch_member_is_not_adopted_again_after_its_batch_is_evicted() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l8-evict").unwrap();
        let now = Utc::now();
        let live = lead_child_row(v, &lead, "s-20261110-l8evl", now);
        assert!(join_batch(
            v,
            &caller(&lead, "R-l8ev"),
            "s-20261110-l8evl",
            "claude",
            live,
            now
        ));
        // 300 later wakes push the batch out of the file.
        let me = caller("s-20261110-l8filler", "R-l8fill");
        let filler: Vec<Wake> = (0..KEPT)
            .map(|i| Wake {
                id: format!("wake-fill-{i}"),
                status: "queued".into(),
                ..plan(&[], &me, 5, "x", now).unwrap()
            })
            .collect();
        save(v, &filler).unwrap();
        assert!(load(v).iter().all(|w| !w.batch), "evicted");
        arm_ended(v, now);
        assert!(
            load(v).iter().all(|w| !w.batch),
            "the live member isn't adopted again"
        );
    }

    /// PER-12 (11-14): a wake that comes due while its conversation is
    /// mid-turn waits for that turn's end, however long it runs, and the
    /// person's message meanwhile doesn't cancel it. One not yet due is
    /// still the person's to supersede (overnight C3).
    #[test]
    fn a_wake_due_during_a_turn_runs_after_it_and_survives_the_persons_message() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = t("2026-11-14T15:00:00Z");
        let key = "s-20261114-hol12";
        let me = caller(key, "run-hol12");
        let w = plan(&[], &me, 5, "verify the install", now).unwrap();
        save(v, std::slice::from_ref(&w)).unwrap();
        let row = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some(key),
            "chat",
            "t",
        )
        .with_payload(json!({ "prompt": "a long turn" }));
        let turn = crate::run_queue::enqueue(v, row, now).unwrap().id();
        let status = || load(v)[0].status.clone();
        // Due at 15:05; the turn runs on well past LATE_MINUTES.
        for minute in [5, 20, 40, 60] {
            let at = now + Duration::minutes(minute);
            assert!(
                claim_due(v, at, Some(false)).is_empty(),
                "busy at +{minute}"
            );
            assert_eq!(status(), "pending", "held, not missed, at +{minute}");
        }
        cancel_session_on_send(v, key, now + Duration::minutes(61));
        assert_eq!(status(), "pending", "the person's message keeps a due wake");
        crate::run_queue::finish(
            v,
            turn,
            crate::run_queue::RowStatus::Done,
            None,
            now + Duration::minutes(62),
        )
        .unwrap();
        let claimed = claim_due(v, now + Duration::minutes(63), Some(false));
        assert_eq!(claimed.len(), 1, "it runs once the turn ends");
        assert_eq!(claimed[0].id, w.id);
        settle(v, &claimed[0], Err("test".into()));
        // Not yet due: the person's message cancels it, as before.
        let later = plan(&[], &me, 30, "later", now).unwrap();
        save(v, std::slice::from_ref(&later)).unwrap();
        cancel_session_on_send(v, key, now);
        assert_eq!(status(), "cancelled");
    }
}
