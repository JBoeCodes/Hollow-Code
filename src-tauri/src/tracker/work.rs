//! Tracking delegated agent work in the tracker (COO rebuild X1, decided in
//! the X0 note `outputs/coo-rebuild-2026-11-10/linear-tracking-audit.md`).
//!
//! A subagent started in a tracked workspace is tied to one issue of the
//! scoped team, found in this order: the start's named `issue`, its parent's
//! link (a helper stops here), a task in the packet's sources that carries
//! `tracker_id`, a title match over the project's open issues, else a new
//! issue in the workspace's tracker project. Issues are read from the local
//! mirror (the sync ledger's snapshots), never the network.
//!
//! The host writes everything here, never an agent: the link at start
//! (`work_links`), and, when Settings' "Track agent work" is on, the facts
//! the tracker gets (`work_events`: create, started, progress, outcome).
//! The sync pass turns those facts into the ledger's work outbox
//! ([`super::work_outbox`]), which sends them crash-safe. While the switch
//! is off links still record and nothing is queued, so turning it on never
//! back-posts old runs.
//!
//! Provider-neutral: nothing here names a provider.

use super::{StateKind, TrackerIssue};
use crate::config::Config;
use crate::session_store;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Hollow tracks agent work in this team only (team ENG).
pub(crate) const AI_TEAM_ID: &str = "11111111-1111-4111-8111-111111111111";
pub(crate) const AI_TEAM_KEY: &str = "ENG";
/// A title match needs a token-set Jaccard of at least this.
pub(crate) const MATCH_THRESHOLD: f64 = 0.6;
/// Every comment body is at most this many bytes (the provider's limit).
pub(crate) const COMMENT_MAX_BYTES: usize = 2000;
/// At most one progress comment per issue in this many minutes.
pub(crate) const PROGRESS_MINUTES: i64 = 15;
/// A run's progress notes within this many seconds fold into one fact.
pub(crate) const PROGRESS_FOLD_SECONDS: i64 = 60;
/// Taken facts are kept this many days.
const EVENT_DAYS: i64 = 30;

/// Whether this process's running config tracks agent work, per vault
/// (registered at setup; the switch applies after a restart). Unregistered
/// means off, so nothing is queued.
static TRACKING: once_cell::sync::Lazy<Mutex<HashMap<PathBuf, bool>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Setup: record the running config's switch for its vault.
pub(crate) fn register_running(config: &Config) {
    set_tracking(
        &config.vault_path,
        config.tracker.track_agent_work && ai_scope(config),
    );
    super::chat_work::register_running(config);
}

pub(crate) fn set_tracking(vault: &Path, on: bool) {
    TRACKING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(vault.to_path_buf(), on);
}

/// Tracking agent work is on for `vault` in this process.
pub(crate) fn tracking(vault: &Path) -> bool {
    TRACKING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(vault)
        .copied()
        .unwrap_or(false)
}

/// How a run got its issue (`work_links.linked_by`).
pub(crate) mod linked_by {
    pub const PACKET: &str = "packet";
    pub const INHERITED: &str = "inherited";
    pub const TASK: &str = "task";
    pub const MATCHED: &str = "matched";
    pub const CREATED: &str = "created";
    /// A person's chat that did real work (T3a, [`super::chat_work`]).
    pub const CHAT: &str = "chat";
    /// A Waiting on you item's copy (T3b, [`super::waiting_work`]): keyed
    /// `waiting:<item id>`, never a run.
    pub const WAITING: &str = "waiting";
    /// A commitment's copy (T3e, [`super::commitment_work`]): keyed
    /// `commitment:<id>`, never a run.
    pub const COMMITMENT: &str = "commitment";

    /// A copy Hollow files and closes on its own (T3b, T3e), not a run.
    pub fn is_copy(linked_by: &str) -> bool {
        linked_by == WAITING || linked_by == COMMITMENT
    }
}

// ── The audit record (X2) ──────────────────────────────────────────────────

/// A run starting or ending, for the audit record ([`crate::audit`]).
pub(crate) enum RunEvent<'a> {
    Started {
        session_key: &'a str,
        link: Option<&'a WorkLink>,
        facts: &'a StartFacts,
    },
    Ended {
        session_key: &'a str,
        status: &'a str,
    },
}

/// Append the run's audit rows: `run_started` (and `issue_linked` when its
/// issue is known) at every subagent start, `run_ended` at every subagent
/// or linked turn end, with its totals so far (usage by `scope` = its
/// session key, working time and turns from `turns`). Never fails the
/// caller. Returns the `run_ended` row it stored.
pub(crate) fn record_run(vault: &Path, event: &RunEvent<'_>) -> Option<crate::audit::AuditEntry> {
    use crate::audit::{self, kind, AuditEntry};
    match event {
        RunEvent::Started {
            session_key,
            link,
            facts,
        } => {
            let parent_key = facts.parent_key.clone().filter(|p| !p.trim().is_empty());
            let root_key = parent_key.as_deref().map(|p| audit::root_of(vault, p));
            let base = AuditEntry {
                session_key: Some((*session_key).to_string()),
                parent_key,
                root_key,
                asked_by: Some(facts.asked_by.clone()),
                agent: facts.agent.clone(),
                model: facts.model.clone(),
                effort: facts.effort.clone(),
                workspace_id: facts.workspace_id.clone(),
                issue_id: link.and_then(|l| l.issue_id.clone()),
                issue_key: link.and_then(|l| l.issue_key.clone()),
                ..AuditEntry::default()
            };
            audit::record(
                vault,
                AuditEntry {
                    kind: kind::RUN_STARTED.into(),
                    outcome: Some("ok".into()),
                    detail: link.map(|l| format!("linked by {}", l.linked_by)),
                    ..base.clone()
                },
            );
            if let Some(link) = link.filter(|l| l.issue_id.is_some()) {
                audit::record(vault, linked_entry(base, link));
            }
            None
        }
        RunEvent::Ended {
            session_key,
            status,
        } => {
            let started = audit::latest(vault, session_key, kind::RUN_STARTED);
            // Who asked for this turn: the oldest start (a subagent start
            // or a sent child task) no earlier turn end took, else the
            // person (a turn typed into the chat). Turns run in order.
            let taken: BTreeSet<String> = audit::rows_of(vault, session_key, kind::RUN_ENDED)
                .into_iter()
                .filter_map(|e| {
                    e.fields
                        .as_ref()
                        .and_then(|f| f.get("startRow"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect();
            let asking = audit::rows_of(vault, session_key, kind::RUN_STARTED)
                .into_iter()
                .find(|e| !taken.contains(&e.id));
            let (parent_key, root_key) = match &started {
                Some(row) => (row.parent_key.clone(), row.root_key.clone()),
                None => audit::lineage(vault, session_key),
            };
            let usage = audit::run_usage(vault, session_key, Utc::now());
            let link = link_for(vault, session_key);
            let agent = session_store::with_read(vault, |conn| {
                conn.query_row(
                    "SELECT agent FROM sessions WHERE key = ?1",
                    params![session_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())
            })
            .ok()
            .flatten();
            let outcome = match *status {
                "success" => "ok",
                "cancelled" => "stopped",
                _ => "failed",
            };
            audit::record(
                vault,
                AuditEntry {
                    kind: kind::RUN_ENDED.into(),
                    session_key: Some((*session_key).to_string()),
                    parent_key: parent_key.clone(),
                    asked_by: Some(
                        asking
                            .as_ref()
                            .and_then(|r| r.asked_by.clone())
                            .unwrap_or_else(|| "person".into()),
                    ),
                    root_key,
                    agent: agent.or_else(|| started.as_ref().and_then(|r| r.agent.clone())),
                    model: usage
                        .model
                        .clone()
                        .or_else(|| started.as_ref().and_then(|r| r.model.clone())),
                    effort: started.as_ref().and_then(|r| r.effort.clone()),
                    account: usage.account.clone(),
                    workspace_id: link
                        .as_ref()
                        .and_then(|l| l.workspace_id.clone())
                        .or_else(|| started.as_ref().and_then(|r| r.workspace_id.clone())),
                    issue_id: link.as_ref().and_then(|l| l.issue_id.clone()),
                    issue_key: link.as_ref().and_then(|l| l.issue_key.clone()),
                    fields: Some(json!({
                        "turns": usage.turns,
                        "costPartial": usage.cost_partial,
                        "startRow": asking.as_ref().map(|r| r.id.clone()),
                    })),
                    outcome: Some(outcome.into()),
                    detail: Some(format!("turn {status}")),
                    cost_usd: usage.cost_usd,
                    tokens_in: Some(usage.tokens_in),
                    tokens_out: Some(usage.tokens_out),
                    working_ms: Some(usage.working_ms),
                    ..AuditEntry::default()
                },
            )
        }
    }
}

/// The `issue_linked` row for `link` on top of a run's facts.
fn linked_entry(base: crate::audit::AuditEntry, link: &WorkLink) -> crate::audit::AuditEntry {
    crate::audit::AuditEntry {
        kind: crate::audit::kind::ISSUE_LINKED.into(),
        action: Some("link".into()),
        outcome: Some("ok".into()),
        detail: Some(format!("linked by {}", link.linked_by)),
        issue_id: link.issue_id.clone(),
        issue_key: link.issue_key.clone(),
        workspace_id: link.workspace_id.clone().or(base.workspace_id.clone()),
        ..base
    }
}

/// A child task sent to an existing conversation (`sessions.send`): its
/// `run_started` row, asked by the sender. Nothing goes to the tracker: the
/// conversation's own link (if any) already reports.
pub(crate) fn record_sent(
    vault: &Path,
    session_key: &str,
    asked_by: &str,
    (agent, model, effort): (Option<String>, Option<String>, Option<String>),
    workspace_id: Option<String>,
) {
    let (parent_key, root_key) = crate::audit::lineage(vault, session_key);
    let link = link_for(vault, session_key);
    crate::audit::record(
        vault,
        crate::audit::AuditEntry {
            kind: crate::audit::kind::RUN_STARTED.into(),
            session_key: Some(session_key.into()),
            parent_key,
            root_key,
            asked_by: Some(asked_by.into()),
            agent,
            model,
            effort,
            workspace_id: link
                .as_ref()
                .and_then(|l| l.workspace_id.clone())
                .or(workspace_id),
            issue_id: link.as_ref().and_then(|l| l.issue_id.clone()),
            issue_key: link.as_ref().and_then(|l| l.issue_key.clone()),
            outcome: Some("ok".into()),
            detail: Some("sent to the conversation".into()),
            ..crate::audit::AuditEntry::default()
        },
    );
}

/// One tracker write Hollow made or refused for agent work, for the audit.
#[derive(Default)]
pub(crate) struct TrackerWrite<'a> {
    pub session_key: &'a str,
    pub issue_id: &'a str,
    pub issue_key: Option<&'a str>,
    /// `create`, `update` (a status move), `comment` or `link`.
    pub action: &'static str,
    /// `ok`, `failed`, `skipped` or `unconfirmed` (it may have gone; it is
    /// never sent again, so the audit says `failed` with that reason).
    pub outcome: &'static str,
    pub before: Option<Value>,
    pub after: Option<Value>,
    /// The outbox entry's id or the create's key.
    pub request_key: Option<&'a str>,
    pub detail: Option<String>,
}

/// Append the write's audit row (`tracker_write`, or `tracker_write_failed`
/// for a refused, failed or unconfirmed one). Called after every agent-work
/// tracker write and refusal. Never fails the caller. The run's session is
/// who asked: Hollow writes for it.
pub(crate) fn record_tracker_write(root: &Path, write: &TrackerWrite<'_>) {
    let (outcome, detail) = match write.outcome {
        "unconfirmed" => (
            "failed",
            Some(write.detail.clone().unwrap_or_else(|| {
                "Unconfirmed: it may have gone; it is never sent again.".into()
            })),
        ),
        other => (other, write.detail.clone()),
    };
    let link = link_for(root, write.session_key);
    crate::audit::tracker_write(
        root,
        crate::audit::Write {
            action: write.action,
            session_key: Some(write.session_key),
            asked_by: write.session_key.to_string(),
            workspace_id: link.as_ref().and_then(|l| l.workspace_id.as_deref()),
            issue_id: Some(write.issue_id),
            issue_key: write
                .issue_key
                .or_else(|| link.as_ref().and_then(|l| l.issue_key.as_deref())),
            before: write.before.clone(),
            after: write.after.clone(),
            request_key: write.request_key,
            outcome,
            detail,
        },
    );
}

// ── Links ───────────────────────────────────────────────────────────────────

/// One row of `work_links`. `issue_id` is none while a create is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkLink {
    pub session_key: String,
    pub issue_id: Option<String>,
    pub issue_key: Option<String>,
    pub workspace_id: Option<String>,
    pub linked_by: String,
    pub created_issue: bool,
    pub at: String,
    /// The run's start depth: 2 or more is a helper, which never posts.
    pub depth: u32,
    /// Its run reports to the tracker (the switch was on at its start).
    pub tracked: bool,
    /// When its Outcome fact was recorded; later turns report as progress.
    pub outcome_at: Option<String>,
    /// T3g: the run's title as it started ([`run_title`]), the name every
    /// build log comment gives it; none on links made before T3g and on
    /// T3b/T3e copies.
    pub title: Option<String>,
}

fn link_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkLink> {
    Ok(WorkLink {
        session_key: row.get(0)?,
        issue_id: row.get(1)?,
        issue_key: row.get(2)?,
        workspace_id: row.get(3)?,
        linked_by: row.get(4)?,
        created_issue: row.get::<_, i64>(5)? != 0,
        at: row.get(6)?,
        depth: row.get::<_, i64>(7)?.max(0) as u32,
        tracked: row.get::<_, i64>(8)? != 0,
        outcome_at: row.get(9)?,
        title: row.get(10)?,
    })
}

const LINK_COLUMNS: &str = "session_key, issue_id, issue_key, workspace_id, linked_by, \
     created_issue, at, depth, tracked, outcome_at, title";

/// The run's link, once; a second start of the same key changes nothing.
pub(crate) fn insert_link(vault: &Path, link: &WorkLink) -> Result<bool, String> {
    session_store::with_write(vault, "work link", |tx| {
        tx.execute(
            &format!("INSERT OR IGNORE INTO work_links({LINK_COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
            params![
                link.session_key,
                link.issue_id,
                link.issue_key,
                link.workspace_id,
                link.linked_by,
                i64::from(link.created_issue),
                link.at,
                i64::from(link.depth),
                i64::from(link.tracked),
                link.outcome_at,
                link.title
            ],
        )
        .map(|n| n > 0)
        .map_err(|e| format!("work link: {e}"))
    })
}

pub(crate) fn link_for(vault: &Path, session_key: &str) -> Option<WorkLink> {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!("SELECT {LINK_COLUMNS} FROM work_links WHERE session_key = ?1"),
            params![session_key],
            link_row,
        )
        .optional()
        .map_err(|e| format!("work link: {e}"))
    })
    .ok()
    .flatten()
}

pub(crate) fn links_for_issue(vault: &Path, issue_id: &str) -> Vec<WorkLink> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {LINK_COLUMNS} FROM work_links WHERE issue_id = ?1 ORDER BY at DESC"
            ))
            .map_err(|e| format!("work links: {e}"))?;
        let rows = stmt
            .query_map(params![issue_id], link_row)
            .map_err(|e| format!("work links: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("work links: {e}"));
        rows
    })
    .unwrap_or_default()
}

/// A pending create was confirmed: the run's link names the new issue.
pub(crate) fn fill_created(
    vault: &Path,
    session_key: &str,
    issue_id: &str,
    issue_key: &str,
) -> Result<(), String> {
    let filled = session_store::with_write(vault, "work link create", |tx| {
        tx.execute(
            "UPDATE work_links SET issue_id = ?2, issue_key = ?3 \
             WHERE session_key = ?1 AND issue_id IS NULL",
            params![session_key, issue_id, issue_key],
        )
        .map_err(|e| format!("work link create: {e}"))
    })?;
    // X2: the run is now tied to its new issue.
    if filled > 0 {
        if let Some(link) = link_for(vault, session_key) {
            // The run's facts from its `run_started` row; a fresh time.
            let started = crate::audit::latest(vault, session_key, crate::audit::kind::RUN_STARTED);
            let base = crate::audit::AuditEntry {
                session_key: Some(session_key.into()),
                at: String::new(),
                ..started.unwrap_or_default()
            };
            crate::audit::record(vault, linked_entry(base, &link));
        }
    }
    Ok(())
}

/// A handover's successor continues its predecessor's run: the same link
/// (issue, depth, whether it reports, its Outcome already posted or not).
pub(crate) fn continue_link(vault: &Path, from: &str, to: &str) {
    let Some(link) = link_for(vault, from) else {
        return;
    };
    let next = WorkLink {
        session_key: to.into(),
        at: now(),
        ..link
    };
    if let Err(error) = insert_link(vault, &next) {
        eprintln!("[tracker work] handover link not recorded: {error}");
    }
}

/// A create that failed or was refused: the run is untied and its waiting
/// facts are taken, so they never hold the intake.
pub(crate) fn drop_link(vault: &Path, session_key: &str) -> Result<(), String> {
    session_store::with_write(vault, "work link drop", |tx| {
        tx.execute(
            "DELETE FROM work_links WHERE session_key = ?1 AND issue_id IS NULL",
            params![session_key],
        )
        .and_then(|_| {
            tx.execute(
                "UPDATE work_events SET journaled = 1 WHERE session_key = ?1",
                params![session_key],
            )
        })
        .map(|_| ())
        .map_err(|e| format!("work link drop: {e}"))
    })
}

fn set_outcome_at(vault: &Path, session_key: &str) -> Result<(), String> {
    session_store::with_write(vault, "work outcome", |tx| {
        tx.execute(
            "UPDATE work_links SET outcome_at = ?2 WHERE session_key = ?1",
            params![session_key, now()],
        )
        .map(|_| ())
        .map_err(|e| format!("work outcome: {e}"))
    })
}

// ── Host facts for the tracker (`work_events`) ─────────────────────────────

/// What the tracker is told about one run. Only host-known facts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkEvent {
    /// A new issue in the workspace's tracker project. `create_key` is the
    /// ledger create journal's key, the same for a retried start.
    #[serde(rename_all = "camelCase")]
    Create {
        create_key: String,
        /// Empty for a create in team PER, which has no project.
        project_id: String,
        title: String,
        description: String,
        /// The team it goes to (T3a): none for team ENG, as before; team
        /// HOL's id for Hollow-only work, created with its `agent` label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        team_id: Option<String>,
        /// Label names it is created with (T3b: `needs-review`), made in
        /// the team when missing. Empty: as before (HOL's `agent`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
        /// Assigned to the signed-in tracker user (the adapter's viewer,
        /// T3b); unassigned, and audited, while the viewer is unknown.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        assign_viewer: bool,
        /// The issue's due date, `YYYY-MM-DD` (T3e: a commitment's own due
        /// date); none leaves it unset.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        due: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Started {
        title: String,
        asked_by: String,
        agent: String,
        model: String,
        effort: String,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        line: String,
        #[serde(default)]
        link: Option<String>,
        /// T3g: the checks a follow-up turn's receipt recorded, for the
        /// build log comment's `Checked:` line. Empty for an agent's own
        /// note and on facts queued before it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checks: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    Outcome {
        /// The turn's status: `success`, `error` or `cancelled`.
        status: String,
        /// The reply's last line word: Done, Partial or Blocked.
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        prs: Vec<u64>,
        #[serde(default)]
        results: Vec<String>,
        #[serde(default)]
        checks: Vec<String>,
        /// T3g: files changed in the turn's repositories (the receipt's
        /// `repoChanges`), for the build log comment's `What:` line.
        #[serde(default)]
        files_changed: usize,
        working_ms: i64,
        tokens: i64,
        #[serde(default)]
        cost_usd: Option<f64>,
        /// Some usage rows had no price: the cost is a floor.
        #[serde(default)]
        cost_partial: bool,
        /// Why a failed or stopped turn ended (host-known only).
        #[serde(default)]
        reason: Option<String>,
    },
    /// A Waiting on you item was answered, dismissed or done (T3b): one
    /// comment saying how, then the issue to the team's completed state.
    /// Only for an issue Hollow created for that item.
    #[serde(rename_all = "camelCase")]
    Resolved {
        /// Short host copy, already redacted.
        answer: String,
    },
    /// A decision record was created or changed status (DR1b): one build log
    /// comment on its issue, carried by a run linked to that issue.
    #[serde(rename_all = "camelCase")]
    Decision {
        /// `recorded`, or the new status (`settled`, `revisit`, ...).
        change: String,
        /// The decision's own text.
        what: String,
        /// The decision's status now.
        status: String,
        why: Option<String>,
        /// Rejected options as the record writes them (`option -> reason; ...`);
        /// empty when none.
        #[serde(default)]
        rejected: String,
        verified: Option<String>,
    },
    /// A fact a later build queued that this one can't read: taken and
    /// dropped, never sent, so it never holds the intake.
    #[serde(other)]
    Unknown,
}

impl WorkEvent {
    fn kind(&self) -> &'static str {
        match self {
            WorkEvent::Create { .. } => "create",
            WorkEvent::Started { .. } => "started",
            WorkEvent::Progress { .. } => "progress",
            WorkEvent::Outcome { .. } => "outcome",
            WorkEvent::Resolved { .. } => "resolved",
            WorkEvent::Decision { .. } => "decision",
            WorkEvent::Unknown => "unknown",
        }
    }
}

/// One fact not yet in the ledger's outbox.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingEvent {
    pub id: i64,
    pub session_key: String,
    pub at: String,
    pub event: WorkEvent,
}

pub(crate) fn add_event(vault: &Path, session_key: &str, event: &WorkEvent) -> Result<i64, String> {
    let payload = serde_json::to_string(event).map_err(|e| format!("work event: {e}"))?;
    session_store::with_write(vault, "work event", |tx| {
        tx.execute(
            "INSERT INTO work_events(session_key, kind, payload, at) VALUES(?1, ?2, ?3, ?4)",
            params![session_key, event.kind(), payload, now()],
        )
        .map_err(|e| format!("work event: {e}"))?;
        Ok(tx.last_insert_rowid())
    })
}

/// Facts the outbox hasn't taken yet, oldest first (tests).
#[cfg(test)]
pub(crate) fn pending_events(vault: &Path, limit: usize) -> Result<Vec<PendingEvent>, String> {
    events_where(vault, "1 = 1", limit)
}

/// Facts ready for the outbox, oldest first: a create, a fact whose run's
/// issue is known, or one whose run has no link (taken and dropped).
/// Progress comes separately ([`progress_events`]), so notes waiting for
/// their fold, or facts waiting for a create, never hold this window.
pub(crate) fn ready_events(vault: &Path, limit: usize) -> Result<Vec<PendingEvent>, String> {
    events_where(
        vault,
        "e.kind != 'progress' AND (e.kind = 'create' OR l.session_key IS NULL \
         OR l.issue_id IS NOT NULL)",
        limit,
    )
}

/// Progress notes whose run's issue is known.
pub(crate) fn progress_events(vault: &Path, limit: usize) -> Result<Vec<PendingEvent>, String> {
    events_where(
        vault,
        "e.kind = 'progress' AND l.issue_id IS NOT NULL",
        limit,
    )
}

fn events_where(vault: &Path, sql: &str, limit: usize) -> Result<Vec<PendingEvent>, String> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT e.id, e.session_key, e.at, e.payload FROM work_events e \
                 LEFT JOIN work_links l ON l.session_key = e.session_key \
                 WHERE e.journaled = 0 AND ({sql}) ORDER BY e.id LIMIT ?1"
            ))
            .map_err(|e| format!("work events: {e}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| format!("work events: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("work events: {e}"))?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, session_key, at, payload)| {
                // A payload this build can't read is skipped, never sent.
                serde_json::from_str(&payload)
                    .ok()
                    .map(|event| PendingEvent {
                        id,
                        session_key,
                        at,
                        event,
                    })
            })
            .collect())
    })
}

/// The outbox holds these now (its ledger was saved first).
pub(crate) fn mark_journaled(vault: &Path, ids: &[i64]) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    session_store::with_write(vault, "work events journaled", |tx| {
        for id in ids {
            tx.execute(
                "UPDATE work_events SET journaled = 1 WHERE id = ?1",
                params![id],
            )
            .map_err(|e| format!("work events journaled: {e}"))?;
        }
        Ok(())
    })
}

/// Take every fact not yet taken (the switch is off: nothing back-posts).
pub(crate) fn take_all_events(vault: &Path) -> Result<usize, String> {
    session_store::with_write(vault, "work events off", |tx| {
        tx.execute(
            "UPDATE work_events SET journaled = 1 WHERE journaled = 0",
            [],
        )
        .map_err(|e| format!("work events off: {e}"))
    })
}

/// Drop taken facts older than [`EVENT_DAYS`].
pub(crate) fn prune_events(vault: &Path) -> Result<(), String> {
    let before = (Utc::now() - chrono::Duration::days(EVENT_DAYS))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    session_store::with_write(vault, "work events prune", |tx| {
        tx.execute(
            "DELETE FROM work_events WHERE journaled = 1 AND at < ?1",
            params![before],
        )
        .map(|_| ())
        .map_err(|e| format!("work events prune: {e}"))
    })
}

/// Fold a run's note into its own untaken note from the last minute, if any.
fn fold_progress(
    vault: &Path,
    session_key: &str,
    line: &str,
    link: Option<&str>,
    checks: &[String],
) -> Result<bool, String> {
    let since = (Utc::now() - chrono::Duration::seconds(PROGRESS_FOLD_SECONDS))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    session_store::with_write(vault, "work progress fold", |tx| {
        let last: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, payload FROM work_events WHERE session_key = ?1 AND kind = 'progress' \
                 AND journaled = 0 AND at >= ?2 ORDER BY id DESC LIMIT 1",
                params![session_key, since],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| format!("work progress fold: {e}"))?;
        let Some((id, payload)) = last else {
            return Ok(false);
        };
        let Ok(WorkEvent::Progress {
            line: old,
            link: old_link,
            checks: mut old_checks,
        }) = serde_json::from_str(&payload)
        else {
            return Ok(false);
        };
        add_checks(&mut old_checks, checks);
        let folded = WorkEvent::Progress {
            line: clip(&format!("{old} / {line}"), 280),
            link: link.map(str::to_string).or(old_link),
            checks: old_checks,
        };
        let payload =
            serde_json::to_string(&folded).map_err(|e| format!("work progress fold: {e}"))?;
        tx.execute(
            "UPDATE work_events SET payload = ?2 WHERE id = ?1",
            params![id, payload],
        )
        .map_err(|e| format!("work progress fold: {e}"))?;
        Ok(true)
    })
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ── Resolution at session start ────────────────────────────────────────────

/// What the host knows about a start, before it is queued.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct StartFacts {
    /// The subagent's title (its task's first line, or a worker's title).
    pub title: String,
    /// The task text as the asking agent wrote it.
    pub prompt: String,
    pub workspace_id: Option<String>,
    /// The workspace's folder (PROJECT.md and WORKSPACE.md live there).
    pub cwd: String,
    /// The conversation that started it (none for an automation's start).
    pub parent_key: Option<String>,
    /// Its start depth: 1 for a lead's or the COO's subagent, 2 for a helper.
    pub depth: u32,
    /// `issue` named by the start (a key such as ENG-123, or an id).
    pub packet_issue: Option<String>,
    /// The packet's source paths.
    pub sources: Vec<String>,
    /// Started by an automation: links to an existing issue only.
    pub automation: bool,
    /// `person`, a session key or `automation:<slug>`.
    pub asked_by: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// Which issue a start ties to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Resolution {
    /// Not tracked (audit only).
    None,
    Link {
        issue_id: Option<String>,
        issue_key: Option<String>,
        linked_by: &'static str,
    },
    Create {
        project_id: String,
        create_key: String,
    },
}

/// A start's resolution, made before it is queued (a refused packet issue
/// refuses the start) and recorded once it has its session key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Prepared {
    pub facts: StartFacts,
    pub resolution: Resolution,
    /// Settings' "Track agent work" was on at the start.
    pub track: bool,
}

/// Where a start runs, as tracking sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Place {
    /// A workspace whose PROJECT.md or brief names its tracker project.
    Tracked(String),
    /// Chats with no project: tied only when the start names an issue.
    QuickChats,
    /// A personal or vault-only workspace, or the vault root: never tied.
    Untracked,
}

/// The issues Hollow mirrors for the scoped team.
#[derive(Debug, Clone, Default)]
pub(crate) struct Mirror {
    pub issues: Vec<TrackerIssue>,
}

impl Mirror {
    /// The live mirrored issues of `team_id` from a sync ledger (no file or
    /// network read); other teams' are dropped by the scope filter.
    pub(crate) fn of(state: &crate::tracker_sync::Ledger) -> Mirror {
        let issues: Vec<TrackerIssue> = state
            .records
            .values()
            .filter(|r| !r.remote_missing && r.state == "active")
            .filter_map(|r| r.issue.clone())
            .filter(|i| i.archived_at.is_none() && !i.trashed)
            .collect();
        let teams = state.teams();
        Mirror {
            issues: issues
                .into_iter()
                .filter(|i: &TrackerIssue| teams.contains(&i.team_id))
                .collect(),
        }
    }

    fn find(&self, named: &str) -> Option<&TrackerIssue> {
        let named = named.trim();
        self.issues
            .iter()
            .find(|i| i.id == named || i.key.eq_ignore_ascii_case(named))
    }
}

/// The sync ledger as last saved, when it is this config's provider and
/// team. A read for display and resolution: no lock, no mirror file read.
pub(crate) fn read_ledger(config: &Config) -> Option<crate::tracker_sync::Ledger> {
    let provider = config.tracker.provider.as_deref()?;
    let scope = config.tracker.team_id.as_deref()?;
    read_ledger_at(&config.vault_path).and_then(|mut state| {
        // A pass for the personal team may be mid-way when this reads.
        if state.scope != scope && state.others.contains_key(scope) {
            state.swap_to(scope);
        }
        (state.provider == provider && state.scope == scope).then_some(state)
    })
}

/// The ledger as saved, whatever its scope (display reads only).
fn read_ledger_at(vault: &Path) -> Option<crate::tracker_sync::Ledger> {
    let raw = std::fs::read(vault.join(crate::tracker_sync::STATE)).ok()?;
    serde_json::from_slice(&raw).ok()
}

/// The configured team is team ENG, by its id (a key can be renamed).
pub(crate) fn ai_scope(config: &Config) -> bool {
    config.tracker.team_id.as_deref() == Some(AI_TEAM_ID)
}

/// `tracker_project` of the workspace in `cwd` (PROJECT.md first, then its
/// brief), when it names this provider.
pub(crate) fn place_of(config: &Config, workspace_id: Option<&str>, cwd: &str) -> Place {
    place_in(
        config.tracker.provider.as_deref().unwrap_or(""),
        workspace_id,
        cwd,
    )
}

/// [`place_of`] for a provider id.
pub(crate) fn place_in(provider: &str, workspace_id: Option<&str>, cwd: &str) -> Place {
    let Some(id) = workspace_id else {
        return Place::Untracked;
    };
    if crate::workspaces::is_quick_chats(id) {
        return Place::QuickChats;
    }
    for name in ["PROJECT.md", "WORKSPACE.md"] {
        let Ok(text) = std::fs::read_to_string(Path::new(cwd).join(name)) else {
            continue;
        };
        let fm = crate::frontmatter::parse(&text);
        if let Some(project) = fm.get("tracker_project").map(str::trim) {
            if !project.is_empty() && !provider.is_empty() && fm.get("tracker") == Some(provider) {
                return Place::Tracked(project.to_string());
            }
        }
    }
    Place::Untracked
}

/// A title's tokens: lowercase letters and digits.
pub(crate) fn tokens(title: &str) -> BTreeSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Token-set Jaccard similarity of two titles.
pub(crate) fn jaccard(a: &str, b: &str) -> f64 {
    let (a, b) = (tokens(a), tokens(b));
    let union = a.union(&b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(&b).count() as f64 / union as f64
}

/// The single best open issue of `project` whose title matches `title`; a
/// tie links nothing.
pub(crate) fn best_match<'a>(
    mirror: &'a Mirror,
    project: &str,
    title: &str,
) -> Option<&'a TrackerIssue> {
    let mut scored: Vec<(f64, &TrackerIssue)> = mirror
        .issues
        .iter()
        .filter(|i| i.project_id.as_deref() == Some(project) && !i.state.kind.is_closed())
        .map(|i| (jaccard(title, &i.title), i))
        .filter(|(score, _)| *score >= MATCH_THRESHOLD)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    match scored.as_slice() {
        [] => None,
        [(_, only)] => Some(only),
        [(best, issue), (next, _), ..] => ((best - next).abs() > f64::EPSILON).then_some(*issue),
    }
}

/// The create journal key a start of this work uses: the same workspace,
/// project, parent and title give the same key, so a retried start finds
/// the first one's create instead of making a second.
pub(crate) fn create_key(facts: &StartFacts, project: &str) -> String {
    use sha2::{Digest, Sha256};
    let title: Vec<String> = tokens(&facts.title).into_iter().collect();
    let digest = Sha256::digest(
        format!(
            "{}\n{project}\n{}\n{}",
            facts.workspace_id.as_deref().unwrap_or(""),
            facts.parent_key.as_deref().unwrap_or(""),
            title.join(" ")
        )
        .as_bytes(),
    );
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("work-{}", &hex[..32])
}

/// X0 section 3, pure: which issue a start ties to. `Err` refuses the start
/// (a named issue Hollow can't use). `task_ids` are the `tracker_id`s of the
/// packet's source tasks, in source order.
pub(crate) fn resolve(
    mirror: Option<&Mirror>,
    ai_scope: bool,
    place: &Place,
    facts: &StartFacts,
    parent_link: Option<&WorkLink>,
    task_ids: &[String],
    track: bool,
) -> Result<Resolution, String> {
    let link = |issue: &TrackerIssue, by: &'static str| Resolution::Link {
        issue_id: Some(issue.id.clone()),
        issue_key: Some(issue.key.clone()),
        linked_by: by,
    };
    if let Some(named) = facts
        .packet_issue
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        if *place == Place::Untracked {
            return Err(format!("The start names tracker issue {named}, but its workspace isn't a tracked project (no tracker_project), so its work never goes to the tracker. Start it in the project's workspace or in quick-chats, or leave issue out. Nothing started."));
        }
        let Some(mirror) = mirror.filter(|_| ai_scope) else {
            return Err(format!("The start names tracker issue {named}, but the task tracker isn't set up for team {AI_TEAM_KEY}. Nothing started."));
        };
        let Some(issue) = mirror.find(named) else {
            return Err(format!("Tracker issue {named} isn't in team {AI_TEAM_KEY}'s mirror (tracker.view lists them; sync first if it is new). Nothing started."));
        };
        return Ok(link(issue, linked_by::PACKET));
    }
    if *place == Place::Untracked {
        return Ok(Resolution::None);
    }
    if let Some(parent) = parent_link {
        return Ok(Resolution::Link {
            issue_id: parent.issue_id.clone(),
            issue_key: parent.issue_key.clone(),
            linked_by: linked_by::INHERITED,
        });
    }
    // A helper (a subagent's subagent) only ever inherits.
    if facts.depth >= 2 {
        return Ok(Resolution::None);
    }
    let Place::Tracked(project) = place else {
        return Ok(Resolution::None);
    };
    let Some(mirror) = mirror.filter(|_| ai_scope) else {
        return Ok(Resolution::None);
    };
    if let Some(issue) = task_ids.iter().find_map(|id| mirror.find(id)) {
        return Ok(link(issue, linked_by::TASK));
    }
    if let Some(issue) = best_match(mirror, project, &facts.title) {
        return Ok(link(issue, linked_by::MATCHED));
    }
    // Never from an automation, and only when the switch lets Hollow write.
    if facts.automation || !track {
        return Ok(Resolution::None);
    }
    Ok(Resolution::Create {
        project_id: project.clone(),
        create_key: create_key(facts, project),
    })
}

/// The `tracker_id` of each packet source that is a mirrored task file.
fn source_task_ids(config: &Config, sources: &[String]) -> Vec<String> {
    let provider = config.tracker.provider.as_deref().unwrap_or("");
    let vault = &config.vault_path;
    sources
        .iter()
        .filter_map(|path| {
            let rel = Path::new(path)
                .strip_prefix(vault)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.trim_start_matches("./").to_string());
            let full = crate::vault::safe_join(vault, &rel).ok()?;
            let text = std::fs::read_to_string(full).ok()?;
            let fm = crate::frontmatter::parse(&text);
            (fm.get("tracker") == Some(provider))
                .then(|| fm.get("tracker_id").map(str::to_string))
                .flatten()
        })
        .collect()
}

/// Resolve a start before it is queued (reads only).
pub(crate) fn prepare(config: &Config, facts: StartFacts) -> Result<Prepared, String> {
    let vault = &config.vault_path;
    let place = place_of(config, facts.workspace_id.as_deref(), &facts.cwd);
    let ledger = (place != Place::Untracked || facts.packet_issue.is_some())
        .then(|| read_ledger(config))
        .flatten();
    let mirror = ledger.as_ref().map(|state| {
        let mut mirror = Mirror::of(state);
        // A just-created issue the mirror hasn't pulled yet still matches,
        // so a second start doesn't create it again.
        for issue in state.creates.values().filter_map(|c| c.issue.clone()) {
            if issue.team_id == state.scope && mirror.find(&issue.id).is_none() {
                mirror.issues.push(issue);
            }
        }
        mirror
    });
    let parent_link = facts
        .parent_key
        .as_deref()
        .and_then(|parent| link_for(vault, parent));
    let tasks = if matches!(place, Place::Tracked(_)) {
        source_task_ids(config, &facts.sources)
    } else {
        Vec::new()
    };
    let track = config.tracker.track_agent_work && ai_scope(config);
    let resolution = match resolve(
        mirror.as_ref(),
        ai_scope(config),
        &place,
        &facts,
        parent_link.as_ref(),
        &tasks,
        track,
    ) {
        Ok(resolution) => resolution,
        Err(reason) => {
            // X2: a named issue Hollow can't use (outside team ENG, unknown,
            // or an untracked place): refused before anything is sent.
            crate::audit::tracker_write(
                vault,
                crate::audit::Write {
                    action: "link",
                    session_key: facts.parent_key.as_deref(),
                    asked_by: facts.asked_by.clone(),
                    workspace_id: facts.workspace_id.as_deref(),
                    issue_key: facts.packet_issue.as_deref(),
                    outcome: "failed",
                    detail: Some(reason.clone()),
                    ..crate::audit::Write::default()
                },
            );
            return Err(reason);
        }
    };
    Ok(Prepared {
        facts,
        resolution,
        track,
    })
}

/// The quoted request a created issue carries, with Hollow's notes.
fn create_description(facts: &StartFacts) -> String {
    let request: String = crate::claude_runner::redact(facts.prompt.trim())
        .chars()
        .take(1500)
        .collect();
    let quoted: Vec<String> = request.lines().map(|l| format!("> {l}")).collect();
    format!(
        "{}\n\n+++ Agent notes\n\nCreated by Hollow for delegated agent work (asked by {}). Hollow posts the run's Started and Outcome comments here.\n\n+++",
        quoted.join("\n"),
        clip(&facts.asked_by, 80)
    )
}

/// Record a queued start: its link, the audit seam and, with tracking on,
/// the facts the tracker gets. Never fails the start.
pub(crate) fn begin(vault: &Path, session_key: &str, prepared: Prepared) {
    begin_as(
        vault,
        session_key,
        prepared,
        Created {
            by: linked_by::CREATED,
            team_id: None,
            description: None,
        },
    );
}

/// How a create's link and fact read (T3a): its `linked_by`, its team (HOL's
/// id for Hollow-only work; none for team ENG) and its description (none:
/// the delegated-work one quoting the start's task).
pub(crate) struct Created {
    pub by: &'static str,
    pub team_id: Option<String>,
    pub description: Option<String>,
}

/// [`begin`] with how a create reads. The link and its facts are written in
/// one transaction, so a crash leaves both or neither; a start whose link
/// already exists queues nothing. Returns whether facts were queued.
pub(crate) fn begin_as(
    vault: &Path,
    session_key: &str,
    prepared: Prepared,
    created: Created,
) -> bool {
    let Prepared {
        facts,
        resolution,
        track,
    } = prepared;
    let link = match &resolution {
        Resolution::None => None,
        Resolution::Link {
            issue_id,
            issue_key,
            linked_by,
        } => Some(WorkLink {
            session_key: session_key.into(),
            issue_id: issue_id.clone(),
            issue_key: issue_key.clone(),
            workspace_id: facts.workspace_id.clone(),
            linked_by: (*linked_by).into(),
            created_issue: false,
            at: now(),
            depth: facts.depth,
            tracked: track && facts.depth < 2,
            outcome_at: None,
            title: run_title(&facts.title),
        }),
        Resolution::Create { .. } => Some(WorkLink {
            session_key: session_key.into(),
            issue_id: None,
            issue_key: None,
            workspace_id: facts.workspace_id.clone(),
            linked_by: created.by.into(),
            created_issue: true,
            at: now(),
            depth: facts.depth,
            tracked: track && facts.depth < 2,
            outcome_at: None,
            title: run_title(&facts.title),
        }),
    };
    // A helper's run (depth 2) is summed into its parent's outcome and never
    // posts; a depth-1 run that inherited its parent's issue (a lane) does.
    let mut events = Vec::new();
    if link.as_ref().is_some_and(|l| l.tracked) {
        if let Resolution::Create {
            project_id,
            create_key,
        } = &resolution
        {
            events.push(WorkEvent::Create {
                create_key: create_key.clone(),
                project_id: project_id.clone(),
                title: clip(facts.title.trim(), 200),
                description: created
                    .description
                    .clone()
                    .unwrap_or_else(|| create_description(&facts)),
                team_id: created.team_id.clone(),
                labels: Vec::new(),
                assign_viewer: false,
                due: None,
            });
        }
        let text = |value: &Option<String>| value.clone().unwrap_or_else(|| "default".into());
        events.push(WorkEvent::Started {
            title: facts.title.clone(),
            asked_by: facts.asked_by.clone(),
            agent: text(&facts.agent),
            model: text(&facts.model),
            effort: text(&facts.effort),
        });
    }
    let mut queued = false;
    if let Some(link) = &link {
        match insert_link_with(vault, link, &events) {
            Ok(inserted) => queued = inserted && !events.is_empty(),
            Err(error) => eprintln!("[tracker work] link not recorded: {error}"),
        }
    }
    record_run(
        vault,
        &RunEvent::Started {
            session_key,
            link: link.as_ref(),
            facts: &facts,
        },
    );
    if queued {
        crate::tracker_sync::request_soon();
    }
    queued
}

/// The link and, only when it is new, its facts, in one transaction.
pub(crate) fn insert_link_with(
    vault: &Path,
    link: &WorkLink,
    events: &[WorkEvent],
) -> Result<bool, String> {
    let payloads = events
        .iter()
        .map(|e| serde_json::to_string(e).map(|p| (e.kind(), p)))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("work event: {e}"))?;
    let at = now();
    session_store::with_write(vault, "work link", |tx| {
        let inserted = tx
            .execute(
                &format!("INSERT OR IGNORE INTO work_links({LINK_COLUMNS}) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
                params![
                    link.session_key,
                    link.issue_id,
                    link.issue_key,
                    link.workspace_id,
                    link.linked_by,
                    i64::from(link.created_issue),
                    link.at,
                    i64::from(link.depth),
                    i64::from(link.tracked),
                    link.outcome_at,
                link.title
                ],
            )
            .map_err(|e| format!("work link: {e}"))?
            > 0;
        if inserted {
            for (kind, payload) in &payloads {
                tx.execute(
                    "INSERT INTO work_events(session_key, kind, payload, at) VALUES(?1, ?2, ?3, ?4)",
                    params![link.session_key, kind, payload, at],
                )
                .map_err(|e| format!("work event: {e}"))?;
            }
        }
        Ok(inserted)
    })
}

/// Links made by `by` whose closing fact isn't queued yet (`outcome_at`
/// unset), T3b's open Waiting on you copies.
pub(crate) fn open_links_by(vault: &Path, by: &str) -> Vec<WorkLink> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {LINK_COLUMNS} FROM work_links WHERE linked_by = ?1 AND outcome_at IS NULL"
            ))
            .map_err(|e| format!("work links: {e}"))?;
        let rows = stmt
            .query_map(params![by], link_row)
            .map_err(|e| format!("work links: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("work links: {e}"));
        rows
    })
    .unwrap_or_default()
}

/// Queue `event` as the link's last fact and mark it (`outcome_at`), in one
/// transaction: once only, so a second call queues nothing. Returns whether
/// it queued.
pub(crate) fn finish_link(
    vault: &Path,
    session_key: &str,
    event: &WorkEvent,
) -> Result<bool, String> {
    let payload = serde_json::to_string(event).map_err(|e| format!("work event: {e}"))?;
    let at = now();
    let queued = session_store::with_write(vault, "work link finish", |tx| {
        let marked = tx
            .execute(
                "UPDATE work_links SET outcome_at = ?2 WHERE session_key = ?1 AND outcome_at IS NULL",
                params![session_key, at],
            )
            .map_err(|e| format!("work link finish: {e}"))?
            > 0;
        if marked {
            tx.execute(
                "INSERT INTO work_events(session_key, kind, payload, at) VALUES(?1, ?2, ?3, ?4)",
                params![session_key, event.kind(), payload, at],
            )
            .map_err(|e| format!("work event: {e}"))?;
        }
        Ok(marked)
    })?;
    if queued {
        crate::tracker_sync::request_soon();
    }
    Ok(queued)
}

/// Whether any fact (taken or not) was ever queued for `session_key`.
pub(crate) fn has_events(vault: &Path, session_key: &str) -> bool {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM work_events WHERE session_key = ?1)",
            params![session_key],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|e| format!("work events: {e}"))
    })
    .unwrap_or(true)
}

// ── Run end ─────────────────────────────────────────────────────────────────

/// The reply's last-line status word (Done, Partial or Blocked).
fn result_word(reply: &str) -> Option<String> {
    let last = reply.lines().rev().map(str::trim).find(|l| !l.is_empty())?;
    let last = last.trim_matches(|c: char| c == '*' || c == '_' || c == '`');
    ["Done", "Partial", "Blocked"]
        .into_iter()
        .find(|word| {
            last.strip_prefix(word)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '.', ' ']))
        })
        .map(str::to_string)
}

/// Pull request numbers the reply names (`/pull/123`, `PR 123`, `PR #123`).
pub(crate) fn pull_requests(reply: &str) -> Vec<u64> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static PR: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?:/pull/|\bPRs?\s*#?)(\d{1,7})\b").unwrap());
    let mut out: Vec<u64> = PR
        .captures_iter(reply)
        .filter_map(|c| c[1].parse().ok())
        .collect();
    out.sort_unstable();
    out.dedup();
    out.truncate(10);
    out
}

/// Working time, tokens and cost of `keys`, from their turns and usage
/// rows, and whether some rows had no price (the cost is then a floor).
fn totals(vault: &Path, keys: &[String]) -> (i64, i64, Option<f64>, bool) {
    let now = Utc::now();
    let mut working = 0;
    let mut tokens = 0;
    let mut cost: Option<f64> = None;
    for key in keys {
        if let Ok(Some(row)) = crate::subagents::row_of(vault, key, now) {
            working += row.working_ms;
            tokens += row.tokens;
            if let Some(c) = row.cost_usd {
                cost = Some(cost.unwrap_or(0.0) + c);
            }
        }
    }
    let marks = vec!["?"; keys.len()].join(",");
    let (rows, priced): (i64, i64) = session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!(
                "SELECT COUNT(*), COUNT(u.cost_usd) FROM usage_ledger u \
                 JOIN turns t ON t.run_id = u.run_id WHERE t.session_key IN ({marks})"
            ),
            rusqlite::params_from_iter(keys.iter()),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("work totals: {e}"))
    })
    .unwrap_or((0, 0));
    (working, tokens, cost, cost.is_some() && rows > priced)
}

/// The session and the helpers that inherited its link.
fn with_helpers(vault: &Path, session_key: &str) -> Vec<String> {
    let mut keys = vec![session_key.to_string()];
    let helpers: Vec<String> = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT w.session_key FROM work_links w JOIN sessions s ON s.key = w.session_key \
                 WHERE s.parent_key = ?1 AND w.linked_by = 'inherited' AND w.depth >= 2",
            )
            .map_err(|e| format!("work helpers: {e}"))?;
        let rows = stmt
            .query_map(params![session_key], |row| row.get::<_, String>(0))
            .map_err(|e| format!("work helpers: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("work helpers: {e}"));
        rows
    })
    .unwrap_or_default();
    keys.extend(helpers);
    keys
}

/// The session was started as a subagent (`sessions.fork_kind`).
fn is_subagent(vault: &Path, session_key: &str) -> bool {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT fork_kind FROM sessions WHERE key = ?1",
            params![session_key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .flatten()
    .is_some_and(|kind| kind == session_store::FORK_SUBAGENT)
}

/// A subagent's or linked run's turn ended (chat.rs, after its receipt): its
/// `run_ended` audit row and, when its run reports and tracking is on now,
/// its Outcome (the first turn end) or a progress note (each later turn,
/// folded like any note).
pub(crate) fn after_turn(
    vault: &Path,
    session_key: &str,
    status: &str,
    receipt: Option<&crate::evidence::EvidenceReceipt>,
    reply: &str,
) {
    let link = link_for(vault, session_key);
    if link.is_none() && !is_subagent(vault, session_key) {
        return;
    }
    // X2: the run's `run_ended` row first: the Outcome sums these rows.
    record_run(
        vault,
        &RunEvent::Ended {
            session_key,
            status,
        },
    );
    let Some(link) = link else {
        return;
    };
    if link.depth >= 2 {
        return;
    }
    if !link.tracked || !tracking(vault) {
        dry_run_build_log(vault, &link, receipt, Utc::now());
        return;
    }
    let prs = pull_requests(reply);
    let result = result_word(reply);
    let reason = match status {
        "success" => None,
        "cancelled" => Some("The turn was stopped.".to_string()),
        _ => Some(
            receipt
                .and_then(|r| r.failures.first())
                .map(|f| format!("The turn failed: {}.", clip(&f.label, 120)))
                .unwrap_or_else(|| "The turn ended with an error.".into()),
        ),
    };
    let checks = receipt.map(checks_of).unwrap_or_default();
    let files_changed = receipt.map_or(0, files_changed);
    if link.outcome_at.is_some() {
        // A follow-up turn: a progress note, never a second Outcome.
        let mut line = match status {
            "success" => "A follow-up turn finished".to_string(),
            "cancelled" => "A follow-up turn was stopped".to_string(),
            _ => "A follow-up turn failed".to_string(),
        };
        if let Some(result) = &result {
            line.push_str(&format!(" ({result})"));
        }
        if !prs.is_empty() {
            let list: Vec<String> = prs.iter().map(u64::to_string).collect();
            line.push_str(&format!("; pull requests {}", list.join(", ")));
        }
        if files_changed > 0 {
            line.push_str(&format!("; {}", files_word(files_changed)));
        }
        if let Some(reason) = reason.as_deref().filter(|_| status != "success") {
            line.push_str(&format!(". {reason}"));
        }
        record_progress(vault, session_key, &clip(&line, 280), None, &checks);
        return;
    }
    let mut results: Vec<String> = Vec::new();
    if let Some(receipt) = receipt {
        for file in &receipt.files {
            let output = file.path.starts_with("outputs/") || file.path.contains("/outputs/");
            if output
                && file.attribution != "shared"
                && file.change_type != "deleted"
                && !results.contains(&file.path)
                && results.len() < 8
            {
                results.push(file.path.clone());
            }
        }
    }
    // Linear's copy equals Hollow's record: the numbers are the audit's
    // `run_ended` rows of the run and its helpers, summed (the live totals
    // only when the audit couldn't be written).
    let keys = with_helpers(vault, session_key);
    let (working_ms, tokens, cost_usd, cost_partial) = match crate::audit::run_sums(vault, &keys) {
        Some(sums) => (
            sums.working_ms,
            sums.tokens,
            sums.cost_usd,
            sums.cost_partial,
        ),
        None => totals(vault, &keys),
    };
    let event = WorkEvent::Outcome {
        status: status.to_string(),
        result,
        prs,
        results,
        checks,
        files_changed,
        working_ms,
        tokens,
        cost_usd,
        cost_partial,
        reason,
    };
    match add_event(vault, session_key, &event).and_then(|_| set_outcome_at(vault, session_key)) {
        Ok(()) => crate::tracker_sync::request_soon(),
        Err(error) => eprintln!("[tracker work] outcome not recorded: {error}"),
    }
}

/// T3g: under `trackAgentWork` `dryRun`, a linked run's turn end leaves an
/// audit row saying which issue its build log comment would go on; nothing
/// is queued or sent. It folds as the real path does: the run's first such
/// row stands for its Outcome; later turn ends are progress, at most one row
/// per issue every [`PROGRESS_MINUTES`]. Keyed by the receipt, or by the
/// session and its turn count without one, so a repeat adds no row. Off (or
/// a copy, or an issue still being created) leaves none.
pub(crate) fn dry_run_build_log(
    vault: &Path,
    link: &WorkLink,
    receipt: Option<&crate::evidence::EvidenceReceipt>,
    now: DateTime<Utc>,
) {
    let dry = super::chat_work::running_mode(vault) == crate::tracker::settings::AgentWork::DryRun;
    let Some(issue_id) = link.issue_id.as_deref().filter(|_| dry) else {
        return;
    };
    if linked_by::is_copy(&link.linked_by) {
        return;
    }
    let key = match receipt {
        Some(receipt) => format!("build-log-{}", receipt.id),
        None => format!(
            "build-log-{}-turn-{}",
            link.session_key,
            turn_count(vault, &link.session_key)
        ),
    };
    let rows = |session: &str| -> Vec<crate::audit::AuditEntry> {
        crate::audit::rows_of(vault, session, crate::audit::kind::TRACKER_WRITE)
            .into_iter()
            .filter(|r| {
                r.action.as_deref() == Some("comment")
                    && r.request_key
                        .as_deref()
                        .is_some_and(|k| k.starts_with("build-log-"))
            })
            .collect()
    };
    let mine = rows(&link.session_key);
    if mine.iter().any(|r| r.request_key.as_deref() == Some(&key)) {
        return;
    }
    let first = mine.is_empty();
    if !first {
        let since = now - chrono::Duration::minutes(PROGRESS_MINUTES);
        let recent = links_for_issue(vault, issue_id)
            .iter()
            .flat_map(|l| rows(&l.session_key))
            .filter(|r| {
                // The run's Outcome row isn't progress: it holds nothing.
                r.detail
                    .as_deref()
                    .is_some_and(|d| d.contains("progress build log"))
            })
            .any(|r| {
                DateTime::parse_from_rfc3339(&r.at).is_ok_and(|at| at.with_timezone(&Utc) > since)
            });
        if recent {
            return;
        }
    }
    let what = if first {
        "this run's build log comment (its outcome)"
    } else {
        "a progress build log comment"
    };
    record_tracker_write(
        vault,
        &TrackerWrite {
            session_key: &link.session_key,
            issue_id,
            issue_key: link.issue_key.as_deref(),
            action: "comment",
            outcome: "skipped",
            request_key: Some(&key),
            detail: Some(format!(
                "Dry run: Hollow would post {what} on {}; nothing was sent.",
                link.issue_key.as_deref().unwrap_or(issue_id)
            )),
            ..TrackerWrite::default()
        },
    );
}

/// How many turns `session_key` has recorded.
fn turn_count(vault: &Path, session_key: &str) -> i64 {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE session_key = ?1",
            params![session_key],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())
    })
    .unwrap_or(0)
}

/// At most this many checks go on one build log comment.
const MAX_CHECKS: usize = 8;

/// T3g: the checks a turn's receipt recorded, as `kind outcome (runner)`, at
/// most [`MAX_CHECKS`], each once. The runner is a known tool and its
/// subcommand only ([`runner_of`]); never arguments, paths, file names or
/// URLs, since the comment can go on a shared team's issue.
fn checks_of(receipt: &crate::evidence::EvidenceReceipt) -> Vec<String> {
    let mut out = Vec::new();
    let lines = receipt.checks.iter().map(|check| {
        let kind = clip(&check.kind, 20);
        let outcome = clip(&check.outcome, 20);
        match runner_of(&check.command) {
            Some(runner) => format!("{kind} {outcome} ({runner})"),
            None => format!("{kind} {outcome}"),
        }
    });
    add_checks(&mut out, &lines.collect::<Vec<_>>());
    out
}

/// Tools named alone.
const RUNNERS: [&str; 16] = [
    "vitest",
    "jest",
    "pytest",
    "tsc",
    "eslint",
    "playwright",
    "mocha",
    "ruff",
    "mypy",
    "prettier",
    "rspec",
    "phpunit",
    "swift",
    "xcodebuild",
    "gradle",
    "mvn",
];
/// Subcommands and package scripts named with their tool.
const RUNNER_STEPS: [&str; 14] = [
    "test",
    "t",
    "build",
    "check",
    "clippy",
    "fmt",
    "nextest",
    "vet",
    "gate",
    "lint",
    "typecheck",
    "smoke",
    "e2e",
    "format",
];
/// Words before the tool that are skipped (wrappers).
const RUNNER_WRAPPERS: [&str; 9] = [
    "env",
    "time",
    "nice",
    "npx",
    "bunx",
    "hollow-slot",
    "with-redis",
    "with-docker",
    "exec",
];

/// T3g: a check command's runner, from a fixed list only: `cargo test`,
/// `npm run gate`, `vitest`, `pytest` (also from `python -m pytest`). Its
/// arguments, paths and anything unknown are dropped; `None` when no known
/// runner is found, so the check reads `kind outcome` alone.
pub(crate) fn runner_of(command: &str) -> Option<String> {
    for segment in command.split(['&', ';', '|', '\n']) {
        let mut words = segment.split_whitespace().peekable();
        let mut program = None;
        while let Some(word) = words.next() {
            let base = word.rsplit('/').next().unwrap_or(word);
            let assignment = word.contains('=') && !word.starts_with('-');
            if assignment || word.starts_with('-') {
                // A wrapper's flag; one that takes a value skips it too.
                if matches!(word, "--on" | "--send" | "--fetch" | "-C") {
                    words.next();
                }
                continue;
            }
            if RUNNER_WRAPPERS.contains(&base) {
                continue;
            }
            if matches!(base, "python" | "python3") {
                if words.next() == Some("-m") {
                    program = words.next().map(str::to_string);
                }
                break;
            }
            program = Some(base.to_string());
            break;
        }
        let Some(program) = program else {
            continue;
        };
        let program = program.as_str();
        if RUNNERS.contains(&program) {
            return Some(program.to_string());
        }
        fn step(word: Option<&str>) -> Option<&str> {
            word.filter(|w| RUNNER_STEPS.contains(w))
        }
        match program {
            "cargo" | "go" | "make" => {
                let sub = words.find(|w| !w.starts_with('-') && !w.starts_with('+'));
                return Some(match step(sub) {
                    Some(sub) => format!("{program} {sub}"),
                    None => program.to_string(),
                });
            }
            "npm" | "pnpm" | "yarn" | "bun" => {
                let sub = words.find(|w| !w.starts_with('-'));
                return Some(match sub {
                    Some("run") => match step(words.find(|w| !w.starts_with('-'))) {
                        Some(script) => format!("{program} run {script}"),
                        None => format!("{program} run"),
                    },
                    other => match step(other) {
                        Some(sub) => format!("{program} {sub}"),
                        None => program.to_string(),
                    },
                });
            }
            "node" | "deno" => return Some(program.to_string()),
            _ => {}
        }
    }
    None
}

fn add_checks(into: &mut Vec<String>, more: &[String]) {
    for line in more {
        if !into.contains(line) && into.len() < MAX_CHECKS {
            into.push(line.clone());
        }
    }
}

/// T3g: files changed or commits made in the turn's repositories.
fn files_changed(receipt: &crate::evidence::EvidenceReceipt) -> usize {
    receipt
        .repo_changes
        .iter()
        .map(|repo| repo.total_files.max(repo.files.len()))
        .sum()
}

fn files_word(n: usize) -> String {
    if n == 1 {
        "1 file changed in repositories".into()
    } else {
        format!("{n} files changed in repositories")
    }
}

/// A progress fact, folded into the run's note from the last minute if any.
fn record_progress(
    vault: &Path,
    session_key: &str,
    line: &str,
    link: Option<&str>,
    checks: &[String],
) {
    let done = match fold_progress(vault, session_key, line, link, checks) {
        Ok(true) => Ok(()),
        _ => add_event(
            vault,
            session_key,
            &WorkEvent::Progress {
                line: line.to_string(),
                link: link.map(str::to_string),
                checks: checks.to_vec(),
            },
        )
        .map(|_| ()),
    };
    match done {
        Ok(()) => crate::tracker_sync::request_soon(),
        Err(error) => eprintln!("[tracker work] progress not recorded: {error}"),
    }
}

// ── Progress (`tracker.report_progress`) ───────────────────────────────────

/// A subagent's one-line progress note for its issue. Recorded here; the
/// sync pass posts at most one progress comment per issue every 15 minutes
/// and folds the rest into the next.
pub(crate) fn report_progress(
    vault: &Path,
    caller: &crate::app_actions::Caller,
    line: &str,
    link: Option<&str>,
) -> Result<Value, String> {
    if caller.surface != "chat" {
        return Err(
            "tracker.report_progress is for a subagent's chat; nothing was recorded.".into(),
        );
    }
    let line = line.trim();
    if line.is_empty() || line.contains(['\n', '\r']) || line.chars().count() > 280 {
        return Err("Progress is one line of 1 to 280 characters; nothing was recorded.".into());
    }
    let link = link.map(str::trim).filter(|l| !l.is_empty());
    if link.is_some_and(|l| l.len() > 500 || l.contains(char::is_whitespace)) {
        return Err("The link is one URL or vault path (no spaces, at most 500 characters); nothing was recorded.".into());
    }
    let Some(work) = link_for(vault, &caller.scope) else {
        return Ok(
            json!({"recorded": false, "note": "This conversation's work isn't tied to a tracker issue, so there is nowhere to report progress."}),
        );
    };
    if work.depth >= 2 {
        return Ok(
            json!({"recorded": false, "note": "A helper reports through its parent: tell the conversation that started you instead."}),
        );
    }
    if !work.tracked || !tracking(vault) {
        return Ok(
            json!({"recorded": false, "note": "Tracking agent work is off (Settings, Task tracker), so nothing goes to the tracker."}),
        );
    }
    record_progress(vault, &caller.scope, line, link, &[]);
    Ok(json!({
        "recorded": true,
        "issue": work.issue_key,
        "note": format!("Recorded. Hollow posts it in the issue's next progress comment (at most one every {PROGRESS_MINUTES} minutes; notes in between fold into it).")
    }))
}

// ── Comment bodies (fixed format, host facts only) ─────────────────────────

/// At most `chars` characters of one line.
pub(crate) fn clip(text: &str, chars: usize) -> String {
    let one: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(chars)
        .collect();
    one
}

/// Redacted, capped at [`COMMENT_MAX_BYTES`] with the record line kept.
fn finish_body(head: &str, lines: &[String], session_key: &str, record_id: &str) -> String {
    let tail = format!("\n\nSession: {session_key}\nHollow record {record_id}");
    let mut text = format!("{head}\n");
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    let mut text = crate::claude_runner::redact(&text);
    let room = COMMENT_MAX_BYTES.saturating_sub(tail.len());
    if text.len() > room {
        let mut cut = room.saturating_sub(3);
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("...");
    }
    text.push_str(&tail);
    text
}

pub(crate) fn render_started(event: &WorkEvent, session_key: &str, record_id: &str) -> String {
    let WorkEvent::Started {
        title,
        asked_by,
        agent,
        model,
        effort,
    } = event
    else {
        return String::new();
    };
    finish_body(
        "Hollow: agent work started",
        &[
            format!("Title: {}", clip(title, 200)),
            format!("Asked by: {}", clip(asked_by, 80)),
            format!(
                "Agent: {} · Model: {} · Effort: {}",
                clip(agent, 40),
                clip(model, 60),
                clip(effort, 20)
            ),
        ],
        session_key,
        record_id,
    )
}

fn minutes(ms: i64) -> String {
    let m = (ms.max(0) + 59_999) / 60_000;
    if m >= 60 {
        format!("{} h {} min", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

/// T3g: the head of a build log comment, from host facts only: when (the
/// fact's time, local), who (the run's own title, as its Started comment
/// gives it) and why (the issue it is linked to, as the issue itself reads).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogHead {
    pub at: String,
    pub who: String,
    pub why: String,
}

impl LogHead {
    /// The head for `session_key`'s fact recorded at `at`, on `issue`.
    pub(crate) fn of(vault: &Path, session_key: &str, at: &str, issue: &TrackerIssue) -> LogHead {
        LogHead {
            at: local_stamp(at),
            who: who_of(vault, session_key),
            why: why_of(issue),
        }
    }
}

/// `at` (RFC 3339) as local date and time with its offset, e.g.
/// `2026-11-15 14:03 (UTC-05:00)`; now when it doesn't parse.
pub(crate) fn local_stamp(at: &str) -> String {
    let local = DateTime::parse_from_rfc3339(at)
        .map(|t| t.with_timezone(&chrono::Local))
        .unwrap_or_else(|_| chrono::Local::now());
    local.format("%Y-%m-%d %H:%M (UTC%:z)").to_string()
}

/// The run's title as it started: its first line, redacted, at most 80
/// characters (what its Started comment or, for a chat, its issue already
/// carries). Stored with the link, so every comment names it the same way.
pub(crate) fn run_title(title: &str) -> Option<String> {
    title
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|t| clip(crate::claude_runner::redact(t).trim(), 80))
        .filter(|t| !t.is_empty())
}

/// The run's name for a build log head: the title its link captured at the
/// start (never the live session title, which a rename or a later summary
/// may change) and what it is: subagent, chat or agent.
pub(crate) fn who_of(vault: &Path, session_key: &str) -> String {
    let link = link_for(vault, session_key);
    let what = if is_subagent(vault, session_key) {
        "subagent"
    } else if link
        .as_ref()
        .is_some_and(|l| l.linked_by == linked_by::CHAT)
    {
        "chat"
    } else {
        "agent"
    };
    match link.and_then(|l| l.title) {
        Some(title) => format!("{title} ({what})"),
        None => format!("A {what}"),
    }
}

/// The issue the work is linked to, as the issue itself reads (a team ENG
/// copy's generic title stays generic).
pub(crate) fn why_of(issue: &TrackerIssue) -> String {
    format!("work on {}, {}", issue.key, clip(&issue.title, 160))
}

fn build_log_head(head: &LogHead) -> String {
    format!(
        "Build log, {}, {}",
        clip(&head.at, 40),
        clip(&head.who, 120)
    )
}

fn checked_line(checks: &[String]) -> String {
    if checks.is_empty() {
        "Checked: no check recorded".into()
    } else {
        format!("Checked: {}", clip(&checks.join(", "), 500))
    }
}

/// T3g: a run's first turn end as a build log comment: dated head, then
/// `What:` (status, result word, pull requests, repository changes,
/// results, a failure's reason), `Why:` (the linked issue) and `Checked:`
/// (the receipt's checks), then the run's totals.
pub(crate) fn render_outcome(
    event: &WorkEvent,
    head: &LogHead,
    session_key: &str,
    record_id: &str,
) -> String {
    let WorkEvent::Outcome {
        status,
        result,
        prs,
        results,
        checks,
        files_changed,
        working_ms,
        tokens,
        cost_usd,
        cost_partial,
        reason,
    } = event
    else {
        return String::new();
    };
    let mut what = match status.as_str() {
        "success" => "Finished".to_string(),
        "cancelled" => "Stopped".to_string(),
        _ => "Failed".to_string(),
    };
    if let Some(result) = result {
        what.push_str(&format!(" ({result})"));
    }
    what.push('.');
    if let Some(reason) = reason {
        what.push_str(&format!(" {}", clip(reason, 200)));
    }
    if !prs.is_empty() {
        let list: Vec<String> = prs.iter().map(u64::to_string).collect();
        what.push_str(&format!(" Pull requests {}.", list.join(", ")));
    }
    if *files_changed > 0 {
        what.push_str(&format!(" {}.", files_word(*files_changed)));
    }
    if !results.is_empty() {
        let list: Vec<String> = results.iter().map(|p| clip(p, 160)).collect();
        what.push_str(&format!(" Results: {}.", list.join(", ")));
    }
    let cost = match (cost_usd, cost_partial) {
        (None, _) => "unknown".to_string(),
        (Some(c), true) => format!("at least ${c:.2}"),
        (Some(c), false) => format!("${c:.2}"),
    };
    let lines = vec![
        format!("What: {}", clip(&what, 900)),
        format!("Why: {}", clip(&head.why, 200)),
        checked_line(checks),
        String::new(),
        format!(
            "Working time: {} · Tokens: {tokens} · Cost: {cost}",
            minutes(*working_ms)
        ),
    ];
    finish_body(&build_log_head(head), &lines, session_key, record_id)
}

/// DR1b: a decision as a build log comment: the dated head, then `What:`
/// (the decision and its status), `Why:`, `Rejected:` (left out when the
/// record has none, never an empty line) and `Verified:`.
pub(crate) fn render_decision(
    event: &WorkEvent,
    at: &str,
    session_key: &str,
    record_id: &str,
) -> String {
    let WorkEvent::Decision {
        change,
        what,
        status,
        why,
        rejected,
        verified,
    } = event
    else {
        return String::new();
    };
    let head = format!(
        "Build log, {}, Decision {}",
        clip(&local_stamp(at), 40),
        clip(change, 40)
    );
    let mut lines = vec![format!("What: {} ({})", clip(what, 500), clip(status, 20))];
    let said = |v: &Option<String>, none: &str| {
        v.as_deref()
            .map(|v| clip(v, 400))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| none.to_string())
    };
    lines.push(format!("Why: {}", said(why, "not recorded")));
    let rejected = clip(rejected, 700);
    if !rejected.is_empty() {
        lines.push(format!("Rejected: {rejected}"));
    }
    lines.push(format!("Verified: {}", said(verified, "no check recorded")));
    finish_body(&head, &lines, session_key, record_id)
}

/// The newest run linked to the issue `key` (`ENG-12`, any case) that may
/// carry a comment for it: not a helper, not a copy. None: the issue is not
/// one Hollow has work on, so a decision naming it makes nothing (DR1b).
pub(crate) fn link_for_issue_key(vault: &Path, key: &str) -> Option<WorkLink> {
    session_store::with_read(vault, |conn| {
        conn.query_row(
            &format!(
                "SELECT {LINK_COLUMNS} FROM work_links WHERE issue_key = ?1 COLLATE NOCASE \
                 AND issue_id IS NOT NULL AND depth < 2 \
                 AND linked_by NOT IN ('waiting', 'commitment') ORDER BY at DESC LIMIT 1"
            ),
            params![key.trim()],
            link_row,
        )
        .optional()
        .map_err(|e| format!("work link: {e}"))
    })
    .ok()
    .flatten()
}

/// T3b and T3e: how a Waiting on you item or a commitment ended (by the
/// link's `linked_by`), before its issue is closed.
pub(crate) fn render_resolved(event: &WorkEvent, linked_by: &str, record_id: &str) -> String {
    let WorkEvent::Resolved { answer } = event else {
        return String::new();
    };
    let (head, what) = if linked_by == linked_by::COMMITMENT {
        ("Hollow: commitment closed", "commitment")
    } else {
        ("Hollow: no longer waiting on you", "item")
    };
    let text = crate::claude_runner::redact(&format!(
        "{head}\n\n{}\n\nThis issue was Hollow's copy of the {what}; Hollow closes it now.",
        clip(answer, 300)
    ));
    format!("{text}\n\nHollow record {record_id}")
}

/// One progress note for [`render_progress`]: its line, link and checks.
pub(crate) type Note = (String, Option<String>, Vec<String>);

/// T3g: one build log comment from the progress notes on an issue since the
/// last one (newest last): `What:` the notes, `Why:` the issue, `Checked:`
/// the follow-up turns' checks. `head.who` names the runs that wrote them.
pub(crate) fn render_progress(
    notes: &[Note],
    head: &LogHead,
    session_key: &str,
    record_id: &str,
) -> String {
    let start = notes.len().saturating_sub(10);
    let mut checks = Vec::new();
    for (_, _, more) in &notes[start..] {
        add_checks(&mut checks, more);
    }
    let title = build_log_head(head);
    let why = format!("Why: {}", clip(&head.why, 200));
    let checked = checked_line(&checks);
    // The newest notes that fit, so `Why:` and `Checked:` are never cut.
    let mut room = COMMENT_MAX_BYTES
        .saturating_sub(title.len() + why.len() + checked.len() + session_key.len() + 120);
    let mut kept = Vec::new();
    for (line, link, _) in notes[start..].iter().rev() {
        let text = match link {
            Some(link) => format!("- {} ({})", clip(line, 280), clip(link, 300)),
            None => format!("- {}", clip(line, 280)),
        };
        if text.len() + 1 > room && !kept.is_empty() {
            break;
        }
        room = room.saturating_sub(text.len() + 1);
        kept.push(text);
    }
    kept.reverse();
    let mut lines = vec!["What:".to_string()];
    lines.extend(kept);
    lines.push(why);
    lines.push(checked);
    finish_body(&title, &lines, session_key, record_id)
}

// ── Reads (IPC) ─────────────────────────────────────────────────────────────

/// A subagent row's issue (lane 1's chip and header).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkIssueRef {
    pub id: String,
    pub key: String,
    pub title: Option<String>,
    pub state_name: Option<String>,
    pub state_kind: Option<StateKind>,
}

/// One run tied to an issue (lane 3's "Agent work" rows).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct WorkRun {
    pub session_key: String,
    pub title: Option<String>,
    pub agent: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// `running`, `queued`, `done`, `failed` or `stopped`.
    #[cfg_attr(test, ts(as = "crate::ts_bindings::SubagentStatus"))]
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    #[cfg_attr(test, ts(type = "number"))]
    pub working_ms: i64,
    #[cfg_attr(test, ts(type = "number"))]
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    /// `packet`, `inherited`, `task`, `matched`, `created` or `person`.
    pub linked_by: String,
    /// G4: one of its runs counts as accepted for hours saved (this issue
    /// reached Done after the run started, or a Result it wrote was
    /// accepted).
    pub accepted: bool,
}

/// `tracker_issue_work`: the runs tied to an issue and its Hollow audit
/// record (newest first).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerIssueWork {
    pub runs: Vec<WorkRun>,
    pub audit: Vec<crate::audit::AuditEntry>,
}

/// How many audit rows the issue detail shows.
const ISSUE_AUDIT_LIMIT: usize = 200;

/// The runs tied to `issue_id` (an id or a key), newest first.
pub(crate) fn issue_work(vault: &Path, issue_id: &str, now: DateTime<Utc>) -> TrackerIssueWork {
    let issue_id = issue_id.trim();
    let mut links = links_for_issue(vault, issue_id);
    if links.is_empty() {
        links = session_store::with_read(vault, |conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {LINK_COLUMNS} FROM work_links WHERE issue_key = ?1 COLLATE NOCASE ORDER BY at DESC"
                ))
                .map_err(|e| format!("work links: {e}"))?;
            let rows = stmt
                .query_map(params![issue_id], link_row)
                .map_err(|e| format!("work links: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("work links: {e}"));
            rows
        })
        .unwrap_or_default();
    }
    let alias = links
        .iter()
        .find(|l| l.issue_id.is_some())
        .map(|l| (l.issue_id.clone(), l.issue_key.clone()))
        .unwrap_or_default();
    let keys: Vec<String> = links.iter().map(|l| l.session_key.clone()).collect();
    let session_runs = runs_of_sessions(vault, &keys);
    let accepted_runs = if session_runs.is_empty() {
        HashMap::new()
    } else {
        crate::run_ledger::acceptance_labels(vault)
    };
    let runs = links
        .into_iter()
        .filter_map(|link| {
            let row = crate::subagents::row_of(vault, &link.session_key, now).ok()??;
            let accepted = session_runs.get(&link.session_key).is_some_and(|ids| {
                ids.iter()
                    .any(|id| accepted_runs.get(id) == Some(&"accepted"))
            });
            Some(WorkRun {
                session_key: link.session_key,
                title: row.title,
                agent: row.agent,
                model: row.model,
                effort: row.effort,
                status: row.status,
                started_at: row.started_at,
                ended_at: row.ended_at,
                working_ms: row.working_ms,
                tokens: row.tokens,
                cost_usd: row.cost_usd,
                linked_by: link.linked_by,
                accepted,
            })
        })
        .collect();
    // The issue's rows by the name asked and by its link's other name (an
    // id asked finds rows that only know the key, and the other way round).
    let mut names = vec![issue_id.to_string()];
    for name in [&alias.0, &alias.1].into_iter().flatten() {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            names.push(name.clone());
        }
    }
    let mut audit: Vec<crate::audit::AuditEntry> = Vec::new();
    for name in names {
        let rows = crate::audit::query(
            vault,
            &crate::audit::AuditFilter {
                issue: Some(name),
                limit: Some(ISSUE_AUDIT_LIMIT),
                ..crate::audit::AuditFilter::default()
            },
        )
        .unwrap_or_else(|error| {
            eprintln!("[tracker work] audit read: {error}");
            Vec::new()
        });
        for row in rows {
            if !audit.iter().any(|a| a.id == row.id) {
                audit.push(row);
            }
        }
    }
    // Ids are time-ordered: newest first.
    audit.sort_by(|a, b| b.id.cmp(&a.id));
    audit.truncate(ISSUE_AUDIT_LIMIT);
    TrackerIssueWork { runs, audit }
}

type MirrorCache = Option<(PathBuf, std::time::SystemTime, u64, std::sync::Arc<Mirror>)>;
static MIRROR_CACHE: once_cell::sync::Lazy<Mutex<MirrorCache>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

/// The ledger's mirror for display, parsed again only when the file changed
/// (its modified time and size), so a Subagents read doesn't parse it each time.
fn cached_mirror(vault: &Path) -> Option<std::sync::Arc<Mirror>> {
    let path = vault.join(crate::tracker_sync::STATE);
    let meta = std::fs::metadata(&path).ok()?;
    let stamp = (meta.modified().ok()?, meta.len());
    let mut cache = MIRROR_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((cached, at, len, mirror)) = cache.as_ref() {
        if *cached == path && (*at, *len) == stamp {
            return Some(mirror.clone());
        }
    }
    let mirror = std::sync::Arc::new(Mirror::of(&read_ledger_at(vault)?));
    *cache = Some((path, stamp.0, stamp.1, mirror.clone()));
    Some(mirror)
}

/// The issue each of `keys` is tied to, from one links read and one ledger
/// read. Runs whose create is still pending have none yet.
pub(crate) fn issue_refs(vault: &Path, keys: &[String]) -> HashMap<String, WorkIssueRef> {
    let mut out = HashMap::new();
    if keys.is_empty() {
        return out;
    }
    let marks = vec!["?"; keys.len()].join(",");
    let links: Vec<WorkLink> = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {LINK_COLUMNS} FROM work_links WHERE session_key IN ({marks}) AND issue_id IS NOT NULL"
            ))
            .map_err(|e| format!("work links: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(keys.iter()), link_row)
            .map_err(|e| format!("work links: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("work links: {e}"));
        rows
    })
    .unwrap_or_default();
    if links.is_empty() {
        return out;
    }
    let mirror = cached_mirror(vault);
    for link in links {
        let (Some(id), Some(key)) = (link.issue_id.clone(), link.issue_key.clone()) else {
            continue;
        };
        let issue = mirror.as_ref().and_then(|m| m.find(&id));
        out.insert(
            link.session_key,
            WorkIssueRef {
                id,
                key,
                title: issue.map(|i| i.title.clone()),
                state_name: issue.map(|i| i.state.name.clone()),
                state_kind: issue.map(|i| i.state.kind),
            },
        );
    }
    out
}

// ── Accepted through the issue (G4) ────────────────────────────────────────
//
// A run counts as accepted for hours saved when its tracker issue reached
// Done (state kind `completed`) after the run started, or when a Result it
// wrote was accepted (`run_ledger`). Read from the local mirror on every
// value read, so an issue reopened later stops counting. An issue mirrored
// before `completed_at` was read counts once its next sync fills it in.

/// One turn of a linked session: a run tied to an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkedRun {
    pub run_id: String,
    pub started_at: String,
    pub issue_id: String,
}

/// Every run with a history run id whose session is tied to an issue.
pub(crate) fn linked_runs(vault: &Path) -> Vec<LinkedRun> {
    session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT t.run_id, t.started_at, w.issue_id FROM turns t \
                 JOIN work_links w ON w.session_key = t.session_key \
                 WHERE t.run_id IS NOT NULL AND w.issue_id IS NOT NULL",
            )
            .map_err(|e| format!("linked runs: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LinkedRun {
                    run_id: row.get(0)?,
                    started_at: row.get(1)?,
                    issue_id: row.get(2)?,
                })
            })
            .map_err(|e| format!("linked runs: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("linked runs: {e}"));
        rows
    })
    .unwrap_or_default()
}

/// The history run ids of each of `keys`' turns.
fn runs_of_sessions(vault: &Path, keys: &[String]) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    if keys.is_empty() {
        return out;
    }
    let marks = vec!["?"; keys.len()].join(",");
    let rows: Vec<(String, String)> = session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT session_key, run_id FROM turns WHERE session_key IN ({marks}) AND run_id IS NOT NULL"
            ))
            .map_err(|e| format!("session runs: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(keys.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| format!("session runs: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("session runs: {e}"));
        rows
    })
    .unwrap_or_default();
    for (key, run_id) in rows {
        out.entry(key).or_default().push(run_id);
    }
    out
}

/// The scoped team's issues that are Done now, by id, with when they were
/// completed. Archived issues still count (a tracker archives Done work);
/// trashed and remotely deleted ones don't.
pub(crate) fn done_issues(state: &crate::tracker_sync::Ledger) -> HashMap<String, Option<String>> {
    state
        .records
        .values()
        .filter(|r| !r.remote_missing)
        .filter_map(|r| r.issue.as_ref())
        .filter(|i| !i.trashed && i.team_id == state.scope && i.state.kind.is_done())
        .map(|i| (i.id.clone(), i.completed_at.clone()))
        .collect()
}

/// The runs accepted through their issue: it is Done, and it was completed
/// after the run started. A Done issue with no completion time yet counts
/// nothing (never guessed from `updated_at`).
pub(crate) fn accepted_by_issue_from(
    runs: &[LinkedRun],
    done: &HashMap<String, Option<String>>,
) -> BTreeSet<String> {
    let at = |raw: &str| DateTime::parse_from_rfc3339(raw.trim()).ok();
    runs.iter()
        .filter(|run| {
            let Some(Some(completed)) = done.get(&run.issue_id) else {
                return false;
            };
            matches!((at(completed), at(&run.started_at)), (Some(c), Some(s)) if c > s)
        })
        .map(|run| run.run_id.clone())
        .collect()
}

/// [`accepted_by_issue_from`] over this vault's links and mirror.
pub(crate) fn accepted_by_issue(vault: &Path) -> BTreeSet<String> {
    let Some(state) = read_ledger_at(vault) else {
        return BTreeSet::new();
    };
    let done = done_issues(&state);
    if done.is_empty() {
        return BTreeSet::new();
    }
    accepted_by_issue_from(&linked_runs(vault), &done)
}

#[cfg(test)]
#[path = "work_tests.rs"]
pub(crate) mod tests;
