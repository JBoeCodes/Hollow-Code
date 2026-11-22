//! The tracker's IPC shapes (overhaul A2): what Settings, Connectors, Task
//! tracker (A14) and the rebuilt Tasks page (B1 to B4) read and send.
//!
//! Commands (`commands/tracker_api.rs`): `tracker_status`, `tracker_view`,
//! `tracker_issue`, `tracker_team_meta` (reads); `tracker_sync`,
//! `edit_tracker_item`, `create_tracker_task`, `create_tracker_project`,
//! `archive_tracker_task` (the person's clicks; project creation is still
//! unavailable); `set_tracker_key`, `clear_tracker_key`,
//! `use_keychain_tracker_key`, `set_tracker_settings` (Settings). The TS
//! types are generated into `src/generated/ipc.ts`; the typed wrappers are in
//! `src/trackerApi.ts`.

use super::settings::KeyState;
use super::{
    Field, ProjectHealth, RateBudget, StateKind, TrackerCycle, TrackerIssue, TrackerLabel,
    TrackerMilestone, TrackerState, TrackerTeam, TrackerUser,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Where the tracker stands, for Settings and the Tasks page header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum TrackerRunState {
    /// No provider chosen.
    Off,
    /// A provider is chosen but its team or key is missing (`missing` says which).
    NeedsSetup,
    /// Set up, but something else holds the task files (`blockedBy` says
    /// what). Nothing does since Notion two-way sync was removed
    /// (2026-11-10); kept so the IPC vocabulary doesn't change.
    #[allow(dead_code)]
    Blocked,
    /// Set up and waiting for the next pass.
    Ready,
    Syncing,
    /// The last pass failed (`lastError`).
    Error,
}

/// One provider Settings can offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerProviderOption {
    pub id: String,
    pub display_name: String,
    /// The provider has a Keychain item Settings can copy the key from.
    pub keychain_item: bool,
    pub transports: Vec<String>,
}

/// `tracker_status`. Never carries the key.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerStatus {
    pub state: TrackerRunState,
    /// The chosen provider's id; null when off.
    pub provider: Option<String>,
    pub display_name: Option<String>,
    pub providers: Vec<TrackerProviderOption>,
    pub team_id: Option<String>,
    pub team_key: Option<String>,
    /// The optional personal team (Personal), beside the shared one.
    pub personal_team_id: Option<String>,
    pub personal_team_key: Option<String>,
    pub transport: String,
    pub poll_seconds: u64,
    /// Settings, Task tracker, "Track agent work in the tracker" (X1): the
    /// running value; a saved change applies after a restart.
    pub track_agent_work: bool,
    /// Settings, Task tracker, "Write a Friday status at 3 PM" (A18), as
    /// saved; a change applies after a restart (`restartNeeded`).
    pub friday_status: bool,
    /// Settings, Task tracker, "Also post it to the tracker" (A18), as saved.
    pub post_status_updates: bool,
    pub key: KeyState,
    /// What a person still has to do, in order; null when nothing.
    pub missing: Option<String>,
    /// What holds the task files instead; always null since Notion two-way
    /// sync was removed (2026-11-10).
    pub blocked_by: Option<String>,
    pub last_sync: Option<String>,
    pub next_sync: Option<String>,
    pub last_error: Option<String>,
    /// Requests and complexity left this hour, when the provider reported them.
    pub rate: Option<RateBudget>,
    /// Edits waiting to be sent (offline or rate-limited).
    pub outbox: u32,
    /// Config values that were ignored while loading.
    pub problems: Vec<String>,
    /// config.json holds tracker settings this run hasn't applied yet.
    pub restart_needed: bool,
    /// Whether the sync engine is part of this build (false until T3 lands).
    pub engine: bool,
}

/// Why a mirrored task's fields are held in the Tasks page editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)] // A6 constructs the legacy, missing-ledger and copied-file locks.
pub enum TrackerLockKind {
    /// A ledger record at this path; tracker-owned fields edit through the tracker.
    Linked,
    /// The issue is gone or unreadable remotely; only its project is held.
    RemoteMissing,
    /// The record points at another path (moved, or a copy of a linked file).
    Moved,
    /// `.tracker-sync.json` can't be read; every mapped field is held.
    Unreadable,
    /// A legacy relation key (`notion_project_ids`) with no ledger record.
    StaleRelations,
    /// The issue left the scoped team; sync stopped for it.
    OutOfScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerLock {
    pub kind: TrackerLockKind,
    /// Vault keys held read-only in the editor.
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerTaskMeta {
    pub provider: String,
    pub id: String,
    pub key: Option<String>,
    pub url: Option<String>,
    pub status_type: Option<StateKind>,
    /// Provider priority, 0 none, 1 urgent, 2 high, 3 medium, 4 low.
    pub priority: u8,
    pub lock: TrackerLock,
}

/// `tracker_view`'s filters. Everything optional; no filter = the open issues.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerViewQuery {
    #[serde(default)]
    pub state_kinds: Option<Vec<StateKind>>,
    #[serde(default)]
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub cycle_id: Option<String>,
    /// Words to find in the key or title.
    #[serde(default)]
    pub search: Option<String>,
    /// Completed and canceled issues too.
    #[serde(default)]
    pub include_closed: bool,
    #[serde(default)]
    pub include_archived: bool,
    /// One team's key (`AI`, `HOL`); none = both teams.
    #[serde(default)]
    pub team_key: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One row of the issue list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerViewRow {
    pub id: String,
    pub key: String,
    pub title: String,
    pub status_name: String,
    pub status_kind: StateKind,
    /// 0 none, 1 urgent, 2 high, 3 medium, 4 low.
    pub priority: u8,
    pub labels: Vec<String>,
    pub project: Option<String>,
    pub project_id: Option<String>,
    pub due: Option<String>,
    pub assignee: Option<String>,
    pub assignee_id: Option<String>,
    pub estimate: Option<f64>,
    pub cycle: Option<String>,
    pub milestone: Option<String>,
    /// The key of the team the issue is in (`AI`, `HOL`).
    pub team_key: Option<String>,
    pub updated_at: String,
    pub url: String,
    /// The mirrored task file, vault-relative.
    pub path: Option<String>,
    pub lock: Option<TrackerLock>,
    /// An edit is waiting to be sent.
    pub pending: bool,
    /// The tracker overrode a local edit; the issue detail shows the note.
    pub conflict: bool,
    pub archived: bool,
    /// Where the issue lives, in `sources::label`'s words (H4b), filled by
    /// the `tracker_view` command from the saved settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub source_label: Option<crate::sources::SourceLabel>,
}

/// One row of the Projects view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerProjectRow {
    pub id: String,
    pub key: Option<String>,
    pub name: String,
    pub status_name: String,
    /// backlog, planned, started, paused, completed, canceled.
    pub status_kind: String,
    pub lead: Option<String>,
    pub target_date: Option<String>,
    /// 0 to 1.
    pub progress: Option<f64>,
    pub issue_count: u32,
    pub open_count: u32,
    pub url: String,
    /// The workspace that tracks this project, when one does.
    pub workspace_id: Option<String>,
    /// The key of the team the project is in (`AI`, `HOL`).
    pub team_key: Option<String>,
}

/// `tracker_view`: the mirrored issues and projects.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerView {
    pub provider: Option<String>,
    pub display_name: Option<String>,
    pub team_key: Option<String>,
    /// The keys of the teams the tracker syncs, the shared team first.
    pub team_keys: Vec<String>,
    pub last_sync: Option<String>,
    pub sync_error: Option<String>,
    /// The team's states in board order, for grouping.
    pub states: Vec<TrackerState>,
    pub rows: Vec<TrackerViewRow>,
    pub projects: Vec<TrackerProjectRow>,
    /// Matching rows before `limit`.
    pub total: u32,
    /// Why the list is empty when it is (off, not set up, not synced yet).
    pub note: Option<String>,
}

/// A local edit the tracker overrode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerConflict {
    pub field: Field,
    /// The value Hollow had, as text.
    pub local: String,
    /// The value the tracker kept, as text.
    pub remote: String,
    pub at: String,
}

/// `tracker_issue`: one issue with everything the detail shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerIssueDetail {
    pub issue: TrackerIssue,
    pub project: Option<TrackerProjectRow>,
    pub path: Option<String>,
    pub workspace_id: Option<String>,
    pub lock: Option<TrackerLock>,
    /// Fields the editor may change (the adapter's writable fields).
    pub writable_fields: Vec<Field>,
    pub conflicts: Vec<TrackerConflict>,
    /// Fields waiting to be sent.
    pub pending: Vec<Field>,
    /// The task file's vault-only notes (below the mirrored blocks).
    pub vault_notes: Option<String>,
}

/// A project for pickers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerProjectOption {
    pub id: String,
    pub name: String,
}

/// `tracker_team_meta`: what the pickers and filters offer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerTeamMeta {
    pub team: TrackerTeam,
    pub states: Vec<TrackerState>,
    pub labels: Vec<TrackerLabel>,
    pub members: Vec<TrackerUser>,
    pub cycles: Vec<TrackerCycle>,
    pub milestones: Vec<TrackerMilestone>,
    pub projects: Vec<TrackerProjectOption>,
    /// Whom the saved key belongs to (the adapter's `whoami`), for the Tasks
    /// page's "My issues". None until a pass has read it; older ledgers
    /// load without it.
    #[serde(default)]
    pub viewer: Option<TrackerUser>,
}

/// `edit_tracker_item`: new values by field (`TrackerField` names), with the
/// values the editor read (a compare-and-swap, like the Notion editor's).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerEdit {
    pub id: String,
    #[cfg_attr(test, ts(type = "Record<string, unknown>"))]
    pub values: BTreeMap<String, serde_json::Value>,
    #[cfg_attr(test, ts(type = "Record<string, unknown>"))]
    pub expected: BTreeMap<String, serde_json::Value>,
}

/// `create_tracker_task`. Names, not ids, where a person types them; the
/// engine resolves them against the team's metadata.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerNewTask {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub state_id: Option<String>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub estimate: Option<f64>,
    #[serde(default)]
    pub cycle_id: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    /// The workspace to file the mirrored task under.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// The team to create it in (`AI` or `HOL`); the shared team when absent.
    #[serde(default)]
    pub team_key: Option<String>,
}

/// `create_tracker_project`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerNewProject {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub lead_id: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    /// "Track in the tracker" for an existing vault-only workspace: once the
    /// project exists, its PROJECT.md names it, so the next pass adopts that
    /// workspace instead of making a new one.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// The team to create it in (`AI` or `HOL`); the shared team when absent.
    #[serde(default)]
    pub team_key: Option<String>,
}

/// What a write did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerWriteOutcome {
    /// The issue or project id, once the tracker has it.
    pub id: Option<String>,
    pub key: Option<String>,
    pub path: Option<String>,
    /// Waiting in the outbox (offline or rate-limited) rather than sent.
    pub queued: bool,
    pub note: String,
}

/// `tracker_sync`: one pass's summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerSyncOutcome {
    pub pulled: u32,
    pub pushed: u32,
    pub conflicts: u32,
    pub dropped_out_of_scope: u32,
    pub errors: Vec<String>,
    pub finished_at: String,
}

/// `write_friday_status` and `tracker.friday_status` (A18): the notes
/// written into Hollow and any tracker copies.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerFridayStatus {
    /// The local date the notes are named for (`YYYY-MM-DD`).
    pub date: String,
    /// The COO's combined digest (vault path); a one-project run writes none.
    pub digest_path: Option<String>,
    /// The line the Herald reads: the digest's title, or the one project's.
    pub herald_line: String,
    pub projects: Vec<TrackerStatusNote>,
    /// The tracker copies tried this run (none while the setting is off).
    pub copies: Vec<TrackerStatusCopy>,
}

/// One project's Friday status note.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerStatusNote {
    pub workspace_id: String,
    pub title: String,
    pub project_id: String,
    /// Vault path of the note.
    pub path: String,
    pub done: u32,
    pub in_progress: u32,
    /// Blocked or overdue.
    pub blocked: u32,
    pub next: u32,
    pub health: ProjectHealth,
}

/// One tracker copy of a project's status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TrackerStatusCopy {
    pub workspace_id: String,
    pub project_id: String,
    /// Confirmed in the tracker.
    pub posted: bool,
    pub note: String,
}
