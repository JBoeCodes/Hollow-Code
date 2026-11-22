use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub url: String,
    pub title: String,
    pub status: String,
    /// Four-level priority `P1`..`P4` (Todoist-style). Parsing is tolerant of
    /// the legacy `High`/`Medium`/`Low` values (mapped High→P1, Medium→P2,
    /// Low→P3); `None` means unprioritized (displayed as P4).
    #[cfg_attr(test, ts(as = "Option<crate::ts_bindings::Priority>"))]
    pub priority: Option<String>,
    pub due: Option<String>,
    pub effort: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(rename = "projectUrl", alias = "projectUrl")]
    pub project_url: Option<String>,
    #[serde(skip_deserializing)]
    pub project_name: Option<String>,
    #[serde(skip_deserializing)]
    pub days_until_due: Option<i64>,
    #[serde(skip_deserializing)]
    #[cfg_attr(test, ts(as = "crate::ts_bindings::TaskBucket"))]
    pub bucket: String,
    /// Vault-relative path to the task's markdown file, e.g. `tasks/foo.md`.
    /// The UI opens this in the Vault tab — the vault is the only link target.
    #[serde(skip_deserializing)]
    pub vault_path: Option<String>,
    /// Cross-cutting labels (Todoist `@label`). Stored as a comma-separated
    /// `labels:` scalar in frontmatter; `label/{slug}` is also derived into
    /// the `tags:` block for Obsidian search.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Section within the task's project (logical grouping). Frontmatter scalar.
    #[serde(default)]
    pub section: Option<String>,
    /// Optional clock time `HH:MM` paired with `due`. Drives the Calendar view.
    #[serde(default)]
    pub due_time: Option<String>,
    /// Optional duration in minutes — block height in the Calendar view.
    #[serde(default)]
    pub duration_min: Option<u32>,
    /// Human recurrence phrase, e.g. `every weekday`. Rolled forward on
    /// completion rather than fired by a scheduler.
    #[serde(default)]
    pub recurrence: Option<String>,
    /// Sub-tasks parsed from the body markdown checklist (`- [ ]` / `- [x]`).
    /// Computed on read; never deserialized from the client.
    #[serde(default, skip_deserializing)]
    pub subtasks: Vec<SubTask>,
    /// sha256 hex of the exact file text this snapshot was parsed from. The
    /// compare-and-swap baseline an editor sends back as
    /// `UpdateTaskInput.expected`.
    #[serde(default, skip_deserializing)]
    pub content_hash: String,
    /// Provider-neutral identity, state kind and ownership of a tracker mirror.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub tracker: Option<crate::tracker::ipc::TrackerTaskMeta>,
    /// Where the task lives, in words computed by `sources::label` (H4b):
    /// "Tracked in Linear", "Stored in Hollow", "Linear unreachable since
    /// 9:40 AM". Filled by list readers that know the settings (the Tasks
    /// list, the phone's `/v1/tasks`); absent elsewhere.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub source_label: Option<crate::sources::SourceLabel>,
    /// Backend-only: the file's legacy `notion_id` (Notion import history),
    /// read for the phone's `notionLinked` alias.
    #[serde(skip)]
    pub notion_id: Option<String>,
    /// The stored idempotency fingerprint (sha256 hex of a create key),
    /// written once at create (Paperclip PR 11). Read-only.
    #[serde(default, skip_deserializing)]
    pub fingerprint: Option<String>,
    /// Where an idempotently created task came from (`heartbeat`,
    /// `meeting:<path>`, `project:<rel>`, `plan:<path>`). Read-only.
    #[serde(default, skip_deserializing)]
    pub origin: Option<String>,
    /// The content of the body's `## Done when` sections (verbatim, joined by
    /// a blank line), or `None`. Read-only; never part of the description.
    #[serde(default, skip_deserializing)]
    pub done_when: Option<String>,
    /// The content of the body's `## Activity` sections, like `done_when`.
    #[serde(default, skip_deserializing)]
    pub activity: Option<String>,
    /// The tasks this one waits on: `vault:tasks/…md` URLs from the
    /// `blocked_by:` frontmatter (Paperclip PR 11b). Hollow-only; never
    /// mapped to a tracker.
    #[serde(default)]
    pub blocked_by: Vec<String>,
    /// The tasks whose `blocked_by` names this one (the reverse index),
    /// sorted. Computed by `vault_tasks::list` only; empty from `get`.
    #[serde(default, skip_deserializing)]
    pub blocks: Vec<String>,
    /// Not finished, not cancelled, not on an explicit hold status, and every
    /// blocker resolved (terminal). Computed by `vault_tasks::list` only;
    /// `false` from `get`.
    #[serde(default, skip_deserializing)]
    pub ready: bool,
    /// The `blocked_by` entries that are missing or not yet terminal.
    /// Computed by `vault_tasks::list` only.
    #[serde(default, skip_deserializing)]
    pub unresolved_blockers: Vec<String>,
    /// Blockers that will never resolve on their own (`missing`, `cancelled`,
    /// `recurring`) and blocker loops (`cycle`). Computed by
    /// `vault_tasks::list` only.
    #[serde(default, skip_deserializing)]
    pub blocker_warnings: Vec<BlockerWarning>,
}

/// A `blocked_by` entry that needs attention (Paperclip PR 11b).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BlockerWarning {
    /// The `blocked_by` entry as stored.
    pub url: String,
    /// `missing` (no task at that path: renamed, moved, archived or deleted),
    /// `cancelled` (its status is cancelled / won't do), `recurring` (Hollow
    /// rolls a recurring task forward instead of completing it) or `cycle`
    /// (the blocker waits, directly or through others, on this task).
    #[cfg_attr(test, ts(as = "crate::ts_bindings::BlockerWarningKind"))]
    pub kind: String,
}

/// One body-checklist item parsed out of a task file's markdown body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubTask {
    pub text: String,
    pub done: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub url: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub description: String,
    pub created: Option<String>,
    #[serde(skip_deserializing)]
    pub vault_path: Option<String>,
    /// Project priority (`P1`..`P4`, tolerant of legacy values). Frontmatter.
    #[serde(default)]
    pub priority: Option<String>,
    /// Optional target/ship date (`YYYY-MM-DD`). Frontmatter.
    #[serde(default)]
    pub target_date: Option<String>,
    /// When the project actually shipped (RFC3339). Stamped automatically
    /// when the file moves into the `deployed` status folder; cleared when
    /// it moves back out. Powers the Portfolio weekly-velocity metric.
    #[serde(default)]
    pub deployed_at: Option<String>,
    /// Lightweight outcome contract. These fields are optional so existing
    /// project files keep their legacy frontmatter shape until a contributor
    /// explicitly adds an outcome brief.
    #[serde(default)]
    pub current_state: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub non_goals: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
    /// Stable vault URLs of the goals this project serves. Hollow-local even
    /// when Notion owns the rest of the project mirror.
    #[serde(default)]
    pub goal_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub path: String,
    pub name: String,
    pub mtime: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteContent {
    pub path: String,
    pub content: String,
    pub mtime: String,
    pub size: u64,
    /// sha256 hex of `content`'s bytes (`vault::revision`): the expected
    /// revision a writer passes back so a write over an edit it never saw
    /// is refused (merged plan slice 4, A·D7).
    pub revision: String,
}

/// Outcome of the editor's compare-and-swap save. `conflict: false` ⇒ written
/// (`note` = what was saved); `conflict: true` ⇒ the file changed underneath
/// (`note` = the current on-disk version, for reload / keep-mine).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveNoteResult {
    pub conflict: bool,
    pub note: NoteContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub path: String,
    pub name: String,
    pub mtime: String,
    pub size: u64,
    pub snippet: String,
    pub score: i64,
    /// Semantic kind from the vault index (so the UI can group/filter results
    /// and route a hit to its canonical surface). See `vault_index::classify`.
    pub kind: String,
}

/// Status of the local semantic-search index for the Settings row: whether the
/// feature is enabled, and how many of the total chunks have been embedded
/// (backfill progress). See `vault_embed_index`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticStatus {
    pub enabled: bool,
    pub indexed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub project: String,
    pub project_dir: String,
    pub session_id: String,
    /// Short AI-generated title (Claude-Code-style summary), cached on disk.
    /// `None` until generated; the frontend falls back to `first_prompt`.
    pub title: Option<String>,
    pub first_prompt: String,
    pub message_count: u32,
    pub last_activity: String,
    pub size_bytes: u64,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTurn {
    pub role: String,
    pub text: String,
    pub timestamp: Option<String>,
    pub tool_uses: Vec<ToolUse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUse {
    /// Claude transcript tool-use identifier. Older/non-Claude transcripts may
    /// omit it, so rehydration keeps this optional.
    pub id: Option<String>,
    pub name: String,
    pub input: serde_json::Value,
    /// Flattened matching `tool_result` content, when the transcript contains
    /// one. `Some("")` is distinct from no result (an unfinished call).
    pub result: Option<String>,
    /// Error state from the matching `tool_result`; absent until a result is
    /// observed.
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub project: String,
    pub session_id: String,
    pub turns: Vec<SessionTurn>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkSource {
    pub path: String,
    pub basename: String,
    pub matches: Vec<BacklinkMatch>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkMatch {
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRef {
    pub path: String,
    pub basename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub sessions: Vec<SessionMeta>,
    pub sessions_total: usize,
    pub recent_notes: Vec<Note>,
    pub tasks: Vec<Task>,
    pub tasks_total: usize,
    pub overdue_count: usize,
    pub today_count: usize,
    pub active_projects: Vec<Project>,
    pub projects_total: usize,
    pub tasks_fetched: Option<String>,
    pub projects_fetched: Option<String>,
    pub timestamp: String,
}

/// Git contribution heatmap for the Today dashboard. A dense day array over
/// the trailing ~53 weeks (one entry per day, `count` 0 on idle days) plus
/// roll-up stats — the "how many days did I ship something" view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitActivity {
    /// One entry per calendar day from `start` through `end`, inclusive and
    /// in order, so the frontend can chunk it into 7-row week columns.
    pub days: Vec<GitDay>,
    pub total_commits: usize,
    /// Distinct days with at least one commit.
    pub active_days: usize,
    /// Consecutive active days ending today (or yesterday if today is idle).
    pub current_streak: usize,
    /// Longest consecutive-active-day run anywhere in the window.
    pub longest_streak: usize,
    /// How many git repos were found + scanned under the root.
    pub repos_scanned: usize,
    pub start: String,
    pub end: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDay {
    /// `YYYY-MM-DD`, local timezone.
    pub date: String,
    pub count: u32,
}

/// Snapshot of resolved paths + runtime toggles for the Settings page.
/// Things the UI can't see on its own (env vars, picked-up binary path)
/// live here. Pure read — no mutation or network.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub vault_path: String,
    pub vault_exists: bool,
    pub config_path: String,
    /// Absolute path to the `claude` binary the chat / runner picked up,
    /// or `None` if not found. Surface this so users can see whether a
    /// PATH shadowing issue is what's breaking chat / sync.
    pub claude_binary: Option<String>,
    pub debug_log_dir: String,
    /// Reflects the `HOLLOW_DEBUG_LOG` env var observed at launch.
    pub debug_log_enabled: bool,
    /// How many `.log` files currently live in the debug log dir.
    pub debug_log_files: usize,
    /// The personal context's health for the capability doctor (PX7): do
    /// the vault's and the clone's `me/` agree, and how fresh are the home
    /// file and the memories. `None` while no personal clone is set.
    pub context_health: Option<crate::context_layers::ContextHealth>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedTask {
    pub url: String,
    pub id: String,
    /// True when an earlier create with the same `idempotencyKey` already
    /// made this task, so nothing new was written.
    pub existing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct TasksResponse {
    pub items: Vec<Task>,
    pub fetched: Option<String>,
    pub source: String,
    pub todo_path: Option<String>,
    /// RFC3339 stamp of the last successful upstream sync for this domain
    /// (None in vault mode, or before the first Notion sync completes).
    pub last_synced_at: Option<String>,
    /// Last upstream sync failure — the mirrors still serve (graceful
    /// degradation); the UI shows a warn chip with this message.
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectsResponse {
    pub items: Vec<Project>,
    pub fetched: Option<String>,
    pub source: String,
    /// See `TasksResponse::last_synced_at` / `sync_error`.
    pub last_synced_at: Option<String>,
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    pub title: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub due_time: Option<String>,
    #[serde(default)]
    pub duration_min: Option<u32>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub recurrence: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    /// Legacy project display name; use project_url for an exact relationship.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    /// Optional idempotency key (at most 16 KiB). A create whose key matches
    /// an existing task's stored fingerprint returns that task
    /// (`CreatedTask.existing`) instead of writing a `slug-2.md` twin.
    #[serde(default)]
    pub idempotency_key: Option<String>,
    /// Optional provenance stored as the task's `origin:` (one line, 200 chars).
    #[serde(default)]
    pub origin: Option<String>,
    /// Optional `vault:tasks/…` blockers. Each must name an existing task and
    /// must not close a cycle (`TASK_BLOCKER: …` / `TASK_CYCLE: …`).
    #[serde(default)]
    pub blocked_by: Vec<String>,
}

/// Full-edit payload for `update_task`. Field semantics: `None` leaves the
/// field unchanged; `Some("")` (or an empty Vec) clears it; `Some(value)`
/// sets it. Editors send only the fields the user changed.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskInput {
    /// The `Task.contentHash` the editor loaded. When non-empty the write is
    /// compare-and-swap: a mismatch returns `TASK_CONFLICT: …` and writes
    /// nothing. Absent or empty means last-writer-wins (still line-patched
    /// under the vault mutate lock).
    #[serde(default)]
    pub expected: Option<String>,
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub due_time: Option<String>,
    #[serde(default)]
    pub duration_min: Option<u32>,
    #[serde(default)]
    pub recurrence: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subtasks: Option<Vec<SubTask>>,
    #[serde(default)]
    pub project: Option<String>,
    /// Replaces the task's `blocked_by` list when present (an empty list
    /// clears it). Only newly added entries are validated: each must name an
    /// existing task (`TASK_BLOCKER: …`) and must not close a cycle
    /// (`TASK_CYCLE: …`); stored entries that no longer resolve are kept.
    #[serde(default)]
    pub blocked_by: Option<Vec<String>>,
}

/// Result of `update_task`: the task's stable URL and the sha256 of the bytes
/// just written, so an optimistic client patch can chain the next edit
/// without a refetch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatedTask {
    pub url: String,
    pub content_hash: String,
}

/// Structured result of natural-language quick-add parsing. The UI shows
/// these as a live preview; `create_task` consumes the same fields.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedQuickAdd {
    pub title: String,
    pub priority: Option<String>,
    pub due: Option<String>,
    pub due_time: Option<String>,
    pub duration_min: Option<u32>,
    pub recurrence: Option<String>,
    pub project: Option<String>,
    pub labels: Vec<String>,
}

/// In-app create-project payload.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub title: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub current_state: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub non_goals: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
    #[serde(default)]
    pub goal_urls: Vec<String>,
}

/// In-app edit-project payload. `url` identifies the file; every other
/// field is optional — `None` leaves the current value untouched, `Some("")`
/// clears it (for the nullable fields). Mirrors `UpdateTaskInput`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectInput {
    pub url: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub current_state: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub non_goals: Option<String>,
    #[serde(default)]
    pub success_evidence: Option<String>,
    /// None leaves links unchanged; Some([]) clears them.
    #[serde(default)]
    pub goal_urls: Option<Vec<String>>,
}

/// A saved task filter — a named query string evaluated client-side
/// (e.g. `today & p1 & @errand`). Persisted to `vault/.task-filters.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFilter {
    pub id: String,
    pub name: String,
    pub query: String,
}

/// AI assist request. `action` is `breakdown` | `rephrase` | `next_steps`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistInput {
    pub action: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The task's linked project (its stored `project_url`). Only
    /// `breakdown` and `next_steps` use it, to add the project's goals as
    /// reference evidence (Paperclip PR 10).
    #[serde(default)]
    pub project_url: Option<String>,
}

/// AI assist result. `suggestions` holds sub-task / next-step lines;
/// `text` holds rephrased prose (for the `rephrase` action).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistResult {
    pub suggestions: Vec<String>,
    pub text: Option<String>,
}

// ── Workflows ────────────────────────────────────────────────────────────
//
// A workflow is a Claude-driven automation stored in
// `vault/workflows/{slug}.md`. The body is the prompt template; the
// frontmatter holds schedule + run metadata. Schedules are 7-field cron
// expressions (the format the `cron` crate consumes) or the literal
// string `manual` to mean "no automatic firing — run on demand only."

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub slug: String,
    pub title: String,
    pub description: String,
    /// `manual` or a 7-field cron expression accepted by the `cron` crate.
    pub schedule: String,
    /// Which agent CLI runs this workflow (`claude` | `codex` | `antigravity`).
    /// None ⇒ Claude (back-compat: pre-existing workflow files have no `agent`).
    pub agent: Option<String>,
    pub model: Option<String>,
    /// Optional working directory override. Empty/None → spawn from vault.
    pub cwd: Option<String>,
    /// Optional vault-relative glob of files this workflow produces
    /// (e.g. `briefings/*.html`). Powers the outputs view.
    pub output_glob: Option<String>,
    /// Built-in routine marker (`morning-brief` | `plan-day` | `close-day` |
    /// `close-week`). None ⇒ a normal user-authored workflow. Drives the
    /// the Automations page routine list.
    pub routine: Option<String>,
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// Event triggers (the `triggers:` frontmatter comma-list). Empty ⇒
    /// schedule-only. `#[serde(default)]` keeps pre-existing files byte-compat.
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Run inside this chat (overhaul F1): the conversation this automation
    /// is bound to (the `in_chat:` frontmatter key). Each run is a new turn
    /// there, in its handover chain's current conversation, instead of a new
    /// chat (`automation_in_chat`). None: a new chat each run. Omitted on
    /// the wire when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub in_chat: Option<String>,
    /// Owner Mac (M4, the `runs_on:` frontmatter key): the machine slug or
    /// id of the one Mac whose scheduler fires it (`runs_on`). None: no
    /// owner. Omitted on the wire when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub runs_on: Option<String>,
    pub created: Option<String>,
    pub last_run_at: Option<String>,
    /// `success` | `error` | `running` | `cancelled` | (None = never run)
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_duration_sec: Option<u64>,
    pub next_run_at: Option<String>,
    /// Misfire grace window in minutes (`misfire_grace_min` frontmatter).
    /// A scheduled run whose `next_run_at` is further past than this is
    /// skipped + rescheduled instead of fired late. None ⇒ default (120).
    pub misfire_grace_min: Option<u64>,
    /// Scheduler v2: a window schedule's catch-up (`skip`, `run-once-on-wake`
    /// or `run-all`), read from the stamped schedule string
    /// (`window:<name>[/<policy>]`). None for a cron (the misfire grace).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub catch_up: Option<String>,
    /// Why `schedule` can't be read (the [`crate::vault_workflows::validate_schedule`]
    /// error: a broken cron, an unknown window or catch-up). None when it
    /// reads. The scheduler never fires such an automation, so the UI says so
    /// instead of showing "—". Filled in `build_workflow`, so `list` and
    /// `read_full` agree; omitted on the wire when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub schedule_problem: Option<String>,
    /// `skip_if_quiet` frontmatter (Paperclip PR 9). Since 9b a scheduled
    /// occurrence is skipped when nothing under `watch` changed since the last
    /// successful run started (`workflow_hygiene::quiet_config` and
    /// `quiet_check`, gated in the run-queue claim).
    pub skip_if_quiet: bool,
    /// `watch` frontmatter comma-list of globs the quiet check walks
    /// (Paperclip PR 9b; defaults to `workspaces/<id>/**` for a workflow
    /// whose `cwd` is exactly that workspace, see `quiet_config`).
    pub watch: Vec<String>,
    /// `precheck` frontmatter (OC13): a one-line shell command run before a
    /// SCHEDULED occurrence is admitted. Exit 0 runs the turn; any other
    /// exit, a failed start or the 20 s limit skips it (`skipped_precheck`,
    /// no agent turn). A stamped permission field: only an approved edit
    /// sets it (`workflow_precheck`). Omitted on the wire when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub precheck: Option<String>,
    /// `notify` frontmatter, `always|failures|never` (Paperclip PR 9): the
    /// per-run notification and skip-banner filter
    /// (`workflow_hygiene::NotifyPolicy`). Absent or unrecognised = always.
    pub notify: Option<String>,
    /// `last_started_at` frontmatter: when the last dispatched run started
    /// (Paperclip PR 9), stamped by `vault_workflows::mark_started` right
    /// before the agent spawns.
    pub last_started_at: Option<String>,
    /// `ends` frontmatter (G2): an RFC 3339 instant after which the scheduler
    /// sets `schedule: manual` and stamps `ended_note`. Omitted when unset.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub ends: Option<String>,
    /// `after_runs` frontmatter (G2): end after this many finished runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub after_runs: Option<u64>,
    /// `runs_counted` frontmatter (G2): runs finished since `after_runs` was
    /// set (written by the run log's finalizer).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub runs_counted: Option<u64>,
    /// `ended_note` frontmatter (G2): why and when the end rule stopped it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub ended_note: Option<String>,
    /// Vault-relative path to the workflow file.
    pub path: String,
    /// Why the permission check would refuse this automation right now
    /// (Paperclip PR 5): `review` (changed outside the automation editor, not
    /// yet approved — or the change was rejected) or `unavailable` (the
    /// permission records can't be read). None when it may run. Filled only by
    /// the `list_workflows` / `read_workflow` commands, from
    /// `workflow_permissions::permission_holds`; omitted on the wire when None.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "\"review\" | \"unavailable\""))]
    pub permission_hold: Option<String>,
    /// The blocking message when a usage budget holds this automation
    /// (Paperclip PR 8a: a hard-stop pause, or a hard-stop budget already at
    /// its limit). Filled only by `list_workflows` / `read_workflow`, from
    /// `budgets::workflow_holds`; omitted on the wire when None. A permission
    /// hold outranks it in the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub budget_hold: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunLogEntry {
    pub at: String,
    pub status: String,
    pub duration_sec: u64,
    pub note: String,
    /// Durable automated-history/evidence run identifier. Old workflow log
    /// rows have no marker and therefore parse to `None`.
    pub run_id: Option<String>,
    /// Last lines of the child process's stderr for a failed run, carried in
    /// a hidden marker on the run-log line. None for successes + legacy rows.
    pub error_tail: Option<String>,
    /// How the run ended beyond its status, from the hidden
    /// `<!-- hollow-exit: … -->` marker (Paperclip PR 8b). Only `budget` is
    /// written: a run a usage budget ended (the guard's kill, or Claude's
    /// spending backstop). It keeps `status: error`; this is what tells it
    /// apart from an ordinary failure (the failure streak and retries skip
    /// it). None for every other row and for legacy rows.
    pub exit_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDetail {
    pub workflow: Workflow,
    /// The prompt body — markdown content of the file below frontmatter,
    /// with the `## Run log` section stripped off.
    pub prompt: String,
    pub run_log: Vec<RunLogEntry>,
    /// The editor baseline (`vault_workflows::edit_digest`, Paperclip PR 9):
    /// a digest of the fields the automation editor owns, as loaded. The
    /// editor sends it back as `WorkflowInput.edit_base`; a file whose owned
    /// fields changed since then refuses the save with
    /// `WORKFLOW_EDIT_CONFLICT: …`. Status and run-log bookkeeping is not in
    /// it, so a run finishing while the editor is open never conflicts.
    pub edit_base: String,
}

/// One file produced by a workflow, surfaced in the outputs view.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOutput {
    /// Vault-relative path to the output file.
    pub path: String,
    /// Display name (file stem).
    pub name: String,
    /// `html` | `md` | other extension.
    pub format: String,
    /// Last-modified time, RFC3339 — newest-first sort + display.
    pub modified: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub schedule: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub output_glob: Option<String>,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    pub prompt: String,
    /// `WorkflowDetail.edit_base` from when the editor loaded the file
    /// (Paperclip PR 9). Some: the save is refused with
    /// `WORKFLOW_EDIT_CONFLICT: …` when the editor-owned fields changed on
    /// disk since. None (the new-automation form, older callers): no check.
    #[serde(default)]
    pub edit_base: Option<String>,
    /// `notify` (Paperclip PR 9, 9a-2): `always` | `failures` | `never`.
    /// None leaves the file's key exactly as it is (the new-automation form
    /// and older callers never send it, and the editor sends it only when the
    /// user changed it). `always` is the default, so it removes the key; any
    /// other value outside the three is refused.
    #[serde(default)]
    pub notify: Option<String>,
    /// `skip_if_quiet` (Paperclip PR 9b). None leaves the file's key as it
    /// is; Some(true) writes the bare boolean, Some(false) removes it. When
    /// the saved workflow skips when quiet, its watch configuration is
    /// validated (`workflow_hygiene::quiet_config`) and a bad one refused.
    #[serde(default)]
    pub skip_if_quiet: Option<bool>,
    /// `watch` globs (Paperclip PR 9b). None leaves the key as it is; an
    /// empty list removes it.
    #[serde(default)]
    pub watch: Option<Vec<String>>,
    /// `precheck` command (OC13). None leaves the file's key as it is; an
    /// empty string removes it. Only the automation editor (a person's
    /// edit) sends it: the agent action `automations.save` strips it
    /// (`app_actions::workflow_input`), and a file edit by anyone else is a
    /// stamped change that needs approval.
    #[serde(default)]
    pub precheck: Option<String>,
    /// Omitted keeps the saved trigger list; Some(empty) removes it.
    #[serde(default)]
    pub triggers: Option<Vec<String>>,
    /// Run inside this chat (overhaul F1): omitted keeps the saved binding;
    /// Some(empty) removes it (a new chat each run); Some(key) binds the
    /// automation to that conversation, which must exist and not be
    /// archived (`automation_in_chat::check_binding`).
    #[serde(default)]
    pub in_chat: Option<String>,
    /// End time (G2): an RFC 3339 instant, or a local `YYYY-MM-DDTHH:MM`.
    /// Omitted keeps the saved one; Some(empty) removes it. Must be in the
    /// future and not combined with `after_runs`.
    #[serde(default)]
    pub ends: Option<String>,
    /// End after this many finished runs (G2). Omitted keeps it; Some(0)
    /// removes it. Not combined with `ends`.
    #[serde(default)]
    pub after_runs: Option<u64>,
    /// Owner Mac (M4): omitted keeps the saved one (a new automation gets
    /// this Mac's slug, when it has one); Some(empty) removes it; `this` or
    /// `this-mac` is this Mac; else a machine slug or id (`runs_on::for_file`).
    #[serde(default)]
    pub runs_on: Option<String>,
}

fn default_permission_mode() -> String {
    "acceptEdits".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunResult {
    pub slug: String,
    pub status: String,
    pub duration_sec: u64,
    /// Last assistant text returned by `claude -p`. Truncated server-side
    /// so the IPC payload stays bounded.
    pub output: String,
    pub error: Option<String>,
    /// Filesystem-backed evidence for this exact run. None when history or
    /// evidence persistence failed; `evidence_error` explains which boundary.
    pub receipt: Option<crate::evidence::EvidenceReceipt>,
    /// History/evidence persistence failure for this run, kept separate from
    /// the workflow process error so the UI can explain a missing receipt.
    pub evidence_error: Option<String>,
}

/// A file the chat agent created/edited, read back for the Chat preview pane.
/// `kind` is the backend's text/image/video/pdf/binary verdict; the frontend layers the
/// render mode (html / markdown / svg / code) on top from the extension.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatPreviewFile {
    /// "text" (utf-8 content in `text`), "image" (`data_url` set), "video"
    /// (size and time only; it plays through `media_grant`), "pdf" (metadata
    /// only; pages use the scoped `pdf_preview_page` command), or "binary"
    /// (no inline preview — open externally).
    pub kind: String,
    pub text: Option<String>,
    pub data_url: Option<String>,
    pub size: u64,
    /// Last-modified, ms since epoch (None when the FS doesn't report it).
    pub modified_ms: Option<u64>,
    /// True when `text` was cut at the read cap.
    pub truncated: bool,
}

// ── Agent sessions (Hollow-owned session identity) ────────────────────────────
//
// `AgentSessionSummary`, `HarnessStint`, `SessionRecordTurn` and
// `AgentSessionDetail` are generated into `src/generated/ipc.ts` (see
// `ts_bindings`); the rest are mirrored in `src/types.ts`. Hollow mints the
// session key; the harness's own id is per-stint metadata and may be absent
// entirely (Antigravity discards it and resumes with `--continue`), which is
// why every id field is optional.

/// One continuous run of a session on one harness. A conversation that moves
/// from Claude to Codex keeps its key and gains a second stint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct HarnessStint {
    pub agent: String,
    /// The CLI's own session/thread id. `None` for Antigravity, and for any
    /// stint whose stream never announced one.
    pub harness_session_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub reason: Option<String>,
}

/// Rail-facing summary of one agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionSummary {
    pub key: String,
    pub title: Option<String>,
    pub agent: String,
    /// Convenience mirror of the newest stint's id — what a resume would use.
    pub harness_session_id: Option<String>,
    pub cwd: String,
    pub created_at: String,
    pub last_activity_at: String,
    pub ended_at: Option<String>,
    pub turn_count: u32,
    pub parent_key: Option<String>,
    pub fork_kind: Option<String>,
    pub adopted: bool,
    pub stints: Vec<HarnessStint>,
    /// `native` | `briefOnly` | `none` — said up front so the UI never offers
    /// a resume that silently lands somewhere else.
    #[cfg_attr(test, ts(as = "crate::ts_bindings::SessionResumability"))]
    pub resumable: String,
    /// Where an imported conversation came from (H5a, `session_import`);
    /// absent for a chat started in Hollow. An imported record is read-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub imported: Option<ImportedProvenance>,
}

/// An imported conversation's provenance (H5a): the Imported chip, its
/// original dates and the Imported filter read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ImportedProvenance {
    /// `claude-code` or `codex`.
    pub source: String,
    /// "Claude Code" or "Codex".
    pub label: String,
    /// `<source>:<session id>`, with `#v<n>` on a later version.
    pub external_id: String,
    /// 1 for the first copy; "Version N" past it.
    pub version: u32,
    pub original_created_at: Option<String>,
    pub original_last_at: Option<String>,
    pub imported_at: String,
    /// When a re-import last updated it in place.
    pub updated_at: Option<String>,
    /// "Updated from Claude Code 3:12 PM, 14 Nov" once updated.
    pub note: Option<String>,
    /// N25: kept out of agent search.
    pub keep_out_of_search: bool,
    /// The session's folder in the source.
    pub source_cwd: Option<String>,
    pub import_id: String,
}

/// One turn inside a session. Ordinals are per-session and 1-based.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SessionRecordTurn {
    pub ord: u32,
    /// Joins to the evidence receipt and the automated-history run.
    pub run_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: Option<String>,
    pub asked: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionDetail {
    pub summary: AgentSessionSummary,
    pub turns: Vec<SessionRecordTurn>,
    /// False once retention has dropped this session's raw events. The session,
    /// stint, and turn rows are kept forever.
    pub events_retained: bool,
}

/// Footprint of the derived, out-of-vault session store.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStoreStatus {
    pub sessions: u32,
    pub events_retained: u32,
    pub bytes: u64,
    pub oldest_retained_iso: Option<String>,
    /// Absolute path, surfaced so the user can see it is not in the vault.
    pub path: String,
}

/// One group of usage-ledger rows in Settings, Usage (Paperclip PR 6).
/// Every run lands in exactly one of `metered` / `partial` / `unknown` /
/// `not_started` / `in_progress`; `unpriced` is the subset of `metered` with
/// tokens but no price. Tokens sum metered and partial rows only. Reported
/// and estimated costs are never mixed, and are `None` (never 0) when no row
/// of that kind exists.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub key: String,
    pub label: String,
    pub runs: u32,
    pub metered: u32,
    pub partial: u32,
    pub unknown: u32,
    pub not_started: u32,
    pub in_progress: u32,
    pub unpriced: u32,
    /// Runs on a self-hosted model (Q8): billed by GPU time, never per
    /// token, so never priced, never in `unpriced` and never $0.
    #[serde(default)]
    pub gpu_time: u32,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reported_cost_usd: Option<f64>,
    pub estimated_cost_usd: Option<f64>,
}

/// Local recording/checkpoint day, using the same rows as the summary. Absent days are not zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageDay {
    pub date: String,
    pub total: UsageBucket,
    pub by_agent: Vec<UsageBucket>,
    /// Direct chat runs linked from night queue/runbook steps, not descendant or manual-lane totals.
    pub night_by_agent: Vec<UsageBucket>,
}

/// Settings, Usage for one window (`usage_summary`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    /// `day` | `week` | `month` | `last30` (Settings PR 8).
    pub window: String,
    /// The agent tab this summary is filtered to (`claude`, `codex`,
    /// `self-hosted`, ...), `None` for every agent. Every list below holds
    /// that agent's rows only, except `agents`.
    #[serde(default)]
    pub agent: Option<String>,
    /// Every agent tab with runs in the period, deliberately NOT filtered by
    /// `agent` (Settings PR 8): Usage draws one tab per agent from it, so the
    /// tab row stays the same whichever tab is open. Every other list holds
    /// the filtered agent's runs only.
    #[serde(default)]
    pub agents: Vec<UsageBucket>,
    /// By the model a run reported, else the one it asked for. Sums to `total`.
    #[serde(default)]
    pub by_model: Vec<UsageBucket>,
    /// By outcome: finished, stopped, failed, moved to another account (a
    /// failed-over turn's earlier attempt) and so on. Sums to `total`.
    #[serde(default)]
    pub outcomes: Vec<UsageBucket>,
    /// The ten costliest conversations of `by_lead`, keyed by session key.
    #[serde(default)]
    pub top_conversations: Vec<UsageBucket>,
    /// The window's start (fixed-width UTC, `…Z`).
    pub since: String,
    pub generated_at: String,
    pub total: UsageBucket,
    pub by_surface: Vec<UsageBucket>,
    /// Workflow runs plus the teammate delegations they made, by slug.
    pub by_workflow: Vec<UsageBucket>,
    pub by_agent: Vec<UsageBucket>,
    /// By `workspace_id`, titled from the workspace list; "none" is "No
    /// project". Sums to `total`.
    #[serde(default)]
    pub by_project: Vec<UsageBucket>,
    /// Chat and delegate rows by the conversation's root (a subagent's usage
    /// rolls up under the conversation that started it).
    #[serde(default)]
    pub by_lead: Vec<UsageBucket>,
    /// Chat and delegate rows of subagent conversations, each its own
    /// bucket: the top 50 by tokens, the rest in one "other" bucket.
    #[serde(default)]
    pub by_subagent: Vec<UsageBucket>,
    /// By the row's hashed account label, or its agent when it has none.
    /// Sums to `total`.
    #[serde(default)]
    pub by_account: Vec<UsageBucket>,
    pub daily: Vec<UsageDay>,
    pub undated_runs: u32,
    pub night_attribution_error: Option<String>,
    /// The oldest row still kept: when recording began.
    pub ledger_since: Option<String>,
    /// The newest recording failure in the window, unless a row was written
    /// after it.
    pub ledger_error: Option<String>,
    /// OB07: the summed `usage_ledger.cost_floor_usd` of the window's runs
    /// with tokens but no cost (killed, interrupted, an unverified resume):
    /// "≥ $X more in N incomplete runs". A lower bound kept apart from the
    /// known costs, never added to them; `None` (never 0) when no run has one.
    #[serde(default)]
    pub partial_floor_usd: Option<f64>,
    /// How many runs that floor covers (N).
    #[serde(default)]
    pub partial_runs: u32,
}

/// The in-flight turn, if any. Tracks the automated-history lease's lifetime,
/// not the child-process handle's — see `agent_sessions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveAgentSession {
    pub key: String,
    pub agent: String,
    pub run_id: String,
    /// `idle` | `queued` | `preparing` | `running` | `pausing` | `paused`
    /// (P1a) | `cancelling` | `failed`
    pub phase: String,
    pub phase_entered_at: String,
    pub failure_reason: Option<String>,
    /// Paperclip PR 15b: RFC 3339 time of the running agent's last stdout or
    /// stderr line (its spawn before the first one). `None` before the spawn
    /// and for a queued turn.
    #[serde(default)]
    pub last_event_at: Option<String>,
    /// Paperclip PR 15b: the progress phase of its latest tool call
    /// (`researching` | `usingTools` | `workingWithFiles` | `coordinating`),
    /// read from the tool name only.
    #[serde(default)]
    pub progress: Option<String>,
}

/// One row of the durable automation queue (`run_queue`, Paperclip PR 7), as
/// the UI sees it. The row's payload (due occurrence, trigger fire facts, a
/// chat turn's composed prompt) is never exposed; `detail` carries only the
/// user's own asked text, for chat rows. Generated into `src/generated/ipc.ts`
/// (see `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RunQueueItem {
    pub id: i64,
    /// `chat` | `cron` | `manual` | `trigger` | `retry` | `heartbeat` | `ingest`
    #[cfg_attr(test, ts(as = "crate::ts_bindings::RunQueueSource"))]
    pub source: String,
    /// The workflow slug for workflow rows, the session key for chat rows.
    pub source_id: Option<String>,
    pub label: String,
    pub detail: Option<String>,
    pub reason: String,
    /// `queued` | `claimed` | `running` | `done` | `failed` | `cancelled` |
    /// `skipped` | `coalesced` | `interrupted`
    #[cfg_attr(test, ts(as = "crate::ts_bindings::RunQueueStatus"))]
    pub status: String,
    pub priority: i64,
    /// 1-based place among queued rows, in claim order; `None` otherwise.
    pub position: Option<u32>,
    pub attempt: u32,
    pub coalesced_count: u32,
    pub requested_at: String,
    pub claimed_at: Option<String>,
    pub finished_at: Option<String>,
    pub history_run_id: Option<String>,
    pub error: Option<String>,
    pub waiting_since: Option<String>,
    /// `LeaseHolder::describe` of the writer this row is waiting for.
    pub waiting_holder: Option<String>,
    /// Why the row is paused rather than waiting (a permission hold): the
    /// run-log note, `PERMISSION_REFUSED_REASON` or
    /// `PERMISSION_RECORDS_UNAVAILABLE_REASON`.
    pub held_reason: Option<String>,
    pub cancel_requested: bool,
    /// Why a queued row hasn't started when neither a lease wait nor a
    /// hold says so: memory pressure, a held build worktree, the
    /// workspace's, a swarm's or the app-wide chat limit (2026-10-20), the
    /// vault lane, a retry time (`run_pump::waiting_reason`, overnight A2).
    /// Set by the queue list and a send's `enqueued` event only.
    #[serde(default)]
    pub waiting_reason: Option<String>,
    /// The conversation whose turn holds the build worktree this row waits
    /// for, when `waiting_reason` names it: the badge's "Open it" (P1-46).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub waiting_session: Option<String>,
    /// Working, not Queued (2026-11-16): the chat row is claimed or running, or a
    /// queued chat row that nothing truly holds back (the pump is about to
    /// claim it, or an internal wait such as a tracker sync write phase
    /// holds it for a moment). Every surface shows it as Working, with
    /// "Preparing" for the part before the agent runs, and shows Queued only
    /// for a row with a real wait (a chat-count limit, memory, another run
    /// holding its folder or the vault lane, a retry time). Set for chat
    /// rows by `run_pump::item_with_reason`; `to_item` alone says it for
    /// claimed and running rows. Absent (false) for workflow rows that wait.
    /// `Some(true)` or absent: never `Some(false)` (ts-rs only marks an
    /// `Option` optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub working: Option<bool>,
}

impl RunQueueItem {
    /// The row reads Working, not Queued ([`RunQueueItem::working`]).
    pub fn reads_working(&self) -> bool {
        self.working == Some(true)
    }
}

/// The `run-queue-event` payload. `kind` is `enqueued` | `coalesced` |
/// `waiting` | `running` | `finished` | `cancelled` | `requeued` | `held`
/// (`run_queue::EVENT_KINDS`). Generated into `src/generated/ipc.ts` (see
/// `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RunQueueEvent {
    #[cfg_attr(test, ts(as = "crate::ts_bindings::RunQueueEventKind"))]
    pub kind: String,
    pub item: RunQueueItem,
}

/// `chat_send`'s reply (Paperclip PR 7b, D3): the turn was ENQUEUED, not
/// started. The pump admits it once the vault lease is free and emits
/// `runStarted` (echoing the request's `clientTurnId`), which can reach the
/// webview before this reply. No history run exists yet, hence `queue_id`
/// rather than a run id. Generated into `src/generated/ipc.ts` (see
/// `ts_bindings`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SendAccepted {
    pub session_key: String,
    pub queue_id: i64,
    /// 1-based place among queued rows, in claim order.
    pub queue_position: u32,
    /// `queued` at accept time (`agent_sessions::SessionPhase::as_str`).
    #[cfg_attr(test, ts(as = "crate::ts_bindings::SessionPhase"))]
    pub phase: String,
    /// `LeaseHolder::describe` (with the live chat detail) of whoever holds
    /// the vault lease the turn waits for; else what else holds it back
    /// (`run_pump::waiting_reason`, overnight A2); `None` when nothing does.
    pub waiting_for: Option<String>,
    /// Mid-turn steering: the send was held for after the conversation's
    /// running turn (Queue for after, Stop and send; `steering::hold`), under
    /// this id, instead of being queued now. `queue_id` is 0 until the pump
    /// sends it (the turn's `done` names it under `next`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub held_id: Option<String>,
    /// One conversation from the person's side (2026-11-07): the send named
    /// this conversation, which had continued elsewhere, so it went to the
    /// chain's current one, `session_key` (`chat_chain::route_send`). Empty:
    /// a new Main agent conversation joined the one still working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub forwarded_from: Option<String>,
    /// The current conversation's title, when it was forwarded there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub forwarded_title: Option<String>,
}

/// One structured question card (Paperclip PR 13), from the `interactions`
/// table in the session store (outside the vault). `status` is `pending`
/// (open, answerable), `stale` (open, its target file changed or vanished),
/// `answered`, `superseded`, `expired` or `dismissed`. `request` is the
/// normalized `interaction_schema::QuestionRequest`, `answer` the validated
/// `interaction_schema::Answer`. The target's absolute path and hash stay
/// internal. Mirrored in `src/types.ts`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChatInteraction {
    pub id: String,
    pub session_key: String,
    pub turn_ord: i64,
    pub run_id: String,
    pub agent: String,
    /// The native session that asked: the answers go there when the
    /// conversation still resumes it, else as an ordinary message.
    pub native_session_id: Option<String>,
    pub tool_use_id: String,
    /// `mcp` or `fenced` (Antigravity).
    pub source: String,
    pub mode: String,
    pub request: serde_json::Value,
    pub target_path: Option<String>,
    pub supersede_on_user_message: bool,
    pub status: String,
    pub status_reason: Option<String>,
    pub superseded_by: Option<String>,
    pub answer: Option<serde_json::Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

/// `resolve_interaction`'s reply: the answered card and the exact prompt the
/// composer sends as the next turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedInteraction {
    pub interaction: ChatInteraction,
    pub prompt: String,
    /// Whether the prompt goes into the native session that asked; false
    /// when the conversation has moved on to another, so it is an ordinary
    /// message (2026-11-04). Either way it names each question in full.
    pub resumes: bool,
}

// ── Saved turn feedback (Paperclip PR 17) ───────────────────────────────────

/// `record_turn_feedback`'s input. The run id is the turn's exact identity
/// (`turns.run_id`); the snapshot is built by the backend from its stores.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFeedbackInput {
    pub run_id: String,
    /// `useful` | `notUseful`.
    pub vote: String,
    pub reason: Option<String>,
}

/// What a vote saw, redacted then capped. Every string comes from a durable
/// store (turn row, transcript, receipt, usage row), never from UI prose.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnFeedbackSnapshot {
    pub asked: String,
    pub answer: String,
    pub answer_truncated: bool,
    pub tools: Vec<String>,
    pub receipt_id: Option<String>,
    pub receipt_status: Option<String>,
    pub verification_state: Option<String>,
    pub recall: Vec<String>,
    /// The turn row's stored status when the vote was saved (`running` there
    /// means the turn was never closed).
    pub turn_status: Option<String>,
}

/// One saved vote (one per turn; a re-vote replaces it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnFeedback {
    pub id: i64,
    pub session_key: String,
    pub session_title: Option<String>,
    pub turn_ord: u32,
    pub run_id: String,
    pub vote: String,
    pub reason: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub snapshot: TurnFeedbackSnapshot,
    pub created_at: String,
    pub updated_at: String,
}

/// `export_feedback`'s reply: the two files written and how many votes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackExport {
    pub json_path: String,
    pub markdown_path: String,
    pub count: u32,
}
