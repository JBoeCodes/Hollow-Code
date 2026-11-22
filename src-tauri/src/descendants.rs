//! A run's descendants (agent effectiveness F10, wave 4 · D2): the
//! completion of the durable child tasks it started, and its cost with
//! theirs.
//!
//! **Completion without polling.** A durable child task (`hollow_app`
//! `start_session` / `send_to_session`, `delegation.rs`) is one queued chat
//! turn, recorded for the asking run with its run-queue row
//! (`DelegationRecord::child_queue_id`). The asking agent learns that it
//! ended in one of two ways, each reported once per child turn
//! (`session_store::deliver_child`, keyed by the queue row):
//! - `wait_for_session` ([`wait_for_session`]): a bounded wait in the tool
//!   call, at most [`crate::delegation::WAIT_MAX_SECONDS`], on one
//!   conversation or any of several (overnight OB10a), that returns as soon
//!   as a turn ends (or at the deadline, saying how long each has run, or at
//!   once when the turns are queued behind the vault this very turn holds);
//! - a `<hollow-child-finished>` note ([`notes_for_turn`]) at the front of
//!   the asking conversation's next turn, listing the child turns that ended
//!   since and were not reported yet. A retry of that same turn carries the
//!   same note again; any later turn doesn't. The note says it is news, not
//!   a request, so a model reading it twice still has nothing to redo.
//!
//! Each line carries what the turn left ([`child_summary`], overnight OB17):
//! its files, pull requests, failures and verification from its receipt, and
//! the end of its answer, at most [`answer_budget`] characters, so the asker
//! rarely needs `read_session` (which still has the whole answer). A
//! Main agent wake's lines carry the same summary.
//!
//! **The caller's own children** (overnight OB21). `list_sessions` `mine`
//! ([`mine_overview`]) lists the conversations a run or chat started and
//! their handover successors, each with that summary once it ended, and
//! `read_session` of an ended one defaults to its last answer
//! ([`is_ended_child`]).
//!
//! **Endings further down the chain** (overnight C1). The same note also
//! lists turns that ended further down the chains the conversation started
//! (a child's child, a handover's successor): every conversation up a chain
//! hears of them, not only the direct parent. [`chain_lines`] follows the
//! turns the chain itself caused: from each child task's turn (its queue
//! row's run) to the child tasks that run started, one query per level (at
//! most [`CHAIN_HOPS`] levels and [`CHAIN_SESSIONS`] runs below the seeds;
//! links up to the delegation records' 90 days, reports only from the last
//! [`CHAIN_DAYS`] days). Each ended turn is reported once per conversation
//! (`session_store::root_deliveries`, checked read-only, then one batched
//! write; separate from the direct parent's
//! [`crate::session_store::deliver_child`]). The Main agent's daily threads
//! count as one conversation: a chain started from last night's thread
//! reaches today's.
//!
//! **Cost roll-up** ([`rollup`]). A run's own usage rows, each bounded
//! consultation (`delegate` rows, which carry the parent's run id), each
//! helper one-shot run for it (`oneshot` rows with its run id: the learning
//! check) and each durable child task's turn, whose cost is that turn's own
//! roll-up (so a grandchild is counted inside its parent child, once). A
//! cost nobody reported stays `null` and makes every sum it is part of
//! `null`; `knownUsd` adds what is known. A child turn that never ran (it
//! was cancelled or skipped while queued) cost nothing: 0. A self-hosted
//! model's run (Q8, `billing` `self-hosted`) is billed by GPU time: `null`
//! in every sum too (never $0), but counted in `gpuTime`, not `unknown`,
//! and no known spend, so the night budget neither charges it nor fails
//! closed on it.

use crate::app_actions::Caller;
use crate::run_manifest::{CostRollup, DelegationRecord, Descendant};
use crate::run_queue::{QueueRow, RowStatus, RunSource};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// How deep a roll-up follows child tasks: a chain of agent-started
/// sessions is at most `app_mcp::MAX_START_DEPTH` (2) deep.
const MAX_DEPTH: u32 = 4;
/// The note's tag (`sessions::split_injected_context`,
/// `run_manifest::composer_layers`: layer `childFinished`).
pub(crate) const NOTE_TAG: &str = "hollow-child-finished";
const NOTE_OPEN: &str = "<hollow-child-finished>";
const NOTE_CLOSE: &str = "</hollow-child-finished>";
/// Child turns one note lists; the rest wait for the next turn.
pub(crate) const MAX_NOTES: usize = 8;
/// The asking conversation's newest delegations a note looks through.
const NOTE_SCAN: usize = 32;
/// Characters kept of a failed turn's error in a note.
const NOTE_ERROR_CHARS: usize = 160;
/// How far down a chain the note looks: a lane, its 24 handover successors,
/// a helper of one and that helper's own 24, with room to spare.
pub(crate) const CHAIN_HOPS: usize = 64;
// Overnight 9 A2 review: the walk must outlast two full chains.
const _: () = assert!(CHAIN_HOPS > 2 * (crate::app_sessions::MAX_HANDOVER_CHAIN as usize + 1));
/// Conversations one walk reads, at most (one query per level).
pub(crate) const CHAIN_SESSIONS: usize = 512;
/// Delegations older than this aren't news (and an upgrade doesn't flood
/// old endings).
pub(crate) const CHAIN_DAYS: i64 = 7;
/// How often a wait reads the queue row again.
const WAIT_POLL: Duration = Duration::from_millis(400);

const NOTE_HEADER: &str = "Durable child tasks you started have ended since your last turn. Each is reported once: this is news, not a new request, so don't redo work for it. Each line carries the end of its answer, its files and pull requests; call hollow_app read_session only when you need more.";
const BEHIND_THIS_TURN: &str = "It is queued for the vault, which this turn holds, so it starts only after this turn ends. Don't wait for it now: finish this turn, and Hollow tells you in your next turn when it has ended (read_session then has its answer).";
const BEHIND_THIS_TURNS_SLOT: &str = "It is waiting for a free chat slot (Settings → Chat → Chats running at once), and this turn holds the only one it could take, so it starts only after this turn ends. Don't wait for it now: finish this turn, and Hollow tells you in your next turn when it has ended (read_session then has its answer).";
const BEHIND_THIS_WORKTREE: &str = "It is queued for the build workspace this turn holds alone, so it starts only after this turn ends. Don't wait for it now: finish this turn, and Hollow tells you in your next turn when it has ended (read_session then has its answer).";
/// A timed-out wait's advice (overnight OB10a: 92% of waits came back
/// "running", one chat made 98 of them): a chat hears of its children's
/// ends in its next turn's note ([`notes_for_turn`]); the Main agent is
/// woken for it, and another chat when it asked to be (`sessions.wake_me`
/// `onSessionsEnd`, overnight OB10b). An automation run has no next turn,
/// and another conversation's turn is never noted.
const STILL_RUNNING: &str = "When a child task you started ends, Hollow reports it once as a <hollow-child-finished> note at the start of your next turn. If nothing else in this turn needs its answer, end the turn; to be woken when it ends, call run_action sessions.wake_me {onSessionsEnd: {keys}} first, where offered. read_session has its answer once it has ended.";
/// [`STILL_RUNNING`] in a Main agent thread, which is woken for its chats
/// already and is refused a wake on their ends (`wake_me::schedule_on_end`):
/// OB10a's wording.
const STILL_RUNNING_MAIN: &str = "When a child task you started ends, Hollow reports it once as a <hollow-child-finished> note at the start of your next turn. If nothing else in this turn needs its answer, end the turn (call run_action sessions.wake_me first, where offered, if you must come back on your own); read_session has its answer once it has ended.";
const STILL_RUNNING_RUN: &str =
    "Wait again only if this run needs its answer; read_session has it once it has ended.";
const BEHIND_ALL: &str = "Each is queued behind this turn, so none starts before this turn ends. Don't wait for them now: finish this turn, and Hollow tells you in your next turn when each has ended (read_session then has its answer).";
/// Conversations one wait names, at most (overnight OB10a).
pub(crate) const WAIT_MAX_KEYS: usize = 12;

/// The `keys` argument's schema as agents see it (`sessions.wait`).
pub(crate) fn wait_keys_schema() -> Value {
    json!({ "type": "array", "items": { "type": "string" }, "maxItems": WAIT_MAX_KEYS })
}

const KEY_AND_KEYS: &str = "wait_for_session takes key or keys, not both. Nothing was waited for.";
const NO_KEYS: &str =
    "wait_for_session needs key (one session) or keys (a non-empty list). Nothing was waited for.";
const EMPTY_KEY: &str =
    "wait_for_session: a key is empty (list_sessions shows them). Nothing was waited for.";
const FIRST_REPORT: &str = "Read its answer with read_session.";
const ALREADY_REPORTED: &str =
    "Already reported to you before; don't redo work for it. read_session has its answer.";

/// The child tasks run `run_id` started, each turn once (by its queue
/// row), oldest first. Records from before D2 carry no queue row and are
/// left out.
pub(crate) fn child_tasks(vault: &Path, run_id: &str) -> Vec<DelegationRecord> {
    once_per_turn(crate::delegation::for_run(vault, run_id))
}

fn once_per_turn(records: Vec<DelegationRecord>) -> Vec<DelegationRecord> {
    let mut seen = HashSet::new();
    records
        .into_iter()
        .filter(|record| record.kind == crate::delegation::Contract::ChildTask.as_str())
        .filter(|record| record.child_queue_id.is_some_and(|id| seen.insert(id)))
        .collect()
}

/// A child task's turn as the run queue holds it: its row, while that row
/// is still that conversation's chat turn (ids restart only with a new
/// database; a pruned row is `None`).
pub(crate) fn child_row(vault: &Path, record: &DelegationRecord) -> Option<QueueRow> {
    let id = record.child_queue_id?;
    let row = crate::run_queue::get(vault, id).ok()??;
    (row.source == RunSource::Chat && row.source_id.as_deref() == record.child_session.as_deref())
        .then_some(row)
}

/// Run `run_id`'s cost with its descendants' (see the module doc).
pub(crate) fn rollup(vault: &Path, run_id: &str) -> CostRollup {
    rollup_at(vault, run_id, 0, &mut HashSet::new())
}

/// Known ledger spend for cap accounting, not a complete cost total.
/// Unlike receipt roll-ups, follow live children and the full supported
/// handover chain. A missing provider cost contributes no *known* dollars;
/// it never turns the receipt's unknown total into a zero or partial total.
/// Share `seen` across roots so a run reached twice is charged only once.
/// Each root has its own existing chain work bound; a capped/pruned walk is a lower
/// bound, just as provider usage that has not arrived yet is unavailable.
pub(crate) fn recorded_chain_cost(
    vault: &Path,
    root: &str,
    since: &str,
    seen: &mut HashSet<String>,
) -> f64 {
    let mut pending = vec![(root.to_string(), 0usize)];
    let mut known = 0.0;
    let mut read = 0usize;
    while let Some((run, depth)) = pending.pop() {
        if read >= CHAIN_SESSIONS {
            break;
        }
        if !seen.insert(run.clone()) {
            continue;
        }
        read += 1;
        if let Ok(mut rows) =
            crate::session_store::usage_rows_for_runs(vault, std::slice::from_ref(&run))
        {
            known += rows
                .remove(&run)
                .unwrap_or_default()
                .iter()
                .filter(|r| r.started_at.as_str() >= since)
                .filter_map(|r| r.cost_usd)
                .sum::<f64>();
        }
        if depth + 1 < CHAIN_HOPS {
            for record in child_tasks(vault, &run) {
                if let Some(child) = child_row(vault, &record).and_then(|r| r.history_run_id) {
                    if !seen.contains(&child) {
                        pending.push((child, depth + 1));
                    }
                }
            }
        }
    }
    known
}

fn rollup_at(vault: &Path, run_id: &str, depth: u32, visited: &mut HashSet<String>) -> CostRollup {
    visited.insert(run_id.to_string());
    let rows = crate::session_store::usage_rows_for_runs(vault, &[run_id.to_string()])
        .ok()
        .and_then(|mut by_run| by_run.remove(run_id))
        .unwrap_or_default();
    let mut own = Vec::new();
    let mut own_gpu = 0u32;
    let mut descendants = Vec::new();
    // Per descendant: the known spend that came with GPU time (Q8), when it
    // was billed by GPU time.
    let mut gpu = Vec::new();
    for row in rows {
        let gpu_time = row.billing == crate::usage::Billing::SelfHosted.as_str();
        let kind = match row.surface.as_str() {
            "delegate" => "consultation",
            "oneshot" => "helper",
            _ if gpu_time => {
                own_gpu += 1;
                continue;
            }
            _ => {
                own.push(row.cost_usd);
                continue;
            }
        };
        gpu.push(gpu_time.then_some(0.0));
        descendants.push(Descendant {
            kind: kind.to_string(),
            agent: row.agent,
            status: row.outcome,
            cost_usd: row.cost_usd,
            purpose: (kind == "helper").then_some(row.scope).flatten(),
            ..Descendant::default()
        });
    }
    for record in child_tasks(vault, run_id) {
        let row = child_row(vault, &record);
        let mut child_gpu = None;
        let (status, child_run, cost) = match &row {
            None => ("unknown".to_string(), None, None),
            Some(row) if row.status.is_live() => (
                crate::run_pump::agent_status(vault, row).to_string(),
                row.history_run_id.clone(),
                None,
            ),
            Some(row) => {
                let status = row.status.as_str().to_string();
                match row.history_run_id.clone() {
                    // It ended without ever running: nothing was spent.
                    None => (status, None, Some(0.0)),
                    Some(child) if visited.contains(&child) || depth + 1 >= MAX_DEPTH => {
                        (status, Some(child), None)
                    }
                    Some(child) => {
                        let child_rollup = rollup_at(vault, &child, depth + 1, visited);
                        // Q8: GPU time and nothing unknown: no Anthropic spend
                        // beyond what it knows.
                        if child_rollup.gpu_time > 0
                            && child_rollup.unknown == 0
                            && child_rollup.total_usd.is_none()
                        {
                            child_gpu = Some(child_rollup.known_usd);
                        }
                        (status, Some(child), child_rollup.total_usd)
                    }
                }
            }
        };
        descendants.push(Descendant {
            kind: crate::delegation::Contract::ChildTask.as_str().to_string(),
            agent: Some(record.agent.clone()),
            status,
            cost_usd: cost,
            child_session: record.child_session.clone(),
            child_queue_id: record.child_queue_id,
            child_run_id: child_run,
            ..Descendant::default()
        });
        gpu.push(child_gpu);
    }
    totals_with(own, own_gpu, descendants, &gpu)
}

/// [`totals_with`] without GPU time.
#[cfg(test)]
fn totals(own: Vec<Option<f64>>, descendants: Vec<Descendant>) -> CostRollup {
    totals_with(own, 0, descendants, &[])
}

/// The sums of a run's own costs and its descendants', with GPU time
/// (Q8): `own_gpu` of the run's own rows were a
/// self-hosted model's, and `gpu[i]` is `Some(known spend)` when
/// descendant `i` was billed by GPU time (a child task's own roll-up may
/// know some spend besides). GPU time is no Anthropic spend and never
/// unknown: it adds nothing to `knownUsd` or `unknown`, yet a sum with it
/// stays `null`, so it is never shown as $0.
fn totals_with(
    own: Vec<Option<f64>>,
    own_gpu: u32,
    descendants: Vec<Descendant>,
    gpu: &[Option<f64>],
) -> CostRollup {
    let is_gpu = |i: usize| gpu.get(i).is_some_and(Option::is_some);
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    // The run's own cost is known, or GPU time, in every row it has.
    let own_settled = (!own.is_empty() || own_gpu > 0) && own.iter().all(Option::is_some);
    let own_usd = (own_settled && own_gpu == 0).then(|| own.iter().flatten().sum::<f64>());
    let descendants_usd = if descendants.iter().any(|d| d.cost_usd.is_none()) {
        None
    } else {
        Some(descendants.iter().filter_map(|d| d.cost_usd).sum())
    };
    let known_usd = own.iter().flatten().sum::<f64>()
        + descendants.iter().filter_map(|d| d.cost_usd).sum::<f64>()
        + gpu.iter().flatten().sum::<f64>();
    let unknown = u32::from(!own_settled)
        + count(
            descendants
                .iter()
                .enumerate()
                .filter(|(i, d)| d.cost_usd.is_none() && !is_gpu(*i))
                .count(),
        );
    let gpu_time = own_gpu.saturating_add(count(gpu.iter().flatten().count()));
    CostRollup {
        own_usd,
        descendants_usd,
        total_usd: own_usd.zip(descendants_usd).map(|(a, b)| a + b),
        known_usd,
        unknown,
        gpu_time,
        descendants,
    }
}

/// `stop` from `sealed` (a receipt's roll-up) onto `live`'s child tasks, by
/// queue row: what the parent's stop did is only on the receipt.
pub(crate) fn with_stops(mut live: CostRollup, sealed: Option<&CostRollup>) -> CostRollup {
    let Some(sealed) = sealed else {
        return live;
    };
    for child in live.descendants.iter_mut() {
        if let Some(id) = child.child_queue_id {
            child.stop = sealed
                .descendants
                .iter()
                .find(|d| d.child_queue_id == Some(id))
                .and_then(|d| d.stop.clone());
        }
    }
    live
}

// ── wait_for_session ───────────────────────────────────────────────────

/// The child turns `caller`'s run and (for a chat) its conversation
/// started, by queue row: the ones a wait reports as delivered.
fn callers_children(vault: &Path, caller: &Caller) -> HashSet<i64> {
    let mut records = crate::delegation::for_run(vault, &caller.run_id);
    if caller.surface == "chat" {
        records.extend(
            crate::session_store::delegations_for_session(
                vault,
                &caller.scope,
                i64::MAX,
                NOTE_SCAN,
            )
            .unwrap_or_default()
            .into_iter()
            .filter_map(|json| serde_json::from_str(&json).ok()),
        );
    }
    once_per_turn(records)
        .into_iter()
        .filter_map(|record| record.child_queue_id)
        .collect()
}

/// The conversations one wait names (overnight OB10a): exactly one of `key`
/// and `keys` (a non-empty list), each trimmed and named once, at most
/// [`WAIT_MAX_KEYS`]. Each is checked as a single key is ([`wait_with`]).
pub(crate) fn wait_keys(
    key: Option<String>,
    keys: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let named = match (key, keys) {
        (Some(_), Some(_)) => return Err(KEY_AND_KEYS.into()),
        (Some(key), None) => vec![key],
        (None, Some(keys)) => keys,
        (None, None) => return Err(NO_KEYS.into()),
    };
    let mut out: Vec<String> = Vec::new();
    for key in named {
        let key = key.trim().to_string();
        if key.is_empty() {
            return Err(EMPTY_KEY.into());
        }
        if !out.contains(&key) {
            out.push(key);
        }
    }
    if out.is_empty() {
        return Err(NO_KEYS.into());
    }
    if out.len() > WAIT_MAX_KEYS {
        return Err(format!(
            "wait_for_session waits on at most {WAIT_MAX_KEYS} sessions at once (got {}). Nothing was waited for.",
            out.len()
        ));
    }
    Ok(out)
}

/// `wait_for_session {key | keys, timeoutSeconds?}` (F10, D2; several keys
/// overnight OB10a): wait until conversation `key`'s turn ends, or any one
/// of `keys`' turns, at most `seconds` (1 to
/// [`crate::delegation::WAIT_MAX_SECONDS`]), reading their queue rows every
/// [`WAIT_POLL`]. Returns at once when a turn already ended, or when every
/// one is queued for the vault this caller's own run holds (it can't start
/// before this turn ends). A finished child of the caller is marked
/// reported, so its note never comes too; a second wait says it was
/// reported already. Never the caller's own conversation.
pub(crate) async fn wait_for_session(
    vault: &Path,
    caller: &Caller,
    keys: &[String],
    seconds: u64,
) -> Result<Value, String> {
    let run = caller.run_id.clone();
    let owner = vault.to_path_buf();
    wait_with(vault, caller, keys, seconds, move || {
        crate::automated_history::run_is_live(&owner, &run)
    })
    .await
}

/// [`wait_for_session`] with the caller's liveness injected: a wait whose
/// caller's run has ended (stopped: its `hollow_app` server is gone) stops
/// reading at once instead of holding a broker thread to its deadline.
pub(crate) async fn wait_with<K: AsRef<str>>(
    vault: &Path,
    caller: &Caller,
    keys: &[K],
    seconds: u64,
    caller_live: impl Fn() -> bool,
) -> Result<Value, String> {
    for key in keys {
        crate::app_sessions::not_own(caller, key.as_ref(), "wait_for_session")?;
    }
    let keys: Vec<&str> = keys.iter().map(|key| key.as_ref().trim()).collect();
    if keys.is_empty() {
        return Err(NO_KEYS.into());
    }
    let seconds = seconds.clamp(1, crate::delegation::WAIT_MAX_SECONDS);
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let children = callers_children(vault, caller);
    loop {
        let mut rows = Vec::with_capacity(keys.len());
        for key in &keys {
            let row = crate::run_queue::newest_chat_row(vault, key)?.ok_or_else(|| {
                format!("{key} has no turn queued, running or recently finished to wait for; read_session shows the conversation.")
            })?;
            rows.push(row);
        }
        let waited = started.elapsed().as_secs();
        // Any one ended answers (overnight OB10a), except a child's ending
        // already reported to the caller while others still run: asked
        // again, the wait waits for those instead of answering at once.
        let live_left = rows.iter().any(|row| row.status.is_live());
        let stale: Vec<bool> = rows
            .iter()
            .map(|row| live_left && row.status.is_terminal() && reported(vault, &children, row))
            .collect();
        let ended = rows
            .iter()
            .zip(&stale)
            .any(|(row, stale)| row.status.is_terminal() && !stale);
        // It returns at once when none of the live ones can start before
        // this turn ends.
        let mut behind = vec![None; rows.len()];
        if !ended {
            for (note, row) in behind.iter_mut().zip(&rows) {
                *note = behind_note(vault, caller, row);
            }
        }
        let all_behind = rows
            .iter()
            .zip(&behind)
            .filter(|(row, _)| row.status.is_live())
            .all(|(_, note)| note.is_some());
        let now = Instant::now();
        if ended || all_behind || now >= deadline || !caller_live() {
            let turns = Turns {
                keys: &keys,
                rows: &rows,
                stale: &stale,
                behind: &behind,
                children: &children,
            };
            return Ok(answer(vault, caller, &turns, waited));
        }
        tokio::time::sleep(WAIT_POLL.min(deadline - now)).await;
    }
}

/// Whether the caller already heard of its child's ended turn `row` (a
/// wait or a note reported it).
fn reported(vault: &Path, children: &HashSet<i64>, row: &QueueRow) -> bool {
    children.contains(&row.id)
        && matches!(
            crate::session_store::child_delivery(vault, row.id),
            Ok(Some(_))
        )
}

/// What one poll of a wait read: each key's newest turn, whether it is an
/// ending already reported while others run, and why a live one can't start
/// yet; with the caller's children, by queue row.
struct Turns<'a> {
    keys: &'a [&'a str],
    rows: &'a [QueueRow],
    stale: &'a [bool],
    behind: &'a [Option<&'static str>],
    children: &'a HashSet<i64>,
}

/// Why a live turn `row` can't start before the caller's turn ends, if it
/// can't: a child queued for the vault this turn holds, or for a build
/// worktree this turn holds alone, or (2026-10-20) for a chat slot only
/// this turn's ending can free (a workspace or app-wide limit this turn
/// alone fills). One in a workspace folder this turn shares otherwise
/// starts beside it (parallel chats in one workspace, 2026-10-07).
fn behind_note(vault: &Path, caller: &Caller, row: &QueueRow) -> Option<&'static str> {
    if row.status != RowStatus::Queued {
        return None;
    }
    let behind = match crate::run_pump::chat_scope_mode(vault, row) {
        None => crate::automated_history::lease_is_run(vault, &caller.run_id),
        Some((scope, mode)) => {
            crate::automated_history::blocked_by(vault, &scope, &mode, &caller.run_id)
        }
    };
    if behind {
        return Some(if crate::run_pump::scoped_chat(vault, row) {
            BEHIND_THIS_WORKTREE
        } else {
            BEHIND_THIS_TURN
        });
    }
    crate::run_pump::waits_only_for_slot_of(vault, row, &caller.run_id)
        .then_some(BEHIND_THIS_TURNS_SLOT)
}

/// A wait's answer once it returns, for the turns it read. One key answers
/// as before OB10a: its ended turn ([`finished`]), or its live one with how
/// long it has run and the wait's note. Several answer with each one's
/// under `sessions`: the ended ones first (named in `ended`), then the live
/// ones, then endings reported before.
fn answer(vault: &Path, caller: &Caller, turns: &Turns<'_>, waited: u64) -> Value {
    let now = chrono::Utc::now();
    let mut ended = Vec::new();
    let mut sessions = Vec::new();
    let mut pending = Vec::new();
    let mut old = Vec::new();
    let mut live = Vec::new();
    let mut live_behind = Vec::new();
    // The note's advice holds only when every live turn is the caller's
    // child: no other conversation's turn is ever noted to it.
    let mut all_children = true;
    let each = turns.keys.iter().zip(turns.rows).zip(turns.stale);
    for (((key, row), stale), behind) in each.zip(turns.behind) {
        if *stale {
            old.push(finished(vault, caller, key, row, waited));
            continue;
        }
        if row.status.is_terminal() {
            ended.push(*key);
            sessions.push(finished(vault, caller, key, row, waited));
            continue;
        }
        all_children &= turns.children.contains(&row.id);
        live_behind.push(*behind);
        let (state, seconds) = live_for(vault, row, now);
        let mut entry = json!({
            "session": key,
            "finished": false,
            "status": crate::run_pump::agent_status(vault, row),
        });
        entry[format!("{state}Seconds")] = json!(seconds);
        if let Some(note) = behind {
            entry["note"] = json!(note);
        }
        live.push(format!("{key} {state} for {}", span(seconds)));
        pending.push(entry);
    }
    // The ended ones lead, then the live ones, then endings reported before.
    sessions.extend(pending);
    sessions.extend(old);
    let note = if !ended.is_empty() {
        None
    } else if let [Some(one)] = live_behind.as_slice() {
        Some(one.to_string())
    } else if live_behind.iter().all(Option::is_some) {
        Some(BEHIND_ALL.to_string())
    } else {
        let advice = if caller.surface == "chat" && all_children {
            if crate::wake_me::in_main_agent(vault, &caller.scope) {
                STILL_RUNNING_MAIN
            } else {
                STILL_RUNNING
            }
        } else {
            STILL_RUNNING_RUN
        };
        Some(format!("Not finished yet: {}. {advice}", live.join(", ")))
    };
    if sessions.len() == 1 {
        let mut only = sessions.remove(0);
        if let Some(note) = note {
            only["waitedSeconds"] = json!(waited);
            only["note"] = json!(note);
        }
        return only;
    }
    let mut out = json!({
        "finished": !ended.is_empty(),
        "sessions": sessions,
        "waitedSeconds": waited,
    });
    match note {
        Some(note) => out["note"] = json!(note),
        None => out["ended"] = json!(ended),
    }
    out
}

/// How long `row`'s live turn has been at it by `now` (overnight OB10a):
/// `("running", seconds)` since it was claimed, else `("queued", seconds)`
/// since it was asked for. An unreadable stamp reads as 0.
pub(crate) fn live_for(
    vault: &Path,
    row: &QueueRow,
    now: chrono::DateTime<chrono::Utc>,
) -> (&'static str, u64) {
    // A queued row nothing truly holds back is being prepared, not queued
    // (2026-11-16): its wait counts from the request.
    let working = row.status == RowStatus::Queued
        && crate::run_pump::agent_status(vault, row) != RowStatus::Queued.as_str();
    let (state, since) = match (row.status, row.claimed_at.as_deref()) {
        _ if working => ("running", row.requested_at.as_str()),
        (RowStatus::Queued, _) | (_, None) => ("queued", row.requested_at.as_str()),
        (_, Some(at)) => ("running", at),
    };
    let seconds = chrono::DateTime::parse_from_rfc3339(since.trim())
        .map(|at| (now - at.with_timezone(&chrono::Utc)).num_seconds())
        .unwrap_or(0);
    (state, u64::try_from(seconds).unwrap_or(0))
}

/// `seconds` as `45s`, `4m 05s` or `2h 03m`.
pub(crate) fn span(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3_599 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3_600, seconds % 3_600 / 60),
    }
}

/// A wait's answer for an ended turn `row`, reported once when it is one of
/// the caller's children.
fn finished(vault: &Path, caller: &Caller, key: &str, row: &QueueRow, waited: u64) -> Value {
    let reported_before = if callers_children(vault, caller).contains(&row.id) {
        match crate::session_store::deliver_child(vault, row.id, &caller.scope, key, "wait", None) {
            Ok(earlier) => Some(earlier.is_some()),
            Err(error) => {
                eprintln!("[descendants] report of {key}: {error}");
                None
            }
        }
    } else {
        None
    };
    let cost = match &row.history_run_id {
        Some(run) => rollup(vault, run).total_usd,
        None => Some(0.0),
    };
    let mut answer = json!({
        "session": key,
        "finished": true,
        "status": row.status.as_str(),
        "finishedAt": row.finished_at,
        "runId": row.history_run_id,
        "costUsd": cost,
        "waitedSeconds": waited,
        "note": if reported_before == Some(true) { ALREADY_REPORTED } else { FIRST_REPORT },
    });
    if let Some(error) = &row.error {
        answer["error"] = json!(error);
    }
    if let Some((next, withdrawn)) = successor_of_turn(vault, row) {
        answer["successor"] = json!(next);
        if withdrawn {
            answer["successorWithdrawn"] = json!(true);
        }
    }
    if let Some(before) = reported_before {
        answer["reportedBefore"] = json!(before);
    }
    // Overnight A8: its reply says checks pass that its receipt doesn't back.
    let claim = row
        .history_run_id
        .as_deref()
        .and_then(|run| chat_receipt(vault, key, run))
        .and_then(|receipt| unbacked_claim(&receipt));
    if let Some(clause) = claim {
        answer["unbackedClaim"] = json!(clause);
    }
    answer
}

// ── The note in the asking conversation's next turn ──────────────────────

/// The `<hollow-child-finished>` note for conversation `key`'s turn
/// `turn_ord`, with the number of child turns it lists; empty with none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Notes {
    pub text: String,
    pub children: u32,
}

/// The child turns conversation `key` started in its turns before
/// `turn_ord` that have ended and were never reported, or were reported by
/// this same turn's note (a retried attempt carries it again). At most
/// [`MAX_NOTES`], oldest first; each is marked reported by this turn. A
/// note compiled for a turn that then never reaches its agent is lost
/// (like an `<operator-update>`): `read_session` still has the answer.
pub(crate) fn notes_for_turn(vault: &Path, key: &str, turn_ord: i64) -> Notes {
    let records: Vec<DelegationRecord> =
        crate::session_store::delegations_for_session(vault, key, turn_ord, NOTE_SCAN)
            .unwrap_or_default()
            .into_iter()
            .rev()
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
    let mut lines = Vec::new();
    let direct = once_per_turn(records);
    let mut children: Vec<String> = direct
        .iter()
        .filter_map(|r| r.child_session.clone())
        .collect();
    // The Main agent's daily threads are one conversation for chains: its
    // children from every thread of the last days are the walk's start.
    let main = crate::main_agent::main_cwd(vault);
    let in_main = main
        .as_deref()
        .is_some_and(|cwd| crate::session_store::session_cwd(vault, key).as_deref() == Some(cwd));
    let root_key = if in_main {
        let since = chrono::Utc::now() - chrono::Duration::days(CHAIN_DAYS);
        let cwd = main.as_deref().unwrap_or("");
        for (_, json) in
            crate::session_store::delegations_for_cwd(vault, cwd, since, CHAIN_SESSIONS)
                .unwrap_or_default()
        {
            let child = serde_json::from_str::<DelegationRecord>(&json)
                .ok()
                .and_then(|r| r.child_session);
            if let Some(child) = child {
                if !children.contains(&child) {
                    children.push(child);
                }
            }
        }
        "main-agent".to_string()
    } else {
        key.to_string()
    };
    for record in direct {
        if lines.len() >= MAX_NOTES {
            break;
        }
        let Some(row) = child_row(vault, &record) else {
            continue;
        };
        if row.status.is_live() {
            continue;
        }
        let ours = |delivery: &crate::session_store::ChildDelivery| {
            delivery.via == "note" && delivery.turn_ord == Some(turn_ord)
        };
        match crate::session_store::child_delivery(vault, row.id) {
            Ok(None) => {}
            Ok(Some(delivery)) if ours(&delivery) => {}
            _ => continue,
        }
        let session = record.child_session.clone().unwrap_or_default();
        match crate::session_store::deliver_child(
            vault,
            row.id,
            key,
            &session,
            "note",
            Some(turn_ord),
        ) {
            Ok(None) => {}
            Ok(Some(delivery)) if ours(&delivery) => {}
            _ => continue,
        }
        lines.push(Ended {
            line: note_line(vault, &session, &record.agent, &row),
            session,
            row,
        });
    }
    let deeper = chain_lines(
        vault,
        &root_key,
        key,
        &children,
        turn_ord,
        MAX_NOTES.saturating_sub(lines.len()),
    );
    if lines.is_empty() && deeper.is_empty() {
        return Notes::default();
    }
    let count = lines.len() + deeper.len();
    // The answers share one budget across every line (overnight OB17).
    let answer_chars = answer_budget(count);
    let render = |ended: &[Ended]| {
        ended
            .iter()
            .map(|e| e.render(vault, answer_chars))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut body = render(&lines);
    if !deeper.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(CHAIN_HEADER);
        body.push('\n');
        body.push_str(&render(&deeper));
    }
    Notes {
        children: u32::try_from(count).unwrap_or(u32::MAX),
        text: format!("{NOTE_OPEN}\n{NOTE_HEADER}\n{body}\n{NOTE_CLOSE}\n\n"),
    }
}

const CHAIN_HEADER: &str = "Further down the chains you started (a child's child, a handover's successor), also reported once:";

/// Turns that ended further down the chains conversation `key` started
/// (`root` is its delivery key: the conversation, or the Main agent's), below
/// its direct children (`children`, plus its child tasks of the window),
/// not yet reported to `root`, at most `room` new ones. Lines this same turn
/// already reported (a retried attempt) come first; the new ones are marked
/// reported by this turn in one write.
fn chain_lines(
    vault: &Path,
    root: &str,
    key: &str,
    children: &[String],
    turn_ord: i64,
    room: usize,
) -> Vec<Ended> {
    let links_since = chrono::Utc::now() - chrono::Duration::days(90);
    let report_since =
        crate::session_store::db_time(chrono::Utc::now() - chrono::Duration::days(CHAIN_DAYS));
    // The seeds: this conversation's own child tasks (and, for the Main
    // agent, every thread's), each one's turn the start of a chain.
    let mut seeds: Vec<DelegationRecord> = crate::session_store::child_delegations_for_session(
        vault,
        key,
        links_since,
        CHAIN_SESSIONS,
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|json| serde_json::from_str(json).ok())
    .collect();
    if root != key {
        if let Some(cwd) = crate::main_agent::main_cwd(vault) {
            for (_, json) in
                crate::session_store::delegations_for_cwd(vault, &cwd, links_since, CHAIN_SESSIONS)
                    .unwrap_or_default()
            {
                if let Ok(record) = serde_json::from_str::<DelegationRecord>(&json) {
                    seeds.push(record);
                }
            }
        }
    }
    // Never report the asking conversation's own turns or a direct child's.
    let skip: HashSet<String> = children.iter().cloned().chain([key.to_string()]).collect();
    let mut once: HashSet<i64> = HashSet::new();
    let mut frontier: Vec<(String, String)> = Vec::new();
    for record in &seeds {
        let (Some(id), Some(session)) = (record.child_queue_id, record.child_session.clone())
        else {
            continue;
        };
        if !once.insert(id) {
            continue;
        }
        if let Some(run) = crate::run_queue::get(vault, id)
            .ok()
            .flatten()
            .and_then(|r| r.history_run_id)
        {
            frontier.push((run, session));
        }
    }
    // The walk: from each turn's run to the child tasks it started.
    let mut found: Vec<(String, DelegationRecord, String)> = Vec::new();
    let mut runs_read = 0usize;
    for _ in 0..CHAIN_HOPS {
        if frontier.is_empty() || runs_read >= CHAIN_SESSIONS {
            break;
        }
        let keep = CHAIN_SESSIONS - runs_read;
        if frontier.len() > keep {
            // The newest turns matter most.
            frontier.drain(..frontier.len() - keep);
        }
        runs_read += frontier.len();
        let parent_of: std::collections::HashMap<String, String> =
            frontier.iter().cloned().collect();
        let runs: Vec<String> = frontier.iter().map(|(run, _)| run.clone()).collect();
        let rows = crate::session_store::child_delegations_for_runs(
            vault,
            &runs,
            links_since,
            CHAIN_SESSIONS * 4,
        )
        .unwrap_or_default();
        let mut next = Vec::new();
        for (run, json) in rows {
            let Ok(record) = serde_json::from_str::<DelegationRecord>(&json) else {
                continue;
            };
            let (Some(id), Some(session)) = (record.child_queue_id, record.child_session.clone())
            else {
                continue;
            };
            if !once.insert(id) || skip.contains(&session) {
                continue;
            }
            let row = crate::run_queue::get(vault, id).ok().flatten();
            if let Some(child_run) = row.as_ref().and_then(|r| r.history_run_id.clone()) {
                next.push((child_run, session.clone()));
            }
            let parent = parent_of.get(&run).cloned().unwrap_or_default();
            found.push((parent, record, run));
        }
        frontier = next;
    }
    // Which ended, and which this root already heard of.
    let ids: Vec<i64> = found
        .iter()
        .filter_map(|(_, r, _)| r.child_queue_id)
        .collect();
    let reported = crate::session_store::root_deliveries(vault, root, &ids).unwrap_or_default();
    let mut ours = Vec::new();
    let mut fresh = Vec::new();
    for (parent, record, _) in &found {
        let Some(id) = record.child_queue_id else {
            continue;
        };
        if record.at.as_str() < report_since.as_str() {
            continue;
        }
        // A retry is this same conversation's same turn: with the Main
        // agent's threads sharing one root, a turn number alone isn't enough.
        let earlier = reported.get(&id);
        if earlier.is_some_and(|(asker, ord)| asker != key || *ord != turn_ord) {
            continue;
        }
        let Some(row) = child_row(vault, record) else {
            continue;
        };
        if row.status.is_live() {
            continue;
        }
        let session = record.child_session.clone().unwrap_or_default();
        let line = Ended {
            line: format!(
                "{} (from {parent})",
                note_line(vault, &session, &record.agent, &row)
            ),
            session,
            row,
        };
        if earlier.is_some() {
            ours.push(line);
        } else {
            fresh.push((id, line));
        }
    }
    let mut lines = ours;
    let mut marked = Vec::new();
    for (id, line) in fresh {
        if lines.len() >= room {
            break;
        }
        lines.push(line);
        marked.push(id);
    }
    // A failed write leaves them unrecorded: the next turn reports them
    // again rather than never.
    let _ = crate::session_store::deliver_to_root(vault, root, key, turn_ord, &marked);
    lines
}

/// The successor the turn that ran as `row` handed over to (overnight C2),
/// from that turn's own `handover` delegation record, and whether a stop
/// withdrew it (its row ended cancelled). Per turn: a conversation that
/// handed over and later took another turn didn't hand that one over.
pub(crate) fn successor_of_turn(vault: &Path, row: &QueueRow) -> Option<(String, bool)> {
    let record = handover_record(vault, row)?;
    let key = record.child_session.clone()?;
    let withdrawn = child_row(vault, &record).is_some_and(|r| r.status == RowStatus::Cancelled);
    Some((key, withdrawn))
}

/// The `handover` delegation record of the turn that ran as `row`, if that
/// turn handed over (its newest).
fn handover_record(vault: &Path, row: &QueueRow) -> Option<DelegationRecord> {
    let run = row.history_run_id.as_deref()?;
    crate::delegation::for_run(vault, run)
        .into_iter()
        .rev()
        .find(|r| {
            r.kind == crate::delegation::Contract::ChildTask.as_str()
                && r.tool == crate::app_sessions::HANDOVER_TOOL
        })
}

/// `, handed over to <key>` (or `, its successor <key> was withdrawn`) for
/// the turn that ran as `row`, so whoever hears of its ending learns the
/// successor's key from Hollow, not only from its reply (morning M3).
pub(crate) fn handed_over(vault: &Path, row: &QueueRow) -> String {
    match successor_of_turn(vault, row) {
        Some((key, false)) => format!(", handed over to {key}"),
        Some((key, true)) => format!(", its successor {key} was withdrawn"),
        None => String::new(),
    }
}

/// One ended child turn a note lists: its line ([`note_line`], plus
/// `(from …)` further down a chain), and what its summary is read from once
/// the note knows how many lines share the answer budget.
struct Ended {
    line: String,
    session: String,
    row: QueueRow,
}

impl Ended {
    fn render(&self, vault: &Path, answer_chars: usize) -> String {
        let summary = child_summary(vault, &self.session, &self.row);
        format!("{}{}", self.line, summary.text(answer_chars))
    }
}

// ── What an ended child turn left (overnight OB17) ───────────────────────

/// Changed paths a summary names, at most.
const SUMMARY_PATHS: usize = 5;
/// Characters kept of one path in a summary (its end, where the name is).
const SUMMARY_PATH_CHARS: usize = 120;
/// Pull requests a summary names, at most.
const SUMMARY_PRS: usize = 5;
/// Characters kept of a PR's action or the verification state (both one
/// word when Hollow wrote them; the store is agent-writable).
const WORD_CHARS: usize = 24;
/// Answer characters one wake or note shares among the children it lists.
const ANSWER_SHARED_CHARS: usize = 3_600;
const ANSWER_MIN_CHARS: usize = 300;
const ANSWER_MAX_CHARS: usize = 1_200;

/// The answer characters each of `children` ended turns gets in one wake or
/// note: `clamp(3,600 / children, 300, 1,200)`, so one child gets 1,200 and
/// eight get 450 each.
pub(crate) fn answer_budget(children: usize) -> usize {
    (ANSWER_SHARED_CHARS / children.max(1)).clamp(ANSWER_MIN_CHARS, ANSWER_MAX_CHARS)
}

/// What one ended child turn left (overnight OB17), from its transcript and
/// its evidence receipt: what a `<hollow-child-finished>` note's line and a
/// Main agent wake's line carry after the child's status and end time, so
/// the asker rarely needs `read_session` first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChildSummary {
    /// Files it changed: the receipt's vault files plus every changed path
    /// of its repositories.
    pub files: usize,
    /// The first of them, at most [`SUMMARY_PATHS`].
    pub paths: Vec<String>,
    /// `(number, action)` of each pull request it merged or opened.
    pub pull_requests: Vec<(u64, String)>,
    /// The receipt's failures (a failed check, a failed run, …).
    pub failures: usize,
    /// The receipt's `verificationState`; `None` when it has no receipt.
    pub verification: Option<String>,
    /// Overnight A8: its reply claims checks pass that the receipt doesn't
    /// back ([`UNBACKED_AFTER_FAIL`] or [`UNBACKED_NO_CHECK`]); `None` when
    /// the receipt raises neither flag.
    pub unbacked_claim: Option<&'static str>,
    /// Its answer, through `claude_runner::redact`; empty when none was
    /// recorded.
    pub answer: String,
    /// COO rebuild L9: the CFO paused it at its budget (the recorded note,
    /// `cfo::budgets::pause_note`), so its parent hears why it stopped.
    pub paused: Option<String>,
}

/// The summary of conversation `session`'s turn that ran as `row`. A turn
/// that never ran (no run id) has none; one without a receipt has only its
/// answer.
pub(crate) fn child_summary(vault: &Path, session: &str, row: &QueueRow) -> ChildSummary {
    let paused = crate::cfo::budgets::pause_note(vault, session);
    let Some(run) = row
        .history_run_id
        .as_deref()
        .filter(|run| !run.trim().is_empty())
    else {
        return ChildSummary {
            paused,
            ..ChildSummary::default()
        };
    };
    let answer = crate::session_store::turn_reply_for_run(vault, session, run)
        .ok()
        .flatten()
        .map(|text| crate::claude_runner::redact(&text))
        .unwrap_or_default();
    let mut summary = ChildSummary {
        answer,
        paused,
        ..ChildSummary::default()
    };
    if let Some(receipt) = chat_receipt(vault, session, run) {
        let repo_paths = receipt
            .repo_changes
            .iter()
            .flat_map(|repo| repo.files.iter().map(|f| f.path.clone()));
        // Only the child's own files: another chat's in a shared workspace
        // folder are not its work (chat concurrency F1, 2026-10-20).
        summary.paths = receipt
            .files
            .iter()
            .filter(|f| f.is_own())
            .map(|f| f.path.clone())
            .chain(repo_paths)
            .take(SUMMARY_PATHS)
            .collect();
        let repo_files: usize = receipt
            .repo_changes
            .iter()
            .map(|repo| repo.total_files.max(repo.files.len()))
            .sum();
        summary.files = receipt.files.iter().filter(|f| f.is_own()).count() + repo_files;
        summary.pull_requests = receipt
            .pull_requests
            .iter()
            .map(|pr| (pr.number, pr.action.clone()))
            .collect();
        summary.failures = receipt.failures.len();
        summary.verification = Some(receipt.verification_state.clone());
        summary.unbacked_claim = unbacked_claim(&receipt);
    }
    summary
}

/// Conversation `session`'s chat receipt for run `run`. The store sits in
/// the agent-writable vault: only this conversation's chat receipt counts
/// (`find_by_history_run` already refuses duplicates).
fn chat_receipt(
    vault: &Path,
    session: &str,
    run: &str,
) -> Option<crate::evidence::EvidenceReceipt> {
    crate::evidence::find_by_history_run(vault, run)
        .ok()
        .flatten()
        .filter(|r| r.source == "chat")
        .filter(|r| r.session_key.as_deref().is_none_or(|key| key == session))
}

/// Overnight A8: the clause for `receipt`'s unbacked claim, a failed last
/// check winning over no check; `None` without a flag. A done or fixed
/// claim is not reported as a pass (overhaul E3).
fn unbacked_claim(receipt: &crate::evidence::EvidenceReceipt) -> Option<&'static str> {
    let v = receipt.verification.as_ref()?;
    let finished = v.claim == "done" || v.claim == "fixed";
    let (after_fail, no_check) = if v.claim == "fixed" {
        (UNBACKED_FIXED_AFTER_FAIL, UNBACKED_FIXED_NO_CHECK)
    } else if finished {
        (UNBACKED_DONE_AFTER_FAIL, UNBACKED_DONE_NO_CHECK)
    } else {
        (UNBACKED_AFTER_FAIL, UNBACKED_NO_CHECK)
    };
    if v.claim_after_fail {
        Some(after_fail)
    } else if v.claim_without_check {
        Some(no_check)
    } else {
        None
    }
}

/// Overnight A8: the clause a child's line gets when its reply says its
/// checks pass but its last recognised check failed
/// (`ReceiptVerification::claim_after_fail`). At most 60 bytes with its `; `.
pub(crate) const UNBACKED_AFTER_FAIL: &str = "says it passes; its last check failed";
/// … or when no check ran after its last edit (`claim_without_check`).
pub(crate) const UNBACKED_NO_CHECK: &str = "says it passes; no check after its last edit";
/// The same two facts when the reply claimed the work is done, not that a
/// check passed.
pub(crate) const UNBACKED_DONE_AFTER_FAIL: &str = "says it is done; its last check failed";
pub(crate) const UNBACKED_DONE_NO_CHECK: &str = "says it is done; no check after its last edit";
/// … or fixed.
pub(crate) const UNBACKED_FIXED_AFTER_FAIL: &str = "says it is fixed; its last check failed";
pub(crate) const UNBACKED_FIXED_NO_CHECK: &str = "says it is fixed; no check after its last edit";

impl ChildSummary {
    /// The summary as it follows a child's line: what the receipt says
    /// (`; 2 files changed (a.rs, b.rs); PR #7 merged; 1 failure;
    /// verification notVerified`), then the end of its answer, at most
    /// `answer_chars` characters ([`answer_tail`]), each line indented
    /// under the child's. `<` is escaped as `‹`, as the lines around it are.
    pub(crate) fn text(&self, answer_chars: usize) -> String {
        let mut out = String::new();
        if let Some(state) = &self.verification {
            if self.files == 0 {
                out.push_str("; no files changed");
            } else {
                let noun = if self.files == 1 { "file" } else { "files" };
                let mut paths: Vec<String> = self.paths.iter().map(|p| clip_path(p)).collect();
                if self.files > paths.len() {
                    paths.push("…".into());
                }
                let listed = paths.join(", ");
                out.push_str(&format!("; {} {noun} changed ({listed})", self.files));
            }
            if !self.pull_requests.is_empty() {
                let mut prs: Vec<String> = self
                    .pull_requests
                    .iter()
                    .take(SUMMARY_PRS)
                    .map(|(n, action)| format!("#{n} {}", one_line(action, WORD_CHARS)))
                    .collect();
                if self.pull_requests.len() > SUMMARY_PRS {
                    prs.push("…".into());
                }
                out.push_str(&format!("; PR {}", prs.join(", ")));
            }
            match self.failures {
                0 => {}
                1 => out.push_str("; 1 failure"),
                n => out.push_str(&format!("; {n} failures")),
            }
            out.push_str(&format!("; verification {}", one_line(state, WORD_CHARS)));
            if let Some(clause) = self.unbacked_claim {
                out.push_str("; ");
                out.push_str(clause);
            }
        }
        // L9: paused at its budget, before what it said of its own work.
        if let Some(note) = &self.paused {
            out.push_str("; ");
            out.push_str(&one_line(note, 400));
        }
        // Repo ideas 2026-10-27 #7: what the child said of its own work, from
        // its last line ("no status line" when it gave none: never done).
        if !self.answer.trim().is_empty() {
            out.push_str("; it says ");
            out.push_str(&one_line(
                &crate::child_status::parse(&self.answer).words(),
                240,
            ));
        }
        let (tail, cut) = answer_tail(&self.answer, answer_chars);
        if !tail.trim().is_empty() {
            if cut {
                out.push_str(&format!(
                    "\n  Its answer ends (last {} of {} characters):",
                    tail.chars().count(),
                    self.answer.trim().chars().count()
                ));
            } else {
                out.push_str("\n  Its answer:");
            }
            for line in tail.lines().filter(|l| !l.trim().is_empty()) {
                out.push_str("\n    ");
                out.push_str(line.trim_end());
            }
        }
        out.replace('<', "‹")
    }
}

/// The end of `answer` (trimmed), at most `chars` characters, and whether
/// anything was cut. A cut inside a line moves on to the next line's start,
/// unless the kept part is one line: then it stays on its char boundary.
pub(crate) fn answer_tail(answer: &str, chars: usize) -> (&str, bool) {
    let answer = answer.trim();
    let total = answer.chars().count();
    if total <= chars {
        return (answer, false);
    }
    let start = answer
        .char_indices()
        .nth(total - chars)
        .map_or(answer.len(), |(at, _)| at);
    let tail = &answer[start..];
    if answer[..start].ends_with('\n') {
        return (tail, true);
    }
    match tail.find('\n') {
        Some(nl) if !tail[nl + 1..].trim().is_empty() => (&tail[nl + 1..], true),
        _ => (tail, true),
    }
}

/// A path on one line (control characters as spaces), cut to its last
/// [`SUMMARY_PATH_CHARS`] characters.
fn clip_path(path: &str) -> String {
    let path = one_line(path, usize::MAX);
    let count = path.chars().count();
    if count <= SUMMARY_PATH_CHARS {
        return path;
    }
    let kept: String = path.chars().skip(count - SUMMARY_PATH_CHARS).collect();
    format!("…{kept}")
}

/// `text` on one line, so a receipt value can never start a line of its
/// own (a fake child): control characters (`\r`, `\n`, NEL among them) and
/// the Unicode line and paragraph separators become spaces, and it is cut
/// to `max` characters with `…`.
fn one_line(text: &str, max: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max).collect();
    cut.push('…');
    cut
}

fn note_line(vault: &Path, session: &str, agent: &str, row: &QueueRow) -> String {
    let mut line = format!("- {session} ({agent}): {}", row.status.as_str());
    if let Some(at) = &row.finished_at {
        line.push_str(&format!(", ended {at}"));
    }
    line.push_str(&handed_over(vault, row));
    if row.status != RowStatus::Done {
        if let Some(error) = row.error.as_deref().filter(|e| !e.trim().is_empty()) {
            let one_line = error.split_whitespace().collect::<Vec<_>>().join(" ");
            let mut clipped: String = one_line.chars().take(NOTE_ERROR_CHARS).collect();
            if clipped.len() < one_line.len() {
                clipped.push('…');
            }
            line.push_str(&format!(" ({})", clipped.replace('<', "‹")));
        }
    }
    line
}

// ── The caller's own children (overnight OB21) ───────────────────────────

/// Rows `list_sessions` `mine` lists, at most (children and successors).
const MINE_ROWS: usize = 30;
/// Answer characters each ended row of `list_sessions` `mine` carries.
const MINE_ANSWER_CHARS: usize = 400;
/// How far back `mine` looks for the caller's child tasks.
const MINE_DAYS: i64 = 90;

/// A conversation the caller started as a child task, or one that such a
/// conversation handed over to (`from`), with the turn of it the chain
/// caused (`None` once the queue pruned it).
struct Mine {
    key: String,
    agent: String,
    row: Option<QueueRow>,
    from: Option<String>,
}

/// The child-task records `caller` made (its run's and, in a chat, its
/// conversation's of the last [`MINE_DAYS`] days), each turn once, newest
/// turn first (queue ids grow).
fn caller_records(vault: &Path, caller: &Caller) -> Vec<DelegationRecord> {
    let mut records = crate::delegation::for_run(vault, &caller.run_id);
    if caller.surface == "chat" {
        let since = chrono::Utc::now() - chrono::Duration::days(MINE_DAYS);
        records.extend(
            crate::session_store::child_delegations_for_session(
                vault,
                &caller.scope,
                since,
                CHAIN_SESSIONS,
            )
            .unwrap_or_default()
            .iter()
            .filter_map(|json| serde_json::from_str::<DelegationRecord>(json).ok()),
        );
    }
    let mut records = once_per_turn(records);
    records.sort_by_key(|r| std::cmp::Reverse(r.child_queue_id));
    records
}

/// The conversations `caller` started ([`caller_records`]), each once with
/// its newest turn, newest first, each followed by its handover successors
/// in order (at most [`CHAIN_HOPS`]). A caller that handed over lists its
/// own successor too: that handover is one of its child tasks (intended).
/// Stops once it has more than [`MINE_ROWS`], so the walk stays bounded.
fn mine(vault: &Path, caller: &Caller) -> Vec<Mine> {
    let mut seen = HashSet::new();
    if caller.surface == "chat" {
        seen.insert(caller.scope.clone());
    }
    let mut out = Vec::new();
    for record in caller_records(vault, caller) {
        if out.len() > MINE_ROWS {
            break;
        }
        let Some(key) = record.child_session.clone() else {
            continue;
        };
        if !seen.insert(key.clone()) {
            continue;
        }
        let mut row = child_row(vault, &record);
        let mut from = key.clone();
        out.push(Mine {
            key,
            agent: record.agent.clone(),
            row: row.clone(),
            from: None,
        });
        for _ in 0..CHAIN_HOPS {
            if out.len() > MINE_ROWS {
                break;
            }
            let Some(handover) = row.as_ref().and_then(|r| handover_record(vault, r)) else {
                break;
            };
            let Some(next) = handover.child_session.clone() else {
                break;
            };
            if !seen.insert(next.clone()) {
                break;
            }
            row = child_row(vault, &handover);
            out.push(Mine {
                key: next.clone(),
                agent: handover.agent.clone(),
                row: row.clone(),
                from: Some(std::mem::replace(&mut from, next)),
            });
        }
    }
    out
}

/// `list_sessions` `mine` (overnight OB21): the conversations `caller`
/// started and their handover successors ([`mine`]), at most
/// [`MINE_ROWS`] (`more: true` when there are others), each with its
/// title, workspace, agent and how its turn stands (the run queue's
/// status terms); once that turn ended, its cost when known and what it
/// left ([`child_summary`]). No other conversation and no run queue. Not
/// redacted here: `sessions.overview` redacts the whole answer.
pub(crate) fn mine_overview(vault: &Path, caller: &Caller) -> Value {
    let mut all = mine(vault, caller);
    let more = all.len() > MINE_ROWS;
    all.truncate(MINE_ROWS);
    let homes = crate::workspaces::SessionHomes::read(vault).ok();
    let rows: Vec<Value> = all
        .into_iter()
        .map(|m| mine_row(vault, homes.as_ref(), m))
        .collect();
    let mut answer = json!({ "sessions": rows });
    if more {
        answer["more"] = json!(true);
    }
    answer
}

fn mine_row(vault: &Path, homes: Option<&crate::workspaces::SessionHomes>, m: Mine) -> Value {
    let session = crate::session_store::session_by_key(vault, &m.key)
        .ok()
        .flatten();
    let workspace = session
        .as_ref()
        .zip(homes)
        .and_then(|(s, h)| h.workspace_for(&s.cwd));
    let mut row = json!({
        "key": m.key,
        "title": session.and_then(|s| s.title),
        "workspace": workspace,
        "agent": m.agent,
        "status": m.row.as_ref().map(|r| crate::run_pump::agent_status(vault, r)),
    });
    if let Some(from) = m.from {
        row["handoverFrom"] = json!(from);
    }
    let Some(turn) = m.row else {
        return row;
    };
    if let Some(at) = &turn.claimed_at {
        row["startedAt"] = json!(at);
    }
    if turn.status.is_live() {
        return row;
    }
    if let Some(at) = &turn.finished_at {
        row["finishedAt"] = json!(at);
    }
    let cost = match &turn.history_run_id {
        Some(run) => rollup(vault, run).total_usd,
        None => Some(0.0),
    };
    if let Some(cost) = cost {
        row["costUsd"] = json!(cost);
    }
    if turn.status != RowStatus::Done {
        if let Some(error) = turn.error.as_deref().filter(|e| !e.trim().is_empty()) {
            row["error"] = json!(one_line(error.trim(), NOTE_ERROR_CHARS));
        }
    }
    let summary = child_summary(vault, &m.key, &turn).text(MINE_ANSWER_CHARS);
    let summary = summary.trim_start_matches("; ").trim();
    if !summary.is_empty() {
        row["summary"] = json!(summary);
    }
    row
}

/// Whether conversation `key`'s newest turn is one `caller` caused (a
/// child task of its, [`caller_records`], or a handover successor's turn
/// down such a chain) and has ended (overnight OB21: `read_session` then
/// defaults to its last answer). A conversation that later ran another
/// turn, queued, running or ended, is not: its last answer isn't the
/// child's. Nor is a turn that never ran or never stored a turn (cancelled
/// while queued, failed before it started): the conversation's latest
/// stored turn must be the one that row ran as, or its last answer is an
/// earlier turn's. The caller's own records are checked first; the chains
/// are walked only when none of them is that turn, and only from the
/// newest [`MINE_ROWS`] records, so a long-lived chat's 90 days of child
/// tasks aren't each walked. It also follows chains from older delegated
/// turns of the same conversation, which [`mine`] (one row per
/// conversation, its newest turn) doesn't list.
pub(crate) fn is_ended_child(vault: &Path, caller: &Caller, key: &str) -> bool {
    let key = key.trim();
    if caller.surface == "chat" && key == caller.scope {
        return false;
    }
    let newest = crate::run_queue::newest_chat_row(vault, key);
    let Some(newest) = newest.ok().flatten().filter(|r| !r.status.is_live()) else {
        return false;
    };
    let Some(run) = newest.history_run_id.as_deref() else {
        return false;
    };
    let latest = crate::session_store::latest_turn_run(vault, key);
    if latest.ok().flatten().as_deref() != Some(run) {
        return false;
    }
    let caused = |record: &DelegationRecord| {
        record.child_session.as_deref() == Some(key) && record.child_queue_id == Some(newest.id)
    };
    let records = caller_records(vault, caller);
    if records.iter().any(caused) {
        return true;
    }
    let mut seen = HashSet::new();
    for record in records.iter().take(MINE_ROWS) {
        let mut row = child_row(vault, record);
        for _ in 0..CHAIN_HOPS {
            let Some(handover) = row.as_ref().and_then(|r| handover_record(vault, r)) else {
                break;
            };
            if caused(&handover) {
                return true;
            }
            if !handover.child_queue_id.is_some_and(|id| seen.insert(id)) {
                break;
            }
            row = child_row(vault, &handover);
        }
    }
    false
}

// ── Waking a parent when its children end (overnight OB10b) ───────────────

/// For each of `keys`, whether it is a conversation `caller` started (a
/// child task of its, or a handover successor down such a chain, as
/// `list_sessions` `mine` lists it, [`mine`]) and the turn of it the caller
/// caused: `(agent, queue row, direct)`, `direct` when it is the caller's
/// own child task. `None` for any other conversation (the caller's own
/// included), or one whose turn the queue pruned.
pub(crate) fn my_turns(
    vault: &Path,
    caller: &Caller,
    keys: &[String],
) -> Vec<Option<(String, i64, bool)>> {
    let all = mine(vault, caller);
    keys.iter()
        .map(|key| {
            let m = all.iter().find(|m| &m.key == key)?;
            let row = m.row.as_ref()?;
            Some((m.agent.clone(), row.id, m.from.is_none()))
        })
        .collect()
}

/// Where the chain from turn `queue_id` of conversation `key` stands
/// (overnight OB10b): that turn, or, once it ended handing over, its
/// successor's turn, and so on ([`chain_end`]).
pub(crate) struct ChainEnd {
    /// The conversation whose turn `row` is (`key`, or a successor).
    pub session: String,
    pub agent: String,
    pub row: QueueRow,
    /// The successors' turns passed on the way (their queue rows).
    pub successors: Vec<i64>,
}

/// The turn the chain from conversation `key`'s turn `queue_id` stands at:
/// that turn while it runs or once it ended without handing over; else the
/// successor's, at most [`CHAIN_HOPS`] on (a withdrawn successor ends the
/// chain where it was handed over). `None` once the queue pruned the turn
/// or the row isn't `key`'s chat turn.
pub(crate) fn chain_end(vault: &Path, key: &str, agent: &str, queue_id: i64) -> Option<ChainEnd> {
    let row = crate::run_queue::get(vault, queue_id)
        .ok()
        .flatten()
        .filter(|r| r.source == RunSource::Chat && r.source_id.as_deref() == Some(key))?;
    let mut end = ChainEnd {
        session: key.to_string(),
        agent: agent.to_string(),
        row,
        successors: Vec::new(),
    };
    for _ in 0..CHAIN_HOPS {
        if end.row.status.is_live() {
            break;
        }
        let Some(handover) = handover_record(vault, &end.row) else {
            break;
        };
        let (Some(next), Some(session)) = (child_row(vault, &handover), handover.child_session)
        else {
            break;
        };
        if next.status == RowStatus::Cancelled || end.successors.contains(&next.id) {
            break;
        }
        end.session = session;
        end.agent = handover.agent;
        end.successors.push(next.id);
        end.row = next;
    }
    Some(end)
}

/// One ended child's line in a woken parent's message (overnight OB10b):
/// the note's line ([`note_line`]) for the turn its chain ended at, then,
/// when `summarized`, what that turn left ([`child_summary`], `answer_chars`
/// of its answer). `<` is escaped as `‹`.
pub(crate) fn ended_line(
    vault: &Path,
    end: &ChainEnd,
    key: &str,
    summarized: bool,
    answer_chars: usize,
) -> String {
    let mut line = note_line(vault, &end.session, &end.agent, &end.row);
    if end.session != key {
        line.push_str(&format!(" (where {key}'s handovers ended)"));
    }
    if summarized {
        line.push_str(&child_summary(vault, &end.session, &end.row).text(answer_chars));
    }
    line.replace('<', "‹")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn usage(vault: &Path, key: &str, surface: &str, run: &str, cost: Option<f64>, outcome: &str) {
        let mut row =
            crate::session_store::tests::usage_row_fixture(key, "2026-10-07 10:00:00", outcome);
        row.surface = surface.into();
        row.run_id = Some(run.into());
        row.cost_usd = cost;
        row.agent = Some("claude".into());
        if surface == "oneshot" {
            row.scope = Some("learning_extract".into());
        }
        crate::session_store::upsert_usage(vault, &row).unwrap();
    }

    #[test]
    fn b6review_later_roots_still_count_after_an_earlier_root_hit_its_bound() {
        let dir = tempfile::tempdir().unwrap();
        usage(
            dir.path(),
            "b6-later",
            "chat",
            "R-b6-later",
            Some(1.5),
            "success",
        );
        let mut seen = (0..CHAIN_SESSIONS)
            .map(|i| format!("earlier-{i}"))
            .collect();
        assert_eq!(
            recorded_chain_cost(dir.path(), "R-b6-later", "2000-01-01 00:00:00", &mut seen),
            1.5
        );
    }

    #[test]
    fn recorded_chain_cost_keeps_partial_spend_and_deduplicates_links_and_cycles() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let root = child_turn(v, "s-b6-root", "R-b6-root", RowStatus::Done);
        let child = child_turn(v, "s-b6-child", "R-b6-child", RowStatus::Running);
        record_child(v, "R-b6-root", "s-b6-child", child);
        record_child(v, "R-b6-root", "s-b6-child", child);
        record_child(v, "R-b6-child", "s-b6-root", root);
        usage(v, "b6-root", "chat", "R-b6-root", Some(2.25), "success");
        usage(v, "b6-child", "chat", "R-b6-child", None, "running");
        usage(
            v,
            "b6-helper",
            "delegate",
            "R-b6-child",
            Some(0.375),
            "success",
        );
        let mut seen = HashSet::new();
        assert_eq!(
            recorded_chain_cost(v, "R-b6-root", "2000-01-01 00:00:00", &mut seen),
            2.625
        );
        assert_eq!(
            recorded_chain_cost(v, "R-b6-child", "2000-01-01 00:00:00", &mut seen),
            0.0
        );
        assert_eq!(rollup(v, "R-b6-root").total_usd, None);
        // A later report replaces the same usage row, never adds a second
        // charge for it; every cap check takes a fresh ledger snapshot.
        usage(v, "b6-child", "chat", "R-b6-child", Some(1.125), "running");
        assert_eq!(
            recorded_chain_cost(v, "R-b6-root", "2000-01-01 00:00:00", &mut HashSet::new()),
            3.75
        );
    }

    #[test]
    fn totals_sum_what_is_known_and_keep_null_for_the_unknown() {
        let d = |cost: Option<f64>| Descendant {
            kind: "consultation".into(),
            status: "success".into(),
            cost_usd: cost,
            ..Descendant::default()
        };
        let known = totals(vec![Some(0.5)], vec![d(Some(0.25)), d(Some(0.0))]);
        assert_eq!(known.own_usd, Some(0.5));
        assert_eq!(known.descendants_usd, Some(0.25));
        assert_eq!(known.total_usd, Some(0.75));
        assert_eq!(known.known_usd, 0.75);
        assert_eq!(known.unknown, 0);
        let unknown = totals(vec![Some(0.5), None], vec![d(Some(0.25)), d(None)]);
        assert_eq!(
            unknown.own_usd, None,
            "an attempt with no cost leaves its run's own null"
        );
        assert_eq!(unknown.descendants_usd, None);
        assert_eq!(
            unknown.total_usd, None,
            "never a partial sum passed off as the total"
        );
        assert_eq!(unknown.known_usd, 0.75, "the known parts still add up");
        assert_eq!(unknown.unknown, 2);
        let alone = totals(vec![Some(0.1)], Vec::new());
        assert_eq!(alone.descendants_usd, Some(0.0));
        assert_eq!(alone.total_usd, Some(0.1));
        assert_eq!(
            totals(Vec::new(), Vec::new()).own_usd,
            None,
            "no row is not 0"
        );
    }

    #[test]
    fn a_run_rolls_up_its_consultations_helpers_and_children_once() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        // The parent's own row, a consultation and a helper one-shot.
        usage(v, "k-own", "chat", "R-parent", Some(0.5), "success");
        usage(v, "k-team", "delegate", "R-parent", Some(0.25), "cancelled");
        usage(v, "k-help", "oneshot", "R-parent", None, "success");
        // A child that ran (with a consultation of its own) and one that
        // was cancelled while queued.
        let ran = child_turn(v, "s-child-ran", "R-child", RowStatus::Done);
        usage(v, "k-child", "chat", "R-child", Some(1.0), "success");
        usage(v, "k-grand", "delegate", "R-child", Some(0.125), "success");
        let queued = child_turn(v, "s-child-queued", "", RowStatus::Cancelled);
        record_child(v, "R-parent", "s-child-ran", ran);
        record_child(v, "R-parent", "s-child-ran", ran);
        record_child(v, "R-parent", "s-child-queued", queued);
        let rollup = rollup(v, "R-parent");
        assert_eq!(rollup.own_usd, Some(0.5));
        let kinds: Vec<(&str, &str, Option<f64>)> = rollup
            .descendants
            .iter()
            .map(|d| (d.kind.as_str(), d.status.as_str(), d.cost_usd))
            .collect();
        assert_eq!(
            kinds,
            [
                ("consultation", "cancelled", Some(0.25)),
                ("helper", "success", None),
                ("childTask", "done", Some(1.125)),
                ("childTask", "cancelled", Some(0.0)),
            ],
            "the child's consultation is inside its cost, a recorded turn counts once"
        );
        assert_eq!(
            rollup.descendants[1].purpose.as_deref(),
            Some("learning_extract")
        );
        assert_eq!(
            rollup.descendants[2].child_run_id.as_deref(),
            Some("R-child")
        );
        assert_eq!(rollup.descendants_usd, None, "the helper's cost is unknown");
        assert_eq!(rollup.total_usd, None);
        assert_eq!(rollup.known_usd, 0.5 + 0.25 + 1.125);
        assert_eq!(rollup.unknown, 1);
        // The child's own roll-up has its consultation, not its parent's.
        let child = super::rollup(v, "R-child");
        assert_eq!(child.total_usd, Some(1.125));
        assert_eq!(child.descendants.len(), 1);
    }

    /// Q8 review: a self-hosted model's rows are GPU time: no known spend
    /// and not unknown, so the night budget neither charges them nor fails
    /// closed on them, and no sum with one is ever $0.
    #[test]
    fn gpu_time_is_neither_spend_nor_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let gpu = |key: &str, surface: &str, run: &str| {
            let mut row = crate::session_store::tests::usage_row_fixture(
                key,
                "2026-10-07 10:00:00",
                "success",
            );
            row.surface = surface.into();
            row.run_id = Some(run.into());
            row.billing = crate::usage::Billing::SelfHosted.as_str().into();
            row.cost_status = "unpriced".into();
            crate::session_store::upsert_usage(v, &row).unwrap();
        };
        gpu("k-qwen", "chat", "R-qwen");
        let alone = rollup(v, "R-qwen");
        assert_eq!((alone.own_usd, alone.total_usd), (None, None), "never $0");
        assert_eq!(
            (alone.known_usd, alone.unknown, alone.gpu_time),
            (0.0, 0, 1)
        );
        // A Claude turn with a Qwen consultation and a Qwen child task whose
        // own helper was priced.
        usage(v, "k-parent", "chat", "R-parent", Some(0.5), "success");
        gpu("k-team", "delegate", "R-parent");
        let ran = child_turn(v, "s-qwen-child", "R-qwen-child", RowStatus::Done);
        gpu("k-child", "chat", "R-qwen-child");
        usage(
            v,
            "k-help",
            "oneshot",
            "R-qwen-child",
            Some(0.25),
            "success",
        );
        record_child(v, "R-parent", "s-qwen-child", ran);
        let parent = rollup(v, "R-parent");
        assert_eq!(parent.own_usd, Some(0.5));
        assert_eq!(parent.total_usd, None, "GPU time is never a dollar sum");
        assert_eq!(parent.known_usd, 0.75, "the child's priced helper counts");
        assert_eq!((parent.unknown, parent.gpu_time), (0, 2));
        // A cost nobody reported beside GPU time is still unknown.
        usage(v, "k-qwen-help", "oneshot", "R-qwen", None, "success");
        let mixed = rollup(v, "R-qwen");
        assert_eq!((mixed.unknown, mixed.gpu_time), (1, 1));
        // Q8 second review: a self-hosted row cut short (partial) or with no
        // usable figures (unknown) is still GPU time, never unknown.
        for (run, status) in [("R-qwen-partial", "partial"), ("R-qwen-unknown", "unknown")] {
            let mut row = crate::session_store::tests::usage_row_fixture(
                run,
                "2026-10-07 10:00:00",
                "cancelled",
            );
            row.run_id = Some(run.into());
            row.billing = crate::usage::Billing::SelfHosted.as_str().into();
            row.cost_status = status.into();
            crate::session_store::upsert_usage(v, &row).unwrap();
            let r = rollup(v, run);
            assert_eq!(
                (r.total_usd, r.unknown, r.gpu_time),
                (None, 0, 1),
                "{status}"
            );
        }
    }

    fn caller(run: &str, scope: &str) -> Caller {
        Caller {
            run_id: run.into(),
            surface: "chat".into(),
            scope: scope.into(),
            access: crate::app_mcp::AppAccess::Write,
            depth: 0,
            perm: "acceptEdits".into(),
        }
    }

    fn block_on<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(f)
    }

    /// F10 (D2): a wait on a child that ended answers at once and reports
    /// it once; a second wait says it was reported, so the asking agent
    /// doesn't redo the follow-up. A wait never reaches the asking
    /// conversation itself.
    #[test]
    fn a_wait_reports_an_ended_child_once() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = caller("R-wait-parent", "s-wait-parent");
        let id = child_turn(v, "s-wait-child", "R-wait-child", RowStatus::Done);
        usage(
            v,
            "k-wait-child",
            "chat",
            "R-wait-child",
            Some(0.5),
            "success",
        );
        record_child(v, "R-wait-parent", "s-wait-child", id);
        let first = block_on(wait_with(v, &parent, &["s-wait-child"], 60, || true)).unwrap();
        assert_eq!(first["finished"], json!(true));
        assert_eq!(first["status"], json!("done"));
        assert_eq!(first["runId"], json!("R-wait-child"));
        assert_eq!(first["costUsd"], json!(0.5));
        assert_eq!(first["reportedBefore"], json!(false));
        assert_eq!(first["note"], json!(FIRST_REPORT));
        let again = block_on(wait_with(v, &parent, &["s-wait-child"], 60, || true)).unwrap();
        assert_eq!(again["reportedBefore"], json!(true));
        assert_eq!(again["note"], json!(ALREADY_REPORTED));
        let delivery = crate::session_store::child_delivery(v, id)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.via, "wait");
        // Another conversation's session is answered, never marked.
        let other = child_turn(v, "s-wait-other", "", RowStatus::Cancelled);
        let answer = block_on(wait_with(v, &parent, &["s-wait-other"], 60, || true)).unwrap();
        assert_eq!(answer["status"], json!("cancelled"));
        assert_eq!(
            answer["costUsd"],
            json!(0.0),
            "a turn that never ran cost nothing"
        );
        assert!(answer.get("reportedBefore").is_none());
        assert_eq!(
            crate::session_store::child_delivery(v, other).unwrap(),
            None
        );
        let own = block_on(wait_with(v, &parent, &["s-wait-parent"], 60, || true)).unwrap_err();
        assert!(own.contains("the conversation you are running in"), "{own}");
        let none = block_on(wait_with(v, &parent, &["s-wait-nothing"], 60, || true)).unwrap_err();
        assert!(
            none.contains("no turn queued, running or recently finished"),
            "{none}"
        );
    }

    /// Overnight OB10a: a wait on several keys returns as soon as any one
    /// ends, names it and reports only it; the others are answered as still
    /// running.
    #[test]
    fn a_wait_on_several_keys_returns_when_any_one_ends() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = caller("R-any-parent", "s-any-parent");
        let a = child_turn(v, "s-any-a", "R-any-a", RowStatus::Running);
        let b = child_turn(v, "s-any-b", "R-any-b", RowStatus::Running);
        record_child(v, "R-any-parent", "s-any-a", a);
        record_child(v, "R-any-parent", "s-any-b", b);
        let vault = v.to_path_buf();
        let ender = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            let now = chrono::Utc::now();
            crate::run_queue::finish(&vault, b, RowStatus::Done, None, now).unwrap();
        });
        let started = Instant::now();
        let keys = ["s-any-a", "s-any-b"];
        let answer = block_on(wait_with(v, &parent, &keys, 10, || true)).unwrap();
        let took = started.elapsed();
        ender.join().unwrap();
        assert!(took >= Duration::from_millis(500), "{took:?}");
        assert!(took < Duration::from_secs(5), "{took:?}");
        assert_eq!(answer["finished"], json!(true));
        assert_eq!(answer["ended"], json!(["s-any-b"]));
        let sessions = answer["sessions"].as_array().unwrap();
        assert_eq!(sessions[0]["session"], json!("s-any-b"));
        assert_eq!(sessions[0]["status"], json!("done"));
        assert_eq!(sessions[0]["reportedBefore"], json!(false));
        assert_eq!(sessions[1]["session"], json!("s-any-a"));
        assert_eq!(sessions[1]["finished"], json!(false));
        assert!(sessions[1]["runningSeconds"].is_u64(), "{answer}");
        assert!(crate::session_store::child_delivery(v, b)
            .unwrap()
            .is_some());
        assert_eq!(crate::session_store::child_delivery(v, a).unwrap(), None);
        // Asked again, the ending already reported doesn't answer at once:
        // the wait waits for the one still running, and lists it last.
        let started = Instant::now();
        let again = block_on(wait_with(v, &parent, &keys, 1, || true)).unwrap();
        let took = started.elapsed();
        assert!(took >= Duration::from_secs(1), "{took:?}");
        assert_eq!(again["finished"], json!(false));
        assert!(again.get("ended").is_none(), "{again}");
        let sessions = again["sessions"].as_array().unwrap();
        assert_eq!(sessions[0]["session"], json!("s-any-a"));
        assert_eq!(sessions[1]["session"], json!("s-any-b"));
        assert_eq!(sessions[1]["reportedBefore"], json!(true));
        // Once every one has ended, it answers at once, as before.
        let now = chrono::Utc::now();
        crate::run_queue::finish(v, a, RowStatus::Done, None, now).unwrap();
        let started = Instant::now();
        let last = block_on(wait_with(v, &parent, &keys, 60, || true)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(20), "not the 60 s");
        assert_eq!(last["finished"], json!(true));
        assert_eq!(last["ended"], json!(["s-any-a", "s-any-b"]));
    }

    /// Overnight OB10a: exactly one of key and keys (a non-empty list, each
    /// key once, at most 12), and every key is checked as a lone key is: one
    /// the caller may not wait on refuses the whole wait.
    #[test]
    fn a_wait_takes_key_or_keys_and_checks_each_like_one() {
        let list = |keys: &[&str]| -> Option<Vec<String>> {
            Some(keys.iter().map(|k| k.to_string()).collect())
        };
        assert_eq!(
            wait_keys(Some("s-a".into()), list(&["s-b"])).unwrap_err(),
            KEY_AND_KEYS
        );
        assert_eq!(wait_keys(None, None).unwrap_err(), NO_KEYS);
        assert_eq!(wait_keys(None, list(&[])).unwrap_err(), NO_KEYS);
        assert_eq!(
            wait_keys(None, list(&["s-a", "   "])).unwrap_err(),
            EMPTY_KEY
        );
        assert_eq!(
            wait_keys(None, list(&[" s-a ", "s-b", "s-a"])).unwrap(),
            ["s-a", "s-b"]
        );
        assert_eq!(wait_keys(Some("s-a ".into()), None).unwrap(), ["s-a"]);
        let many: Vec<String> = (0..=WAIT_MAX_KEYS).map(|i| format!("s-{i}")).collect();
        assert!(wait_keys(None, Some(many))
            .unwrap_err()
            .contains("at most 12"));

        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = caller("R-keys-parent", "s-keys-parent");
        child_turn(v, "s-keys-child", "R-keys-child", RowStatus::Done);
        let refused =
            |keys: [&str; 2]| block_on(wait_with(v, &parent, &keys, 60, || true)).unwrap_err();
        let own = refused(["s-keys-child", "s-keys-parent"]);
        assert!(own.contains("the conversation you are running in"), "{own}");
        let bad = refused(["s-keys-child", "../etc"]);
        assert!(bad.contains("is not a session key"), "{bad}");
        let gone = refused(["s-keys-child", "s-keys-nothing"]);
        assert!(gone.contains("no turn queued, running"), "{gone}");
        let none: [&str; 0] = [];
        assert_eq!(
            block_on(wait_with(v, &parent, &none, 60, || true)).unwrap_err(),
            NO_KEYS
        );
        // Agents see the cap; a key that isn't a string is refused.
        let wait = crate::app_actions::find("sessions.wait").unwrap();
        let schema = crate::app_actions::input_schema(wait.params);
        let keys = &schema["properties"]["keys"];
        assert_eq!(keys["maxItems"], json!(WAIT_MAX_KEYS));
        assert_eq!(keys["items"]["type"], json!("string"));
        let mixed = json!({ "keys": ["s-a", 1] });
        assert!(crate::app_actions::validate(wait, &mixed).is_err());
    }

    /// Parallel chats in one workspace (2026-10-07): a child queued for the
    /// build worktree its caller's turn holds alone is answered at once; one
    /// queued for a workspace folder the caller shares is simply waited for.
    #[test]
    fn a_wait_on_a_child_behind_the_callers_own_worktree_answers_at_once() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let workspace = crate::workspaces::create(v, "Build", "Ship it").unwrap();
        let tree = tempfile::Builder::new()
            .prefix("worktree-")
            .tempdir()
            .unwrap();
        crate::build_workspaces::bind_for_test(v, &workspace.id, tree.path());
        let queue = |key: &str, cwd: &Path| {
            let row = crate::run_queue::NewRow::new(RunSource::Chat, Some(key), "chat", "test")
                .with_payload(json!({ "cwd": cwd.to_string_lossy() }));
            crate::run_queue::enqueue(v, row, chrono::Utc::now()).unwrap();
        };
        let held =
            crate::automated_history::try_begin_scoped(v, &workspace.path, "chat", None).unwrap();
        let parent = caller(held.id(), "s-wt-parent");
        queue("s-wt-child", tree.path());
        let started = Instant::now();
        let answer = block_on(wait_with(v, &parent, &["s-wt-child"], 30, || true)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(15), "not the 30 s");
        assert_eq!(answer["note"], json!(BEHIND_THIS_WORKTREE));
        held.finish("success").unwrap();

        let shared =
            crate::automated_history::try_begin_scoped_shared(v, &workspace.path, "chat", None)
                .unwrap();
        let parent = caller(shared.id(), "s-ws-parent");
        queue("s-ws-child", &v.join(&workspace.path));
        let answer = block_on(wait_with(v, &parent, &["s-ws-child"], 1, || true)).unwrap();
        let note = answer["note"].as_str().unwrap();
        assert!(
            note.starts_with("Not finished yet"),
            "it isn't behind the caller"
        );
        shared.finish("success").unwrap();
    }

    /// 2026-10-20 review: a child held only because its caller's own turn
    /// fills a chat limit (its workspace's, or the app-wide one) can't start
    /// before that turn ends, so a wait on it answers at once and says so;
    /// with another turn that could free a slot, it is simply waited for.
    #[test]
    fn a_wait_on_a_child_behind_the_callers_own_chat_slot_answers_at_once() {
        use crate::chat_limits::{set_test_limits, ChatLimits};
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let a = crate::workspaces::create(v, "Slot A", "Do it").unwrap();
        let b = crate::workspaces::create(v, "Slot B", "Do it").unwrap();
        let queue = |key: &str, cwd: &Path| {
            let row = crate::run_queue::NewRow::new(RunSource::Chat, Some(key), "chat", "test")
                .with_payload(json!({ "cwd": cwd.to_string_lossy() }));
            crate::run_queue::enqueue(v, row, chrono::Utc::now()).unwrap();
        };
        let limits = |chats_at_once, chats_per_workspace| ChatLimits {
            chats_at_once,
            chats_per_workspace,
            ..Default::default()
        };
        let shared = |path: &str| {
            crate::automated_history::try_begin_scoped_shared(v, path, "chat", None).unwrap()
        };

        // One chat per workspace: the child in the caller's own folder.
        set_test_limits(limits(10, 1));
        let held = shared(&a.path);
        let parent = caller(held.id(), "s-slot-parent");
        queue("s-slot-child", &v.join(&a.path));
        let started = Instant::now();
        let answer = block_on(wait_with(v, &parent, &["s-slot-child"], 30, || true)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(15), "not the 30 s");
        assert_eq!(answer["note"], json!(BEHIND_THIS_TURNS_SLOT));

        // One chat at once: the child in another workspace.
        set_test_limits(limits(1, 5));
        queue("s-app-child", &v.join(&b.path));
        let started = Instant::now();
        let answer = block_on(wait_with(v, &parent, &["s-app-child"], 30, || true)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(15), "not the 30 s");
        assert_eq!(answer["note"], json!(BEHIND_THIS_TURNS_SLOT));

        // A Run now waiting for the free slot changes nothing: there is none
        // while the caller runs (delta review).
        let manual = crate::run_queue::NewRow::new(
            RunSource::Manual,
            Some("brief"),
            "Morning brief",
            "run now",
        );
        crate::run_queue::enqueue(v, manual, chrono::Utc::now()).unwrap();
        let started = Instant::now();
        let answer = block_on(wait_with(v, &parent, &["s-app-child"], 30, || true)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(15), "not the 30 s");
        assert_eq!(answer["note"], json!(BEHIND_THIS_TURNS_SLOT));

        // Two at once, and another turn running: its end frees a slot too.
        set_test_limits(limits(2, 5));
        let other = shared(&b.path);
        let answer = block_on(wait_with(v, &parent, &["s-app-child"], 1, || true)).unwrap();
        let note = answer["note"].as_str().unwrap();
        assert!(note.starts_with("Not finished yet"), "{note}");
        other.finish("success").unwrap();
        held.finish("success").unwrap();
    }

    /// F10 (D2): a wait is bounded: a turn still running is answered at the
    /// deadline (never later), and at once when the waiting run has ended.
    #[test]
    fn a_wait_ends_at_its_deadline_or_with_its_caller() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = caller("R-bound-parent", "s-bound-parent");
        let id = child_turn(v, "s-bound-child", "R-bound-child", RowStatus::Running);
        record_child(v, "R-bound-parent", "s-bound-child", id);
        let started = Instant::now();
        let answer = block_on(wait_with(v, &parent, &["s-bound-child"], 1, || true)).unwrap();
        let took = started.elapsed();
        assert_eq!(answer["finished"], json!(false));
        assert_eq!(answer["status"], json!("running"));
        let note = answer["note"].as_str().unwrap();
        assert!(note.ends_with(STILL_RUNNING), "{note}");
        assert!(took >= Duration::from_secs(1), "{took:?}");
        // The answer above is the deadline's (still running); this only
        // rules out waiting on far past it, with room for a loaded gate.
        assert!(took < Duration::from_secs(10), "{took:?}");
        let started = Instant::now();
        let gone = block_on(wait_with(v, &parent, &["s-bound-child"], 300, || false)).unwrap();
        assert_eq!(gone["finished"], json!(false));
        assert!(started.elapsed() < Duration::from_secs(30), "not the 300 s");
        // The longest wait is the table's.
        assert!(crate::delegation::WAITING_TOOLS
            .iter()
            .any(|(_, tool, longest)| *tool == "wait_for_session"
                && *longest > crate::delegation::WAIT_MAX_SECONDS));
    }

    /// Overnight OB10a: a wait that comes back unfinished says how long
    /// each turn has run (or queued), and that a chat hears of its child's
    /// end in its next turn's note, so it can end its turn instead of
    /// waiting again; a wait on another conversation's turn, or from an
    /// automation run, which has no next turn, isn't told that. A Main agent
    /// thread isn't offered a wake on its children's ends (OB10b).
    #[test]
    fn a_timed_out_wait_says_how_long_each_ran_and_to_end_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = caller("R-long-parent", "s-long-parent");
        let now = chrono::Utc::now();
        let ran = now - chrono::Duration::minutes(5);
        let row =
            |key: &str| crate::run_queue::NewRow::new(RunSource::Chat, Some(key), "chat", "test");
        let long = crate::run_queue::enqueue(v, row("s-long-a"), ran)
            .unwrap()
            .id();
        crate::run_queue::claim(v, long, ran).unwrap();
        crate::run_queue::mark_running(v, long, "R-long-a", ran).unwrap();
        let queued = now - chrono::Duration::seconds(90);
        let b = crate::run_queue::enqueue(v, row("s-long-b"), queued)
            .unwrap()
            .id();
        record_child(v, "R-long-parent", "s-long-a", long);
        record_child(v, "R-long-parent", "s-long-b", b);
        let keys = ["s-long-a", "s-long-b"];
        // A caller that has ended answers at once, as the deadline does.
        let answer = block_on(wait_with(v, &parent, &keys, 1, || false)).unwrap();
        assert_eq!(answer["finished"], json!(false));
        assert!(answer.get("ended").is_none(), "{answer}");
        let sessions = answer["sessions"].as_array().unwrap();
        assert!(sessions[1]["queuedSeconds"].is_u64(), "{answer}");
        let note = answer["note"].as_str().unwrap();
        assert!(
            note.starts_with("Not finished yet: s-long-a running for 5m "),
            "{note}"
        );
        assert!(note.contains(", s-long-b queued for 1m "), "{note}");
        assert!(note.contains(NOTE_OPEN), "{note}");
        assert!(note.contains("end the turn"), "{note}");
        assert!(note.contains("run_action sessions.wake_me"), "{note}");
        // Another conversation's turn is never noted to this one.
        crate::run_queue::enqueue(v, row("s-long-c"), queued).unwrap();
        let mixed = ["s-long-a", "s-long-c"];
        let answer = block_on(wait_with(v, &parent, &mixed, 1, || false)).unwrap();
        let note = answer["note"].as_str().unwrap();
        assert!(note.ends_with(STILL_RUNNING_RUN), "{note}");
        // One key keeps its own answer's shape.
        let one = block_on(wait_with(v, &parent, &["s-long-a"], 1, || false)).unwrap();
        assert_eq!(one["session"], json!("s-long-a"));
        assert!(
            one["runningSeconds"].as_u64().is_some_and(|s| s >= 300),
            "{one}"
        );
        assert!(one["note"].as_str().unwrap().contains(NOTE_OPEN));
        assert!(
            one["note"].as_str().unwrap().ends_with(STILL_RUNNING),
            "{one}"
        );
        // A Main agent thread is woken for its chats already and can't ask
        // for a wake on their ends: it keeps OB10a's advice (OB10b review).
        crate::workspaces::ensure_main_agent(v).unwrap();
        let main = crate::workspaces::main_agent_cwd(v).unwrap();
        crate::session_store::start_turn(v, "s-long-main", "claude", &main, "R-long-main", "go")
            .unwrap();
        record_child(v, "R-long-main", "s-long-a", long);
        let thread = caller("R-long-main", "s-long-main");
        let mains = block_on(wait_with(v, &thread, &["s-long-a"], 1, || false)).unwrap();
        let note = mains["note"].as_str().unwrap();
        assert!(note.ends_with(STILL_RUNNING_MAIN), "{note}");
        assert!(!note.contains("onSessionsEnd"), "{note}");
        let automation = Caller {
            surface: "workflow".into(),
            ..caller("R-long-auto", "nightly")
        };
        let answer = block_on(wait_with(v, &automation, &keys, 1, || false)).unwrap();
        let note = answer["note"].as_str().unwrap();
        assert!(note.ends_with(STILL_RUNNING_RUN), "{note}");
        assert!(!note.contains(NOTE_OPEN), "{note}");
    }

    /// F10 (D2): the next turn's note lists each child turn that ended,
    /// once: a retry of that same turn carries it again, a later turn
    /// doesn't, one a wait already reported never gets a note, and one still
    /// running waits for its own.
    #[test]
    fn a_childs_end_is_noted_once_in_the_next_turn() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-note-parent";
        let ord =
            crate::session_store::start_turn(v, parent, "claude", "/v", "R-note-1", "go").unwrap();
        let done = child_turn(v, "s-note-done", "R-note-done", RowStatus::Done);
        let failed = child_turn(v, "s-note-failed", "R-note-failed", RowStatus::Failed);
        let running = child_turn(v, "s-note-running", "R-note-running", RowStatus::Running);
        let waited = child_turn(v, "s-note-waited", "", RowStatus::Cancelled);
        for (key, id) in [
            ("s-note-done", done),
            ("s-note-failed", failed),
            ("s-note-running", running),
            ("s-note-waited", waited),
        ] {
            record_child(v, "R-note-1", key, id);
        }
        crate::session_store::deliver_child(v, waited, parent, "s-note-waited", "wait", None)
            .unwrap();
        // Not in the turn that started them.
        assert_eq!(notes_for_turn(v, parent, ord), Notes::default());
        let next = ord + 1;
        let notes = notes_for_turn(v, parent, next);
        assert_eq!(notes.children, 2);
        assert!(notes.text.starts_with(NOTE_OPEN), "{}", notes.text);
        assert!(
            notes.text.contains("- s-note-done (claude): done"),
            "{}",
            notes.text
        );
        assert!(
            notes.text.contains("- s-note-failed (claude): failed"),
            "{}",
            notes.text
        );
        assert!(!notes.text.contains("s-note-running"));
        assert!(!notes.text.contains("s-note-waited"));
        assert!(notes.text.contains("don't redo work for it"));
        assert_eq!(
            notes_for_turn(v, parent, next),
            notes,
            "a retried attempt gets it again"
        );
        assert_eq!(
            notes_for_turn(v, parent, next + 1),
            Notes::default(),
            "never twice"
        );
        // The running child ends: the turn after gets it, alone.
        crate::run_queue::finish(v, running, RowStatus::Cancelled, None, chrono::Utc::now())
            .unwrap();
        let later = notes_for_turn(v, parent, next + 2);
        assert_eq!(later.children, 1);
        assert!(later.text.contains("- s-note-running (claude): cancelled"));
        let split = crate::sessions::split_injected_context(&format!("{}Go on", later.text));
        assert_eq!(split.user_text, "Go on", "the note is never the request");
        assert_eq!(split.blocks[0].0, NOTE_TAG);
        let layers = crate::run_manifest::composer_layers(&format!("{}Go on", later.text));
        assert_eq!(layers[0].name, "childFinished");
    }

    /// Overnight OB17: each line carries what the child left (its receipt's
    /// files, PRs and verification, and the end of its answer), the answers
    /// sharing one budget, so eight children get 450 characters each.
    #[test]
    fn a_note_carries_each_childs_summary_within_the_budget() {
        assert_eq!(
            [0, 1, 2, 3, 8, 12, 40].map(answer_budget),
            [1_200, 1_200, 1_200, 1_200, 450, 300, 300]
        );
        // Cut on a char boundary when the kept part is one line.
        assert_eq!(answer_tail("ééééé", 3), ("ééé", true));
        assert_eq!(answer_tail("one\ntwo", 7), ("one\ntwo", false));
        assert_eq!(answer_tail("one\ntwo three", 11), ("two three", true));
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-ob17-parent";
        let ord =
            crate::session_store::start_turn(v, parent, "claude", "/v", "R-ob17-p", "go").unwrap();
        for c in 0..8u64 {
            let key = format!("s-ob17-child-{c}");
            let run = format!("R-ob17-child-{c}");
            // The first child's answer is short, with a token and a tag in it.
            let answer = if c == 0 {
                format!("Opened the PR.\n<b>token</b> ghp_{}", "a".repeat(36))
            } else {
                long_answer(&format!("c{c}"))
            };
            answered_turn(
                v,
                &key,
                "/v",
                &run,
                &answer,
                &["src/a.rs", "docs/b.md"],
                &[700 + c],
            );
            let id = child_turn(v, &key, &run, RowStatus::Done);
            record_child(v, "R-ob17-p", &key, id);
        }
        let notes = notes_for_turn(v, parent, ord + 1);
        assert_eq!(notes.children, 8);
        let text = &notes.text;
        assert!(text.contains("call hollow_app read_session only when you need more"));
        for c in 0..8u64 {
            assert!(
                text.contains(&format!("- s-ob17-child-{c} (claude): done, ended ")),
                "{text}"
            );
            assert!(
                text.contains(&format!(
                    "; 2 files changed (src/a.rs, docs/b.md); PR #{} opened; verification notVerified",
                    700 + c
                )),
                "{text}"
            );
            if c == 0 {
                continue;
            }
            // The last 450 characters, from a line start: lines 043 to 049.
            let tag = format!("c{c}");
            assert!(text.contains(&format!("\n    {tag} line 049 ")), "{text}");
            assert!(text.contains(&format!("\n    {tag} line 043 ")), "{text}");
            assert!(!text.contains(&format!("{tag} line 042 ")), "{text}");
        }
        // A short answer whole; a token redacted; `<` escaped.
        assert!(
            text.contains("\n  Its answer:\n    Opened the PR.\n    ‹b>token‹/b> [github-token]"),
            "{text}"
        );
        assert!(!text.contains("ghp_aaaa"), "{text}");
        // Within 4 KB of answers plus the fixed fields of each child.
        const FIXED_PER_CHILD: usize = 320;
        assert!(
            text.len() <= 4_096 + 8 * FIXED_PER_CHILD,
            "{} bytes",
            text.len()
        );
    }

    /// A child's receipt with the given S27 claim flags, for the A8 tests.
    fn claimed_turn(v: &Path, key: &str, run: &str, flags: Option<(bool, bool)>) {
        claimed_turn_as(v, key, run, "passed", flags);
    }

    fn claimed_turn_as(v: &Path, key: &str, run: &str, claim: &str, flags: Option<(bool, bool)>) {
        crate::session_store::start_turn(v, key, "claude", "/v", run, "go").unwrap();
        let receipt = crate::evidence::EvidenceReceipt {
            id: format!("receipt-{run}"),
            run_id: run.into(),
            history_run_id: run.into(),
            source: "chat".into(),
            session_key: Some(key.into()),
            status: "success".into(),
            finished_at: chrono::Utc::now().to_rfc3339(),
            verification_state: "notVerified".into(),
            verification: flags.map(|(without_check, after_fail)| {
                crate::evidence::ReceiptVerification {
                    claim: claim.into(),
                    claim_without_check: without_check,
                    claim_after_fail: after_fail,
                }
            }),
            ..Default::default()
        };
        crate::evidence::persist_for_test(v, receipt).unwrap();
    }

    /// Overnight A8: a child whose reply says its tests pass with no check
    /// after its last edit (or after a failed one) says so on its line, in
    /// the parent's note and wherever else the summary goes.
    #[test]
    fn a_child_claiming_a_pass_without_a_check_is_flagged_to_the_parent() {
        assert!(UNBACKED_AFTER_FAIL.len() + 2 <= 60);
        assert!(UNBACKED_NO_CHECK.len() + 2 <= 60);
        assert!(UNBACKED_DONE_AFTER_FAIL.len() + 2 <= 60);
        assert!(UNBACKED_DONE_NO_CHECK.len() + 2 <= 60);
        assert!(UNBACKED_FIXED_AFTER_FAIL.len() + 2 <= 60);
        assert!(UNBACKED_FIXED_NO_CHECK.len() + 2 <= 60);
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-a8-parent";
        let ord =
            crate::session_store::start_turn(v, parent, "claude", "/v", "R-a8-p", "go").unwrap();
        for (key, flags) in [
            ("s-a8-nocheck", (true, false)),
            ("s-a8-afterfail", (false, true)),
            ("s-a8-both", (true, true)),
        ] {
            let run = format!("R-{key}");
            claimed_turn(v, key, &run, Some(flags));
            let id = child_turn(v, key, &run, RowStatus::Done);
            record_child(v, "R-a8-p", key, id);
        }
        let notes = notes_for_turn(v, parent, ord + 1);
        assert_eq!(notes.children, 3);
        let line = |key: &str| {
            notes
                .text
                .lines()
                .find(|l| l.starts_with(&format!("- {key} ")))
                .unwrap_or_else(|| panic!("{}", notes.text))
                .to_string()
        };
        assert!(
            line("s-a8-nocheck").ends_with(
                "; verification notVerified; says it passes; no check after its last edit"
            ),
            "{}",
            notes.text
        );
        assert!(
            line("s-a8-afterfail")
                .ends_with("; verification notVerified; says it passes; its last check failed"),
            "{}",
            notes.text
        );
        // A failed check is the stronger fact: one clause, never both.
        let both = line("s-a8-both");
        assert!(both.ends_with("its last check failed"), "{both}");
        assert!(!both.contains("no check after"), "{both}");
        // wait_for_session's answer carries it too.
        let waiter = caller("R-a8-p", parent);
        let waited = block_on(wait_with(v, &waiter, &["s-a8-nocheck"], 60, || true)).unwrap();
        assert_eq!(waited["unbackedClaim"], json!(UNBACKED_NO_CHECK));
    }

    /// Overhaul E3: a child that said it was done, with no check, is not
    /// reported to its parent as a pass claim.
    #[test]
    fn a_done_claim_is_not_reported_as_a_pass() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let key = "s-e3-done";
        let run = "R-e3-done";
        claimed_turn_as(v, key, run, "done", Some((true, false)));
        let id = child_turn(v, key, run, RowStatus::Done);
        let row = crate::run_queue::get(v, id).unwrap().unwrap();
        let text = child_summary(v, key, &row).text(1_200);
        assert!(text.contains(UNBACKED_DONE_NO_CHECK), "{text}");
        assert!(!text.contains("says it passes"), "{text}");
        let key = "s-e3-fixed";
        let run = "R-e3-fixed";
        claimed_turn_as(v, key, run, "fixed", Some((false, true)));
        let id = child_turn(v, key, run, RowStatus::Done);
        let row = crate::run_queue::get(v, id).unwrap().unwrap();
        let text = child_summary(v, key, &row).text(1_200);
        assert!(text.contains(UNBACKED_FIXED_AFTER_FAIL), "{text}");
        assert!(!text.contains("says it passes"), "{text}");
    }

    /// Overnight A8: no flag, no clause; a receipt without the S27 block,
    /// or with a backed claim, reads as before.
    #[test]
    fn no_clause_without_a_flag() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        for (key, flags) in [("s-a8-none", None), ("s-a8-backed", Some((false, false)))] {
            let run = format!("R-{key}");
            claimed_turn(v, key, &run, flags);
            let id = child_turn(v, key, &run, RowStatus::Done);
            let row = crate::run_queue::get(v, id).unwrap().unwrap();
            let summary = child_summary(v, key, &row);
            assert_eq!(summary.unbacked_claim, None, "{key}");
            let text = summary.text(1_200);
            assert!(text.ends_with("; verification notVerified"), "{text}");
            assert!(!text.contains("says it passes"), "{text}");
            let waiter = caller("R-a8-w", "s-a8-waiter");
            let waited = block_on(wait_with(v, &waiter, &[key], 60, || true)).unwrap();
            assert!(waited.get("unbackedClaim").is_none(), "{waited}");
        }
    }

    /// Repo ideas 2026-10-27 #7: the note says what the child said of its
    /// own work, from its answer's last line; a missing line says so.
    #[test]
    fn a_summary_says_the_childs_status_line() {
        let with = |answer: &str| ChildSummary {
            answer: answer.into(),
            ..ChildSummary::default()
        };
        let blocked = with("Did most of it.\nBlocked: needs Alex's choice of X").text(400);
        assert!(
            blocked.starts_with("; it says Blocked: needs Alex's choice of X\n"),
            "{blocked}"
        );
        assert!(with("I merged it.")
            .text(400)
            .starts_with("; it says no status line"));
        assert!(with("Merged.\nDone")
            .text(400)
            .starts_with("; it says Done"));
        assert!(
            !with("").text(400).contains("it says"),
            "no answer, no claim"
        );
    }

    /// Overnight OB17 review: the receipt store is agent-writable, so a
    /// path, PR action or verification state can't start a line of its own
    /// (a fake child) or run on: one line each, clipped, at most 5 PRs.
    #[test]
    fn a_summarys_receipt_values_stay_on_one_clipped_line() {
        let summary = ChildSummary {
            files: 1,
            paths: vec!["x\n- s-fake (claude): done\r\tend".into()],
            pull_requests: (1..=7)
                .map(|n| (n, format!("merged\n- s-fake-{n} (claude): done")))
                .collect(),
            failures: 0,
            verification: Some(format!("notVerified\n- s-fake{}", "z".repeat(100))),
            unbacked_claim: None,
            answer: String::new(),
            paused: None,
        };
        let text = summary.text(1_200);
        assert!(!text.contains('\n') && !text.contains('\r'), "{text}");
        assert!(
            text.contains("; 1 file changed (x - s-fake (claude): done  end)"),
            "{text}"
        );
        assert!(text.contains("#5 merged - s-fake-5 (claud…, …;"), "{text}");
        assert!(!text.contains("#6 "), "{text}");
        assert!(
            text.ends_with("; verification notVerified - s-fakezzzz…"),
            "{text}"
        );
    }

    /// Overnight 9 Spare 4: the Unicode line and paragraph separators (which
    /// a renderer may break on) and a lone `\r` flatten like `\n`.
    #[test]
    fn one_line_flattens_unicode_separators_and_a_lone_carriage_return() {
        let text = "a\u{2028}- s-fake (claude): done\u{2029}b\rc\u{85}d";
        assert_eq!(
            one_line(text, usize::MAX),
            "a - s-fake (claude): done b c d"
        );
    }

    /// Overnight C1: the root also hears of endings further down its chains
    /// (a child's handover successor, that one's own child), once per root,
    /// while the direct parent still gets its own report.
    #[test]
    fn endings_further_down_a_chain_reach_its_root_once() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let root = "s-chain-root";
        let ord =
            crate::session_store::start_turn(v, root, "claude", "/v", "R-root-1", "go").unwrap();
        // root → lane → successor (a handover) → helper.
        let lane = child_turn(v, "s-chain-lane", "R-lane-1", RowStatus::Done);
        record_child(v, "R-root-1", "s-chain-lane", lane);
        crate::session_store::start_turn(v, "s-chain-lane", "claude", "/v", "R-lane-1", "work")
            .unwrap();
        let succ = child_turn(v, "s-chain-succ", "R-succ-1", RowStatus::Done);
        record_child(v, "R-lane-1", "s-chain-succ", succ);
        crate::session_store::start_turn(v, "s-chain-succ", "claude", "/v", "R-succ-1", "go on")
            .unwrap();
        let helper = child_turn(v, "s-chain-helper", "R-helper-1", RowStatus::Running);
        record_child(v, "R-succ-1", "s-chain-helper", helper);

        let notes = notes_for_turn(v, root, ord + 1);
        assert!(
            notes.text.contains("- s-chain-lane (claude): done"),
            "{}",
            notes.text
        );
        assert!(notes.text.contains(CHAIN_HEADER), "{}", notes.text);
        assert!(
            notes.text.contains("- s-chain-succ (claude): done"),
            "{}",
            notes.text
        );
        assert!(notes.text.contains("(from s-chain-lane)"), "{}", notes.text);
        assert!(!notes.text.contains("s-chain-helper"), "still running");
        assert_eq!(notes.children, 2);
        assert_eq!(
            notes_for_turn(v, root, ord + 1),
            notes,
            "a retried attempt gets it again"
        );
        // The helper ends: the root's next turn hears of it, alone, once.
        crate::run_queue::finish(
            v,
            helper,
            RowStatus::Failed,
            Some("boom"),
            chrono::Utc::now(),
        )
        .unwrap();
        let later = notes_for_turn(v, root, ord + 2);
        assert_eq!(later.children, 1, "{}", later.text);
        assert!(
            later.text.contains("- s-chain-helper (claude): failed"),
            "{}",
            later.text
        );
        assert!(later.text.contains("(from s-chain-succ)"), "{}", later.text);
        assert_eq!(
            notes_for_turn(v, root, ord + 3),
            Notes::default(),
            "never twice"
        );
        // Each conversation hears of the chains it started: the lane, whose
        // own child is the successor, gets that as a direct report and the
        // helper below it as further down; the root's reports didn't use them up.
        let lane_notes = notes_for_turn(v, "s-chain-lane", 2);
        assert!(
            lane_notes.text.contains("- s-chain-succ (claude): done"),
            "{}",
            lane_notes.text
        );
        let further = lane_notes
            .text
            .find(CHAIN_HEADER)
            .expect("the helper is further down");
        assert!(
            lane_notes.text[further..].contains("s-chain-helper"),
            "{}",
            lane_notes.text
        );
        assert!(!lane_notes.text[..further].contains("s-chain-helper"));
    }

    /// C1 review: a later turn of a conversation the walk already visited is
    /// reported too, records older than a week aren't, and a chain started
    /// from an earlier Main agent thread reaches today's thread.
    #[test]
    fn chain_reports_cover_later_turns_skip_old_records_and_span_main_threads() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let root = "s-c1r-root";
        let ord =
            crate::session_store::start_turn(v, root, "claude", "/v", "R-c1r-1", "go").unwrap();
        let lane = child_turn(v, "s-c1r-lane", "R-c1r-lane", RowStatus::Done);
        record_child(v, "R-c1r-1", "s-c1r-lane", lane);
        crate::session_store::start_turn(v, "s-c1r-lane", "claude", "/v", "R-c1r-lane", "w")
            .unwrap();
        // The lane messages its helper twice: two turns of one conversation.
        let first = child_turn(v, "s-c1r-helper", "R-c1r-h1", RowStatus::Done);
        record_child(v, "R-c1r-lane", "s-c1r-helper", first);
        let second = child_turn(v, "s-c1r-helper", "R-c1r-h2", RowStatus::Done);
        record_child(v, "R-c1r-lane", "s-c1r-helper", second);
        let notes = notes_for_turn(v, root, ord + 1);
        assert_eq!(
            notes.text.matches("s-c1r-helper (claude)").count(),
            2,
            "{}",
            notes.text
        );
        // A record older than the window isn't news.
        let old = child_turn(v, "s-c1r-old", "R-c1r-old", RowStatus::Done);
        record_child(v, "R-c1r-lane", "s-c1r-old", old);
        crate::session_store::with_write(v, "test", |tx| {
            tx.execute(
                "UPDATE delegations SET at = '2020-01-01T00:00:00.000Z' WHERE record_json LIKE '%s-c1r-old%'",
                [],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
        assert!(!notes_for_turn(v, root, ord + 2).text.contains("s-c1r-old"));

        // Main agent: yesterday's thread started a lane whose successor ended;
        // today's thread hears of it.
        crate::workspaces::ensure_main_agent(v).unwrap();
        let main = crate::workspaces::main_agent_cwd(v).unwrap();
        crate::session_store::start_turn(v, "s-c1r-main-old", "claude", &main, "R-c1r-m1", "go")
            .unwrap();
        let mlane = child_turn(v, "s-c1r-mlane", "R-c1r-ml", RowStatus::Done);
        record_child(v, "R-c1r-m1", "s-c1r-mlane", mlane);
        crate::session_store::start_turn(v, "s-c1r-mlane", "claude", "/v", "R-c1r-ml", "w")
            .unwrap();
        let msucc = child_turn(v, "s-c1r-msucc", "R-c1r-ms", RowStatus::Done);
        record_child(v, "R-c1r-ml", "s-c1r-msucc", msucc);
        let today = crate::session_store::start_turn(
            v,
            "s-c1r-main-new",
            "claude",
            &main,
            "R-c1r-m2",
            "hi",
        )
        .unwrap();
        let main_notes = notes_for_turn(v, "s-c1r-main-new", today + 1);
        assert!(
            main_notes.text.contains("s-c1r-msucc (claude): done"),
            "{}",
            main_notes.text
        );
        // Once for the Main agent, whichever thread asks next.
        let again = crate::session_store::start_turn(
            v,
            "s-c1r-main-newer",
            "claude",
            &main,
            "R-c1r-m3",
            "hey",
        )
        .unwrap();
        assert!(!notes_for_turn(v, "s-c1r-main-newer", again + 1)
            .text
            .contains("s-c1r-msucc"));
    }

    /// C1 re-review: the walk follows the turns a chain caused. A
    /// conversation the root messages had an older child of its own (from a
    /// turn the root didn't cause): that isn't the root's news.
    #[test]
    fn only_turns_the_chain_caused_are_followed() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        // X's first turn (driven by someone else) started Y.
        crate::session_store::start_turn(v, "s-run-x", "claude", "/v", "R-x-1", "own").unwrap();
        let y = child_turn(v, "s-run-y", "R-y-1", RowStatus::Done);
        record_child(v, "R-x-1", "s-run-y", y);
        // The root then sends X a turn, which starts Z.
        let root = "s-run-root";
        let ord =
            crate::session_store::start_turn(v, root, "claude", "/v", "R-root-x", "go").unwrap();
        let x2 = child_turn(v, "s-run-x", "R-x-2", RowStatus::Done);
        record_child(v, "R-root-x", "s-run-x", x2);
        crate::session_store::start_turn(v, "s-run-x", "claude", "/v", "R-x-2", "from root")
            .unwrap();
        let z = child_turn(v, "s-run-z", "R-z-1", RowStatus::Done);
        record_child(v, "R-x-2", "s-run-z", z);
        let notes = notes_for_turn(v, root, ord + 1);
        assert!(
            notes.text.contains("- s-run-z (claude): done"),
            "{}",
            notes.text
        );
        assert!(!notes.text.contains("s-run-y"), "{}", notes.text);
    }

    /// Morning M3: whoever hears of a chat's ending learns the successor it
    /// handed over to, from Hollow (the note's line, a wait's answer).
    #[test]
    fn an_ending_names_the_successor_it_handed_over_to() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-m3-parent";
        let ord = crate::session_store::start_turn(v, parent, "claude", "/v", "R-m3-parent", "go")
            .unwrap();
        let child = child_turn(v, "s-m3-child", "R-m3-child", RowStatus::Done);
        record_child(v, "R-m3-parent", "s-m3-child", child);
        let row = |id| crate::run_queue::get(v, id).unwrap().unwrap();
        assert_eq!(handed_over(v, &row(child)), "");
        // The child's turn handed over (its record, as `handover` stores it).
        let heir = child_turn(v, "s-m3-heir", "", RowStatus::Queued);
        let record = DelegationRecord {
            kind: "childTask".into(),
            tool: crate::app_sessions::HANDOVER_TOOL.into(),
            agent: "claude".into(),
            child_session: Some("s-m3-heir".into()),
            child_queue_id: Some(heir),
            at: crate::session_store::db_time(chrono::Utc::now()),
            ..DelegationRecord::default()
        };
        crate::delegation::store(v, "R-m3-child", &record);
        assert_eq!(handed_over(v, &row(child)), ", handed over to s-m3-heir");
        let notes = notes_for_turn(v, parent, ord + 1);
        assert!(
            notes.text.contains("- s-m3-child (claude): done")
                && notes.text.contains(", handed over to s-m3-heir"),
            "{}",
            notes.text
        );
        let me = caller("R-m3-parent", parent);
        let answer = finished(v, &me, "s-m3-child", &row(child), 0);
        assert_eq!(answer["successor"], "s-m3-heir");
        assert!(answer.get("successorWithdrawn").is_none());
        // A later turn of the same conversation handed nothing over.
        let later = child_turn(v, "s-m3-child", "R-m3-child-2", RowStatus::Done);
        assert_eq!(handed_over(v, &row(later)), "");
        assert!(finished(v, &me, "s-m3-child", &row(later), 0)
            .get("successor")
            .is_none());
        // A stop withdrew the successor: said so, never as a live one.
        crate::run_queue::cancel(v, heir, chrono::Utc::now()).unwrap();
        assert_eq!(
            handed_over(v, &row(child)),
            ", its successor s-m3-heir was withdrawn"
        );
        assert_eq!(
            finished(v, &me, "s-m3-child", &row(child), 0)["successorWithdrawn"],
            true
        );
    }

    /// A 50-line answer of 3,049 characters (about 3 KB): each line 60
    /// characters, starting `{tag} line NNN ` (overnight OB17).
    pub(crate) fn long_answer(tag: &str) -> String {
        (0..50)
            .map(|i| {
                let head = format!("{tag} line {i:03} ");
                format!("{head}{}", "x".repeat(60 - head.len()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Conversation `key`'s turn that ran as `run`, answered `answer`, with
    /// the chat receipt a finished turn leaves: `files` changed and `prs`
    /// opened, not verified (overnight OB17).
    pub(crate) fn answered_turn(
        vault: &Path,
        key: &str,
        cwd: &str,
        run: &str,
        answer: &str,
        files: &[&str],
        prs: &[u64],
    ) {
        let ord = crate::session_store::start_turn(vault, key, "claude", cwd, run, "go").unwrap();
        crate::session_store::append_event(vault, key, ord, "assistant", answer).unwrap();
        crate::session_store::finish_turn(vault, key, ord, "success").unwrap();
        let receipt = crate::evidence::EvidenceReceipt {
            id: format!("receipt-{run}"),
            run_id: run.into(),
            history_run_id: run.into(),
            source: "chat".into(),
            session_key: Some(key.into()),
            status: "success".into(),
            finished_at: chrono::Utc::now().to_rfc3339(),
            verification_state: "notVerified".into(),
            files: files
                .iter()
                .map(|path| crate::evidence::ReceiptFile {
                    path: (*path).into(),
                    change_type: "modify".into(),
                    attribution: "confirmed".into(),
                    tool: Some("Edit".into()),
                    additions: None,
                    deletions: None,
                    not_recoverable: false,
                    also_changed_by: Vec::new(),
                    hollow_write: false,
                })
                .collect(),
            pull_requests: prs
                .iter()
                .map(|number| crate::evidence::ReceiptPullRequest {
                    number: *number,
                    action: "opened".into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        crate::evidence::persist_for_test(vault, receipt).unwrap();
    }

    /// A queued chat row for conversation `key`, ended as `status` (with
    /// `run` as its history run unless empty).
    pub(crate) fn child_turn(vault: &Path, key: &str, run: &str, status: RowStatus) -> i64 {
        let row = crate::run_queue::NewRow::new(RunSource::Chat, Some(key), "chat", "test");
        let id = crate::run_queue::enqueue(vault, row, chrono::Utc::now())
            .unwrap()
            .id();
        if status == RowStatus::Queued {
            return id;
        }
        if !run.is_empty() {
            crate::run_queue::claim(vault, id, chrono::Utc::now()).unwrap();
            crate::run_queue::mark_running(vault, id, run, chrono::Utc::now()).unwrap();
            if status == RowStatus::Running {
                return id;
            }
            crate::run_queue::finish(vault, id, status, None, chrono::Utc::now()).unwrap();
        } else {
            crate::run_queue::cancel(vault, id, chrono::Utc::now()).unwrap();
        }
        id
    }

    pub(crate) fn record_child(vault: &Path, run: &str, key: &str, queue_id: i64) {
        let record = DelegationRecord {
            kind: "childTask".into(),
            tool: "start_session".into(),
            agent: "claude".into(),
            child_session: Some(key.into()),
            child_queue_id: Some(queue_id),
            at: crate::session_store::db_time(chrono::Utc::now()),
            ..DelegationRecord::default()
        };
        crate::delegation::store(vault, run, &record);
    }
}
