//! The frontend's IPC types, generated from the Rust structs (merged plan
//! slice 6 item 7).
//!
//! `src/generated/ipc.ts` is written from the structs below with ts-rs (a
//! dev-dependency: `#[cfg_attr(test, derive(ts_rs::TS))]` on each struct, so
//! nothing reaches the shipped binary). ts-rs reads the same serde attributes
//! the wire uses, so a field renamed, added, dropped or made optional in Rust
//! changes the generated type. [`generated_types_are_current`] regenerates the
//! file and fails when the committed copy differs, so `cargo test --lib` (and
//! the merge gate) catches a stale file.
//!
//! Each page's main response also has a serialized fixture under
//! `tests/fixtures/ipc/`, written by serde from a representative value here
//! and checked the same way; `src/ipcFixtures.test.ts` parses each one with
//! the generated type and asserts its keys, so serde's output and the
//! generated type cannot disagree unnoticed.
//!
//! To regenerate both after changing a type: from `src-tauri/`, run
//! `HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings`, then commit.
//!
//! Where a Rust field is a `String` holding a fixed vocabulary that the
//! frontend narrows to a union, the field points at one of the TS-only enums
//! here with `#[ts(as = "…")]`. They are not used at runtime; the Rust code
//! still writes the strings.

use std::any::TypeId;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use ts_rs::{Config, TypeVisitor, TS};

/// Set (to anything) to rewrite the generated file and the fixtures instead
/// of failing on a difference.
const REGENERATE: &str = "HOLLOW_REGENERATE_IPC";

const HEADER: &str = "\
// Generated from the Rust IPC types by src-tauri/src/ts_bindings.rs (ts-rs).
// Do not edit by hand: change the Rust type, then from src-tauri/ run
//   HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings
// `cargo test --lib` fails while this file is stale.
";

// ── TS-only vocabularies ────────────────────────────────────────────────────

/// `Task.priority`: Todoist-style, P1 highest (`null` reads as P4).
#[derive(TS)]
pub(crate) enum Priority {
    P1,
    P2,
    P3,
    P4,
}

/// `ConnectionReason.kind` (five asks A2).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ConnectionKind {
    Project,
    People,
    Link,
    Source,
    Task,
    Terms,
    Chat,
}

/// `Task.bucket`: the date bucket `enrich_tasks` computes.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum TaskBucket {
    Overdue,
    Today,
    ThisWeek,
    Later,
    Undated,
    Done,
}

/// `BlockerWarning.kind`.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum BlockerWarningKind {
    Missing,
    Cancelled,
    Recurring,
    Cycle,
}

/// `WorkResult.review`: `changed` is derived, never stored.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ResultReview {
    Draft,
    Ready,
    Accepted,
    Changed,
}

/// `SubagentRow.status` (L2).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum SubagentStatus {
    Running,
    Queued,
    Done,
    Failed,
    Stopped,
}

/// `ReviewEvent.actor`: only a person accepts (decision 3).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ReviewActor {
    Person,
    Agent,
}

/// `NeedsYouItem.kind`.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum NeedsYouKind {
    Approval,
    Question,
    Review,
    Result,
    Failure,
    Conflict,
    /// An ask-canvas whose answer isn't sent (Canvas C4 A3).
    Canvas,
    /// This launch's "Resume N interrupted chats" (overnight A1).
    Interrupted,
    /// The vault's iCloud/Finder conflict copies (overnight A10).
    Copies,
    /// One context sync clash card (CX6c).
    Clash,
    /// "Delete the memory X?" once a lesson's fix is installed (build run L2).
    #[ts(rename = "lesson_memory")]
    LessonMemory,
}

/// `Canvas.scope`: a workspace's `outputs/canvases/` or a conversation's
/// app-data store.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum CanvasScope {
    Workspace,
    Session,
}

/// `AgentSettings.canvases`: Settings, Chat, Canvases (Canvas C3 wave 2
/// C4). `helps` is the default and is never stored, but a chat's own choice
/// may carry it.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum CanvasFrequency {
    Off,
    Helps,
    Most,
}

/// `CanvasRecorded.outcome`.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum CanvasRecordOutcome {
    Created,
    Revised,
    Unchanged,
    Retired,
}

/// `ActivityEvent.kind` (Canvas C5b · Q3).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ActivityKind {
    Session,
    Run,
    Result,
    Install,
}

/// `ActivityEvent.status`: a conversation (`started`, `running`), a run
/// (`running`, `succeeded`, `failed`, `stopped`, `interrupted`, `unknown`),
/// a result (`created`, `modified`, `deleted` by a run or a canvas revision;
/// `added`, `edited` with no kept run record naming the change; `pending`,
/// changed while one of the workspace's turns runs, before its receipt;
/// `draft`, `ready`, `accepted`),
/// a build (`building`, `built`, `installing`, `failed`) or an install
/// (`installed`, `failed`, `unknown`).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ActivityStatus {
    Started,
    Running,
    Succeeded,
    Failed,
    Stopped,
    Interrupted,
    Unknown,
    Created,
    Modified,
    Deleted,
    Added,
    Edited,
    Pending,
    Draft,
    Ready,
    Accepted,
    Building,
    Built,
    Installing,
    Installed,
}

/// `ActivityResult.actor`: a run's receipt, a chat's canvas revision, or
/// who recorded a review.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum ActivityActor {
    Run,
    Chat,
    Person,
    Agent,
}

/// `AgentSessionSummary.resumable`: said up front so the UI never offers a
/// resume that silently lands somewhere else. `briefOnly` is Antigravity, or a
/// session whose harness id was never seen; `none`, the transcript is gone.
#[derive(TS)]
#[ts(rename_all = "camelCase")]
pub(crate) enum SessionResumability {
    Native,
    BriefOnly,
    None,
}

/// `RunQueueItem.source`: what asked for the run (`run_queue::RunSource`).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum RunQueueSource {
    #[ts(rename = "restrictedResume")]
    RestrictedResume,
    Chat,
    Cron,
    Manual,
    Trigger,
    Retry,
    Heartbeat,
    Ingest,
}

/// `RunQueueItem.status`: the row's place in its lifecycle
/// (`run_queue::RowStatus`).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum RunQueueStatus {
    Paused,
    Queued,
    Claimed,
    Running,
    Done,
    Failed,
    Cancelled,
    Skipped,
    Coalesced,
    Interrupted,
}

/// `RunQueueEvent.kind`: what just happened to the row
/// (`run_queue::EVENT_KINDS`).
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum RunQueueEventKind {
    Enqueued,
    Coalesced,
    Waiting,
    Running,
    Finished,
    Cancelled,
    Requeued,
    Held,
}

/// `SendAccepted.phase`: where a live turn is (`agent_sessions::SessionPhase`).
/// Tracks the automated-history lease's lifetime, not the child process's.
#[derive(TS)]
#[ts(rename_all = "lowercase")]
pub(crate) enum SessionPhase {
    Idle,
    Queued,
    Preparing,
    Running,
    Pausing,
    Paused,
    Cancelling,
    Failed,
}

/// `RunFlag.kind`: which protected file an automated run changed without
/// Hollow's attribution, or a shared workspace doc it wrote over
/// (`sharedDoc`) (`run_guard::FLAG_*`).
#[derive(TS)]
#[ts(rename_all = "camelCase")]
pub(crate) enum RunFlagKind {
    Workflow,
    WorkflowSettings,
    OperatorProfile,
    AgentInstructions,
    AgentConfig,
    StateFile,
    HistoryManifest,
    SharedDoc,
}

// ── Generation ──────────────────────────────────────────────────────────────

pub(crate) fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri sits in the repository")
        .to_path_buf()
}

/// Every type a root reaches, by TS name: its Rust path and declaration.
struct Collect {
    cfg: Config,
    seen: HashSet<TypeId>,
    decls: BTreeMap<String, (&'static str, String)>,
}

impl TypeVisitor for Collect {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        // Primitives, `Option`, `Vec` and maps have no declaration of their
        // own; their type parameters are visited separately.
        if T::output_path().is_none() || !self.seen.insert(TypeId::of::<T>()) {
            return;
        }
        let name = T::ident(&self.cfg);
        let rust = std::any::type_name::<T>();
        if let Some((other, _)) = self.decls.get(&name) {
            panic!("{other} and {rust} both generate the TS type {name}; rename one");
        }
        self.decls.insert(name, (rust, pretty(&T::decl(&self.cfg))));
        T::visit_dependencies(self);
    }
}

/// The generated file: each page's main IPC response and every type it
/// reaches, sorted by TS name.
fn generated() -> String {
    let mut c = Collect {
        // serde_json writes u64/i64 as plain JSON numbers.
        cfg: Config::new().with_large_int("number"),
        seen: HashSet::new(),
        decls: BTreeMap::new(),
    };
    // Work: `list_workspaces`.
    c.visit::<crate::workspaces::Workspace>();
    // A workspace's panel: `read_workspace`, `attach_workspace_source`,
    // `apply_source_refresh`.
    c.visit::<crate::workspaces::WorkspaceDetail>();
    // Your work rows: `workspace_digests`.
    c.visit::<crate::workspace_results::WorkspaceDigest>();
    // Conversations: `list_agent_sessions` (and inside `WorkspaceDetail`).
    c.visit::<crate::types::AgentSessionSummary>();
    // The rail tree: `workspace_sessions`.
    c.visit::<crate::workspaces::WorkspaceSessions>();
    // Home's Recently deleted: `trashed_workspaces` (P1-41).
    c.visit::<crate::workspaces::TrashedWorkspace>();
    // One conversation's turns: `read_agent_session`.
    c.visit::<crate::types::AgentSessionDetail>();
    // The run queue: `list_run_queue`, `cancel_queued_run`, and the
    // `run-queue-event` payload.
    c.visit::<crate::types::RunQueueItem>();
    c.visit::<crate::types::RunQueueEvent>();
    // A chat send: `chat_send`.
    c.visit::<crate::types::SendAccepted>();
    // One Main conversation (2026-11-07): `conversation_chain`.
    c.visit::<crate::chat_chain::ChainView>();
    // A conversation's subagents (L2): `session_subagents`.
    c.visit::<crate::subagents::SubagentsView>();
    // History & recovery: `list_history_runs`, `list_note_history`,
    // `get_history_run`, `restore_history_change`, `restore_history_run`,
    // `history_storage_status`.
    c.visit::<crate::automated_history::HistoryRun>();
    c.visit::<crate::automated_history::HistoryStorageStatus>();
    // Merge into… (master plan #18): `preview_workspace_merge`, `merge_workspace`.
    c.visit::<crate::workspace_merge::MergeOutcome>();
    c.visit::<crate::workspace_merge::UnmergeOutcome>();
    // Bulk learning review: `resolve_learning_proposals_batch`,
    // `undo_learning_batch`.
    c.visit::<crate::vault_learning::LearningBatchOutcome>();
    // Workspace connections (five asks A2): `workspace_connections`.
    c.visit::<crate::workspace_connections::WorkspaceConnections>();
    // Pressure tests (master plan #8): `personas_list`, `personas_draft`.
    c.visit::<crate::stakeholder_sim::PersonaView>();
    // Tasks: `tasks`.
    c.visit::<crate::types::TasksResponse>();
    // The task tracker (overhaul A2): `tracker_status` (also the key and
    // settings commands' answer), `tracker_view`, `tracker_issue`,
    // `tracker_team_meta`, the writes' outcome, `tracker_sync`; and their
    // inputs.
    c.visit::<crate::tracker::ipc::TrackerStatus>();
    c.visit::<crate::tracker::ipc::TrackerView>();
    c.visit::<crate::tracker::ipc::TrackerIssueDetail>();
    c.visit::<crate::tracker::ipc::TrackerTeamMeta>();
    c.visit::<crate::tracker::ipc::TrackerWriteOutcome>();
    c.visit::<crate::tracker::ipc::TrackerFridayStatus>();
    c.visit::<crate::tracker::ipc::TrackerSyncOutcome>();
    c.visit::<crate::tracker::ipc::TrackerViewQuery>();
    c.visit::<crate::tracker::ipc::TrackerEdit>();
    c.visit::<crate::tracker::ipc::TrackerNewTask>();
    c.visit::<crate::tracker::ipc::TrackerNewProject>();
    c.visit::<crate::tracker::settings::TrackerSettingsUpdate>();
    // Sources of truth (H4a): `sources_status`, `sources_set` and the owner
    // switch commands (`sources_migration_*`).
    c.visit::<crate::sources::SourcesStatus>();
    c.visit::<crate::sources::Update>();
    c.visit::<crate::sources::migration::Target>();
    c.visit::<crate::sources::migration::Plan>();
    c.visit::<crate::sources::migration::Outcome>();
    c.visit::<crate::sources::migration::Status>();
    // The issue detail's activity: `tracker_issue_history`.
    c.visit::<crate::tracker::TrackerHistory>();
    // Agent work in the tracker (X1): `tracker_issue_work`.
    c.visit::<crate::tracker::work::TrackerIssueWork>();
    // Library: `list_all_results`.
    c.visit::<crate::workspace_results::LibraryResult>();
    // Results: `workspace_results`, `set_result_review`.
    c.visit::<crate::workspace_results::WorkResult>();
    // Automations: `list_workflows`.
    c.visit::<crate::types::Workflow>();
    // Settings: `get_agent_settings`, `set_agent_settings`.
    c.visit::<crate::agent_settings::AgentSettings>();
    // The CFO (COO rebuild L7): `cfo_settings`, `cfo_save_settings`,
    // `cfo_view`, `cfo_approve_extra_usage`.
    c.visit::<crate::cfo::CfoSettings>();
    c.visit::<crate::cfo::CfoView>();
    c.visit::<crate::cfo::settings::Patch>();
    // Agent updates: `agent_cli_status`, `update_agent_cli`.
    c.visit::<crate::agent_cli::AgentCliStatus>();
    // Settings, Advanced, Folders agents can run tools from (A12):
    // `agent_tool_path`.
    c.visit::<crate::shell_path::AgentToolPath>();
    // Settings, Models, Self-hosted models (self-hosted models Q4):
    // `self_hosted_models` and its changes, `test_self_hosted_model`.
    c.visit::<crate::self_hosted_registry::SelfHostedModels>();
    c.visit::<crate::self_hosted_registry::SelfHostedTest>();
    // A self-hosted chat's start chip (Q6): `wake_self_hosted_model` and
    // `self_hosted_wake_status`.
    c.visit::<crate::self_hosted_wake::SelfHostedWake>();
    c.visit::<crate::self_hosted_wake::SelfHostedWakeStatus>();
    // Its Background jobs (self-hosted models Q10): `self_hosted_jobs`,
    // `set_self_hosted_job`, `run_self_hosted_jobs_now`, and Try again's
    // `retry_self_hosted_job`.
    c.visit::<crate::self_hosted_jobs::SelfHostedJobs>();
    c.visit::<crate::self_hosted_jobs::SelfHostedRetry>();
    // Waiting on you: `list_needs_you`.
    c.visit::<crate::needs_you::NeedsYouPage>();
    // Resume after restart (overnight A1): `list_interrupted_chats`,
    // `resume_interrupted_chats`.
    c.visit::<crate::restart_resume::InterruptedChat>();
    c.visit::<crate::restart_resume::ResumeOutcome>();
    // Extraction model (overnight A11): `get_extraction_settings`,
    // `set_extraction_settings`.
    c.visit::<crate::extract::ExtractionSettings>();
    c.visit::<crate::extract::ExtractionView>();
    // Learning suggestions (fewer learnings, 10-11): `get_learning_settings`,
    // `set_learning_suggestions`.
    c.visit::<crate::vault_learning::LearningSettings>();
    // Conflict copies (overnight A10): `conflict_copies`, `conflict_copy_pair`.
    c.visit::<crate::conflict_copies::ConflictCopy>();
    c.visit::<crate::conflict_copies::ConflictPair>();
    // Context sync clash cards (CX6c): `sync_clashes`, `resolve_sync_clash`.
    c.visit::<crate::sync_engine::clash::SyncClash>();
    c.visit::<crate::sync_engine::clash::ClashResolved>();
    // The context review card (CX20): `context_review_list`,
    // `context_review_accept`.
    c.visit::<crate::context_review::ContextReviewList>();
    c.visit::<crate::context_review::ContextReviewAccepted>();
    // Saved chat canvases (Canvas C3): `read_canvas`, `list_canvases`,
    // `record_canvas`, `save_canvas_state`, `read_canvas_revision`.
    c.visit::<crate::workspace_canvases::CanvasRecorded>();
    c.visit::<crate::workspace_canvases::CanvasSummary>();
    c.visit::<crate::workspace_canvases::CanvasStateSaved>();
    c.visit::<crate::workspace_canvases::CanvasRevisionHtml>();
    c.visit::<crate::canvas_items::CanvasItemSource>();
    c.visit::<crate::canvas_items::CanvasPlanItems>();
    // The Plan view's row toggle (Canvas C5a): `toggle_plan_row`.
    c.visit::<crate::plan_rows::PlanRowToggled>();
    // A project's record (overhaul B3): `project_portfolio`, `set_project_portfolio`.
    c.visit::<crate::project_portfolio::ProjectPortfolio>();
    c.visit::<crate::project_portfolio::PortfolioUpdate>();
    // The Activity section's timeline (Canvas C5b): `workspace_activity`.
    c.visit::<crate::workspace_activity::WorkspaceActivity>();
    // Settings, Context and the bed: a run's context manifest (agent
    // effectiveness F11).
    c.visit::<crate::run_manifest::RunContextView>();
    // Scheduler v2 (F1): named windows.
    c.visit::<crate::schedule_windows::AutomationWindows>();
    c.visit::<crate::schedule_windows::WindowSettings>();
    // The peer link (M2 PR 3): `peer_machines` and `peer-machines-changed`,
    // `peer_pair`, `peer_reorder_hosts`, `peer_sessions`, `peer_detail`,
    // `peer_send`/`peer_answer`, `peer-chat-event`, and every peer command's
    // error.
    c.visit::<crate::commands::peer_api::PeerMachinesView>();
    c.visit::<crate::commands::peer_api::PeerSessionsPage>();
    c.visit::<crate::commands::peer_api::PeerChatDetail>();
    c.visit::<crate::commands::peer_api::PeerChatEvent>();
    c.visit::<crate::commands::peer_api::PeerFailure>();
    c.visit::<crate::peer_client::Accepted>();
    // M5 PR2: `chat_move_to_runner`, `chat_move_keep_here`,
    // `chat_move_release_here`.
    c.visit::<crate::chat_move_trigger::ChatMoveResult>();
    // M5 UI: `chat_move_homes`.
    c.visit::<crate::chat_move::HomeRow>();
    // M5 offline UI: `chat_offline_rows`.
    c.visit::<crate::chat_offline::OfflineRow>();
    // Updates (row U5): `update_get_state` and the `update-state` event,
    // `update_get_settings`, `update_install_now`, `update_last_result` and
    // the `update-last-result` event, `update_connect`.
    c.visit::<crate::update_client::state::UpdateState>();
    c.visit::<crate::update_client::settings::UpdateSettings>();
    c.visit::<crate::update_client::install::UpdateInstallOutcome>();
    c.visit::<crate::update_client::last::UpdateLastResult>();
    c.visit::<crate::update_client::ipc::UpdateConnected>();
    // Publish to Main (row U8): `update_publish_status`, `update_publish_main`,
    // `update_withdraw_main`.
    c.visit::<crate::update_client::publish::UpdatePublishStatus>();
    c.visit::<crate::update_client::publish::UpdatePublishOutcome>();
    // People (row U9): `update_people_list`, `update_people_add`.
    c.visit::<crate::update_client::people_admin::UpdatePeopleList>();
    c.visit::<crate::update_client::people_admin::UpdatePersonAdded>();
    // Go back (row U6): `update_go_back_plan` and `update_go_back`.
    c.visit::<crate::update_client::goback::UpdateGoBackPlan>();
    c.visit::<crate::update_client::goback_run::UpdateGoBackOutcome>();

    let mut out = String::from(HEADER);
    for (rust, decl) in c.decls.values() {
        write!(out, "\n// {rust}\nexport {decl}\n").expect("writing to a String");
    }
    out
}

const CHAT_EVENT_HEADER: &str = "\
// Generated from src-tauri/src/chat_event.rs by src-tauri/src/ts_bindings.rs (ts-rs).
// The `chat-event` payload schema (AG2). Do not edit by hand: change the Rust
// type, then from src-tauri/ run
//   HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings
// Nested objects use the frontend's own types, imported here.
import type { EvidenceReceiptWire, EvidenceRecallItemWire } from '../executionReceipt';
import type { PauseReason } from '../chatStop';
import type { SessionResetReason } from '../chatContinue';
import type { RemoteRunEvent } from '../remoteRunNotice';
import type { SteerSupportRow, SteerWire } from '../chatSteer';
import type { ChatRecallTrace } from '../store/chatRuntime';
import type { BudgetNotice, ChatInteraction, ProgressPhase, TurnUsage } from '../types';

/** A union member without its `type` (kept distributive over a union). */
type WithoutType<T> = T extends unknown ? Omit<T, 'type'> : never;
";

/// `src/generated/chatEvent.ts`: the `chat-event` envelope and every type it
/// reaches (AG2).
fn generated_chat_event() -> String {
    let mut c = Collect {
        cfg: Config::new().with_large_int("number"),
        seen: HashSet::new(),
        decls: BTreeMap::new(),
    };
    c.visit::<crate::chat_event::ChatEventEnvelope>();
    let mut out = String::from(CHAT_EVENT_HEADER);
    for (rust, decl) in c.decls.values() {
        write!(out, "\n// {rust}\nexport {decl}\n").expect("writing to a String");
    }
    out
}

/// A declaration with its field doc comments dropped (the docs live on the
/// Rust struct) and the outermost object's fields one per line, so a field
/// change is a one-line diff. A union stays on one line: splitting each
/// object member of an internally tagged enum would scatter it.
fn pretty(decl: &str) -> String {
    let flat = strip_docs(decl);
    if top_level_union(&flat) {
        return flat;
    }
    let mut out = String::with_capacity(flat.len() + 64);
    let mut depth = 0usize;
    let mut in_str = false;
    let mut chars = flat.chars().peekable();
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            in_str = c != '"';
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '{' | '[' | '(' | '<' => {
                depth += 1;
                out.push(c);
                if c == '{' && depth == 1 && chars.peek() == Some(&' ') {
                    chars.next();
                    out.push_str("\n  ");
                }
            }
            '}' | ']' | ')' | '>' => {
                depth = depth.saturating_sub(1);
                out.push(c);
            }
            ',' if depth == 1 => {
                out.push(',');
                if chars.peek() == Some(&' ') {
                    chars.next();
                    if chars.peek() == Some(&'}') {
                        out.push('\n');
                    } else {
                        out.push_str("\n  ");
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Whether `decl` has a `|` outside every bracket and string: a union type.
fn top_level_union(decl: &str) -> bool {
    let mut depth = 0usize;
    let mut in_str = false;
    for c in decl.chars() {
        if in_str {
            in_str = c != '"';
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' | '(' | '<' => depth += 1,
            '}' | ']' | ')' | '>' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// `decl` without its `/** … */` comments and newlines.
fn strip_docs(decl: &str) -> String {
    let mut out = String::with_capacity(decl.len());
    let mut rest = decl;
    while let Some(start) = rest.find("/**") {
        out.push_str(&rest[..start]);
        let comment = &rest[start + 3..];
        rest = match comment.find("*/") {
            Some(end) => &comment[end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out.replace('\n', "")
}

/// Passes when `path` holds exactly `want`; rewrites it when
/// [`REGENERATE`] is set; otherwise fails naming the first differing line.
pub(crate) fn check_or_write(path: &Path, want: &str) {
    let have = std::fs::read_to_string(path).unwrap_or_default();
    if have == want {
        return;
    }
    if std::env::var_os(REGENERATE).is_some() {
        let dir = path.parent().expect("a file path has a parent");
        std::fs::create_dir_all(dir).expect("create the output folder");
        std::fs::write(path, want).expect("write the generated file");
        return;
    }
    let line = have
        .lines()
        .zip(want.lines())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| have.lines().count().min(want.lines().count()));
    panic!(
        "{} is stale (first difference at line {}):\n  committed: {:?}\n  generated: {:?}\n\
         From src-tauri/, run `{REGENERATE}=1 cargo test --lib ts_bindings`, then commit.",
        path.display(),
        line + 1,
        have.lines().nth(line),
        want.lines().nth(line),
    );
}

/// Serializes `value` exactly as the command returns it, as the fixture
/// `tests/fixtures/ipc/<name>.json`.
fn fixture(name: &str, value: &impl serde::Serialize) {
    let mut text = serde_json::to_string_pretty(value).expect("the fixture serializes");
    text.push('\n');
    let path = repo_root()
        .join("tests/fixtures/ipc")
        .join(format!("{name}.json"));
    check_or_write(&path, &text);
}

#[test]
fn generated_types_are_current() {
    check_or_write(&repo_root().join("src/generated/ipc.ts"), &generated());
}

#[test]
fn chat_event_fixture_is_current() {
    check_or_write(
        &repo_root().join("src/generated/chatEvent.ts"),
        &generated_chat_event(),
    );
}

#[test]
fn pretty_puts_one_field_per_line_and_drops_field_docs() {
    let decl = "type A = { \n/**\n * The id.\n */\nid: string, kind?: \"a\" | \"b\", };";
    let want = "type A = {\n  id: string,\n  kind?: \"a\" | \"b\",\n};";
    assert_eq!(pretty(decl), want);
    // Nested objects and generics stay on their field's line.
    let nested = "type B = { m: { [key in string]: Array<number> }, n: number, };";
    let want = "type B = {\n  m: { [key in string]: Array<number> },\n  n: number,\n};";
    assert_eq!(pretty(nested), want);
    assert_eq!(pretty("type K = \"x\" | \"y\";"), "type K = \"x\" | \"y\";");
    // A union of objects (an internally tagged enum) stays on one line.
    let union = "type C = { \"s\": \"a\" } | { \"s\": \"b\", reason: R | null, };";
    assert_eq!(pretty(union), union);
}

/// The string literals of a TS-only enum's generated union, e.g.
/// `type K = "x" | "y";` → `{"x", "y"}`.
fn ts_union<T: TS>() -> std::collections::BTreeSet<String> {
    let decl = T::decl(&Config::new());
    let (_, body) = decl.split_once('=').expect("a type declaration");
    body.trim()
        .trim_end_matches(';')
        .split('|')
        .map(|v| v.trim().trim_matches('"').to_string())
        .collect()
}

/// `RunQueueSource` / `RunQueueStatus` are hand-listed mirrors of
/// `run_queue::RunSource` / `RowStatus`, whose wire strings come from
/// `as_str`. A variant added to either side, or a spelling that differs,
/// fails here. The Rust enums derive `TS` in tests, so their derived unions
/// name every variant: one missing from the lists below fails too (lane EH
/// review: a wildcard-free `match` didn't force the lists).
#[test]
fn run_queue_enums_match_run_queue_variants() {
    use crate::run_queue::{RowStatus, RunSource};

    let sources = [
        RunSource::Chat,
        RunSource::Cron,
        RunSource::Manual,
        RunSource::Trigger,
        RunSource::Retry,
        RunSource::RestrictedResume,
        RunSource::Heartbeat,
        RunSource::Ingest,
    ];
    let listed: std::collections::BTreeSet<String> =
        sources.iter().map(|s| format!("{s:?}")).collect();
    assert_eq!(
        listed,
        ts_union::<RunSource>(),
        "a RunSource variant isn't listed"
    );
    let rust: std::collections::BTreeSet<String> =
        sources.iter().map(|s| s.as_str().to_string()).collect();
    assert_eq!(rust.len(), sources.len(), "a RunSource listed twice");
    assert_eq!(
        ts_union::<RunQueueSource>(),
        rust,
        "RunQueueSource vs RunSource"
    );

    let statuses = [
        RowStatus::Queued,
        RowStatus::Claimed,
        RowStatus::Running,
        RowStatus::Done,
        RowStatus::Failed,
        RowStatus::Cancelled,
        RowStatus::Skipped,
        RowStatus::Coalesced,
        RowStatus::Interrupted,
        RowStatus::Paused,
    ];
    let listed: std::collections::BTreeSet<String> =
        statuses.iter().map(|s| format!("{s:?}")).collect();
    assert_eq!(
        listed,
        ts_union::<RowStatus>(),
        "a RowStatus variant isn't listed"
    );
    let rust: std::collections::BTreeSet<String> =
        statuses.iter().map(|s| s.as_str().to_string()).collect();
    assert_eq!(rust.len(), statuses.len(), "a RowStatus listed twice");
    assert_eq!(
        ts_union::<RunQueueStatus>(),
        rust,
        "RunQueueStatus vs RowStatus"
    );

    // Each TS string reads back as the variant that writes it.
    for v in ts_union::<RunQueueSource>() {
        assert_eq!(
            RunSource::parse(&v).map(RunSource::as_str),
            Some(v.as_str())
        );
    }
    for v in ts_union::<RunQueueStatus>() {
        assert_eq!(
            RowStatus::parse(&v).map(RowStatus::as_str),
            Some(v.as_str())
        );
    }
}

/// `RunQueueEventKind` mirrors `run_queue::EVENT_KINDS`, the list
/// `run_queue::event` asserts every emitted kind against.
#[test]
fn run_queue_event_kinds_match_event_kinds() {
    use crate::run_queue::EVENT_KINDS;

    let rust: std::collections::BTreeSet<String> =
        EVENT_KINDS.iter().map(|k| k.to_string()).collect();
    assert_eq!(rust.len(), EVENT_KINDS.len(), "a kind listed twice");
    assert_eq!(
        ts_union::<RunQueueEventKind>(),
        rust,
        "RunQueueEventKind vs run_queue::EVENT_KINDS"
    );
}

/// `SessionPhase` mirrors `agent_sessions::SessionPhase`, whose wire strings
/// come from `as_str`. The Rust enum derives `TS` in tests, so its derived
/// union names every variant: one missing from the list below fails.
#[test]
fn session_phase_matches_agent_sessions_phase() {
    use crate::agent_sessions::SessionPhase as Phase;

    let phases = [
        Phase::Idle,
        Phase::Queued,
        Phase::Preparing,
        Phase::Running,
        Phase::Pausing,
        Phase::Paused,
        Phase::Cancelling,
        Phase::Failed,
    ];
    let listed: std::collections::BTreeSet<String> =
        phases.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(
        listed,
        ts_union::<Phase>(),
        "a SessionPhase variant isn't listed"
    );
    let rust: std::collections::BTreeSet<String> =
        phases.iter().map(|p| p.as_str().to_string()).collect();
    assert_eq!(rust.len(), phases.len(), "a SessionPhase listed twice");
    assert_eq!(
        ts_union::<SessionPhase>(),
        rust,
        "SessionPhase vs agent_sessions::SessionPhase"
    );
}

/// `RunFlagKind` mirrors the `run_guard::FLAG_*` consts. They are read from
/// the source too (at any visibility and indent), so a new `FLAG_*` missing
/// from the list below fails.
#[test]
fn run_flag_kinds_match_flag_consts() {
    use crate::run_guard as guard;

    let flags = [
        guard::FLAG_WORKFLOW,
        guard::FLAG_WORKFLOW_SETTINGS,
        guard::FLAG_OPERATOR_PROFILE,
        guard::FLAG_AGENT_INSTRUCTIONS,
        guard::FLAG_AGENT_CONFIG,
        guard::FLAG_STATE_FILE,
        guard::FLAG_HISTORY_MANIFEST,
        guard::FLAG_SHARED_DOC,
    ];
    let rust: std::collections::BTreeSet<String> = flags.iter().map(|f| f.to_string()).collect();
    assert_eq!(rust.len(), flags.len(), "a flag listed twice");
    let declared: std::collections::BTreeSet<String> = include_str!("run_guard.rs")
        .lines()
        .filter_map(|line| {
            // `const FLAG_`, `pub const FLAG_`, `pub(crate) const FLAG_`, …
            let (vis, rest) = line.trim_start().split_once("const FLAG_")?;
            (vis.is_empty() || vis.starts_with("pub")).then_some(rest)
        })
        .filter_map(|rest| rest.split('"').nth(1))
        .map(str::to_string)
        .collect();
    assert_eq!(declared, rust, "a run_guard::FLAG_* const isn't listed");
    assert_eq!(
        ts_union::<RunFlagKind>(),
        rust,
        "RunFlagKind vs run_guard::FLAG_*"
    );
}

// ── One fixture per page ────────────────────────────────────────────────────

fn s(text: &str) -> String {
    text.to_string()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

#[test]
fn fixture_work_list_workspaces() {
    use crate::build_workspaces::BuildInfo;
    use crate::workspaces::Workspace;
    let build = Workspace {
        id: s("local-results-view"),
        title: s("Results view"),
        status: s("active"),
        path: s("workspaces/local-results-view"),
        cwd: s("/Users/fixture/Hollow/Worktrees/hollow/results-view"),
        modified: s("2026-10-03T09:00:00Z"),
        build: Some(BuildInfo {
            repo: s("/Users/fixture/Projects/hollow"),
            worktree: s("/Users/fixture/Hollow/Worktrees/hollow/results-view"),
            branch: s("hollow/results-view"),
            base: s("origin/main"),
            exists: true,
            runs_on: Some(s("runner")),
            port: None,
        }),
        kind: None,
        merged_into: None,
        archived_from: None,
        canvas_theme: Some(s("Graphite")),
        pinned: Some(1),
    };
    let quick = Workspace {
        id: s(crate::workspaces::QUICK_CHATS_ID),
        title: s(crate::workspaces::QUICK_CHATS_TITLE),
        status: s("active"),
        path: s("workspaces/quick-chats"),
        cwd: s("/Users/fixture/vault/workspaces/quick-chats"),
        modified: s("2026-10-02T18:30:00Z"),
        build: None,
        kind: Some(s("quick")),
        merged_into: None,
        archived_from: None,
        canvas_theme: None,
        pinned: None,
    };
    // A workspace merged into another (Merge into…, master plan #18).
    let merged = Workspace {
        id: s("old-research-0a1b2c3d"),
        title: s("Old research"),
        status: s("archived"),
        path: s("workspaces/old-research-0a1b2c3d"),
        cwd: s("/Users/fixture/vault/workspaces/old-research-0a1b2c3d"),
        modified: s("2026-10-01T12:00:00Z"),
        build: None,
        kind: None,
        merged_into: Some(s("local-results-view")),
        archived_from: Some(s("active")),
        canvas_theme: None,
        pinned: None,
    };
    fixture("work", &[build, quick, merged]);
}

#[test]
fn fixture_tasks() {
    use crate::types::{BlockerWarning, SubTask, Task, TasksResponse};
    let task = Task {
        url: s("vault:tasks/ship-generated-types.md"),
        title: s("Ship generated IPC types"),
        status: s("In progress"),
        priority: Some(s("P1")),
        due: Some(s("2026-10-05")),
        effort: None,
        description: s("Generate the frontend's IPC types from Rust."),
        project_url: Some(s("vault:projects/hollow.md")),
        project_name: Some(s("Hollow")),
        days_until_due: Some(2),
        bucket: s("thisweek"),
        vault_path: Some(s("tasks/ship-generated-types.md")),
        labels: strings(&["frontend"]),
        section: None,
        due_time: Some(s("09:30")),
        duration_min: Some(45),
        recurrence: None,
        subtasks: vec![SubTask {
            text: s("Add the drift check"),
            done: true,
        }],
        content_hash: s("c0ffee"),
        fingerprint: None,
        origin: Some(s("plan:workspaces/local-results-view/PLAN.md")),
        done_when: Some(s("`cargo test --lib` passes.")),
        activity: None,
        blocked_by: strings(&["vault:tasks/fmt-gate.md", "vault:tasks/old.md"]),
        blocks: Vec::new(),
        ready: false,
        unresolved_blockers: strings(&["vault:tasks/fmt-gate.md", "vault:tasks/old.md"]),
        blocker_warnings: vec![BlockerWarning {
            url: s("vault:tasks/old.md"),
            kind: s("missing"),
        }],
        ..Default::default()
    };
    let response = TasksResponse {
        items: vec![task],
        fetched: Some(s("2026-10-03T08:00:00Z")),
        source: s("vault"),
        todo_path: None,
        last_synced_at: None,
        sync_error: None,
    };
    fixture("tasks", &response);
}

/// An accepted result with its run, checks and two review events.
fn sample_result() -> crate::workspace_results::WorkResult {
    use crate::workspace_results::{Acceptance, ResultChecks, ResultOrigin};
    use crate::workspace_results::{ReviewEvent, WorkResult};
    WorkResult {
        path: s("workspaces/local-results-view/outputs/brief.md"),
        name: s("outputs/brief.md"),
        title: s("Brief"),
        kind: s("document"),
        size: 2048,
        modified: Some(s("2026-10-03T08:45:00Z")),
        sha256: Some(s("c0ffee")),
        review: s("accepted"),
        accepted_sha256: Some(s("c0ffee")),
        accepted_at: Some(s("2026-10-03T08:50:00Z")),
        ready_at: Some(s("2026-10-03T08:46:00Z")),
        has_snapshot: true,
        acceptance: Some(Acceptance {
            actor: s("person"),
            sha256: s("c0ffee"),
            bytes: 2048,
            run_id: Some(s("run-1")),
            mode: Some(s("auto")),
            basis: s("the version on screen; 3 of 3 checks passed"),
            snapshot: true,
            at: s("2026-10-03T08:50:00Z"),
        }),
        history: vec![
            ReviewEvent {
                state: s("accepted"),
                sha256: Some(s("c0ffee")),
                at: s("2026-10-03T08:50:00Z"),
                note: Some(s("Looks right")),
                actor: Some(s("person")),
                run_id: None,
            },
            ReviewEvent {
                state: s("ready"),
                sha256: Some(s("c0ffee")),
                at: s("2026-10-03T08:46:00Z"),
                note: None,
                actor: Some(s("agent")),
                run_id: Some(s("run-1")),
            },
        ],
        origin: Some(ResultOrigin {
            session_key: Some(s("hollow-session-1")),
            run_id: s("run-1"),
            finished_at: s("2026-10-03T08:44:00Z"),
            status: s("success"),
            verification: s("verified"),
        }),
        checks: ResultChecks {
            total: Some(3),
            failed: Some(0),
            unknown: Some(0),
        },
        canvas: None,
    }
}

/// A saved chat canvas listed as a Result (Canvas C3 · S3): accepted at
/// revision 2, then revised to 3, so it reads `changed`.
fn sample_canvas_result() -> crate::workspace_results::WorkResult {
    use crate::workspace_results::{ResultCanvas, ResultChecks, WorkResult};
    WorkResult {
        path: s("workspaces/local-results-view/outputs/canvases/release-status.canvas.html"),
        name: s("outputs/canvases/release-status.canvas.html"),
        title: s("Release status"),
        kind: s("canvas"),
        size: 812,
        modified: Some(s("2026-10-04T10:02:00Z")),
        sha256: Some(s("decade")),
        review: s("changed"),
        accepted_sha256: Some(s("facade")),
        accepted_at: Some(s("2026-10-04T10:01:30Z")),
        ready_at: None,
        has_snapshot: true,
        acceptance: None,
        history: Vec::new(),
        origin: None,
        checks: ResultChecks {
            total: None,
            failed: None,
            unknown: None,
        },
        canvas: Some(ResultCanvas {
            id: s("release-status"),
            revision: Some(3),
        }),
    }
}

#[test]
fn fixture_results_workspace_results() {
    fixture("results", &[sample_result(), sample_canvas_result()]);
}

#[test]
fn fixture_library_list_all_results() {
    let item = crate::workspace_results::LibraryResult {
        workspace_id: s("local-results-view"),
        workspace_title: s("Results view"),
        result: sample_result(),
    };
    fixture("library", &[item]);
}

#[test]
fn fixture_automations_list_workflows() {
    let workflow = crate::types::Workflow {
        slug: s("morning-brief"),
        title: s("Morning brief"),
        description: s("Summarise the day ahead."),
        schedule: s("0 0 8 * * Mon-Fri *"),
        agent: Some(s("claude")),
        model: None,
        cwd: None,
        output_glob: Some(s("briefings/*.md")),
        routine: Some(s("morning-brief")),
        permission_mode: s("auto"),
        allowed_tools: strings(&["Read"]),
        disallowed_tools: Vec::new(),
        triggers: Vec::new(),
        in_chat: None,
        runs_on: None,
        created: Some(s("2026-09-01T07:00:00Z")),
        last_run_at: Some(s("2026-10-03T08:00:00Z")),
        last_status: Some(s("success")),
        last_error: None,
        last_duration_sec: Some(42),
        next_run_at: Some(s("2026-10-06T08:00:00Z")),
        misfire_grace_min: None,
        catch_up: Some(s("run-once-on-wake")),
        schedule_problem: None,
        skip_if_quiet: false,
        watch: Vec::new(),
        precheck: None,
        notify: Some(s("failures")),
        last_started_at: Some(s("2026-10-03T08:00:00Z")),
        ends: None,
        after_runs: None,
        runs_counted: None,
        ended_note: None,
        path: s("workflows/morning-brief.md"),
        permission_hold: Some(s(crate::workflow_permissions::HOLD_REVIEW)),
        budget_hold: Some(s(
            "Budget paused: this automation reached today's token limit.",
        )),
    };
    fixture("automations", &[workflow]);
}

#[test]
fn fixture_settings_agent_settings() {
    let settings = crate::agent_settings::AgentSettings {
        default_agent: Some(s("claude")),
        models: BTreeMap::from([(s("claude"), s("opus"))]),
        efforts: BTreeMap::from([(s("claude"), s("high"))]),
        // Off: an explicit null, which the wire keeps (the default is omitted).
        codex_no_output_timeout_min: None,
        no_output_timeout_min: BTreeMap::from([(s("claude"), Some(45)), (s("grok"), None)]),
        deadline_min: BTreeMap::from([(s("codex"), 120)]),
        canvases: Some(s("most")),
        // OB22: on here so the fixture names the key (off is absent).
        compact_window: Some(400_000),
        // Repo ideas 10-27 #8: set here so the fixture names it (Wait is absent).
        when_out: Some(s("codex")),
        // The remote runner (2026-11-01; `runner` since M6): absent is this Mac.
        runs_on: Some(s("runner")),
        // Overnight N6: absent follows Runs on (M7: nothing runs on the runner until chosen).
        unattended_runs_on: Some(s("same")),
        builds_on: Some(s("runner")),
        app_builds_on: Some(s("runner")),
        // M7: the one-time freeze ran (absent until it has).
        runner_defaults_frozen: true,
        // M5 PR2: absent is off; true moves running chats at sleep.
        move_on_sleep: Some(false),
        // M3a PR2: COO on this Mac, off here so the fixture names it (auto is absent).
        coo_dispatcher: Some(s("off")),
        // OC1: Grok's ACP switch on, so the fixture names the key (off is absent).
        acp: BTreeMap::from([(s("grok"), true)]),
    };
    fixture("settings", &settings);
}

#[test]
fn fixture_needs_you_list_needs_you() {
    use crate::needs_you::{NeedsYouItem, NeedsYouPage};
    let result = NeedsYouItem {
        id: s("result:workspaces/local-results-view/outputs/brief.md"),
        kind: s("result"),
        source_id: s("workspaces/local-results-view/outputs/brief.md"),
        title: s("Brief"),
        detail: Some(s("Ready for review")),
        state: s("ready"),
        at: Some(s("2026-10-03T08:46:00Z")),
        workspace_id: Some(s("local-results-view")),
        session_key: Some(s("hollow-session-1")),
        path: Some(s("workspaces/local-results-view/outputs/brief.md")),
        category: Some(s("document")),
        link: None,
    };
    let failure = NeedsYouItem {
        id: s("failure:notif-7"),
        kind: s("failure"),
        source_id: s("notif-7"),
        title: s("Morning brief failed"),
        detail: None,
        state: s("unread"),
        at: Some(s("2026-10-03T08:01:00Z")),
        workspace_id: None,
        session_key: None,
        path: None,
        category: Some(s("workflow")),
        link: Some(s("workflow:morning-brief")),
    };
    let page = NeedsYouPage {
        items: vec![result, failure],
        total: 3,
        next_cursor: Some(s("opaque-cursor")),
        unavailable: strings(&["notifications"]),
    };
    fixture("needs-you", &page);
}

#[test]
fn fixture_canvas_record_canvas() {
    use crate::workspace_canvases::{Canvas, CanvasRecorded, CanvasRevision};
    let fence = s("4f9c1a0e3b7d2c6a8e5f1b0d9c3a7e2f6b4d8c1a5e9f3b7d0c2a6e4f8b1d5c9a");
    let revision = |n: u64, sha256: &str, title: &str, at: &str| CanvasRevision {
        revision: n,
        fence_sha256: s(sha256),
        title: s(title),
        saved_at: s(at),
        ask: false,
        height: Some(480),
        session: Some(s("s-release-1")),
        turn_id: Some(s("turn-12")),
        extra: serde_json::Map::new(),
    };
    let recorded = CanvasRecorded {
        outcome: s("revised"),
        matched_revision: 2,
        canvas: Canvas {
            scope: s("workspace"),
            workspace_id: Some(s("local-results-view")),
            session_key: None,
            id: s("release-status"),
            title: s("Release status"),
            kit: None,
            session: Some(s("s-release-1")),
            fence_sha256: fence.clone(),
            created_at: s("2026-10-04T09:00:00Z"),
            updated_at: s("2026-10-04T09:30:00Z"),
            revision: 2,
            ask: false,
            height: Some(480),
            html: s("<hc-canvas id=\"release-status\" title=\"Release status\" height=\"480\">\n  <p>29 checks</p>\n</hc-canvas>\n"),
            sha256: s("0d1c2b3a49586776a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f00f1e"),
            path: Some(s("workspaces/local-results-view/outputs/canvases/release-status.canvas.html")),
            file: Some(s("outputs/canvases/release-status.canvas.html")),
            html_changed: false,
            state: serde_json::json!({ "tab": "checks" }),
            state_saved_at: Some(s("2026-10-04T09:10:00Z")),
            revisions: vec![
                revision(
                    1,
                    "9a5c1d8b4f0e6a2c7d3b9f5e1a8c4d0b6e2f7a3c9d5b1e8f4a0c6d2b7e3f9a5c",
                    "Release status",
                    "2026-10-04T09:00:00Z",
                ),
                revision(2, &fence, "Release status", "2026-10-04T09:30:00Z"),
            ],
        },
    };
    fixture("canvas", &recorded);
}

#[test]
fn fixture_activity_workspace_activity() {
    use crate::workspace_activity::{
        ActivityCanvases, ActivityEvent, ActivityGaps, ActivityInstall, ActivityResult,
        ActivityRun, ActivitySession, WorkspaceActivity,
    };
    let run = ActivityEvent {
        id: s("run:run-2"),
        kind: s("run"),
        at: s("2026-10-05T10:05:00Z"),
        status: s("succeeded"),
        title: s("Draft the summary"),
        session_key: Some(s("hollow-session-1")),
        run_id: Some(s("run-2")),
        receipt_id: Some(s("receipt-0a1b2c3d4e5f60718293a4")),
        path: None,
        session: None,
        run: Some(ActivityRun {
            source: s("chat"),
            source_id: Some(s("native-thread-9")),
            turn: Some(2),
            started_at: Some(s("2026-10-05T10:00:00Z")),
            ended_at: Some(s("2026-10-05T10:05:00Z")),
            agent: Some(s("claude")),
            model: Some(s("claude-opus-4")),
            cost_usd: Some(0.42),
            cost_status: Some(s("reported")),
            input_tokens: Some(12_000),
            output_tokens: Some(1_800),
            files: Some(2),
            outputs: Some(1),
            checks: Some(1),
            failed_checks: Some(0),
            verification: Some(s("verified")),
            result_kind: Some(s("success")),
            canvases: Some(ActivityCanvases {
                count: 1,
                bytes: 6_200,
            }),
        }),
        result: None,
        install: None,
    };
    let result = ActivityEvent {
        id: s("result:receipt-0a1b2c3d4e5f60718293a4:outputs/summary.md"),
        kind: s("result"),
        at: s("2026-10-05T10:05:00Z"),
        status: s("created"),
        title: s("Summary"),
        session_key: Some(s("hollow-session-1")),
        run_id: Some(s("run-2")),
        receipt_id: Some(s("receipt-0a1b2c3d4e5f60718293a4")),
        path: Some(s("workspaces/local-results-view/outputs/summary.md")),
        session: None,
        run: None,
        result: Some(ActivityResult {
            name: s("outputs/summary.md"),
            kind: s("document"),
            exists: true,
            actor: Some(s("run")),
            revision: None,
            note: None,
        }),
        install: None,
    };
    let session = ActivityEvent {
        id: s("session:hollow-session-1"),
        kind: s("session"),
        at: s("2026-10-05T09:58:00Z"),
        status: s("started"),
        title: s("Quarterly summary"),
        session_key: Some(s("hollow-session-1")),
        session: Some(ActivitySession {
            agent: s("claude"),
            turns: 2,
            last_activity_at: s("2026-10-05T10:05:00Z"),
        }),
        ..Default::default()
    };
    let install = ActivityEvent {
        id: s("install:20261004-100000-47e5bd3a"),
        kind: s("install"),
        at: s("2026-10-04T10:00:00Z"),
        status: s("installed"),
        title: s("Install and restart"),
        install: Some(ActivityInstall {
            step: s("install"),
            commit: Some(s("63c79a889a21")),
            branch: Some(s("hollow/quarterly")),
            summary: Some(s("Installed 63c79a88 at 2026-10-04T10:00:00Z; the running app reports the same commit.")),
            matches_running: Some(true),
        }),
        ..Default::default()
    };
    let activity = WorkspaceActivity {
        workspace_id: s("local-results-view"),
        as_of: s("2026-10-05T12:00:00Z"),
        events: vec![run, result, session, install],
        total: 9,
        gaps: ActivityGaps {
            runs_without_receipt: 1,
            runs_without_usage: 2,
            results_without_receipt: 0,
            turns_truncated: false,
            results_truncated: false,
            receipts_error: None,
            reviews_error: Some(s("Results review state could not be read")),
            usage_error: None,
        },
        next_cursor: Some(s(
            "WzE3NTk2NjAwMDAwMDAsMywic2Vzc2lvbjpob2xsb3ctc2Vzc2lvbi0xIl0",
        )),
    };
    fixture("activity", &activity);
}

/// A conversation moved from Claude to Codex: one key, two stints.
fn sample_session() -> crate::types::AgentSessionSummary {
    use crate::types::{AgentSessionSummary, HarnessStint};
    AgentSessionSummary {
        key: s("s-20261005-abcd0001"),
        title: Some(s("Draft the summary")),
        agent: s("codex"),
        harness_session_id: Some(s("codex-thread-9")),
        cwd: s("/Users/fixture/vault/workspaces/local-results-view"),
        created_at: s("2026-10-05T09:58:00Z"),
        last_activity_at: s("2026-10-05T10:05:00Z"),
        ended_at: None,
        turn_count: 2,
        parent_key: None,
        fork_kind: None,
        adopted: false,
        stints: vec![
            HarnessStint {
                agent: s("claude"),
                harness_session_id: Some(s("claude-session-1")),
                started_at: s("2026-10-05T09:58:00Z"),
                ended_at: Some(s("2026-10-05T10:00:00Z")),
                reason: Some(s("agent changed")),
            },
            HarnessStint {
                agent: s("codex"),
                harness_session_id: Some(s("codex-thread-9")),
                started_at: s("2026-10-05T10:00:00Z"),
                ended_at: None,
                reason: None,
            },
        ],
        resumable: s("native"),
        imported: None,
    }
}

#[test]
fn fixture_workspace_read_workspace() {
    use crate::workspace_reentry::{BriefStale, LastRun, WorkspaceReentry};
    use crate::workspaces::{
        ContextDocHealth, SourceContext, WorkFile, Workspace, WorkspaceDetail,
    };
    let session = sample_session();
    let detail = WorkspaceDetail {
        project_agent_key: crate::project_agents::key("local-results-view"),
        workspace: Workspace {
            id: s("local-results-view"),
            title: s("Results view"),
            status: s("active"),
            path: s("workspaces/local-results-view"),
            cwd: s("/Users/fixture/vault/workspaces/local-results-view"),
            modified: s("2026-10-05T10:05:00Z"),
            build: None,
            kind: None,
            merged_into: None,
            archived_from: None,
            canvas_theme: None,
            pinned: None,
        },
        files: vec![
            WorkFile {
                path: s("workspaces/local-results-view/PLAN.md"),
                name: s("PLAN.md"),
                kind: s("context"),
                size: 61_000,
                modified: Some(s("2026-10-05T09:00:00Z")),
            },
            WorkFile {
                path: s("workspaces/local-results-view/outputs/summary.md"),
                name: s("outputs/summary.md"),
                kind: s("output"),
                size: 2_048,
                modified: None,
            },
        ],
        sessions: vec![session],
        sources: vec![SourceContext {
            path: s("sources/q3-export.csv"),
            original: Some(s("/Users/fixture/Downloads/q3-export.csv")),
            captured: Some(s("2026-10-04T16:00:00Z")),
            freshness: s("unchanged_metadata"),
            coverage: s("excerpt"),
            included_bytes: 24_000,
            total_bytes: 81_920,
        }],
        sources_omitted: 2,
        reentry: WorkspaceReentry {
            as_of: s("2026-10-05T12:00:00Z"),
            next_step: Some(s("Send the summary to Dana.")),
            open_items: strings(&["Send the summary to Dana"]),
            open_item_count: Some(3),
            last_run: Some(LastRun {
                session_key: s("s-20261005-abcd0001"),
                status: Some(s("success")),
                finished_at: Some(s("2026-10-05T10:05:00Z")),
                failed_checks: Some(0),
                check_count: Some(4),
                unknown_checks: None,
                verification: Some(s("verified")),
            }),
            gaps: Vec::new(),
            brief_stale: Some(BriefStale {
                days: 9,
                conversations: 2,
            }),
        },
        context_health: vec![
            ContextDocHealth {
                name: s("WORKSPACE.md"),
                bytes: 3_100,
                included: 3_100,
                limit: 24_000,
            },
            ContextDocHealth {
                name: s("PLAN.md"),
                bytes: 61_000,
                included: 44_900,
                limit: 44_900,
            },
        ],
    };
    fixture("workspace", &detail);
}

#[test]
fn fixture_digests_workspace_digests() {
    use crate::workspace_reentry::WorkspaceReentry;
    use crate::workspace_results::{LatestResult, WorkspaceDigest};
    let reentry = || WorkspaceReentry {
        as_of: s("2026-10-05T12:00:00Z"),
        next_step: None,
        open_items: Vec::new(),
        open_item_count: None,
        last_run: None,
        gaps: strings(&["No SOURCES.md yet."]),
        brief_stale: None,
    };
    let counted = WorkspaceDigest {
        workspace_id: s("local-results-view"),
        reentry: reentry(),
        latest_result: Some(LatestResult {
            path: s("workspaces/local-results-view/outputs/summary.md"),
            title: s("Summary"),
            kind: s("document"),
            modified: Some(s("2026-10-05T10:05:00Z")),
            review: s("ready"),
        }),
        result_count: 4,
        awaiting_review: 1,
        last_activity_at: Some(s("2026-10-05T10:05:00Z")),
        results_error: None,
    };
    // A damaged review store: nothing counted, and it says why (B·F08).
    let damaged = WorkspaceDigest {
        workspace_id: s("local-q3-review"),
        reentry: reentry(),
        latest_result: None,
        result_count: 0,
        awaiting_review: 0,
        last_activity_at: None,
        results_error: Some(s("Results review state could not be read")),
    };
    fixture("digests", &[counted, damaged]);
}

#[test]
fn fixture_run_queue_list_run_queue() {
    use crate::types::RunQueueItem;
    let chat = RunQueueItem {
        id: 44,
        source: s("chat"),
        source_id: Some(s("s-20261005-abcd0001")),
        label: s("Chat"),
        detail: Some(s("Summarise the launch notes")),
        reason: s("chat"),
        status: s("queued"),
        priority: 0,
        position: Some(1),
        attempt: 0,
        coalesced_count: 0,
        requested_at: s("2026-10-05T10:06:00Z"),
        claimed_at: None,
        finished_at: None,
        history_run_id: None,
        error: None,
        waiting_since: None,
        waiting_holder: None,
        held_reason: None,
        cancel_requested: false,
        waiting_reason: Some(s(
            "\u{201c}Fix the panel\u{201d} to finish (it has used this worktree for 12 min)",
        )),
        waiting_session: Some(s("s-20261005-0a1b2c3d")),
        working: None,
    };
    // A finished row: `waitingReason` is null, never absent.
    let cron = RunQueueItem {
        id: 41,
        source: s("cron"),
        source_id: Some(s("morning-brief")),
        label: s("Morning brief"),
        detail: None,
        reason: s("scheduled"),
        status: s("done"),
        priority: 3,
        position: None,
        attempt: 1,
        coalesced_count: 2,
        requested_at: s("2026-10-05T08:00:00Z"),
        claimed_at: Some(s("2026-10-05T08:00:01Z")),
        finished_at: Some(s("2026-10-05T08:00:43Z")),
        history_run_id: Some(s("run-41")),
        error: None,
        waiting_since: None,
        waiting_holder: None,
        held_reason: None,
        cancel_requested: false,
        waiting_reason: None,
        waiting_session: None,
        working: None,
    };
    fixture("run-queue", &[chat, cron]);
}

#[test]
fn fixture_workspace_sessions_workspace_sessions() {
    use crate::types::AgentSessionSummary;
    use crate::workspaces::{SessionChanges, StartedBy, WorkspaceSessions};
    let parent = sample_session();
    // A child task the first conversation started (overnight F2).
    let child = AgentSessionSummary {
        key: s("s-20261005-abcd0002"),
        title: Some(s("Check the totals")),
        agent: s("claude"),
        harness_session_id: None,
        created_at: s("2026-10-05T10:01:00Z"),
        last_activity_at: s("2026-10-05T10:03:00Z"),
        ended_at: Some(s("2026-10-05T10:03:00Z")),
        turn_count: 1,
        stints: Vec::new(),
        resumable: s("briefOnly"),
        ..sample_session()
    };
    let busy = WorkspaceSessions {
        project_agent_key: crate::project_agents::key("local-results-view"),
        workspace_id: s("local-results-view"),
        total: 3,
        changes: vec![SessionChanges {
            key: parent.key.clone(),
            additions: 120,
            deletions: 8,
        }],
        started_by: vec![StartedBy {
            key: child.key.clone(),
            parent: parent.key.clone(),
        }],
        sessions: vec![parent, child],
    };
    // Nothing changed and nothing started another: `changes` is still sent,
    // `startedBy` is not.
    let quiet = WorkspaceSessions {
        project_agent_key: crate::project_agents::key("local-q3-review"),
        workspace_id: s("local-q3-review"),
        total: 0,
        sessions: Vec::new(),
        changes: Vec::new(),
        started_by: Vec::new(),
    };
    fixture("workspace-sessions", &[busy, quiet]);
}

#[test]
fn fixture_session_detail_read_agent_session() {
    use crate::types::{AgentSessionDetail, SessionRecordTurn};
    let detail = AgentSessionDetail {
        summary: sample_session(),
        turns: vec![
            SessionRecordTurn {
                ord: 1,
                run_id: Some(s("run-20261005-0001")),
                started_at: s("2026-10-05T09:58:00Z"),
                ended_at: Some(s("2026-10-05T10:00:00Z")),
                status: Some(s("success")),
                asked: Some(s("Draft the summary")),
            },
            // Still running: no end, no status yet.
            SessionRecordTurn {
                ord: 2,
                run_id: None,
                started_at: s("2026-10-05T10:04:00Z"),
                ended_at: None,
                status: None,
                asked: Some(s("Now shorten it")),
            },
        ],
        events_retained: true,
    };
    fixture("session-detail", &detail);
}

#[test]
fn fixture_history_get_history_run() {
    use crate::automated_history::{Coverage, HistoryChange, HistoryRun, UncapturedReason};
    use crate::run_guard::RunFlag;
    // Restored by a later run, and changed by a parallel chat in the same
    // folder too (so a restore would ask first).
    let edited = HistoryChange {
        vault_path: s("workspaces/local-results-view/outputs/summary.md"),
        change_type: s("modify"),
        before_hash: Some(s("sha256-summary-before")),
        after_hash: Some(s("sha256-summary-after")),
        before_size: Some(1_800),
        after_size: Some(2_048),
        text: true,
        notion_mirror: false,
        before_notion_id: None,
        after_notion_id: None,
        before_uncaptured: false,
        after_uncaptured: false,
        before_coverage: Some(Coverage::Captured),
        after_coverage: Some(Coverage::Captured),
        restored_at: Some(s("2026-10-05T11:00:00Z")),
        restored_by_run_id: Some(s("run-20261005-0003")),
        also_changed_by: strings(&["run-20261005-0002"]),
        // Another chat's change in the shared folder (F1, 2026-10-20).
        unattributed: true,
        // A run closed after its process died (Q-R11, 2026-11-15).
        unverified: true,
        hollow_write: false,
    };
    // Created over 2 MiB: History holds no copy, so restore refuses it.
    let large = HistoryChange {
        vault_path: s("workspaces/local-results-view/sources/q3-export.csv"),
        change_type: s("create"),
        before_hash: None,
        after_hash: None,
        before_size: None,
        after_size: Some(3_145_728),
        text: false,
        notion_mirror: false,
        before_notion_id: None,
        after_notion_id: None,
        before_uncaptured: false,
        after_uncaptured: true,
        before_coverage: Some(Coverage::Absent),
        after_coverage: Some(Coverage::Uncaptured {
            reason: Some(UncapturedReason::TooLarge),
        }),
        restored_at: None,
        restored_by_run_id: None,
        also_changed_by: Vec::new(),
        unattributed: false,
        unverified: false,
        hollow_write: false,
    };
    // A Notion-imported note whose earlier side History can't vouch for.
    let mirror = HistoryChange {
        vault_path: s("workspaces/local-results-view/notion-brief.md"),
        change_type: s("modify"),
        before_hash: None,
        after_hash: Some(s("sha256-brief-after")),
        before_size: None,
        after_size: Some(900),
        text: true,
        notion_mirror: true,
        before_notion_id: Some(s("notion-page-1")),
        after_notion_id: Some(s("notion-page-1")),
        before_uncaptured: true,
        after_uncaptured: false,
        before_coverage: Some(Coverage::Unknown),
        after_coverage: Some(Coverage::Captured),
        restored_at: None,
        restored_by_run_id: None,
        also_changed_by: Vec::new(),
        unattributed: false,
        unverified: false,
        hollow_write: false,
    };
    let run = HistoryRun {
        id: s("run-20261005-0001"),
        source: s("chat"),
        source_id: Some(s("s-20261005-abcd0001")),
        started_at: s("2026-10-05T09:58:00Z"),
        finished_at: Some(s("2026-10-05T10:00:00Z")),
        status: s("success"),
        changes: vec![edited, large, mirror],
        flags: vec![RunFlag {
            kind: s(crate::run_guard::FLAG_AGENT_INSTRUCTIONS),
            path: s("workspaces/local-results-view/CLAUDE.md"),
            change_type: Some(s("modify")),
            detail: None,
            scan: None,
        }],
        scope: Some(s("workspaces/local-results-view")),
        overlaps: strings(&["run-20261005-0002"]),
    };
    fixture("history", &run);
}
