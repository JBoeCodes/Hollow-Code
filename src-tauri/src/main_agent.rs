//! The Main agent's mission control (main agent M2, 2026-10-07; see
//! `docs/COO.md`).
//!
//! **The chats it started** ([`children`]): every durable child task a Main
//! agent conversation queued (`hollow_app` `start_session` /
//! `send_to_session`, recorded by `delegation.rs` with its run-queue row) in
//! the last [`LOOKBACK_DAYS`] days, with the row's state, its workspace, its
//! cost and whether the Main agent has been told it ended. The top bar's
//! orb and the Main agent's "Chats I started" list read this.
//!
//! **Waking the Main agent** ([`after_chat_finished`], [`decide`]).
//! Elsewhere a parent conversation learns that a child ended when it next
//! takes a turn (`descendants::notes_for_turn`), or when it asked to be woken
//! for it (`wake_me::schedule_on_end`, overnight OB10b). The Main agent is meant
//! to watch work, so when a chat it started ends, Hollow queues one turn in
//! *today's* Main agent thread (the one the orb opens; a new one when today
//! has none), listing the ended chats in its message ([`Wake::message`]) and
//! recording them as reported (`deliver_child` via `wake`). Guards:
//! - only a turn that ran and ended is news (`done`, `failed`,
//!   `interrupted`), never one the person stopped or that never ran;
//! - a chat that handed over is news when the last successor of its chain
//!   ends ([`chain_end`]); the day's cap holds back only chats Hollow's own
//!   turns started, and a held-back wake is kept with its reason
//!   ([`Decision::Skipped`], shown in the side band) (overnight 10-11 A1);
//! - each child turn wakes at most once, within [`WAKE_WINDOW_HOURS`] of its
//!   end, unless it was already reported (a wait or a note), and at most
//!   [`MAX_WAKES_PER_DAY`] wakes a local day; a wake is recorded only once
//!   it was queued, so a refused send tries again later;
//! - none while today's thread has a turn queued or running, and none in the
//!   [`GRACE_SECS`] after its turn ends (the person may be replying): a
//!   later check is scheduled instead;
//! - a wake turn runs in the thread's own last mode ([`wake_mode`],
//!   2026-11-12; before, it was capped at `acceptEdits`): what it may do with
//!   nobody watching is limited by the unattended allow list
//!   ([`unattended`]) and Hollow's cards, not by its mode; one check runs at
//!   a time ([`WAKE_LOCK`]).
//!
//! **Check-ins (M3)** ([`decide_pulse`]), **acting on its own (M4)**
//! ([`unattended`]), **swarms (M5)** ([`advance`]) and **learning from its
//! suggestions (M6)** ([`record_outcome`]) follow; [`SCHEDULE_NOTE`] tells the
//! agent exactly what runs it.

use crate::core_host::Core;
use crate::delegation::Contract;
use crate::run_manifest::DelegationRecord;
use crate::session_store;
use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The Main agent prompt's line about its schedule (M3): what really runs
/// it, so the standing orders' wording never reads as a promise of its own.
pub(crate) const SCHEDULE_NOTE: &str = "How you are run: when the person messages you; when a chat you started ends (Hollow wakes you once to report it); when a swarm you ran finishes; and at check-ins on the schedule in Settings → Background work → COO (by default every 30 minutes, 8:00 to 18:00 on working days), which reach you only when something new came up or at the day's first check-in, and never past the day's cost cap. Settings decide the schedule; your standing orders say what to watch. If asked, describe exactly this.";

/// Claude tool rules that keep a Main agent turn's own file tools off the
/// standing orders (`chat.rs`).
pub(crate) const STANDING_ORDERS_DENY: &[&str] = &[
    "Edit(**/STANDING_ORDERS.md)",
    "Write(**/STANDING_ORDERS.md)",
    "MultiEdit(**/STANDING_ORDERS.md)",
];

/// How a wake turn's message starts; the webview shows such a message as a
/// note from Hollow, not as something the person typed (`src/mainAgent.ts`).
pub(crate) const WAKE_PREFIX: &str = "⟲ Hollow:";
/// The message of a wake turn, followed by one line per finished chat with
/// its summary (`descendants::child_summary`, overnight OB17): the end of its
/// answer, its files and pull requests.
pub(crate) const WAKE_MESSAGE: &str = "⟲ Hollow: a chat you started has finished. Its answer, files and pull requests are below; call read_session only when you need more. Report in two or three lines what it found and what is next. This is a check-in, not a new request: don't start new work unless the person already asked for this follow-up.";
/// The wake message from V5b until T3a (2026-11-15), when Main's chats table
/// in PLAN.md stopped being written: stored turns open with it too.
pub(crate) const V5B_WAKE_MESSAGE: &str = "⟲ Hollow: a chat you started has finished. Its answer, files and pull requests are below; call read_session only when you need more. Report in two or three lines what it found and what is next, and log it in PLAN.md. This is a check-in, not a new request: don't start new work unless the person already asked for this follow-up.";
/// The wake message before V5b (2026-11-15): stored turns still open with it,
/// so [`host_subject`] reads their lines too.
pub(crate) const LEGACY_WAKE_MESSAGE: &str = "⟲ Hollow: a chat you started has finished. Its answer, files and pull requests are below; call read_session only when you need more. Report in two or three lines what it found and what is next, and update PLAN.md. This is a check-in, not a new request: don't start new work unless the person already asked for this follow-up.";
/// How a check-in's message opens (then the time). The golden envelope's
/// check-in (`chat_g2_budget_tests`, OB15) builds its text from it.
pub(crate) const CHECKIN_OPEN: &str = "⟲ Hollow: check-in at";
/// What comes before a check-in's signal lines (one per line, then a blank
/// line).
pub(crate) const CHECKIN_NEW: &str = " New since your last check-in:\n";
/// A wake line's place for a chat in no workspace.
const NO_WORKSPACE: &str = "no workspace";
/// How a finished swarm's wake message opens (then its goal).
const SWARM_OPEN: &str = "⟲ Hollow: the swarm “";

/// The `turns.asked_by` of a turn Hollow queued itself (OB20,
/// `ChatSendRequest::host_asker`): every one starts `hollow:`, which is what
/// readers test (`session_store::person_asks`); `agent` is another agent's
/// turn, and none (NULL) the person's. A wake for chats that ended.
pub(crate) const ASKED_BY_WAKE: &str = "hollow:wake";
/// A check-in ([`decide_pulse`]).
pub(crate) const ASKED_BY_CHECKIN: &str = "hollow:checkin";
/// The weekly update's prose pass (`digest`).
pub(crate) const ASKED_BY_WEEKLY: &str = "hollow:weekly";
/// A wake-up an agent asked for (`wake_me::run_due`).
pub(crate) const ASKED_BY_WAKE_ME: &str = "hollow:wake_me";
/// A finished swarm's synthesis wake ([`advance`]).
pub(crate) const ASKED_BY_SWARM: &str = "hollow:swarm";

/// Whether `text` is one of Hollow's own check-ins or wakes: a
/// [`WAKE_PREFIX`] message (a finished chat, a check-in, a finished swarm, a
/// `wake_me` wake-up, the weekly update, the morning review), except the two
/// that open a conversation as its task: a step retry's seed and a friction
/// fix's brief (the task-state record's goal, `task_state::fold`).
pub(crate) fn is_wake_message(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(WAKE_PREFIX)
        && ![crate::step_spine::RETRY_OPEN, crate::friction::FIX_OPEN]
            .iter()
            .any(|open| text.starts_with(open))
}

/// What a wake or check-in is about, for its retrieval, recall, attached
/// sources and connector selection (overnight OB18): a wake's chat titles
/// and workspace names, read from its lines ([`Wake::message`]); a
/// check-in's signal lines (an empty subject when nothing new came up).
/// Its template ("report", "PLAN.md", "finished") is never the query.
/// `None` for anything else, another Hollow message included (a swarm, a
/// `wake_me` wake-up, the weekly update, the morning review): those read
/// as their request.
pub(crate) fn host_subject(asked: &str) -> Option<String> {
    let text = asked.trim_start();
    if !is_wake_message(text) {
        return None;
    }
    if let Some(lines) = text
        .strip_prefix(WAKE_MESSAGE)
        .or_else(|| text.strip_prefix(V5B_WAKE_MESSAGE))
        .or_else(|| text.strip_prefix(LEGACY_WAKE_MESSAGE))
    {
        let mut subject: Vec<&str> = Vec::new();
        for (title, place) in lines.lines().filter_map(wake_line_subject) {
            for part in [title, place] {
                if !part.is_empty() && part != NO_WORKSPACE && !subject.contains(&part) {
                    subject.push(part);
                }
            }
        }
        return Some(subject.join("\n"));
    }
    if text.starts_with(CHECKIN_OPEN) {
        let signals = text
            .split_once(CHECKIN_NEW)
            .and_then(|(_, rest)| rest.split("\n\n").next())
            .unwrap_or("");
        let lines: Vec<&str> = signals
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        return Some(lines.join("\n"));
    }
    None
}

/// A wake line's chat title and place (`- “title” in place (agent,
/// session key): status…`); `None` for its summary's indented lines.
fn wake_line_subject(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("- “")?;
    // A title may hold ", session " itself: the first one whose head parses.
    rest.match_indices(", session ").find_map(|(at, _)| {
        let (named, _agent) = rest[..at].rsplit_once(" (")?;
        let (title, place) = named.rsplit_once("” in ")?;
        Some((title.trim(), place.trim()))
    })
}

/// How far back the list of started chats looks.
pub(crate) const LOOKBACK_DAYS: i64 = 7;
/// The most started chats listed.
const LIST_LIMIT: usize = 64;
/// A child turn that ended longer ago than this never wakes anyone (it was
/// missed while Hollow was closed, or before this existed).
pub(crate) const WAKE_WINDOW_HOURS: i64 = 6;
/// Wakes in one local day.
pub(crate) const MAX_WAKES_PER_DAY: u32 = 24;
/// The strongest mode an unattended turn starts a chat in
/// ([`unattended::allows`], `app_actions::sessions_start`): a standing
/// order, not the mode the unattended turn itself runs in ([`wake_mode`]).
pub(crate) const UNATTENDED_START_CAP: &str = "acceptEdits";
/// Child turns remembered as having woken the Main agent.
const WOKEN_KEPT: usize = 256;

/// One chat the Main agent started (the webview's `MainAgentChild`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainAgentChild {
    /// The Main agent conversation that started it.
    pub parent_key: String,
    pub child_session: String,
    pub title: String,
    pub workspace_id: Option<String>,
    pub workspace_title: Option<String>,
    pub agent: String,
    /// The turn's run-queue state (`queued`, `running`, `done`, `failed`,
    /// `cancelled`, …), or `unknown` once the row is gone.
    pub status: String,
    /// Queued, claimed or running.
    pub live: bool,
    pub queue_id: i64,
    pub requested_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    /// The Main agent has been told it ended (a wait or a note).
    pub reported: bool,
    /// The turn's cost with its own children's; `None` while unknown.
    pub cost_usd: Option<f64>,
    /// Hollow asked for it (a wake or check-in turn started it), not the
    /// person: only these count toward the daily cap (overnight 10-11 A1).
    pub by_hollow: bool,
    /// The successors it handed over to, oldest first; `status`, `live`,
    /// `finished_at` and `error` are the last one's (overnight 10-11 A1).
    pub successors: Vec<String>,
    /// The queue row whose end is news: the last successor's, else its own.
    pub end_queue_id: i64,
    /// Repo ideas 2026-10-27 #7: what the chat said of its own work in its
    /// answer's last line (`child_status`), once it ended; `None` while it
    /// runs or never answered. A missing line is `unknown`, never done.
    pub outcome: Option<crate::child_status::Outcome>,
    /// The successors' queue rows, oldest first (reported to the Main agent
    /// as a chain root, `root_deliveries`, never as their predecessor's).
    #[serde(skip)]
    pub chain_ids: Vec<i64>,
}

/// The Main agent's folder, once it exists.
pub(crate) fn main_cwd(vault: &Path) -> Option<String> {
    crate::workspaces::main_agent_cwd(vault)
}

/// The chats the Main agent started, newest first (see the module doc).
pub(crate) fn children(vault: &Path, now: DateTime<Utc>) -> Vec<MainAgentChild> {
    collect(vault, now, true, None)
}

/// Budget roots without workspace decoration or complete receipt roll-ups.
/// The cap reads recorded chain spend separately, including running heirs.
pub(crate) fn budget_children(vault: &Path, now: DateTime<Utc>) -> Vec<MainAgentChild> {
    collect(vault, now, false, None)
}

/// The key the Main agent's threads share as a chain root
/// (`descendants::chain_lines`).
const CHAIN_ROOT: &str = "main-agent";

/// The started chats; `full` adds what only the list shows (workspace and
/// cost), which a wake check, run on every chat's end and every minute,
/// skips. `ended_since` keeps only chats whose chain ended in a reportable
/// state at or after it, before anything else is read.
fn collect(
    vault: &Path,
    now: DateTime<Utc>,
    full: bool,
    ended_since: Option<DateTime<Utc>>,
) -> Vec<MainAgentChild> {
    let Some(cwd) = main_cwd(vault) else {
        return Vec::new();
    };
    let rows = session_store::delegation_runs_for_cwd(
        vault,
        &cwd,
        now - Duration::days(LOOKBACK_DAYS),
        LIST_LIMIT,
    )
    .unwrap_or_default();
    let workspaces = if full {
        crate::workspaces::list(vault).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut seen = HashSet::new();
    let mut asked_by_hollow: std::collections::HashMap<String, bool> = Default::default();
    let mut out = Vec::new();
    for (parent_key, parent_run, json) in rows {
        let Ok(record) = serde_json::from_str::<DelegationRecord>(&json) else {
            continue;
        };
        if record.kind != Contract::ChildTask.as_str() {
            continue;
        }
        let (Some(queue_id), Some(child_session)) =
            (record.child_queue_id, record.child_session.clone())
        else {
            continue;
        };
        if !seen.insert(queue_id) {
            continue;
        }
        // The Main agent's own handover continues its one conversation: not a
        // chat it started, so its successor's end never wakes it (the
        // once-per-handover self-wake, gone 2026-11-07).
        if record.tool == crate::app_sessions::HANDOVER_TOOL
            && session_store::session_cwd(vault, &child_session).as_deref() == Some(cwd.as_str())
        {
            continue;
        }
        let row = crate::descendants::child_row(vault, &record);
        // A chat that handed over ends when its last successor does.
        let (end, successors, chain_ids) = match row.clone() {
            Some(r) if !r.status.is_live() => {
                let (end, keys, ids) = chain_end(vault, r);
                (Some(end), keys, ids)
            }
            other => (other, Vec::new(), Vec::new()),
        };
        if let Some(since) = ended_since {
            let news = end.as_ref().is_some_and(|r| {
                !r.status.is_live()
                    && reportable(r.status.as_str())
                    && r.finished_at
                        .as_deref()
                        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                        .is_some_and(|at| at.with_timezone(&Utc) >= since)
            });
            if !news {
                continue;
            }
        }
        let summary = session_store::read_session(vault, &child_session)
            .ok()
            .map(|d| d.summary);
        let workspace = summary
            .as_ref()
            .and_then(|s| workspaces.iter().find(|w| w.cwd == s.cwd));
        let asked = row
            .as_ref()
            .and_then(|r| r.detail.as_deref())
            .map(first_line);
        let title = summary
            .as_ref()
            .and_then(|s| s.title.clone())
            .filter(|t| !t.trim().is_empty())
            .or(asked)
            .unwrap_or_else(|| child_session.clone());
        // A successor's report to the chat that handed over is that chat's;
        // the Main agent heard of it only as the chain's root.
        let reported = end.as_ref().is_some_and(|r| {
            if chain_ids.is_empty() {
                session_store::child_delivery(vault, r.id)
                    .ok()
                    .flatten()
                    .is_some()
            } else {
                session_store::root_deliveries(vault, CHAIN_ROOT, &[r.id])
                    .ok()
                    .is_some_and(|m| m.contains_key(&r.id))
            }
        });
        let cost_usd = if full {
            row.as_ref()
                .and_then(|r| r.history_run_id.as_deref())
                .and_then(|run| crate::descendants::rollup(vault, run).total_usd)
        } else {
            None
        };
        let by_hollow = *asked_by_hollow
            .entry(parent_run.clone())
            .or_insert_with(|| unattended::is_unattended_run(vault, &parent_run));
        // Repo ideas 2026-10-27 #7: the ended chain's last answer, read for
        // its status line (the list only; a wake check skips it).
        let outcome = if full {
            end.as_ref().filter(|r| !r.status.is_live()).and_then(|r| {
                let session = successors.last().unwrap_or(&child_session);
                let run = r
                    .history_run_id
                    .as_deref()
                    .filter(|run| !run.trim().is_empty())?;
                let answer = session_store::turn_reply_for_run(vault, session, run)
                    .ok()
                    .flatten()?;
                (!answer.trim().is_empty()).then(|| crate::child_status::parse(&answer))
            })
        } else {
            None
        };
        out.push(MainAgentChild {
            parent_key,
            child_session,
            title,
            workspace_id: workspace.map(|w| w.id.clone()),
            workspace_title: workspace.map(|w| w.title.clone()),
            agent: record.agent.clone(),
            status: end
                .as_ref()
                .map(|r| r.status.as_str().to_string())
                .unwrap_or_else(|| "unknown".into()),
            live: end.as_ref().is_some_and(|r| r.status.is_live()),
            queue_id,
            requested_at: row.as_ref().map(|r| r.requested_at.clone()),
            finished_at: end.as_ref().and_then(|r| r.finished_at.clone()),
            error: end.as_ref().and_then(|r| r.error.clone()),
            reported,
            cost_usd,
            by_hollow,
            end_queue_id: end.as_ref().map_or(queue_id, |r| r.id),
            successors,
            chain_ids,
            outcome,
        });
    }
    out
}

/// The last turn of the chain of handovers that starts with the ended turn
/// `row`, and the successors' session keys and queue rows, oldest first
/// (overnight 10-11 A1: an overnight lane hands over every few hours, and the
/// Main agent heard only of the first chat's end, never the lane's). A
/// successor someone stopped (its row cancelled) is the end: not news.
pub(crate) fn chain_end(
    vault: &Path,
    row: crate::run_queue::QueueRow,
) -> (crate::run_queue::QueueRow, Vec<String>, Vec<i64>) {
    let mut row = row;
    let mut keys: Vec<String> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    for _ in 0..crate::app_sessions::MAX_HANDOVER_CHAIN {
        if row.status.is_live() || row.status == crate::run_queue::RowStatus::Cancelled {
            break;
        }
        let Some(run) = row.history_run_id.clone() else {
            break;
        };
        let Some(record) = crate::delegation::for_run(vault, &run)
            .into_iter()
            .rev()
            .find(|r| {
                r.kind == Contract::ChildTask.as_str()
                    && r.tool == crate::app_sessions::HANDOVER_TOOL
            })
        else {
            break;
        };
        let (Some(key), Some(next)) = (
            record.child_session.clone(),
            crate::descendants::child_row(vault, &record),
        ) else {
            break;
        };
        if keys.contains(&key) {
            break;
        }
        keys.push(key);
        ids.push(next.id);
        row = next;
    }
    (row, keys, ids)
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut clipped: String = line.chars().take(80).collect();
    if clipped.chars().count() < line.chars().count() {
        clipped.push('…');
    }
    clipped
}

/// A child turn a wake reports.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WokenChild {
    pub queue_id: i64,
    /// The Main agent conversation that started it (its delivery is
    /// recorded against that one).
    pub parent_key: String,
    pub child_session: String,
    /// Its line in the wake message.
    pub line: String,
    /// The chat that started a handover chain, `(queue row, session)`,
    /// recorded as reported to its parent with it (overnight 10-11 A1).
    pub also: Vec<(i64, String)>,
    /// The chain's successors' queue rows (`queue_id` the last), recorded as
    /// reported to the Main agent as their chain root; empty: no handover.
    pub chain: Vec<i64>,
}

/// One wake: today's Main agent conversation (or a new one when today has
/// none), the mode it runs in and the child turns it reports.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Wake {
    /// Today's Main agent conversation; `None` starts today's.
    pub target_key: Option<String>,
    pub session_id: Option<String>,
    pub agent: Option<String>,
    /// The model the thread's newest turn asked for
    /// (`wake_me::wake_model`), so a wake keeps a thread on a non-default
    /// model, and its native session; `None` asks for the saved default.
    pub model: Option<String>,
    pub cwd: String,
    pub mode: String,
    pub children: Vec<WokenChild>,
    /// A check-in's message (M3); `None` for a child wake.
    pub text: Option<String>,
    /// The signals a check-in reports (M3), remembered so they don't
    /// trigger the next one.
    pub signal_ids: Vec<String>,
}

impl Wake {
    /// The wake turn's message: [`WAKE_MESSAGE`] and one line per child turn.
    pub(crate) fn message(&self) -> String {
        if let Some(text) = &self.text {
            return text.clone();
        }
        let lines: Vec<&str> = self.children.iter().map(|c| c.line.as_str()).collect();
        format!("{WAKE_MESSAGE}\n\n{}", lines.join("\n"))
    }
}

/// What a wake check decided.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Decision {
    /// No ended chat to report.
    Nothing,
    /// Chats to report, but today's thread has a turn queued or running:
    /// its end, or the minute timer, checks again.
    Busy,
    /// A wake is due but today's conversation finished a turn moments ago
    /// (the person may be replying): check again after [`GRACE_SECS`].
    Later,
    /// Chats ended but a guard holds the wake back; the person sees why
    /// (overnight 10-11 A1: four wakes vanished on 10-10 with no trace).
    Skipped(WakeSkip),
    Wake(Wake),
}

/// A wake a guard held back, as the Main agent's side band shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeSkip {
    /// When the check decided it (RFC 3339).
    pub at: String,
    pub reason: String,
    /// The ended chats it would have reported.
    pub sessions: Vec<String>,
}

/// After a Main agent turn ends, how long a wake waits, so a person reading
/// that answer can reply before Hollow queues its own turn.
pub(crate) const GRACE_SECS: i64 = 60;

/// The mode a turn Hollow runs in a conversation on its own (a Main agent
/// wake or check-in, an event trigger, an automation in the chat, a
/// `sessions.wake_me` wake) starts from: the conversation's own last mode,
/// kept as it is (bypass and auto too), or Hollow's default when it recorded
/// none (`app_mcp::own_mode`). Until 2026-11-12 it was capped at
/// `acceptEdits`, so a chat in another mode fell to Accept edits whenever
/// Hollow ran a turn for it.
pub(crate) fn wake_mode(last: Option<&str>) -> String {
    crate::app_mcp::own_mode(last).to_string()
}

/// The local calendar day of an RFC 3339 time.
fn local_day(at: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

/// Whether a child turn that ended this way is news: it ran and ended
/// (`done`, `failed`, `interrupted`). One the person stopped, or that never
/// ran, is not.
fn reportable(status: &str) -> bool {
    matches!(status, "done" | "failed" | "interrupted")
}

/// The session whose end a wake reports: the last successor, else the chat.
fn end_session(c: &MainAgentChild) -> String {
    c.successors
        .last()
        .cloned()
        .unwrap_or_else(|| c.child_session.clone())
}

/// Whether to wake the Main agent now (see the module doc). `state` says
/// which child turns woke it already and how many wakes today had.
pub(crate) fn decide(vault: &Path, now: DateTime<Utc>, state: &WakeState) -> Decision {
    let Some(cwd) = main_cwd(vault) else {
        return Decision::Nothing;
    };
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    let since = now - Duration::hours(WAKE_WINDOW_HOURS);
    // A swarm's parts report together, when the swarm ends (`advance`).
    let in_swarms = swarm_rows(vault);
    let ended: Vec<MainAgentChild> = collect(vault, now, false, Some(since))
        .into_iter()
        .filter(|c| !in_swarms.contains(&c.queue_id))
        .filter(|c| !c.live && !c.reported && reportable(&c.status))
        .filter(|c| !state.woken.contains(&c.end_queue_id))
        .collect();
    if ended.is_empty() {
        return Decision::Nothing;
    }
    let skipped = |reason: String, of: &[MainAgentChild]| {
        Decision::Skipped(WakeSkip {
            at: now.to_rfc3339(),
            reason,
            sessions: of.iter().map(end_session).collect(),
        })
    };
    if state.day == today && state.count >= MAX_WAKES_PER_DAY {
        return skipped(
            format!("it was already woken {MAX_WAKES_PER_DAY} times today"),
            &ended,
        );
    }
    // The daily cap stops what Hollow runs on its own. A chat the person
    // asked for is reported past it: hearing it ended is part of the ask
    // (overnight 10-11 A1: lanes the person started counted toward the cap
    // and silenced every wake after 17:51 on 10-10).
    let ended = if over_cap(vault, now, &settings()) {
        let (asked, own): (Vec<_>, Vec<_>) = ended.into_iter().partition(|c| !c.by_hollow);
        if asked.is_empty() {
            return skipped(
                "today's cap for what it runs on its own is reached (Settings → COO)".into(),
                &own,
            );
        }
        asked
    } else {
        ended
    };
    // Newest ended first, so the summarized lines are the chats that ended
    // most recently, not the ones started most recently (OB17 review). A
    // chat with no readable end time goes last.
    let mut ended = ended;
    let ended_at = |c: &MainAgentChild| {
        c.finished_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.with_timezone(&Utc))
    };
    ended.sort_by_key(|c| std::cmp::Reverse(ended_at(c)));
    // The Main agent's current conversation (`chat_chain::main_current`, as
    // the Main agent button opens it): busy, the wake waits for its turn to
    // end, whatever day it started (11-04 12:04 AM: a wake opened a second
    // thread beside one still running past midnight).
    let target = match today_thread(vault, &cwd, now) {
        Thread::Busy => return Decision::Busy,
        Thread::JustAnswered => return Decision::Later,
        Thread::Idle(target) => target,
    };
    // Session, agent, model and mode: today's conversation's, else the
    // newest one's.
    let basis = thread_basis(vault, &cwd, target.as_deref());
    let workspaces = crate::workspaces::list(vault).unwrap_or_default();
    // The first `MAX_NOTES` lines carry a summary, their answers sharing one
    // budget; later ones keep the bare line, so a wake stays bounded however
    // many chats ended (overnight OB17).
    let summarized = ended.len().min(crate::descendants::MAX_NOTES);
    let answer_chars = crate::descendants::answer_budget(summarized);
    let unsummarized = ended.len() - summarized;
    let children = ended
        .into_iter()
        .enumerate()
        .map(|(at, c)| {
            let place = session_store::read_session(vault, &c.child_session)
                .ok()
                .and_then(|d| workspaces.iter().find(|w| w.cwd == d.summary.cwd))
                .map(|w| w.title.clone())
                .unwrap_or_else(|| NO_WORKSPACE.into());
            // A title can't start a line of its own (a fake child line).
            let mut line = format!(
                "- “{}” in {} ({}, session {}): {}",
                one_line(&c.title),
                place,
                c.agent,
                c.child_session,
                c.status
            );
            if !c.successors.is_empty() {
                line.push_str(&format!(
                    " after handing over to {}",
                    c.successors.join(" → ")
                ));
            }
            if let Some(at) = &c.finished_at {
                line.push_str(&format!(", ended {at}"));
            }
            if let Some(row) = crate::run_queue::get(vault, c.end_queue_id).ok().flatten() {
                line.push_str(&crate::descendants::handed_over(vault, &row));
                if at < summarized {
                    // What the turn that ended the chain left: its answer, files, PRs.
                    let session = end_session(&c);
                    let summary = crate::descendants::child_summary(vault, &session, &row);
                    line.push_str(&summary.text(answer_chars));
                }
            }
            if at == summarized {
                let cap = crate::descendants::MAX_NOTES;
                line.push_str(&match unsummarized - 1 {
                    0 => format!(
                        "\n  No summary for this chat (a wake summarizes {cap}); read_session has its answer."
                    ),
                    more => format!(
                        "\n  No summary for this chat and the {more} after it (a wake summarizes {cap}); read_session has their answers."
                    ),
                });
            }
            let also = if c.chain_ids.is_empty() {
                Vec::new()
            } else {
                vec![(c.queue_id, c.child_session.clone())]
            };
            WokenChild {
                queue_id: c.end_queue_id,
                parent_key: c.parent_key.clone(),
                child_session: end_session(&c),
                line: line.replace('<', "‹"),
                also,
                chain: c.chain_ids.clone(),
            }
        })
        .collect();
    Decision::Wake(Wake {
        session_id: basis.session_id,
        agent: basis.agent,
        model: basis.model,
        target_key: target,
        cwd,
        mode: basis.mode,
        children,
        text: None,
        signal_ids: Vec::new(),
    })
}

/// What the wake guard remembers between checks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct WakeState {
    /// The local day `count` is for (`YYYY-MM-DD`).
    pub day: String,
    pub count: u32,
    /// Child turns (queue rows) that already woke the Main agent.
    pub woken: Vec<i64>,
    /// The last check-in queued (M3), RFC 3339.
    pub last_pulse_at: Option<String>,
    /// The local day `pulses_today` is for.
    pub pulse_day: String,
    pub pulses_today: u32,
    /// Signals already reported in a check-in (newest last, bounded).
    pub reported_signals: Vec<String>,
    /// The last wake a guard held back, until a wake is queued.
    pub skipped: Option<WakeSkip>,
}

/// Record a wake that was queued: today's count, and its child turns, which
/// never wake it again.
pub(crate) fn record(state: &mut WakeState, wake: &Wake, today: &str) {
    if state.day != today {
        state.day = today.to_string();
        state.count = 0;
    }
    state.count += 1;
    state.skipped = None;
    state.woken.extend(wake.children.iter().map(|c| c.queue_id));
    let excess = state.woken.len().saturating_sub(WOKEN_KEPT);
    state.woken.drain(..excess);
}

fn state_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(crate::test_temp::process_dir("hollow-test-main-agent-"))
    }
    #[cfg(not(test))]
    {
        crate::instance::support_dir().map(|root| root.join("MainAgent"))
    }
}

fn state_path(vault: &Path) -> Option<PathBuf> {
    state_root().map(|root| root.join(format!("{}.json", session_store::vault_file_stem(vault))))
}

fn load_state(vault: &Path) -> WakeState {
    state_path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn atomic_write_json(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create dir {}: {e}", dir.display()))?;
    }
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("state.json");
    let tmp = path.with_file_name(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    if let Err(error) = std::fs::write(&tmp, text) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write {}: {error}", tmp.display()));
    }
    if let Err(error) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "rename {} -> {}: {error}",
            tmp.display(),
            path.display()
        ));
    }
    Ok(())
}

fn save_state(vault: &Path, state: &WakeState) {
    let Some(path) = state_path(vault) else {
        return;
    };
    match serde_json::to_string(state) {
        Ok(text) => {
            if let Err(error) = atomic_write_json(&path, &text) {
                eprintln!(
                    "[main_agent] save_state failed for {}: {error}",
                    path.display()
                );
            }
        }
        Err(error) => eprintln!("[main_agent] serialize state failed: {error}"),
    }
}

/// A wake was queued: remember it, and record each child turn as reported
/// to the conversation that started it (so that conversation's next note
/// doesn't report it again).
pub(crate) fn wake_queued(vault: &Path, wake: &Wake, now: DateTime<Utc>) {
    let mut state = load_state(vault);
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    record(&mut state, wake, &today);
    save_state(vault, &state);
    for child in &wake.children {
        if child.chain.is_empty() {
            let _ = session_store::deliver_child(
                vault,
                child.queue_id,
                &child.parent_key,
                &child.child_session,
                "wake",
                None,
            );
            continue;
        }
        // A chain: its first chat to the thread that started it, the
        // successors to the Main agent as their root (a successor's own
        // delivery belongs to the chat that handed over). `wake` as the asker
        // is no conversation, so no turn's note lists them again.
        for (id, key) in &child.also {
            let _ = session_store::deliver_child(vault, *id, &child.parent_key, key, "wake", None);
        }
        let _ = session_store::deliver_to_root(vault, CHAIN_ROOT, "wake", -1, &child.chain);
    }
}

/// Nothing is left to report: a held-back wake no longer applies.
pub(crate) fn clear_skip(vault: &Path) {
    let mut state = load_state(vault);
    if state.skipped.take().is_some() {
        save_state(vault, &state);
    }
}

/// A wake a guard held back: remember it for the side band (a repeat of the
/// same skip keeps its first time, so a check every minute writes nothing).
pub(crate) fn wake_skipped(vault: &Path, skip: WakeSkip) {
    let mut state = load_state(vault);
    let same = state
        .skipped
        .as_ref()
        .is_some_and(|s| s.reason == skip.reason && s.sessions == skip.sessions);
    if !same {
        state.skipped = Some(skip);
        save_state(vault, &state);
    }
}

/// One wake check at a time, so two chats ending together queue one wake.
static WAKE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// A later check is already scheduled ([`Decision::Later`]).
static RECHECK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A chat turn ended (`run_pump::emit_row`): wake the Main agent when a
/// chat it started ended while it is idle (see the module doc), and any
/// other conversation that asked to be woken when its children end
/// (`sessions.wake_me` `onSessionsEnd`, overnight OB10b: never a Main agent
/// thread, so it isn't woken twice).
pub(crate) fn after_chat_finished(app: &Core) {
    // A candidate build runs no background agent unless asked (instance.rs).
    if !crate::instance::background_agents_allowed() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        check(app.clone()).await;
        let config = app
            .try_state::<crate::commands::AppState>()
            .map(|s| s.config.clone());
        if let Some(config) = config {
            // Marked due once; sent now unless the parent is busy (then
            // the minute timer's `run_due` sends it).
            let vault = config.vault_path.clone();
            let due = tauri::async_runtime::spawn_blocking(move || {
                crate::wake_me::arm_ended(&vault, Utc::now())
            })
            .await
            .unwrap_or(0);
            if due > 0 {
                crate::wake_me::run_due(&app, &config).await;
            }
            advance(&app, &config).await;
            crate::stakeholder_sim::advance(&app, &config).await;
            // LF1: a child that ended without a verdict gets its one retry.
            crate::child_verdict::after_chat_finished(&app, &config).await;
        }
    });
}

/// One minute's COO pass for `role` (M3b, `runner_coo`): the wake check,
/// then the check-in where the role checks in. The app (`Single`) runs both,
/// as before; `hollow serve` (runner or not) only the wake check.
pub(crate) async fn coo_pass(app: &Core, role: crate::runner_coo::Role) {
    check(app.clone()).await;
    if role.check_ins() {
        check_pulse(app.clone()).await;
    }
}

async fn check(app: Core) {
    use std::sync::atomic::Ordering;
    let Some(config) = app
        .try_state::<crate::commands::AppState>()
        .map(|s| s.config.clone())
    else {
        return;
    };
    let _guard = WAKE_LOCK.lock().await;
    let vault = config.vault_path.clone();
    let decision = tauri::async_runtime::spawn_blocking(move || {
        decide(&vault, Utc::now(), &load_state(&vault))
    })
    .await
    .unwrap_or(Decision::Nothing);
    match decision {
        Decision::Busy => {}
        Decision::Nothing => {
            let vault = config.vault_path.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || clear_skip(&vault)).await;
        }
        Decision::Skipped(skip) => {
            let vault = config.vault_path.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || wake_skipped(&vault, skip)).await;
        }
        Decision::Later => {
            if !RECHECK.swap(true, Ordering::SeqCst) {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(GRACE_SECS as u64 + 15))
                        .await;
                    RECHECK.store(false, Ordering::SeqCst);
                    after_chat_finished(&app);
                });
            }
        }
        Decision::Wake(wake) => {
            // Recorded only once queued: a refused send wakes it later.
            if queue_wake(&app, &config, &wake, ASKED_BY_WAKE).await {
                let vault = config.vault_path.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    wake_queued(&vault, &wake, Utc::now())
                })
                .await;
            }
        }
    }
}

// ── Check-ins (M3) ─────────────────────────────────────────────────────

/// The Main agent's settings (Settings, Background work, COO), from the
/// decisions of 2026-10-07: check in every 30 minutes, 8:00 to 18:00 on
/// working days, stop at $100 a day, act on safe classes, swarms of at most 6.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MainAgentSettings {
    /// Timed check-ins (the pulse).
    pub check_ins: bool,
    pub interval_min: u32,
    /// Local hours: from `start_hour` up to (not including) `end_hour`.
    pub start_hour: u32,
    pub end_hour: u32,
    pub weekdays_only: bool,
    /// Hard stop for everything the Main agent runs on its own today
    /// (check-ins, wakes, swarm chats it launches), in US dollars.
    pub daily_cap_usd: f64,
    /// Unprompted turns may act on safe classes (M4); off: suggest only.
    pub act_on_own: bool,
    /// Most chats one swarm may start (M5).
    pub swarm_max: u32,
    /// The model a swarm's parts run on unless a part names its own; `""`
    /// runs them on the agent's own default. Claude Haiku 5.5 by default
    /// (2026-10-07: $0.10 / $0.50 per million tokens, about 40× below Opus 5.5).
    pub swarm_model: String,
    /// Swarm parts that run at once, within the app-wide chat limits they
    /// share with every other turn (2026-10-20); build-workspace parts stay
    /// at two (the toolchain slots).
    pub swarm_concurrency: u32,
    /// Overnight 9 A9: a Main-agent chat holding 350K or more starts a
    /// fresh session (with the conversation and its task state) when it
    /// comes back after more than an hour, instead of re-reading it all
    /// uncached (`chat_resume::rests`). On by default; absent in an older
    /// file reads as on.
    pub fresh_after_away: bool,
}

/// Swarm parts' default model when Claude Code never said what `haiku`
/// resolves to (Claude Haiku 5.5, released 2026-10-07).
pub(crate) const SWARM_MODEL: &str = crate::agent_models::haiku_fallback_id!();

/// Swarm parts' default model: the newest Haiku as Claude Code resolves
/// it (same family and price tier), so settings that were never changed
/// follow a new Haiku. A model the person saved stays as saved; Settings
/// flags it when its family has a newer one.
pub(crate) fn default_swarm_model() -> String {
    crate::agent_models::newest_claude("haiku").unwrap_or_else(|| SWARM_MODEL.into())
}
/// The largest swarm Settings allow.
pub(crate) const MAX_SWARM: u32 = 100;
/// The most swarm parts Settings let run at once (each is a CLI process
/// with its servers; this machine's guard counts node processes).
pub(crate) const MAX_SWARM_CONCURRENCY: u32 = 10;

impl Default for MainAgentSettings {
    fn default() -> Self {
        Self {
            check_ins: true,
            interval_min: 30,
            start_hour: 8,
            end_hour: 18,
            weekdays_only: true,
            daily_cap_usd: 100.0,
            act_on_own: true,
            swarm_max: 6,
            swarm_model: default_swarm_model(),
            swarm_concurrency: 6,
            fresh_after_away: true,
        }
    }
}

impl MainAgentSettings {
    /// Settings a person could have meant, or why not.
    pub(crate) fn validated(self) -> Result<Self, String> {
        if !(15..=240).contains(&self.interval_min) {
            return Err("Check in every 15 to 240 minutes.".into());
        }
        if self.start_hour >= self.end_hour || self.end_hour > 24 {
            return Err("Check-in hours must start before they end, within the day.".into());
        }
        if !self.daily_cap_usd.is_finite() || !(0.0..=1000.0).contains(&self.daily_cap_usd) {
            return Err("The daily cap must be between $0 and $1,000.".into());
        }
        if !(1..=MAX_SWARM).contains(&self.swarm_max) {
            return Err(format!("A swarm may start 1 to {MAX_SWARM} chats."));
        }
        if !(1..=MAX_SWARM_CONCURRENCY).contains(&self.swarm_concurrency) {
            return Err(format!(
                "Run 1 to {MAX_SWARM_CONCURRENCY} swarm parts at once."
            ));
        }
        let model = self.swarm_model.trim();
        if model.len() > 100 || model.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("The swarm model is a model id like claude-haiku-5-5, or empty for the agent's default.".into());
        }
        Ok(Self {
            swarm_model: model.to_string(),
            ..self
        })
    }
}

fn settings_path() -> Option<PathBuf> {
    state_root().map(|root| root.join("settings.json"))
}

/// The saved swarm limit, failing closed: a settings file Hollow can't read
/// or validate is an error, never the wider default (#1068 review). No file
/// is the default.
pub(crate) fn saved_swarm_max() -> Result<u32, String> {
    match settings_path() {
        Some(path) => swarm_max_from(std::fs::read_to_string(path)),
        None => Ok(MainAgentSettings::default().swarm_max),
    }
}

fn swarm_max_from(read: std::io::Result<String>) -> Result<u32, String> {
    let unknown = "The COO settings can't be read, so the swarm limit is unknown. Check Settings → Background work → COO → Largest swarm (chats). Nothing started.";
    match read {
        Ok(text) => serde_json::from_str::<MainAgentSettings>(&text)
            .ok()
            .and_then(|s| s.validated().ok())
            .map(|s| s.swarm_max)
            .ok_or_else(|| unknown.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(MainAgentSettings::default().swarm_max)
        }
        Err(_) => Err(unknown.into()),
    }
}

/// The saved settings, else the defaults.
pub(crate) fn settings() -> MainAgentSettings {
    settings_from(
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .as_deref(),
    )
}

/// Settings from a saved file's text (`None`: no file, the defaults). A file
/// Hollow can't use never widens what it may do: the defaults, but
/// suggesting only and without timed check-ins.
pub(crate) fn settings_from(text: Option<&str>) -> MainAgentSettings {
    let Some(text) = text else {
        return MainAgentSettings::default();
    };
    serde_json::from_str::<MainAgentSettings>(text)
        .ok()
        .and_then(|s| s.validated().ok())
        .unwrap_or(MainAgentSettings {
            act_on_own: false,
            check_ins: false,
            ..MainAgentSettings::default()
        })
}

fn save_settings(s: &MainAgentSettings) -> Result<(), String> {
    let path = settings_path().ok_or("No app data folder.")?;
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    atomic_write_json(&path, &text).inspect_err(|error| {
        eprintln!(
            "[main_agent] save_settings failed for {}: {error}",
            path.display()
        );
    })
}

/// Local midnight of `now`'s day.
pub(crate) fn day_start(now: DateTime<Utc>) -> DateTime<Utc> {
    crate::usage_ledger::window_bounds(
        crate::usage_ledger::UsageWindow::Day,
        now.with_timezone(&Local),
    )
    .start
}

/// Whether the day's hard stop is reached.
pub(crate) fn over_cap(vault: &Path, now: DateTime<Utc>, s: &MainAgentSettings) -> bool {
    crate::budgets::main_agent_spend(vault, now, s.daily_cap_usd).over
}

/// Meetings starting within the hour that have no prep pack yet, as
/// check-in signals (overnight 10-11 A4: the standing order "Meetings in the
/// next hour"). Filled by [`refresh_upcoming`] only when a check-in may run,
/// so the calendar is read at most once per check-in interval.
static UPCOMING: std::sync::Mutex<Vec<Signal>> = std::sync::Mutex::new(Vec::new());

/// Whether a timed check-in could run now: on, in hours, the interval passed.
fn pulse_window_open(now: DateTime<Utc>, s: &MainAgentSettings, state: &WakeState) -> bool {
    s.check_ins
        && in_hours(now.with_timezone(&Local), s)
        && state
            .last_pulse_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .is_none_or(|at| {
                (now - at.with_timezone(&Utc)).num_minutes() >= i64::from(s.interval_min)
            })
}

/// The signal lines for meetings in the next hour without a prep pack.
pub(crate) fn meeting_signals(
    vault: &Path,
    meetings: &[crate::meeting_prep::Meeting],
) -> Vec<Signal> {
    meetings
        .iter()
        .filter(|m| !m.id.is_empty() && m.minutes_until <= 60 && !crate::prep_pack::exists(vault, m))
        .map(|m| {
            let at = DateTime::parse_from_rfc3339(&m.start)
                .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
                .unwrap_or_else(|_| m.start.clone());
            Signal {
                id: format!("meeting-prep:{}", m.id),
                line: format!(
                    "- “{}” starts at {at} (in {} min) with no prep pack. Standing order “Meetings in the next hour”: run agenda.prepare_meeting {{\"meetingId\": \"{}\"}} if you act on your own, then tell the person where the pack is; otherwise suggest it.",
                    one_line(&m.subject).replace('<', "‹"),
                    m.minutes_until.max(0),
                    m.id
                ),
            }
        })
        .collect()
}

/// The calendar is read for check-ins at most this often: a check-in that
/// finds nothing new leaves the window open every minute.
const UPCOMING_EVERY: std::time::Duration = std::time::Duration::from_secs(10 * 60);
static UPCOMING_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Whether the calendar may be read again now (and marks it read).
fn upcoming_due(now: std::time::Instant) -> bool {
    let mut at = UPCOMING_AT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if at.is_some_and(|t| now.duration_since(t) < UPCOMING_EVERY) {
        return false;
    }
    *at = Some(now);
    true
}

/// Read the next hour's meetings for the check-in about to be decided, at
/// most every [`UPCOMING_EVERY`].
async fn refresh_upcoming(vault: &Path) {
    if !upcoming_due(std::time::Instant::now()) {
        return;
    }
    let report = crate::meeting_prep::for_this_mac(vault, 1).await;
    let signals = meeting_signals(vault, &report.meetings);
    *UPCOMING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = signals;
}

/// One thing a check-in can notice, without spending a token.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Signal {
    pub id: String,
    pub line: String,
}

/// What a check-in would notice now: things waiting on the person (except
/// the Main agent's own waiting suggestions), the current nudges (overdue
/// tasks and commitments, cold projects) and runs that failed since `since`.
pub(crate) fn signals(vault: &Path, now: DateTime<Utc>, since: DateTime<Utc>) -> Vec<Signal> {
    let mut out = Vec::new();
    if let Ok(page) = crate::needs_you::list(vault, Vec::new(), None, None, Some(20)) {
        for item in page.items {
            if item.workspace_id.as_deref() == Some(crate::workspaces::MAIN_AGENT_ID) {
                continue;
            }
            // Resuming interrupted chats is the person's click only
            // (overnight A1): a check-in never acts on it.
            if item.kind == crate::needs_you::INTERRUPTED_KIND {
                continue;
            }
            // Conflict copies are the person's to resolve (overnight A10).
            if item.kind == crate::needs_you::COPIES_KIND {
                continue;
            }
            // So are context sync clash cards (CX6c).
            if item.kind == crate::needs_you::CLASH_KIND {
                continue;
            }
            // Deleting a lesson's memory is the person's answer (L2).
            if item.kind == crate::lesson_memories::KIND {
                continue;
            }
            out.push(Signal {
                id: format!("need:{}", item.id),
                line: format!(
                    "- Waiting on you ({}): {}",
                    item.kind,
                    one_line(&item.title)
                ),
            });
        }
    }
    for (key, title, body) in crate::nudges::current(vault, now) {
        out.push(Signal {
            id: format!("nudge:{key}"),
            line: format!("- {} ({})", one_line(&title), one_line(&body)),
        });
    }
    if let Ok(rows) = crate::run_queue::list(vault, 30) {
        // The chats this launch offers to resume are the person's item, and
        // one resumed or continued since is handled (overnight A1 and S5):
        // neither is a failure to act on. Every other interrupted row (lost,
        // unattended, a helper's, nobody went back to) is still reported.
        let offered: Vec<String> = crate::restart_resume::offered(vault, now)
            .unwrap_or_default()
            .into_iter()
            .map(|c| c.chat.session_key)
            .collect();
        for (row, _) in rows {
            let failed = matches!(row.status.as_str(), "failed" | "interrupted");
            let recent = row
                .finished_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .is_some_and(|at| at.with_timezone(&Utc) >= since);
            // The Main agent's own turns aren't news to it.
            let own = row.source_id.as_deref().is_some_and(|key| {
                session_store::read_session(vault, key)
                    .is_ok_and(|d| main_cwd(vault).as_deref() == Some(d.summary.cwd.as_str()))
            });
            let cut_short = row.source == crate::run_queue::RunSource::Chat
                && row.status.as_str() == "interrupted"
                && row.source_id.as_deref().is_some_and(|key| {
                    offered.iter().any(|o| o == key)
                        || crate::run_queue::newest_chat_row(vault, key)
                            .ok()
                            .flatten()
                            .is_some_and(|newest| newest.id > row.id)
                });
            if failed && recent && !own && !cut_short {
                let why = row.error.as_deref().map(one_line).unwrap_or_default();
                out.push(Signal {
                    id: format!("failed:{}", row.id),
                    line: format!("- A run failed: {} {}", one_line(&row.label), why),
                });
            }
        }
    }
    // Meetings in the next hour with no prep pack (overnight 10-11 A4),
    // read by the async check just before this one.
    out.extend(
        UPCOMING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned(),
    );
    out
}

fn one_line(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut clipped: String = flat.chars().take(140).collect();
    if clipped.chars().count() < flat.chars().count() {
        clipped.push('…');
    }
    clipped.replace('<', "‹")
}

/// Signals reported in check-ins, remembered.
const REPORTED_KEPT: usize = 512;
/// How far back a check-in looks for failed runs when it has none before.
const FIRST_LOOKBACK_HOURS: i64 = 12;

/// Whether the clock allows a check-in now.
pub(crate) fn in_hours(now: DateTime<Local>, s: &MainAgentSettings) -> bool {
    use chrono::{Datelike, Timelike, Weekday};
    if s.weekdays_only && matches!(now.weekday(), Weekday::Sat | Weekday::Sun) {
        return false;
    }
    (s.start_hour..s.end_hour).contains(&now.hour())
}

/// Whether to check in now, and with what (see the module doc). A check-in
/// with nothing new spends nothing; the day's first runs anyway, as the
/// morning brief.
pub(crate) fn decide_pulse(
    vault: &Path,
    now: DateTime<Utc>,
    s: &MainAgentSettings,
    state: &WakeState,
) -> Decision {
    let local = now.with_timezone(&Local);
    if !s.check_ins || !in_hours(local, s) {
        return Decision::Nothing;
    }
    let Some(cwd) = main_cwd(vault) else {
        return Decision::Nothing;
    };
    let last = state
        .last_pulse_at
        .as_deref()
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|at| at.with_timezone(&Utc));
    if last.is_some_and(|at| (now - at).num_minutes() < i64::from(s.interval_min)) {
        return Decision::Nothing;
    }
    let today = local.format("%Y-%m-%d").to_string();
    let first_today = state.pulse_day != today || state.pulses_today == 0;
    let since = last.unwrap_or(now - Duration::hours(FIRST_LOOKBACK_HOURS));
    let fresh: Vec<Signal> = signals(vault, now, since)
        .into_iter()
        .filter(|sig| !state.reported_signals.contains(&sig.id))
        .collect();
    if fresh.is_empty() && !first_today {
        return Decision::Nothing;
    }
    if over_cap(vault, now, s) {
        return Decision::Nothing;
    }
    let target = match today_thread(vault, &cwd, now) {
        Thread::Busy => return Decision::Nothing,
        Thread::JustAnswered => return Decision::Later,
        Thread::Idle(target) => target,
    };
    let mut text = format!("{CHECKIN_OPEN} {}.", local.format("%H:%M"));
    if first_today {
        text.push_str(
            " This is the first check-in today: open with a brief of the day in three lines.",
        );
    }
    if fresh.is_empty() {
        text.push_str(" Nothing new came up since the last check-in.");
    } else {
        text.push_str(CHECKIN_NEW);
        text.push_str(
            &fresh
                .iter()
                .map(|sig| sig.line.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let never = never_list(&load_suggestions(vault));
    if !never.is_empty() {
        text.push_str("\n\nDeclined for good, never suggest again:\n");
        text.push_str(
            &never
                .iter()
                .map(|n| format!("- {n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    text.push_str("\n\n");
    text.push_str(PULSE_GUIDANCE);
    let basis = thread_basis(vault, &cwd, target.as_deref());
    Decision::Wake(Wake {
        target_key: target,
        session_id: basis.session_id,
        agent: basis.agent,
        model: basis.model,
        cwd,
        mode: basis.mode,
        children: Vec::new(),
        text: Some(text),
        signal_ids: fresh.into_iter().map(|sig| sig.id).collect(),
    })
}

/// What a check-in turn is told to do with what it noticed.
pub(crate) const PULSE_GUIDANCE: &str = "Decide what needs the person. If nothing does, reply with exactly PULSE_OK and nothing else. Otherwise answer in a few short lines, then offer at most three suggestions as question cards: one ask_user_questions call (or one hollow-questions block, where that is how you ask) with a single-choice question per suggestion, its options Do it, Not now and Never, the one you recommend marked; then end your turn. Act on your own only as far as your standing orders and this run allow; ask in a suggestion for anything more.";

/// Today's Main agent thread, as a check-in or a wake sees it.
enum Thread {
    /// It has a turn queued or running.
    Busy,
    /// Its turn ended moments ago.
    JustAnswered,
    /// Idle: its key, or `None` when today has no thread yet.
    Idle(Option<String>),
}

/// The Main agent's thread for a Hollow turn: its current conversation
/// (the newest, `chat_chain::main_current`) whatever day it started while it
/// has a turn queued or running or just answered, so a wake or check-in
/// never opens a second thread beside it (one Main conversation,
/// 2026-11-07); idle, today's, or `None` when it started on an earlier day
/// (the daily soft reset: today's thread continues from it, `continued_for`).
fn today_thread(vault: &Path, cwd: &str, now: DateTime<Utc>) -> Thread {
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    let Some((key, created, _)) = session_store::newest_session_in(vault, cwd).ok().flatten()
    else {
        return Thread::Idle(None);
    };
    if let Some(row) = crate::run_queue::newest_chat_row(vault, &key)
        .ok()
        .flatten()
    {
        if row.status.is_live() {
            return Thread::Busy;
        }
        let recent = row
            .finished_at
            .as_deref()
            .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
            .is_some_and(|at| (now - at.with_timezone(&Utc)).num_seconds() < GRACE_SECS);
        if recent {
            return Thread::JustAnswered;
        }
    }
    let todays = local_day(&created).as_deref() == Some(today.as_str());
    Thread::Idle(todays.then_some(key))
}

/// What a Hollow turn in a Main agent thread runs with ([`thread_basis`]).
#[derive(Debug, Clone, PartialEq)]
struct ThreadBasis {
    /// The target's native session (none for today's new thread).
    session_id: Option<String>,
    agent: Option<String>,
    /// The model the basis thread's newest turn asked for
    /// (`wake_me::wake_model`). It was always `None`, the saved default, so
    /// a thread on another model got a fresh native session on every wake
    /// (the model is in the resume fingerprint).
    model: Option<String>,
    /// The thread's own mode ([`wake_mode`]).
    mode: String,
}

/// The native session, agent, model and mode a turn in `target` (or,
/// with none, in today's new thread) runs with: the target's own, else the
/// newest Main agent conversation's.
fn thread_basis(vault: &Path, cwd: &str, target: Option<&str>) -> ThreadBasis {
    let basis = target.map(str::to_string).or_else(|| {
        session_store::newest_session_in(vault, cwd)
            .ok()
            .flatten()
            .map(|(key, _, _)| key)
    });
    let last = basis
        .as_deref()
        .and_then(|key| session_store::last_permission(vault, key).ok().flatten());
    let summary = basis
        .as_deref()
        .and_then(|key| session_store::read_session(vault, key).ok())
        .map(|d| d.summary);
    let agent = summary.as_ref().map(|s| s.agent.clone());
    let model = basis
        .as_deref()
        .zip(agent.as_deref())
        .and_then(|(key, agent)| crate::wake_me::wake_model(vault, key, agent));
    ThreadBasis {
        session_id: target.and(summary.as_ref().and_then(|s| s.harness_session_id.clone())),
        agent,
        model,
        mode: wake_mode(last.as_deref()),
    }
}

/// A check-in was queued: remember when, how many today, and what it reported.
pub(crate) fn pulse_queued(vault: &Path, wake: &Wake, now: DateTime<Utc>) {
    let mut state = load_state(vault);
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    if state.pulse_day != today {
        state.pulse_day = today;
        state.pulses_today = 0;
    }
    state.pulses_today += 1;
    state.last_pulse_at = Some(now.to_rfc3339());
    state
        .reported_signals
        .extend(wake.signal_ids.iter().cloned());
    let excess = state.reported_signals.len().saturating_sub(REPORTED_KEPT);
    state.reported_signals.drain(..excess);
    save_state(vault, &state);
}

/// The check-in timer: every minute, one [`decide_pulse`] (the gate itself
/// spends no tokens). Queue owner only, and never in a candidate build
/// unless asked (`instance::background_agents_allowed`).
pub(crate) fn start_timer(app: impl Into<Core>) {
    let app = app.into();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            // A wake missed by a restart, a refused send or a busy thread
            // (overnight 10-11 A1): the check is reads only, and a chat's end
            // still runs one at once (`after_chat_finished`).
            coo_pass(&app, crate::runner_coo::role()).await;
            // A finished swarm whose synthesis waited on a busy thread.
            let config = app
                .try_state::<crate::commands::AppState>()
                .map(|s| s.config.clone());
            if let Some(config) = config {
                advance(&app, &config).await;
                crate::stakeholder_sim::advance(&app, &config).await;
                // A wake on children's ends whose children ended without a
                // `finished` event (cancelled while queued, interrupted at
                // startup) is marked due here (overnight OB10b): one file
                // read, children checked only for waiting wakes.
                let vault = config.vault_path.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    crate::wake_me::arm_ended(&vault, Utc::now())
                })
                .await;
                crate::wake_me::run_due(&app, &config).await;
                if let Some(state) = app.try_state::<crate::commands::AppState>() {
                    let config_path = state.config_path.clone();
                    crate::disk_guard::check_daily(&app, &config.vault_path, &config_path).await;
                }
            }
        }
    });
}

async fn check_pulse(app: Core) {
    let Some(config) = app
        .try_state::<crate::commands::AppState>()
        .map(|s| s.config.clone())
    else {
        return;
    };
    // The calendar only when a check-in may run (once per interval).
    if pulse_window_open(Utc::now(), &settings(), &load_state(&config.vault_path)) {
        refresh_upcoming(&config.vault_path).await;
    }
    let _guard = WAKE_LOCK.lock().await;
    let vault = config.vault_path.clone();
    let decision = tauri::async_runtime::spawn_blocking(move || {
        decide_pulse(&vault, Utc::now(), &settings(), &load_state(&vault))
    })
    .await
    .unwrap_or(Decision::Nothing);
    if let Decision::Wake(wake) = decision {
        if queue_wake(&app, &config, &wake, ASKED_BY_CHECKIN).await {
            let vault = config.vault_path.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                pulse_queued(&vault, &wake, Utc::now())
            })
            .await;
        }
    }
}

/// What [`queue_hollow_turn`] did.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TurnQueued {
    Queued,
    /// Today's thread is busy or was just answered: try again later.
    Busy,
    /// Not now and not later (no Main agent workspace, the day's cap).
    Skipped(&'static str),
}

/// Queue one Hollow turn (`text` starts with [`WAKE_PREFIX`]) in today's
/// Main agent thread, as a check-in is: unattended, in the thread's own
/// mode ([`wake_mode`]), never past the day's cap, never over a busy thread.
/// Used by host window jobs (the weekly update's prose pass). `asker` is the
/// turn's `asked_by` tag (an `ASKED_BY_*`).
pub(crate) async fn queue_hollow_turn(
    app: &impl crate::core_host::CoreHost,
    config: &crate::config::Config,
    text: String,
    asker: &'static str,
) -> TurnQueued {
    queue_hollow_turn_as(&app.core(), config, text, false, asker).await
}

async fn queue_hollow_turn_as(
    app: &Core,
    config: &crate::config::Config,
    text: String,
    isolated: bool,
    asker: &'static str,
) -> TurnQueued {
    let _guard = WAKE_LOCK.lock().await;
    let vault = config.vault_path.clone();
    let decided = tauri::async_runtime::spawn_blocking(move || {
        let now = Utc::now();
        let Some(cwd) = main_cwd(&vault) else {
            return Err(TurnQueued::Skipped("no COO workspace"));
        };
        if over_cap(&vault, now, &settings()) {
            return Err(TurnQueued::Skipped("the COO's daily cap is reached"));
        }
        let target = queue_target(&vault, &cwd, now)?;
        let basis = thread_basis(&vault, &cwd, target.as_deref());
        Ok(Wake {
            target_key: target,
            session_id: basis.session_id,
            agent: basis.agent,
            model: basis.model,
            cwd,
            mode: basis.mode,
            children: Vec::new(),
            text: Some(text),
            signal_ids: Vec::new(),
        })
    })
    .await;
    match decided {
        Ok(Ok(wake)) => {
            if queue_wake_as(app, config, &wake, isolated, asker).await {
                TurnQueued::Queued
            } else {
                TurnQueued::Busy
            }
        }
        Ok(Err(not_now)) => not_now,
        Err(_) => TurnQueued::Busy,
    }
}

/// The thread a host window job's turn goes to ([`today_thread`]): busy or
/// just answered, try again later.
fn queue_target(vault: &Path, cwd: &str, now: DateTime<Utc>) -> Result<Option<String>, TurnQueued> {
    match today_thread(vault, cwd, now) {
        Thread::Busy | Thread::JustAnswered => Err(TurnQueued::Busy),
        Thread::Idle(target) => Ok(target),
    }
}

/// Queue a wake or check-in turn, tagged `asker` (an `ASKED_BY_*`); whether
/// it was accepted.
async fn queue_wake(
    app: &Core,
    config: &crate::config::Config,
    wake: &Wake,
    asker: &'static str,
) -> bool {
    queue_wake_as(app, config, wake, false, asker).await
}

/// The earlier Main agent thread a wake that starts today's continues from
/// (OB19): the newest Main agent conversation, when it was started before
/// `now`'s local day. None when the wake goes to today's thread, or there
/// is no earlier one. A wake opened a new day's thread with no link to
/// yesterday's, so an overnight programme split at midnight (all ten new
/// Main agent manifests of 10-16/17: `taskState.merged` 0).
fn continued_for(vault: &Path, wake: &Wake, now: DateTime<Utc>) -> Vec<String> {
    if wake.target_key.is_some() {
        return Vec::new();
    }
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    session_store::newest_session_in(vault, &wake.cwd)
        .ok()
        .flatten()
        .filter(|(_, created, _)| local_day(created).is_some_and(|day| day < today))
        .map(|(key, _, _)| key)
        .into_iter()
        .collect()
}

/// The send for `wake`: depth 0, never typed, tagged `asker` (OB20), and
/// continuing `continued_from` ([`continued_for`]): as Hollow's own turn
/// (`host_asker`), it gets that thread's task state and key, not its turns.
/// The phone API's push tests queue a wake through it too.
pub(crate) fn wake_input(
    wake: &Wake,
    isolated: bool,
    asker: &'static str,
    continued_from: Vec<String>,
) -> crate::commands::ChatSendInput {
    crate::commands::ChatSendInput {
        prompt: wake.message(),
        session_id: wake.session_id.clone(),
        session_key: wake.target_key.clone(),
        cwd: Some(wake.cwd.clone()),
        model: wake.model.clone(),
        permission_mode: Some(wake.mode.clone()),
        agent: wake.agent.clone(),
        client_turn_id: None,
        app_depth: 0,
        ultracode: false,
        canvases: None,
        continued_from,
        unattended: isolated,
        typed: false,
        host_asker: Some(asker),
        canned: None,
        worker: false,
    }
}

/// [`queue_wake`]; `isolated` queues it as an unattended turn (no shared
/// connectors, teammates, computer control or services), for a wake that
/// carries text other runs wrote (master plan #1's morning review).
async fn queue_wake_as(
    app: &Core,
    config: &crate::config::Config,
    wake: &Wake,
    isolated: bool,
    asker: &'static str,
) -> bool {
    let continued_from = if wake.target_key.is_none() {
        let vault = config.vault_path.clone();
        let owned = wake.clone();
        tauri::async_runtime::spawn_blocking(move || continued_for(&vault, &owned, Utc::now()))
            .await
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let input = wake_input(wake, isolated, asker, continued_from);
    match crate::commands::send_chat_on(Some(app), config, input, None, None).await {
        Ok(accepted) => {
            unattended::register(accepted.queue_id);
            true
        }
        Err(error) => {
            eprintln!("[COO] wake: {error}");
            false
        }
    }
}

/// The Main agent's settings.
#[tauri::command]
pub fn get_main_agent_settings() -> MainAgentSettings {
    settings()
}

/// Save the Main agent's settings (Settings, Background work, the whole record).
/// The COO's route is [`agent_set_settings`], behind a confirm card.
#[tauri::command]
pub fn set_main_agent_settings(settings: MainAgentSettings) -> Result<MainAgentSettings, String> {
    let settings = settings.validated()?;
    save_settings(&settings)?;
    Ok(settings)
}

/// The fields a `settings.set_main_agent_settings` call may change; a field
/// left out keeps its saved value.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SettingsPatch {
    check_ins: Option<bool>,
    interval_min: Option<u32>,
    start_hour: Option<u32>,
    end_hour: Option<u32>,
    weekdays_only: Option<bool>,
    daily_cap_usd: Option<f64>,
    act_on_own: Option<bool>,
    swarm_max: Option<u32>,
    swarm_model: Option<String>,
    swarm_concurrency: Option<u32>,
    fresh_after_away: Option<bool>,
}

/// `base` with the patch's fields, validated as Settings validates a save.
pub(crate) fn patched(
    base: MainAgentSettings,
    patch: SettingsPatch,
) -> Result<MainAgentSettings, String> {
    MainAgentSettings {
        check_ins: patch.check_ins.unwrap_or(base.check_ins),
        interval_min: patch.interval_min.unwrap_or(base.interval_min),
        start_hour: patch.start_hour.unwrap_or(base.start_hour),
        end_hour: patch.end_hour.unwrap_or(base.end_hour),
        weekdays_only: patch.weekdays_only.unwrap_or(base.weekdays_only),
        daily_cap_usd: patch.daily_cap_usd.unwrap_or(base.daily_cap_usd),
        act_on_own: patch.act_on_own.unwrap_or(base.act_on_own),
        swarm_max: patch.swarm_max.unwrap_or(base.swarm_max),
        swarm_model: patch.swarm_model.unwrap_or(base.swarm_model),
        swarm_concurrency: patch.swarm_concurrency.unwrap_or(base.swarm_concurrency),
        fresh_after_away: patch.fresh_after_away.unwrap_or(base.fresh_after_away),
    }
    .validated()
}

/// The patch an action's arguments make.
pub(crate) fn patch_from(args: serde_json::Value) -> Result<SettingsPatch, String> {
    serde_json::from_value(args)
        .map_err(|e| format!("the COO settings don't fit: {e}. Nothing changed."))
}

/// What changing `old` to `new` shows on a card, one line per field that
/// differs, and a last line when it widens what the COO does on its own.
pub(crate) fn settings_diff(old: &MainAgentSettings, new: &MainAgentSettings) -> Vec<String> {
    fn on(v: bool) -> &'static str {
        if v {
            "on"
        } else {
            "off"
        }
    }
    let mut lines = Vec::new();
    if old.check_ins != new.check_ins {
        lines.push(format!(
            "Check-ins: {} → {}",
            on(old.check_ins),
            on(new.check_ins)
        ));
    }
    if old.interval_min != new.interval_min {
        lines.push(format!(
            "Check in every: {} → {} minutes",
            old.interval_min, new.interval_min
        ));
    }
    if old.start_hour != new.start_hour || old.end_hour != new.end_hour {
        lines.push(format!(
            "Hours: {}:00 to {}:00 → {}:00 to {}:00",
            old.start_hour, old.end_hour, new.start_hour, new.end_hour
        ));
    }
    if old.weekdays_only != new.weekdays_only {
        lines.push(format!(
            "Weekdays only: {} → {}",
            on(old.weekdays_only),
            on(new.weekdays_only)
        ));
    }
    if old.daily_cap_usd != new.daily_cap_usd {
        lines.push(format!(
            "Daily cap: ${} → ${}",
            old.daily_cap_usd, new.daily_cap_usd
        ));
    }
    if old.act_on_own != new.act_on_own {
        lines.push(format!(
            "Acts on its own: {} → {}",
            on(old.act_on_own),
            on(new.act_on_own)
        ));
    }
    if old.swarm_max != new.swarm_max {
        lines.push(format!(
            "Largest swarm: {} → {} chats",
            old.swarm_max, new.swarm_max
        ));
    }
    if old.swarm_model != new.swarm_model {
        lines.push(format!(
            "Swarm model: {} → {}",
            if old.swarm_model.is_empty() {
                "the agent's default"
            } else {
                old.swarm_model.as_str()
            },
            if new.swarm_model.is_empty() {
                "the agent's default"
            } else {
                new.swarm_model.as_str()
            }
        ));
    }
    if old.swarm_concurrency != new.swarm_concurrency {
        lines.push(format!(
            "Swarm parts at once: {} → {}",
            old.swarm_concurrency, new.swarm_concurrency
        ));
    }
    if old.fresh_after_away != new.fresh_after_away {
        lines.push(format!(
            "Fresh session after time away: {} → {}",
            on(old.fresh_after_away),
            on(new.fresh_after_away)
        ));
    }
    let widens = new.daily_cap_usd > old.daily_cap_usd
        || (new.act_on_own && !old.act_on_own)
        || (new.check_ins && !old.check_ins)
        || new.swarm_max > old.swarm_max
        || new.swarm_concurrency > old.swarm_concurrency;
    if widens && !lines.is_empty() {
        lines.push("This widens what the COO may do and spend on its own.".into());
    }
    lines
}

/// `settings.set_main_agent_settings`'s card: old → new, refused when the
/// values are invalid or nothing would change.
pub(crate) fn agent_settings_card(
    _cx: &crate::app_actions::Cx,
    a: &crate::app_actions::Args,
) -> Result<Option<crate::app_actions::Card>, String> {
    let old = settings();
    let new = patched(
        old.clone(),
        patch_from(serde_json::Value::Object(a.0.clone()))?,
    )?;
    let lines = settings_diff(&old, &new);
    if lines.is_empty() {
        return Err("Those are already the COO's settings. Nothing changed.".into());
    }
    let mut card = crate::app_actions::Card::new("Hollow app tools: change the COO's settings");
    for line in lines {
        card = card.line(line);
    }
    Ok(Some(card))
}

/// `settings.set_main_agent_settings`: save the patched settings. The
/// COO's own limits: only after the person allowed the card.
pub(crate) fn agent_set_settings(args: serde_json::Value) -> Result<MainAgentSettings, String> {
    let next = patched(settings(), patch_from(args)?)?;
    save_settings(&next)?;
    Ok(next)
}

/// What the Main agent's header shows: today's spend against the cap, and
/// the last and next check-in.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MainAgentStatus {
    /// What it ran on its own today (wakes, check-ins, chats those started,
    /// swarms): what the daily cap stops.
    pub spent_today_usd: f64,
    /// Its turns and chats the person asked for today: shown, never capped.
    pub asked_today_usd: f64,
    /// The last wake a guard held back today, and why.
    pub wake_skipped: Option<WakeSkip>,
    pub daily_cap_usd: f64,
    pub check_ins: bool,
    pub last_check_in: Option<String>,
    pub next_check_in: Option<String>,
    pub check_ins_today: u32,
    pub suggestions: SuggestionStats,
    pub act_on_own: bool,
    /// M3b: this Hollow's COO role (`single` in the app, `runner` or `off`
    /// in `hollow serve`; `runner_coo::Role`).
    pub role: crate::runner_coo::Role,
    /// M3b: the runner COO's current chat, the one a peer reaches it at;
    /// `None` unless this is the runner and it has a thread.
    pub runner_chat_key: Option<String>,
    /// M3a PR2: whether this Mac's COO is the dispatcher now, to which Mac,
    /// since when, or why not (`dispatcher_coo::Dispatcher`).
    pub dispatcher: crate::dispatcher_coo::Dispatcher,
}

/// When the next check-in is due (ENG-97): the interval after the last one, or
/// now, and when that falls outside the allowed hours the first allowed
/// moment, at most a week out. The allowed hours are whole local hours, so an
/// out-of-hours time jumps to the next top of the hour: the old 15-minute step
/// from an arbitrary minute read up to 14 minutes late.
pub(crate) fn next_check_in_at(
    s: &MainAgentSettings,
    last: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    use chrono::Timelike;
    let earliest = last.map_or(now, |at| {
        (at + Duration::minutes(i64::from(s.interval_min))).max(now)
    });
    let mut at = earliest;
    for _ in 0..(7 * 24 + 2) {
        if in_hours(at.with_timezone(&Local), s) {
            return Some(at);
        }
        let local = at.with_timezone(&Local);
        let into_hour = Duration::minutes(i64::from(local.minute()))
            + Duration::seconds(i64::from(local.second()))
            + Duration::nanoseconds(i64::from(local.nanosecond()));
        at = at - into_hour + Duration::hours(1);
    }
    None
}

pub(crate) fn status(vault: &Path, now: DateTime<Utc>) -> MainAgentStatus {
    status_for(vault, now, crate::runner_coo::role())
}

/// [`status`] for `role`: the runner COO has no check-ins whatever its
/// settings say (M3b).
pub(crate) fn status_for(
    vault: &Path,
    now: DateTime<Utc>,
    role: crate::runner_coo::Role,
) -> MainAgentStatus {
    let s = settings();
    let state = load_state(vault);
    let today = now.with_timezone(&Local).format("%Y-%m-%d").to_string();
    let check_ins = s.check_ins && role.check_ins();
    let next = check_ins
        .then(|| {
            let last = state
                .last_pulse_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .map(|at| at.with_timezone(&Utc));
            next_check_in_at(&s, last, now).map(|at| at.to_rfc3339())
        })
        .flatten();
    let (own, asked) = crate::budgets::main_agent_split(vault, now);
    let wake_skipped = state
        .skipped
        .clone()
        .filter(|s| local_day(&s.at).as_deref() == Some(today.as_str()));
    MainAgentStatus {
        spent_today_usd: own,
        asked_today_usd: asked,
        wake_skipped,
        daily_cap_usd: s.daily_cap_usd,
        check_ins,
        last_check_in: state.last_pulse_at.clone(),
        next_check_in: next,
        check_ins_today: if state.pulse_day == today {
            state.pulses_today
        } else {
            0
        },
        suggestions: stats(&load_suggestions(vault), now),
        act_on_own: s.act_on_own,
        role,
        runner_chat_key: crate::runner_coo::peer_coo_for(role, vault),
        dispatcher: crate::dispatcher_coo::dispatcher_status(role, now),
    }
}

#[tauri::command]
pub async fn main_agent_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<MainAgentStatus, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || status(&vault, Utc::now()))
        .await
        .map_err(|e| e.to_string())
}

/// The chats the Main agent started (the webview's "Chats I started").
#[tauri::command]
pub async fn main_agent_children(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Vec<MainAgentChild>, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || children(&vault, Utc::now()))
        .await
        .map_err(|e| e.to_string())
}

// ── Acting on its own (M4) ─────────────────────────────────────────────

/// Turns Hollow queued for the Main agent with nobody watching (wakes and
/// check-ins), and what such a turn may do through Hollow app tools.
///
/// The limits come from the 2026-10-07 decision (M-Q1, act on safe classes)
/// and hold whatever the conversation's mode: reads; starting, steering or
/// stopping chats (a chat it starts at most in `acceptEdits`,
/// [`UNATTENDED_START_CAP`]; a message to a chat runs in that chat's own
/// mode, `app_sessions::send_mode`); creating and updating
/// tasks; writing notes in the Main agent's own folder. Everything else
/// (builds, installs, settings, automations, deletes, anything that leaves
/// this Mac) is refused with a pointer to suggest it instead. With "Act on
/// its own" off, only reads. Allowed actions show no card (nobody is there
/// to answer it) and each is logged ([`log`]). The run's own file tools and
/// shell follow the thread's own mode ([`wake_mode`]; until 2026-11-12 it
/// was capped at `acceptEdits`).
pub(crate) mod unattended {
    use super::*;
    use std::sync::Mutex;

    /// Queue rows of unattended turns (this process: a restart interrupts
    /// queued turns anyway).
    static ROWS: Mutex<Vec<i64>> = Mutex::new(Vec::new());
    const ROWS_KEPT: usize = 256;

    /// Mark queue row `id` as an unattended turn.
    pub(crate) fn register(id: i64) {
        let mut rows = ROWS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rows.push(id);
        let excess = rows.len().saturating_sub(ROWS_KEPT);
        rows.drain(..excess);
    }

    fn registered(id: i64) -> bool {
        ROWS.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&id)
    }

    /// Whether history run `run_id` is an unattended Main agent turn: one
    /// Hollow queued (registered), a chat such a turn started (registered by
    /// `app_sessions::queue_turn` and swarm launches, so the limits pass
    /// down), or, failing closed across a restart or a fast first call, any
    /// Main agent turn whose message is Hollow's ("⟲ Hollow:").
    pub(crate) fn is_unattended_run(vault: &Path, run_id: &str) -> bool {
        if run_id.trim().is_empty() {
            return false;
        }
        if crate::run_queue::id_for_run(vault, run_id)
            .ok()
            .flatten()
            .is_some_and(registered)
        {
            return true;
        }
        recorded_unattended(vault, run_id)
    }

    /// [`is_unattended_run`] from what is stored only, without this
    /// process's registry: the run's queue row was queued unattended, or it is
    /// a Main agent turn whose message is Hollow's. What a later launch can
    /// still tell about an earlier launch's run (overnight A1).
    pub(crate) fn recorded_unattended(vault: &Path, run_id: &str) -> bool {
        if run_id.trim().is_empty() {
            return false;
        }
        if crate::run_queue::id_for_run(vault, run_id)
            .ok()
            .flatten()
            .is_some_and(|id| crate::chat::queued_unattended(vault, id))
        {
            return true;
        }
        let main = main_cwd(vault);
        session_store::asked_for_run(vault, run_id)
            .ok()
            .flatten()
            .is_some_and(|(asked, cwd)| {
                main.as_deref() == Some(cwd.as_str()) && asked.starts_with(WAKE_PREFIX)
            })
    }

    /// A vault-relative path with `.` and empty parts dropped; `None` for
    /// `..` or an absolute path.
    fn normal(path: &str) -> Option<String> {
        if path.starts_with('/') {
            return None;
        }
        let mut parts = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => return None,
                p => parts.push(p),
            }
        }
        Some(parts.join("/"))
    }

    /// Actions an unattended turn may take besides reads.
    pub(crate) const SAFE: &[&str] = &[
        "sessions.start",
        "sessions.send",
        "sessions.stop",
        "tasks.create_task",
        "tasks.update_task",
        "tasks.set_task_status",
        "tasks.set_task_priority",
        "vault.vault_create_note",
        "vault.vault_write_note",
        "vault.vault_give_up_write",
        "vault.vault_save_note_cas",
        "sessions.suggestion_outcome",
        // LF1: a child's own verdict; it only records into its own chat.
        "sessions.submit_result",
        "learning.submit_learning_proposal",
        // Overnight 10-11 A4: a new prep note before a meeting; sends nothing.
        "agenda.prepare_meeting",
    ];

    const REFUSED: &str = "This is an unattended COO turn (a check-in or a wake), so it may only read, start chats (at most acceptEdits) or steer them, update tasks and write notes in its own folder. Offer this to the person as a suggestion instead; nothing changed.";

    /// Whether an unattended turn may run `action` with `args` (reads always).
    pub(crate) fn allows(
        vault: &Path,
        action_id: &str,
        read: bool,
        args: &serde_json::Map<String, serde_json::Value>,
        act_on_own: bool,
    ) -> Result<(), String> {
        if read {
            return Ok(());
        }
        if !act_on_own {
            return Err("Acting on its own is off (Settings → Background work → COO → Act on its own), so an unattended turn only reads. Offer this to the person as a suggestion instead; nothing changed.".into());
        }
        if !SAFE.contains(&action_id) {
            return Err(REFUSED.into());
        }
        // A check-in writes a new prep pack; it never rewrites one the
        // person may have edited (overnight 10-11 A4 review).
        if action_id == "agenda.prepare_meeting"
            && args.get("regenerate").and_then(|v| v.as_bool()) == Some(true)
        {
            return Err("An unattended turn writes a meeting's prep pack once and never rewrites it; suggest Write it again to the person instead. Nothing changed.".into());
        }
        // A start names its mode; a send's `permission` may only lower the
        // target's own (`app_sessions::send_mode`), so it needs no cap here.
        if action_id == "sessions.start" {
            if let Some(mode) = args.get("permission").and_then(|v| v.as_str()) {
                if crate::app_mcp::perm_rank(mode) > crate::app_mcp::perm_rank(UNATTENDED_START_CAP)
                {
                    return Err(format!("An unattended turn starts chats at most in {UNATTENDED_START_CAP}; ask the person for {mode} in a suggestion. Nothing changed."));
                }
            }
        }
        // A chat it starts inherits these limits; the vault root and its own
        // folder (whose files it may not reach from here) are not targets.
        if action_id == "sessions.start" {
            let target = args
                .get("workspace")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if target == "vault" || crate::workspaces::is_main_agent(target) {
                return Err("An unattended turn starts chats in a workspace or Chats (id quick-chats), not the vault root or the COO itself. Nothing started.".into());
            }
            // Master plan #1: nobody watching never edits a person's branch.
            if crate::build_workspaces::binding_for(vault, target).is_some() {
                return Err("An unattended turn doesn't start chats in a build workspace (a person's branch); suggest it instead. Nothing started.".into());
            }
        }
        if action_id == "sessions.send" {
            let key = args
                .get("session")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let in_main = session_store::read_session(vault, key)
                .is_ok_and(|d| main_cwd(vault).as_deref() == Some(d.summary.cwd.as_str()));
            if in_main || key.is_empty() {
                return Err("An unattended turn messages other chats, not COO conversations. Nothing was sent.".into());
            }
            let in_build = session_store::read_session(vault, key).is_ok_and(|d| {
                crate::build_workspaces::workspace_for_worktree(vault, Path::new(&d.summary.cwd))
                    .is_some()
            });
            if in_build {
                return Err("An unattended turn doesn't steer a chat in a build workspace (a person's branch); suggest it instead. Nothing was sent.".into());
            }
            // Resuming a chat a restart cut short is the person's click only
            // (overnight A1 and S5).
            let offered = crate::restart_resume::offered(vault, Utc::now())
                .unwrap_or_default()
                .iter()
                .any(|c| c.chat.session_key == key);
            if offered {
                return Err("That chat was cut short by a restart; resuming it is the person's click (Review). Nothing was sent.".into());
            }
        }
        if action_id.starts_with("vault.") {
            let own = format!("workspaces/{}/", crate::workspaces::MAIN_AGENT_ID);
            let path = args.get("path").and_then(|v| v.as_str()).and_then(normal);
            let orders = format!("{own}{}", crate::workspaces::STANDING_ORDERS).to_lowercase();
            let ok = path.is_some_and(|p| p.starts_with(&own) && p.to_lowercase() != orders);
            if !ok {
                return Err(format!("An unattended turn writes notes only in its own folder ({own}), never the standing orders. Nothing changed."));
            }
        }
        Ok(())
    }

    /// One thing the Main agent did on its own (the webview's "Done on its own").
    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct Act {
        pub at: String,
        pub action: String,
        pub summary: String,
        pub run_id: String,
    }

    const LOG_KEPT: usize = 200;

    fn log_path(vault: &Path) -> Option<PathBuf> {
        super::state_root().map(|root| {
            root.join(format!(
                "{}-activity.json",
                session_store::vault_file_stem(vault)
            ))
        })
    }

    /// The acts, newest first.
    pub(crate) fn acts(vault: &Path) -> Vec<Act> {
        let mut acts: Vec<Act> = log_path(vault)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        acts.reverse();
        acts
    }

    /// Log one allowed act (best effort).
    pub(crate) fn log(
        vault: &Path,
        action_id: &str,
        args: &serde_json::Map<String, serde_json::Value>,
        run_id: &str,
    ) {
        let Some(path) = log_path(vault) else {
            return;
        };
        let mut acts: Vec<Act> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let pick = |key: &str| args.get(key).and_then(|v| v.as_str()).map(one_line);
        let summary = [
            pick("workspace"),
            pick("session"),
            pick("title"),
            pick("path"),
            pick("prompt"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        acts.push(Act {
            at: Utc::now().to_rfc3339(),
            action: action_id.to_string(),
            summary,
            run_id: run_id.to_string(),
        });
        let excess = acts.len().saturating_sub(LOG_KEPT);
        acts.drain(..excess);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&acts) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }
}

/// What the Main agent did on its own, newest first.
#[tauri::command]
pub async fn main_agent_activity(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Vec<unattended::Act>, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || unattended::acts(&vault))
        .await
        .map_err(|e| e.to_string())
}

// ── Swarms (M5) ────────────────────────────────────────────────────────

/// A swarm: one goal split into chats that run on their own, at most
/// `swarm_concurrency` at once (at most [`MAX_LIVE_BUILD`] in build
/// workspaces, the machine's two toolchain slots), then one wake turn that
/// writes the synthesis. Started only on request (`sessions.swarm`, which
/// asks on a card outside bypass); never by an unattended turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Swarm {
    pub id: String,
    pub goal: String,
    pub created_at: String,
    /// The Main agent conversation and run that started it.
    pub parent_key: String,
    pub run_id: String,
    pub depth: u32,
    pub mode: String,
    #[serde(default)]
    pub budget_usd: Option<f64>,
    /// The parts' model unless a part names its own (`None`: the agent's default).
    #[serde(default)]
    pub model: Option<String>,
    pub tasks: Vec<SwarmTask>,
    /// The synthesis wake was queued.
    #[serde(default)]
    pub reported: bool,
}

/// One part of a swarm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmTask {
    pub title: String,
    pub workspace: String,
    pub prompt: String,
    #[serde(default)]
    pub agent: Option<String>,
    /// Its own model, over the swarm's.
    #[serde(default)]
    pub model: Option<String>,
    /// Its queue row once launched.
    #[serde(default)]
    pub queue_id: Option<i64>,
    #[serde(default)]
    pub session: Option<String>,
    /// Why it never launched (the day's cap or the swarm's budget).
    #[serde(default)]
    pub skipped: Option<String>,
    /// It runs in a build workspace (counts against the toolchain slots).
    #[serde(default)]
    pub build: bool,
}

/// Of a swarm's live parts, at most this many in build workspaces: the
/// machine's two toolchain slots. How many run at once overall is the
/// `swarm_concurrency` setting ([`swarm_part_limit`]), within the app-wide
/// chat limits every turn counts toward (2026-10-20).
pub(crate) const MAX_LIVE_BUILD: usize = 2;
const SWARMS_KEPT: usize = 50;

fn swarms_path(vault: &Path) -> Option<PathBuf> {
    state_root().map(|root| {
        root.join(format!(
            "{}-swarms.json",
            session_store::vault_file_stem(vault)
        ))
    })
}

pub(crate) fn load_swarms(vault: &Path) -> Vec<Swarm> {
    swarms_path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_swarms(vault: &Path, swarms: &[Swarm]) {
    let Some(path) = swarms_path(vault) else {
        return;
    };
    let keep = &swarms[swarms.len().saturating_sub(SWARMS_KEPT)..];
    match serde_json::to_string(keep) {
        Ok(text) => {
            if let Err(error) = atomic_write_json(&path, &text) {
                eprintln!(
                    "[main_agent] save_swarms failed for {}: {error}",
                    path.display()
                );
            }
        }
        Err(error) => eprintln!("[main_agent] serialize swarms failed: {error}"),
    }
}

/// The queue rows of every swarm's chats (their ends wake nobody on their
/// own: the swarm reports once, when all have ended).
pub(crate) fn swarm_rows(vault: &Path) -> HashSet<i64> {
    load_swarms(vault)
        .iter()
        .flat_map(|s| s.tasks.iter().filter_map(|t| t.queue_id))
        .collect()
}

/// A task's state: `pending` (not launched), `live`, `ended` (its row is
/// terminal or gone) or `skipped`.
fn task_state(vault: &Path, task: &SwarmTask) -> &'static str {
    if task.skipped.is_some() {
        return "skipped";
    }
    let Some(id) = task.queue_id else {
        return "pending";
    };
    match crate::run_queue::get(vault, id).ok().flatten() {
        Some(row) if row.status.is_live() => "live",
        _ => "ended",
    }
}

/// Which pending tasks may launch now, by index: within the swarm's live
/// limit and the build-workspace limit.
pub(crate) fn launchable(states: &[(&str, bool)], max_live: usize) -> Vec<usize> {
    let mut live = states.iter().filter(|(s, _)| *s == "live").count();
    let mut live_build = states.iter().filter(|(s, b)| *s == "live" && *b).count();
    let mut out = Vec::new();
    for (i, (state, build)) in states.iter().enumerate() {
        if *state != "pending" || live >= max_live {
            continue;
        }
        if *build && live_build >= MAX_LIVE_BUILD {
            continue;
        }
        live += 1;
        if *build {
            live_build += 1;
        }
        out.push(i);
    }
    out
}

/// Plan a swarm from `sessions.swarm`'s arguments (checked, nothing queued).
pub(crate) fn plan_swarm(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    goal: &str,
    tasks: &serde_json::Value,
    budget_usd: Option<f64>,
    model: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Swarm, String> {
    plan_swarm_with(
        vault,
        caller,
        goal,
        tasks,
        budget_usd,
        model,
        now,
        &settings(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_swarm_with(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    goal: &str,
    tasks: &serde_json::Value,
    budget_usd: Option<f64>,
    model: Option<&str>,
    now: DateTime<Utc>,
    s: &MainAgentSettings,
) -> Result<Swarm, String> {
    let model_ok = |m: &str| {
        !m.is_empty() && m.len() <= 100 && !m.chars().any(|c| c.is_whitespace() || c.is_control())
    };
    let main = main_cwd(vault).ok_or("The COO doesn't exist yet. Nothing started.")?;
    let from_main = caller.surface == "chat"
        && session_store::read_session(vault, &caller.scope).is_ok_and(|d| d.summary.cwd == main);
    // L1: the COO's turn counts from depth 0 whoever messaged it.
    if !from_main || crate::app_sessions::effective_depth(vault, caller) > 0 {
        return Err("Only the COO's own conversation can start a swarm. Nothing started.".into());
    }
    let goal = goal.trim();
    if goal.is_empty() || goal.chars().count() > 300 {
        return Err("Give the swarm a goal of 1 to 300 characters. Nothing started.".into());
    }
    let list = tasks
        .as_array()
        .ok_or("tasks must be a list of {workspace, prompt, title?, agent?}. Nothing started.")?;
    if list.is_empty() || list.len() > s.swarm_max as usize {
        return Err(format!(
            "A swarm runs 1 to {} chats (Settings → Background work → COO → Largest swarm (chats)). Nothing started.",
            s.swarm_max
        ));
    }
    if over_cap(vault, now, s) {
        return Err(format!(
            "The COO reached today's ${:.0} cap. Nothing started.",
            s.daily_cap_usd
        ));
    }
    let swarm_model = match model.map(str::trim) {
        Some(m) if !model_ok(m) => {
            return Err(format!("`{m}` is not a model id. Nothing started."))
        }
        Some(m) => Some(m.to_string()),
        None => (!s.swarm_model.is_empty()).then(|| s.swarm_model.clone()),
    };
    let mut planned = Vec::new();
    for (i, task) in list.iter().enumerate() {
        let get = |k: &str| {
            task.get(k)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let (Some(workspace), Some(prompt)) = (get("workspace"), get("prompt")) else {
            return Err(format!(
                "Task {} needs a workspace and a prompt. Nothing started.",
                i + 1
            ));
        };
        let target = crate::app_sessions::resolve_workspace(vault, workspace)
            .map_err(|e| format!("Task {}: {e}", i + 1))?;
        let build = target
            .workspace_id
            .as_deref()
            .and_then(|id| crate::build_workspaces::binding_for(vault, id))
            .is_some();
        planned.push(SwarmTask {
            title: get("title")
                .map(one_line)
                .unwrap_or_else(|| one_line(prompt)),
            workspace: workspace.to_string(),
            prompt: prompt.to_string(),
            agent: get("agent").map(str::to_string),
            model: match get("model") {
                Some(m) if !model_ok(m) => {
                    return Err(format!(
                        "Task {}: `{m}` is not a model id. Nothing started.",
                        i + 1
                    ))
                }
                m => m.map(str::to_string),
            },
            queue_id: None,
            session: None,
            skipped: None,
            build,
        });
    }
    Ok(Swarm {
        id: format!(
            "swarm-{}-{:04x}",
            now.format("%Y%m%d-%H%M%S"),
            now.timestamp_subsec_nanos() % 0x10000
        ),
        goal: one_line(goal),
        created_at: now.to_rfc3339(),
        parent_key: caller.scope.clone(),
        run_id: caller.run_id.clone(),
        depth: crate::app_sessions::effective_depth(vault, caller),
        mode: caller.perm.clone(),
        budget_usd: budget_usd.filter(|b| b.is_finite() && *b > 0.0),
        model: swarm_model,
        tasks: planned,
        reported: false,
    })
}

/// Queue one swarm task as a durable child task of the run that started the
/// swarm (listed, rolled up and stopped with the other chats it started).
pub(crate) fn swarm_part_header(swarm: &Swarm, index: usize) -> String {
    format!("This chat is part {} of {} of a swarm the COO started. Swarm goal: {}. Do your part and end with a short summary of what you found or made; the COO combines the parts.", index + 1, swarm.tasks.len(), swarm.goal)
}

pub(crate) fn swarm_part_prompt(swarm: &Swarm, index: usize) -> String {
    let task = &swarm.tasks[index];
    format!(
        "{}\n\n{}\n\n{}",
        task.title,
        swarm_part_header(swarm, index),
        task.prompt
    )
}

async fn launch(
    app: &Core,
    config: &crate::config::Config,
    swarm: &Swarm,
    index: usize,
) -> Result<(i64, String), String> {
    let vault = &config.vault_path;
    let task = &swarm.tasks[index];
    let target = crate::app_sessions::resolve_workspace(vault, &task.workspace)?;
    let caller = crate::app_actions::Caller {
        run_id: swarm.run_id.clone(),
        surface: "chat".into(),
        scope: swarm.parent_key.clone(),
        access: crate::app_mcp::AppAccess::Write,
        depth: swarm.depth,
        perm: swarm.mode.clone(),
    };
    let (prompt, packet, rendered) = crate::app_sessions::child_prompt(
        vault,
        &caller,
        crate::delegation::Request::default(),
        &swarm_part_prompt(swarm, index),
    );
    // The swarm's model is for Claude parts; a part on another agent keeps
    // that agent's own default unless it names a model.
    let claude_part = task.agent.as_deref().is_none_or(|a| a == "claude");
    let model = task
        .model
        .clone()
        .or_else(|| claude_part.then(|| swarm.model.clone()).flatten());
    // A Claude model id runs on Claude whatever the app's default agent is.
    let agent = task.agent.clone().or_else(|| {
        model
            .as_deref()
            .filter(|m| m.starts_with("claude-"))
            .map(|_| "claude".to_string())
    });
    let input = crate::commands::ChatSendInput {
        prompt,
        session_id: None,
        session_key: None,
        cwd: Some(target.cwd.clone()),
        model,
        permission_mode: Some(swarm.mode.clone()),
        agent: agent.clone(),
        client_turn_id: None,
        app_depth: swarm.depth + 1,
        ultracode: false,
        canvases: None,
        continued_from: Vec::new(),
        unattended: false,
        typed: false,
        host_asker: None,
        canned: None,
        worker: false,
    };
    // L1: a swarm part is a subagent of the chat that started the swarm.
    let parent = crate::agent_sessions::is_safe_session_key(&swarm.parent_key)
        .then(|| swarm.parent_key.clone());
    let accepted = crate::commands::send_chat_on(Some(app), config, input, None, parent).await?;
    swarm_parts::add(vault, &accepted.session_key);
    let mut record = packet.record(
        &rendered,
        "start_swarm",
        agent.as_deref().unwrap_or("default"),
        Some(accepted.session_key.clone()),
    );
    record.child_queue_id = Some(accepted.queue_id);
    crate::delegation::store(vault, &swarm.run_id, &record);
    {
        let _ = app.emit(
            crate::app_sessions::STARTED_EVENT,
            serde_json::json!({
                "sessionKey": accepted.session_key,
                "workspaceId": target.workspace_id,
                "cwd": target.cwd,
                "startedBy": swarm.parent_key,
            }),
        );
    }
    Ok((accepted.queue_id, accepted.session_key))
}

/// The conversations that are swarm parts, so the run pump can hold them
/// to the swarm's own limit ([`swarm_part_limit`]). Filled at launch and, once per
/// process, from the saved swarms.
pub(crate) mod swarm_parts {
    use std::collections::HashSet;
    use std::sync::Mutex;

    static KEYS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    fn with<T>(vault: Option<&std::path::Path>, f: impl FnOnce(&mut HashSet<String>) -> T) -> T {
        let mut guard = KEYS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys = guard.get_or_insert_with(|| {
            vault
                .map(|v| {
                    super::load_swarms(v)
                        .iter()
                        .flat_map(|s| s.tasks.iter().filter_map(|t| t.session.clone()))
                        .collect()
                })
                .unwrap_or_default()
        });
        f(keys)
    }

    pub(crate) fn add(vault: &std::path::Path, key: &str) {
        with(Some(vault), |keys| {
            keys.insert(key.to_string());
        });
    }

    pub(crate) fn contains(vault: &std::path::Path, key: &str) -> bool {
        with(Some(vault), |keys| keys.contains(key))
    }
}

/// How many swarm parts may run at once, when `session_key` is a swarm
/// part in an ordinary workspace: the `swarm_concurrency` setting, which
/// the run pump holds such a part to (2026-10-07). Since 2026-10-20 that is
/// on top of, not instead of, the app-wide chat limits every turn counts
/// toward (`run_pump::QueueFacts::hold`), so a swarm's parts share the
/// person's chats' budget. `None` when the row isn't a swarm part, or is
/// one in a build workspace (alone in its worktree, and at most
/// [`MAX_LIVE_BUILD`] at launch).
pub(crate) fn swarm_part_limit(
    vault: &Path,
    session_key: Option<&str>,
    exclusive: bool,
) -> Option<usize> {
    let key = session_key?;
    if exclusive || !swarm_parts::contains(vault, key) {
        return None;
    }
    Some(settings().swarm_concurrency as usize)
}

/// Whether `session_key` is a swarm part's conversation.
pub(crate) fn is_swarm_part(vault: &Path, session_key: Option<&str>) -> bool {
    session_key.is_some_and(|k| swarm_parts::contains(vault, k))
}

/// One at a time: launching, skipping and reporting swarms.
static SWARM_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Move every unfinished swarm on: launch what may start, skip what the
/// day's cap or the swarm's budget no longer allows, and queue the
/// synthesis wake once every part has ended.
// Indexed: each swarm is updated in place across the awaits that launch its parts.
#[allow(clippy::needless_range_loop)]
pub(crate) async fn advance(app: &Core, config: &crate::config::Config) {
    let _guard = SWARM_LOCK.lock().await;
    let vault = config.vault_path.clone();
    let mut swarms = load_swarms(&vault);
    let s = settings();
    let now = Utc::now();
    let mut changed = false;
    for si in 0..swarms.len() {
        if swarms[si].reported {
            continue;
        }
        let states: Vec<(&str, bool)> = swarms[si]
            .tasks
            .iter()
            .map(|t| (task_state(&vault, t), t.build))
            .collect();
        let spent: f64 = {
            let ids: HashSet<i64> = swarms[si].tasks.iter().filter_map(|t| t.queue_id).collect();
            children(&vault, now)
                .iter()
                .filter(|c| ids.contains(&c.queue_id))
                .filter_map(|c| c.cost_usd)
                .sum()
        };
        let stop = if over_cap(&vault, now, &s) {
            Some(format!("today's ${:.0} cap was reached", s.daily_cap_usd))
        } else {
            swarms[si]
                .budget_usd
                .filter(|b| spent >= *b)
                .map(|b| format!("the swarm's ${b:.2} budget was spent"))
        };
        if let Some(why) = stop {
            for (i, (state, _)) in states.iter().enumerate() {
                if *state == "pending" {
                    swarms[si].tasks[i].skipped = Some(why.clone());
                    changed = true;
                }
            }
        } else {
            for i in launchable(&states, s.swarm_concurrency as usize) {
                match launch(app, config, &swarms[si], i).await {
                    Ok((queue_id, session)) => {
                        swarms[si].tasks[i].queue_id = Some(queue_id);
                        swarms[si].tasks[i].session = Some(session);
                    }
                    Err(error) => swarms[si].tasks[i].skipped = Some(one_line(&error)),
                }
                changed = true;
            }
        }
        let done = swarms[si]
            .tasks
            .iter()
            .all(|t| matches!(task_state(&vault, t), "ended" | "skipped"));
        if done {
            // Lock order everywhere: SWARM_LOCK, then WAKE_LOCK.
            let _wake = WAKE_LOCK.lock().await;
            if let Some(wake) = synthesis_wake(&vault, &swarms[si], now) {
                if queue_wake(app, config, &wake, ASKED_BY_SWARM).await {
                    swarms[si].reported = true;
                    changed = true;
                    for child in &wake.children {
                        let _ = session_store::deliver_child(
                            &vault,
                            child.queue_id,
                            &child.parent_key,
                            &child.child_session,
                            "wake",
                            None,
                        );
                    }
                }
            }
        }
    }
    if changed {
        save_swarms(&vault, &swarms);
    }
}

/// The wake that asks for a finished swarm's synthesis, in today's thread;
/// `None` while that thread is busy (the next chat end retries).
fn synthesis_wake(vault: &Path, swarm: &Swarm, now: DateTime<Utc>) -> Option<Wake> {
    let cwd = main_cwd(vault)?;
    let target = match today_thread(vault, &cwd, now) {
        Thread::Idle(target) => target,
        Thread::Busy | Thread::JustAnswered => return None,
    };
    let mut lines = Vec::new();
    let mut children_out = Vec::new();
    for task in &swarm.tasks {
        let state = match (&task.skipped, task.queue_id) {
            (Some(why), _) => format!("not started ({why})"),
            (None, Some(id)) => crate::run_queue::get(vault, id)
                .ok()
                .flatten()
                .map(|r| r.status.as_str().to_string())
                .unwrap_or_else(|| "ended".into()),
            (None, None) => "not started".into(),
        };
        lines.push(format!(
            "- “{}” in {} (session {}): {state}",
            task.title,
            task.workspace,
            task.session.as_deref().unwrap_or("none")
        ));
        if let (Some(id), Some(session)) = (task.queue_id, task.session.clone()) {
            children_out.push(WokenChild {
                queue_id: id,
                parent_key: swarm.parent_key.clone(),
                child_session: session,
                line: String::new(),
                also: Vec::new(),
                chain: Vec::new(),
            });
        }
    }
    let text = format!(
        "{SWARM_OPEN}{}” has finished ({} parts).\n{}\n\nRead each part with read_session, then write one synthesis to outputs/swarms/{}.md that cites each part's session, and reply with its five most important lines.",
        swarm.goal,
        swarm.tasks.len(),
        lines.join("\n").replace('<', "‹"),
        swarm.id
    );
    let basis = thread_basis(vault, &cwd, target.as_deref());
    Some(Wake {
        target_key: target,
        session_id: basis.session_id,
        agent: basis.agent,
        model: basis.model,
        cwd,
        mode: basis.mode,
        children: children_out,
        text: Some(text),
        signal_ids: Vec::new(),
    })
}

/// `sessions.swarm`: plan, save, launch the first parts.
pub(crate) async fn start_swarm(
    app: Option<&tauri::AppHandle>,
    config: &crate::config::Config,
    caller: &crate::app_actions::Caller,
    goal: &str,
    tasks: &serde_json::Value,
    budget_usd: Option<f64>,
    model: Option<&str>,
) -> Result<serde_json::Value, String> {
    let swarm = plan_swarm(
        &config.vault_path,
        caller,
        goal,
        tasks,
        budget_usd,
        model,
        Utc::now(),
    )?;
    let concurrency = settings().swarm_concurrency;
    let shown_model = swarm
        .model
        .clone()
        .unwrap_or_else(|| "the agent's default model".into());
    let id = swarm.id.clone();
    let count = swarm.tasks.len();
    {
        let _guard = SWARM_LOCK.lock().await;
        let mut swarms = load_swarms(&config.vault_path);
        swarms.push(swarm);
        save_swarms(&config.vault_path, &swarms);
    }
    if let Some(app) = app.map(Core::from) {
        advance(&app, config).await;
    }
    Ok(serde_json::json!({
        "swarm": id,
        "parts": count,
        "model": shown_model,
        "note": format!("Parts run on {shown_model} unless a part named its own. Hollow runs at most {concurrency} at once ({MAX_LIVE_BUILD} in build workspaces) and wakes you once every part has ended, to write outputs/swarms/{id}.md. Don't wait for it now."),
    }))
}

// ── Learning from its suggestions (M6) ─────────────────────────────────

/// What became of the Main agent's suggestions, and the ones declined for
/// good. The agent records an outcome when the person answers a suggestion
/// (`sessions.suggestion_outcome`); a "never" also asks it to propose a
/// lasting preference, which only the person can accept (Decision 3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SuggestionLog {
    pub outcomes: Vec<SuggestionOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionOutcome {
    pub at: String,
    pub suggestion: String,
    /// `done`, `not_now` or `never`.
    pub outcome: String,
}

const OUTCOMES_KEPT: usize = 500;
/// Declined-for-good suggestions a check-in lists.
pub(crate) const NEVER_LISTED: usize = 10;

fn suggestions_path(vault: &Path) -> Option<PathBuf> {
    state_root().map(|root| {
        root.join(format!(
            "{}-suggestions.json",
            session_store::vault_file_stem(vault)
        ))
    })
}

pub(crate) fn load_suggestions(vault: &Path) -> SuggestionLog {
    suggestions_path(vault)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Record what the person said to a suggestion.
pub(crate) fn record_outcome(
    vault: &Path,
    suggestion: &str,
    outcome: &str,
    now: DateTime<Utc>,
) -> Result<SuggestionStats, String> {
    if !matches!(outcome, "done" | "not_now" | "never") {
        return Err("outcome is done, not_now or never. Nothing recorded.".into());
    }
    let suggestion = one_line(suggestion);
    if suggestion.is_empty() {
        return Err("Name the suggestion. Nothing recorded.".into());
    }
    let mut log = load_suggestions(vault);
    log.outcomes.push(SuggestionOutcome {
        at: now.to_rfc3339(),
        suggestion,
        outcome: outcome.to_string(),
    });
    let excess = log.outcomes.len().saturating_sub(OUTCOMES_KEPT);
    log.outcomes.drain(..excess);
    let path = suggestions_path(vault).ok_or("No app data folder.")?;
    let text = serde_json::to_string(&log).map_err(|e| e.to_string())?;
    atomic_write_json(&path, &text)?;
    Ok(stats(&log, now))
}

/// How the last 30 days of suggestions went.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestionStats {
    pub answered: u32,
    pub taken: u32,
    pub declined_for_good: u32,
    /// Taken / answered, when anything was answered.
    pub taken_share: Option<f64>,
}

pub(crate) fn stats(log: &SuggestionLog, now: DateTime<Utc>) -> SuggestionStats {
    let since = now - Duration::days(30);
    let recent: Vec<&SuggestionOutcome> = log
        .outcomes
        .iter()
        .filter(|o| {
            DateTime::parse_from_rfc3339(&o.at).is_ok_and(|at| at.with_timezone(&Utc) >= since)
        })
        .collect();
    let answered = recent.len() as u32;
    let taken = recent.iter().filter(|o| o.outcome == "done").count() as u32;
    let never = recent.iter().filter(|o| o.outcome == "never").count() as u32;
    SuggestionStats {
        answered,
        taken,
        declined_for_good: never,
        taken_share: (answered > 0).then(|| f64::from(taken) / f64::from(answered)),
    }
}

/// The newest declined-for-good suggestions, for a check-in's message.
pub(crate) fn never_list(log: &SuggestionLog) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for o in log.outcomes.iter().rev().filter(|o| o.outcome == "never") {
        if !out.contains(&o.suggestion) {
            out.push(o.suggestion.clone());
        }
        if out.len() >= NEVER_LISTED {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descendants::tests::{child_turn, record_child};
    use crate::run_queue::RowStatus;

    fn wake_of(ids: &[i64]) -> Wake {
        Wake {
            target_key: Some("a".into()),
            session_id: None,
            agent: Some("claude".into()),
            model: None,
            cwd: "/v/workspaces/main".into(),
            mode: "acceptEdits".into(),
            children: ids
                .iter()
                .map(|id| WokenChild {
                    queue_id: *id,
                    parent_key: "a".into(),
                    child_session: format!("s-{id}"),
                    line: format!("- {id}"),
                    also: Vec::new(),
                    chain: Vec::new(),
                })
                .collect(),
            text: None,
            signal_ids: Vec::new(),
        }
    }

    /// 2026-11-12: a wake keeps the thread's own mode (bypass and auto
    /// too); none recorded is Hollow's default.
    #[test]
    fn a_wake_runs_in_the_threads_own_mode() {
        assert_eq!(wake_mode(Some("bypassPermissions")), "bypassPermissions");
        assert_eq!(wake_mode(Some("auto")), "auto");
        assert_eq!(wake_mode(Some("acceptEdits")), "acceptEdits");
        assert_eq!(wake_mode(Some("default")), "default");
        assert_eq!(wake_mode(Some("plan")), "plan");
        assert_eq!(wake_mode(None), "acceptEdits");
        assert_eq!(wake_mode(Some("nonsense")), "acceptEdits");
    }

    #[test]
    fn recording_counts_the_day_and_stays_bounded() {
        let mut state = WakeState::default();
        record(&mut state, &wake_of(&[1, 2]), "2026-10-07");
        assert_eq!((state.count, state.woken.clone()), (1, vec![1, 2]));
        record(&mut state, &wake_of(&[3]), "2026-10-08");
        assert_eq!(state.count, 1, "a new day resets the count");
        assert_eq!(state.woken, vec![1, 2, 3], "but remembers what woke it");
        state.woken = (0..WOKEN_KEPT as i64).collect();
        record(&mut state, &wake_of(&[10_000]), "2026-10-08");
        assert_eq!(state.woken.len(), WOKEN_KEPT);
        assert_eq!(state.woken.last(), Some(&10_000));
    }

    #[test]
    fn only_a_turn_that_ran_and_ended_is_news() {
        for status in ["done", "failed", "interrupted"] {
            assert!(reportable(status), "{status}");
        }
        for status in ["cancelled", "skipped", "coalesced", "unknown", "running"] {
            assert!(!reportable(status), "{status}");
        }
    }

    #[test]
    fn without_a_main_agent_nothing_is_listed_or_woken() {
        let dir = tempfile::tempdir().unwrap();
        assert!(children(dir.path(), Utc::now()).is_empty());
        assert_eq!(
            decide(dir.path(), Utc::now(), &WakeState::default()),
            Decision::Nothing
        );
        assert!(crate::workspaces::main_agent_cwd(dir.path()).is_none());
    }

    #[test]
    fn the_wake_message_reads_as_hollows() {
        assert!(WAKE_MESSAGE.starts_with(WAKE_PREFIX));
        assert!(wake_of(&[7]).message().ends_with("\n\n- 7"));
        assert!(SCHEDULE_NOTE.contains("Settings → Background work → COO ("));
    }

    #[test]
    fn started_chats_are_listed_and_todays_thread_wakes_once() {
        // Midnight-proof (overnight S6): "today's thread" is the Main agent
        // conversation created on the local day of `now`. The test creates it
        // with the real clock, so a run straddling midnight (seen 00:00 ET,
        // 2026-10-10) found yesterday's thread. Before each read, restamp the
        // thread as created at the moment the read uses.
        fn at(v: &Path, now: DateTime<Utc>) -> DateTime<Utc> {
            // The format `start_turn` writes, so `local_day` still reads it.
            let touched = session_store::with_write(v, "test", |tx| {
                tx.execute(
                    "UPDATE sessions SET created_at = ?1 WHERE key = ?2",
                    rusqlite::params![now.to_rfc3339(), "s-main-parent"],
                )
                .map_err(|e| e.to_string())
            })
            .unwrap();
            assert_eq!(touched, 1, "the COO thread was restamped");
            now
        }
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        let parent = "s-main-parent";
        session_store::start_turn(v, parent, "claude", &cwd, "R-main-1", "start a chat").unwrap();
        session_store::start_turn(
            v,
            "s-child-done",
            "claude",
            &other.cwd,
            "R-child-done",
            "Draft",
        )
        .unwrap();
        let done = child_turn(v, "s-child-done", "R-child-done", RowStatus::Done);
        let running = child_turn(v, "s-child-running", "R-child-running", RowStatus::Running);
        let stopped = child_turn(
            v,
            "s-child-stopped",
            "R-child-stopped",
            RowStatus::Cancelled,
        );
        record_child(v, "R-main-1", "s-child-done", done);
        record_child(v, "R-main-1", "s-child-running", running);
        record_child(v, "R-main-1", "s-child-stopped", stopped);
        // A chat another workspace's conversation started is not the Main agent's.
        session_store::start_turn(v, "s-elsewhere", "claude", &other.cwd, "R-else", "x").unwrap();
        let elsewhere = child_turn(v, "s-else-child", "R-else-child", RowStatus::Done);
        record_child(v, "R-else", "s-else-child", elsewhere);

        let list = children(v, at(v, Utc::now()));
        assert_eq!(list.len(), 3, "{list:?}");
        let finished = list
            .iter()
            .find(|c| c.child_session == "s-child-done")
            .unwrap();
        assert_eq!(
            (finished.status.as_str(), finished.live, finished.reported),
            ("done", false, false)
        );
        assert_eq!(finished.parent_key, parent);
        assert_eq!(finished.workspace_id.as_deref(), Some(other.id.as_str()));
        assert!(list
            .iter()
            .any(|c| c.child_session == "s-child-running" && c.live));

        let state = WakeState::default();
        let Decision::Wake(wake) = decide(v, at(v, Utc::now()), &state) else {
            panic!("a wake is due");
        };
        assert_eq!(wake.target_key.as_deref(), Some(parent), "today's thread");
        assert_eq!(wake.cwd, cwd);
        assert_eq!(wake.mode, "acceptEdits");
        let ids: Vec<i64> = wake.children.iter().map(|c| c.queue_id).collect();
        assert_eq!(ids, vec![done], "the stopped chat is not news");
        assert!(
            wake.children[0].line.contains("in Launch") && wake.children[0].line.contains(": done"),
            "{}",
            wake.children[0].line
        );
        // Ended too long ago: nothing.
        assert_eq!(
            decide(
                v,
                Utc::now() + Duration::hours(WAKE_WINDOW_HOURS + 1),
                &state
            ),
            Decision::Nothing
        );
        // Today's thread is busy: nothing yet; it just finished: later.
        let busy = child_turn(v, parent, "R-main-2", RowStatus::Running);
        assert_eq!(decide(v, at(v, Utc::now()), &state), Decision::Busy);
        crate::run_queue::finish(v, busy, RowStatus::Done, None, Utc::now()).unwrap();
        assert_eq!(decide(v, at(v, Utc::now()), &state), Decision::Later);
        let after = at(v, Utc::now() + Duration::seconds(GRACE_SECS + 5));
        let Decision::Wake(wake) = decide(v, after, &state) else {
            panic!("due after the grace period");
        };
        // The daily cap.
        let full = WakeState {
            day: after.with_timezone(&Local).format("%Y-%m-%d").to_string(),
            count: MAX_WAKES_PER_DAY,
            ..WakeState::default()
        };
        let Decision::Skipped(skip) = decide(v, after, &full) else {
            panic!("the day's wakes are used up: a skip the person can see");
        };
        assert!(skip.reason.contains("times today"), "{}", skip.reason);
        assert_eq!(skip.sessions, vec!["s-child-done".to_string()]);
        // Queued: never again for that child turn, and it counts as reported.
        wake_queued(v, &wake, after);
        assert_eq!(decide(v, after, &load_state(v)), Decision::Nothing);
        assert!(children(v, after)
            .iter()
            .any(|c| c.queue_id == done && c.reported));
    }

    /// Overnight OB17: a wake line carries what the child left, so the wake
    /// needn't start with read_session: its receipt's files and PR, and the
    /// last 1,200 characters of its 3 KB answer (one child), from a line start.
    #[test]
    fn a_wake_line_carries_the_childs_answer_files_and_prs() {
        use crate::descendants::tests::{answered_turn, long_answer};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-ob17-main", "claude", &cwd, "R-ob17-main", "go").unwrap();
        let answer = long_answer("w");
        assert!(answer.len() > 3_000);
        answered_turn(
            v,
            "s-ob17-child",
            &other.cwd,
            "R-ob17-child",
            &answer,
            &["src/a.rs", "docs/b.md"],
            &[712],
        );
        let done = child_turn(v, "s-ob17-child", "R-ob17-child", RowStatus::Done);
        record_child(v, "R-ob17-main", "s-ob17-child", done);
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("a wake is due");
        };
        let line = &wake.children[0].line;
        assert!(
            line.contains("in Launch") && line.contains(": done"),
            "{line}"
        );
        assert!(
            line.contains("; 2 files changed (src/a.rs, docs/b.md); PR #712 opened"),
            "{line}"
        );
        assert!(line.contains("\n    w line 049 "), "{line}");
        assert!(line.contains("\n    w line 031 "), "{line}");
        assert!(!line.contains("w line 030 "), "{line}");
        assert!(line.len() <= 4_096, "{} bytes", line.len());
        let message = wake.message();
        assert!(message.contains("call read_session only when you need more"));
        assert!(message.ends_with(line.as_str()));
    }

    /// Overnight OB18: a wake retrieves on what it reports (its chats'
    /// titles and workspaces), never on its template or the child's answer.
    #[test]
    fn a_wakes_subject_is_its_chats_and_workspaces_not_its_template() {
        use crate::descendants::tests::answered_turn;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-ob18-main", "claude", &cwd, "R-ob18-main", "go").unwrap();
        let first = session_store::start_turn(
            v,
            "s-ob18-child",
            "claude",
            &other.cwd,
            "R-ob18-first",
            "Fix the Notion sync",
        )
        .unwrap();
        session_store::finish_turn(v, "s-ob18-child", first, "success").unwrap();
        answered_turn(
            v,
            "s-ob18-child",
            &other.cwd,
            "R-ob18-child",
            "Finished: the report is in PLAN.md.",
            &["src/sync.rs"],
            &[],
        );
        let done = child_turn(v, "s-ob18-child", "R-ob18-child", RowStatus::Done);
        record_child(v, "R-ob18-main", "s-ob18-child", done);
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("a wake is due");
        };
        let message = wake.message();
        assert!(
            message.contains("“Fix the Notion sync” in Launch"),
            "{message}"
        );
        let subject = host_subject(&message).expect("a wake has a subject");
        assert_eq!(subject, "Fix the Notion sync\nLaunch");
        let terms: Vec<String> = crate::directory_connections::retrieval_terms(&subject)
            .into_iter()
            .map(|t| t.text)
            .collect();
        for template in ["report", "plan", "finished", "chat", "sync.rs"] {
            assert!(!terms.contains(&template.to_string()), "{template}");
        }
        // A chat in no workspace adds only its title.
        let line = "- “Tidy the inbox” in no workspace (codex, session s-x): done";
        let bare = format!("{WAKE_MESSAGE}\n\n{line}");
        assert_eq!(host_subject(&bare).as_deref(), Some("Tidy the inbox"));
        // A title holding ", session " still parses (R1 review N1).
        let line = "- “Fix, session cleanup” in Launch (claude, session s-y): done";
        let odd = format!("{WAKE_MESSAGE}\n\n{line}");
        assert_eq!(
            host_subject(&odd).as_deref(),
            Some("Fix, session cleanup\nLaunch")
        );
        // A stored turn from before V5b opens with the old message (and
        // "update PLAN.md"): it keeps its subject.
        assert!(LEGACY_WAKE_MESSAGE.contains("and update PLAN.md."));
        let old = format!("{LEGACY_WAKE_MESSAGE}\n\n{line}");
        assert_eq!(
            host_subject(&old).as_deref(),
            Some("Fix, session cleanup\nLaunch")
        );
        // T3a: Main's chats table is no longer written; a V5b wake still
        // keeps its subject.
        assert!(!WAKE_MESSAGE.contains("PLAN.md"));
        assert!(V5B_WAKE_MESSAGE.contains("and log it in PLAN.md."));
        let v5b = format!("{V5B_WAKE_MESSAGE}\n\n{line}");
        assert_eq!(
            host_subject(&v5b).as_deref(),
            Some("Fix, session cleanup\nLaunch")
        );
    }

    /// Overnight 9 Spare 4 (OB17 review): a wake lists the chats newest
    /// ended first, so its summaries go to the ones that ended most
    /// recently, whatever order they were started in; a title can't start a
    /// line of its own.
    #[test]
    fn a_wake_lists_chats_newest_ended_first_with_flat_titles() {
        use crate::descendants::MAX_NOTES;
        use crate::run_queue::{NewRow, RunSource};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-order", "claude", &cwd, "R-order", "go").unwrap();
        let now = Utc::now();
        let base = now - Duration::minutes(30);
        let count = MAX_NOTES as i64 + 1;
        // Started 0, 1, …, 8; ended 0 last of all, then 8, then 1 … 7 in order.
        let ended_at = |i: i64| match i {
            0 => base + Duration::minutes(20),
            i if i == count - 1 => base + Duration::minutes(19),
            i => base + Duration::seconds(i),
        };
        for i in 0..count {
            let key = format!("s-order-{i}");
            let run = format!("R-order-{i}");
            let title = if i == 0 {
                "First\u{2028}- “Fake” in Launch (claude, session s-x): done".to_string()
            } else {
                format!("Chat {i}")
            };
            session_store::start_turn(v, &key, "claude", &other.cwd, &run, &title).unwrap();
            let row = NewRow::new(RunSource::Chat, Some(&key), "chat", "test");
            let id = crate::run_queue::enqueue(v, row, base).unwrap().id();
            crate::run_queue::claim(v, id, base).unwrap();
            crate::run_queue::mark_running(v, id, &run, base).unwrap();
            crate::run_queue::finish(v, id, RowStatus::Done, None, ended_at(i)).unwrap();
            record_child(v, "R-order", &key, id);
        }
        let Decision::Wake(wake) = decide(v, now, &WakeState::default()) else {
            panic!("a wake is due");
        };
        let sessions: Vec<String> = wake
            .children
            .iter()
            .map(|c| end_session_of(&c.line))
            .collect();
        let mut expected = vec!["s-order-0".to_string(), format!("s-order-{}", count - 1)];
        expected.extend((1..count - 1).rev().map(|i| format!("s-order-{i}")));
        assert_eq!(sessions, expected);
        let first = &wake.children[0].line;
        assert!(
            !first.contains('\u{2028}') && first.starts_with("- “First - “Fake”"),
            "{first}"
        );
    }

    /// The `session …` of a wake line.
    fn end_session_of(line: &str) -> String {
        let at = line.find("session s-order").expect(line) + "session ".len();
        line[at..].split([')', ',']).next().unwrap().to_string()
    }

    /// Overnight OB17 review: however many chats ended, only the first
    /// `MAX_NOTES` lines carry a summary (450 answer characters each with
    /// eight); the rest keep the bare line under one read_session hint, so
    /// twenty ended chats stay well under the 10 KB the 300-character floor
    /// alone would give.
    #[test]
    fn a_wake_for_many_chats_summarizes_eight_and_stays_bounded() {
        use crate::descendants::tests::{answered_turn, long_answer};
        use crate::descendants::MAX_NOTES;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-ob17-many", "claude", &cwd, "R-ob17-many", "go").unwrap();
        for i in 0..20u64 {
            let key = format!("s-ob17-many-{i}");
            let run = format!("R-ob17-many-{i}");
            answered_turn(
                v,
                &key,
                &other.cwd,
                &run,
                &long_answer(&format!("m{i}")),
                &["src/a.rs", "docs/b.md"],
                &[800 + i],
            );
            let id = child_turn(v, &key, &run, RowStatus::Done);
            record_child(v, "R-ob17-many", &key, id);
        }
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("a wake is due");
        };
        assert_eq!(
            wake.children.len(),
            20,
            "every ended chat is still reported"
        );
        let message = wake.message();
        assert_eq!(
            message.matches("\n  Its answer").count(),
            MAX_NOTES,
            "{message}"
        );
        assert_eq!(message.matches("; verification ").count(), MAX_NOTES);
        for child in &wake.children[..MAX_NOTES] {
            assert!(child.line.contains("(last "), "{}", child.line);
            assert!(child.line.len() <= 1_024, "{}", child.line);
        }
        assert!(
            wake.children[MAX_NOTES]
                .line
                .contains("No summary for this chat and the 11 after it (a wake summarizes 8); read_session has their answers."),
            "{}",
            wake.children[MAX_NOTES].line
        );
        for child in &wake.children[MAX_NOTES + 1..] {
            assert!(!child.line.contains('\n'), "{}", child.line);
        }
        // 4 KB of answers plus the fixed fields of each line.
        assert!(message.len() <= 4_096 + 20 * 220, "{} bytes", message.len());
    }

    fn anytime() -> MainAgentSettings {
        MainAgentSettings {
            start_hour: 0,
            end_hour: 24,
            weekdays_only: false,
            ..MainAgentSettings::default()
        }
    }

    #[test]
    fn the_swarm_limit_for_worker_batches_fails_closed() {
        use std::io::{Error, ErrorKind};
        let saved = serde_json::to_string(&MainAgentSettings {
            swarm_max: 4,
            ..MainAgentSettings::default()
        })
        .unwrap();
        assert_eq!(swarm_max_from(Ok(saved)), Ok(4));
        assert_eq!(swarm_max_from(Err(Error::from(ErrorKind::NotFound))), Ok(6));
        assert!(swarm_max_from(Err(Error::from(ErrorKind::PermissionDenied))).is_err());
        assert!(swarm_max_from(Ok("{not json".into())).is_err());
        assert!(swarm_max_from(Ok(r#"{"swarmMax":0}"#.into())).is_err());
    }

    #[test]
    fn the_defaults_are_the_users_decisions_and_bad_settings_are_refused() {
        let d = MainAgentSettings::default();
        assert_eq!((d.interval_min, d.start_hour, d.end_hour), (30, 8, 18));
        assert!(d.check_ins && d.weekdays_only && d.act_on_own && d.fresh_after_away);
        // Overnight 9 A9: a file saved before the field reads it as on.
        assert!(settings_from(Some(r#"{"checkIns":false}"#)).fresh_after_away);
        assert_eq!((d.daily_cap_usd, d.swarm_max), (100.0, 6));
        assert_eq!(
            (d.swarm_model.as_str(), d.swarm_concurrency),
            ("claude-haiku-5-5", 6)
        );
        assert!(d.clone().validated().is_ok());
        for bad in [
            MainAgentSettings {
                interval_min: 5,
                ..d.clone()
            },
            MainAgentSettings {
                start_hour: 18,
                end_hour: 8,
                ..d.clone()
            },
            MainAgentSettings {
                daily_cap_usd: -1.0,
                ..d.clone()
            },
            MainAgentSettings {
                daily_cap_usd: f64::NAN,
                ..d.clone()
            },
            MainAgentSettings {
                swarm_max: 0,
                ..d.clone()
            },
        ] {
            assert!(bad.validated().is_err());
        }
    }

    #[test]
    fn check_ins_keep_to_their_hours() {
        use chrono::TimeZone;
        let s = MainAgentSettings::default();
        let at = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 10, d, h, 0, 0).unwrap();
        assert!(in_hours(at(7, 8), &s), "Wednesday 8:00");
        assert!(in_hours(at(7, 17), &s));
        assert!(!in_hours(at(7, 18), &s), "until 18:00, not after");
        assert!(!in_hours(at(7, 7), &s));
        assert!(!in_hours(at(10, 10), &s), "Saturday");
        assert!(in_hours(
            at(10, 10),
            &MainAgentSettings {
                weekdays_only: false,
                ..s
            }
        ));
    }

    /// ENG-97: the header's next check-in never reads late by the old 15-minute
    /// step: out of hours it lands on the first allowed moment (the top of the
    /// start hour), in hours it is the interval after the last one.
    #[test]
    fn next_check_in_is_exact_not_stepped_by_fifteen_minutes() {
        let s = MainAgentSettings {
            start_hour: 8,
            end_hour: 18,
            weekdays_only: false,
            interval_min: 30,
            ..MainAgentSettings::default()
        };
        let local = |d, h, m, sec| {
            chrono::NaiveDate::from_ymd_opt(2026, 11, d)
                .unwrap()
                .and_hms_opt(h, m, sec)
                .and_then(|t| t.and_local_timezone(Local).earliest())
                .unwrap()
                .with_timezone(&Utc)
        };
        // 7:46:12 with no earlier check-in: due at 8:00:00 sharp, not 8:01:12 or 8:16.
        assert_eq!(
            next_check_in_at(&s, None, local(11, 7, 46, 12)),
            Some(local(11, 8, 0, 0))
        );
        // In hours: now, or the interval after the last one.
        assert_eq!(
            next_check_in_at(&s, None, local(11, 9, 7, 0)),
            Some(local(11, 9, 7, 0))
        );
        assert_eq!(
            next_check_in_at(&s, Some(local(11, 9, 0, 0)), local(11, 9, 10, 0)),
            Some(local(11, 9, 30, 0))
        );
        // Last check-in at 17:50 pushes past the end: next morning at 8:00.
        assert_eq!(
            next_check_in_at(&s, Some(local(11, 17, 50, 0)), local(11, 17, 55, 0)),
            Some(local(12, 8, 0, 0))
        );
    }

    #[test]
    fn a_check_in_spends_nothing_unless_something_is_new_or_it_is_the_first_today() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        // Never within the last hours of a local day: the steps below add about an
        // hour and must stay on one day. Never after the real time either, so a
        // run that just failed counts as recent.
        let now = {
            use chrono::Timelike;
            let real = Local::now();
            if real.hour() >= 20 {
                real.date_naive()
                    .and_hms_opt(20, 0, 0)
                    .and_then(|t| t.and_local_timezone(Local).earliest())
                    .unwrap()
                    .with_timezone(&Utc)
            } else {
                real.with_timezone(&Utc)
            }
        };
        assert_eq!(
            decide_pulse(v, now, &anytime(), &WakeState::default()),
            Decision::Nothing,
            "no COO"
        );
        crate::workspaces::ensure_main_agent(v).unwrap();
        // The day's first check-in runs anyway: the morning brief.
        let Decision::Wake(first) = decide_pulse(v, now, &anytime(), &WakeState::default()) else {
            panic!("first check-in of the day");
        };
        let text = first.message();
        assert!(
            text.starts_with(WAKE_PREFIX) && text.contains("first check-in today"),
            "{text}"
        );
        assert!(text.contains("PULSE_OK"));
        assert_eq!(first.mode, "acceptEdits");
        pulse_queued(v, &first, now);
        let state = load_state(v);
        assert_eq!(state.pulses_today, 1);
        // Within the interval: nothing.
        assert_eq!(
            decide_pulse(v, now + Duration::minutes(10), &anytime(), &state),
            Decision::Nothing
        );
        // Past it with nothing new: nothing (no tokens).
        let later = now + Duration::minutes(31);
        assert_eq!(
            decide_pulse(v, later, &anytime(), &state),
            Decision::Nothing
        );
        // A run failed: that is new, once.
        let failed =
            crate::descendants::tests::child_turn(v, "s-somewhere", "R-failed", RowStatus::Failed);
        let Decision::Wake(pulse) = decide_pulse(v, later, &anytime(), &state) else {
            panic!("a failed run is news");
        };
        assert_eq!(pulse.signal_ids, vec![format!("failed:{failed}")]);
        assert!(
            pulse.message().contains("A run failed"),
            "{}",
            pulse.message()
        );
        pulse_queued(v, &pulse, later);
        let state = load_state(v);
        assert_eq!(
            decide_pulse(v, later + Duration::minutes(31), &anytime(), &state),
            Decision::Nothing,
            "never reported twice"
        );
        // Off, or out of hours: nothing.
        let off = MainAgentSettings {
            check_ins: false,
            ..anytime()
        };
        assert_eq!(
            decide_pulse(v, now, &off, &WakeState::default()),
            Decision::Nothing
        );
        let never = MainAgentSettings {
            start_hour: 0,
            end_hour: 1,
            weekdays_only: false,
            ..anytime()
        };
        let at_noon = Local::now()
            .date_naive()
            .and_hms_opt(12, 0, 0)
            .and_then(|t| t.and_local_timezone(Local).earliest())
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            decide_pulse(v, at_noon, &never, &WakeState::default()),
            Decision::Nothing
        );
        // The day's cap: nothing.
        let broke = MainAgentSettings {
            daily_cap_usd: 0.0,
            ..anytime()
        };
        assert_eq!(
            decide_pulse(v, now, &broke, &WakeState::default()),
            Decision::Nothing
        );
    }

    /// Overnight OB18: a check-in retrieves on its new signals, never on
    /// its guidance; with nothing new its subject is empty (no retrieval).
    /// Any other message, Hollow's other templates included, has none.
    #[test]
    fn a_check_ins_subject_is_its_new_signal_lines() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let now = Utc::now();
        // Whatever the fresh vault already signals counts as reported, so
        // the day's first check-in has nothing new.
        let reported = WakeState {
            reported_signals: signals(v, now, now - Duration::hours(FIRST_LOOKBACK_HOURS))
                .into_iter()
                .map(|sig| sig.id)
                .collect(),
            ..WakeState::default()
        };
        let Decision::Wake(quiet) = decide_pulse(v, now, &anytime(), &reported) else {
            panic!("first check-in of the day");
        };
        assert!(quiet.signal_ids.is_empty());
        let text = quiet.message();
        assert!(text.contains("Nothing new came up"), "{text}");
        assert_eq!(host_subject(&text).as_deref(), Some(""));
        let failed = crate::descendants::tests::child_turn(
            v,
            "s-ob18-somewhere",
            "R-ob18-failed",
            RowStatus::Failed,
        );
        let Decision::Wake(pulse) = decide_pulse(v, now, &anytime(), &WakeState::default()) else {
            panic!("a failed run is news");
        };
        assert!(pulse.signal_ids.contains(&format!("failed:{failed}")));
        let subject = host_subject(&pulse.message()).expect("a subject");
        assert_eq!(subject.lines().count(), pulse.signal_ids.len());
        assert!(subject.contains("- A run failed:"), "{subject}");
        for guidance in ["PULSE_OK", "check-in at", "first check-in today"] {
            assert!(!subject.contains(guidance), "{guidance}: {subject}");
        }
        // Not a wake or a check-in: read as the request.
        assert_eq!(host_subject("Fix the Notion sync retry backoff"), None);
        assert_eq!(
            host_subject(&format!(
                "{SWARM_OPEN}Map the vendors” has finished (3 parts)."
            )),
            None
        );
        assert_eq!(
            host_subject(&crate::digest::prose_message("2026-W42", "outputs/w42.md")),
            None
        );
        assert_eq!(
            host_subject(&format!(
                "{} item 3 added by you at 01:00. Plan the Q3 offsite agenda.",
                crate::night_queue::NIGHT_PREFIX
            )),
            None
        );
    }

    /// Fix-up 10-17 (red first): a Main agent thread on a non-default model
    /// keeps it on a check-in and a wake. Every wake sent `model: None`, the
    /// saved default, so the model in the resume fingerprint changed and the
    /// thread got a fresh native session each time.
    #[test]
    fn a_wake_and_a_check_in_keep_the_threads_model() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let key = "s-fx17-model";
        let turn = |run: &str, model: Option<&str>| {
            session_store::start_turn(v, key, "claude", &cwd, run, "go").unwrap();
            let at = session_store::db_time(Utc::now());
            let mut r = session_store::tests::usage_row_fixture(run, &at, "success");
            r.run_id = Some(run.into());
            r.agent = Some("claude".into());
            r.requested_model = model.map(str::to_string);
            session_store::upsert_usage(v, &r).unwrap();
        };
        turn("R-fx17-1", Some("claude-sonnet-5-5"));
        let basis = thread_basis(v, &cwd, Some(key));
        assert_eq!(basis.model.as_deref(), Some("claude-sonnet-5-5"));
        let Decision::Wake(pulse) = decide_pulse(v, Utc::now(), &anytime(), &WakeState::default())
        else {
            panic!("a check-in");
        };
        assert_eq!(pulse.target_key.as_deref(), Some(key), "today's thread");
        let input = wake_input(&pulse, false, ASKED_BY_CHECKIN, Vec::new());
        assert_eq!(input.model.as_deref(), Some("claude-sonnet-5-5"));
        // The newest turn ran the CLI's own default: kept as "", not the
        // saved model.
        turn("R-fx17-2", None);
        let wake = Wake {
            model: thread_basis(v, &cwd, Some(key)).model,
            ..pulse
        };
        assert_eq!(
            wake_input(&wake, false, ASKED_BY_WAKE, Vec::new())
                .model
                .as_deref(),
            Some("")
        );
    }

    /// 2026-11-12: a check-in and a wake send their turn in the thread's
    /// own mode, end to end through `decide_pulse` and `wake_input`: a thread
    /// in bypass or auto stays there (before, both fell to acceptEdits), and
    /// one in plan stays in plan.
    #[test]
    fn a_wake_and_a_check_in_keep_the_threads_own_mode() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let key = "s-perm-a-wake";
        for (n, mode) in ["bypassPermissions", "auto", "plan"]
            .into_iter()
            .enumerate()
        {
            let run = format!("R-perm-a-{n}");
            let ord = session_store::start_turn(v, key, "claude", &cwd, &run, "go").unwrap();
            session_store::record_turn_permission(v, key, ord, mode).unwrap();
            assert_eq!(thread_basis(v, &cwd, Some(key)).mode, mode);
            let Decision::Wake(pulse) =
                decide_pulse(v, Utc::now(), &anytime(), &WakeState::default())
            else {
                panic!("a check-in");
            };
            assert_eq!(pulse.target_key.as_deref(), Some(key), "today's thread");
            assert_eq!(pulse.mode, mode);
            for asker in [ASKED_BY_CHECKIN, ASKED_BY_WAKE] {
                let input = wake_input(&pulse, false, asker, Vec::new());
                assert_eq!(input.permission_mode.as_deref(), Some(mode), "{asker}");
            }
        }
    }

    /// OB19 + OB20 (red first): a check-in that finds no thread today
    /// starts one that continues the newest earlier Main agent thread (never
    /// an older one), and its send is tagged `hollow:checkin`. Once today's
    /// thread exists, a wake goes there, continues nothing, and is tagged
    /// `hollow:wake`; a wake that raced today's thread into being never
    /// continues it.
    #[test]
    fn a_wake_that_starts_todays_thread_continues_the_last_one_and_is_tagged() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let now = Utc::now();
        let stamp = |key: &str, at: DateTime<Utc>| {
            session_store::with_write(v, "test", |tx| {
                tx.execute(
                    "UPDATE sessions SET created_at = ?1 WHERE key = ?2",
                    rusqlite::params![at.to_rfc3339(), key],
                )
                .map_err(|e| e.to_string())
            })
            .unwrap()
        };
        // Days apart, so no clock (or DST day) puts them on today.
        let (older, last) = ("s-ob19-older", "s-ob19-last");
        session_store::start_turn(v, older, "claude", &cwd, "R-ob19-0", "Plan Q3").unwrap();
        session_store::start_turn(v, last, "claude", &cwd, "R-ob19-1", "Run the programme")
            .unwrap();
        assert_eq!(stamp(older, now - Duration::days(4)), 1);
        assert_eq!(stamp(last, now - Duration::days(2)), 1);

        let Decision::Wake(pulse) = decide_pulse(v, now, &anytime(), &WakeState::default()) else {
            panic!("first check-in of the day");
        };
        assert_eq!(pulse.target_key, None, "today has no thread yet");
        let continued = continued_for(v, &pulse, now);
        assert_eq!(continued, vec![last.to_string()]);
        let input = wake_input(&pulse, false, ASKED_BY_CHECKIN, continued);
        assert_eq!(input.continued_from, vec![last.to_string()]);
        assert_eq!(input.host_asker, Some("hollow:checkin"));
        assert_eq!(input.session_key, None);
        assert_eq!((input.app_depth, input.typed), (0, false));

        // Today's thread exists: the wake goes to it and continues nothing.
        let today = "s-ob19-today";
        session_store::start_turn(v, today, "claude", &cwd, "R-ob19-2", "Morning").unwrap();
        assert_eq!(stamp(today, now), 1);
        let wake = Wake {
            target_key: Some(today.into()),
            ..pulse.clone()
        };
        assert!(continued_for(v, &wake, now).is_empty());
        let input = wake_input(&wake, false, ASKED_BY_WAKE, Vec::new());
        assert_eq!(input.host_asker, Some("hollow:wake"));
        assert!(input.continued_from.is_empty());
        assert!(
            continued_for(v, &pulse, now).is_empty(),
            "never today's own thread"
        );
        for tag in [
            ASKED_BY_WAKE,
            ASKED_BY_CHECKIN,
            ASKED_BY_WEEKLY,
            ASKED_BY_WAKE_ME,
            ASKED_BY_SWARM,
        ] {
            assert!(tag.starts_with("hollow:") && tag.len() <= 40, "{tag}");
        }
    }

    #[test]
    fn todays_spend_counts_the_main_agents_turns() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let checkin = format!("{WAKE_PREFIX} check-in at 10:00.");
        session_store::start_turn(v, "s-main-spend", "claude", &cwd, "R-spend", &checkin).unwrap();
        let mut row =
            session_store::tests::usage_row_fixture("k-spend", "2026-10-07 10:00:00", "success");
        row.run_id = Some("R-spend".into());
        row.cost_usd = Some(2.5);
        session_store::upsert_usage(v, &row).unwrap();
        // Another folder's turn doesn't count.
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-other-spend", "claude", &other.cwd, "R-other", "hi")
            .unwrap();
        let mut row =
            session_store::tests::usage_row_fixture("k-other", "2026-10-07 10:00:00", "success");
        row.run_id = Some("R-other".into());
        row.cost_usd = Some(40.0);
        session_store::upsert_usage(v, &row).unwrap();
        let spent = crate::budgets::main_agent_spent(v, Utc::now());
        assert!((spent - 2.5).abs() < 1e-9, "{spent}");
        assert!(over_cap(
            v,
            Utc::now(),
            &MainAgentSettings {
                daily_cap_usd: 2.0,
                ..anytime()
            }
        ));
        assert!(!over_cap(v, Utc::now(), &anytime()));
        // The person's own turn is shown apart and never counts (overnight
        // 10-11 A1).
        session_store::start_turn(v, "s-main-spend", "claude", &cwd, "R-person", "hi").unwrap();
        let mut row =
            session_store::tests::usage_row_fixture("k-person", "2026-10-07 10:00:00", "success");
        row.run_id = Some("R-person".into());
        row.cost_usd = Some(70.0);
        session_store::upsert_usage(v, &row).unwrap();
        let (own, asked) = crate::budgets::main_agent_split(v, Utc::now());
        assert!(
            (own - 2.5).abs() < 1e-9 && (asked - 70.0).abs() < 1e-9,
            "{own} {asked}"
        );
        assert!((crate::budgets::main_agent_spent(v, Utc::now()) - 2.5).abs() < 1e-9);
    }

    /// A Main agent thread, a person turn `R-person` and a check-in turn
    /// `R-hollow` in it, each starting one chat in another workspace; returns
    /// `(vault dir, person's child row, Hollow's child row)`.
    fn two_starters(child_status: RowStatus) -> (tempfile::TempDir, i64, i64) {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Lane", "Build").unwrap();
        session_store::start_turn(v, "s-main", "claude", &cwd, "R-person", "start two lanes")
            .unwrap();
        let checkin = format!("{WAKE_PREFIX} check-in at 10:00.");
        session_store::start_turn(v, "s-main", "claude", &cwd, "R-hollow", &checkin).unwrap();
        for (key, run) in [("s-lane-a", "R-lane-a"), ("s-lane-h", "R-lane-h")] {
            session_store::start_turn(v, key, "claude", &other.cwd, run, "Lane").unwrap();
        }
        let person = child_turn(v, "s-lane-a", "R-lane-a", child_status);
        let hollow = child_turn(v, "s-lane-h", "R-lane-h", child_status);
        record_child(v, "R-person", "s-lane-a", person);
        record_child(v, "R-hollow", "s-lane-h", hollow);
        (dir, person, hollow)
    }

    fn spend(v: &Path, key: &str, run: &str, usd: f64) {
        let mut row = session_store::tests::usage_row_fixture(
            key,
            &session_store::db_time(Utc::now()),
            "success",
        );
        row.run_id = Some(run.into());
        row.cost_usd = Some(usd);
        session_store::upsert_usage(v, &row).unwrap();
    }

    #[test]
    fn b6review_main_folder_children_are_counted_once_under_their_parent() {
        let (dir, _, _) = two_starters(RowStatus::Running);
        let v = dir.path();
        let cwd = main_cwd(v).unwrap();
        session_store::start_turn(
            v,
            "s-b6-nested",
            "claude",
            &cwd,
            "R-b6-nested",
            "delegated work",
        )
        .unwrap();
        let id = child_turn(v, "s-b6-nested", "R-b6-nested", RowStatus::Running);
        record_child(v, "R-hollow", "s-b6-nested", id);
        spend(v, "b6-main", "R-b6-nested", 3.125);
        assert_eq!(
            crate::budgets::main_agent_split(v, Utc::now()),
            (3.125, 0.0)
        );
        // A Main-folder child may start its own child. Its ordinary-looking
        // prompt must not override the older unattended root's attribution.
        let child = child_turn(v, "s-b6-grandchild", "R-b6-grandchild", RowStatus::Running);
        record_child(v, "R-b6-nested", "s-b6-grandchild", child);
        spend(v, "b6-grandchild", "R-b6-grandchild", 1.25);
        assert_eq!(
            crate::budgets::main_agent_split(v, Utc::now()),
            (4.375, 0.0)
        );
    }

    #[test]
    fn b6review_pre_midnight_roots_charge_only_todays_ledger_rows() {
        let (dir, _, hollow) = two_starters(RowStatus::Done);
        let v = dir.path();
        let yesterday = day_start(Utc::now()) - Duration::hours(1);
        session_store::with_write(v, "b6 fixture", |conn| {
            conn.execute(
                "UPDATE run_queue SET requested_at=?1 WHERE id=?2",
                rusqlite::params![yesterday.to_rfc3339(), hollow],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
        let mut old = session_store::tests::usage_row_fixture(
            "b6-yesterday",
            &session_store::db_time(yesterday),
            "success",
        );
        old.run_id = Some("R-lane-h".into());
        old.cost_usd = Some(17.0);
        session_store::upsert_usage(v, &old).unwrap();
        let id = child_turn(v, "s-b6-today", "R-b6-today", RowStatus::Running);
        record_child(v, "R-lane-h", "s-b6-today", id);
        let mut today = session_store::tests::usage_row_fixture(
            "b6-today",
            &session_store::db_time(Utc::now()),
            "success",
        );
        today.run_id = Some("R-b6-today".into());
        today.cost_usd = Some(2.75);
        session_store::upsert_usage(v, &today).unwrap();
        assert_eq!(crate::budgets::main_agent_split(v, Utc::now()), (2.75, 0.0));
    }

    #[test]
    fn running_successors_keep_recorded_spend_in_the_main_agent_cap() {
        let (dir, _person, _hollow) = two_starters(RowStatus::Done);
        let v = dir.path();
        spend(v, "b6-person", "R-lane-a", 150.0);
        spend(v, "b6-root", "R-lane-h", 3.0);
        let mut parent = "R-lane-h".to_string();
        // Longer than the receipt roll-up's four-level bound; the active
        // successor must not erase costs from any of its predecessors.
        for i in 0..12 {
            let key = format!("s-b6-{i}");
            let run = format!("R-b6-{i}");
            let status = if i == 11 {
                RowStatus::Running
            } else {
                RowStatus::Done
            };
            let id = child_turn(v, &key, &run, status);
            record_child(v, &parent, &key, id);
            spend(v, &format!("b6-cost-{i}"), &run, 1.25);
            parent = run;
        }
        assert_eq!(crate::descendants::rollup(v, "R-lane-h").total_usd, None);
        assert_eq!(
            crate::budgets::main_agent_split(v, Utc::now()),
            (18.0, 150.0)
        );
        assert!(over_cap(
            v,
            Utc::now(),
            &MainAgentSettings {
                daily_cap_usd: 17.0,
                ..anytime()
            }
        ));
        // A provider may report another partial ledger row while still
        // running. Use what is recorded, without inventing a final total.
        spend(v, "b6-more", &parent, 0.75);
        assert_eq!(
            crate::budgets::main_agent_split(v, Utc::now()),
            (18.75, 150.0)
        );
        assert_eq!(crate::descendants::rollup(v, "R-lane-h").total_usd, None);
    }

    #[test]
    fn chats_the_person_asked_for_never_count_toward_the_cap_and_always_wake() {
        // 10-10: two lanes the person asked the Main agent to start spent
        // past $100, so every later wake was dropped without a trace.
        let (dir, person, hollow) = two_starters(RowStatus::Done);
        let v = dir.path();
        spend(v, "k-a", "R-lane-a", 150.0);
        spend(v, "k-h", "R-lane-h", 3.0);
        let list = children(v, Utc::now());
        let by = |id: i64| list.iter().find(|c| c.queue_id == id).unwrap().by_hollow;
        assert!(!by(person) && by(hollow), "{list:?}");
        let (own, asked) = crate::budgets::main_agent_split(v, Utc::now());
        assert!(
            (own - 3.0).abs() < 1e-9 && (asked - 150.0).abs() < 1e-9,
            "{own} {asked}"
        );
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("under the cap both wake");
        };
        assert_eq!(wake.children.len(), 2);

        // Over the cap (Hollow's own spend): the person's chat still wakes
        // it; Hollow's own one alone is held back with a reason.
        spend(v, "k-h2", "R-lane-h", 200.0);
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("the person's chat wakes past the cap");
        };
        let ids: Vec<i64> = wake.children.iter().map(|c| c.queue_id).collect();
        assert_eq!(ids, vec![person]);
        wake_queued(v, &wake, Utc::now());
        let Decision::Skipped(skip) = decide(v, Utc::now(), &load_state(v)) else {
            panic!("Hollow's own chat is held back at the cap");
        };
        assert!(skip.reason.contains("cap"), "{}", skip.reason);
        assert_eq!(skip.sessions, vec!["s-lane-h".to_string()]);
        wake_skipped(v, skip.clone());
        let shown = status(v, Utc::now());
        assert_eq!(
            shown.wake_skipped.as_ref(),
            Some(&skip),
            "the side band shows why"
        );
        // The same skip a minute later keeps its first time.
        wake_skipped(
            v,
            WakeSkip {
                at: (Utc::now() + Duration::minutes(1)).to_rfc3339(),
                ..skip.clone()
            },
        );
        assert_eq!(load_state(v).skipped.unwrap().at, skip.at);
    }

    /// One Main conversation (2026-11-07): a wake or check-in never opens a
    /// second thread while the current one works, whatever day it started
    /// (11-04 12:04 AM: s-20261103-da024a6c ran past midnight and a wake
    /// opened s-20261104-6e772c5f beside it); idle and from an earlier day,
    /// today's thread starts from it (the daily soft reset).
    #[test]
    fn a_hollow_turn_waits_for_the_current_main_thread_across_midnight() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = main_cwd(v).unwrap();
        session_store::start_turn(v, "s-mcc-night", "claude", &cwd, "R-mcc-night", "overnight")
            .unwrap();
        let row = child_turn(v, "s-mcc-night", "R-mcc-night", RowStatus::Running);
        let tomorrow = Utc::now() + Duration::days(1);
        assert!(matches!(today_thread(v, &cwd, tomorrow), Thread::Busy));
        assert!(matches!(
            queue_target(v, &cwd, tomorrow),
            Err(TurnQueued::Busy)
        ));
        crate::run_queue::finish(v, row, RowStatus::Done, None, Utc::now()).unwrap();
        let later = Utc::now() + Duration::days(2);
        assert!(
            matches!(today_thread(v, &cwd, later), Thread::Idle(None)),
            "idle from an earlier day: today's thread starts and continues from it"
        );
    }

    /// The Main agent's own handover continues its one conversation: its
    /// successor is no chat it started, so its end never wakes it (the
    /// once-per-handover self-wake, 2026-11-07).
    #[test]
    fn a_main_thread_handover_is_not_a_started_chat_and_never_wakes_it() {
        let (dir, _, _) = two_starters(RowStatus::Running);
        let v = dir.path();
        let cwd = main_cwd(v).unwrap();
        session_store::start_turn(v, "s-main-heir", "claude", &cwd, "R-main-heir", "continue")
            .unwrap();
        let heir = child_turn(v, "s-main-heir", "R-main-heir", RowStatus::Done);
        crate::delegation::store(
            v,
            "R-person",
            &DelegationRecord {
                kind: "childTask".into(),
                tool: crate::app_sessions::HANDOVER_TOOL.into(),
                agent: "claude".into(),
                child_session: Some("s-main-heir".into()),
                child_queue_id: Some(heir),
                at: session_store::db_time(Utc::now()),
                ..DelegationRecord::default()
            },
        );
        let listed = children(v, Utc::now());
        assert!(
            listed.iter().all(|c| c.child_session != "s-main-heir"),
            "{listed:?}"
        );
        assert!(listed.iter().any(|c| c.child_session == "s-lane-a"));
    }

    #[test]
    fn a_chat_that_hands_over_wakes_it_when_its_last_successor_ends() {
        let (dir, person, hollow) = two_starters(RowStatus::Done);
        let v = dir.path();
        // Hollow's chat is reported already; only the lane matters here.
        session_store::deliver_child(v, hollow, "s-main", "s-lane-h", "note", None).unwrap();
        let handover = |from_run: &str, to: &str, row: i64| {
            crate::delegation::store(
                v,
                from_run,
                &DelegationRecord {
                    kind: "childTask".into(),
                    tool: crate::app_sessions::HANDOVER_TOOL.into(),
                    agent: "claude".into(),
                    child_session: Some(to.into()),
                    child_queue_id: Some(row),
                    at: session_store::db_time(Utc::now()),
                    ..DelegationRecord::default()
                },
            )
        };
        // Two hops: s-lane-a → s-lane-a2 (done) → s-lane-a3 (running).
        let a2 = child_turn(v, "s-lane-a2", "R-lane-a2", RowStatus::Done);
        handover("R-lane-a", "s-lane-a2", a2);
        let a3 = child_turn(v, "s-lane-a3", "R-lane-a3", RowStatus::Running);
        handover("R-lane-a2", "s-lane-a3", a3);
        // A note delivered the first chat to the thread meanwhile (the person
        // wrote to it), and each successor's end reached the chat that handed
        // over: none of that is the Main agent hearing the lane ended.
        session_store::deliver_child(v, person, "s-main", "s-lane-a", "note", None).unwrap();
        session_store::deliver_child(v, a2, "s-lane-a", "s-lane-a2", "note", None).unwrap();
        let lane = children(v, Utc::now())
            .into_iter()
            .find(|c| c.queue_id == person)
            .unwrap();
        assert!(lane.live, "{lane:?}");
        assert_eq!(
            lane.successors,
            vec!["s-lane-a2".to_string(), "s-lane-a3".to_string()]
        );
        assert_eq!(lane.end_queue_id, a3);
        assert_eq!(
            decide(v, Utc::now(), &WakeState::default()),
            Decision::Nothing
        );

        crate::run_queue::finish(v, a3, RowStatus::Done, None, Utc::now()).unwrap();
        session_store::deliver_child(v, a3, "s-lane-a2", "s-lane-a3", "note", None).unwrap();
        let Decision::Wake(wake) = decide(v, Utc::now(), &WakeState::default()) else {
            panic!("the lane ended with its last successor");
        };
        assert_eq!(wake.children.len(), 1);
        let child = &wake.children[0];
        assert_eq!(
            (child.queue_id, child.child_session.as_str()),
            (a3, "s-lane-a3")
        );
        assert!(
            child
                .line
                .contains("after handing over to s-lane-a2 → s-lane-a3"),
            "{}",
            child.line
        );
        assert_eq!(child.chain, vec![a2, a3]);
        wake_queued(v, &wake, Utc::now());
        // Reported to the Main agent as the chains' root: no turn's note
        // lists it again, and the chat that handed over keeps its own report.
        let roots = session_store::root_deliveries(v, CHAIN_ROOT, &[a2, a3]).unwrap();
        assert!(
            roots.contains_key(&a2) && roots.contains_key(&a3),
            "{roots:?}"
        );
        assert_eq!(
            session_store::child_delivery(v, a3).unwrap().unwrap().via,
            "note"
        );
        assert_eq!(
            decide(v, Utc::now(), &WakeState::default()),
            Decision::Nothing
        );
    }

    #[test]
    fn a_lane_stopped_from_the_list_is_not_news() {
        // Stop in "Chats it started" cancels the live successor: the chain
        // ends there, not at the done chat before it.
        let (dir, person, hollow) = two_starters(RowStatus::Done);
        let v = dir.path();
        session_store::deliver_child(v, hollow, "s-main", "s-lane-h", "note", None).unwrap();
        let next = child_turn(v, "s-lane-a2", "R-lane-a2", RowStatus::Running);
        crate::delegation::store(
            v,
            "R-lane-a",
            &DelegationRecord {
                kind: "childTask".into(),
                tool: crate::app_sessions::HANDOVER_TOOL.into(),
                agent: "claude".into(),
                child_session: Some("s-lane-a2".into()),
                child_queue_id: Some(next),
                at: session_store::db_time(Utc::now()),
                ..DelegationRecord::default()
            },
        );
        crate::run_queue::cancel(v, next, Utc::now()).unwrap();
        let row = crate::run_queue::get(v, person).unwrap().unwrap();
        let (end, keys, _) = chain_end(v, row);
        assert_eq!((end.id, keys.len()), (next, 1));
        assert_eq!(
            decide(v, Utc::now(), &WakeState::default()),
            Decision::Nothing
        );
    }

    #[test]
    fn an_unattended_turn_acts_only_on_the_safe_classes() {
        use serde_json::{json, Map, Value};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-main-act", "claude", &cwd, "R-main-act", "hi").unwrap();
        session_store::start_turn(
            v,
            "s-launch-act",
            "claude",
            &other.cwd,
            "R-launch-act",
            "hi",
        )
        .unwrap();
        let args = |v: Value| v.as_object().cloned().unwrap_or_else(Map::new);
        let none = args(json!({}));
        let allows =
            |id: &str, a: &Map<String, Value>, on: bool| unattended::allows(v, id, false, a, on);
        assert!(
            unattended::allows(v, "sessions.overview", true, &none, false).is_ok(),
            "reads always"
        );
        let start = args(json!({"workspace": other.id}));
        let send = args(json!({"session": "s-launch-act"}));
        for (id, a) in [
            ("sessions.start", &start),
            ("sessions.send", &send),
            ("tasks.create_task", &none),
            ("sessions.suggestion_outcome", &none),
        ] {
            assert!(allows(id, a, true).is_ok(), "{id}");
            assert!(allows(id, a, false).is_err(), "{id} with acting off");
        }
        for id in [
            "build.start_app_build",
            "build.install_built_app",
            "build.publish_main",
            "settings.change",
            "automations.save",
            "vault.vault_delete_note",
            "sessions.swarm",
            "tasks.delete_task",
        ] {
            let err = allows(id, &none, true).unwrap_err();
            assert!(
                err.contains("unattended") || err.contains("Unattended"),
                "{id}: {err}"
            );
        }
        assert!(allows(
            "sessions.start",
            &args(json!({"workspace": other.id, "permission": "bypassPermissions"})),
            true
        )
        .is_err());
        assert!(allows(
            "sessions.start",
            &args(json!({"workspace": other.id, "permission": "acceptEdits"})),
            true
        )
        .is_ok());
        // A send's `permission` may only lower the target's own mode
        // (`app_sessions::send_mode`), so it is not capped here.
        assert!(allows(
            "sessions.send",
            &args(json!({"session": "s-launch-act", "permission": "auto"})),
            true
        )
        .is_ok());
        // Not the vault root or itself; not its own conversations.
        assert!(allows("sessions.start", &args(json!({"workspace": "vault"})), true).is_err());
        assert!(allows("sessions.start", &args(json!({"workspace": "main"})), true).is_err());
        assert!(allows(
            "sessions.send",
            &args(json!({"session": "s-main-act"})),
            true
        )
        .is_err());
        let note = |path: &str| {
            allows(
                "vault.vault_write_note",
                &args(json!({ "path": path })),
                true,
            )
        };
        assert!(note("workspaces/main/outputs/digest.md").is_ok());
        assert!(note("./workspaces/main/PLAN.md").is_ok());
        for bad in [
            "workspaces/main/STANDING_ORDERS.md",
            "workspaces/main/./STANDING_ORDERS.md",
            "./workspaces/main/standing_orders.md",
            "workspaces/main//STANDING_ORDERS.md",
            "workspaces/main/../launch/WORKSPACE.md",
            "/workspaces/main/x.md",
            "me/Guardrails.md",
            "workspaces/launch/WORKSPACE.md",
        ] {
            assert!(note(bad).is_err(), "{bad}");
        }
        // Registered rows are recognised by their run; so is any Main agent
        // turn whose message is Hollow's, registered or not.
        let id =
            crate::descendants::tests::child_turn(v, "s-wake", "R-wake-run", RowStatus::Running);
        assert!(!unattended::is_unattended_run(v, "R-wake-run"));
        unattended::register(id);
        assert!(unattended::is_unattended_run(v, "R-wake-run"));
        assert!(
            !unattended::is_unattended_run(v, "R-main-act"),
            "a person's turn"
        );
        session_store::start_turn(
            v,
            "s-main-act",
            "claude",
            &cwd,
            "R-main-wake",
            &format!("{WAKE_PREFIX} check-in at 10:00."),
        )
        .unwrap();
        assert!(unattended::is_unattended_run(v, "R-main-wake"));
        session_store::start_turn(
            v,
            "s-launch-act",
            "claude",
            &other.cwd,
            "R-launch-wake",
            &format!("{WAKE_PREFIX} x"),
        )
        .unwrap();
        assert!(
            !unattended::is_unattended_run(v, "R-launch-wake"),
            "only in the COO"
        );
        assert!(!unattended::is_unattended_run(v, ""));
        // Acts are logged, newest first.
        unattended::log(
            v,
            "sessions.start",
            &args(json!({"workspace": "launch", "prompt": "Draft the note"})),
            "R-wake-run",
        );
        let acts = unattended::acts(v);
        assert_eq!(acts[0].action, "sessions.start");
        assert!(
            acts[0].summary.contains("launch") && acts[0].summary.contains("Draft"),
            "{}",
            acts[0].summary
        );
        // Main agent turns never edit the standing orders with their own tools.
        assert!(STANDING_ORDERS_DENY.iter().any(|r| r.starts_with("Edit(")));
    }

    #[test]
    fn a_swarm_runs_its_concurrency_at_once_and_two_in_build_workspaces() {
        let s = |v: &[(&'static str, bool)], max: usize| launchable(v, max);
        assert_eq!(s(&[("pending", false); 8], 3), vec![0, 1, 2]);
        assert_eq!(s(&[("pending", false); 8], 6), vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(
            s(&[("live", false), ("live", false), ("pending", false)], 3),
            vec![2]
        );
        assert_eq!(
            s(
                &[
                    ("live", false),
                    ("live", false),
                    ("live", false),
                    ("pending", false)
                ],
                3
            ),
            Vec::<usize>::new()
        );
        assert_eq!(
            s(
                &[
                    ("pending", true),
                    ("pending", true),
                    ("pending", true),
                    ("pending", false)
                ],
                6
            ),
            vec![0, 1, 3]
        );
        assert_eq!(
            s(
                &[("ended", false), ("skipped", false), ("pending", true)],
                6
            ),
            vec![2]
        );
    }

    #[test]
    fn only_the_main_agents_own_conversation_plans_a_swarm_within_its_limit() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        crate::workspaces::ensure_main_agent(v).unwrap();
        let cwd = crate::workspaces::main_agent_cwd(v).unwrap();
        let other = crate::workspaces::create(v, "Launch", "Ship it").unwrap();
        session_store::start_turn(v, "s-main-swarm", "claude", &cwd, "R-swarm", "go").unwrap();
        session_store::start_turn(v, "s-other-swarm", "claude", &other.cwd, "R-other", "go")
            .unwrap();
        let caller = |scope: &str, depth: u32| crate::app_actions::Caller {
            run_id: "R-swarm".into(),
            surface: "chat".into(),
            scope: scope.into(),
            access: crate::app_mcp::AppAccess::Write,
            depth,
            perm: "bypassPermissions".into(),
        };
        let tasks = json!([
            {"workspace": other.id, "prompt": "Research vendor A", "title": "Vendor A"},
            {"workspace": "quick-chats", "prompt": "Research vendor B"}
        ]);
        let now = Utc::now();
        let plan = plan_swarm(
            v,
            &caller("s-main-swarm", 0),
            "Compare vendors",
            &tasks,
            Some(5.0),
            None,
            now,
        )
        .unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].title, "Vendor A");
        assert_eq!(plan.tasks[1].title, "Research vendor B");
        assert!(!plan.tasks[0].build);
        assert_eq!(plan.budget_usd, Some(5.0));
        assert!(
            plan_swarm_with(
                v,
                &caller("s-other-swarm", 0),
                "x",
                &tasks,
                None,
                None,
                now,
                &MainAgentSettings::default()
            )
            .is_err(),
            "not the COO"
        );
        // L1: the COO's own turn an agent sent counts from depth 0.
        assert!(
            plan_swarm_with(
                v,
                &caller("s-main-swarm", 1),
                "x",
                &tasks,
                None,
                None,
                now,
                &MainAgentSettings::default()
            )
            .is_ok(),
            "the COO messaged by an agent"
        );
        let seven = json!((0..7)
            .map(|i| json!({"workspace": "quick-chats", "prompt": format!("part {i}")}))
            .collect::<Vec<_>>());
        assert!(plan_swarm_with(
            v,
            &caller("s-main-swarm", 0),
            "x",
            &seven,
            None,
            None,
            now,
            &MainAgentSettings {
                swarm_max: 6,
                ..MainAgentSettings::default()
            }
        )
        .unwrap_err()
        .contains("1 to 6"));
        // Huge swarms on Haiku 5.5 by default; a part or the swarm may name another model.
        let thirty = json!((0..30)
            .map(|i| json!({"workspace": "quick-chats", "prompt": format!("part {i}")}))
            .collect::<Vec<_>>());
        let big = plan_swarm_with(
            v,
            &caller("s-main-swarm", 0),
            "x",
            &thirty,
            None,
            None,
            now,
            &MainAgentSettings {
                swarm_max: 50,
                ..MainAgentSettings::default()
            },
        )
        .unwrap();
        assert_eq!(
            (big.tasks.len(), big.model.as_deref()),
            (30, Some("claude-haiku-5-5"))
        );
        let mixed = json!([
            {"workspace": "quick-chats", "prompt": "p", "model": "claude-opus-5-5"},
            {"workspace": "quick-chats", "prompt": "q"}
        ]);
        let plan = plan_swarm_with(
            v,
            &caller("s-main-swarm", 0),
            "x",
            &mixed,
            None,
            Some("claude-sonnet-5-5"),
            now,
            &MainAgentSettings::default(),
        )
        .unwrap();
        assert_eq!(plan.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(plan.tasks[0].model.as_deref(), Some("claude-opus-5-5"));
        assert_eq!(plan.tasks[1].model, None);
        let agents_default = MainAgentSettings {
            swarm_model: String::new(),
            ..MainAgentSettings::default()
        };
        assert_eq!(
            plan_swarm_with(
                v,
                &caller("s-main-swarm", 0),
                "x",
                &tasks,
                None,
                None,
                now,
                &agents_default
            )
            .unwrap()
            .model,
            None,
            "empty: the agent's default"
        );
        assert!(plan_swarm_with(
            v,
            &caller("s-main-swarm", 0),
            "x",
            &tasks,
            None,
            Some("bad model"),
            now,
            &MainAgentSettings::default()
        )
        .is_err());
        assert!(plan_swarm(
            v,
            &caller("s-main-swarm", 0),
            "x",
            &json!([{"workspace": "nope", "prompt": "p"}]),
            None,
            None,
            now
        )
        .is_err());
        assert!(plan_swarm_with(
            v,
            &caller("s-main-swarm", 0),
            "",
            &tasks,
            None,
            None,
            now,
            &MainAgentSettings::default()
        )
        .is_err());
        // A swarm's parts never wake the Main agent one by one.
        let mut planned = plan;
        planned.tasks[0].queue_id = Some(41);
        save_swarms(v, &[planned]);
        assert!(swarm_rows(v).contains(&41));
    }

    #[test]
    fn suggestions_are_counted_and_never_means_never() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        record_outcome(v, "Prepare the board meeting", "done", now).unwrap();
        record_outcome(v, "Archive the old workspace", "not_now", now).unwrap();
        let stats = record_outcome(v, "Draft weekly digests", "never", now).unwrap();
        assert_eq!(
            (stats.answered, stats.taken, stats.declined_for_good),
            (3, 1, 1)
        );
        assert!((stats.taken_share.unwrap() - 1.0 / 3.0).abs() < 1e-9);
        record_outcome(v, "Draft weekly digests", "never", now).unwrap();
        assert_eq!(
            never_list(&load_suggestions(v)),
            vec!["Draft weekly digests".to_string()]
        );
        assert!(record_outcome(v, "x", "maybe", now).is_err());
        assert!(record_outcome(v, "  ", "done", now).is_err());
        // A month later, the old answers no longer count.
        let later = stats_at(v, now + Duration::days(31));
        assert_eq!(later.answered, 0);
        // A check-in lists the declined ones.
        crate::workspaces::ensure_main_agent(v).unwrap();
        let Decision::Wake(first) = decide_pulse(v, now, &anytime(), &WakeState::default()) else {
            panic!("first check-in");
        };
        assert!(
            first
                .message()
                .contains("never suggest again:\n- Draft weekly digests"),
            "{}",
            first.message()
        );
    }

    fn stats_at(v: &Path, now: DateTime<Utc>) -> SuggestionStats {
        stats(&load_suggestions(v), now)
    }

    #[test]
    fn an_unreadable_settings_file_never_widens_what_it_may_do() {
        assert_eq!(settings_from(None), MainAgentSettings::default());
        for broken in ["{not json", r#"{"intervalMin": 1}"#] {
            let s = settings_from(Some(broken));
            assert!(!s.act_on_own && !s.check_ins, "{broken}");
        }
        let saved = settings_from(Some(r#"{"actOnOwn": false, "dailyCapUsd": 20}"#));
        assert!(!saved.act_on_own && saved.check_ins);
        assert_eq!(saved.daily_cap_usd, 20.0);
    }

    #[test]
    fn a_swarm_part_in_an_ordinary_workspace_keeps_the_swarm_limit() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        swarm_parts::add(v, "s-part-slot");
        let cap = settings().swarm_concurrency as usize;
        assert_eq!(swarm_part_limit(v, Some("s-part-slot"), false), Some(cap));
        assert_eq!(
            swarm_part_limit(v, Some("s-part-slot"), true),
            None,
            "a build workspace part"
        );
        assert_eq!(swarm_part_limit(v, Some("s-not-a-part"), false), None);
        assert_eq!(swarm_part_limit(v, None, false), None);
        assert!(is_swarm_part(v, Some("s-part-slot")));
        assert!(!is_swarm_part(v, Some("s-not-a-part")) && !is_swarm_part(v, None));
    }

    #[test]
    fn concurrent_state_and_settings_saves_use_unique_temp_files_without_clobbering() {
        use std::sync::{Arc, Barrier};

        let vault = Arc::new(tempfile::tempdir().unwrap());
        let scratch = Arc::new(tempfile::tempdir().unwrap());
        let json_path = Arc::new(scratch.path().join("concurrent.json"));
        let threads = 8usize;
        let barrier = Arc::new(Barrier::new(threads));
        let mut handles = Vec::with_capacity(threads);
        for t in 0..threads {
            let vault = Arc::clone(&vault);
            let json_path = Arc::clone(&json_path);
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                for iter in 0..10u32 {
                    let state = WakeState {
                        day: format!("2026-11-20-{t}"),
                        count: iter,
                        ..WakeState::default()
                    };
                    save_state(vault.path(), &state);
                    let payload = serde_json::json!({"thread": t, "iter": iter}).to_string();
                    atomic_write_json(&json_path, &payload).unwrap();
                    save_settings(&MainAgentSettings::default()).unwrap();
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }
        let final_state = load_state(vault.path());
        assert!(
            final_state.day.starts_with("2026-11-20-"),
            "{final_state:?}"
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&*json_path).unwrap()).unwrap();
        assert!(parsed.get("thread").is_some());
        let leftovers: Vec<_> = std::fs::read_dir(scratch.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    }
}

#[cfg(test)]
mod coo_settings_patch_tests {
    use super::*;

    fn patch(value: serde_json::Value) -> SettingsPatch {
        patch_from(value).unwrap()
    }

    #[test]
    fn a_patch_changes_only_the_fields_it_names() {
        let base = MainAgentSettings::default();
        let next = patched(
            base.clone(),
            patch(serde_json::json!({ "dailyCapUsd": 20.0 })),
        )
        .unwrap();
        assert_eq!(next.daily_cap_usd, 20.0);
        assert_eq!(
            MainAgentSettings {
                daily_cap_usd: base.daily_cap_usd,
                ..next
            },
            base
        );
    }

    #[test]
    fn a_patch_is_validated_like_a_settings_save() {
        let base = MainAgentSettings::default();
        for bad in [
            serde_json::json!({ "intervalMin": 5 }),
            serde_json::json!({ "startHour": 20, "endHour": 8 }),
            serde_json::json!({ "dailyCapUsd": 5000.0 }),
            serde_json::json!({ "swarmMax": 0 }),
            serde_json::json!({ "swarmConcurrency": 99 }),
            serde_json::json!({ "swarmModel": "two words" }),
        ] {
            assert!(patched(base.clone(), patch(bad.clone())).is_err(), "{bad}");
        }
        assert!(patch_from(serde_json::json!({ "bogus": 1 })).is_err());
    }

    #[test]
    fn the_diff_shows_old_to_new_and_flags_widening() {
        let old = MainAgentSettings::default();
        let new = patched(
            old.clone(),
            patch(serde_json::json!({ "dailyCapUsd": 250.0, "checkIns": false })),
        )
        .unwrap();
        let lines = settings_diff(&old, &new);
        assert!(
            lines.contains(&"Daily cap: $100 → $250".to_string()),
            "{lines:?}"
        );
        assert!(
            lines.contains(&"Check-ins: on → off".to_string()),
            "{lines:?}"
        );
        assert!(lines.last().unwrap().contains("widens"), "{lines:?}");
        let narrower = patched(
            old.clone(),
            patch(serde_json::json!({ "dailyCapUsd": 10.0 })),
        )
        .unwrap();
        assert!(!settings_diff(&old, &narrower)
            .iter()
            .any(|l| l.contains("widens")));
        assert!(settings_diff(&old, &old).is_empty());
    }
}
