//! One conversation's subagents (COO rebuild L2, 2026-11-10): the read behind
//! the side panel's Subagents tab (L3) and the `sessions.subagents` agent
//! action.
//!
//! A subagent is a conversation another one started (`start_session`, a
//! worker batch, a swarm part): its `sessions.parent_key` names the starter
//! and its `fork_kind` is `subagent` (L1). The parent is read as a chain: the
//! COO's every daily thread, or the handovers that led to a conversation, so
//! a roll or a handover never hides what the earlier thread started. A
//! subagent that handed its own work over is still one row: its successors'
//! turns, time and tokens count with it, and its status is the last one's.
//! A start still waiting for its first turn has no session row yet, so the
//! queued rows that name the parent in their payload are listed too.
//!
//! Working time is the sum of the turns' own spans (a running turn up to
//! now); the wall span runs from the first turn's start to the last one's
//! end (or now). Tokens and cost are summed from every usage ledger row of
//! the conversation's turns (its consultations included). Nothing here
//! writes.

use crate::run_queue::{QueueRow, RowStatus, RunSource};
use crate::session_store;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

mod activity;
pub(crate) use activity::progress as reported_progress;
pub use activity::{SubagentActivity, SubagentGroup};

/// Earlier chats listed at most (a lead's other chats, the COO's quick
/// chats). L8: they left the sidebar for here, so the list is long; the
/// project's Conversation history has the rest.
pub(crate) const EARLIER_MAX: usize = 100;
/// Finished subagents of the current conversation listed at most, newest first.
pub(crate) const DONE_MAX: usize = 200;
/// A handover predecessor's subagents still count as the current
/// conversation's for this long after it (a mid-task handover must not
/// empty the tab).
const HANDOVER_WINDOW_HOURS: i64 = 24;

/// One row of the Subagents tab.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubagentRow {
    /// The conversation it was started as (messages go to its chain's
    /// current one, `chat_chain::current_of`).
    pub key: String,
    /// The conversation its newest turn ran in (itself, or its last
    /// handover successor).
    pub current: String,
    pub title: Option<String>,
    pub agent: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// The account its newest priced turn ran on (`usage_ledger.account_ref`).
    pub account: Option<String>,
    /// `running`, `queued`, `done`, `failed` or `stopped`.
    #[cfg_attr(test, ts(as = "crate::ts_bindings::SubagentStatus"))]
    pub status: String,
    /// Queued: its place among the queued turns (1 = next).
    pub queue_place: Option<u32>,
    /// Queued: what it waits for, in the words every queue surface uses after
    /// "Waiting for" (`run_pump::waiting_reason`: "a free subagent slot: this
    /// chat is at its limit of 6 subagents at once (Settings, Chat, Chats at once)"); none while it
    /// runs, and none when nothing but its turn in line holds it.
    pub waiting_for: Option<String>,
    /// RFC 3339: when it was started (its first turn, or its queue row).
    pub started_at: String,
    /// RFC 3339: when its last turn ended; none while one runs or waits.
    pub ended_at: Option<String>,
    /// Milliseconds its turns ran, a running turn up to now.
    #[cfg_attr(test, ts(type = "number"))]
    pub working_ms: i64,
    /// Milliseconds from its first turn's start to its last turn's end (or
    /// now while one runs).
    #[cfg_attr(test, ts(type = "number"))]
    pub wall_ms: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub input_tokens: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub cached_input_tokens: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub output_tokens: i64,
    /// Input plus output.
    #[cfg_attr(test, ts(type = "number"))]
    pub tokens: i64,
    /// Summed from the rows that know their cost; none when none does.
    pub cost_usd: Option<f64>,
    /// An earlier chat (a lead's other chat, a quick chat under the COO),
    /// not a started subagent.
    pub earlier: bool,
    /// Every conversation counted in it, the started one first.
    pub conversations: Vec<String>,
    /// The tracker issue's key ("ENG-123") it works (lane 4's link).
    pub tracker_issue: Option<String>,
    /// That issue as the mirror has it (X1; X0 section 6): none while
    /// untracked or while its create is pending.
    pub issue: Option<crate::tracker::work::WorkIssueRef>,
    /// K3: the conversation a small follow-up was sent to when Hollow
    /// started this one fresh with a brief instead (`fresh_brief`); the tab
    /// says "Fresh chat with a brief".
    pub fresh_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub parent_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub group: Option<SubagentGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub activity: Option<SubagentActivity>,
    /// Running or queued: its live run-queue row, so Stop ends that turn
    /// by its row (`cancel_queued_run`), never the next turn the same key
    /// starts while the confirm is open (overnight 11-13).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub queue_id: Option<i64>,
    /// Who started it (11-14 card 2): `lead` (a project lead), `main` (the
    /// COO), `chat` (any other conversation: a worker, a subagent) or
    /// `you` (no starting conversation: the person).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "\"lead\" | \"main\" | \"chat\" | \"you\""))]
    pub started_by: Option<String>,
}

/// `session_subagents`: a conversation's subagents, by state.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubagentsView {
    /// The conversation asked about.
    pub parent: String,
    /// Its chain, current first: the conversations whose subagents these are.
    pub parents: Vec<String>,
    /// Running first, then queued by place.
    pub active: Vec<SubagentRow>,
    /// Ended, newest first: the current conversation's own (itself plus any
    /// handover predecessor active within the last 24 hours). The rest of the
    /// chain's finished ones are `done_older`, paged in by "Show older".
    pub done: Vec<SubagentRow>,
    /// The chain's older finished subagents, newest first: the first `older`
    /// the caller asked for, none by default.
    pub done_older: Vec<SubagentRow>,
    /// How many older finished subagents the chain has in all (the Show older
    /// control is there while `done_older` lists fewer).
    pub older_total: u32,
    /// Earlier chats, newest first ("Earlier chats", listed last under Done).
    pub earlier: Vec<SubagentRow>,
    pub running: u32,
    pub queued: u32,
    /// How many of the current conversation's have ended (the chip's count;
    /// may be more than `done` lists, never counts the older ones).
    pub done_count: u32,
    /// RFC 3339: when this was read (the live elapsed counts from here).
    pub at: String,
}

/// One conversation's turn: its span and status.
pub(crate) struct Turn {
    pub(crate) started: Option<DateTime<Utc>>,
    pub(crate) ended: Option<DateTime<Utc>>,
    status: Option<String>,
}

fn stamp(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at.trim())
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// The chain `parent` belongs to, current first: the COO's threads, or the
/// handovers that led to the current conversation.
pub(crate) fn parent_chain(vault: &Path, parent: &str) -> Vec<String> {
    let current =
        crate::chat_chain::current_of(vault, parent).unwrap_or_else(|| parent.to_string());
    let mut out = vec![current.clone()];
    for key in crate::chat_chain::chain_keys(vault, &current) {
        if !out.contains(&key) {
            out.push(key);
        }
    }
    if !out.iter().any(|k| k == parent) {
        out.push(parent.to_string());
    }
    out
}

/// `key` and its handover successors, in order (`from -> successor`).
fn handover_chain(key: &str, next: &HashMap<String, String>) -> Vec<String> {
    let mut out = vec![key.to_string()];
    let mut at = key.to_string();
    while let Some(succ) = next.get(&at) {
        if out.contains(succ) {
            break;
        }
        out.push(succ.clone());
        at = succ.clone();
    }
    out
}

struct SessionRow {
    key: String,
    title: Option<String>,
    agent: String,
    created_at: String,
    parent_key: Option<String>,
}

fn sessions_where(
    conn: &rusqlite::Connection,
    sql_where: &str,
    args: &[String],
    limit: usize,
) -> Result<Vec<SessionRow>, String> {
    let sql = format!(
        "SELECT key, title, agent, created_at, parent_key FROM sessions WHERE {sql_where} \
         ORDER BY last_activity_at DESC LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("subagents: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            Ok(SessionRow {
                key: row.get(0)?,
                title: row
                    .get::<_, Option<String>>(1)?
                    .filter(|t| !t.trim().is_empty()),
                agent: row.get(2)?,
                created_at: row.get(3)?,
                parent_key: row.get(4)?,
            })
        })
        .map_err(|e| format!("subagents: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("subagents: {e}"))?;
    Ok(rows)
}

/// When each of `keys` was last active (a conversation with no row, or an
/// unreadable stamp, is left out).
fn sessions_activity(
    conn: &rusqlite::Connection,
    keys: &[String],
) -> Result<HashMap<String, DateTime<Utc>>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT key, last_activity_at FROM sessions WHERE key IN ({})",
            placeholders(keys.len())
        ))
        .map_err(|e| format!("subagents: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("subagents: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("subagents: {e}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|(k, at)| stamp(&at).map(|t| (k, t)))
        .collect())
}

/// The turns, totals, newest model and account of conversations `keys`
/// (also the audit's `run_ended` totals, X2, so both read the same).
pub(crate) struct Stats {
    pub(crate) turns: Vec<Turn>,
    pub(crate) input: i64,
    pub(crate) cached: i64,
    pub(crate) output: i64,
    pub(crate) cost: Option<f64>,
    /// Usage rows, and those that know their cost (fewer: a floor).
    pub(crate) rows: i64,
    pub(crate) priced: i64,
    pub(crate) model: Option<String>,
    pub(crate) account: Option<String>,
}

/// The turns of conversations `keys`, oldest first.
fn turns_of(conn: &rusqlite::Connection, keys: &[String]) -> Result<Vec<Turn>, String> {
    let marks = placeholders(keys.len());
    let mut stmt = conn
        .prepare(&format!(
            "SELECT started_at, ended_at, status FROM turns WHERE session_key IN ({marks}) \
             ORDER BY started_at"
        ))
        .map_err(|e| format!("subagent turns: {e}"))?;
    let turns = stmt
        .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
            Ok(Turn {
                started: row.get::<_, Option<String>>(0)?.as_deref().and_then(stamp),
                ended: row.get::<_, Option<String>>(1)?.as_deref().and_then(stamp),
                status: row.get(2)?,
            })
        })
        .map_err(|e| format!("subagent turns: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("subagent turns: {e}"))?;
    Ok(turns)
}

pub(crate) fn stats(conn: &rusqlite::Connection, keys: &[String]) -> Result<Stats, String> {
    let marks = placeholders(keys.len());
    let turns = turns_of(conn, keys)?;
    let (input, cached, output, cost, priced, rows): (i64, i64, i64, f64, i64, i64) = conn
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.cached_input_tokens), 0), \
                 COALESCE(SUM(u.output_tokens), 0), COALESCE(SUM(u.cost_usd), 0), COUNT(u.cost_usd), \
                 COUNT(u.id) FROM usage_ledger u JOIN turns t ON t.run_id = u.run_id \
                 WHERE t.session_key IN ({marks})"
            ),
            rusqlite::params_from_iter(keys.iter()),
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|e| format!("subagent usage: {e}"))?;
    let newest: Option<(Option<String>, Option<String>, Option<String>)> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT u.model, u.requested_model, u.account_ref FROM usage_ledger u \
                 JOIN turns t ON t.run_id = u.run_id \
                 WHERE t.session_key IN ({marks}) AND u.surface = 'chat' \
                 ORDER BY t.started_at DESC, u.id DESC LIMIT 1"
            ))
            .map_err(|e| format!("subagent model: {e}"))?;
        let mut rows = stmt
            .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("subagent model: {e}"))?;
        rows.next()
            .transpose()
            .map_err(|e| format!("subagent model: {e}"))?
    };
    let (model, requested, account) = newest.unwrap_or((None, None, None));
    Ok(Stats {
        turns,
        input,
        cached,
        output,
        cost: (priced > 0).then_some(cost),
        rows,
        priced,
        model: model.or(requested).filter(|m| !m.trim().is_empty()),
        account: account.filter(|a| !a.trim().is_empty()),
    })
}

/// What conversations `keys` used (L9's subagent budgets): tokens (input
/// plus output, as a row sums them) and working time at `now`.
pub(crate) fn chain_usage(
    conn: &rusqlite::Connection,
    keys: &[String],
    now: DateTime<Utc>,
) -> Result<crate::cfo::budgets::Used, String> {
    if keys.is_empty() {
        return Ok(crate::cfo::budgets::Used::default());
    }
    let stats = stats(conn, keys)?;
    let turns: Vec<Span> = stats.turns.iter().map(|t| (t.started, t.ended)).collect();
    let (working_ms, _, _) = spans(&turns, now);
    Ok(crate::cfo::budgets::Used {
        tokens: stats.input.saturating_add(stats.output).max(0) as u64,
        working_ms,
    })
}

/// A turn's end status as the tab says it.
fn ended_status(status: Option<&str>) -> &'static str {
    match status {
        Some("error" | "failed" | "spawn_failed") => "failed",
        Some("cancelled" | "interrupted" | "stopped") => "stopped",
        _ => "done",
    }
}

/// A row's status from its turns and its live queue row (`row_for` and the
/// walk's light read say it the same way).
fn status_of(turns: &[Turn], queued: Option<&QueueRow>) -> &'static str {
    match queued {
        Some(row) if row.status == RowStatus::Queued => "queued",
        Some(row) if row.status.is_live() => "running",
        _ if turns
            .iter()
            .any(|t| t.started.is_some() && t.ended.is_none() && t.status.is_none()) =>
        {
            "running"
        }
        _ => ended_status(turns.last().and_then(|t| t.status.as_deref())),
    }
}

fn is_active(status: &str) -> bool {
    status == "running" || status == "queued"
}

/// Q-C1: a conversation's status, the stamp Done sorts it by (its end, or
/// its start) and its handover chain, from one turns read. The walk reads
/// every descendant this way and builds full rows (usage, activity) only for
/// the ones it shows.
fn light(
    conn: &rusqlite::Connection,
    session: &SessionRow,
    next: &HashMap<String, String>,
    live: &HashMap<String, &QueueRow>,
    now: DateTime<Utc>,
) -> Result<(&'static str, String, Vec<String>), String> {
    let chain = handover_chain(&session.key, next);
    let current = chain.last().cloned().unwrap_or_else(|| session.key.clone());
    let turns = turns_of(conn, &chain)?;
    let status = status_of(&turns, live.get(&current).copied());
    let spans_in: Vec<Span> = turns.iter().map(|t| (t.started, t.ended)).collect();
    let (_, _, ended) = spans(&spans_in, now);
    let started = turns
        .iter()
        .filter_map(|t| t.started)
        .min()
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| session.created_at.clone());
    let newest = match ended {
        Some(end) if !is_active(status) => end.to_rfc3339(),
        _ => started,
    };
    Ok((status, newest, chain))
}

/// Working time, wall span and end of `turns` at `now`.
/// A turn's start and end, as [`spans`] reads them.
pub(crate) type Span = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

pub(crate) fn spans(turns: &[Span], now: DateTime<Utc>) -> (i64, i64, Option<DateTime<Utc>>) {
    let mut working = 0i64;
    let mut first: Option<DateTime<Utc>> = None;
    let mut last_end: Option<DateTime<Utc>> = None;
    let mut open = false;
    for (start, end) in turns {
        let Some(start) = *start else { continue };
        first = Some(first.map_or(start, |f| f.min(start)));
        match end {
            Some(end) => {
                working += (*end - start).num_milliseconds().max(0);
                last_end = Some(last_end.map_or(*end, |l| l.max(*end)));
            }
            None => {
                working += (now - start).num_milliseconds().max(0);
                open = true;
            }
        }
    }
    let until = if open { Some(now) } else { last_end };
    let wall = match (first, until) {
        (Some(f), Some(u)) => (u - f).num_milliseconds().max(0),
        _ => 0,
    };
    (working, wall, if open { None } else { last_end })
}

/// The newest chat queue row per conversation among `live`.
fn live_by_key(live: &[QueueRow]) -> HashMap<String, &QueueRow> {
    let mut out: HashMap<String, &QueueRow> = HashMap::new();
    for row in live.iter().filter(|r| r.source == RunSource::Chat) {
        if let Some(key) = row.source_id.as_deref() {
            let newer = out.get(key).is_none_or(|old| old.id < row.id);
            if newer {
                out.insert(key.to_string(), row);
            }
        }
    }
    out
}

/// Each queued chat row's place among the queued rows (1 = next), in the
/// queue's own claim order, and what it waits for (`waits`, 2026-11-12).
struct Places {
    place: HashMap<i64, u32>,
    waits: HashMap<i64, String>,
    /// Queued rows that read Working, not Queued (2026-11-16): nothing
    /// truly holds them back (`RunQueueItem::working`).
    working: std::collections::HashSet<i64>,
}

impl Places {
    fn of(&self, row: &QueueRow) -> Option<u32> {
        self.place.get(&row.id).copied()
    }

    fn wait_of(&self, row: &QueueRow) -> Option<String> {
        self.waits.get(&row.id).cloned()
    }

    /// `row` is queued in the table and truly waits (a limit, memory,
    /// another run's folder), as opposed to being about to start.
    fn queued_for_real(&self, row: &QueueRow) -> bool {
        row.status == RowStatus::Queued && !self.working.contains(&row.id)
    }
}

fn queue_places(vault: &Path, live: &[QueueRow], now: DateTime<Utc>) -> Places {
    let facts = crate::run_pump::QueueFacts::read(vault, live);
    let queued = || live.iter().filter(|r| r.status == RowStatus::Queued);
    let items: Vec<_> = queued()
        .map(|r| {
            (
                r.id,
                crate::run_pump::item_with_reason(vault, r, None, &facts, now),
            )
        })
        .collect();
    Places {
        place: queued()
            .enumerate()
            .map(|(i, r)| (r.id, i as u32 + 1))
            .collect(),
        working: items
            .iter()
            .filter(|(_, item)| item.reads_working())
            .map(|(id, _)| *id)
            .collect(),
        waits: items
            .into_iter()
            .filter_map(|(id, item)| item.waiting_reason.map(|w| (id, w)))
            .collect(),
    }
}

#[allow(clippy::too_many_arguments)]
fn row_for(
    conn: &rusqlite::Connection,
    vault: &Path,
    session: &SessionRow,
    next: &HashMap<String, String>,
    live: &HashMap<String, &QueueRow>,
    places: &Places,
    earlier: bool,
    now: DateTime<Utc>,
) -> Result<SubagentRow, String> {
    let chain = handover_chain(&session.key, next);
    let current = chain.last().cloned().unwrap_or_else(|| session.key.clone());
    let s = stats(conn, &chain)?;
    let spans_in: Vec<_> = s.turns.iter().map(|t| (t.started, t.ended)).collect();
    let (working_ms, wall_ms, ended) = spans(&spans_in, now);
    let queued_row = live.get(&current).copied();
    // A queued row nothing truly holds back is working (it is being
    // prepared), never "queued" (2026-11-16).
    let status = match status_of(&s.turns, queued_row) {
        "queued" if queued_row.is_some_and(|r| !places.queued_for_real(r)) => "running",
        other => other,
    };
    let queue_place = queued_row
        .filter(|r| places.queued_for_real(r))
        .and_then(|r| places.of(r));
    // Q-C4: a re-queued conversation whose new run has no turn row yet has
    // no current step: the newest turn is the previous run's, so its steps
    // and plan would read as this run's. A turn counts once it is open or
    // started at or after the queue row was requested.
    let requested = queued_row.and_then(|r| stamp(&r.requested_at));
    let run_has_turn = s.turns.iter().any(|t| {
        (t.started.is_some() && t.ended.is_none() && t.status.is_none())
            || t.started
                .zip(requested)
                .is_some_and(|(at, asked)| at >= asked)
    });
    let starting = queued_row.is_some_and(|r| r.status.is_live()) && !run_has_turn;
    let live_payload = queued_row.map(|r| &r.payload);
    let effort = live_payload
        .and_then(|p| p.get("effort"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| session_store::last_effort(vault, &current).ok().flatten());
    let model = s.model.clone().or_else(|| {
        live_payload
            .and_then(|p| p.get("model"))
            .and_then(serde_json::Value::as_str)
            .filter(|m| !m.is_empty())
            .map(str::to_string)
    });
    let started_at = s
        .turns
        .iter()
        .filter_map(|t| t.started)
        .min()
        .map(|t| t.to_rfc3339())
        .unwrap_or_else(|| session.created_at.clone());
    let active = status == "running" || status == "queued";
    Ok(SubagentRow {
        key: session.key.clone(),
        activity: if earlier {
            None
        } else if starting {
            Some(SubagentActivity::default())
        } else {
            activity::read(conn, &current, status).ok()
        },
        parent_key: session.parent_key.clone(),
        group: None,
        current,
        title: session.title.clone(),
        agent: session.agent.clone(),
        model,
        effort,
        account: s.account.clone(),
        status: status.to_string(),
        queue_place,
        waiting_for: queued_row
            .filter(|r| places.queued_for_real(r))
            .and_then(|r| places.wait_of(r)),
        started_at,
        ended_at: if active {
            None
        } else {
            ended.map(|t| t.to_rfc3339())
        },
        working_ms,
        wall_ms,
        input_tokens: s.input,
        cached_input_tokens: s.cached,
        output_tokens: s.output,
        tokens: s.input + s.output,
        cost_usd: s.cost,
        earlier,
        conversations: chain,
        tracker_issue: None,
        issue: None,
        fresh_from: crate::fresh_brief::from_in(conn, &session.key),
        queue_id: queued_row.filter(|r| r.status.is_live()).map(|r| r.id),
        started_by: None,
    })
}

/// A start still waiting for its first turn (no session row yet).
fn queued_start(
    vault: &Path,
    row: &QueueRow,
    places: &Places,
    now: DateTime<Utc>,
) -> Option<SubagentRow> {
    let key = row.source_id.clone()?;
    let text = |field: &str| {
        row.payload
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let title = row
        .detail
        .as_deref()
        .map(crate::chat_title::short_title)
        .filter(|t| !t.trim().is_empty());
    let queued = places.queued_for_real(row);
    let since = stamp(&row.requested_at).unwrap_or(now);
    let fresh_from = crate::fresh_brief::from_of(vault, &key);
    Some(SubagentRow {
        activity: None,
        group: None,
        parent_key: crate::run_pump::subagent_parent_of(row).map(str::to_string),
        current: key.clone(),
        conversations: vec![key.clone()],
        key,
        title,
        agent: text("agent").unwrap_or_else(|| "claude".to_string()),
        model: text("model"),
        effort: text("effort"),
        account: None,
        status: if queued { "queued" } else { "running" }.to_string(),
        queue_place: if queued { places.of(row) } else { None },
        waiting_for: if queued { places.wait_of(row) } else { None },
        started_at: row.requested_at.clone(),
        ended_at: None,
        working_ms: 0,
        wall_ms: if queued {
            0
        } else {
            (now - since).num_milliseconds().max(0)
        },
        input_tokens: 0,
        cached_input_tokens: 0,
        output_tokens: 0,
        tokens: 0,
        cost_usd: None,
        earlier: false,
        tracker_issue: None,
        issue: None,
        fresh_from,
        queue_id: Some(row.id),
        started_by: None,
    })
}

/// What kind of conversation `key` is, as a row's starter: `lead`, `main`
/// (the COO) or `chat`.
fn starter_kind(vault: &Path, key: &str) -> &'static str {
    if crate::chat_chain::is_lead_conversation(vault, key) {
        "lead"
    } else if crate::chat_chain::is_main_conversation(vault, key) {
        "main"
    } else {
        "chat"
    }
}

/// The folder whose other chats are `parent`'s earlier chats: a project
/// lead's project folder; none for any other conversation (the COO's quick
/// chats list in the sidebar's Quick chats section since 2026-11-14).
fn earlier_folder(vault: &Path, current: &str) -> Option<String> {
    // L8: a lead's every conversation (a successor of its handovers too),
    // and a lead that hasn't run yet: its project's folder.
    if let Some(lead) = crate::chat_chain::lead_root(vault, current) {
        return session_store::session_by_key(vault, current)
            .ok()
            .flatten()
            .map(|s| s.cwd)
            .or_else(|| {
                let all = crate::workspaces::list(vault).ok()?;
                crate::project_agents::project_of(vault, &all, &lead).map(|w| w.cwd.clone())
            });
    }
    // The COO's earlier chats stopped including the quick chats (quick
    // chats, 2026-11-14): they list in the sidebar's Quick chats section.
    None
}

/// `parent`'s subagents at `now` (L2), with no older finished ones paged in.
#[cfg(test)]
pub(crate) fn subagents_at(
    vault: &Path,
    parent: &str,
    now: DateTime<Utc>,
) -> Result<SubagentsView, String> {
    subagents_paged(vault, parent, now, 0)
}

/// `parent`'s subagents at `now`. Active spans the whole chain. Done is the
/// current conversation's (its own key, plus handover predecessors active in
/// the last 24 hours); the chain's other finished subagents are the older
/// ones, `older` of them listed newest first (the "Show older" control asks
/// for more in batches of 25, the panel's `OLDER_BATCH`).
pub(crate) fn subagents_paged(
    vault: &Path,
    parent: &str,
    now: DateTime<Utc>,
    older: usize,
) -> Result<SubagentsView, String> {
    let parent = parent.trim();
    if !crate::agent_sessions::is_safe_session_key(parent) {
        return Err(format!(
            "`{parent}` is not a session key (list_sessions shows them)."
        ));
    }
    let parents = parent_chain(vault, parent);
    let next: HashMap<String, String> = crate::app_sessions::handover_pairs(vault)
        .into_iter()
        .map(|(succ, from)| (from, succ))
        .collect();
    let live = crate::run_queue::live_rows(vault).unwrap_or_default();
    let by_key = live_by_key(&live);
    let places = queue_places(vault, &live, now);
    let current = parents[0].clone();
    let earlier_cwd = earlier_folder(vault, &current);
    let (mut active, mut done, mut done_older, older_total, mut earlier, done_count, visited) =
        session_store::with_read(vault, |conn| {
            let marks = placeholders(parents.len());
            // The current conversation's own scope: itself and the
            // handover predecessors that were last active within a day.
            let mut scope: Vec<String> = vec![current.clone()];
            let mut rest: Vec<String> = Vec::new();
            if parents.len() > 1 {
                let recent = sessions_activity(conn, &parents[1..])?;
                for key in &parents[1..] {
                    let fresh = recent
                        .get(key)
                        .is_some_and(|t| now - *t < chrono::Duration::hours(HANDOVER_WINDOW_HOURS));
                    if fresh {
                        scope.push(key.clone());
                    } else {
                        rest.push(key.clone());
                    }
                }
            }
            let children_of = |keys: &[String], limit: usize| {
                sessions_where(
                    conn,
                    &format!(
                        "parent_key IN ({}) AND fork_kind = '{}' AND archived = 0",
                        placeholders(keys.len()),
                        session_store::FORK_SUBAGENT
                    ),
                    keys,
                    limit,
                )
            };
            let children = children_of(&scope, DONE_MAX + 50)?;
            let mut active = Vec::new();
            // The current conversation's finished subagents, light (status,
            // sort stamp) until the newest DONE_MAX are chosen to show.
            let mut finished_here: Vec<(String, SessionRow)> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            // Conversations whose finished children belong under Done.
            let mut here: HashSet<String> = HashSet::new();
            // Q-C1: every listed conversation's subtree is walked, finished
            // or not, so a running grandchild of any of them is found.
            let mut frontier: Vec<String> = Vec::new();
            for child in children {
                seen.insert(child.key.clone());
                let (status, newest, chain) = light(conn, &child, &next, &by_key, now)?;
                seen.extend(chain.iter().cloned());
                frontier.extend(chain.iter().cloned());
                here.extend(chain);
                if is_active(status) {
                    active.push(row_for(
                        conn, vault, &child, &next, &by_key, &places, false, now,
                    )?);
                } else {
                    finished_here.push((newest, child));
                }
            }
            // The rest of the chain: a live one is Active whatever its age;
            // the finished ones are only read as far as asked for.
            let mut older_rows: Vec<SubagentRow> = Vec::new();
            let mut older_total = 0u32;
            // Every older candidate, listed or left for Show older.
            let mut older_keys: HashSet<String> = HashSet::new();
            if !rest.is_empty() {
                let candidates = children_of(&rest, 100_000)?;
                let mut finished: Vec<&SessionRow> = Vec::new();
                for child in &candidates {
                    seen.insert(child.key.clone());
                    older_keys.insert(child.key.clone());
                    let chain = handover_chain(&child.key, &next);
                    seen.extend(chain.iter().cloned());
                    frontier.extend(chain.iter().cloned());
                    let tail = chain.last().cloned().unwrap_or_else(|| child.key.clone());
                    if by_key.contains_key(&tail) {
                        let row = row_for(conn, vault, child, &next, &by_key, &places, false, now)?;
                        if is_active(&row.status) {
                            here.extend(chain);
                            active.push(row);
                            continue;
                        }
                    }
                    finished.push(child);
                }
                older_total = finished.len() as u32;
                for child in finished.into_iter().take(older) {
                    let row = row_for(conn, vault, child, &next, &by_key, &places, false, now)?;
                    if is_active(&row.status) {
                        active.push(row);
                        older_total = older_total.saturating_sub(1);
                    } else {
                        older_rows.push(row);
                    }
                }
            }
            let mut earlier = Vec::new();
            if let Some(cwd) = earlier_cwd.as_deref() {
                let mut args = vec![cwd.to_string()];
                args.extend(parents.iter().cloned());
                let others = sessions_where(
                    conn,
                    &format!(
                        "cwd = ? AND archived = 0 AND (fork_kind IS NULL OR fork_kind <> '{}') \
                         AND key NOT IN ({marks})",
                        session_store::FORK_SUBAGENT
                    ),
                    &args,
                    EARLIER_MAX + parents.len(),
                )?;
                for other in others.iter().filter(|o| !seen.contains(&o.key)) {
                    // A handover successor is part of its predecessor's row.
                    if next.values().any(|succ| succ == &other.key) {
                        continue;
                    }
                    if earlier.len() >= EARLIER_MAX {
                        break;
                    }
                    earlier.push(row_for(
                        conn, vault, other, &next, &by_key, &places, true, now,
                    )?);
                }
            }
            // A listed earlier chat's running or queued subagents are Active
            // too: seed its conversations into the walk (its finished ones
            // aren't the current conversation's Done).
            for row in &earlier {
                seen.extend(row.conversations.iter().cloned());
                frontier.extend(row.conversations.iter().cloned());
            }
            // Include orchestrator descendants, bounded (250 found, 8 levels)
            // and cycle-safe. Handover keys belong to their existing row, not
            // another child row. A running or queued one is Active wherever
            // it sits; a finished one is Done only under the current
            // conversation's rows (an older chat's finished subtree stays in
            // Show older's pages; an earlier chat's finished subagents in
            // this folder are listed under Earlier by the pass below).
            let mut budget = 250usize.saturating_sub(active.len() + finished_here.len());
            for _ in 0..8 {
                if frontier.is_empty() || budget == 0 {
                    break;
                }
                let mut found = Vec::new();
                for keys in frontier.chunks(500) {
                    if budget == 0 {
                        break;
                    }
                    let got = children_of(keys, budget)?;
                    budget = budget.saturating_sub(got.len());
                    found.extend(got);
                }
                frontier.clear();
                for child in found {
                    if !seen.insert(child.key.clone()) {
                        continue;
                    }
                    let (status, newest, chain) = light(conn, &child, &next, &by_key, now)?;
                    let mine = child.parent_key.as_ref().is_some_and(|p| here.contains(p));
                    frontier.extend(chain.iter().cloned());
                    seen.extend(chain.iter().cloned());
                    if is_active(status) {
                        // Only under the current conversation's rows: an
                        // Earlier chat's running subagent's finished
                        // children aren't this conversation's Done.
                        if mine {
                            here.extend(chain);
                        }
                        active.push(row_for(
                            conn, vault, &child, &next, &by_key, &places, false, now,
                        )?);
                    } else if mine {
                        here.extend(chain);
                        finished_here.push((newest, child));
                    }
                }
            }
            // 11-14 card 2: every chat working in the folder shows, whoever
            // started it. A subagent another conversation started here (the
            // COO's, a worker's, one moved in) that the walk above didn't
            // reach: running or queued under Active, finished under Earlier.
            // The walk visits more than it lists (an earlier chat's finished
            // subagents), so this checks what is listed, not what was seen.
            let mut listed: HashSet<String> = older_keys;
            for row in active.iter().chain(earlier.iter()) {
                listed.extend(row.conversations.iter().cloned());
            }
            for (_, child) in &finished_here {
                listed.extend(handover_chain(&child.key, &next));
                listed.insert(child.key.clone());
            }
            if let Some(cwd) = earlier_cwd.as_deref() {
                // Newest first; most of a busy folder's are listed already,
                // so the window is wider than what it may add.
                let others = sessions_where(
                    conn,
                    &format!(
                        "cwd = ? AND archived = 0 AND fork_kind = '{}'",
                        session_store::FORK_SUBAGENT
                    ),
                    &[cwd.to_string()],
                    EARLIER_MAX * 5,
                )?;
                let successors: HashSet<&String> = next.values().collect();
                for other in &others {
                    if successors.contains(&other.key) || listed.contains(&other.key) {
                        continue;
                    }
                    let (status, _, chain) = light(conn, other, &next, &by_key, now)?;
                    seen.extend(chain.iter().cloned());
                    listed.extend(chain);
                    if is_active(status) {
                        active.push(row_for(
                            conn, vault, other, &next, &by_key, &places, false, now,
                        )?);
                    } else if earlier.len() < EARLIER_MAX {
                        earlier.push(row_for(
                            conn, vault, other, &next, &by_key, &places, true, now,
                        )?);
                    }
                }
            }
            // Only the newest DONE_MAX finished rows are built (Q-C5: no
            // usage or activity read for a row that isn't shown); the chip
            // counts them all.
            let done_count = finished_here.len() as u32;
            finished_here.sort_by(|a, b| b.0.cmp(&a.0));
            finished_here.truncate(DONE_MAX);
            let mut done = Vec::with_capacity(finished_here.len());
            for (_, child) in &finished_here {
                let row = row_for(conn, vault, child, &next, &by_key, &places, false, now)?;
                if is_active(&row.status) {
                    active.push(row);
                } else {
                    done.push(row);
                }
            }
            Ok((
                active,
                done,
                older_rows,
                older_total,
                earlier,
                done_count,
                seen,
            ))
        })?;
    // Starts still waiting for their first turn, under any conversation
    // the walk visited (a finished row past Done's cap, an earlier chat),
    // not only the listed rows.
    let known_parents: HashSet<String> = parents
        .iter()
        .cloned()
        .chain(visited)
        .chain(
            active
                .iter()
                .chain(done.iter())
                .flat_map(|r| r.conversations.clone()),
        )
        .collect();
    for row in live.iter().filter(|r| {
        crate::run_pump::subagent_parent_of(r).is_some_and(|p| known_parents.contains(p))
    }) {
        let listed = row.source_id.as_deref().is_some_and(|key| {
            active
                .iter()
                .chain(done.iter())
                .any(|r| r.conversations.iter().any(|c| c == key))
        });
        if !listed {
            if let Some(start) = queued_start(vault, row, &places, now) {
                active.push(start);
            }
        }
    }
    let rank = |r: &SubagentRow| {
        (
            u8::from(r.status != "running"),
            r.queue_place.unwrap_or(u32::MAX),
        )
    };
    active.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.started_at.cmp(&b.started_at)));
    let newest = |r: &SubagentRow| r.ended_at.clone().unwrap_or_else(|| r.started_at.clone());
    done.sort_by_key(|r| std::cmp::Reverse(newest(r)));
    done_older.sort_by_key(|r| std::cmp::Reverse(newest(r)));
    earlier.sort_by_key(|r| std::cmp::Reverse(newest(r)));
    // X1: each row's tracker issue, from one links read and one ledger read.
    let keys: Vec<String> = active
        .iter()
        .chain(done.iter())
        .chain(done_older.iter())
        .chain(earlier.iter())
        .map(|r| r.key.clone())
        .collect();
    let refs = crate::tracker::work::issue_refs(vault, &keys);
    for row in active
        .iter_mut()
        .chain(done.iter_mut())
        .chain(done_older.iter_mut())
        .chain(earlier.iter_mut())
    {
        if let Some(issue) = refs.get(&row.key) {
            row.tracker_issue = Some(issue.key.clone());
            row.issue = Some(issue.clone());
        }
    }
    // Who started each row, one lookup per starting conversation.
    let mut starters: HashMap<String, &'static str> = HashMap::new();
    for row in active
        .iter_mut()
        .chain(done.iter_mut())
        .chain(done_older.iter_mut())
        .chain(earlier.iter_mut())
    {
        let kind = match row.parent_key.as_deref() {
            None => "you",
            Some(p) => *starters
                .entry(p.to_string())
                .or_insert_with(|| starter_kind(vault, p)),
        };
        row.started_by = Some(kind.to_string());
    }
    let swarms = crate::main_agent::load_swarms(vault);
    for row in active
        .iter_mut()
        .chain(done.iter_mut())
        .chain(done_older.iter_mut())
    {
        activity::join_swarm(row, &swarms);
    }
    let running = active.iter().filter(|r| r.status == "running").count() as u32;
    let queued = active.iter().filter(|r| r.status == "queued").count() as u32;
    Ok(SubagentsView {
        parent: parent.to_string(),
        parents,
        active,
        done,
        done_older,
        older_total,
        earlier,
        running,
        queued,
        done_count,
        at: now.to_rfc3339(),
    })
}

/// One conversation as a Subagents row (X1: an issue's runs, a linked run's
/// totals); none when no such conversation exists yet.
pub(crate) fn row_of(
    vault: &Path,
    key: &str,
    now: DateTime<Utc>,
) -> Result<Option<SubagentRow>, String> {
    let next: HashMap<String, String> = crate::app_sessions::handover_pairs(vault)
        .into_iter()
        .map(|(succ, from)| (from, succ))
        .collect();
    let live = crate::run_queue::live_rows(vault).unwrap_or_default();
    let by_key = live_by_key(&live);
    let places = queue_places(vault, &live, now);
    let session = session_store::with_read(vault, |conn| {
        sessions_where(conn, "key = ?", &[key.to_string()], 1)
    })?;
    let Some(session) = session.into_iter().next() else {
        return Ok(live
            .iter()
            .find(|r| r.source == RunSource::Chat && r.source_id.as_deref() == Some(key))
            .and_then(|row| queued_start(vault, row, &places, now)));
    };
    session_store::with_read(vault, |conn| {
        row_for(conn, vault, &session, &next, &by_key, &places, false, now).map(Some)
    })
}

/// `session_subagents`: conversation `parent`'s subagents (the Subagents tab).
#[tauri::command]
pub async fn session_subagents(
    state: tauri::State<'_, crate::commands::AppState>,
    parent: String,
    older: Option<u32>,
) -> Result<SubagentsView, String> {
    let vault = state.config.vault_path.clone();
    let older = older.unwrap_or(0) as usize;
    crate::vault::spawn_blocking_logged(move || subagents_paged(&vault, &parent, Utc::now(), older))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::{claim, enqueue, mark_running, NewRow};
    use chrono::Duration;

    fn usage(
        v: &Path,
        run: &str,
        input: i64,
        cached: i64,
        output: i64,
        cost: Option<f64>,
        account: &str,
    ) {
        session_store::with_write(v, "test usage", |tx| {
            tx.execute(
                "INSERT INTO usage_ledger(run_key, at, started_at, surface, scope, run_id, agent, model, \
                 billing, outcome, spawned, basis, input_tokens, cached_input_tokens, output_tokens, \
                 cost_usd, cost_status, account_ref) \
                 VALUES(?1, ?2, ?2, 'chat', 'x', ?3, 'claude', 'claude-sonnet-5-5', 'subscription', \
                 'success', 1, 'per_run', ?4, ?5, ?6, ?7, 'known', ?8)",
                rusqlite::params![
                    format!("key-{run}"),
                    "2026-11-10T12:00:00+00:00",
                    run,
                    input,
                    cached,
                    output,
                    cost,
                    account
                ],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
    }

    fn turn(v: &Path, key: &str, run: &str, start: &str, end: Option<&str>, status: &str) {
        let ord =
            session_store::start_turn(v, key, "claude", "/vault/p", run, "Do the part").unwrap();
        session_store::with_write(v, "test turn", |tx| {
            tx.execute(
                "UPDATE turns SET started_at = ?3, ended_at = ?4, status = ?5 WHERE session_key = ?1 AND ord = ?2",
                rusqlite::params![key, ord, start, end, (!status.is_empty()).then_some(status)],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
    }

    /// 2026-11-16, Working, not Queued: a subagent start nothing truly holds
    /// back (a workspace folder with room) reads running, with no place or
    /// reason; the same start at its chat's subagent cap reads queued.
    #[test]
    fn a_subagent_nothing_holds_back_reads_working_not_queued() {
        use crate::chat_limits::{set_test_limits, ChatLimits};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let folder = v.join("workspaces").join("w1");
        std::fs::create_dir_all(&folder).unwrap();
        let lead = crate::project_agents::key("proj-work").unwrap();
        session_store::start_turn(v, &lead, "claude", "/vault/p", "run-lead", "Plan").unwrap();
        let now = Utc::now();
        let past = now - Duration::minutes(5);
        let key = "s-20261116-work0001";
        session_store::set_parent(v, key, &lead, session_store::FORK_SUBAGENT).unwrap();
        let payload =
            serde_json::json!({"agent": "claude", "parent": lead, "cwd": folder.to_string_lossy()});
        enqueue(
            v,
            NewRow::new(RunSource::Chat, Some(key), "chat", "t").with_payload(payload),
            past,
        )
        .unwrap();
        set_test_limits(ChatLimits::default());
        let view = subagents_at(v, &lead, now).unwrap();
        let row = view.active.iter().find(|r| r.key == key).unwrap();
        assert_eq!(row.status, "running", "{row:?}");
        assert_eq!(row.waiting_for, None);
        assert_eq!(row.queue_place, None);
        assert_eq!(view.queued, 0);
    }

    /// 2026-11-12: a start held by its chat's subagent cap says so on its
    /// row, in the words the queue badge and `start_session` use, naming the
    /// number and the setting; a running one carries no reason.
    #[test]
    fn a_start_held_by_the_subagent_cap_carries_the_plain_reason() {
        use crate::chat_limits::{set_test_limits, ChatLimits};
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-cap").unwrap();
        session_store::start_turn(v, &lead, "claude", "/vault/p", "run-lead", "Plan").unwrap();
        let now = Utc::now();
        let past = now - Duration::minutes(5);
        let running_key = "s-20261112-capa0001";
        turn(v, running_key, "run-cap1", &past.to_rfc3339(), None, "");
        session_store::set_parent(v, running_key, &lead, session_store::FORK_SUBAGENT).unwrap();
        let payload = serde_json::json!({"agent": "claude", "parent": lead});
        let row = NewRow::new(RunSource::Chat, Some(running_key), "chat", "t")
            .with_payload(payload.clone());
        let id = enqueue(v, row, past).unwrap().id();
        claim(v, id, past).unwrap();
        mark_running(v, id, "run-cap1", past).unwrap();
        let waiting = NewRow::new(RunSource::Chat, Some("s-20261112-capb0001"), "chat", "t")
            .with_payload(payload);
        enqueue(v, waiting, past).unwrap();

        set_test_limits(ChatLimits {
            subagents_per_chat: 1,
            ..ChatLimits::default()
        });
        let view = subagents_at(v, &lead, now).unwrap();
        let running = view.active.iter().find(|r| r.key == running_key).unwrap();
        assert_eq!(running.waiting_for, None);
        let queued = view
            .active
            .iter()
            .find(|r| r.key == "s-20261112-capb0001")
            .unwrap();
        assert_eq!(queued.status, "queued");
        assert_eq!(
            queued.waiting_for.as_deref(),
            Some(
                "a free subagent slot: this chat is at its limit of 1 subagents at once (Settings → Chat → Chats at once)"
            )
        );
        // Room under the cap: whatever else holds it, it isn't the cap (here
        // the vault root's one run).
        set_test_limits(ChatLimits::default());
        let view = subagents_at(v, &lead, now).unwrap();
        let queued = view
            .active
            .iter()
            .find(|r| r.key == "s-20261112-capb0001")
            .unwrap();
        assert!(
            !queued
                .waiting_for
                .as_deref()
                .unwrap_or_default()
                .contains("subagent slot"),
            "{:?}",
            queued.waiting_for
        );
    }

    #[test]
    fn descendants_are_groupable_and_swarm_titles_preserve_renames() {
        let dir = tempfile::Builder::new()
            .prefix("subagent-view-")
            .tempdir()
            .unwrap();
        let v = dir.path();
        let parent = "s-20261112-parent01";
        let child = "s-20261112-child001";
        let grandchild = "s-20261112-child002";
        for key in [parent, child, grandchild] {
            turn(
                v,
                key,
                key,
                "2026-11-12T12:00:00Z",
                Some("2026-11-12T12:01:00Z"),
                "success",
            );
        }
        session_store::set_parent(v, child, parent, session_store::FORK_SUBAGENT).unwrap();
        session_store::set_parent(v, grandchild, child, session_store::FORK_SUBAGENT).unwrap();
        let view = subagents_at(v, parent, Utc::now()).unwrap();
        assert_eq!(view.done.len(), 2);
        let mut row = view
            .done
            .iter()
            .find(|r| r.key == grandchild)
            .unwrap()
            .clone();
        assert_eq!(row.parent_key.as_deref(), Some(child));
        let swarm: crate::main_agent::Swarm = serde_json::from_value(serde_json::json!({
            "id":"swarm-one", "goal":"Improve the app", "createdAt":"now", "parentKey":parent,
            "runId":"run", "depth":0, "mode":"plan", "tasks":[{
                "title":"Review keyboard navigation", "workspace":"quick-chats", "prompt":"Review keys", "session":grandchild
            }]
        })).unwrap();
        let prompt = crate::main_agent::swarm_part_prompt(&swarm, 0);
        assert_eq!(
            crate::chat_title::short_title(&prompt),
            "Review keyboard navigation"
        );
        assert!(prompt.contains("This chat is part 1 of 1"));
        row.title = Some(crate::chat_title::short_title(
            &crate::main_agent::swarm_part_header(&swarm, 0),
        ));
        activity::join_swarm(&mut row, std::slice::from_ref(&swarm));
        assert_eq!(row.title.as_deref(), Some("Review keyboard navigation"));
        assert_eq!(row.group.as_ref().unwrap().id, "swarm-one");
        row.title = Some("My chosen name".into());
        activity::join_swarm(&mut row, &[swarm]);
        assert_eq!(row.title.as_deref(), Some("My chosen name"));
    }

    /// Q-C1 (2026-11-13): a running grandchild under a finished part that
    /// falls outside Done's DONE_MAX newest is still found, Active and
    /// counted; Done keeps only the newest DONE_MAX and the chip counts all.
    #[test]
    fn a_running_grandchild_of_a_finished_part_past_done_max_is_active() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-20261113-lead0001";
        turn(v, parent, "run-lead", "2026-11-13T00:00:00Z", None, "");
        let base = stamp("2026-11-13T01:00:00Z").unwrap();
        let at = |m: i64| (base + Duration::minutes(m)).to_rfc3339();
        for i in 0..=DONE_MAX as i64 {
            let key = format!("s-20261113-part{i:04}");
            turn(
                v,
                &key,
                &format!("run-p{i}"),
                &at(i),
                Some(&at(i + 1)),
                "success",
            );
            session_store::set_parent(v, &key, parent, session_store::FORK_SUBAGENT).unwrap();
        }
        // The oldest part ended first, so it is the one past DONE_MAX.
        let grandchild = "s-20261113-grand001";
        turn(v, grandchild, "run-g", &at(1), None, "");
        session_store::set_parent(
            v,
            grandchild,
            "s-20261113-part0000",
            session_store::FORK_SUBAGENT,
        )
        .unwrap();
        // A queued start (no session row yet) under that same part is found
        // through the walk's visited keys, not only the listed rows.
        let now = Utc::now();
        let start = "s-20261113-qstart01";
        let row = NewRow::new(RunSource::Chat, Some(start), "chat", "t")
            .with_payload(serde_json::json!({"agent": "claude", "parent": "s-20261113-part0000"}));
        enqueue(v, row, now).unwrap();
        let view = subagents_at(v, parent, now).unwrap();
        let mut active: Vec<&str> = view.active.iter().map(|r| r.key.as_str()).collect();
        active.sort();
        assert_eq!(active, vec![grandchild, start]);
        // The start nothing holds back reads working, not queued
        // (2026-11-16), so it counts with the running grandchild.
        assert_eq!(view.running, 2);
        assert_eq!(view.queued, 0);
        assert_eq!(view.done.len(), DONE_MAX);
        assert!(view.done.iter().all(|r| r.key != "s-20261113-part0000"));
        assert_eq!(view.done[0].key, format!("s-20261113-part{DONE_MAX:04}"));
        assert_eq!(view.done_count as usize, DONE_MAX + 1);
    }

    /// Review of #1375 (2026-11-13): a listed Earlier chat's running
    /// subagent is Active, not left out because Earlier chats were read
    /// after the descendant walk.
    #[test]
    fn an_earlier_chats_running_subagent_is_active() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-earlier").unwrap();
        let other = "s-20261113-other001";
        let kid = "s-20261113-otherkid";
        turn(
            v,
            &lead,
            "run-lead",
            "2026-11-13T00:00:00Z",
            Some("2026-11-13T00:01:00Z"),
            "success",
        );
        turn(
            v,
            other,
            "run-other",
            "2026-11-13T00:02:00Z",
            Some("2026-11-13T00:03:00Z"),
            "success",
        );
        turn(v, kid, "run-kid", "2026-11-13T00:04:00Z", None, "");
        session_store::set_parent(v, kid, other, session_store::FORK_SUBAGENT).unwrap();
        // The running subagent's own finished child is not the lead's Done.
        let grandkid = "s-20261113-othergk1";
        turn(
            v,
            grandkid,
            "run-gk",
            "2026-11-13T00:05:00Z",
            Some("2026-11-13T00:06:00Z"),
            "success",
        );
        session_store::set_parent(v, grandkid, kid, session_store::FORK_SUBAGENT).unwrap();
        let view = subagents_at(v, &lead, Utc::now()).unwrap();
        assert!(
            view.earlier.iter().any(|r| r.key == other),
            "listed earlier"
        );
        let active: Vec<&str> = view.active.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(active, vec![kid]);
        assert!(
            view.done.is_empty(),
            "an earlier chat's subagents aren't Done"
        );
        assert_eq!(view.done_count, 0);
    }

    /// 11-14 card 2: a lead's Subagents tab lists every chat working in its
    /// project, whoever started it, and says who did.
    #[test]
    fn a_leads_tab_lists_every_chat_in_its_project_with_its_starter() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-all").unwrap();
        let (mine, other, okid, outside, xkid) = (
            "s-20261114-mine0001",
            "s-20261114-other001",
            "s-20261114-okid0001",
            "s-20261114-outside1",
            "s-20261114-xkid0001",
        );
        let (start, end) = ("2026-11-14T10:00:00Z", Some("2026-11-14T10:01:00Z"));
        turn(v, &lead, "run-lead", start, end, "success");
        for (key, run) in [(mine, "run-m"), (other, "run-o"), (okid, "run-ok")] {
            turn(v, key, run, start, end, "success");
        }
        session_store::set_parent(v, mine, &lead, session_store::FORK_SUBAGENT).unwrap();
        session_store::set_parent(v, okid, other, session_store::FORK_SUBAGENT).unwrap();
        // A conversation outside the project starts one in it.
        session_store::start_turn(v, outside, "claude", "/vault/q", "run-x", "Plan").unwrap();
        turn(v, xkid, "run-xk", start, end, "success");
        session_store::set_parent(v, xkid, outside, session_store::FORK_SUBAGENT).unwrap();
        let view = subagents_at(v, &lead, Utc::now()).unwrap();
        let by = |rows: &[SubagentRow], key: &str| {
            rows.iter()
                .find(|r| r.key == key)
                .and_then(|r| r.started_by.clone())
        };
        assert_eq!(by(&view.done, mine).as_deref(), Some("lead"));
        assert_eq!(by(&view.earlier, other).as_deref(), Some("you"));
        assert_eq!(by(&view.earlier, okid).as_deref(), Some("chat"));
        assert_eq!(by(&view.earlier, xkid).as_deref(), Some("chat"));
        assert!(!view
            .earlier
            .iter()
            .any(|r| r.key == outside || r.key == mine));
        let all = view
            .active
            .iter()
            .chain(&view.done)
            .chain(&view.earlier)
            .filter(|r| r.key == mine)
            .count();
        assert_eq!(all, 1, "listed once");
    }

    /// Q-C4 (2026-11-13): a re-queued subagent whose new run has no turn row
    /// yet shows no current step or plan from its previous turn.
    #[test]
    fn a_requeued_subagent_without_its_new_turn_shows_no_old_activity() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let parent = "s-20261113-lead0002";
        let child = "s-20261113-again001";
        turn(v, parent, "run-lead2", "2026-11-13T00:00:00Z", None, "");
        turn(
            v,
            child,
            "run-old",
            "2026-11-13T01:00:00Z",
            Some("2026-11-13T01:05:00Z"),
            "success",
        );
        session_store::set_parent(v, child, parent, session_store::FORK_SUBAGENT).unwrap();
        session_store::with_write(v, "test plan", |tx| {
            tx.execute(
                "INSERT INTO events(session_key, turn_ord, at, kind, payload) \
                 VALUES(?1, 1, '2026-11-13T01:01:00Z', 'plan', ?2)",
                rusqlite::params![child, r#"{"completed":1,"total":2}"#],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
        let now = Utc::now();
        let before = subagents_at(v, parent, now).unwrap();
        let progress = |row: &SubagentRow| row.activity.as_ref().and_then(|a| a.progress.clone());
        assert!(
            progress(&before.done[0]).is_some(),
            "the finished turn shows its plan"
        );
        let row = NewRow::new(RunSource::Chat, Some(child), "chat", "t")
            .with_payload(serde_json::json!({"agent": "claude", "parent": parent}));
        let id = enqueue(v, row, now).unwrap().id();
        claim(v, id, now).unwrap();
        mark_running(v, id, "run-new", now).unwrap();
        let view = subagents_at(v, parent, now).unwrap();
        let row = view.active.iter().find(|r| r.key == child).unwrap();
        assert_eq!(row.status, "running");
        let activity = row.activity.as_ref().unwrap();
        assert!(activity.current.is_none());
        assert!(activity.progress.is_none());
        assert!(activity.recent.is_empty());
    }

    #[test]
    fn working_time_sums_turns_and_a_running_turn_counts_up_to_now() {
        let now = stamp("2026-11-10T12:10:00Z").unwrap();
        let at = |s: &str| stamp(s);
        let (working, wall, ended) = spans(
            &[
                (at("2026-11-10T12:00:00Z"), at("2026-11-10T12:01:00Z")),
                (at("2026-11-10T12:05:00Z"), at("2026-11-10T12:05:30Z")),
            ],
            now,
        );
        assert_eq!((working, wall), (90_000, 330_000));
        assert_eq!(ended, at("2026-11-10T12:05:30Z"));
        let (working, wall, ended) = spans(
            &[
                (at("2026-11-10T12:00:00Z"), at("2026-11-10T12:01:00Z")),
                (at("2026-11-10T12:08:00Z"), None),
            ],
            now,
        );
        assert_eq!((working, wall, ended), (180_000, 600_000, None));
        assert_eq!(spans(&[], now), (0, 0, None));
    }

    /// The tab's read on a fixture: tokens and cost summed from the ledger,
    /// a handover chain counted once (its successor's turns and tokens with
    /// it, its status the successor's), a running one live, a start still
    /// waiting for its first turn listed as queued, and a lead's other chats
    /// under Earlier chats.
    #[test]
    fn a_parents_subagents_sum_the_ledger_and_count_a_handover_chain_once() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lead = crate::project_agents::key("proj-l2").unwrap();
        session_store::start_turn(v, &lead, "claude", "/vault/p", "run-lead", "Plan").unwrap();
        let (a, b, b2, c, other) = (
            "s-20261110-suba0001",
            "s-20261110-subb0001",
            "s-20261110-subb0002",
            "s-20261110-subc0001",
            "s-20261109-oldchat1",
        );
        turn(
            v,
            a,
            "run-a1",
            "2026-11-10T12:00:00+00:00",
            Some("2026-11-10T12:02:00+00:00"),
            "success",
        );
        turn(
            v,
            a,
            "run-a2",
            "2026-11-10T12:03:00+00:00",
            Some("2026-11-10T12:04:00+00:00"),
            "success",
        );
        usage(v, "run-a1", 1000, 200, 300, Some(0.10), "acme");
        usage(v, "run-a2", 500, 0, 100, Some(0.05), "acme");
        turn(
            v,
            b,
            "run-b1",
            "2026-11-10T12:00:00+00:00",
            Some("2026-11-10T12:01:00+00:00"),
            "success",
        );
        usage(v, "run-b1", 100, 0, 10, None, "personal");
        crate::app_sessions::record_handover(v, b, b2, 1).unwrap();
        turn(
            v,
            b2,
            "run-b2",
            "2026-11-10T12:02:00+00:00",
            Some("2026-11-10T12:03:00+00:00"),
            "error",
        );
        usage(v, "run-b2", 50, 0, 5, None, "personal");
        let now = Utc::now();
        let running_start = (now - Duration::minutes(4)).to_rfc3339();
        turn(v, c, "run-c1", &running_start, None, "");
        turn(
            v,
            other,
            "run-o1",
            "2026-11-09T09:00:00+00:00",
            Some("2026-11-09T09:01:00+00:00"),
            "success",
        );
        for key in [a, b, c] {
            session_store::set_parent(v, key, &lead, session_store::FORK_SUBAGENT).unwrap();
        }
        // C's turn is running; a fourth start waits for its first turn.
        let past = now - Duration::minutes(5);
        let row = NewRow::new(RunSource::Chat, Some(c), "chat", "t")
            .with_payload(serde_json::json!({"agent": "claude", "effort": "high", "parent": lead}));
        let id = enqueue(v, row, past).unwrap().id();
        claim(v, id, past).unwrap();
        mark_running(v, id, "run-c1", past).unwrap();
        let mut waiting = NewRow::new(RunSource::Chat, Some("s-20261110-subd0001"), "chat", "t")
            .with_payload(serde_json::json!({"agent": "codex", "model": "gpt-6", "parent": lead}));
        waiting.detail = Some("Review the PR".into());
        let waiting_id = enqueue(v, waiting, past).unwrap().id();

        let view = subagents_at(v, &lead, now).unwrap();
        assert_eq!(view.parents, vec![lead.clone()]);
        assert_eq!((view.running, view.queued, view.done_count), (1, 1, 2));
        let running = &view.active[0];
        assert_eq!(
            (running.key.as_str(), running.status.as_str()),
            (c, "running")
        );
        assert_eq!(running.effort.as_deref(), Some("high"));
        assert!(running.working_ms >= 239_000, "{}", running.working_ms);
        assert!(running.ended_at.is_none());
        let queued = &view.active[1];
        assert_eq!(queued.key, "s-20261110-subd0001");
        assert_eq!(
            (queued.status.as_str(), queued.queue_place),
            ("queued", Some(1))
        );
        assert_eq!(queued.title.as_deref(), Some("Review the PR"));
        assert_eq!(queued.agent, "codex");
        // Stop goes by the live row, never the key (overnight 11-13).
        assert_eq!(
            (running.queue_id, queued.queue_id),
            (Some(id), Some(waiting_id))
        );
        let done_a = view.done.iter().find(|r| r.key == a).unwrap();
        assert_eq!(
            (
                done_a.input_tokens,
                done_a.cached_input_tokens,
                done_a.output_tokens
            ),
            (1500, 200, 400)
        );
        assert_eq!(done_a.tokens, 1900);
        assert!((done_a.cost_usd.unwrap() - 0.15).abs() < 1e-9);
        assert_eq!(done_a.working_ms, 180_000);
        assert_eq!(done_a.wall_ms, 240_000);
        assert_eq!(done_a.account.as_deref(), Some("acme"));
        assert_eq!(done_a.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(done_a.status, "done");
        assert_eq!(done_a.queue_id, None);
        // The handover chain is one row: both conversations' tokens and
        // time, the successor's status, no cost when none is known.
        let chain = view.done.iter().find(|r| r.key == b).unwrap();
        assert_eq!(chain.conversations, vec![b.to_string(), b2.to_string()]);
        assert_eq!(chain.current, b2);
        assert_eq!(chain.tokens, 165);
        assert_eq!(chain.working_ms, 120_000);
        assert_eq!(chain.status, "failed");
        assert_eq!(chain.cost_usd, None);
        assert!(!view.done.iter().any(|r| r.key == b2), "counted once");
        // Earlier chats: the project's other chat, not the lead or a subagent.
        let earlier: Vec<&str> = view.earlier.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(earlier, vec![other]);
        assert!(view.earlier[0].earlier);
        // Not a key: refused.
        assert!(subagents_at(v, "../etc", now).is_err());
    }

    fn active_at(v: &Path, key: &str, at: &str) {
        session_store::with_write(v, "test activity", |tx| {
            tx.execute(
                "UPDATE sessions SET last_activity_at = ?2 WHERE key = ?1",
                rusqlite::params![key, at],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })
        .unwrap();
    }

    /// A long chain shows only the current conversation's finished subagents
    /// under Done (plus a handover predecessor active in the last day); Show
    /// older pages in the rest newest first; Active still spans the chain;
    /// the chip's count never includes the older ones.
    #[test]
    fn done_is_the_current_conversation_and_show_older_pages_in_the_rest_of_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let ago = |h: i64| (now - Duration::hours(h)).to_rfc3339();
        let (l1, l2, l3) = (
            crate::project_agents::key("proj-scope").unwrap(),
            "s-20261001-scopel02".to_string(),
            "s-20261110-scopel03".to_string(),
        );
        for (key, run) in [(&l1, "run-l1"), (&l2, "run-l2"), (&l3, "run-l3")] {
            turn(
                v,
                key,
                run,
                "2026-11-10T12:00:00+00:00",
                Some("2026-11-10T12:01:00+00:00"),
                "success",
            );
        }
        crate::app_sessions::record_handover(v, &l1, &l2, 1).unwrap();
        crate::app_sessions::record_handover(v, &l2, &l3, 2).unwrap();
        active_at(v, &l1, &ago(24 * 40));
        active_at(v, &l2, &ago(24 * 2));
        active_at(v, &l3, &ago(0));
        // (child, parent, hours since it was last active)
        let kids = [
            ("s-20261110-kidnew01", &l3, 1),
            ("s-20261109-kidmid01", &l2, 30),
            ("s-20261001-kidold01", &l1, 24 * 30),
            ("s-20261002-kidold02", &l1, 24 * 35),
        ];
        for (i, (key, parent, hours)) in kids.iter().enumerate() {
            turn(
                v,
                key,
                &format!("run-k{i}"),
                "2026-11-09T09:00:00+00:00",
                Some("2026-11-09T09:01:00+00:00"),
                "success",
            );
            session_store::set_parent(v, key, parent, session_store::FORK_SUBAGENT).unwrap();
            active_at(v, key, &ago(*hours));
        }
        // An old one still running (a live queue row) is Active whatever its age.
        let live = "s-20261001-kidlive1";
        turn(v, live, "run-live", &ago(24 * 39), None, "");
        session_store::set_parent(v, live, &l1, session_store::FORK_SUBAGENT).unwrap();
        active_at(v, live, &ago(24 * 39));
        let past = now - Duration::hours(24 * 39);
        let row = NewRow::new(RunSource::Chat, Some(live), "chat", "t")
            .with_payload(serde_json::json!({"agent": "claude", "parent": l1}));
        let id = enqueue(v, row, past).unwrap().id();
        claim(v, id, past).unwrap();
        mark_running(v, id, "run-live", past).unwrap();

        let view = subagents_at(v, &l3, now).unwrap();
        assert_eq!(view.parents[0], l3);
        let keys = |rows: &[SubagentRow]| rows.iter().map(|r| r.key.clone()).collect::<Vec<_>>();
        assert_eq!(
            keys(&view.active),
            vec![live.to_string()],
            "Active spans the chain"
        );
        assert_eq!(keys(&view.done), vec!["s-20261110-kidnew01".to_string()]);
        assert_eq!((view.done_count, view.older_total), (1, 3));
        assert!(view.done_older.is_empty());
        // The same read from the chain's first key lands on the same view.
        let from_old = subagents_at(v, &l1, now).unwrap();
        assert_eq!(keys(&from_old.done), keys(&view.done));
        // Show older: newest first, in batches, until none are left.
        let one = subagents_paged(v, &l3, now, 1).unwrap();
        assert_eq!(
            keys(&one.done_older),
            vec!["s-20261109-kidmid01".to_string()]
        );
        assert_eq!(one.older_total, 3);
        let all = subagents_paged(v, &l3, now, 25).unwrap();
        assert_eq!(
            keys(&all.done_older),
            vec![
                "s-20261109-kidmid01".to_string(),
                "s-20261001-kidold01".to_string(),
                "s-20261002-kidold02".to_string()
            ]
        );
        assert_eq!(all.older_total, 3);
        assert_eq!(keys(&all.active), vec![live.to_string()]);
        assert_eq!(all.done_count, 1, "the chip never counts the older ones");
        // A predecessor active in the last day still counts as current, so a
        // mid-task handover doesn't empty the tab.
        active_at(v, &l2, &ago(3));
        let recent = subagents_at(v, &l3, now).unwrap();
        assert_eq!(
            keys(&recent.done)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>(),
            ["s-20261110-kidnew01", "s-20261109-kidmid01"]
                .map(String::from)
                .into_iter()
                .collect()
        );
        assert_eq!((recent.done_count, recent.older_total), (2, 2));
    }
}
