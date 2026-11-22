//! Host-owned request provenance, independent of queue priority and permissions.
//! Records survive queue pruning. Missing legacy records are Unknown, never
//! inferred from Manual, a payload, a role or a standing approval. This module
//! records origin only: it grants no budget, tool permission or launch authority.
use crate::run_queue::{NewRow, QueueRow, RunSource};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::path::Path;

pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS work_origin_requests (
 request_id TEXT PRIMARY KEY, class TEXT NOT NULL, authority TEXT NOT NULL,
 source TEXT NOT NULL, scope TEXT NOT NULL, requested_at TEXT NOT NULL,
 event_key TEXT
);
CREATE TABLE IF NOT EXISTS work_origin_queue (
 queue_id INTEGER PRIMARY KEY, root_request_id TEXT NOT NULL,
 parent_run_id TEXT, source TEXT NOT NULL, scope TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS work_origin_occurrences (
 request_id TEXT NOT NULL, queue_id INTEGER NOT NULL, PRIMARY KEY(request_id,queue_id)
);
CREATE TABLE IF NOT EXISTS work_origin_runs (
 run_id TEXT PRIMARY KEY, queue_id INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_work_origin_runs_queue ON work_origin_runs(queue_id);
CREATE TABLE IF NOT EXISTS work_origin_absorptions (
 sibling_queue_id INTEGER PRIMARY KEY, admitted_queue_id INTEGER NOT NULL,
 scope TEXT NOT NULL, cron_due TEXT
);
CREATE TABLE IF NOT EXISTS work_origin_terminal (
 queue_id INTEGER PRIMARY KEY, status TEXT NOT NULL, history_run_id TEXT,
 finished_at TEXT NOT NULL
);
";

const REFUSED: &str = "WORK_ORIGIN: request provenance could not be verified";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Direct,
    Autonomous,
    Unknown,
}

/// Rust-only intent supplied by a specific host entry point. Never deserialize
/// this from a model, frontend field, queue payload or environment variable.
///
/// It once carried a second field, a restricted-workflow allocation pin.
/// Nothing captured one after the company's restricted workflows were removed
/// (11-10, L11), so it went (11-10 sweep). A database that still holds
/// `work_origin_allocation_pins` rows opens as before; nothing reads them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Request(Kind);

#[derive(Debug, Clone, Default)]
enum Kind {
    #[default]
    Unknown,
    PersonWorkflow(String),
    SchedulerWorkflow,
    Caller(crate::app_actions::Caller),
    Retry {
        run_id: String,
        queue_id: Option<i64>,
    },
}

impl Request {
    /// Called only by the person's workflow command. Each invocation creates a
    /// distinct request; cloning this token preserves exact retry identity.
    pub(crate) fn person_workflow() -> Self {
        Self(Kind::PersonWorkflow(format!(
            "person-{:032x}",
            rand::random::<u128>()
        )))
    }

    /// Called by the verified scheduler enqueue path, not inferred at admission.
    pub(crate) fn scheduled_workflow() -> Self {
        Self(Kind::SchedulerWorkflow)
    }

    /// The broker has authenticated Caller. Check actual liveness here, then
    /// recheck its exact queue/run binding inside the enqueue transaction. A
    /// live caller without retained provenance inherits Unknown.
    pub(crate) fn caller(
        vault: &Path,
        caller: &crate::app_actions::Caller,
    ) -> Result<Self, String> {
        if !matches!(caller.surface.as_str(), "chat" | "workflow")
            || !crate::automated_history::run_is_live(vault, &caller.run_id)
        {
            return Err(REFUSED.into());
        }
        crate::session_store::with_read(vault, |conn| caller_origin(conn, caller).map(|_| ()))?;
        Ok(Self(Kind::Caller(caller.clone())))
    }

    pub(crate) fn retry(run_id: &str, queue_id: Option<i64>) -> Self {
        Self(Kind::Retry {
            run_id: run_id.into(),
            queue_id,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    id: String,
    class: Class,
    authority: String,
    source: String,
    scope: String,
    requested_at: String,
    event_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    root: Root,
    queue_id: i64,
    parent_run_id: Option<String>,
    source: String,
    scope: String,
}

impl Binding {
    pub(crate) fn class(&self) -> Class {
        self.root.class
    }
    pub(crate) fn request_id(&self) -> &str {
        &self.root.id
    }
    pub(crate) fn queue_id(&self) -> i64 {
        self.queue_id
    }
}

fn error(error: rusqlite::Error) -> String {
    format!("work origin: {error}")
}

struct RootRecord {
    class: String,
    authority: String,
    source: String,
    scope: String,
    requested_at: String,
    event_key: Option<String>,
}

fn read_root(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<RootRecord> {
    Ok(RootRecord {
        class: row.get(offset)?,
        authority: row.get(offset + 1)?,
        source: row.get(offset + 2)?,
        scope: row.get(offset + 3)?,
        requested_at: row.get(offset + 4)?,
        event_key: row.get(offset + 5)?,
    })
}

fn validate_root(id: &str, row: RootRecord) -> Result<Root, String> {
    let class = match (
        row.class.as_str(),
        row.authority.as_str(),
        row.source.as_str(),
    ) {
        ("direct", "personWorkflow", "manual") if row.event_key.is_none() => Class::Direct,
        ("autonomous", "schedulerWorkflow", "cron" | "trigger")
            if row.event_key.as_deref().is_some_and(|s| !s.is_empty()) =>
        {
            Class::Autonomous
        }
        _ => return Err(REFUSED.into()),
    };
    if id.is_empty()
        || !crate::budgets::valid_workflow_slug(&row.scope)
        || DateTime::parse_from_rfc3339(&row.requested_at).is_err()
    {
        return Err(REFUSED.into());
    }
    Ok(Root {
        id: id.into(),
        class,
        authority: row.authority,
        source: row.source,
        scope: row.scope,
        requested_at: row.requested_at,
        event_key: row.event_key,
    })
}

fn root_in(conn: &Connection, id: &str) -> Result<Root, String> {
    let record = conn.query_row(
        "SELECT class,authority,source,scope,requested_at,event_key FROM work_origin_requests WHERE request_id=?1",
        [id], |r| read_root(r, 0),
    ).optional().map_err(error)?.ok_or(REFUSED)?;
    validate_root(id, record)
}

fn validate_binding(
    queue_id: i64,
    root: Root,
    parent_run_id: Option<String>,
    source: String,
    scope: String,
    parent_root: Option<String>,
) -> Result<Binding, String> {
    if queue_id <= 0
        || scope.is_empty()
        || !matches!(
            source.as_str(),
            "manual" | "cron" | "trigger" | "retry" | "chat" | "restrictedResume"
        )
    {
        return Err(REFUSED.into());
    }
    if parent_run_id.is_some() {
        if parent_root.as_deref() != Some(root.id.as_str()) {
            return Err(REFUSED.into());
        }
    } else if root.source != source || root.scope != scope {
        return Err(REFUSED.into());
    }
    Ok(Binding {
        root,
        queue_id,
        parent_run_id,
        source,
        scope,
    })
}

fn binding_in(conn: &Connection, queue_id: i64) -> Result<Option<Binding>, String> {
    let raw = conn.query_row(
        "SELECT root_request_id,parent_run_id,source,scope FROM work_origin_queue WHERE queue_id=?1",
        [queue_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
    ).optional().map_err(error)?;
    raw.map(|(id, parent_run_id, source, scope)| {
        let root = root_in(conn, &id)?;
        let parent_root = if let Some(parent) = parent_run_id.as_deref() {
            conn.query_row(
                "SELECT q.root_request_id FROM work_origin_runs r JOIN work_origin_queue q ON q.queue_id=r.queue_id WHERE r.run_id=?1",
                [parent], |r| r.get::<_, String>(0),
            ).optional().map_err(error)?
        } else { None };
        validate_binding(queue_id, root, parent_run_id, source, scope, parent_root)
    }).transpose()
}

/// Advisory scheduling only. One bulk lookup, sharing the authority validators
/// above; missing or damaged records never promote a row or block ordinary chat.
/// The caller holds a read snapshot and releases it before History/eligibility.
pub(crate) fn direct_workflows_in(
    conn: &Connection,
    rows: &[QueueRow],
) -> std::collections::HashSet<i64> {
    let candidates: std::collections::HashMap<_, _> = rows
        .iter()
        .filter(|r| r.status == crate::run_queue::RowStatus::Queued && r.source.is_workflow())
        .map(|r| (r.id, r))
        .collect();
    if candidates.is_empty() {
        return std::collections::HashSet::new();
    }
    let read = || -> Result<std::collections::HashSet<i64>, String> {
        let mut statement = conn
            .prepare(
                "SELECT o.queue_id,o.root_request_id,o.parent_run_id,o.source,o.scope, \
             r.class,r.authority,r.source,r.scope,r.requested_at,r.event_key,p.root_request_id \
             FROM run_queue q JOIN work_origin_queue o ON o.queue_id=q.id \
             JOIN work_origin_requests r ON r.request_id=o.root_request_id \
             LEFT JOIN work_origin_runs pr ON pr.run_id=o.parent_run_id \
             LEFT JOIN work_origin_queue p ON p.queue_id=pr.queue_id \
             WHERE q.status='queued' AND q.source IN ('manual','cron','trigger','retry')",
            )
            .map_err(error)?;
        let records = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    read_root(r, 5)?,
                    r.get::<_, Option<String>>(11)?,
                ))
            })
            .map_err(error)?;
        let mut direct = std::collections::HashSet::new();
        for (id, root_id, parent, source, scope, record, parent_root) in records.flatten() {
            let Some(row) = candidates.get(&id) else {
                continue;
            };
            let Ok(root) = validate_root(&root_id, record) else {
                continue;
            };
            let Ok(binding) = validate_binding(id, root, parent, source, scope, parent_root) else {
                continue;
            };
            if binding.class() == Class::Direct
                && binding.source == row.source.as_str()
                && Some(binding.scope.as_str()) == row.source_id.as_deref()
            {
                direct.insert(id);
            }
        }
        Ok(direct)
    };
    read().unwrap_or_else(|error| {
        eprintln!("[run queue] direct-request ordering unavailable: {error}");
        std::collections::HashSet::new()
    })
}

/// Missing means Unknown. Callers must not synthesize a stronger classification.
pub(crate) fn for_queue_in(conn: &Connection, row: &QueueRow) -> Result<Option<Binding>, String> {
    let binding = binding_in(conn, row.id)?;
    if binding.as_ref().is_some_and(|b| {
        b.source != row.source.as_str() || Some(b.scope.as_str()) != row.source_id.as_deref()
    }) {
        return Err(REFUSED.into());
    }
    Ok(binding)
}

pub(crate) fn for_run_in(conn: &Connection, run_id: &str) -> Result<Option<Binding>, String> {
    let queue_id: Option<i64> = conn
        .query_row(
            "SELECT queue_id FROM work_origin_runs WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    queue_id
        .map(|id| binding_in(conn, id)?.ok_or_else(|| REFUSED.to_string()))
        .transpose()
}

fn caller_origin(
    conn: &Connection,
    caller: &crate::app_actions::Caller,
) -> Result<Option<Binding>, String> {
    let binding = for_run_in(conn, &caller.run_id)?;
    let mut stmt = conn.prepare("SELECT id,source,source_id,status,finished_at,cancel_requested FROM run_queue WHERE history_run_id=?1 LIMIT 2").map_err(error)?;
    let rows = stmt
        .query_map([&caller.run_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(error)?;
    if rows.is_empty() && binding.is_none() {
        // The broker already proved a live run. Legacy callers without any
        // queue/origin record keep their existing permission path as Unknown.
        return Ok(None);
    }
    let [(id, source, scope, status, finished, cancelled)] = rows.as_slice() else {
        return Err(REFUSED.into());
    };
    if scope.as_deref() != Some(caller.scope.as_str())
        || status != "running"
        || finished.is_some()
        || *cancelled != 0
    {
        return Err(REFUSED.into());
    }
    let matched = match caller.surface.as_str() {
        "chat" => source == "chat",
        "workflow" => matches!(source.as_str(), "manual" | "cron" | "trigger" | "retry"),
        _ => false,
    };
    if !matched {
        return Err(REFUSED.into());
    }
    if binding
        .as_ref()
        .is_some_and(|b| b.queue_id != *id || b.source != *source || b.scope != caller.scope)
    {
        return Err(REFUSED.into());
    }
    // A binding on the row without its exact run binding is damaged, not legacy.
    if binding.is_none() && binding_in(conn, *id)?.is_some() {
        return Err(REFUSED.into());
    }
    Ok(binding)
}

pub(crate) struct Prepared {
    root: Option<Root>,
    parent_run_id: Option<String>,
    scheduler: bool,
}

pub(crate) fn prepare_in(
    conn: &Connection,
    row: &NewRow,
    now: DateTime<Utc>,
) -> Result<Prepared, String> {
    let mut prepared = Prepared {
        root: None,
        parent_run_id: None,
        scheduler: false,
    };
    let scope = row.source_id.as_deref().unwrap_or("");
    match &row.origin.0 {
        Kind::Unknown => {}
        Kind::PersonWorkflow(id) => {
            if row.source != RunSource::Manual || !crate::budgets::valid_workflow_slug(scope) {
                return Err(REFUSED.into());
            }
            prepared.root = Some(Root {
                id: id.clone(),
                class: Class::Direct,
                authority: "personWorkflow".into(),
                source: "manual".into(),
                scope: scope.into(),
                requested_at: crate::session_store::db_time(now),
                event_key: None,
            });
        }
        Kind::SchedulerWorkflow => {
            use sha2::{Digest, Sha256};
            if !matches!(row.source, RunSource::Cron | RunSource::Trigger)
                || !crate::budgets::valid_workflow_slug(scope)
            {
                return Err(REFUSED.into());
            }
            let key = row
                .idempotency_key
                .as_deref()
                .filter(|key| !key.is_empty())
                .ok_or(REFUSED)?;
            let hash = Sha256::digest(
                serde_json::to_vec(&(row.source.as_str(), scope, key))
                    .map_err(|e| e.to_string())?,
            );
            prepared.root = Some(Root {
                id: format!("schedule-{hash:x}"),
                class: Class::Autonomous,
                authority: "schedulerWorkflow".into(),
                source: row.source.as_str().into(),
                scope: scope.into(),
                requested_at: crate::session_store::db_time(now),
                event_key: Some(key.into()),
            });
            prepared.scheduler = true;
        }
        Kind::Caller(caller) => {
            prepared.root = caller_origin(conn, caller)?.map(|b| b.root);
            prepared.parent_run_id = prepared.root.as_ref().map(|_| caller.run_id.clone());
        }
        Kind::Retry { run_id, queue_id } => {
            if row.source != RunSource::Retry {
                return Err(REFUSED.into());
            }
            if let Some(parent) = for_run_in(conn, run_id)? {
                if Some(parent.queue_id()) != *queue_id
                    || parent.scope != scope
                    || !matches!(
                        parent.source.as_str(),
                        "manual" | "cron" | "trigger" | "retry"
                    )
                {
                    return Err(REFUSED.into());
                }
                prepared.root = Some(parent.root);
                prepared.parent_run_id = Some(run_id.clone());
            }
        }
    }
    Ok(prepared)
}

pub(crate) fn compatible_in(
    conn: &Connection,
    queue_id: i64,
    incoming: &Prepared,
) -> Result<bool, String> {
    let existing = binding_in(conn, queue_id)?;
    let existing_class = existing.as_ref().map_or(Class::Unknown, Binding::class);
    let incoming_class = incoming
        .root
        .as_ref()
        .map_or(Class::Unknown, |root| root.class);
    if existing_class != incoming_class {
        return Ok(false);
    }
    Ok(match (existing, incoming.root.as_ref()) {
        (None, None) => true,
        (Some(existing), Some(root)) => {
            (existing.request_id() == root.id && existing.parent_run_id == incoming.parent_run_id)
                || (incoming.scheduler
                    && existing.parent_run_id.is_none()
                    && existing.root.class == Class::Autonomous
                    && existing.root.authority == root.authority
                    && existing.root.source == root.source
                    && existing.root.scope == root.scope)
        }
        _ => false,
    })
}

/// Called in the same transaction as insertion/coalescing. Retain each scheduler
/// occurrence even when one queued execution represents several occurrences.
pub(crate) fn enqueued_in(
    tx: &Transaction<'_>,
    row: &NewRow,
    id: i64,
    prepared: &Prepared,
) -> Result<(), String> {
    let Some(root) = &prepared.root else {
        return Ok(());
    };
    let class = match root.class {
        Class::Direct => "direct",
        Class::Autonomous => "autonomous",
        Class::Unknown => return Err(REFUSED.into()),
    };
    tx.execute("INSERT OR IGNORE INTO work_origin_requests(request_id,class,authority,source,scope,requested_at,event_key) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![root.id,class,root.authority,root.source,root.scope,root.requested_at,root.event_key]).map_err(error)?;
    let saved = root_in(tx, &root.id)?;
    if saved.class != root.class
        || saved.authority != root.authority
        || saved.source != root.source
        || saved.scope != root.scope
        || saved.event_key != root.event_key
    {
        return Err(REFUSED.into());
    }
    tx.execute("INSERT OR IGNORE INTO work_origin_queue(queue_id,root_request_id,parent_run_id,source,scope) VALUES(?1,?2,?3,?4,?5)", params![id,root.id,prepared.parent_run_id,row.source.as_str(),row.source_id]).map_err(error)?;
    // Inherited requests share a root, not a new occurrence of that root.
    if prepared.parent_run_id.is_none() {
        if prepared.scheduler && row.source == RunSource::Cron {
            if let Some(due) = crate::run_queue::due_at(&row.payload) {
                let consumed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM automation_occurrences WHERE slug=?1 AND due_at=?2)", params![row.source_id,due], |r| r.get(0)).map_err(error)?;
                if consumed {
                    return Err(
                        "WORK_ORIGIN: this scheduled occurrence was already consumed".into(),
                    );
                }
            }
        }
        let mut stmt = tx
            .prepare(
                "SELECT queue_id FROM work_origin_occurrences WHERE request_id=?1 AND queue_id!=?2",
            )
            .map_err(error)?;
        let previous = stmt
            .query_map(params![root.id, id], |r| r.get::<_, i64>(0))
            .map_err(error)?;
        for prior in previous {
            let prior = prior.map_err(error)?;
            if !prepared.scheduler || !unconsumed_terminal_in(tx, prior)? {
                return Err("WORK_ORIGIN: this request already belongs to an active or consumed queue attempt".into());
            }
        }
        tx.execute(
            "INSERT OR IGNORE INTO work_origin_occurrences(request_id,queue_id) VALUES(?1,?2)",
            params![root.id, id],
        )
        .map_err(error)?;
    }
    Ok(())
}

fn unconsumed_terminal_in(conn: &Connection, queue_id: i64) -> Result<bool, String> {
    let terminal: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT status,history_run_id FROM work_origin_terminal WHERE queue_id=?1",
            [queue_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(error)?;
    let Some((status, run_id)) = terminal else {
        return Ok(false);
    };
    let current: Option<String> = conn
        .query_row(
            "SELECT status FROM run_queue WHERE id=?1",
            [queue_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?;
    if current.as_deref().is_some_and(|current| current != status) {
        return Err(REFUSED.into());
    }
    if let Some(run_id) = run_id.as_deref() {
        if for_run_in(conn, run_id)?.is_none_or(|binding| binding.queue_id() != queue_id) {
            return Err(REFUSED.into());
        }
    }
    // Coalesced work belongs to its receiving attempt, not to an unstarted
    // standalone row. Only that attempt's retained interruption frees it.
    if status == "coalesced" {
        let parent: Option<i64> = conn
            .query_row(
                "SELECT admitted_queue_id FROM work_origin_absorptions WHERE sibling_queue_id=?1",
                [queue_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        let Some(parent) = parent else {
            return Ok(false);
        };
        let terminal: Option<String> = conn
            .query_row(
                "SELECT status FROM work_origin_terminal WHERE queue_id=?1",
                [parent],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        return Ok(terminal.as_deref() == Some("interrupted"));
    }
    Ok(status == "interrupted"
        || (run_id.is_none() && matches!(status.as_str(), "failed" | "cancelled" | "skipped")))
}

/// Retain terminal evidence before the queue's normal pruning. No replay is
/// authorized here; the scheduler must independently decide the event is due.
pub(crate) fn terminal_in(tx: &Transaction<'_>, queue_id: i64) -> Result<(), String> {
    tx.execute("INSERT INTO work_origin_terminal(queue_id,status,history_run_id,finished_at) SELECT q.id,q.status,q.history_run_id,q.finished_at FROM run_queue q JOIN work_origin_queue o ON o.queue_id=q.id WHERE q.id=?1 AND q.finished_at IS NOT NULL ON CONFLICT(queue_id) DO NOTHING", [queue_id]).map_err(error)?;
    let Some(bound) = binding_in(tx, queue_id)? else {
        return Ok(());
    };
    let row: Option<(String, String, String, String)> = tx.query_row(
        "SELECT source_id,history_run_id,status,finished_at FROM run_queue WHERE id=?1 AND status IN ('done','failed','cancelled') AND history_run_id IS NOT NULL AND finished_at IS NOT NULL",
        [queue_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).optional().map_err(error)?;
    let Some((scope, run_id, status, finished_at)) = row else {
        return Ok(());
    };
    if for_run_in(tx, &run_id)?.as_ref() != Some(&bound) {
        return Err(REFUSED.into());
    }
    // Absorption is an immutable host fact, retained independently of the
    // sibling queue row. A failed workflow-file write must not replay it, nor
    // leave its still-due schedule permanently unable to heal after pruning.
    for due in absorbed_cron_in(tx, &bound, &scope)? {
        tx.execute("INSERT OR IGNORE INTO automation_occurrences(slug,due_at,status,finished_at) VALUES(?1,?2,?3,?4)", params![scope,due,status,finished_at]).map_err(error)?;
    }
    Ok(())
}

pub(crate) fn bind_running_in(
    tx: &Transaction<'_>,
    row: &QueueRow,
    run_id: &str,
) -> Result<(), String> {
    if for_queue_in(tx, row)?.is_none() {
        return Ok(());
    }
    if run_id.trim().is_empty() {
        return Err(REFUSED.into());
    }
    tx.execute(
        "INSERT INTO work_origin_runs(run_id,queue_id) VALUES(?1,?2) ON CONFLICT(run_id) DO NOTHING",
        params![run_id, row.id],
    )
    .map_err(error)?;
    let saved: i64 = tx
        .query_row(
            "SELECT queue_id FROM work_origin_runs WHERE run_id=?1",
            [run_id],
            |r| r.get(0),
        )
        .map_err(error)?;
    if saved != row.id {
        return Err(REFUSED.into());
    }
    Ok(())
}

/// Admission may absorb scheduled siblings, but cannot turn their autonomous
/// work into a direct request or promote an unknown legacy occurrence.
pub(crate) fn absorb_in(
    tx: &Transaction<'_>,
    admitted: &QueueRow,
    sibling: &QueueRow,
) -> Result<bool, String> {
    let compatible = match (for_queue_in(tx, admitted)?, for_queue_in(tx, sibling)?) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.class() == Class::Autonomous
                && b.class() == Class::Autonomous
                && a.root.authority == "schedulerWorkflow"
                && b.root.authority == "schedulerWorkflow"
                && a.root.scope == b.root.scope
                && a.parent_run_id.is_none()
                && b.parent_run_id.is_none()
        }
        _ => false,
    };
    if compatible {
        let scope = sibling.source_id.as_deref().ok_or(REFUSED)?;
        if admitted.source_id.as_deref() != Some(scope) {
            return Err(REFUSED.into());
        }
        let known = binding_in(tx, sibling.id)?.is_some();
        let due = if sibling.source == RunSource::Cron {
            match cron_due_in(tx, sibling.id, scope, known) {
                Ok(due) => Some(due),
                Err(error) if known => return Err(error),
                // Legacy malformed rows keep their old absorption behavior,
                // but supply no positive occurrence-consumption evidence.
                Err(_) => None,
            }
        } else {
            None
        };
        tx.execute(
            "INSERT INTO work_origin_absorptions(sibling_queue_id,admitted_queue_id,scope,cron_due) VALUES(?1,?2,?3,?4)",
            params![sibling.id, admitted.id, scope, due],
        )
        .map_err(error)?;
    }
    Ok(compatible)
}

/// Preserve the old watermark behavior only for legacy Unknown admissions.
/// A proven request cannot consume an unevaluated scheduler event: no durable
/// occurrence exists yet that could establish compatibility with its root.
pub(crate) fn consumes_pending_trigger_in(
    conn: &Connection,
    queue_id: i64,
) -> Result<bool, String> {
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM run_queue WHERE id=?1 AND source='manual' AND status='claimed')", [queue_id], |r| r.get(0)).map_err(error)?;
    if !exists {
        return Err(REFUSED.into());
    }
    Ok(binding_in(conn, queue_id)?.is_none())
}

/// The exact receiving run may advance a schedule only for its own Cron row or
/// a compatible Cron row it durably absorbed. Legacy queue-free tests alone
/// retain unconditional advancement; actual Unknown rows also need an owner.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ScheduleConsumption {
    Legacy,
    Preserve,
    Occurrence(String),
}

fn cron_due_in(conn: &Connection, id: i64, scope: &str, known: bool) -> Result<String, String> {
    let (raw, key): (String, String) = conn.query_row("SELECT payload_json,idempotency_key FROM run_queue WHERE id=?1 AND source='cron' AND source_id=?2", params![id,scope], |r| Ok((r.get(0)?,r.get(1)?))).map_err(error)?;
    let payload = serde_json::from_str(&raw).map_err(|_| REFUSED)?;
    let due = crate::run_queue::due_at(&payload).ok_or(REFUSED)?;
    if DateTime::parse_from_rfc3339(&due).is_err() || key != format!("cron:{scope}:{due}") {
        return Err(REFUSED.into());
    }
    let retained: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM work_origin_occurrences o JOIN work_origin_requests r ON r.request_id=o.request_id WHERE o.queue_id=?1 AND r.event_key=?2 AND r.source='cron' AND r.scope=?3 AND r.class='autonomous' AND r.authority='schedulerWorkflow')", params![id,key,scope], |r| r.get(0)).map_err(error)?;
    if known && !retained {
        return Err(REFUSED.into());
    }
    Ok(due)
}

/// Validate retained absorption against the immutable scheduler occurrence and
/// exact receiving root. No queue row or current workflow schedule is needed.
fn absorbed_cron_in(
    conn: &Connection,
    receiver: &Binding,
    scope: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare("SELECT sibling_queue_id,scope,cron_due FROM work_origin_absorptions WHERE admitted_queue_id=?1 AND cron_due IS NOT NULL ORDER BY sibling_queue_id").map_err(error)?;
    let rows = stmt
        .query_map([receiver.queue_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(error)?;
    let mut dues = Vec::new();
    for row in rows {
        let (id, saved_scope, due) = row.map_err(error)?;
        let sibling = binding_in(conn, id)?.ok_or(REFUSED)?;
        if saved_scope != scope
            || receiver.scope != scope
            || receiver.class() != Class::Autonomous
            || receiver.root.authority != "schedulerWorkflow"
            || receiver.parent_run_id.is_some()
            || sibling.scope != scope
            || sibling.source != "cron"
            || sibling.class() != Class::Autonomous
            || sibling.root.authority != "schedulerWorkflow"
            || sibling.parent_run_id.is_some()
            || DateTime::parse_from_rfc3339(&due).is_err()
        {
            return Err(REFUSED.into());
        }
        let retained: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM work_origin_occurrences o JOIN work_origin_requests r ON r.request_id=o.request_id JOIN work_origin_terminal t ON t.queue_id=o.queue_id WHERE o.queue_id=?1 AND r.event_key=?2 AND r.source='cron' AND r.scope=?3 AND r.class='autonomous' AND r.authority='schedulerWorkflow' AND t.status='coalesced' AND t.history_run_id IS NULL)", params![id,format!("cron:{scope}:{due}"),scope], |r| r.get(0)).map_err(error)?;
        if !retained {
            return Err(REFUSED.into());
        }
        dues.push(due);
    }
    Ok(dues)
}

pub(crate) fn consumes_schedule_in(
    conn: &Connection,
    queue_id: i64,
    run_id: &str,
    scope: &str,
) -> Result<ScheduleConsumption, String> {
    let queue: Option<(String, String)> = conn.query_row(
        "SELECT source,source_id FROM run_queue WHERE id=?1 AND history_run_id=?2 AND status='running' AND finished_at IS NULL",
        params![queue_id, run_id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).optional().map_err(error)?;
    let Some((source, saved_scope)) = queue else {
        return Err(REFUSED.into());
    };
    if saved_scope != scope {
        return Err(REFUSED.into());
    }
    let bound = binding_in(conn, queue_id)?;
    let known = bound.is_some();
    if let Some(ref bound) = bound {
        if bound.source != source
            || bound.scope != scope
            || for_run_in(conn, run_id)?.as_ref() != Some(bound)
        {
            return Err(REFUSED.into());
        }
        if source == "restrictedResume" {
            // A removed company continuation (11-10) consumes nothing.
            return Ok(ScheduleConsumption::Preserve);
        }
        if bound.class() != Class::Autonomous
            || bound.root.authority != "schedulerWorkflow"
            || bound.parent_run_id.is_some()
        {
            return Ok(ScheduleConsumption::Preserve);
        }
    }
    if source == "cron" {
        return cron_due_in(conn, queue_id, scope, known).map(ScheduleConsumption::Occurrence);
    }
    if let Some(bound) = bound {
        return Ok(absorbed_cron_in(conn, &bound, scope)?
            .into_iter()
            .next()
            .map(ScheduleConsumption::Occurrence)
            .unwrap_or(ScheduleConsumption::Preserve));
    }
    let mut stmt = conn
        .prepare("SELECT sibling_queue_id FROM work_origin_absorptions WHERE admitted_queue_id=?1")
        .map_err(error)?;
    let siblings = stmt
        .query_map([queue_id], |r| r.get::<_, i64>(0))
        .map_err(error)?;
    for sibling in siblings {
        let sibling_id = sibling.map_err(error)?;
        let cron: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM run_queue WHERE id=?1 AND coalesced_into=?2 AND status='coalesced' AND source='cron' AND source_id=?3)", params![sibling_id,queue_id,scope], |r| r.get(0)).map_err(error)?;
        if cron {
            match (known, binding_in(conn, sibling_id)?) {
                (false, None) => {}
                (true, Some(sibling))
                    if sibling.source == "cron"
                        && sibling.scope == scope
                        && sibling.class() == Class::Autonomous
                        && sibling.root.authority == "schedulerWorkflow"
                        && sibling.parent_run_id.is_none() => {}
                _ => return Err(REFUSED.into()),
            }
            return cron_due_in(conn, sibling_id, scope, known)
                .map(ScheduleConsumption::Occurrence);
        }
    }
    Ok(ScheduleConsumption::Preserve)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_queue::{self, Enqueued, RowStatus};

    fn manual(scope: &str, origin: Request) -> NewRow {
        NewRow::new(RunSource::Manual, Some(scope), "Workflow", "run now").with_origin(origin)
    }

    fn scheduled(scope: &str, source: RunSource, key: &str) -> NewRow {
        NewRow::new(source, Some(scope), "Workflow", "scheduled")
            .with_key(key)
            .with_origin(Request::scheduled_workflow())
    }

    fn origin(v: &Path, id: i64) -> Option<Binding> {
        let row = run_queue::get(v, id).unwrap().unwrap();
        crate::session_store::with_read(v, |conn| for_queue_in(conn, &row)).unwrap()
    }

    fn next_id(vault: &Path, now: DateTime<Utc>, gap: Option<DateTime<Utc>>) -> i64 {
        match run_queue::next_ready(vault, now, gap).unwrap() {
            run_queue::NextRow::Ready(row) => row.id,
            other => panic!("expected ready row: {other:?}"),
        }
    }

    #[test]
    fn direct_order_agrees_with_list_positions_and_preserves_chat_fifo() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let unknown = run_queue::enqueue(
            v,
            manual("unknown", Request::default())
                .with_payload(serde_json::json!({"origin":"direct", "typed":true})),
            now,
        )
        .unwrap()
        .id();
        let first = run_queue::enqueue(v, manual("first", Request::person_workflow()), now)
            .unwrap()
            .id();
        let second = run_queue::enqueue(v, manual("second", Request::person_workflow()), now)
            .unwrap()
            .id();
        assert_eq!(next_id(v, now, None), first);
        let chat = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Chat, Some("s-first"), "chat", "typed"),
            now,
        )
        .unwrap()
        .id();
        let accepted = run_queue::enqueue_chat(
            v,
            NewRow::new(RunSource::Chat, Some("s-second"), "chat", "typed"),
            &std::collections::HashSet::new(),
            now,
        )
        .unwrap();
        let run_queue::ChatEnqueue::Accepted { id, position } = accepted else {
            panic!("chat accepted");
        };
        assert_eq!(position, 2);
        assert_eq!(next_id(v, now, None), chat);
        let listed: Vec<_> = run_queue::list(v, 0)
            .unwrap()
            .into_iter()
            .map(|(r, p)| (r.id, p))
            .collect();
        assert_eq!(
            listed,
            vec![
                (chat, Some(1)),
                (id, Some(2)),
                (first, Some(3)),
                (second, Some(4)),
                (unknown, Some(5))
            ]
        );
        assert_eq!(
            run_queue::get(v, first).unwrap().unwrap().priority,
            RunSource::Manual.default_priority()
        );
    }

    #[test]
    fn inherited_direct_retry_precedes_unknown_only_after_its_backoff_and_gap() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let parent = run_queue::enqueue(v, manual("parent", Request::person_workflow()), now)
            .unwrap()
            .id();
        run_queue::claim(v, parent, now).unwrap();
        run_queue::mark_running(v, parent, "order-parent", now).unwrap();
        run_queue::finish(v, parent, RowStatus::Failed, None, now).unwrap();
        let unknown = run_queue::enqueue(v, manual("unknown", Request::default()), now)
            .unwrap()
            .id();
        let mut retry = NewRow::new(RunSource::Retry, Some("parent"), "retry", "retry")
            .with_origin(Request::retry("order-parent", Some(parent)));
        retry.not_before = Some(now + chrono::Duration::minutes(2));
        let retry = run_queue::enqueue(v, retry, now).unwrap().id();
        assert_eq!(next_id(v, now, None), unknown);
        let later = now + chrono::Duration::minutes(3);
        assert_eq!(
            next_id(v, later, Some(later + chrono::Duration::seconds(30))),
            unknown
        );
        assert_eq!(next_id(v, later, None), retry);
        assert_eq!(
            run_queue::get(v, retry).unwrap().unwrap().priority,
            RunSource::Retry.default_priority()
        );
        assert_eq!(
            run_queue::next_ready_where(v, later, None, |r| r.id != retry).unwrap(),
            run_queue::NextRow::Ready(run_queue::get(v, unknown).unwrap().unwrap()),
            "a higher-ranked row rejected by the existing eligibility callback cannot block others"
        );
        run_queue::cancel(v, retry, later).unwrap();
        assert_eq!(next_id(v, later, None), unknown);
    }

    #[test]
    fn damaged_provenance_never_promotes_or_blocks_unrelated_chat_and_valid_requests() {
        for damage in ["root", "scope", "parent"] {
            let dir = tempfile::tempdir().unwrap();
            let v = dir.path();
            let now = Utc::now();
            let unknown = run_queue::enqueue(v, manual("unknown", Request::default()), now)
                .unwrap()
                .id();
            let damaged = run_queue::enqueue(v, manual("damaged", Request::person_workflow()), now)
                .unwrap()
                .id();
            let valid = run_queue::enqueue(v, manual("valid", Request::person_workflow()), now)
                .unwrap()
                .id();
            crate::session_store::with_write(v,"damage origin fixture",|tx| {
                match damage {
                    "root" => tx.execute("UPDATE work_origin_requests SET requested_at='invalid' WHERE request_id=(SELECT root_request_id FROM work_origin_queue WHERE queue_id=?1)",[damaged]),
                    "scope" => tx.execute("UPDATE work_origin_queue SET scope='wrong-scope' WHERE queue_id=?1",[damaged]),
                    _ => tx.execute("UPDATE work_origin_queue SET parent_run_id='missing-parent' WHERE queue_id=?1",[damaged]),
                }.map_err(error)?;
                Ok(())
            }).unwrap();
            assert_eq!(next_id(v, now, None), valid);
            let queued: Vec<_> = run_queue::live_rows(v)
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect();
            assert_eq!(queued, vec![valid, unknown, damaged]);
            let accepted = run_queue::enqueue_chat(
                v,
                NewRow::new(RunSource::Chat, Some("s-safe"), "chat", "typed"),
                &std::collections::HashSet::new(),
                now,
            )
            .unwrap();
            let run_queue::ChatEnqueue::Accepted { id, position } = accepted else {
                panic!("chat accepted");
            };
            assert_eq!(position, 1);
            assert_eq!(next_id(v, now, None), id);
        }
    }

    #[test]
    fn person_requests_are_distinct_and_payloads_manual_priority_never_promote_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let forged = manual("brief", Request::default()).with_payload(serde_json::json!({
            "origin":"direct", "typed":true, "caller":"person", "approval":true
        }));
        let unknown = run_queue::enqueue(v, forged, now).unwrap().id();
        assert!(origin(v, unknown).is_none());
        let person = Request::person_workflow();
        let first = run_queue::enqueue(v, manual("brief", person.clone()), now).unwrap();
        let second =
            run_queue::enqueue(v, manual("brief", Request::person_workflow()), now).unwrap();
        assert!(matches!(first, Enqueued::Inserted(_)));
        assert!(matches!(second, Enqueued::Inserted(_)));
        assert_ne!(first.id(), second.id());
        let first_origin = origin(v, first.id()).unwrap();
        assert_eq!(first_origin.class(), Class::Direct);
        assert_ne!(
            first_origin.request_id(),
            origin(v, second.id()).unwrap().request_id()
        );
        assert_eq!(
            run_queue::enqueue(v, manual("brief", person), now).unwrap(),
            Enqueued::Coalesced(first.id())
        );
        assert_eq!(
            run_queue::enqueue(v, manual("brief", Request::default()), now).unwrap(),
            Enqueued::Coalesced(unknown)
        );
    }

    #[test]
    fn scheduler_coalescing_retains_occurrences_and_leaves_legacy_unknown_separate() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let legacy = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Cron, Some("brief"), "legacy", "scheduled"),
            now,
        )
        .unwrap()
        .id();
        let first = run_queue::enqueue(
            v,
            scheduled("brief", RunSource::Cron, "cron:brief:one"),
            now,
        )
        .unwrap()
        .id();
        assert_ne!(legacy, first);
        let second = run_queue::enqueue(
            v,
            scheduled("brief", RunSource::Cron, "cron:brief:two"),
            now + chrono::Duration::seconds(1),
        )
        .unwrap();
        assert_eq!(second, Enqueued::Coalesced(first));
        assert!(origin(v, legacy).is_none());
        assert_eq!(origin(v, first).unwrap().class(), Class::Autonomous);
        crate::session_store::with_read(v, |conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM work_origin_occurrences WHERE queue_id=?1",
                    [first],
                    |r| r.get(0),
                )
                .map_err(error)?;
            assert_eq!(count, 2);
            let times: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT requested_at) FROM work_origin_requests",
                    [],
                    |r| r.get(0),
                )
                .map_err(error)?;
            assert_eq!(times, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn duplicate_key_cannot_mix_origin_or_create_a_phantom_request() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let first = run_queue::enqueue(
            v,
            manual("brief", Request::person_workflow()).with_key("one"),
            now,
        )
        .unwrap()
        .id();
        for incoming in [Request::default(), Request::person_workflow()] {
            assert!(run_queue::enqueue(v, manual("brief", incoming).with_key("one"), now).is_err());
        }
        assert_eq!(run_queue::live_rows(v).unwrap().len(), 1);
        assert_eq!(origin(v, first).unwrap().class(), Class::Direct);
        crate::session_store::with_read(v, |conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM work_origin_requests", [], |r| {
                    r.get(0)
                })
                .map_err(error)?;
            assert_eq!(count, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn retry_inherits_the_retained_run_after_real_queue_pruning_without_trusting_its_payload() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let old = Utc::now() - chrono::Duration::days(40);
        let parent = run_queue::enqueue(v, manual("brief", Request::person_workflow()), old)
            .unwrap()
            .id();
        let root = origin(v, parent).unwrap();
        run_queue::claim(v, parent, old).unwrap().unwrap();
        run_queue::mark_running(v, parent, "retained-run", old).unwrap();
        run_queue::finish(v, parent, RowStatus::Failed, None, old).unwrap();
        let sweep = run_queue::enqueue(v, manual("sweep", Request::default()), Utc::now())
            .unwrap()
            .id();
        run_queue::finish(v, sweep, RowStatus::Cancelled, None, Utc::now()).unwrap();
        assert!(run_queue::get(v, parent).unwrap().is_none());
        let retry = NewRow::new(RunSource::Retry, Some("brief"), "Retry", "retry")
            .with_origin(Request::retry("retained-run", Some(parent)));
        let retry_id = run_queue::enqueue(v, retry, Utc::now()).unwrap().id();
        assert_eq!(origin(v, retry_id).unwrap().request_id(), root.request_id());
        let wrong = NewRow::new(RunSource::Retry, Some("other"), "Retry", "retry")
            .with_origin(Request::retry("retained-run", Some(parent)));
        assert!(run_queue::enqueue(v, wrong, Utc::now()).is_err());
        let forged = NewRow::new(RunSource::Retry, Some("brief"), "Retry", "retry")
            .with_payload(serde_json::json!({"failedRunId":"retained-run","origin":"direct"}));
        let forged = run_queue::enqueue(v, forged, Utc::now()).unwrap().id();
        assert!(origin(v, forged).is_none());
        let missing = NewRow::new(RunSource::Retry, Some("brief"), "Retry", "retry")
            .with_origin(Request::retry("missing-run", Some(parent)));
        let missing = run_queue::enqueue(v, missing, Utc::now()).unwrap().id();
        assert!(origin(v, missing).is_none());
    }

    #[test]
    fn broker_inherits_only_a_live_exact_binding_and_does_not_upgrade_standing_approval() {
        let _lock = crate::chat::TURN_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for parent_row in [
            manual("parent", Request::default()),
            manual("parent", Request::person_workflow()),
            scheduled("parent", RunSource::Trigger, "scheduled-parent"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let v = dir.path();
            let now = Utc::now();
            let target = crate::vault_workflows::seed_workflow(v, "Origin Child", "manual", "BODY");
            let parent = run_queue::enqueue(v, parent_row, now).unwrap().id();
            let lease =
                crate::automated_history::try_begin_owned(v, "workflow", Some("parent")).unwrap();
            run_queue::claim(v, parent, now).unwrap().unwrap();
            run_queue::mark_running(v, parent, lease.id(), now).unwrap();
            let caller = crate::app_actions::Caller {
                run_id: lease.id().into(),
                surface: "workflow".into(),
                scope: "parent".into(),
                access: crate::app_mcp::AppAccess::Write,
                depth: 0,
                perm: "bypassPermissions".into(),
            };
            let request = Request::caller(v, &caller).unwrap();
            let older = run_queue::enqueue(v, manual("older-unknown", Request::default()), now)
                .unwrap()
                .id();
            let (child, _receiver, _) = crate::workflows::enqueue_run_now(
                v,
                &target.slug,
                now,
                crate::workflows::precheck,
                None,
                request.clone(),
            )
            .unwrap();
            let child = child.id();
            assert_eq!(
                origin(v, child).map(|b| b.root),
                origin(v, parent).map(|b| b.root)
            );
            let expected = if origin(v, parent).is_some_and(|b| b.class() == Class::Direct) {
                child
            } else {
                older
            };
            assert_eq!(
                next_id(v, now, None),
                expected,
                "only retained direct lineage changes order"
            );
            let mut wrong = caller.clone();
            wrong.scope = "not-parent".into();
            assert!(Request::caller(v, &wrong).is_err());
            run_queue::cancel(v, child, now).unwrap();
            wrong = caller.clone();
            wrong.surface = "chat".into();
            assert!(Request::caller(v, &wrong).is_err());
            run_queue::finish(v, parent, RowStatus::Cancelled, None, now).unwrap();
            assert!(run_queue::enqueue(v, manual("after-cancel", request), now).is_err());
            lease.finish("cancelled").unwrap();
            assert!(Request::caller(v, &caller).is_err());
        }
    }

    #[test]
    fn unique_run_binding_and_damaged_root_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let first = run_queue::enqueue(v, manual("one", Request::person_workflow()), now)
            .unwrap()
            .id();
        let second = run_queue::enqueue(v, manual("two", Request::person_workflow()), now)
            .unwrap()
            .id();
        for id in [first, second] {
            run_queue::claim(v, id, now).unwrap().unwrap();
        }
        run_queue::mark_running(v, first, "one-run", now).unwrap();
        // The public owner already treats a repeat on a Running row as false;
        // the binding helper also accepts only the exact same durable pair.
        assert!(!run_queue::mark_running(v, first, "one-run", now).unwrap());
        let first_row = run_queue::get(v, first).unwrap().unwrap();
        crate::session_store::with_write(v, "exact origin retry", |tx| {
            bind_running_in(tx, &first_row, "one-run")
        })
        .unwrap();
        assert!(run_queue::mark_running(v, second, "one-run", now).is_err());
        assert_eq!(
            run_queue::get(v, second).unwrap().unwrap().status,
            RowStatus::Claimed
        );
        crate::session_store::with_write(v, "damage fixture", |tx| {
            tx.execute("UPDATE work_origin_requests SET class='autonomous' WHERE request_id=(SELECT root_request_id FROM work_origin_queue WHERE queue_id=?1)", [second]).map_err(error)?;
            Ok(())
        }).unwrap();
        let damaged = run_queue::get(v, second).unwrap().unwrap();
        assert!(crate::session_store::with_read(v, |conn| for_queue_in(conn, &damaged)).is_err());
    }

    #[test]
    fn live_legacy_broker_without_queue_provenance_remains_unknown() {
        let _lock = crate::chat::TURN_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let lease =
            crate::automated_history::try_begin_owned(v, "workflow", Some("legacy")).unwrap();
        let caller = crate::app_actions::Caller {
            run_id: lease.id().into(),
            surface: "workflow".into(),
            scope: "legacy".into(),
            access: crate::app_mcp::AppAccess::Write,
            depth: 0,
            perm: "bypassPermissions".into(),
        };
        let request = Request::caller(v, &caller).unwrap();
        let child = run_queue::enqueue(v, manual("child", request), Utc::now())
            .unwrap()
            .id();
        assert!(origin(v, child).is_none());
        lease.finish("success").unwrap();
        assert!(Request::caller(v, &caller).is_err());
    }

    #[test]
    fn sibling_absorption_never_relabels_autonomous_work_as_direct() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let direct = run_queue::enqueue(v, manual("brief", Request::person_workflow()), now)
            .unwrap()
            .id();
        let cron = run_queue::enqueue(v, scheduled("brief", RunSource::Cron, "cron-one"), now)
            .unwrap()
            .id();
        let trigger = run_queue::enqueue(
            v,
            scheduled("brief", RunSource::Trigger, "trigger-one"),
            now,
        )
        .unwrap()
        .id();
        assert!(run_queue::absorb_siblings(v, direct, "brief", now)
            .unwrap()
            .is_empty());
        assert_eq!(
            run_queue::get(v, trigger).unwrap().unwrap().status,
            RowStatus::Queued
        );
        let absorbed = run_queue::absorb_siblings(v, cron, "brief", now).unwrap();
        assert_eq!(absorbed.len(), 1);
        assert_eq!(absorbed[0].id, trigger);
        crate::session_store::with_read(v, |conn| {
            let into: i64 = conn.query_row("SELECT admitted_queue_id FROM work_origin_absorptions WHERE sibling_queue_id=?1", [trigger], |r| r.get(0)).map_err(error)?;
            assert_eq!(into, cron);
            Ok(())
        }).unwrap();
        assert_eq!(origin(v, trigger).unwrap().class(), Class::Autonomous);
    }

    #[test]
    fn interrupted_scheduled_occurrence_can_rebind_after_retention_without_replaying_consumed_work()
    {
        for absorbed in [false, true] {
            let dir = tempfile::Builder::new()
                .prefix("origin-recovery-")
                .tempdir()
                .unwrap();
            let v = dir.path();
            let now = Utc::now();
            let old = now - chrono::Duration::days(40);
            let due = (old - chrono::Duration::minutes(1)).to_rfc3339();
            let workflow = crate::vault_workflows::seed_workflow_with(
                v,
                "Origin Retained Recovery",
                "0 0 7 * * * *",
                "BODY",
                |fm| fm.next_run_at = Some(due.clone()),
            );
            crate::workflows::enqueue_due(v, old);
            let cron = run_queue::live_rows(v)
                .unwrap()
                .into_iter()
                .find(|r| r.source == RunSource::Cron)
                .unwrap();
            let original = origin(v, cron.id).unwrap();
            let receiver = if absorbed {
                run_queue::enqueue(
                    v,
                    scheduled(&workflow.slug, RunSource::Trigger, "trigger-receiver"),
                    old,
                )
                .unwrap()
                .id()
            } else {
                cron.id
            };
            run_queue::claim(v, receiver, old).unwrap().unwrap();
            run_queue::mark_running(v, receiver, "old-receiver", old).unwrap();
            if absorbed {
                assert_eq!(
                    run_queue::absorb_siblings(v, receiver, &workflow.slug, old).unwrap()[0].id,
                    cron.id
                );
            }
            run_queue::recover_at_startup(
                v,
                old + chrono::Duration::seconds(1),
                old + chrono::Duration::seconds(2),
                &std::collections::HashSet::new(),
            )
            .unwrap();
            // Ordinary retention removes old queue rows, not origin/terminal facts.
            let sweep = run_queue::enqueue(v, manual("sweep", Request::default()), now)
                .unwrap()
                .id();
            run_queue::finish(v, sweep, RowStatus::Cancelled, None, now).unwrap();
            assert!(run_queue::get(v, cron.id).unwrap().is_none());
            assert!(run_queue::get(v, receiver).unwrap().is_none());
            crate::workflows::enqueue_due(v, now);
            let replacement = run_queue::live_rows(v)
                .unwrap()
                .into_iter()
                .find(|r| r.source == RunSource::Cron)
                .unwrap();
            assert_ne!(replacement.id, cron.id);
            assert_eq!(
                origin(v, replacement.id).unwrap().request_id(),
                original.request_id()
            );
            crate::workflows::enqueue_due(v, now);
            assert_eq!(
                run_queue::live_rows(v)
                    .unwrap()
                    .iter()
                    .filter(|r| r.source == RunSource::Cron)
                    .count(),
                1
            );
            run_queue::claim(v, replacement.id, now).unwrap().unwrap();
            run_queue::mark_running(v, replacement.id, "recovered-receiver", now).unwrap();
            run_queue::finish(v, replacement.id, RowStatus::Done, None, now).unwrap();
            let replay = scheduled(
                &workflow.slug,
                RunSource::Cron,
                &format!("cron:{}:{due}", workflow.slug),
            )
            .with_payload(run_queue::cron_payload(&due));
            assert!(
                run_queue::enqueue(v, replay, now).is_err(),
                "consumption is rechecked inside enqueue"
            );
            crate::workflows::enqueue_due(v, now);
            assert!(!run_queue::live_rows(v)
                .unwrap()
                .iter()
                .any(|r| r.source == RunSource::Cron));
        }
    }

    #[test]
    fn a_person_request_does_not_rebind_after_its_terminal_queue_row() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let person = Request::person_workflow();
        let id = run_queue::enqueue(v, manual("brief", person.clone()), now)
            .unwrap()
            .id();
        run_queue::finish(v, id, RowStatus::Interrupted, None, now).unwrap();
        assert!(run_queue::enqueue(v, manual("brief", person), now).is_err());
        assert!(run_queue::live_rows(v).unwrap().is_empty());
    }

    #[test]
    fn automatic_retry_does_not_consume_a_new_occurrence_of_its_scheduler_root() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let now = Utc::now();
        let due = now.to_rfc3339();
        let parent = run_queue::enqueue(
            v,
            scheduled("brief", RunSource::Cron, &format!("cron:brief:{due}"))
                .with_payload(run_queue::cron_payload(&due)),
            now,
        )
        .unwrap()
        .id();
        run_queue::claim(v, parent, now).unwrap().unwrap();
        run_queue::mark_running(v, parent, "cron-parent", now).unwrap();
        assert_eq!(
            crate::session_store::with_read(v, |conn| consumes_schedule_in(
                conn,
                parent,
                "cron-parent",
                "brief"
            ))
            .unwrap(),
            ScheduleConsumption::Occurrence(due)
        );
        run_queue::finish(v, parent, RowStatus::Failed, None, now).unwrap();
        let retry = run_queue::enqueue(
            v,
            NewRow::new(RunSource::Retry, Some("brief"), "retry", "retry")
                .with_origin(Request::retry("cron-parent", Some(parent))),
            now,
        )
        .unwrap()
        .id();
        run_queue::claim(v, retry, now).unwrap().unwrap();
        run_queue::mark_running(v, retry, "cron-retry", now).unwrap();
        assert_eq!(origin(v, retry).unwrap().class(), Class::Autonomous);
        assert_eq!(
            crate::session_store::with_read(v, |conn| consumes_schedule_in(
                conn,
                retry,
                "cron-retry",
                "brief"
            ))
            .unwrap(),
            ScheduleConsumption::Preserve
        );
    }
}
