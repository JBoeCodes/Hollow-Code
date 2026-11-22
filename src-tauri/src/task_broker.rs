//! The GUI half of the `hollow_tasks` MCP (Paperclip PR 12): the only place
//! an agent's task write actually happens.
//!
//! `hollow tasks-mcp` (`tasks_mcp.rs`) is a separate process and cannot share
//! the GUI's in-process locks (`vault::MUTATE_LOCK`, `vault_tasks`'s
//! `CREATE_LOCK`, the review queue's lock), so it forwards every write here
//! over a per-pid Unix socket (write option (a) of the contract; modeled on
//! `approvals.rs`). The broker:
//!
//! 1. refuses everything while Settings, Agent tools, Task tools is off;
//! 2. accepts only the `chat` and `workflow` surfaces, and checks the proof
//!    the GUI minted into the server's environment
//!    (`sha256(secret ‖ run id ‖ surface ‖ scope ‖ access)`, the secret held
//!    only in this process's memory), and that the access is `write`;
//! 3. parses and validates every field strictly (dates, clock times,
//!    durations, priorities, recurrences, one-line titles, caps), before any
//!    lock, so a bad value is a tool error naming the field and nothing is
//!    written;
//! 4. runs the whole operation inside
//!    [`crate::automated_history::with_live_run`], which re-reads the lease
//!    under `HISTORY_LOCK` and writes while holding it: a write is inside the
//!    requesting run's finalizing diff (History & recovery, the receipt as
//!    `observed`) or refused once that run has finished, never check-then-act;
//! 5. writes through the same in-process writers the Tauri commands use
//!    (`create_agent_task`, `apply_update`, `set_status_with_recurrence`,
//!    `append_activity`, the review queue), so review-queue writes stay
//!    host-attributed (`run_guard::write_host_state`) and a proposal never
//!    raises a false PR 5 protected-change flag.
//!
//! The proof guards against stale or foreign processes, not against the
//! running agent itself (it can read its server's environment and edit files
//! with its own tools); the guarantee is lease-bound, GUI-locked writes.
//!
//! Lock order: `HISTORY_LOCK` (held by `with_live_run`) → {`vault::MUTATE_LOCK`,
//! `CREATE_LOCK`, the review queue's lock} → run_guard `TRACKED`. `PROPOSED`
//! is a leaf taken briefly, never across a queue write.

use crate::review_queue::{self, NewReviewItem, ReviewItem};
use crate::tasks_mcp::{self, TasksAccess};
use crate::types::{CreateTaskInput, SubTask, Task, UpdateTaskInput};
use crate::vault_tasks;
use chrono::{DateTime, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The review kind `propose_tasks` queues (Tasks → Task suggestions).
pub(crate) const AGENT_TASK_KIND: &str = "agent_task";
/// Emitted after a proposal queued something (listener in
/// `src/store/runtime.ts`); the vault watcher ignores the dot-prefixed queue.
pub(crate) const REVIEW_EVENT: &str = "review-queue-changed";
/// Surfaces whose runs may write tasks. The heartbeat, ingest and one-shots
/// never get a proof, and are refused here even if one were forged. `ask` is
/// a plan the person previewed and ran from the command bar (overnight 10-11
/// A2, `ask_hollow::run`).
const WRITE_SURFACES: [&str; 3] = ["chat", "workflow", "ask"];

const MAX_PROPOSE_PER_CALL: usize = 10;
const MAX_PROPOSE_PER_RUN: usize = 25;
/// How long a tracked-task write waits for a running tracker sync cycle.
const EDIT_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_PENDING_AGENT_TASKS: usize = 50;
const TITLE_MAX: usize = 240;
const STATUS_MAX: usize = 60;
const SECTION_MAX: usize = 60;
const LABEL_MAX: usize = 60;
const MAX_LABELS: usize = 20;
const DESCRIPTION_MAX: usize = 20_000;
const NOTE_MAX: usize = 2_000;
const EVIDENCE_MAX: usize = 500;
const MAX_BLOCKERS: usize = 50;
const MAX_SUBTASKS: usize = 200;
const SUBTASK_MAX: usize = 500;
const DURATION_MAX: u64 = 1_440;
/// A request line is read up to this many bytes.
#[cfg(unix)]
const REQUEST_MAX_BYTES: u64 = 1 << 20;
/// Runs remembered for the per-run proposal cap (one lease runs at a time,
/// so a handful is plenty; the list never grows past this).
const PROPOSED_RUNS_KEPT: usize = 8;

const SWITCHED_OFF: &str =
    "Task tools are switched off in Settings → Connectors; nothing was written.";
const PROOF_REFUSED: &str =
    "This task request doesn't carry a valid Hollow run identity; nothing was written.";
const READ_ONLY_REFUSED: &str = "This run can read tasks but not change them (plan mode, or an automation whose allowed tools don't include task writes); nothing was written.";

/// Random per-process secret behind every proof.
static SECRET: OnceLock<[u8; 32]> = OnceLock::new();
/// The broker's socket while [`start`]'s listener is serving; cleared when
/// that listener stops, so `tasks_mcp::server_entry` never attaches a server
/// whose every write would fail.
static SOCKET_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
/// `(history run id, proposals queued)` for the most recent runs.
static PROPOSED: Mutex<VecDeque<(String, usize)>> = Mutex::new(VecDeque::new());
/// Serializes `create_project`'s find-by-key and create, so two retries of
/// one key can't both create.
static PROJECT_CREATE: Mutex<()> = Mutex::new(());

// ── Settings ─────────────────────────────────────────────────────────────

/// Load the persisted Task tools switch (`agent-tools.json`) into the
/// process flag `tasks_mcp::server_entry` and the socket listener read. Called in
/// setup before the run-queue pump starts, so an automation admitted before
/// the webview boots already sees the user's choice. A damaged file reads as
/// off (`agent_settings::tasks_tools_enabled`).
pub(crate) fn load_settings() {
    tasks_mcp::set_enabled(crate::agent_settings::tasks_tools_enabled());
}

/// Persist the switch, then apply it. New runs see it; a run already
/// holding the vault keeps its attached server, and the broker refuses its
/// writes from now on.
pub(crate) fn save_settings(enabled: bool) -> Result<(), String> {
    crate::agent_settings::set_tasks_tools_enabled(enabled)?;
    tasks_mcp::set_enabled(enabled);
    Ok(())
}

// ── Identity ─────────────────────────────────────────────────────────────

/// The broker's socket, when it is listening.
pub(crate) fn socket_path() -> Option<PathBuf> {
    SOCKET_PATH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// The proof a run's server carries: hex sha256 over this process's secret
/// and the NUL-separated trimmed run id, surface, scope and access. The
/// scope is included because it scopes idempotency keys.
pub(crate) fn proof_for(run_id: &str, surface: &str, scope: &str, access: TasksAccess) -> String {
    let secret = SECRET.get_or_init(rand::random::<[u8; 32]>);
    let mut hasher = Sha256::new();
    hasher.update(secret);
    for part in [run_id.trim(), surface.trim(), scope.trim(), access.as_str()] {
        hasher.update([0u8]);
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Constant-time comparison of two proofs (both fixed-width hex).
fn proof_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ── Requests ─────────────────────────────────────────────────────────────

/// One forwarded write, as `tasks_mcp::RunIdentity::request` writes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrokerRequest {
    #[serde(default)]
    pub v: u32,
    pub op: String,
    pub run_id: String,
    pub surface: String,
    pub scope: String,
    pub access: String,
    pub proof: String,
    #[serde(default)]
    pub args: Value,
}

/// What a dispatched write produced.
#[derive(Debug, Clone)]
pub(crate) struct Dispatched {
    /// The tool result the agent sees.
    pub result: Value,
    /// Review items queued (a nonzero count raises [`REVIEW_EVENT`]).
    pub review_changed: usize,
}

/// Dispatch one forwarded write: the switch, the proof, validation, then the
/// operation inside the run's live lease. Sync and AppHandle-free; the socket
/// listener runs it on its own thread with the live switch and clock, tests
/// pass both in.
pub(crate) fn dispatch_at(
    vault: &Path,
    req: &BrokerRequest,
    enabled: bool,
    now: DateTime<Local>,
) -> Result<Dispatched, String> {
    dispatch_at_checked(
        vault,
        req,
        enabled,
        now,
        &crate::shared_mcp::request_current,
    )
}

pub(crate) fn dispatch_at_checked(
    vault: &Path,
    req: &BrokerRequest,
    enabled: bool,
    now: DateTime<Local>,
    valid: &dyn Fn() -> bool,
) -> Result<Dispatched, String> {
    if !valid() {
        return Err(crate::app_broker::RUN_ENDED.into());
    }
    if !enabled {
        return Err(SWITCHED_OFF.into());
    }
    if req.v != 1 {
        return Err(format!(
            "unsupported task request version {}; nothing was written.",
            req.v
        ));
    }
    let surface = req.surface.trim();
    if !WRITE_SURFACES.contains(&surface) {
        return Err("Task writes are available only to Hollow chat turns and automations; nothing was written.".into());
    }
    let access = TasksAccess::parse(&req.access).ok_or(PROOF_REFUSED)?;
    let (run_id, scope) = (req.run_id.trim(), req.scope.trim());
    if run_id.is_empty() || scope.is_empty() {
        return Err(PROOF_REFUSED.into());
    }
    if !proof_matches(&proof_for(run_id, surface, scope, access), req.proof.trim()) {
        return Err(PROOF_REFUSED.into());
    }
    if access != TasksAccess::ReadWrite {
        return Err(READ_ONLY_REFUSED.into());
    }
    // Plan completion L2: a guarded retry never repeats the exact task write
    // the run it retries already made (`run_effects`).
    if let Some(refused) =
        crate::run_effects::refusal(run_id, "hollow_tasks", req.op.trim(), &req.args)
    {
        return Err(refused);
    }
    // Validation needs no lock; a refusal writes nothing.
    let op = parse_op(&req.op, &req.args, vault, now.date_naive())?;
    let _tracker = if needs_tracker_lock(vault, &op) {
        Some(crate::tracker_sync::seams::hold_running_edit_lock(
            vault,
            EDIT_LOCK_WAIT,
        )?)
    } else {
        None
    };
    let ctx = WriteCtx {
        vault,
        run_id,
        surface,
        scope,
        now,
        device_name: None,
    };
    let touches = touches(&op);
    // X2: a tracker edit staged here is audited as this run's (its session
    // key, or `automation:<slug>`), not the person's.
    let asker = crate::audit::caller_asker(surface, scope);
    crate::automated_history::with_live_run_touching(vault, run_id, &[surface], touches, || {
        if !valid() {
            return Err(crate::app_broker::RUN_ENDED.into());
        }
        crate::audit::asking_sync(&asker, || apply(&ctx, op))
    })
}

fn needs_tracker_lock(vault: &Path, op: &Op) -> bool {
    match op {
        Op::TrackerUpdate { .. } => true,
        Op::Claim { rel, .. } | Op::Release { rel, .. } => {
            crate::tracker_sync::seams::tracked(vault, rel)
        }
        _ => false,
    }
}

/// Where `op` can change vault files (chat concurrency F7, 2026-10-20), so
/// a workspace conversation's task write snapshots just those instead of
/// the whole vault twice. Tasks live under `tasks/` (a status change rolls
/// a repeat forward in place), projects under `projects/` (a status change
/// moves the note between its folders) or at a workspace's `PROJECT.md`,
/// and a proposal's review queue is a dot-file History never diffs. Any
/// write keeps the whole vault when a link sits in these folders
/// (`scoped_live_write`).
fn touches(op: &Op) -> crate::automated_history::Touches {
    use crate::automated_history::Touches;
    let mut paths = vec!["tasks".to_string(), "projects".to_string()];
    if let Op::UpdateProject { rel, .. }
    | Op::SetProjectStatus { rel, .. }
    | Op::TrackerUpdate { rel, .. } = op
    {
        paths.push(rel.clone());
    }
    Touches::Under(paths)
}

/// The surface a paired companion device's op writes as (`companion_inbox`).
pub(crate) const COMPANION_SURFACE: &str = "companion";

/// Why a companion write didn't land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanionRefusal {
    /// A run (or another background writer) holds the vault, or tracker
    /// sync is mid-cycle: nothing was written; try the op again later.
    Busy(String),
    /// The op itself was refused (validation, unknown ref, a field the
    /// tracker holds, `TASK_CONFLICT: …` on a stale hash). Nothing was written.
    Refused(String),
}

/// Apply one already-verified companion op through the same parsing,
/// validation and writers agents use, as the person's own edit: the
/// signature check upstream (`companion_inbox`) stands in for the run proof.
/// `op` is one of `create_task`, `update_task`, `set_status`, `add_comment`;
/// `op_id` is the scope of the create/comment idempotency. Holds tracker
/// sync's edit lock for a tracked write, then a short companion lease (so
/// the write never lands inside an automated run's History capture).
pub(crate) fn dispatch_companion(
    vault: &Path,
    device_id: &str,
    device_name: &str,
    op_id: &str,
    op: &str,
    args: &Value,
    now: DateTime<Local>,
) -> Result<Value, CompanionRefusal> {
    dispatch_companion_gated(vault, device_id, device_name, op_id, op, args, now, || {
        Ok(())
    })
}

/// [`dispatch_companion`] with a commit-point `gate` (R1: the phone is still
/// paired), run once the edit lock (up to [`EDIT_LOCK_WAIT`]) and the
/// companion lease are held, right before the write. A refusal is returned
/// as `Refused(reason)` and nothing is written.
pub(crate) fn dispatch_companion_gated(
    vault: &Path,
    device_id: &str,
    device_name: &str,
    op_id: &str,
    op: &str,
    args: &Value,
    now: DateTime<Local>,
    gate: impl FnOnce() -> Result<(), String>,
) -> Result<Value, CompanionRefusal> {
    if !matches!(
        op,
        "create_task" | "update_task" | "set_status" | "add_comment"
    ) {
        return Err(CompanionRefusal::Refused(format!(
            "unknown companion task operation: {op}"
        )));
    }
    let parsed = parse_op(op, args, vault, now.date_naive()).map_err(CompanionRefusal::Refused)?;
    let _tracker = if matches!(&parsed, Op::TrackerUpdate { .. }) {
        Some(
            crate::tracker_sync::seams::hold_running_edit_lock(vault, EDIT_LOCK_WAIT)
                .map_err(CompanionRefusal::Busy)?,
        )
    } else {
        None
    };
    let _lease = crate::automated_history::companion_write_lease(vault).map_err(|e| match e {
        crate::automated_history::BeginRunError::Busy(holder) => {
            CompanionRefusal::Busy(holder.busy_message())
        }
        crate::automated_history::BeginRunError::Failed(error) => CompanionRefusal::Busy(error),
    })?;
    gate().map_err(CompanionRefusal::Refused)?;
    let ctx = WriteCtx {
        vault,
        run_id: op_id,
        surface: COMPANION_SURFACE,
        scope: device_id,
        now,
        device_name: Some(device_name),
    };
    apply(&ctx, parsed)
        .map(|done| done.result)
        .map_err(CompanionRefusal::Refused)
}

/// The run a write is made for.
struct WriteCtx<'a> {
    vault: &'a Path,
    run_id: &'a str,
    surface: &'a str,
    scope: &'a str,
    now: DateTime<Local>,
    /// A companion device's name (its writes are the person's, not an agent's).
    device_name: Option<&'a str>,
}

/// A validated operation.
enum Op {
    Create(CreateTaskInput),
    TrackerUpdate {
        rel: String,
        expected: String,
        fields: Map<String, Value>,
    },
    Update {
        rel: String,
        input: UpdateTaskInput,
        /// TaskModal field names the update touches, for the tracker hold.
        touched: Vec<&'static str>,
        /// Values as the agent wrote them, before normalization; only
        /// `project_url` is read.
        exact: std::collections::BTreeMap<String, String>,
    },
    SetStatus {
        rel: String,
        status: String,
        expected: Option<String>,
    },
    Comment {
        rel: String,
        body: String,
        key: Option<String>,
        expected: Option<String>,
    },
    /// S09: claim a task for the requesting run, or release its claim.
    Claim {
        rel: String,
        expected: Option<String>,
    },
    Release {
        rel: String,
        expected: Option<String>,
    },
    Propose(Vec<ProposalDraft>),
    CreateProject {
        input: crate::types::CreateProjectInput,
        key: String,
    },
    UpdateProject {
        rel: String,
        input: crate::types::UpdateProjectInput,
        expected: String,
    },
    SetProjectStatus {
        rel: String,
        status: String,
        expected: Option<String>,
    },
}

/// One validated `propose_tasks` item.
#[derive(Debug, Clone)]
struct ProposalDraft {
    title: String,
    due: Option<String>,
    priority: Option<String>,
    note: String,
    project_url: Option<String>,
    labels: Vec<String>,
    evidence: String,
}

fn apply(ctx: &WriteCtx, op: Op) -> Result<Dispatched, String> {
    let vault = ctx.vault;
    let applied = |result: Value| Dispatched {
        result,
        review_changed: 0,
    };
    match op {
        Op::TrackerUpdate {
            rel,
            expected,
            fields,
        } => crate::tracker_sync::seams::edit_task_at(
            vault,
            &rel,
            &expected,
            &fields,
            ctx.now.date_naive(),
        )
        .map(applied),
        Op::Create(input) => {
            let created = if ctx.surface == COMPANION_SURFACE {
                vault_tasks::create_companion_task(vault, &input, ctx.scope, ctx.run_id)?
            } else {
                vault_tasks::create_agent_task(vault, &input, ctx.surface, ctx.scope, ctx.run_id)?
            };
            Ok(applied(json!({
                "ref": format!("vault:{}", created.rel),
                "disposition": if created.existing { "duplicate" } else { "applied" },
                "title": created.title,
                "status": created.status,
                "contentHash": created.content_hash,
            })))
        }
        Op::Update {
            rel,
            input,
            touched,
            exact,
        } => {
            let task = task_with_locks(vault, &rel)?;
            let held = vault_tasks::held_modal_fields(&task);
            let blocked: Vec<&str> = touched
                .iter()
                .copied()
                .filter(|f| held.contains(f))
                .collect();
            if !blocked.is_empty() {
                return Err(linked_refusal(&blocked));
            }
            let hash = match exact.get("project_url") {
                Some(url) => {
                    vault_tasks::apply_update_finishing(vault, &rel, &input, &|_before, text| {
                        local_project_link(vault, text, url)
                    })?
                }
                None => vault_tasks::apply_update(vault, &rel, &input)?,
            };
            Ok(applied(
                json!({ "ref": format!("vault:{rel}"), "disposition": "applied", "contentHash": hash }),
            ))
        }
        Op::SetStatus {
            rel,
            status,
            expected,
        } => {
            let task = task_with_locks(vault, &rel)?;
            if vault_tasks::held_modal_fields(&task).contains("status") {
                return Err(linked_refusal(&["status"]));
            }
            if let Some(recurrence) = task.recurrence.as_deref().filter(|r| !r.trim().is_empty()) {
                if vault_tasks::is_terminal_status(&status) && expected.is_none() {
                    return Err(format!(
                        "set_status: this task repeats (\"{recurrence}\"), so finishing it rolls its due date forward. Pass the expectedHash you last read so a retry can't roll it forward twice. Nothing was saved."
                    ));
                }
            }
            let today = ctx.now.date_naive();
            let outcome = vault_tasks::set_status_with_recurrence(
                vault,
                &rel,
                &status,
                today,
                expected.as_deref(),
            )?;
            Ok(applied(json!({
                "ref": format!("vault:{rel}"),
                "disposition": "applied",
                "status": outcome.status,
                "rolledForwardTo": outcome.rolled_to,
                "contentHash": outcome.content_hash,
            })))
        }
        Op::Comment {
            rel,
            body,
            key,
            expected,
        } => {
            let marker = key
                .as_deref()
                .and_then(|k| vault_tasks::activity_marker(ctx.surface, ctx.scope, k));
            let author = match ctx.device_name {
                Some(device) => format!("{device} (companion)"),
                None => format!("agent ({})", ctx.surface),
            };
            let outcome = vault_tasks::append_activity(
                vault,
                &rel,
                &author,
                &body,
                ctx.now,
                expected.as_deref(),
                marker.as_deref(),
            )?;
            Ok(applied(json!({
                "ref": format!("vault:{rel}"),
                "disposition": if outcome.duplicate { "duplicate" } else { "applied" },
                "contentHash": outcome.content_hash,
            })))
        }
        Op::Claim { rel, expected } => {
            let task = task_with_locks(vault, &rel)?;
            if task.tracker.is_some() {
                crate::tracker_sync::seams::check_claim(vault, &rel)?;
            }
            let owner = claim_owner(ctx);
            let outcome =
                vault_tasks::claim_task(vault, &rel, &owner, ctx.now, expected.as_deref())?;
            Ok(applied(claim_result(&rel, outcome)))
        }
        Op::Release { rel, expected } => {
            if crate::tracker_sync::seams::tracked(vault, &rel) {
                crate::tracker_sync::seams::check_claim(vault, &rel)?;
            }
            let owner = claim_owner(ctx);
            let outcome = vault_tasks::release_task(vault, &rel, &owner, expected.as_deref())?;
            Ok(applied(claim_result(&rel, outcome)))
        }
        Op::Propose(drafts) => propose(ctx, drafts),
        Op::CreateProject { input, key } => {
            // Scoped like create_task's key, so two conversations' keys never meet.
            let scoped = vault_tasks::scoped_agent_key(ctx.surface, ctx.scope, &key)
                .ok_or("create_project: an idempotencyKey is required. Nothing was created.")?;
            create_project(vault, &input, &scoped).map(applied)
        }
        Op::UpdateProject {
            rel,
            input,
            expected,
        } => project_write(vault, &rel, Some(&expected), |finish| {
            crate::vault_projects::update_finishing(vault, &rel, &input, finish)
        })
        .map(applied),
        Op::SetProjectStatus {
            rel,
            status,
            expected,
        } => project_write(vault, &rel, expected.as_deref(), |finish| {
            crate::vault_projects::set_status_finishing(vault, &rel, &status, finish)
        })
        .map(applied),
    }
}

/// Who a claim belongs to (S09): the proof-checked surface and scope, i.e.
/// `chat:<session key>` or `workflow:<slug>` (`ask:<scope>` from the command
/// bar). Every turn of one chat session shares it, so a later turn may
/// release what an earlier one claimed.
fn claim_owner(ctx: &WriteCtx) -> String {
    format!("{}:{}", ctx.surface, ctx.scope)
}

/// The `claim_task` / `release_task` result. Always `applied`: an owner's
/// repeat claim succeeded too, so it says `already: true` (nothing changed)
/// rather than `duplicate`, which read like a refusal (S24).
fn claim_result(rel: &str, outcome: vault_tasks::ClaimOutcome) -> Value {
    json!({
        "ref": format!("vault:{rel}"),
        "disposition": "applied",
        "already": outcome.already,
        "status": outcome.status,
        "claimedBy": outcome.owner,
        "claimedAt": outcome.claimed_at,
        "contentHash": outcome.content_hash,
    })
}

/// The task at `rel` with its tracker lock filled in.
fn task_with_locks(vault: &Path, rel: &str) -> Result<Task, String> {
    let task = vault_tasks::get(vault, rel)
        .ok_or_else(|| format!("{rel} is not a readable task (it has no title)."))?;
    let mut one = vec![task];
    vault_tasks::mark_task_locks(vault, &mut one);
    Ok(one.remove(0))
}

/// The [`vault_tasks::Finish`] step that files a task under the project
/// note `url` (a project's ref): its `project` name and `project_url` link.
fn local_project_link(vault: &Path, text: String, url: &str) -> Result<String, String> {
    let (name, url) = crate::vault_projects::task_project_link(vault, url)?;
    crate::frontmatter::patch(
        &text,
        &[
            ("project", crate::frontmatter::KeyEdit::Set(name)),
            ("project_url", crate::frontmatter::KeyEdit::Set(url)),
        ],
        None,
    )
}

/// `create_project`: a local project note. A retry with the same scoped key returns the
/// project it made; the same key for a different title is refused.
fn create_project(
    vault: &Path,
    input: &crate::types::CreateProjectInput,
    key: &str,
) -> Result<Value, String> {
    let _one = PROJECT_CREATE.lock().unwrap_or_else(|e| e.into_inner());
    let view = |rel: &str, disposition: &str| -> Result<Value, String> {
        let text = std::fs::read_to_string(crate::vault::safe_join(vault, rel)?)
            .map_err(|e| format!("read {rel}: {e}"))?;
        let project = crate::vault_projects::parse(rel, &text, vault)
            .ok_or_else(|| format!("{rel} has no title"))?;
        Ok(json!({
            "ref": format!("vault:{rel}"),
            "disposition": disposition,
            "title": project.name,
            "status": project.status,
            "contentHash": vault_tasks::content_hash(&text),
        }))
    };
    // What this key was for: the requested title, so a later rename of the
    // project doesn't make a retry look like different work.
    let work = format!(
        "{:x}",
        Sha256::digest(vault_tasks::slugify(input.title.trim()).as_bytes())
    )[..12]
        .to_string();
    if let Some((rel, stored)) = crate::vault_projects::find_by_agent_key(vault, key) {
        let existing = view(&rel, "duplicate")?;
        if !stored.is_empty() && stored != work {
            return Err(format!(
                "{}: this idempotency key already created \"{}\" (vault:{rel}), which is different work from \"{}\". Nothing was created; use a new key for new work.",
                vault_tasks::TASK_IDEMPOTENCY_MISMATCH_ERROR,
                existing["title"].as_str().unwrap_or(""),
                input.title.trim()
            ));
        }
        return Ok(existing);
    }
    if !vault.join("projects").is_dir() {
        return Err(
            "create_project creates legacy task-project notes, not Hollow's sidebar projects. This vault has no legacy projects/ folder. Nothing was created. For a local Hollow project, use hollow_app list_workspaces to check for it, then create_workspace {title, goal}."
                .into(),
        );
    }
    let fm = crate::vault_projects::frontmatter_for_create(input)?;
    let rel = crate::vault_projects::create_keyed(vault, fm, Some(&format!("{key}#{work}")))?;
    view(&rel, "applied")
}

/// One project edit (`write` calls `update_finishing` or
/// `set_status_finishing` with the step it is given). The step checks
/// `expected` against the file it is about to change.
fn project_write(
    vault: &Path,
    rel: &str,
    expected: Option<&str>,
    write: impl FnOnce(vault_tasks::Finish) -> Result<String, String>,
) -> Result<Value, String> {
    let written = std::cell::RefCell::new(String::new());
    let new_rel = write(&|before, after| {
        if let Some(expected) = expected.filter(|h| !h.is_empty()) {
            if vault_tasks::content_hash(before) != expected {
                return Err(format!(
                    "{}: vault:{rel} changed after you read it. Nothing was saved; read the project again (get_project), then reapply your change.",
                    vault_tasks::TASK_CONFLICT_ERROR
                ));
            }
        }
        // The bytes written (in place or at the new status folder), hashed
        // here rather than re-read after the lock is gone.
        *written.borrow_mut() = after.clone();
        Ok(after)
    })?;
    let written = written.into_inner();
    let status = crate::vault_projects::parse(&new_rel, &written, vault).map(|p| p.status);
    Ok(json!({
        "ref": format!("vault:{new_rel}"),
        "disposition": "applied",
        "status": status,
        "contentHash": vault_tasks::content_hash(&written),
    }))
}

/// `TASK_LINKED: …` for fields a task's tracker issue holds.
fn linked_refusal(fields: &[&str]) -> String {
    format!(
        "{}: {} on this task {} held by its tracker issue and can't be changed here. Nothing was saved; change {} with update_task's tracker fields or from Hollow's Tasks page. add_comment still works.",
        vault_tasks::TASK_LINKED_ERROR,
        fields.join(", "),
        if fields.len() == 1 { "is" } else { "are" },
        if fields.len() == 1 { "it" } else { "them" },
    )
}

// ── Proposals ────────────────────────────────────────────────────────────

/// The `agent_task` fingerprint: `agent-task:` + 24 hex of sha256(title
/// slug), so a reworded-only-in-case repeat is one suggestion and a rejected
/// one stays rejected.
pub(crate) fn agent_task_fingerprint(title: &str) -> String {
    let slug = vault_tasks::slugify(title);
    format!(
        "agent-task:{}",
        &format!("{:x}", Sha256::digest(slug.as_bytes()))[..24]
    )
}

/// Title slugs of every `agent_task` suggestion in any state, for the
/// heartbeat's own "already proposed" dedup.
pub(crate) fn agent_task_slugs(vault: &Path) -> HashSet<String> {
    review_queue::list(vault, Some(AGENT_TASK_KIND))
        .unwrap_or_default()
        .iter()
        .filter_map(|item| item.payload.get("title").and_then(Value::as_str))
        .map(vault_tasks::slugify)
        .collect()
}

fn proposed_count(run_id: &str) -> usize {
    let runs = PROPOSED.lock().unwrap_or_else(|e| e.into_inner());
    runs.iter()
        .find(|(id, _)| id == run_id)
        .map(|(_, n)| *n)
        .unwrap_or(0)
}

fn add_proposed(run_id: &str, count: usize) {
    if count == 0 {
        return;
    }
    let mut runs = PROPOSED.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = runs.iter_mut().find(|(id, _)| id == run_id) {
        entry.1 += count;
        return;
    }
    runs.push_back((run_id.to_string(), count));
    while runs.len() > PROPOSED_RUNS_KEPT {
        runs.pop_front();
    }
}

/// Whether `enqueue` created `item` for this run, rather than handing back an
/// item another proposal (or a reviewed card) already holds.
fn fresh_for(item: &ReviewItem, ctx: &WriteCtx) -> bool {
    item.status == "pending"
        && item.payload.get("runId").and_then(Value::as_str) == Some(ctx.run_id)
}

fn propose(ctx: &WriteCtx, drafts: Vec<ProposalDraft>) -> Result<Dispatched, String> {
    let vault = ctx.vault;
    let existing: HashSet<String> = vault_tasks::list_parsed(vault)
        .iter()
        .map(|t| vault_tasks::slugify(&t.title))
        .collect();
    let items = review_queue::list(vault, None)?;
    let proposed: HashSet<String> = items
        .iter()
        .filter(|i| {
            matches!(
                i.kind.as_str(),
                "heartbeat_task" | "meeting_task" | AGENT_TASK_KIND
            )
        })
        .filter_map(|i| i.payload.get("title").and_then(Value::as_str))
        .map(vault_tasks::slugify)
        .collect();
    // The fingerprints too: an accepted suggestion whose title the user
    // edited no longer carries the proposed title in its payload, but keeps
    // the fingerprint of the original one, and `review_queue::enqueue` would
    // hand that reviewed item back instead of queueing a new card.
    let fingerprints: HashSet<&str> = items
        .iter()
        .filter(|i| i.kind == AGENT_TASK_KIND)
        .map(|i| i.fingerprint.as_str())
        .collect();
    let pending = items
        .iter()
        .filter(|i| {
            i.kind == AGENT_TASK_KIND
                && matches!(i.status.as_str(), "pending" | "snoozed" | "applying")
        })
        .count();

    let mut seen: HashSet<String> = HashSet::new();
    let mut to_queue: Vec<ProposalDraft> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for draft in drafts {
        let slug = vault_tasks::slugify(&draft.title);
        if existing.contains(&slug) {
            skipped.push(json!({ "title": draft.title, "reason": "exists" }));
        } else if proposed.contains(&slug)
            || fingerprints.contains(agent_task_fingerprint(&draft.title).as_str())
            || !seen.insert(slug)
        {
            skipped.push(json!({ "title": draft.title, "reason": "duplicate" }));
        } else {
            to_queue.push(draft);
        }
    }
    if !to_queue.is_empty() {
        let already = proposed_count(ctx.run_id);
        if already + to_queue.len() > MAX_PROPOSE_PER_RUN {
            return Err(format!(
                "This run has already proposed {already} task(s); at most {MAX_PROPOSE_PER_RUN} per run. Nothing was queued; let the user review Tasks → Task suggestions first."
            ));
        }
        if pending + to_queue.len() > MAX_PENDING_AGENT_TASKS {
            return Err(format!(
                "Review Task suggestions first: {pending} agent-proposed task(s) are already waiting (at most {MAX_PENDING_AGENT_TASKS}). Nothing was queued."
            ));
        }
    }
    let mut queued: Vec<Value> = Vec::new();
    let mut failure: Option<String> = None;
    for draft in to_queue {
        let payload = json!({
            "title": draft.title,
            "due": draft.due,
            "priority": draft.priority,
            "note": draft.note,
            "projectUrl": draft.project_url,
            "labels": draft.labels,
            "evidenceExcerpt": draft.evidence,
            "runId": ctx.run_id,
            "surface": ctx.surface,
            "scope": ctx.scope,
        });
        let item = NewReviewItem {
            kind: AGENT_TASK_KIND.into(),
            fingerprint: agent_task_fingerprint(&draft.title),
            payload,
            risk: "local_write".into(),
            source: ctx.surface.into(),
            source_path: None,
        };
        match review_queue::enqueue(vault, item) {
            // `enqueue` returns an existing item with the same fingerprint
            // unchanged (one that landed after the list above): that is not a
            // new card, so it is reported as skipped and not counted.
            Ok(item) if !fresh_for(&item, ctx) => {
                skipped.push(json!({ "title": draft.title, "reason": "duplicate" }));
            }
            Ok(item) => queued.push(json!({ "title": draft.title, "id": item.id })),
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    add_proposed(ctx.run_id, queued.len());
    if let Some(error) = failure {
        return Err(format!(
            "{error} ({} suggestion(s) were queued before this).",
            queued.len()
        ));
    }
    let count = queued.len();
    Ok(Dispatched {
        result: json!({
            "queued": queued,
            "skipped": skipped,
            "message": if count == 0 {
                "Nothing new to suggest: every item already exists or was already proposed.".to_string()
            } else {
                format!("Queued {count} suggestion(s) for the user's review in Tasks → Task suggestions. No task was created.")
            },
        }),
        review_changed: count,
    })
}

/// An `agent_task` review payload. It lives in the agent-writable
/// `.review-queue.json`, so [`accept_agent_task`] validates it again.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentTaskPayload {
    pub title: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub evidence_excerpt: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub scope: String,
}

/// Accept an `agent_task` suggestion (the `resolve_review_item` arm): the
/// meeting-task pattern. Edits (title, due, priority, note) and the stored
/// payload are validated again, the item is claimed with the approved value,
/// the task is created through the shared idempotent create keyed by the
/// item's fingerprint (so a double accept, or a re-accept after a crash left
/// the item `applying` and recovery returned it, creates exactly one task,
/// and a task with the same title slug is returned instead of a twin), then
/// the item is accepted; any error after the claim returns it to pending.
pub(crate) fn accept_agent_task(
    vault: &Path,
    item: &ReviewItem,
    edited: Option<Value>,
) -> Result<(), String> {
    if item.kind != AGENT_TASK_KIND {
        return Err(format!("review item {} is not an agent task", item.id));
    }
    if !WRITE_SURFACES.contains(&item.source.as_str()) {
        return Err("agent task suggestion has an unknown source".into());
    }
    accept_task_item(vault, item, edited)
}

/// Accept an extracted task (master plan F3: a `meeting_task` with an
/// anchor, from a recording or a chat): the same validation, claim and
/// idempotent create as [`accept_agent_task`].
pub(crate) fn accept_extracted_task(
    vault: &Path,
    item: &ReviewItem,
    edited: Option<Value>,
) -> Result<(), String> {
    if item.kind != crate::extract::TASK_KIND || item.payload.get("anchor").is_none() {
        return Err(format!("review item {} is not an extracted task", item.id));
    }
    if !matches!(item.source.as_str(), "meeting" | "chat") {
        return Err("extracted task suggestion has an unknown source".into());
    }
    accept_task_item(vault, item, edited)
}

fn accept_task_item(vault: &Path, item: &ReviewItem, edited: Option<Value>) -> Result<(), String> {
    let original: AgentTaskPayload = serde_json::from_value(item.payload.clone())
        .map_err(|error| format!("invalid agent task: {error}"))?;
    let mut approved = original;
    if let Some(value) = edited {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
        if let Some(title) = text("title") {
            approved.title = title;
        }
        approved.due = text("due");
        approved.priority = text("priority");
        if let Some(note) = text("note") {
            approved.note = note;
        }
    }
    approved.title = vault_tasks::one_line(&approved.title, TITLE_MAX);
    approved.due = vault_tasks::valid_due(approved.due.take());
    approved.priority = vault_tasks::valid_priority(approved.priority.take());
    approved.note = approved.note.chars().take(NOTE_MAX).collect();
    approved.evidence_excerpt = vault_tasks::one_line(&approved.evidence_excerpt, EVIDENCE_MAX);
    approved.labels = approved
        .labels
        .iter()
        .map(|l| vault_tasks::one_line(l, LABEL_MAX).replace(',', " "))
        .filter(|l| !l.trim().is_empty())
        .take(MAX_LABELS)
        .collect();
    if approved.title.is_empty() {
        return Err("agent task title is no longer valid".into());
    }
    // Merged into the stored payload, so fields the task payload doesn't
    // know (an extracted task's `anchor`, `sourceId`, `owner`) survive a
    // claim that fails and returns the item to pending.
    let mut claimed = item.payload.clone();
    match (
        claimed.as_object_mut(),
        serde_json::to_value(&approved).map_err(|error| error.to_string())?,
    ) {
        (Some(stored), Value::Object(fields)) => stored.extend(fields),
        (_, fields) => claimed = fields,
    }
    review_queue::claim(vault, &item.id, Some(claimed))?;
    if let Err(error) = create_accepted(vault, item, &approved) {
        let _ = review_queue::fail(vault, &item.id, &error);
        return Err(error);
    }
    review_queue::accept(vault, &item.id)?;
    Ok(())
}

/// The accepted suggestion's task, through `vault_tasks::create_idempotent`
/// (fingerprint = the review item's, then the title slug, then a new file).
fn create_accepted(
    vault: &Path,
    item: &ReviewItem,
    approved: &AgentTaskPayload,
) -> Result<vault_tasks::CreatedTaskFile, String> {
    // A project that no longer resolves is left off rather than failing the
    // accept; the note and title still carry the work.
    let project = approved
        .project_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .and_then(|url| crate::vault_projects::task_project_link(vault, url).ok());
    let mut body = approved.note.trim().to_string();
    if !approved.evidence_excerpt.trim().is_empty() {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str("Evidence: ");
        body.push_str(approved.evidence_excerpt.trim());
    }
    let now = Utc::now().to_rfc3339();
    let origin = if approved.run_id.trim().is_empty() {
        "agent".to_string()
    } else {
        format!("run:{}", approved.run_id.trim())
    };
    let fm = vault_tasks::TaskFrontmatter {
        title: approved.title.clone(),
        status: "Not started".into(),
        priority: approved
            .priority
            .as_deref()
            .and_then(vault_tasks::normalize_priority),
        due: approved.due.clone(),
        project: project.as_ref().map(|p| p.0.clone()),
        project_url: project.map(|p| p.1),
        labels: vault_tasks::parse_labels(&approved.labels.join(",")),
        created: Some(now.clone()),
        last_synced_at: Some(now),
        origin: vault_tasks::task_origin(&origin),
        fingerprint: vault_tasks::fingerprint_for_key(&item.fingerprint),
        ..Default::default()
    };
    vault_tasks::create_idempotent(vault, fm, &body, true, vault_tasks::ExistingPolicy::Return)
}

// ── Validation ───────────────────────────────────────────────────────────

type Args = Map<String, Value>;

fn invalid(field: &str, why: &str) -> String {
    format!("Invalid {field}: {why}. Nothing was written.")
}

fn reject_unknown(op: &str, args: &Args, known: &[&str]) -> Result<(), String> {
    match args.keys().find(|key| !known.contains(&key.as_str())) {
        Some(key) => Err(format!(
            "{op}: unknown field `{key}` (expected one of: {}). Nothing was written.",
            known.join(", ")
        )),
        None => Ok(()),
    }
}

/// An optional string argument: absent or null is `None`.
fn opt_string(args: &Args, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(key, "must be a string")),
    }
}

fn required_string(args: &Args, key: &str, op: &str) -> Result<String, String> {
    opt_string(args, key)?
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("{op}: {key} is required. Nothing was written."))
}

fn opt_string_list(args: &Args, key: &str, max: usize) -> Result<Option<Vec<String>>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > max {
                return Err(invalid(key, &format!("at most {max} entries")));
            }
            items
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| invalid(key, "every entry must be a string"))
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        }
        Some(_) => Err(invalid(key, "must be a list of strings")),
    }
}

fn is_multiline(raw: &str) -> bool {
    raw.contains('\n') || raw.contains('\r')
}

fn title_value(raw: &str) -> Result<String, String> {
    if is_multiline(raw) {
        return Err(invalid("title", "must be one line"));
    }
    let title = vault_tasks::one_line(raw, usize::MAX);
    if title.is_empty() {
        return Err(invalid("title", "can't be empty"));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(invalid("title", &format!("at most {TITLE_MAX} characters")));
    }
    Ok(title)
}

fn due_value(raw: &str) -> Result<String, String> {
    vault_tasks::valid_due(Some(raw.to_string())).ok_or_else(|| invalid("due", "use YYYY-MM-DD"))
}

fn clock_value(raw: &str) -> Result<String, String> {
    vault_tasks::valid_clock(raw).ok_or_else(|| invalid("dueTime", "use 24-hour HH:MM"))
}

fn duration_value(value: &Value) -> Result<u32, String> {
    value
        .as_u64()
        .filter(|n| (1..=DURATION_MAX).contains(n))
        .map(|n| n as u32)
        .ok_or_else(|| {
            invalid(
                "durationMin",
                &format!("a whole number of minutes from 1 to {DURATION_MAX}"),
            )
        })
}

/// `P1`..`P4`, or the legacy `High` / `Medium` / `Low` (any case). Other
/// words `normalize_priority` tolerates on read (`urgent`, `none`) are
/// refused for agent writes.
fn priority_value(raw: &str) -> Result<String, String> {
    let lower = raw.trim().to_ascii_lowercase();
    match lower.as_str() {
        "p1" | "p2" | "p3" | "p4" | "high" | "medium" | "low" => {
            vault_tasks::normalize_priority(&lower)
                .ok_or_else(|| invalid("priority", "use P1 to P4"))
        }
        _ => Err(invalid("priority", "use P1 to P4")),
    }
}

fn status_value(raw: &str) -> Result<String, String> {
    if is_multiline(raw) {
        return Err(invalid("status", "must be one line"));
    }
    let status = vault_tasks::one_line(raw, usize::MAX);
    if status.is_empty() {
        return Err(invalid("status", "can't be empty"));
    }
    if status.chars().count() > STATUS_MAX {
        return Err(invalid(
            "status",
            &format!("at most {STATUS_MAX} characters"),
        ));
    }
    Ok(status)
}

fn recurrence_value(raw: &str, today: NaiveDate) -> Result<String, String> {
    if is_multiline(raw) {
        return Err(invalid("recurrence", "must be one line"));
    }
    let recurrence = raw.trim().to_string();
    if crate::quick_add::next_occurrence(&recurrence, today).is_none() {
        return Err(invalid(
            "recurrence",
            &format!("Hollow can't read \"{recurrence}\"; use a phrase like \"every week\" or \"every weekday\""),
        ));
    }
    Ok(recurrence)
}

fn short_line(field: &str, raw: &str, max: usize) -> Result<String, String> {
    if is_multiline(raw) {
        return Err(invalid(field, "must be one line"));
    }
    let value = raw.trim().to_string();
    if value.chars().count() > max {
        return Err(invalid(field, &format!("at most {max} characters")));
    }
    Ok(value)
}

fn labels_value(labels: Vec<String>) -> Result<Vec<String>, String> {
    if labels.len() > MAX_LABELS {
        return Err(invalid("labels", &format!("at most {MAX_LABELS}")));
    }
    labels
        .into_iter()
        .map(|label| {
            let label = short_line("labels", &label, LABEL_MAX)?;
            if label.is_empty() || label.contains(',') {
                return Err(invalid(
                    "labels",
                    "each label is one short word or phrase, with no commas",
                ));
            }
            Ok(label)
        })
        .collect()
}

fn description_value(raw: String) -> Result<String, String> {
    if raw.chars().count() > DESCRIPTION_MAX {
        return Err(invalid(
            "description",
            &format!("at most {DESCRIPTION_MAX} characters"),
        ));
    }
    Ok(raw)
}

fn project_url_value(vault: &Path, raw: &str) -> Result<String, String> {
    crate::vault_projects::task_project_link(vault, raw.trim())
        .map(|(_, url)| url)
        .map_err(|error| invalid("projectUrl", &error))
}

fn blockers_value(list: Vec<String>) -> Result<Vec<String>, String> {
    list.into_iter()
        .map(|entry| short_line("blockedBy", &entry, 400))
        .collect()
}

fn subtasks_value(value: &Value) -> Result<Vec<SubTask>, String> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid("subtasks", "must be a list of {text, done}"))?;
    if items.len() > MAX_SUBTASKS {
        return Err(invalid("subtasks", &format!("at most {MAX_SUBTASKS}")));
    }
    items
        .iter()
        .map(|item| {
            let text = item
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("subtasks", "every step needs text"))?;
            let text = short_line("subtasks", text, SUBTASK_MAX)?;
            if text.is_empty() {
                return Err(invalid("subtasks", "a step can't be empty"));
            }
            let done = match item.get("done") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(invalid("subtasks", "done must be true or false")),
            };
            Ok(SubTask { text, done })
        })
        .collect()
}

/// A non-empty optional string (blank is `None`).
fn present(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

fn parse_op(op: &str, args: &Value, vault: &Path, today: NaiveDate) -> Result<Op, String> {
    let args = args
        .as_object()
        .ok_or_else(|| format!("{op}: arguments must be an object. Nothing was written."))?;
    match op {
        "create_task" => parse_create(args, vault, today).map(Op::Create),
        "update_task" => parse_update(args, vault, today),
        "set_status" => {
            reject_unknown(op, args, &["ref", "status", "expectedHash"])?;
            let rel = vault_tasks::resolve_task_ref(vault, &required_string(args, "ref", op)?)?;
            let status = status_value(&required_string(args, "status", op)?)?;
            if crate::tracker_sync::seams::tracked(vault, &rel) {
                return Ok(Op::TrackerUpdate {
                    rel,
                    expected: required_string(args, "expectedHash", op)?,
                    fields: Map::from_iter([("status".into(), json!(status))]),
                });
            }
            Ok(Op::SetStatus {
                rel,
                status,
                expected: present(opt_string(args, "expectedHash")?).map(|h| h.trim().to_string()),
            })
        }
        "add_comment" => {
            reject_unknown(op, args, &["ref", "body", "idempotencyKey", "expectedHash"])?;
            let rel = vault_tasks::resolve_task_ref(vault, &required_string(args, "ref", op)?)?;
            let body = required_string(args, "body", op)?;
            let key = present(opt_string(args, "idempotencyKey")?);
            if key
                .as_ref()
                .is_some_and(|k| k.len() > vault_tasks::TASK_KEY_MAX_BYTES)
            {
                return Err(invalid("idempotencyKey", "too long"));
            }
            Ok(Op::Comment {
                rel,
                body,
                key,
                expected: present(opt_string(args, "expectedHash")?).map(|h| h.trim().to_string()),
            })
        }
        "claim_task" | "release_task" => {
            reject_unknown(op, args, &["ref", "expectedHash"])?;
            let rel = vault_tasks::resolve_task_ref(vault, &required_string(args, "ref", op)?)?;
            let expected = present(opt_string(args, "expectedHash")?);
            let expected = expected.map(|h| h.trim().to_string());
            if op == "claim_task" {
                Ok(Op::Claim { rel, expected })
            } else {
                Ok(Op::Release { rel, expected })
            }
        }
        "propose_tasks" => parse_propose(args, vault),
        "create_project" => parse_create_project(args, vault),
        "update_project" => parse_update_project(args, vault),
        "set_project_status" => {
            reject_unknown(op, args, &["ref", "status", "expectedHash"])?;
            let rel =
                crate::vault_projects::resolve_ref(vault, &required_string(args, "ref", op)?)?;
            let status = status_value(&required_string(args, "status", op)?)?;
            if crate::tracker_sync::seams::tracked(vault, &rel) {
                return Ok(Op::TrackerUpdate {
                    rel,
                    expected: required_string(args, "expectedHash", op)?,
                    fields: Map::from_iter([("status".into(), json!(status))]),
                });
            }
            Ok(Op::SetProjectStatus {
                rel,
                status,
                expected: present(opt_string(args, "expectedHash")?).map(|h| h.trim().to_string()),
            })
        }
        other => Err(format!("unknown task operation: {other}")),
    }
}

fn parse_create(args: &Args, vault: &Path, today: NaiveDate) -> Result<CreateTaskInput, String> {
    const OP: &str = "create_task";
    reject_unknown(
        OP,
        args,
        &[
            "idempotencyKey",
            "title",
            "due",
            "dueTime",
            "durationMin",
            "priority",
            "status",
            "description",
            "recurrence",
            "section",
            "labels",
            "projectUrl",
            "blockedBy",
        ],
    )?;
    let key = required_string(args, "idempotencyKey", OP)?;
    let title = title_value(
        &opt_string(args, "title")?
            .ok_or_else(|| format!("{OP}: title is required. Nothing was written."))?,
    )?;
    let due = present(opt_string(args, "due")?)
        .map(|d| due_value(&d))
        .transpose()?;
    let due_time = present(opt_string(args, "dueTime")?)
        .map(|t| clock_value(&t))
        .transpose()?;
    let duration_min = match args.get("durationMin") {
        None | Some(Value::Null) => None,
        Some(value) => Some(duration_value(value)?),
    };
    let priority = present(opt_string(args, "priority")?)
        .map(|p| priority_value(&p))
        .transpose()?;
    let status = present(opt_string(args, "status")?)
        .map(|s| status_value(&s))
        .transpose()?;
    let description = present(opt_string(args, "description")?)
        .map(description_value)
        .transpose()?;
    let recurrence = present(opt_string(args, "recurrence")?)
        .map(|r| recurrence_value(&r, today))
        .transpose()?;
    let section = present(opt_string(args, "section")?)
        .map(|s| short_line("section", &s, SECTION_MAX))
        .transpose()?;
    let labels = opt_string_list(args, "labels", MAX_LABELS)?
        .map(labels_value)
        .transpose()?
        .unwrap_or_default();
    let project_url = present(opt_string(args, "projectUrl")?)
        .map(|u| project_url_value(vault, &u))
        .transpose()?;
    let blocked_by = opt_string_list(args, "blockedBy", MAX_BLOCKERS)?
        .map(blockers_value)
        .transpose()?
        .unwrap_or_default();
    Ok(CreateTaskInput {
        title,
        due,
        due_time,
        duration_min,
        priority,
        status,
        description,
        recurrence,
        section,
        labels,
        project: None,
        project_url,
        idempotency_key: Some(key),
        origin: None,
        blocked_by,
    })
}

fn parse_update(args: &Args, vault: &Path, today: NaiveDate) -> Result<Op, String> {
    const OP: &str = "update_task";
    reject_unknown(OP, args, &["ref", "expectedHash", "fields"])?;
    let rel = vault_tasks::resolve_task_ref(vault, &required_string(args, "ref", OP)?)?;
    let expected = required_string(args, "expectedHash", OP).map_err(|_| {
        format!(
            "{OP}: expectedHash is required (the contentHash you last read). Nothing was written."
        )
    })?;
    let fields = args
        .get("fields")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            format!("{OP}: fields must be an object of the fields to change. Nothing was written.")
        })?;
    if fields.is_empty() {
        return Err(format!("{OP}: fields is empty; nothing to change."));
    }
    if crate::tracker_sync::seams::tracked(vault, &rel) {
        if fields.contains_key("status") {
            return Err(
                "update_task cannot change status; use set_status with expectedHash.".into(),
            );
        }
        return Ok(Op::TrackerUpdate {
            rel,
            expected,
            fields: fields.clone(),
        });
    }
    let mut input = UpdateTaskInput {
        expected: Some(expected.trim().to_string()),
        url: format!("vault:{rel}"),
        ..Default::default()
    };
    let mut touched: Vec<&'static str> = Vec::new();
    let mut exact = std::collections::BTreeMap::new();
    // `Some("")` clears a text field (apply_update's rule).
    let clearable = |key: &str,
                     check: &dyn Fn(&str) -> Result<String, String>|
     -> Result<Option<String>, String> {
        match opt_string(fields, key)? {
            None => Ok(None),
            Some(raw) if raw.trim().is_empty() => Ok(Some(String::new())),
            Some(raw) => check(&raw).map(Some),
        }
    };
    for key in fields.keys() {
        match key.as_str() {
            "status" => {
                return Err(format!(
                    "{OP} can't change the status; use set_status (it rolls recurring tasks forward). Nothing was written."
                ))
            }
            "project" => {
                return Err(format!(
                    "{OP}: set projectUrl (a project's ref from list_projects), not a project name. Nothing was written."
                ))
            }
            "projectUrl" => {
                let raw = required_string(fields, "projectUrl", OP)?;
                let rel = crate::vault_projects::resolve_ref(vault, &raw)?;
                exact.insert("project_url".to_string(), format!("vault:{rel}"));
                touched.push("project");
            }
            "title" => {
                input.title = Some(title_value(&opt_string(fields, "title")?.unwrap_or_default())?);
                touched.push("title");
            }
            "priority" => {
                input.priority = clearable("priority", &priority_value)?;
                if let Some(raw) = opt_string(fields, "priority")?.filter(|r| !r.trim().is_empty()) {
                    exact.insert("priority".to_string(), raw.trim().to_string());
                }
                touched.push("priority");
            }
            "due" => {
                input.due = clearable("due", &due_value)?;
                touched.push("due");
            }
            "dueTime" => {
                input.due_time = clearable("dueTime", &clock_value)?;
                touched.push("dueTime");
            }
            "durationMin" => {
                input.duration_min = match fields.get("durationMin") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(duration_value(value)?),
                };
            }
            "recurrence" => input.recurrence = clearable("recurrence", &|r: &str| recurrence_value(r, today))?,
            "section" => input.section = clearable("section", &|s: &str| short_line("section", s, SECTION_MAX))?,
            "labels" => input.labels = opt_string_list(fields, "labels", MAX_LABELS)?.map(labels_value).transpose()?,
            "description" => {
                input.description = opt_string(fields, "description")?.map(description_value).transpose()?;
                touched.push("description");
            }
            "subtasks" => {
                input.subtasks = match fields.get("subtasks") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(subtasks_value(value)?),
                };
            }
            // PR 11b's checks run inside apply_update: TASK_BLOCKER / TASK_CYCLE
            // come back as tool errors and nothing is written.
            "blockedBy" => {
                input.blocked_by = opt_string_list(fields, "blockedBy", MAX_BLOCKERS)?
                    .map(blockers_value)
                    .transpose()?
            }
            other => {
                return Err(format!(
                    "{OP}: unknown field `{other}` (title, priority, due, dueTime, durationMin, recurrence, section, labels, description, subtasks, blockedBy). Nothing was written."
                ))
            }
        }
    }
    Ok(Op::Update {
        rel,
        input,
        touched,
        exact,
    })
}

/// Vault-only fields accepted beside tracker changes. Validate before any
/// journal save; the shared renderer writes these in the same file CAS.
pub(crate) fn tracker_local_value(key: &str, value: &Value) -> Result<Value, String> {
    if key == "duration_min" {
        return if value.is_null() {
            Ok(Value::Null)
        } else {
            duration_value(value).map(|v| json!(v))
        };
    }
    let text = if value.is_null() {
        ""
    } else {
        value.as_str().ok_or_else(|| invalid(key, "must be text"))?
    };
    if text.is_empty() {
        return Ok(json!(""));
    }
    let text = match key {
        "due_time" => clock_value(text)?,
        "section" => short_line("section", text, SECTION_MAX)?,
        "recurrence" => recurrence_value(text, Local::now().date_naive())?,
        _ => return Err(invalid(key, "read-only")),
    };
    Ok(json!(text))
}

/// The project fields `create_project` and `update_project` share, onto
/// `(title, priority, target_date, description, outcome…)` setters.
fn project_fields(
    fields: &Args,
    op: &str,
    input: &mut crate::types::UpdateProjectInput,
) -> Result<(), String> {
    for key in fields.keys() {
        match key.as_str() {
            "title" => {
                input.title = Some(title_value(&opt_string(fields, "title")?.unwrap_or_default())?)
            }
            "priority" => {
                let raw = opt_string(fields, "priority")?.unwrap_or_default();
                input.priority = Some(if raw.trim().is_empty() {
                    String::new()
                } else {
                    priority_value(&raw)?
                });
            }
            "targetDate" => {
                let raw = opt_string(fields, "targetDate")?.unwrap_or_default();
                input.target_date = Some(if raw.trim().is_empty() {
                    String::new()
                } else {
                    vault_tasks::valid_due(Some(raw))
                        .ok_or_else(|| invalid("targetDate", "use YYYY-MM-DD"))?
                });
            }
            "description" => input.description = opt_string(fields, "description")?,
            "currentState" => input.current_state = opt_string(fields, "currentState")?,
            "desiredState" => input.desired_state = opt_string(fields, "desiredState")?,
            "nonGoals" => input.non_goals = opt_string(fields, "nonGoals")?,
            "successEvidence" => input.success_evidence = opt_string(fields, "successEvidence")?,
            "goalUrls" => input.goal_urls = opt_string_list(fields, "goalUrls", 50)?,
            "status" => {
                return Err(format!(
                    "{op} can't change the status; use set_project_status. Nothing was written."
                ))
            }
            other => {
                return Err(format!(
                    "{op}: unknown field `{other}` (expected title, priority, targetDate, description, currentState, desiredState, nonGoals, successEvidence, goalUrls). Nothing was written."
                ))
            }
        }
    }
    Ok(())
}

fn parse_update_project(args: &Args, vault: &Path) -> Result<Op, String> {
    const OP: &str = "update_project";
    reject_unknown(OP, args, &["ref", "expectedHash", "fields"])?;
    let rel = crate::vault_projects::resolve_ref(vault, &required_string(args, "ref", OP)?)?;
    let expected = required_string(args, "expectedHash", OP).map_err(|_| {
        format!(
            "{OP}: expectedHash is required (the contentHash you last read). Nothing was written."
        )
    })?;
    let fields = args
        .get("fields")
        .and_then(Value::as_object)
        .filter(|f| !f.is_empty())
        .ok_or_else(|| {
            format!("{OP}: fields must be an object of the fields to change. Nothing was written.")
        })?;
    if crate::tracker_sync::seams::tracked(vault, &rel) {
        return Ok(Op::TrackerUpdate {
            rel,
            expected,
            fields: fields.clone(),
        });
    }
    let mut input = crate::types::UpdateProjectInput {
        url: format!("vault:{rel}"),
        ..Default::default()
    };
    project_fields(fields, OP, &mut input)?;
    Ok(Op::UpdateProject {
        rel,
        input,
        expected: expected.trim().to_string(),
    })
}

fn parse_create_project(args: &Args, _vault: &Path) -> Result<Op, String> {
    const OP: &str = "create_project";
    reject_unknown(
        OP,
        args,
        &[
            "idempotencyKey",
            "title",
            "status",
            "priority",
            "targetDate",
            "description",
            "currentState",
            "desiredState",
            "nonGoals",
            "successEvidence",
            "goalUrls",
        ],
    )?;
    let key = required_string(args, "idempotencyKey", OP)?;
    if key.len() > vault_tasks::TASK_KEY_MAX_BYTES {
        return Err(invalid("idempotencyKey", "too long"));
    }
    let title = title_value(&required_string(args, "title", OP)?)?;
    let status = present(opt_string(args, "status")?)
        .map(|s| status_value(&s))
        .transpose()?;
    let mut fields = args.clone();
    for key in ["idempotencyKey", "title", "status"] {
        fields.remove(key);
    }
    let mut update = crate::types::UpdateProjectInput::default();
    project_fields(&fields, OP, &mut update)?;
    let input = crate::types::CreateProjectInput {
        title,
        status,
        priority: update.priority.filter(|p| !p.is_empty()),
        description: update.description,
        target_date: update.target_date.filter(|d| !d.is_empty()),
        current_state: update.current_state,
        desired_state: update.desired_state,
        non_goals: update.non_goals,
        success_evidence: update.success_evidence,
        goal_urls: update.goal_urls.unwrap_or_default(),
    };
    Ok(Op::CreateProject {
        input,
        key: key.trim().to_string(),
    })
}

fn parse_propose(args: &Args, vault: &Path) -> Result<Op, String> {
    const OP: &str = "propose_tasks";
    reject_unknown(OP, args, &["items"])?;
    let items = args
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{OP}: items must be a list. Nothing was queued."))?;
    if items.is_empty() {
        return Err(format!("{OP}: items is empty. Nothing was queued."));
    }
    if items.len() > MAX_PROPOSE_PER_CALL {
        return Err(format!(
            "{OP}: at most {MAX_PROPOSE_PER_CALL} items per call (got {}). Nothing was queued.",
            items.len()
        ));
    }
    let mut drafts = Vec::with_capacity(items.len());
    for item in items {
        let item = item
            .as_object()
            .ok_or_else(|| format!("{OP}: every item must be an object. Nothing was queued."))?;
        reject_unknown(
            OP,
            item,
            &[
                "title",
                "due",
                "priority",
                "note",
                "projectUrl",
                "labels",
                "evidence",
            ],
        )?;
        let title = title_value(
            &opt_string(item, "title")?
                .ok_or_else(|| format!("{OP}: every item needs a title. Nothing was queued."))?,
        )?;
        let note = opt_string(item, "note")?.unwrap_or_default();
        if note.chars().count() > NOTE_MAX {
            return Err(invalid("note", &format!("at most {NOTE_MAX} characters")));
        }
        let evidence = opt_string(item, "evidence")?.unwrap_or_default();
        drafts.push(ProposalDraft {
            title,
            due: present(opt_string(item, "due")?)
                .map(|d| due_value(&d))
                .transpose()?,
            priority: present(opt_string(item, "priority")?)
                .map(|p| priority_value(&p))
                .transpose()?,
            note: note.trim().to_string(),
            project_url: present(opt_string(item, "projectUrl")?)
                .map(|u| project_url_value(vault, &u))
                .transpose()?,
            labels: opt_string_list(item, "labels", MAX_LABELS)?
                .map(labels_value)
                .transpose()?
                .unwrap_or_default(),
            evidence: vault_tasks::one_line(&evidence, EVIDENCE_MAX),
        });
    }
    Ok(Op::Propose(drafts))
}

// ── Socket ───────────────────────────────────────────────────────────────

/// Bind the broker socket and start serving. Called once from `lib.rs`
/// setup (configured vaults only). Non-fatal on failure: without a socket
/// `tasks_mcp::server_entry` attaches nothing.
#[cfg(unix)]
pub(crate) fn start(app: impl Into<crate::core_host::Core>, vault: PathBuf) {
    let app = app.into();
    let dir = crate::instance::cache_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("[task broker] cannot create {}: {error}", dir.display());
        return;
    }
    // Per-pid, like the approvals socket: two Hollow processes never share
    // one, and a stale file from a crashed run never shadows a live broker.
    let path = dir.join(format!("tasks-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("[task broker] bind {} failed: {error}", path.display());
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = Some(path.clone());
    let on_review: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(move |count| {
        let _ = app.emit(
            REVIEW_EVENT,
            json!({ "kind": AGENT_TASK_KIND, "count": count }),
        );
    });
    let on_stop: Box<dyn FnOnce() + Send> = Box::new(move || {
        let mut current = SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner());
        if current.as_deref() == Some(path.as_path()) {
            *current = None;
        }
        let _ = std::fs::remove_file(&path);
        eprintln!("[task broker] stopped; task tools stay detached until Hollow restarts");
    });
    let peers = crate::run_auth::global();
    if !serve_listener(
        listener,
        vault,
        on_review,
        tasks_mcp::enabled,
        peers,
        on_stop,
    ) {
        *SOCKET_PATH.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Consecutive accept errors the listener rides out before it stops.
#[cfg(unix)]
const MAX_ACCEPT_ERRORS: u32 = 20;

/// Whether an `accept` error is worth retrying: descriptor or buffer
/// pressure, an interrupted call, or a peer that gave up while queued. Any
/// other error stops the listener.
#[cfg(unix)]
fn transient_accept_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::Interrupted
            | ErrorKind::WouldBlock
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
    ) || matches!(
        error.raw_os_error(),
        Some(code) if code == libc::EMFILE || code == libc::ENFILE || code == libc::ENOBUFS || code == libc::ENOMEM
    )
}

#[cfg(not(unix))]
pub(crate) fn start(_app: impl Into<crate::core_host::Core>, _vault: PathBuf) {}

/// Accept connections on a background thread; each is served on its own
/// thread: one request line in, one answer line out. `enabled` is read per
/// request (the live Settings switch in production). A transient accept
/// error is logged and retried after a short backoff; an unrecoverable one,
/// or [`MAX_ACCEPT_ERRORS`] in a row, ends the loop and runs `on_stop` (which
/// clears the socket path in production). Every request must come from
/// inside the tree of the run its proof names (`run_auth::check_peer`,
/// merged plan slice 6); `peers` is the global registry in production.
#[cfg(unix)]
pub(crate) fn serve_listener(
    listener: std::os::unix::net::UnixListener,
    vault: PathBuf,
    on_review: Arc<dyn Fn(usize) + Send + Sync>,
    enabled: fn() -> bool,
    peers: &'static crate::run_auth::Peers,
    on_stop: Box<dyn FnOnce() + Send>,
) -> bool {
    let spawned = std::thread::Builder::new()
        .name("hollow-task-broker".into())
        .spawn(move || {
            let mut errors = 0u32;
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        errors = 0;
                        let (vault, on_review) = (vault.clone(), on_review.clone());
                        let _ = std::thread::Builder::new()
                            .name("hollow-task-request".into())
                            .spawn(move || {
                                handle_conn(stream, &vault, on_review.as_ref(), enabled, peers)
                            });
                    }
                    Err(error) => {
                        errors += 1;
                        if !transient_accept_error(&error) || errors >= MAX_ACCEPT_ERRORS {
                            eprintln!("[task broker] accept: {error}; stopping");
                            break;
                        }
                        eprintln!("[task broker] accept: {error}; retrying");
                        std::thread::sleep(std::time::Duration::from_millis(
                            50 * u64::from(errors),
                        ));
                    }
                }
            }
            on_stop();
        });
    match spawned {
        Ok(_) => true,
        Err(error) => {
            eprintln!("[task broker] cannot start: {error}");
            false
        }
    }
}

#[cfg(unix)]
fn handle_conn(
    stream: std::os::unix::net::UnixStream,
    vault: &Path,
    on_review: &(dyn Fn(usize) + Send + Sync),
    enabled: fn() -> bool,
    peers: &crate::run_auth::Peers,
) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::fd::AsRawFd;
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut line = String::new();
    {
        let mut reader = BufReader::new((&stream).take(REQUEST_MAX_BYTES));
        if reader.read_line(&mut line).is_err() {
            return;
        }
    }
    let fd = stream.as_raw_fd();
    let admit = |req: &BrokerRequest| {
        crate::run_auth::check_peer(peers, fd, "task broker", Some(&req.run_id))
            .map_err(|refused| format!("{refused} Nothing was written."))
    };
    let now = Local::now();
    let answer = match serde_json::from_str::<BrokerRequest>(line.trim()) {
        Ok(req) => match admit(&req).and_then(|()| dispatch_at(vault, &req, enabled(), now)) {
            Ok(done) => {
                if done.review_changed > 0 {
                    on_review(done.review_changed);
                }
                json!({ "ok": true, "result": done.result })
            }
            Err(error) => json!({ "ok": false, "error": error }),
        },
        Err(error) => {
            json!({ "ok": false, "error": format!("unreadable task request ({error}); nothing was written.") })
        }
    };
    let mut out = answer.to_string();
    out.push('\n');
    let mut stream = stream;
    let _ = stream.write_all(out.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automated_history::{self, HistoryRunHandle};
    use crate::tasks_mcp::{BrokerClient, Ctx, RunIdentity};
    use chrono::TimeZone;

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn put(dir: &Path, rel: &str, body: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 26, 10, 0, 0).unwrap()
    }

    fn task_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir.join("tasks"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn begin(v: &Path, surface: &str, scope: &str) -> HistoryRunHandle {
        put(v, "notes/seed.md", "seed");
        automated_history::try_begin_run(v, surface, Some(scope)).unwrap()
    }

    fn identity(run: &str, surface: &str, scope: &str, access: TasksAccess) -> RunIdentity {
        RunIdentity {
            run_id: run.into(),
            surface: surface.into(),
            scope: scope.into(),
            access,
            proof: proof_for(run, surface, scope, access),
        }
    }

    fn request(identity: &RunIdentity, op: &str, args: Value) -> BrokerRequest {
        serde_json::from_value(identity.request(op, &args)).unwrap()
    }

    fn run(v: &Path, identity: &RunIdentity, op: &str, args: Value) -> Result<Value, String> {
        dispatch_at(v, &request(identity, op, args), true, now()).map(|d| d.result)
    }

    /// The MCP server wired to the broker in-process, as the socket would.
    struct InProcess<'a> {
        vault: &'a Path,
        identity: RunIdentity,
    }

    impl BrokerClient for InProcess<'_> {
        fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
            run(self.vault, &self.identity, op, args.clone())
        }
    }

    fn mcp(v: &Path, client: &dyn BrokerClient, name: &str, args: Value) -> Result<Value, String> {
        let ctx = Ctx {
            vault_root: Some(v.to_path_buf()),
            access: TasksAccess::ReadWrite,
            client: Some(client),
        };
        let resp = tasks_mcp::handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }), &ctx).unwrap();
        let text = resp["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        if resp["result"]["isError"] == json!(true) {
            Err(text)
        } else {
            Ok(serde_json::from_str(&text).unwrap())
        }
    }

    const TASK: &str =
        "---\ntitle: Draft the brief\nstatus: Not started\npriority: P3\n---\n\nThe brief.\n";

    /// Plan completion L2: a guarded retry refuses the exact task write the
    /// run it retries already made, and writes nothing; a different write
    /// still goes through.
    #[test]
    fn a_guarded_retry_never_repeats_the_same_task_write() {
        let dir = vault();
        let v = dir.path();
        let handle = begin(v, "workflow", "guarded");
        let run_id = handle.id.clone();
        let args = json!({ "title": "Call the vendor", "idempotencyKey": "k-1" });
        let effects = crate::run_effects::classify(&[crate::run_effects::Call {
            name: "mcp__hollow_tasks__create_task",
            input: &args,
            result: Some("{}"),
            is_error: false,
            sequence: 1,
        }])
        .unwrap();
        let guard = crate::run_effects::guard_for(&run_id, "run-failed", &effects).unwrap();
        crate::run_effects::write_guard(&guard).unwrap();
        let who = identity(&run_id, "workflow", "guarded", TasksAccess::ReadWrite);
        let before = task_files(v);
        let refused = run(v, &who, "create_task", args.clone()).unwrap_err();
        assert!(refused.starts_with("Not repeated"), "{refused}");
        assert_eq!(task_files(v), before, "nothing was written");
        crate::run_effects::remove_guard(&run_id);
        assert!(run(
            v,
            &who,
            "create_task",
            json!({ "title": "Another", "idempotencyKey": "k-2" })
        )
        .is_ok());
        let _ = automated_history::finish_run(v, handle, "success");
    }

    /// 2026-09-29: a workspace's conversation (a scoped run, which holds
    /// only its folder) writes tasks too. Each write takes the vault in its
    /// name for the call and lands in that conversation's History.
    #[test]
    fn a_workspace_conversation_writes_tasks_into_its_own_history() {
        let dir = vault();
        let v = dir.path();
        put(v, "workspaces/w1/WORKSPACE.md", "brief");
        let lease =
            automated_history::try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        let who = identity(lease.id(), "chat", "s-1", TasksAccess::ReadWrite);
        let created = run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k1", "title": "From the workspace" }),
        )
        .unwrap();
        assert!(
            automated_history::lease_holder(v).is_none(),
            "the vault is free again after the write"
        );
        put(v, "workspaces/w1/PLAN.md", "plan");
        let run_id = lease.id().to_string();
        let finished = lease.finish("success").unwrap();
        let paths: Vec<&str> = finished
            .changes
            .iter()
            .map(|c| c.vault_path.as_str())
            .collect();
        assert!(
            paths.iter().any(|p| p.starts_with("tasks/")),
            "{paths:?} {created}"
        );
        assert!(paths.contains(&"workspaces/w1/PLAN.md"), "{paths:?}");
        assert!(finished.flags.is_empty(), "{:?}", finished.flags);
        let late = identity(&run_id, "chat", "s-1", TasksAccess::ReadWrite);
        assert_eq!(
            run(
                v,
                &late,
                "create_task",
                json!({ "idempotencyKey": "k2", "title": "Too late" })
            )
            .unwrap_err(),
            automated_history::LIVE_RUN_FINISHED_ERROR
        );
    }

    /// Chat concurrency F7 (2026-10-20, red first): a workspace
    /// conversation's task writes snapshot only `tasks/` and `projects/`,
    /// not the whole vault twice each, and its History holds exactly the
    /// task they wrote.
    #[test]
    fn a_workspace_conversations_task_writes_walk_only_the_task_folders() {
        let dir = vault();
        let v = dir.path();
        put(v, "workspaces/w1/WORKSPACE.md", "brief");
        for i in 0..300 {
            put(v, &format!("notes/n{i}.md"), "a note");
        }
        let lease =
            automated_history::try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        let who = identity(lease.id(), "chat", "s-1", TasksAccess::ReadWrite);
        automated_history::take_snapshot_visits();
        let created = run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k1", "title": "From the workspace" }),
        )
        .unwrap();
        let task = created["ref"].as_str().unwrap().to_string();
        run(
            v,
            &who,
            "set_status",
            json!({ "ref": task, "status": "In progress" }),
        )
        .unwrap();
        run(
            v,
            &who,
            "add_comment",
            json!({ "ref": task, "body": "Started." }),
        )
        .unwrap();
        let visits = automated_history::take_snapshot_visits();
        assert!(
            visits <= 20,
            "three task writes walked {visits} entries; the vault holds 300 notes"
        );
        let finished = lease.finish("success").unwrap();
        let paths: Vec<&str> = finished
            .changes
            .iter()
            .map(|c| c.vault_path.as_str())
            .collect();
        assert_eq!(paths, [task.trim_start_matches("vault:")]);
    }

    /// F7 review (2026-10-20, red first): a link inside a declared folder
    /// (`projects/active` → `notes/`) sends a project create and a status
    /// move to a file outside it. The narrow walk never follows the link, so
    /// the write snapshots the whole vault instead, and History holds the
    /// note the link's target gained as well as the move's source.
    #[cfg(unix)]
    #[test]
    fn a_project_write_through_a_linked_folder_is_still_recorded() {
        let dir = vault();
        let v = dir.path();
        put(v, "workspaces/w1/WORKSPACE.md", "brief");
        put(v, "notes/seed.md", "seed");
        std::fs::create_dir_all(v.join("projects/discovery")).unwrap();
        std::os::unix::fs::symlink("../notes", v.join("projects/active")).unwrap();
        let lease =
            automated_history::try_begin_scoped(v, "workspaces/w1", "chat", Some("s-1")).unwrap();
        let who = identity(lease.id(), "chat", "s-1", TasksAccess::ReadWrite);
        let made = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k1", "title": "Linked", "status": "active" }),
        )
        .unwrap();
        assert_eq!(made["ref"], json!("vault:projects/active/Linked.md"));
        assert!(
            v.join("notes/Linked.md").is_file(),
            "landed through the link"
        );
        let moved = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k2", "title": "Mover" }),
        )
        .unwrap();
        assert_eq!(moved["ref"], json!("vault:projects/discovery/Mover.md"));
        run(
            v,
            &who,
            "set_project_status",
            json!({ "ref": moved["ref"], "status": "active" }),
        )
        .unwrap();
        assert!(v.join("notes/Mover.md").is_file(), "moved through the link");
        let finished = lease.finish("success").unwrap();
        let paths: Vec<&str> = finished
            .changes
            .iter()
            .map(|c| c.vault_path.as_str())
            .collect();
        assert_eq!(paths, ["notes/Linked.md", "notes/Mover.md"], "{paths:?}");
    }

    #[test]
    fn writes_require_a_valid_proof_and_the_live_lease() {
        let dir = vault();
        let v = dir.path();
        let create = |n: &str| json!({ "idempotencyKey": n, "title": format!("Task {n}") });

        // No lease at all.
        let nobody = identity("run-nobody", "chat", "s-1", TasksAccess::ReadWrite);
        assert_eq!(
            run(v, &nobody, "create_task", create("0")).unwrap_err(),
            automated_history::LIVE_RUN_FINISHED_ERROR
        );

        let handle = begin(v, "chat", "s-1");
        let good = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        // A wrong proof, and a proof minted for another scope.
        let mut forged = good.clone();
        forged.proof = "0".repeat(64);
        assert_eq!(
            run(v, &forged, "create_task", create("1")).unwrap_err(),
            PROOF_REFUSED
        );
        let mut rescoped = good.clone();
        rescoped.scope = "s-2".into();
        assert_eq!(
            run(v, &rescoped, "create_task", create("1")).unwrap_err(),
            PROOF_REFUSED
        );
        // A read-only proof never writes.
        let reader = identity(&handle.id, "chat", "s-1", TasksAccess::ReadOnly);
        assert_eq!(
            run(v, &reader, "create_task", create("1")).unwrap_err(),
            READ_ONLY_REFUSED
        );
        // The switch off.
        let off = dispatch_at(v, &request(&good, "create_task", create("1")), false, now());
        assert_eq!(off.unwrap_err(), SWITCHED_OFF);
        assert!(task_files(v).is_empty());

        // The right run, proof and access: applied.
        let created = run(v, &good, "create_task", create("1")).unwrap();
        assert_eq!(created["disposition"], json!("applied"));
        assert_eq!(task_files(v), ["task-1.md"]);

        // Another run id with a valid proof of its own.
        let other = identity("run-other", "chat", "s-1", TasksAccess::ReadWrite);
        assert_eq!(
            run(v, &other, "create_task", create("2")).unwrap_err(),
            automated_history::LIVE_RUN_FINISHED_ERROR
        );

        let finished = automated_history::finish_run(v, handle, "success").unwrap();
        assert!(
            finished
                .changes
                .iter()
                .any(|c| c.vault_path == "tasks/task-1.md"),
            "the write is in the run's diff"
        );
        assert_eq!(
            run(v, &good, "create_task", create("3")).unwrap_err(),
            automated_history::LIVE_RUN_FINISHED_ERROR
        );
        assert_eq!(task_files(v), ["task-1.md"]);
    }

    #[test]
    fn heartbeat_and_ingest_leases_cannot_write_tasks() {
        let dir = vault();
        let v = dir.path();
        for surface in ["heartbeat", "ingest", "oneshot"] {
            let handle = begin(v, surface, "beat");
            let forged = identity(&handle.id, surface, "beat", TasksAccess::ReadWrite);
            let refused = run(
                v,
                &forged,
                "create_task",
                json!({ "idempotencyKey": "k", "title": "Sneak" }),
            )
            .unwrap_err();
            assert!(
                refused.contains("only to Hollow chat turns and automations"),
                "{surface}: {refused}"
            );
            automated_history::finish_run(v, handle, "success").unwrap();
        }
        assert!(task_files(v).is_empty());
    }

    /// 2026-11-13: what the broker writes for a run is marked as that run's
    /// own in its History (`HistoryChange::hollow_write`), which the quiet
    /// check credits; a file the run changed some other way is not.
    #[test]
    fn broker_writes_are_marked_as_the_runs_own_in_history() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "workflow", "brief");
        let who = identity(&handle.id, "workflow", "brief", TasksAccess::ReadWrite);
        let hash = vault_tasks::content_hash(TASK);
        run(v, &who, "update_task", json!({ "ref": "vault:tasks/draft-the-brief.md", "expectedHash": hash, "fields": { "priority": "high" } })).unwrap();
        run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k", "title": "Fresh task" }),
        )
        .unwrap();
        put(v, "tasks/by-hand.md", "written without the broker");
        let finished = automated_history::finish_run(v, handle, "success").unwrap();
        let marked: Vec<(&str, bool)> = finished
            .changes
            .iter()
            .map(|c| (c.vault_path.as_str(), c.hollow_write))
            .collect();
        assert_eq!(marked.len(), 3, "{marked:?}");
        for (path, hollow_write) in &marked {
            assert_eq!(*hollow_write, *path != "tasks/by-hand.md", "{marked:?}");
        }
    }

    #[test]
    fn invalid_fields_are_refused_and_nothing_is_written() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "workflow", "brief");
        let who = identity(&handle.id, "workflow", "brief", TasksAccess::ReadWrite);
        let hash = vault_tasks::content_hash(TASK);
        let bad: Vec<(Value, &str)> = vec![
            (json!({ "due": "next Friday" }), "due"),
            (json!({ "dueTime": "4pm" }), "dueTime"),
            (json!({ "durationMin": 0 }), "durationMin"),
            (json!({ "durationMin": 5000 }), "durationMin"),
            (json!({ "recurrence": "whenever" }), "recurrence"),
            (json!({ "title": "Two\nlines" }), "title"),
            (json!({ "title": "   " }), "title"),
            (json!({ "priority": "urgent" }), "priority"),
            (json!({ "labels": ["a,b"] }), "labels"),
        ];
        for (fields, field) in &bad {
            let mut create = json!({ "idempotencyKey": "k", "title": "Fine title" });
            for (k, val) in fields.as_object().unwrap() {
                create[k] = val.clone();
            }
            let error = run(v, &who, "create_task", create).unwrap_err();
            assert!(
                error.contains(&format!("Invalid {field}")),
                "create {fields}: {error}"
            );
            let error = run(v, &who, "update_task", json!({ "ref": "vault:tasks/draft-the-brief.md", "expectedHash": hash, "fields": fields })).unwrap_err();
            assert!(
                error.contains(&format!("Invalid {field}")),
                "update {fields}: {error}"
            );
        }
        let error = run(v, &who, "create_task", json!({ "idempotencyKey": "k", "title": "X", "projectUrl": "vault:projects/active/Nope.md" })).unwrap_err();
        assert!(error.contains("Invalid projectUrl"), "{error}");
        let error = run(v, &who, "create_task", json!({ "idempotencyKey": "k" })).unwrap_err();
        assert!(error.contains("title is required"), "{error}");
        let error = run(v, &who, "create_task", json!({ "title": "No key" })).unwrap_err();
        assert!(error.contains("idempotencyKey is required"), "{error}");
        let error = run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k", "title": "X", "project": "Launch" }),
        )
        .unwrap_err();
        assert!(error.contains("unknown field `project`"), "{error}");
        assert_eq!(task_files(v), ["draft-the-brief.md"]);
        assert_eq!(read(v, "tasks/draft-the-brief.md"), TASK);

        // Legacy High / Medium / Low still read as a priority.
        let created = run(v, &who, "create_task", json!({ "idempotencyKey": "k", "title": "Fine title", "priority": "High", "due": "2026-10-01", "dueTime": "09:30", "durationMin": 45, "recurrence": "every week", "labels": ["ops"] })).unwrap();
        let rel = created["ref"]
            .as_str()
            .unwrap()
            .trim_start_matches("vault:")
            .to_string();
        let task = vault_tasks::get(v, &rel).unwrap();
        assert_eq!(task.priority.as_deref(), Some("P1"));
        assert_eq!(task.due_time.as_deref(), Some("09:30"));
        assert_eq!(task.duration_min, Some(45));
        assert_eq!(
            task.origin.as_deref(),
            Some(format!("run:{}", handle.id).as_str())
        );
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn update_task_conflicts_when_the_gui_wrote_between_read_and_write() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let client = InProcess {
            vault: v,
            identity: identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite),
        };

        let read_back = mcp(
            v,
            &client,
            "get_task",
            json!({ "ref": "vault:tasks/draft-the-brief.md" }),
        )
        .unwrap();
        let h1 = read_back["contentHash"].as_str().unwrap().to_string();
        // The GUI (the Tasks page) writes in between.
        vault_tasks::set_priority(v, "tasks/draft-the-brief.md", Some("P1")).unwrap();
        let after_gui = read(v, "tasks/draft-the-brief.md");

        let conflict = mcp(v, &client, "update_task", json!({ "ref": "vault:tasks/draft-the-brief.md", "expectedHash": h1, "fields": { "title": "Renamed" } })).unwrap_err();
        assert!(conflict.starts_with("TASK_CONFLICT"), "{conflict}");
        assert_eq!(
            read(v, "tasks/draft-the-brief.md"),
            after_gui,
            "the GUI's write survives"
        );
        let task = vault_tasks::get(v, "tasks/draft-the-brief.md").unwrap();
        assert_eq!(
            (task.title.as_str(), task.priority.as_deref()),
            ("Draft the brief", Some("P1"))
        );

        // Re-read, reapply: it lands.
        let fresh = vault_tasks::content_hash(&after_gui);
        let updated = mcp(v, &client, "update_task", json!({ "ref": "vault:tasks/draft-the-brief.md", "expectedHash": fresh, "fields": { "title": "Renamed" } })).unwrap();
        assert_eq!(
            updated["contentHash"],
            json!(vault_tasks::content_hash(&read(
                v,
                "tasks/draft-the-brief.md"
            )))
        );
        assert_eq!(
            vault_tasks::get(v, "tasks/draft-the-brief.md")
                .unwrap()
                .title,
            "Renamed"
        );
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn idempotency_keys_are_scoped_to_the_conversation() {
        let dir = vault();
        let v = dir.path();
        let handle = begin(v, "chat", "s-a");
        let a = identity(&handle.id, "chat", "s-a", TasksAccess::ReadWrite);
        let first = run(
            v,
            &a,
            "create_task",
            json!({ "idempotencyKey": "1", "title": "Book the venue" }),
        )
        .unwrap();
        assert_eq!(first["disposition"], json!("applied"));
        automated_history::finish_run(v, handle, "success").unwrap();

        // Session B reuses the generic key for other work: a new task.
        let handle = begin(v, "chat", "s-b");
        let b = identity(&handle.id, "chat", "s-b", TasksAccess::ReadWrite);
        let other = run(
            v,
            &b,
            "create_task",
            json!({ "idempotencyKey": "1", "title": "Order the cake" }),
        )
        .unwrap();
        assert_eq!(other["disposition"], json!("applied"));
        assert_ne!(other["ref"], first["ref"]);
        automated_history::finish_run(v, handle, "success").unwrap();

        // A retried turn in session A: the same task back, with its state.
        let handle = begin(v, "chat", "s-a");
        let a = identity(&handle.id, "chat", "s-a", TasksAccess::ReadWrite);
        let again = run(
            v,
            &a,
            "create_task",
            json!({ "idempotencyKey": "1", "title": "Book the venue" }),
        )
        .unwrap();
        assert_eq!(again["disposition"], json!("duplicate"));
        assert_eq!(again["ref"], first["ref"]);
        assert_eq!(again["title"], json!("Book the venue"));
        assert_eq!(again["status"], json!("Not started"));
        let mismatch = run(
            v,
            &a,
            "create_task",
            json!({ "idempotencyKey": "1", "title": "Something else" }),
        )
        .unwrap_err();
        assert!(mismatch.starts_with("IDEMPOTENCY_MISMATCH"), "{mismatch}");
        assert_eq!(task_files(v).len(), 2);
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn only_transient_accept_errors_are_retried() {
        use std::io::{Error, ErrorKind};
        for code in [
            libc::EMFILE,
            libc::ENFILE,
            libc::ENOBUFS,
            libc::ENOMEM,
            libc::EINTR,
            libc::ECONNABORTED,
        ] {
            assert!(
                transient_accept_error(&Error::from_raw_os_error(code)),
                "{code}"
            );
        }
        assert!(transient_accept_error(&Error::from(ErrorKind::WouldBlock)));
        for code in [libc::EBADF, libc::EINVAL, libc::ENOTSOCK] {
            assert!(
                !transient_accept_error(&Error::from_raw_os_error(code)),
                "{code}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn socket_round_trip_creates_once_and_reports_duplicate() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let dir = vault();
        let v = dir.path().to_path_buf();
        let sock_dir = tempfile::Builder::new().prefix("tb").tempdir().unwrap();
        let sock = sock_dir.path().join("t.sock");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let reviews = Arc::new(AtomicUsize::new(0));
        let seen = reviews.clone();
        let handle = begin(&v, "workflow", "brief");
        // This test process stands in for the run's process tree.
        let peers = crate::run_auth::Peers::leaked();
        peers.register(
            "tasks-test",
            std::process::id() as i32,
            vec![handle.id.clone()],
        );
        let started = serve_listener(
            listener,
            v.clone(),
            Arc::new(move |n| {
                seen.fetch_add(n, Ordering::SeqCst);
            }),
            || true,
            peers,
            Box::new(|| {}),
        );
        assert!(started);

        let client = tasks_mcp::SocketClient {
            path: sock.clone(),
            identity: identity(&handle.id, "workflow", "brief", TasksAccess::ReadWrite),
        };
        let args = json!({ "idempotencyKey": "weekly", "title": "Send the weekly summary" });
        let first = client.call("create_task", &args).unwrap();
        let second = client.call("create_task", &args).unwrap();
        assert_eq!(first["disposition"], json!("applied"));
        assert_eq!(second["disposition"], json!("duplicate"));
        assert_eq!(first["ref"], second["ref"]);
        assert_eq!(task_files(&v), ["send-the-weekly-summary.md"]);
        assert_eq!(
            reviews.load(Ordering::SeqCst),
            0,
            "a create queues no review"
        );

        let refused = client
            .call(
                "set_status",
                &json!({ "ref": "vault:tasks/nope.md", "status": "Done" }),
            )
            .unwrap_err();
        assert!(refused.contains("No task at"), "{refused}");
        let proposed = client
            .call(
                "propose_tasks",
                &json!({ "items": [{ "title": "Draft the agenda" }] }),
            )
            .unwrap();
        assert_eq!(proposed["queued"].as_array().unwrap().len(), 1);
        assert_eq!(
            reviews.load(Ordering::SeqCst),
            1,
            "a queued proposal raises the review event"
        );
        automated_history::finish_run(&v, handle, "success").unwrap();
    }

    /// Merged plan slice 6 (reviews A·S5, B·F11): a valid proof is not
    /// enough. From a process outside every run Hollow started, or from
    /// inside a different run (a proof replayed from another run's
    /// environment), the write is refused and nothing lands; from inside
    /// the run the proof names, the same request is written.
    #[cfg(unix)]
    #[test]
    fn a_proof_from_outside_its_run_tree_is_refused() {
        let dir = vault();
        let v = dir.path().to_path_buf();
        let handle = begin(&v, "workflow", "brief");
        let who = identity(&handle.id, "workflow", "brief", TasksAccess::ReadWrite);
        let args = json!({ "idempotencyKey": "k1", "title": "Replayed write" });
        let serve = |peers: &'static crate::run_auth::Peers, name: &str| {
            let sock_dir = tempfile::Builder::new().prefix("tb").tempdir().unwrap();
            let sock = sock_dir.path().join(name);
            let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
            let review: Arc<dyn Fn(usize) + Send + Sync> = Arc::new(|_: usize| {});
            assert!(serve_listener(
                listener,
                v.clone(),
                review,
                || true,
                peers,
                Box::new(|| {})
            ));
            let client = tasks_mcp::SocketClient {
                path: sock,
                identity: who.clone(),
            };
            (sock_dir, client)
        };

        let outside = crate::run_auth::Peers::leaked();
        let (_dir1, client) = serve(outside, "outside.sock");
        let refused = client.call("create_task", &args).unwrap_err();
        assert!(refused.contains("Hollow refused"), "{refused}");
        assert!(task_files(&v).is_empty(), "nothing written from outside");

        let other_run = crate::run_auth::Peers::leaked();
        other_run.register(
            "other",
            std::process::id() as i32,
            vec!["run-weaker".into()],
        );
        let (_dir2, client) = serve(other_run, "other.sock");
        let replay = client.call("create_task", &args).unwrap_err();
        assert!(replay.contains(&handle.id), "{replay}");
        assert!(task_files(&v).is_empty(), "nothing written by a replay");

        let own_run = crate::run_auth::Peers::leaked();
        own_run.register("own", std::process::id() as i32, vec![handle.id.clone()]);
        let (_dir3, client) = serve(own_run, "own.sock");
        let written = client.call("create_task", &args).unwrap();
        assert_eq!(written["disposition"], json!("applied"));
        assert_eq!(task_files(&v), ["replayed-write.md"]);
        automated_history::finish_run(&v, handle, "success").unwrap();
    }

    #[test]
    fn set_status_rolls_recurring_tasks_forward_through_the_mcp() {
        let dir = vault();
        let v = dir.path();
        let recurring = "---\ntitle: Water plants\nstatus: Not started\ndue: 2026-09-20\nrecurrence: every week\n---\n";
        put(v, "tasks/water-plants.md", recurring);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);

        let blind = run(
            v,
            &who,
            "set_status",
            json!({ "ref": "tasks/water-plants.md", "status": "Done" }),
        )
        .unwrap_err();
        assert!(blind.contains("expectedHash"), "{blind}");
        assert_eq!(read(v, "tasks/water-plants.md"), recurring);

        let next = crate::quick_add::next_occurrence("every week", now().date_naive())
            .unwrap()
            .to_string();
        let done = run(v, &who, "set_status", json!({ "ref": "tasks/water-plants.md", "status": "Done", "expectedHash": vault_tasks::content_hash(recurring) })).unwrap();
        assert_eq!(done["status"], json!("Not started"));
        assert_eq!(done["rolledForwardTo"], json!(next));
        let task = vault_tasks::get(v, "tasks/water-plants.md").unwrap();
        assert_eq!(task.due.as_deref(), Some(next.as_str()));
        assert_eq!(done["contentHash"], json!(task.content_hash));

        // A retry with the stale hash can't roll it twice.
        let retry = run(v, &who, "set_status", json!({ "ref": "tasks/water-plants.md", "status": "Done", "expectedHash": vault_tasks::content_hash(recurring) })).unwrap_err();
        assert!(retry.starts_with("TASK_CONFLICT"), "{retry}");
        // A plain task completes without a hash.
        put(v, "tasks/draft-the-brief.md", TASK);
        let plain = run(
            v,
            &who,
            "set_status",
            json!({ "ref": "tasks/draft-the-brief.md", "status": "Done" }),
        )
        .unwrap();
        assert_eq!(plain["status"], json!("Done"));
        assert_eq!(plain["rolledForwardTo"], Value::Null);
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn update_task_refuses_status_and_requires_expected_hash() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let hash = vault_tasks::content_hash(TASK);
        let status = run(v, &who, "update_task", json!({ "ref": "tasks/draft-the-brief.md", "expectedHash": hash, "fields": { "status": "Done" } })).unwrap_err();
        assert!(status.contains("use set_status"), "{status}");
        let unhashed = run(
            v,
            &who,
            "update_task",
            json!({ "ref": "tasks/draft-the-brief.md", "fields": { "title": "X" } }),
        )
        .unwrap_err();
        assert!(unhashed.contains("expectedHash is required"), "{unhashed}");
        let moved = run(v, &who, "update_task", json!({ "ref": "tasks/draft-the-brief.md", "expectedHash": hash, "fields": { "projectUrl": "vault:projects/a.md" } })).unwrap_err();
        assert!(moved.contains("No project at"), "{moved}");
        let empty = run(
            v,
            &who,
            "update_task",
            json!({ "ref": "tasks/draft-the-brief.md", "expectedHash": hash, "fields": {} }),
        )
        .unwrap_err();
        assert!(empty.contains("nothing to change"), "{empty}");
        assert_eq!(read(v, "tasks/draft-the-brief.md"), TASK);

        // An empty string clears a text field; untouched keys keep their bytes.
        let cleared = run(v, &who, "update_task", json!({ "ref": "tasks/draft-the-brief.md", "expectedHash": hash, "fields": { "priority": "", "section": "Writing", "subtasks": [{ "text": "outline" }] } })).unwrap();
        let task = vault_tasks::get(v, "tasks/draft-the-brief.md").unwrap();
        assert_eq!(task.priority, None);
        assert_eq!(task.section.as_deref(), Some("Writing"));
        assert_eq!(task.subtasks.len(), 1);
        assert_eq!(cleared["contentHash"], json!(task.content_hash));
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    /// Cross-M3/M4 M16: blockedBy goes through PR 11b's checks.
    #[test]
    fn update_task_blocked_by_goes_through_the_blocker_checks() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/a.md", "---\ntitle: A\nstatus: Not started\n---\n");
        put(
            v,
            "tasks/b.md",
            "---\ntitle: B\nstatus: Not started\nblocked_by: vault:tasks/a.md\n---\n",
        );
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let a_hash = vault_tasks::content_hash(&read(v, "tasks/a.md"));
        let cycle = run(v, &who, "update_task", json!({ "ref": "tasks/a.md", "expectedHash": a_hash, "fields": { "blockedBy": ["vault:tasks/b.md"] } })).unwrap_err();
        assert!(cycle.starts_with("TASK_CYCLE"), "{cycle}");
        let missing = run(v, &who, "update_task", json!({ "ref": "tasks/a.md", "expectedHash": a_hash, "fields": { "blockedBy": ["vault:tasks/nope.md"] } })).unwrap_err();
        assert!(missing.starts_with("TASK_BLOCKER"), "{missing}");
        assert_eq!(vault_tasks::content_hash(&read(v, "tasks/a.md")), a_hash);
        put(v, "tasks/c.md", "---\ntitle: C\nstatus: Not started\n---\n");
        run(v, &who, "update_task", json!({ "ref": "tasks/a.md", "expectedHash": a_hash, "fields": { "blockedBy": ["vault:tasks/c.md"] } })).unwrap();
        assert_eq!(
            vault_tasks::get(v, "tasks/a.md").unwrap().blocked_by,
            ["vault:tasks/c.md"]
        );
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn add_comment_appends_to_activity_through_the_mcp() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let first = run(v, &who, "add_comment", json!({ "ref": "tasks/draft-the-brief.md", "body": "Outline done.\n- [ ] not a step", "idempotencyKey": "c1" })).unwrap();
        assert_eq!(first["disposition"], json!("applied"));
        let task = vault_tasks::get(v, "tasks/draft-the-brief.md").unwrap();
        let activity = task.activity.unwrap();
        assert!(
            activity.contains("2026-09-26 10:00 agent (chat): Outline done."),
            "{activity}"
        );
        assert!(task.subtasks.is_empty(), "a comment never becomes a step");
        assert_eq!(task.description, "The brief.");

        let again = run(v, &who, "add_comment", json!({ "ref": "tasks/draft-the-brief.md", "body": "Outline done.\n- [ ] not a step", "idempotencyKey": "c1" })).unwrap();
        assert_eq!(again["disposition"], json!("duplicate"));
        assert_eq!(again["contentHash"], first["contentHash"]);
        assert_eq!(
            read(v, "tasks/draft-the-brief.md")
                .matches("Outline done.")
                .count(),
            1
        );
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    /// A task imported from Notion, with the retired two-way sync's ledger
    /// still on disk (stale, even unreadable): its `notion_id` keys are
    /// history only.
    fn imported_task(v: &Path) -> String {
        put(v, ".notion-sync.json", "{not json");
        let id = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let text = format!("---\ntitle: \"Synced\"\nstatus: \"Active\"\npriority: \"Medium\"\ndue: \"2026-09-30T15:00:00.000-04:00\"\ndescription: \"Remote\"\nnotion_project_ids: \"[\\\"aaaa\\\"]\"\nproject_url: \"\"\nproject: \"\"\nnotion_id: \"{id}\"\nnotion_url: \"https://www.notion.so/{id}\"\n---\n\nBody.\n");
        put(v, "tasks/synced.md", &text);
        text
    }

    /// Notion two-way sync is gone (2026-11-10): a task it once held is a
    /// plain vault task, written on the plain path with no `notion` result.
    #[test]
    fn a_task_imported_from_notion_is_a_plain_vault_task() {
        let dir = vault();
        let v = dir.path();
        imported_task(v);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let hash = |rel: &str| vault_tasks::content_hash(&read(v, rel));
        let renamed = run(v, &who, "update_task", json!({ "ref": "tasks/synced.md", "expectedHash": hash("tasks/synced.md"), "fields": { "title": "Renamed", "dueTime": "09:00" } })).unwrap();
        assert!(renamed.get("notion").is_none(), "{renamed}");
        let text = read(v, "tasks/synced.md");
        assert!(text.contains("Renamed"), "{text}");
        let done = run(
            v,
            &who,
            "set_status",
            json!({ "ref": "tasks/synced.md", "status": "Complete" }),
        )
        .unwrap();
        assert!(done.get("notion").is_none(), "{done}");
        assert!(read(v, "tasks/synced.md").contains("Complete"));
        // The legacy keys stay as history.
        assert!(read(v, "tasks/synced.md").contains("notion_id:"));
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    const IMPORTED_PROJECT_REL: &str =
        "workspaces/notion-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/PROJECT.md";

    #[test]
    fn a_project_imported_from_notion_is_edited_as_a_local_note() {
        let dir = vault();
        let v = dir.path();
        put(v, ".notion-sync.json", "{not json");
        let original = "---\ntitle: \"Atlas\"\nstatus: \"Backlog\"\ndescription: \"Remote\"\ncustom_key: keep me\nnotion_id: \"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"\n---\n\nProject notes.\n";
        put(v, IMPORTED_PROJECT_REL, original);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let r = format!("vault:{IMPORTED_PROJECT_REL}");
        let stale = run(
            v,
            &who,
            "update_project",
            json!({ "ref": r, "expectedHash": "0", "fields": { "title": "X" } }),
        )
        .unwrap_err();
        assert!(stale.starts_with("TASK_CONFLICT"), "{stale}");
        let both = run(v, &who, "update_project", json!({ "ref": r, "expectedHash": vault_tasks::content_hash(original), "fields": { "title": "Atlas 2.0" } })).unwrap();
        assert!(both.get("notion").is_none(), "{both}");
        let text = read(v, IMPORTED_PROJECT_REL);
        assert!(text.contains("Atlas 2.0"), "{text}");
        assert!(
            text.contains("custom_key: keep me"),
            "unknown keys survive: {text}"
        );
        assert_eq!(both["contentHash"], json!(vault_tasks::content_hash(&text)));
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn local_projects_are_created_once_and_move_with_their_status() {
        let dir = vault();
        let v = dir.path();
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        // A fresh local vault also has no legacy projects/ folder: recover
        // through local workspaces.
        let refused = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k0", "title": "Nope" }),
        )
        .unwrap_err();
        assert!(
            refused.contains("create_workspace {title, goal}"),
            "{refused}"
        );
        assert!(!refused.to_lowercase().contains("notion"), "{refused}");
        assert!(!v.join("projects").exists());
        // The recommended recovery creates a normal sidebar project with no
        // legacy project directory.
        let local = crate::workspaces::create(v, "test", "Keep test work together").unwrap();
        assert!(crate::workspaces::list(v)
            .unwrap()
            .iter()
            .any(|w| w.id == local.id));
        assert!(!v.join("projects").exists());
        assert!(!v.join(".notion-sync.json").exists());
        std::fs::create_dir_all(v.join("projects/active")).unwrap();
        let made = run(v, &who, "create_project", json!({ "idempotencyKey": "k1", "title": "Garage pilot", "priority": "P2", "desiredState": "Two sites live" })).unwrap();
        assert_eq!(made["disposition"], json!("applied"));
        assert_eq!(
            made["ref"],
            json!("vault:projects/discovery/Garage pilot.md")
        );
        let again = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k1", "title": "Garage pilot" }),
        )
        .unwrap();
        assert_eq!(again["disposition"], json!("duplicate"));
        assert_eq!(again["ref"], made["ref"]);
        let other = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k1", "title": "Something else" }),
        )
        .unwrap_err();
        assert!(other.starts_with("IDEMPOTENCY_MISMATCH"), "{other}");
        // A rename doesn't make a retry look like different work.
        run(v, &who, "update_project", json!({ "ref": made["ref"], "expectedHash": made["contentHash"], "fields": { "title": "Garage pilot (renamed)" } })).unwrap();
        let made = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k1", "title": "Garage pilot" }),
        )
        .unwrap();
        assert_eq!(made["disposition"], json!("duplicate"));
        let notion = run(
            v,
            &who,
            "create_project",
            json!({ "idempotencyKey": "k2", "title": "In Notion", "inNotion": true }),
        )
        .unwrap_err();
        assert!(notion.contains("unknown field"), "{notion}");
        let notion = run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k3", "title": "In Notion", "inNotion": true }),
        )
        .unwrap_err();
        assert!(notion.contains("unknown field `inNotion`"), "{notion}");

        let r = made["ref"].as_str().unwrap();
        let active = run(
            v,
            &who,
            "set_project_status",
            json!({ "ref": r, "status": "active", "expectedHash": made["contentHash"] }),
        )
        .unwrap();
        assert_eq!(
            active["ref"],
            json!("vault:projects/active/Garage pilot (renamed).md")
        );
        assert!(active.get("notion").is_none(), "{active}");
        assert!(!v.join("projects/discovery/Garage pilot.md").exists());
        let text = read(v, "projects/active/Garage pilot (renamed).md");
        assert!(
            text.contains("Two sites live") && text.contains("agent_key"),
            "{text}"
        );
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn tasks_move_between_projects_by_ref() {
        let dir = vault();
        let v = dir.path();
        imported_task(v);
        put(
            v,
            "projects/active/local.md",
            "---\ntitle: Local one\nstatus: active\n---\n",
        );
        put(
            v,
            "tasks/plain.md",
            "---\ntitle: Plain\nstatus: Not started\n---\n",
        );
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let hash = |rel: &str| vault_tasks::content_hash(&read(v, rel));

        // A local task files under any project note.
        run(v, &who, "update_task", json!({ "ref": "tasks/plain.md", "expectedHash": hash("tasks/plain.md"), "fields": { "projectUrl": "vault:projects/active/local.md" } })).unwrap();
        let text = read(v, "tasks/plain.md");
        assert!(
            text.contains("project: Local one")
                && text.contains("project_url: \"vault:projects/active/local.md\""),
            "{text}"
        );

        // A task imported from Notion files under a local project too.
        run(v, &who, "update_task", json!({ "ref": "tasks/synced.md", "expectedHash": hash("tasks/synced.md"), "fields": { "projectUrl": "vault:projects/active/local.md" } })).unwrap();
        let text = read(v, "tasks/synced.md");
        assert!(
            text.contains("project_url: \"vault:projects/active/local.md\""),
            "{text}"
        );

        // A name is ambiguous; the ref is required.
        let named = run(v, &who, "update_task", json!({ "ref": "tasks/plain.md", "expectedHash": hash("tasks/plain.md"), "fields": { "project": "Local one" } })).unwrap_err();
        assert!(named.contains("projectUrl"), "{named}");
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    fn queued_titles(v: &Path) -> Vec<String> {
        review_queue::list(v, Some(AGENT_TASK_KIND))
            .unwrap()
            .into_iter()
            .map(|item| item.payload["title"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn propose_tasks_queues_agent_tasks_and_bounds_the_queue() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        let done = dispatch_at(v, &request(&who, "propose_tasks", json!({ "items": [
            { "title": "Book the venue", "due": "2026-10-02", "priority": "P2", "note": "Mentioned twice.", "evidence": "notes/plan.md" },
            { "title": "Draft the brief" },
            { "title": "book the venue" }
        ] })), true, now()).unwrap();
        assert_eq!(done.review_changed, 1);
        let skipped: Vec<(String, String)> = done.result["skipped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["title"].as_str().unwrap().into(),
                    s["reason"].as_str().unwrap().into(),
                )
            })
            .collect();
        assert_eq!(
            skipped,
            [
                ("Draft the brief".to_string(), "exists".to_string()),
                ("book the venue".to_string(), "duplicate".to_string())
            ]
        );
        let item = review_queue::list(v, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);
        assert_eq!(
            (
                item.status.as_str(),
                item.source.as_str(),
                item.risk.as_str()
            ),
            ("pending", "chat", "local_write")
        );
        assert_eq!(item.payload["runId"], json!(handle.id));
        assert_eq!(item.payload["evidenceExcerpt"], json!("notes/plan.md"));
        assert_eq!(item.fingerprint, agent_task_fingerprint("Book the venue"));
        assert!(
            task_files(v) == ["draft-the-brief.md"],
            "a proposal creates no task"
        );

        // A repeat is a duplicate and queues nothing (no review event).
        let repeat = dispatch_at(
            v,
            &request(
                &who,
                "propose_tasks",
                json!({ "items": [{ "title": "Book the Venue" }] }),
            ),
            true,
            now(),
        )
        .unwrap();
        assert_eq!(repeat.review_changed, 0);
        assert_eq!(repeat.result["skipped"][0]["reason"], json!("duplicate"));

        // At most 10 per call.
        let eleven: Vec<Value> = (0..11)
            .map(|i| json!({ "title": format!("Call {i}") }))
            .collect();
        assert!(run(v, &who, "propose_tasks", json!({ "items": eleven }))
            .unwrap_err()
            .contains("at most 10"));
        // At most 25 per run: 1 + 10 + 10, then 5 more is refused.
        for batch in 0..2 {
            let items: Vec<Value> = (0..10)
                .map(|i| json!({ "title": format!("Batch {batch} item {i}") }))
                .collect();
            run(v, &who, "propose_tasks", json!({ "items": items })).unwrap();
        }
        let five: Vec<Value> = (0..5)
            .map(|i| json!({ "title": format!("Late {i}") }))
            .collect();
        assert!(
            run(v, &who, "propose_tasks", json!({ "items": five.clone() }))
                .unwrap_err()
                .contains("at most 25 per run")
        );
        automated_history::finish_run(v, handle, "success").unwrap();

        // A new run starts its own count, but the vault-wide pending cap (50)
        // still applies: 21 are pending, so 29 more fit and the 30th doesn't.
        let handle = begin(v, "chat", "s-1");
        let next = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        run(v, &next, "propose_tasks", json!({ "items": five })).unwrap();
        for i in 0..24 {
            let fp = format!("agent-task:seed{i}");
            review_queue::enqueue(
                v,
                NewReviewItem {
                    kind: AGENT_TASK_KIND.into(),
                    fingerprint: fp,
                    payload: json!({ "title": format!("Seeded {i}") }),
                    risk: "local_write".into(),
                    source: "workflow".into(),
                    source_path: None,
                },
            )
            .unwrap();
        }
        assert_eq!(queued_titles(v).len(), 50);
        let full = run(
            v,
            &next,
            "propose_tasks",
            json!({ "items": [{ "title": "One too many" }] }),
        )
        .unwrap_err();
        assert!(full.starts_with("Review Task suggestions first"), "{full}");
        automated_history::finish_run(v, handle, "success").unwrap();
    }

    #[test]
    fn propose_then_accept_creates_exactly_one_task() {
        let dir = vault();
        let v = dir.path();
        let handle = begin(v, "workflow", "brief");
        let who = identity(&handle.id, "workflow", "brief", TasksAccess::ReadWrite);
        run(v, &who, "propose_tasks", json!({ "items": [{ "title": "Book the venue", "priority": "P2", "note": "From the plan." }] })).unwrap();
        automated_history::finish_run(v, handle, "success").unwrap();
        let item = review_queue::list(v, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);

        // Accepted with an edited due date.
        accept_agent_task(v, &item, Some(json!({ "title": "Book the venue", "due": "2026-10-03", "priority": "P1", "note": "From the plan." }))).unwrap();
        assert_eq!(task_files(v), ["book-the-venue.md"]);
        let task = vault_tasks::get(v, "tasks/book-the-venue.md").unwrap();
        assert_eq!(
            (task.due.as_deref(), task.priority.as_deref()),
            (Some("2026-10-03"), Some("P1"))
        );
        assert_eq!(
            task.fingerprint,
            vault_tasks::fingerprint_for_key(&item.fingerprint)
        );
        let accepted = review_queue::list(v, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);
        assert_eq!(accepted.status, "accepted");

        // A second accept creates nothing.
        assert!(accept_agent_task(v, &accepted, None).is_err());
        assert_eq!(task_files(v), ["book-the-venue.md"]);

        // A crash after the create left the item applying; recovery returned
        // it to pending; the re-accept finds the task by fingerprint.
        let dir2 = vault();
        let v2 = dir2.path();
        let handle = begin(v2, "chat", "s-9");
        let who = identity(&handle.id, "chat", "s-9", TasksAccess::ReadWrite);
        run(
            v2,
            &who,
            "propose_tasks",
            json!({ "items": [{ "title": "Order the cake" }] }),
        )
        .unwrap();
        automated_history::finish_run(v2, handle, "success").unwrap();
        let item = review_queue::list(v2, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);
        review_queue::claim(v2, &item.id, None).unwrap();
        let payload: AgentTaskPayload = serde_json::from_value(item.payload.clone()).unwrap();
        create_accepted(v2, &item, &payload).unwrap();
        review_queue::fail(v2, &item.id, "crashed before accept").unwrap();
        let recovered = review_queue::list(v2, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);
        accept_agent_task(v2, &recovered, None).unwrap();
        assert_eq!(task_files(v2), ["order-the-cake.md"]);

        // Accepted under an edited title, the card keeps the original
        // title's fingerprint: proposing the original title again is a
        // duplicate, never "queued" (review, 2026-09-26).
        let dir3 = vault();
        let v3 = dir3.path();
        let handle = begin(v3, "chat", "s-3");
        let who = identity(&handle.id, "chat", "s-3", TasksAccess::ReadWrite);
        run(
            v3,
            &who,
            "propose_tasks",
            json!({ "items": [{ "title": "Foo" }] }),
        )
        .unwrap();
        let item = review_queue::list(v3, Some(AGENT_TASK_KIND))
            .unwrap()
            .remove(0);
        accept_agent_task(v3, &item, Some(json!({ "title": "Bar" }))).unwrap();
        assert_eq!(task_files(v3), ["bar.md"]);
        let again = run(
            v3,
            &who,
            "propose_tasks",
            json!({ "items": [{ "title": "Foo" }] }),
        )
        .unwrap();
        automated_history::finish_run(v3, handle, "success").unwrap();
        assert_eq!(again["queued"], json!([]));
        assert_eq!(
            again["skipped"],
            json!([{ "title": "Foo", "reason": "duplicate" }])
        );
        assert!(again["message"]
            .as_str()
            .unwrap()
            .starts_with("Nothing new to suggest"));
        assert_eq!(
            review_queue::list(v3, Some(AGENT_TASK_KIND)).unwrap().len(),
            1
        );
    }

    #[test]
    fn broker_writes_raise_no_protected_change_flag() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let who = identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite);
        run(
            v,
            &who,
            "propose_tasks",
            json!({ "items": [{ "title": "Book the venue" }] }),
        )
        .unwrap();
        run(
            v,
            &who,
            "create_task",
            json!({ "idempotencyKey": "k", "title": "Order the cake" }),
        )
        .unwrap();
        run(
            v,
            &who,
            "set_status",
            json!({ "ref": "tasks/draft-the-brief.md", "status": "In progress" }),
        )
        .unwrap();
        run(
            v,
            &who,
            "add_comment",
            json!({ "ref": "tasks/draft-the-brief.md", "body": "Started." }),
        )
        .unwrap();
        let finished = automated_history::finish_run(v, handle, "success").unwrap();
        assert!(finished.flags.is_empty(), "{:?}", finished.flags);

        // Control: the same queue written directly during a run IS flagged.
        let handle = begin(v, "chat", "s-1");
        let queue = read(v, review_queue::FILE_NAME);
        put(
            v,
            review_queue::FILE_NAME,
            &queue.replace("Book the venue", "Book the venue now"),
        );
        let flagged = automated_history::finish_run(v, handle, "success").unwrap();
        assert!(
            flagged
                .flags
                .iter()
                .any(|f| f.kind == "stateFile" && f.path == review_queue::FILE_NAME),
            "{:?}",
            flagged.flags
        );
    }

    #[test]
    fn task_tools_switch_persists_across_restart() {
        // app_data_dir() is a per-process temp dir under test.
        let _switch = tasks_mcp::SWITCH_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        save_settings(false).unwrap();
        tasks_mcp::set_enabled(true);
        load_settings();
        assert!(!tasks_mcp::enabled(), "the saved switch wins at startup");
        let parent = crate::usage_ledger::DelegateParent {
            surface: crate::usage_ledger::Surface::Chat,
            scope: "s".into(),
            run_id: "r".into(),
            scratch: None,
            tool_limits: Default::default(),
        };
        assert!(
            tasks_mcp::server_entry(Path::new("/vault"), &parent, TasksAccess::ReadWrite).is_none()
        );
        save_settings(true).unwrap();
        assert!(tasks_mcp::enabled());
        let file = crate::agent_settings::app_data_dir()
            .unwrap()
            .join(crate::agent_settings::AGENT_TOOLS_FILE);
        std::fs::remove_file(file).unwrap();
        tasks_mcp::set_enabled(false);
        load_settings();
        assert!(tasks_mcp::enabled(), "a missing file means on");
    }

    #[test]
    fn proofs_are_bound_to_every_identity_part() {
        let base = proof_for("r", "chat", "s", TasksAccess::ReadWrite);
        assert_eq!(base.len(), 64);
        assert_eq!(base, proof_for(" r ", "chat", "s", TasksAccess::ReadWrite));
        for other in [
            proof_for("r2", "chat", "s", TasksAccess::ReadWrite),
            proof_for("r", "workflow", "s", TasksAccess::ReadWrite),
            proof_for("r", "chat", "s2", TasksAccess::ReadWrite),
            proof_for("r", "chat", "s", TasksAccess::ReadOnly),
        ] {
            assert_ne!(base, other);
        }
        assert!(proof_matches(&base, &base));
        assert!(!proof_matches(&base, &base[..63]));
    }

    /// The UI command and the MCP share the one roll-forward writer, and the
    /// accept arm routes through the shared idempotent create.
    #[test]
    fn set_task_status_and_the_broker_share_the_roll_forward() {
        let broker = include_str!("task_broker.rs");
        assert!(broker.contains(concat!("vault_tasks::", "set_status_with_recurrence(")));
        assert!(broker.contains(concat!("vault_tasks::", "create_idempotent(vault, fm")));
        assert!(broker.contains(concat!(
            "automated_history::",
            "with_live_run_touching(vault, run_id, &[surface]"
        )));
        let commands = include_str!("commands.rs");
        let start = commands
            .find(concat!("pub async fn ", "set_task_status("))
            .unwrap();
        let body = &commands[start..start + commands[start..].find("\n}\n").unwrap()];
        assert!(body.contains(concat!("set_status_with", "_recurrence(")));
        assert!(!body.contains(concat!("next_", "occurrence")));
        assert!(commands.contains(concat!(
            "\"agent_task\" => crate::task_broker::",
            "accept_agent_task("
        )));
    }

    /// S09: a claim crosses the broker as `chat:<session key>`, another
    /// session's claim is refused by name, only the owner releases, and
    /// next_claimable offers the released task again.
    #[test]
    fn claims_go_through_the_broker_as_the_calling_session() {
        let dir = vault();
        let v = dir.path();
        put(v, "tasks/draft-the-brief.md", TASK);
        let handle = begin(v, "chat", "s-1");
        let one = InProcess {
            vault: v,
            identity: identity(&handle.id, "chat", "s-1", TasksAccess::ReadWrite),
        };
        let two = InProcess {
            vault: v,
            identity: identity(&handle.id, "chat", "s-2", TasksAccess::ReadWrite),
        };
        let brief = json!("vault:tasks/draft-the-brief.md");
        let reference = json!({ "ref": brief });
        let next = mcp(v, &one, "next_claimable", json!({})).unwrap();
        assert_eq!(next["task"]["ref"], brief);
        let claimed = mcp(v, &one, "claim_task", reference.clone()).unwrap();
        assert_eq!(claimed["claimedBy"], json!("chat:s-1"));
        assert_eq!(claimed["status"], json!("In progress"));
        assert_eq!(claimed["disposition"], json!("applied"));
        assert_eq!(claimed["already"], json!(false));
        // S24: the owner's repeat succeeds as `applied`, flagged `already`.
        let again = mcp(v, &one, "claim_task", reference.clone()).unwrap();
        assert_eq!(again["disposition"], json!("applied"));
        assert_eq!(again["already"], json!(true));
        assert_eq!(again["claimedBy"], json!("chat:s-1"));
        assert_eq!(again["contentHash"], claimed["contentHash"]);
        let next = mcp(v, &one, "next_claimable", json!({})).unwrap();
        assert_eq!(next["task"], Value::Null);
        let refused = mcp(v, &two, "claim_task", reference.clone()).unwrap_err();
        assert!(refused.starts_with("TASK_CLAIMED"), "{refused}");
        assert!(refused.contains("chat:s-1"), "{refused}");
        let refused = mcp(v, &two, "release_task", reference.clone()).unwrap_err();
        assert!(refused.contains("only its owner"), "{refused}");
        let released = mcp(v, &one, "release_task", reference).unwrap();
        assert_eq!(released["claimedBy"], Value::Null);
        assert_eq!(released["status"], json!("Not started"));
        let next = mcp(v, &one, "next_claimable", json!({})).unwrap();
        assert_eq!(next["task"]["ref"], brief);
        automated_history::finish_run(v, handle, "success").unwrap();
    }
}
