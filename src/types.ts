// Shared types between Rust commands and Solid components.
// Field names match #[serde(rename_all = "camelCase")] on the Rust side.

import type { EvidenceRecallItemWire, EvidenceReceiptWire, EvidenceRepoChangesWire, ReceiptSwitchWire } from './executionReceipt';

import type {
  AgentSessionDetail, AgentSessionSummary, AgentSettings, BlockerWarning, BlockerWarningKind, HarnessStint, HistoryChange,
  HistoryCoverage, HistoryRun, HistoryStorageStatus, HistoryUncapturedReason, Priority, RunFlag,
  RunFlagKind, RunQueueEvent, RunQueueEventKind, RunQueueItem, RunQueueSource, RunQueueStatus, SendAccepted, SessionPhase,
  SessionRecordTurn, SessionResumability, SubTask, Task, TaskBucket, TasksResponse, Workflow,
} from './generated/ipc';
// Generated from the Rust structs (src/generated/ipc.ts, merged plan slice 6);
// re-exported so existing imports keep working. Their field docs live on the
// Rust side (`types::Task`, `types::Workflow`, `agent_settings::AgentSettings`).
// Notes the Rust docs don't carry:
// - `Task.bucket`: `done` is terminal, so a completed task leaves the date
//   groups and the overdue/today counts. `Priority` P4 is lowest/unset.
// - `Task.contentHash`: send it back as `UpdateTaskFields.expected` so a stale
//   edit is refused, not applied.
// - `AgentSettings`: build save payloads with `agentSettingsPayload`
//   (src/agentSettingsPayload.ts) so a save never resets the omitted fields;
//   read the watchdog and deadline maps with `agentWatchdog` / `agentDeadline`.
// - `RunQueueItem` (Rust `types::RunQueueItem`, Paperclip PR 7): the row's
//   payload never crosses IPC. `waitingReason` is phrased to follow "waiting
//   for" and is null except on the list and an `enqueued` event.
//   `RunQueueSource` / `RunQueueStatus` mirror `run_queue::RunSource` /
//   `RowStatus`. `RunQueueEvent` is the `run-queue-event` payload; its
//   `RunQueueEventKind` mirrors `run_queue::EVENT_KINDS`.
// - `AgentSessionSummary` / `HarnessStint` (Rust `types`): see "Agent
//   sessions" below. `SessionResumability`: `native` = a harness id we can
//   resume; `briefOnly` = Antigravity, or a session whose id we never saw;
//   `none` = the transcript is gone. `AgentSessionDetail` (`read_agent_session`)
//   adds the turns (`SessionRecordTurn`, ordinals 1-based); `eventsRetained`
//   turns false once retention dropped the raw events, but the session, stint
//   and turn rows are kept forever.
// - `SendAccepted` (`chat_send`, Paperclip PR 7b): the turn is ENQUEUED, not
//   started, so it carries `queueId`, not a run id. The pump admits it when
//   the vault lease is free, and its `runStarted` event (echoing the
//   request's `clientTurnId`) can arrive before this reply resolves. `phase`
//   is `queued` at accept time; `waitingFor` names whoever holds the turn
//   back, else null. `SessionPhase` mirrors `agent_sessions::SessionPhase`:
//   where a live turn is, tracking the automated-history lease's lifetime,
//   NOT the child-process handle's.
export type {
  AgentSessionDetail, AgentSessionSummary, AgentSettings, BlockerWarning, BlockerWarningKind, HarnessStint, HistoryChange,
  HistoryCoverage, HistoryRun, HistoryStorageStatus, HistoryUncapturedReason, Priority, RunFlag,
  RunFlagKind, RunQueueEvent, RunQueueEventKind, RunQueueItem, RunQueueSource, RunQueueStatus, SendAccepted, SessionPhase,
  SessionRecordTurn, SessionResumability, SubTask, Task, TaskBucket, TasksResponse, Workflow,
};

export interface Project {
  url: string;
  name: string;
  status: string;
  description: string;
  created: string | null;
  vaultPath: string | null;
  priority: Priority | null;
  targetDate: string | null;
  /** RFC3339 ship stamp — set when the project entered the deployed status. */
  deployedAt: string | null;
  /** Optional outcome contract stored in project frontmatter. */
  currentState: string | null;
  desiredState: string | null;
  nonGoals: string | null;
  successEvidence: string | null;
  /** Stable vault URLs of the goals this project serves. */
  goalUrls?: string[];
}

/** Outcome-contract fields accepted by both project create and edit flows. */
export interface ProjectOutcomeFields {
  currentState?: string;
  desiredState?: string;
  nonGoals?: string;
  successEvidence?: string;
}

export interface CreateProjectFields extends ProjectOutcomeFields {
  title: string;
  status?: string;
  priority?: string;
  description?: string;
  targetDate?: string;
  goalUrls?: string[];
}

export interface UpdateProjectFields extends ProjectOutcomeFields {
  url: string;
  title?: string;
  status?: string;
  priority?: string;
  description?: string;
  targetDate?: string;
  /** Omit to preserve links; pass [] to clear them. */
  goalUrls?: string[];
}

export type GoalStatus = 'active' | 'paused' | 'achieved' | 'dropped';

export interface Goal {
  url: string;
  title: string;
  status: GoalStatus;
  horizon: string | null;
  targetDate: string | null;
  why: string | null;
  successEvidence: string | null;
  created: string | null;
  vaultPath: string;
}

export interface CreateGoalInput {
  title: string;
  status?: GoalStatus;
  horizon?: string;
  targetDate?: string;
  why?: string;
  successEvidence?: string;
}

/** Patch input: omitted leaves unchanged; an empty string clears a scalar. */
export interface UpdateGoalInput {
  title?: string;
  status?: GoalStatus;
  horizon?: string;
  targetDate?: string;
  why?: string;
  successEvidence?: string;
}

export interface GoalAlignment {
  activeGoalCount: number;
  activeProjectCount: number;
  healthyGoalCount: number;
  healthyProjectCount: number;
  activeGoalsWithoutActiveProjects: Array<{
    goalUrl: string;
    goalTitle: string;
  }>;
  activeProjectsWithoutGoals: Array<{
    projectUrl: string;
    projectName: string;
  }>;
  brokenGoalUrls: Array<{
    projectUrl: string;
    projectName: string;
    goalUrl: string;
    reason: 'invalidGoalUrl' | 'goalNotFound';
  }>;
  projectsOnlyLinkedToInactiveGoals: Array<{
    projectUrl: string;
    projectName: string;
    inactiveGoalUrls: string[];
  }>;
}

export interface ProjectNextMoveCandidate {
  title: string;
  basedOn: string;
  priority: Priority | null;
  due: string | null;
}

export interface ProjectNextMove {
  kind: 'clarifyOutcome' | 'existingTask' | 'newTask' | string;
  title: string;
  basedOn: string;
  priority: Priority | null;
  due: string | null;
  taskUrl: string | null;
  outcomeField: 'currentState' | 'desiredState' | 'nonGoals' | 'successEvidence' | string | null;
  candidates: ProjectNextMoveCandidate[];
}

// `HistoryRun`, `HistoryChange`, `HistoryCoverage` (Rust
// `automated_history::Coverage`), `HistoryUncapturedReason` (Rust
// `UncapturedReason`), `RunFlag` / `RunFlagKind` and `HistoryStorageStatus` are
// generated; see the top of this file. Notes the Rust docs don't carry:
// - `HistoryChange.beforeCoverage` / `afterCoverage` are absent on records
//   from before merged plan slice 4: read a side through `sideCoverage` in
//   `historyRun.ts`, which derives it. Only `absent` and `captured` can be
//   restored; an `uncaptured` side's `reason` is null only on slice 1 records.
// - `RunFlag`: only `workflow` changes are refused; the rest are flag-only and
//   reverted through History & recovery (`workflowSettings`, an automation's
//   unstamped notify/quiet settings, in the automation editor).
// - `HistoryRun.scope` is a workspace conversation's vault folder
//   (`workspaces/<id>`; its repository changes are in the receipt), absent for
//   full-vault runs; `flags`, `overlaps` and `alsoChangedBy` are absent when
//   empty.

/** A history run's repository-changes sidecar (Paperclip PR 16,
 *  `read_history_repo_changes`; Rust `repo_diff::RepoSidecar`): the run's
 *  repo rows with bounded, redacted patches. Display-only. */
export interface RepoSidecar {
  schemaVersion: number;
  runId: string;
  capturedAt: string;
  repos: EvidenceRepoChangesWire[];
}

export interface LearningProposalPayload {
  kind: string;
  targetKind: string;
  targetPath: string;
  subject: string;
  statement: string;
  rationale: string;
  evidenceExcerpt: string;
  sourceRunId: string | null;
  /** A bulk Reject's optional reason (at most 500 characters). */
  rejectReason?: string;
  /** The bulk review that resolved it (its Undo token). */
  batchId?: string;
  /** The approved note an accept wrote. */
  learnedPath?: string;
  learnedLine?: string;
  alreadyIn?: string;
  /** Memory maintenance: the saved line this proposal takes out or rewords. */
  retires?: { proposal: string; path: string; line: string; cycle: string };
  /** One home per fact (PX5): the saved learning note Save retires (a link to its me/ line). */
  retiresNote?: { path: string; sha256: string };
  possibleDuplicate?: string;
  editsPath?: string;
  editsHash?: string;
}

export interface LearningProposal {
  id: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  fingerprint: string;
  payload: LearningProposalPayload;
  error: string | null;
  /** The version this was read at (ME1); a Save sends it back. */
  version: string;
}

export type ReviewItemKind =
  | 'learning'
  | 'commitment'
  | 'project_activation'
  | 'goal'
  | 'heartbeat_send'
  | 'heartbeat_task'
  | 'reply_draft'
  | 'meeting_task'
  | 'meeting_followup'
  | 'workflow_permissions'
  | 'workflow_failure_streak'
  | 'budget_pause'
  | 'agent_task'
  | 'gardener'
  | 'record'
  | 'person_alias'
  | 'companion_conflict'
  | 'companion_old_op'
  | 'memory_recheck'
  | 'eval_candidate';

export type ReviewItemStatus = 'pending' | 'applying' | 'accepted' | 'rejected' | 'snoozed';
export type ReviewItemRisk = 'local_write' | 'external_send';
export type ReviewItemSource = 'activation' | 'heartbeat' | 'inbox' | 'meeting' | 'chat' | 'agent' | 'workflow' | 'budget' | 'gardener' | 'companion' | 'memory';

/** One entry of a workflow permission review's structured diff. */
export type WorkflowPermissionDiffEntry =
  | { field: string; kind: 'scalar'; before: string | null; after: string | null }
  | { field: string; kind: 'list'; added: string[]; removed: string[]; reordered: boolean }
  | { field: 'prompt'; kind: 'text'; before: string | null; after: string; truncated: boolean };

/** Payload of a `workflow_permissions` review (Rust
 *  `workflow_permissions::review_payload`). Deterministic; recomputed from disk
 *  on approval, so an edited card can never be approved. */
export interface WorkflowPermissionReviewPayload {
  slug: string;
  title: string;
  path: string;
  observedDigest: string;
  approvedDigest: string | null;
  changedFields: string[];
  escalation: boolean;
  summary: string;
  diff: WorkflowPermissionDiffEntry[];
}

/** One persistent, auditable approval. The payload remains kind-specific. */
export interface ReviewInboxItem {
  id: string;
  kind: ReviewItemKind;
  status: ReviewItemStatus;
  createdAt: string;
  updatedAt: string;
  fingerprint: string;
  payload: Record<string, unknown>;
  risk: ReviewItemRisk;
  source: ReviewItemSource;
  sourcePath: string | null;
  snoozedUntil: string | null;
  error: string | null;
  /** The version this was read at (`list_review_inbox`, ME1); a learning's
   *  Save sends it back so an edit of words that changed meanwhile is refused. */
  version?: string;
}

export interface ProjectOutcomeGap {
  projectUrl: string;
  projectName: string;
  missingFields: Array<'currentState' | 'desiredState' | 'nonGoals' | 'successEvidence'>;
}

export interface ActivationInventory {
  activeProjectCount: number;
  planningProjectCount: number;
  projectOutcomeGaps: ProjectOutcomeGap[];
  alignment: GoalAlignment;
  sourceCounts: { email: number; chat: number; meeting: number; recording: number };
  commitmentCount: number;
  learningCount: number;
  pendingActivationProposals: number;
  dismissedActivationFingerprints: number;
}

export interface ActivationScanResult {
  inventory: ActivationInventory;
  added: { projectActivation: number; goal: number; commitment: number; learning: number };
  warnings: string[];
}

export interface LearningContext {
  path: string;
  title: string;
  targetKind: string;
  targetPath: string;
  subject: string;
  statement: string;
  sourceRunId: string | null;
  learnedAt: string;
  supersedes: string | null;
  supersededBy: string | null;
}

export interface MemoryMaintenanceEntry {
  /** What a click names: the note path, or `line:<proposal id>` for a saved You or Project line. */
  key: string;
  kind: 'note' | 'line';
  line: string | null;
  /** Whether quiet aging applies (profile rules never age). */
  ages: boolean;
  /** The waiting proposal to take out or reword this line. */
  proposalId: string | null;
  path: string;
  /** `changed`: a saved line edited by hand since; Hollow leaves it alone. */
  status: 'active' | 'stale' | 'archived' | 'expired' | 'changed';
  pinned: boolean;
  uses: number;
  lastUsed: string | null;
  expiresOn: string | null;
  expectedHash: string;
}
export interface MemoryMaintenance {
  entries: MemoryMaintenanceEntry[];
  coreBytes: number;
  coreBudgetBytes: number;
  consolidationProposal: string | null;
}

export interface LearningRecallResult {
  items: Array<{ path: string; targetPath: string; subject: string; statement: string; score: number }>;
  promptBlock: string;
}

export type CommitmentDirection = 'owed_by_me' | 'owed_to_me';
export type CommitmentStatus = 'open' | 'fulfilled' | 'cancelled';
export type CommitmentChannel = 'email' | 'teams' | 'slack' | 'meeting' | 'manual';

export interface Commitment {
  url: string;
  vaultPath: string;
  title: string;
  direction: CommitmentDirection;
  counterparty: string;
  personPath: string | null;
  projectUrl: string | null;
  due: string | null;
  status: CommitmentStatus;
  channel: CommitmentChannel;
  sourcePath: string | null;
  created: string;
  fulfilledAt: string | null;
  fingerprint: string;
  context: string;
  /** `YYYY-MM-DD`: snoozed (open) until that day (overnight 10-11 A5). */
  snoozedUntil?: string | null;
  /** The words it rests on, quoted from its source. */
  evidence?: string | null;
}

export interface CreateCommitmentInput {
  title: string;
  direction: CommitmentDirection;
  counterparty: string;
  personPath?: string;
  projectUrl?: string;
  due?: string;
  status?: CommitmentStatus;
  channel?: CommitmentChannel;
  sourcePath?: string;
  context?: string;
}

export interface UpdateCommitmentInput {
  url: string;
  title?: string;
  direction?: CommitmentDirection;
  counterparty?: string;
  personPath?: string;
  projectUrl?: string;
  due?: string;
  status?: CommitmentStatus;
  channel?: CommitmentChannel;
  sourcePath?: string;
  context?: string;
  /** `YYYY-MM-DD` snoozes an open one; `''` wakes it. */
  snoozedUntil?: string;
  /** Restores a fulfilled commitment's original stamp (the ledger's undo of a reopen); used only when the status ends fulfilled. */
  fulfilledAt?: string;
}

export interface TrashedCommitment { url: string; content: string; }

export interface CommitmentProposalPayload extends CreateCommitmentInput {
  channel: CommitmentChannel;
  rationale: string;
  evidenceExcerpt: string;
  sourceRunId?: string;
}

export interface CommitmentProposal {
  id: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  fingerprint: string;
  payload: CommitmentProposalPayload;
  error: string | null;
}

/** The vault source: only `obsidian` since Notion sync was removed (11-10). */
export type WorkspaceSource = 'obsidian';

export interface WorkspaceSyncSummary {
  source: WorkspaceSource | string;
  scanned: number;
  created: number;
  updatedRemote: number;
  updatedLocal: number;
  deletedRemote: number;
  deletedLocal: number;
  unchanged: number;
  conflicts: number;
  skipped: number;
  errors: string[];
}

/** Structured result of natural-language quick-add parsing. */
export interface ParsedQuickAdd {
  title: string;
  priority: Priority | null;
  due: string | null;
  dueTime: string | null;
  durationMin: number | null;
  recurrence: string | null;
  project: string | null;
  labels: string[];
}

/** Create-task payload (mirrors Rust `CreateTaskInput`). */
export interface CreateTaskFields {
  title: string;
  due?: string;
  dueTime?: string;
  durationMin?: number;
  priority?: string;
  status?: string;
  description?: string;
  recurrence?: string;
  section?: string;
  labels?: string[];
  project?: string;
  projectUrl?: string;
  /** Idempotency key (at most 16 KiB). A create whose key an existing task
   *  carries returns that task (`CreatedTask.existing`) instead of a twin. */
  idempotencyKey?: string;
  /** Provenance stored as `origin:` (one line, 200 characters). */
  origin?: string;
  /** `vault:tasks/…` blockers; each must name an existing task and must not
   *  close a loop (`TASK_BLOCKER: …` / `TASK_CYCLE: …`). */
  blockedBy?: string[];
}

/** Full task-edit payload (mirrors Rust `UpdateTaskInput`). Omit a field to
 *  leave it unchanged; an empty string / empty array clears it. */
export interface UpdateTaskFields {
  url: string;
  /** The `Task.contentHash` the editor loaded. When set, the write is
   *  compare-and-swap: a changed file rejects with `TASK_CONFLICT: …`. */
  expected?: string;
  title?: string;
  status?: string;
  priority?: string;
  due?: string;
  dueTime?: string;
  durationMin?: number;
  recurrence?: string;
  section?: string;
  labels?: string[];
  description?: string;
  subtasks?: SubTask[];
  project?: string;
  /** Replaces the blocker list (an empty list clears it). Only new entries
   *  are validated (`TASK_BLOCKER: …`, `TASK_CYCLE: …`). */
  blockedBy?: string[];
}

/** `update_task` result: the task URL and the hash of the bytes just written. */
export interface UpdatedTask {
  url: string;
  contentHash: string;
}

/** AI assist request for a task/project. `action` is 'breakdown' | 'next_steps' | 'rephrase'. */
export interface AssistInput {
  action: string;
  title: string;
  description?: string;
  /** The task's stored project URL (linked tasks only). The backend adds the
   *  project's goals as reference evidence to breakdown and next_steps. */
  projectUrl?: string;
}

/** AI assist result for a task/project. */
export interface AssistResult {
  suggestions: string[];
  text: string | null;
}

export interface SessionMeta {
  project: string;
  projectDir: string;
  sessionId: string;
  /** Short AI-generated title (cached on disk); null until generated. */
  title: string | null;
  firstPrompt: string;
  messageCount: number;
  lastActivity: string;
  sizeBytes: number;
  cwd: string | null;
  gitBranch: string | null;
}

export interface SessionTurn {
  role: 'user' | 'assistant';
  text: string;
  timestamp: string | null;
  toolUses: {
    id: string | null;
    name: string;
    input: unknown;
    result: string | null;
    isError: boolean | null;
  }[];
}

export interface SessionDetail {
  project: string;
  sessionId: string;
  turns: SessionTurn[];
}

export interface Note {
  path: string;
  name: string;
  mtime: string;
  size: number;
}

export interface NoteContent {
  path: string;
  content: string;
  mtime: string;
  size: number;
  /// sha256 hex of `content` (Rust `vault::revision`); pass it back as
  /// `vault_write_note`'s `expectedRevision`. Optional so mocks may omit it.
  revision?: string;
}

/// Outcome of the editor's compare-and-swap save. `conflict: false` ⇒ written
/// (`note` = what was saved); `conflict: true` ⇒ the file changed underneath
/// (`note` = the current on-disk version, for reload / keep-mine).
export interface SaveNoteResult {
  conflict: boolean;
  note: NoteContent;
}

/// Payload of the `vault-changed` Tauri event (mirrors `vault_watch::VaultChange`).
/// Emitted (debounced) by the backend FS watcher whenever vault files are
/// created/modified/removed — by the app, by Obsidian, by the embedded Claude,
/// or by any editor. `folders` is the distinct set of top-level folders touched
/// (drives which resources the UI refetches); `paths` is the vault-relative
/// file paths (capped), used to decide whether the open note should live-reload.
export interface VaultChange {
  folders: string[];
  paths: string[];
}

export type VaultTree = { [name: string]: VaultNode };
export interface VaultFolderNode { kind: 'folder'; children: VaultTree; }
export interface VaultFileNode extends Note { kind: 'file'; }
export type VaultNode = VaultFolderNode | VaultFileNode;

export interface BacklinkSource {
  path: string;
  basename: string;
  matches: { line: number; text: string }[];
}

export interface NoteRef {
  path: string;
  basename: string;
  /** Frontmatter `title` or first `# heading`, when it says more than the file name. */
  title?: string;
  /** First sentence of the note's prose. */
  summary?: string;
}

/** One daily brief from the Morning Brief workflow. `format` decides the
 *  renderer: `'html'` → sandboxed iframe (HtmlDoc), `'md'` → NoteRenderer. */
export interface Dashboard {
  sessions: SessionMeta[];
  sessionsTotal: number;
  recentNotes: Note[];
  tasks: Task[];
  tasksTotal: number;
  overdueCount: number;
  todayCount: number;
  activeProjects: Project[];
  projectsTotal: number;
  tasksFetched: string | null;
  projectsFetched: string | null;
  timestamp: string;
}

/** One calendar day in the Today git-contribution heatmap. */
export interface GitDay {
  /** YYYY-MM-DD, local timezone. */
  date: string;
  count: number;
}

/** Git contribution heatmap — dense day array over the trailing ~53 weeks
 *  plus roll-up stats. Mirrors Rust `GitActivity`. */
export interface GitActivity {
  /** One entry per day from `start`..`end`, in order (idle days have count 0). */
  days: GitDay[];
  totalCommits: number;
  activeDays: number;
  currentStreak: number;
  longestStreak: number;
  reposScanned: number;
  start: string;
  end: string;
}

export interface GraphNode {
  /** Vault-relative path — stable id used by edges. */
  id: string;
  label: string;
  /** Top-level folder ("" for root-level notes). UI color-codes by this. */
  folder: string;
  /** Semantic kind (folder-derived) — lets the graph filter out the
   *  high-volume synced mirrors. Mirrors `vault_index::kind_from_folder`. */
  kind: VaultKind;
  inDegree: number;
  outDegree: number;
}
export interface GraphEdge { source: string; target: string }
export interface VaultGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface Diagnostics {
  vaultPath: string;
  vaultExists: boolean;
  configPath: string;
  claudeBinary: string | null;
  debugLogDir: string;
  debugLogEnabled: boolean;
  debugLogFiles: number;
  /** The personal context's health (PX7); null while no personal clone is set. */
  contextHealth?: ContextHealth | null;
}

/** Mirrors `context_layers::ContextHealth`. */
export interface ContextHealth {
  meCompared: boolean;
  meDiffers: string[];
  homeChecked: string | null;
  homeAgeDays: number | null;
  /** The home file's staleness label (`context_stale::home_staleness`), the
   *  rule recall uses; `null` when fresh or undated. */
  homeStale: string | null;
  memoryFiles: number;
  memoryDated: number;
  memoryExpired: string[];
  memoryUnverified: string[];
}

// ── Setup / onboarding (mirrors setup.rs) ────────────────────────────────

export interface SetupStatus {
  configured: boolean;
  configPath: string | null;
  configError: string | null;
  vaultPath: string | null;
  vaultExists: boolean;
}

export interface ProfileNotesResult {
  created: number;
  skipped: number;
  errors: string[];
}

export interface WorkspaceSourceUpdate {
  source: WorkspaceSource;
}

/// Mirrors `themes::CustomTheme` on the Rust side. `colors` is a
/// 16-slot ANSI palette; missing slots come through as `null` and the
/// frontend derivation falls back to sensible defaults.
export interface CustomTheme {
  name: string;
  source: string;
  isDark: boolean;
  accent: string;
  cursor: string | null;
  foreground: string;
  background: string;
  selectionForeground: string | null;
  selectionBackground: string | null;
  colors: (string | null)[];
  success: string | null;
  warning: string | null;
  error: string | null;
}

/// Lightweight directory-listing entry — enough to render a picker row
/// without reading every theme file.
export interface CustomThemeMeta {
  name: string;
  path: string;
  isDark: boolean;
  accent: string;
  background: string;
  foreground: string;
}

export interface CreatedTask {
  url: string;
  id: string;
  /** True when an earlier create with the same key already made this task. */
  existing: boolean;
}

export interface SearchResult extends Note {
  snippet: string;
  score: number;
  /** Semantic kind from the vault index (groups + routes results). */
  kind: VaultKind;
}

/** Status of the local semantic-search index (Settings row). Mirrors the Rust
 *  `SemanticStatus`: enabled + backfill progress (embedded / total chunks). */
export interface SemanticStatus {
  enabled: boolean;
  indexed: number;
  total: number;
}

/** The project content index (repo features R14b; Rust
 *  `project_text_index::Status`): Settings, Advanced, Search & indexes's switch and
 *  progress, and Library's "Contents indexed" line. */
export interface ConnectedIndexStatus {
  enabled: boolean;
  /** Listed files whose current text is in the index. */
  indexed: number;
  /** Listed text, PDF and Office files the index is for. */
  total: number;
  /** Read and failed (damaged, or crashed while parsing). */
  failed: number;
  /** PDFs with no text layer. */
  imageOnly: number;
  /** Left out on purpose: restricted folders, secret names. */
  skipped: number;
}

/** The semantic kind vocabulary. Mirrors the `KIND_*` constants in
 *  `src-tauri/src/vault_index.rs`. `src/vaultKinds.ts` owns the per-kind UI
 *  metadata (label, icon, lens membership). */
export type VaultKind =
  | 'task' | 'project' | 'meeting' | 'recording' | 'email' | 'chat'
  | 'workflow' | 'briefing' | 'person' | 'domain' | 'daily' | 'weekly'
  | 'goal' | 'commitment' | 'learning'
  | 'note' | 'source' | 'operator' | 'capture' | 'unfiled';

/** Provenance of a vault item — drives the read-only / trust affordances. */
export type VaultTrust = 'authored' | 'synced' | 'generated' | 'imported';

/** Canonical surface an item is best opened on. The Vault tab routes on this:
 *  `tasks`/`projects` open the modal editors, `inbox` jumps to the Inbox
 *  reader, `vault` opens in the Vault editor/reader. Mirrors `surface_for`. */
export type VaultSurface = 'tasks' | 'projects' | 'inbox' | 'vault';

/** One classified vault file from the semantic index (`vault_items`).
 *  Mirrors Rust `vault_index::VaultItem` (serde camelCase). */
export interface VaultItem {
  path: string;
  title: string;
  kind: VaultKind;
  surface: VaultSurface;
  /** Top-level folder ("" for a vault-root file). */
  folder: string;
  editable: boolean;
  /** Hidden from the default browse lens (the high-volume synced mirrors). */
  hidden: boolean;
  trust: VaultTrust;
  mtime: string;
  size: number;
  /** One-line body excerpt for the list preview (may be empty). */
  excerpt: string;
  /** Cleanup reason when this is a drift candidate; null when healthy. */
  drift: string | null;
}

export type BackupHealthStatus = 'protected' | 'partial' | 'unknown' | 'unavailable';

export interface BackupSignal {
  id: string;
  label: string;
  state: 'confirmed' | 'partial' | 'missing' | 'unavailable' | string;
  detail: string;
}

export interface BackupHealth {
  status: BackupHealthStatus;
  summary: string;
  signals: BackupSignal[];
}

/** One pending inline chat approval — emitted by the Rust approval broker
 *  (`approvals.rs`) as an `approval-request` event when a confirm-gated MCP
 *  tool (M365 send/reply/delete, computer-control tier 2) wants permission.
 *  Resolved via `approval_respond`; an `approval-resolved` event clears it. */
export interface ApprovalRequest {
  id: string;
  /** The surface asking, e.g. "Microsoft 365" / "Computer control". */
  title: string;
  /** Human-readable summary of what will happen (multi-line). */
  action: string;
  /** Request tag for the UI: 'm365' | 'computer' | 'generic'. */
  kind: string;
  /** Paperclip PR 21: the bare MCP tool name that asked (`quit_app`), or
   *  null from a gate that names none. */
  tool?: string | null;
  /** The agent tool call this request was bound to (same tool, same
   *  arguments), or null when the broker could not bind it. Never guessed. */
  toolUseId?: string | null;
  /** The Hollow conversation that tool call belongs to, when bound. */
  sessionKey?: string | null;
  /** SA1: set when that conversation is a subagent: its own key, the
   *  conversation that started it, and its title. The card shows in the
   *  parent's chat too, naming it. */
  subagent?: { key: string; parentKey: string; title?: string | null } | null;
  /** 2026-11-20: set when the card is raised to the COO chat, the one place
   *  it is answered: it comes from a project lead's chain or a subagent of
   *  one. The asking chat shows only that it waits there. */
  routedTo?: { askedKey: string; askedTitle?: string | null; askedKind: 'lead' | 'subagent'; cooKey: string } | null;
}

/** How an approval request ended (Paperclip PR 21). */
export type ApprovalReason = 'allowed' | 'denied' | 'timed_out' | 'cancelled';

/** The `approval-resolved` event. `approved` / `cancelled` keep their
 *  pre-PR-21 meaning; `reason` is absent from an older backend. */
export interface ApprovalResolved {
  id: string;
  approved: boolean;
  cancelled: boolean;
  reason?: ApprovalReason;
  toolUseId?: string | null;
  sessionKey?: string | null;
  tool?: string | null;
  /** B4: set when an iPhone answered (absent or null for this window's click). */
  answeredBy?: { kind: 'device'; deviceId: string; deviceName: string; faceId: boolean; at: string; receipt: string } | null;
}

export interface Skill {
  name: string;
  source: string;        // "user" | "plugin:<name>" | "marketplace:<market>/<plugin>"
  sourceKind: 'user' | 'plugin' | 'marketplace';
  plugin: string | null;
  path: string;          // absolute path to SKILL.md
  description: string;
  version: string | null;
  writable: boolean;
  /** Static health check (NV1); only the shared library's skills carry one. */
  lint?: SkillLint | null;
}

/** One lint finding (Rust `skill_lint::LintFinding`). */
export interface SkillLintFinding {
  rule: string;
  area: 'schema' | 'description' | 'size' | 'safety';
  deduction: number;
  message: string;
  suggestion: string | null;
}

/** A skill's lint (Rust `skill_lint::SkillLint`): 100 minus fixed capped deductions. */
export interface SkillLint {
  score: number;
  grade: 'good' | 'fair' | 'weak';
  findings: SkillLintFinding[];
}

/** What a skill does for an agent (Rust `skill_portability::Effect`). */
export type PortabilityEffect = 'loads' | 'unknown' | 'degraded' | 'noLoad';
/** How a rule's evidence stands against the installed CLI (Rust `VersionStatus`). */
export type PortabilityVersionStatus = 'hollow' | 'verified' | 'notVerifiedOnThisVersion' | 'installedUnknown' | 'unverified';

/** One rule of an agent's profile (Rust `skill_portability::RuleRow`). */
export interface PortabilityRule {
  id: string;
  /** `hollow`: Hollow's own delivery. `native`: the CLI's own skill loader. */
  channel: 'hollow' | 'native';
  effect: PortabilityEffect;
  what: string;
  sentence: string;
  /** The CLI version the rule was checked on; null for Hollow's own code or unverified. */
  verifiedOn: string | null;
  unverifiedReason: string | null;
  status: PortabilityVersionStatus;
}

/** A rule that hit the assessed skill (Rust `skill_portability::Finding`). */
export interface PortabilityFinding {
  rule: string;
  channel: 'hollow' | 'native';
  effect: PortabilityEffect;
  sentence: string;
  verifiedOn: string | null;
  status: PortabilityVersionStatus;
}

/** One agent's row (Rust `skill_portability::AgentRow`). */
export interface PortabilityAgentRow {
  agent: string;
  label: string;
  installed: string | null;
  rules: PortabilityRule[];
  /** Present only when a skill was assessed. */
  summary: PortabilityEffect | null;
  findings: PortabilityFinding[];
  /** Some rule was checked on another CLI version than the installed one. */
  stale: boolean;
}

/** The skill portability table (Rust `skill_portability::PortabilityTable`). */
export interface PortabilityTable {
  agents: PortabilityAgentRow[];
  skill: string | null;
}

/** Where a skill import comes from (Rust `skill_import::ImportSource`). */
export type SkillImportSource =
  | { kind: 'path'; path: string }
  | { kind: 'bytes'; name: string; data: string }
  | { kind: 'files'; files: { path: string; data: string }[] };

/** What happens when ~/.agents/skills/<name> already exists. */
export type SkillImportClash = 'cancel' | 'replace' | 'keep-both';

/** A staged import, before Install (Rust `skill_import::ImportPreview`). */
export interface SkillImportPreview {
  token: string;
  name: string;
  description: string;
  files: { path: string; size: number; script: boolean }[];
  totalBytes: number;
  hasScripts: boolean;
  skipped: string[];
  folder: string | null;
  existing: string | null;
  canReplace: boolean;
  keepBothName: string;
  /** SD2: the name SKILL.md came with, when the import was renamed because
   *  that name was illegal, and the files whose references to it were rewritten. */
  renamedFrom?: string;
  rewritten?: { path: string; count: number }[];
  /** Check for update (repo ideas 10-27 #2): how this staged copy differs
   *  from the installed one, which added or changed files are scripts, and
   *  their text (`skill_provenance::FileDiff`, 64 KB in all). */
  update?: { changes: SkillFileChanges; scriptsChanged: string[]; diffs: SkillFileDiff[] };
}

/** One changed or added file's text on the update card: `- `/`+ ` lines
 *  around the change, or an added file's opening lines; `listedOnly` for a
 *  binary or oversized file. */
export interface SkillFileDiff { path: string; script: boolean; text: string; truncated: boolean; listedOnly: boolean }

/** Files added, removed or changed (`skill_provenance::Changes`). */
export interface SkillFileChanges { added: string[]; removed: string[]; modified: string[] }

/** A shared skill's import record (`skill_provenance::View`): none means
 *  "Not imported by Hollow". */
export interface SkillProvenance {
  name: string;
  record: {
    from: { kind: 'path'; path: string } | { kind: 'dropped'; name: string };
    origin?: { url: string; commit?: string; subdir: string } | null;
    files: Record<string, string>;
    importedAt: string;
  } | null;
  /** What differs from the record now (null without one). */
  changed: SkillFileChanges | null;
  error: string | null;
  /** Check for update has an https origin or a path to read. */
  canCheck: boolean;
  note: string | null;
}

/** What Install did (Rust `skill_import::ImportOutcome`). */
export interface SkillImportOutcome {
  skill: Skill;
  files: number;
  hasScripts: boolean;
  replaced: boolean;
  renamedFrom: string | null;
}

// ── Ingestion ────────────────────────────────────────────────────────────
//
// One ingestion run drives Claude Code through reading a source (PDF /
// image / YouTube / generic URL / inline text) and writing it into
// `vault/ingested/{slug}/` as one note by default — or several linked
// notes when the source clearly holds multiple separable ideas (Claude
// decides), with `[[wikilinks]]` between siblings.

export type IngestSource =
  | { kind: 'file'; path: string; displayName?: string }
  | { kind: 'url'; url: string }
  | { kind: 'text'; content: string; title: string };

export interface IngestOpts {
  /** Cognitive anchor — domain through which to read the source. */
  domain: string;
  /** Why the user is ingesting — shapes which concepts surface. */
  purpose: string;
}

export interface IngestResult {
  targetDir: string;
  createdCount: number;
  summary: string;
}

// ── Workflows ────────────────────────────────────────────────────────────
//
// A workflow is a Claude-driven automation stored in
// `vault/workflows/{slug}.md`. The frontmatter holds schedule + run
// metadata; the body is the prompt template. `schedule` is either the
// literal string `manual` or a 7-field cron expression (the form the
// Rust `cron` crate consumes — seconds, minutes, hours, day-of-month,
// month, day-of-week, year).

export type WorkflowStatus = 'success' | 'error' | 'running' | 'cancelled' | string;

// `Workflow` is generated (see the top of this file). `lastStatus` is a
// `WorkflowStatus`; `permissionHold` (absent when it may run) and `budgetHold`
// are omitted from the wire, never null.

// ── Usage budgets (Paperclip PR 8a; Rust `budgets.rs`) ─────────────────────

export type BudgetMetric = 'tokens' | 'costUsd';
export type BudgetWindow = 'day' | 'week' | 'month';
export type BudgetState = 'ok' | 'warning' | 'over' | 'paused' | 'unknown';
export type BudgetEnforceability = 'ok' | 'partial' | 'none';

export interface BudgetPause {
  windowKey: string;
  windowEnd: string;
  episode: string;
  pausedAt: string;
  observed: number;
  limit: number;
  /** "Keep paused": survives the window reset until Resume. */
  held: boolean;
  reviewItemId: string | null;
}

export interface BudgetStatus {
  id: string;
  scope: string;
  scopeLabel: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  limit: number;
  warnPercent: number;
  hardStop: boolean;
  windowKey: string;
  windowStart: string;
  windowEnd: string;
  /** null when the usage ledger could not be read (never shown as 0). */
  observed: number | null;
  percent: number | null;
  state: BudgetState;
  pause: BudgetPause | null;
  runs: number;
  /** For a spending budget, never a self-hosted model's run (`gpuTimeRuns`). */
  unknownRuns: number;
  partialRuns: number;
  unpricedRuns: number;
  /** A spending budget's runs on a self-hosted model (Q8): no spend. */
  gpuTimeRuns?: number;
  cachedInputTokens: number;
  estimated: boolean;
  enforceability: BudgetEnforceability;
  enforceabilityReason: string | null;
  scopeMissing: boolean;
  suggestedLimit: number | null;
  ledgerError: string | null;
}

export interface BlockedAutomations {
  allAutomations: boolean;
  heartbeat: boolean;
  workflows: string[];
}

export interface BudgetsView {
  budgets: BudgetStatus[];
  ledgerDays: number;
  suggestionsAvailable: boolean;
  blocked: BlockedAutomations;
  /** budgets.json could not be read: automations are held until repaired. */
  storeError: string | null;
  foreignEdit: boolean;
  enforcement: string;
}

/** A stored budget (Rust `budgets::BudgetDef`), as `save_budget` returns it. */
export interface BudgetDef {
  id: string;
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  limit: number;
  warnPercent: number;
  hardStop: boolean;
  createdAt: string;
  updatedAt: string;
}

/** A budget from the Settings form (no `id` = create). */
export interface BudgetInput {
  id?: string | null;
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  limit: number;
  warnPercent?: number | null;
  hardStop?: boolean | null;
}

export type BudgetNoticeLevel = 'warn' | 'limit' | 'paused' | 'unavailable';

/** A budget message (Rust `budgets::BudgetNotice`). */
export interface BudgetNotice {
  budgetId: string | null;
  scope: string | null;
  scopeLabel: string | null;
  level: BudgetNoticeLevel;
  metric: BudgetMetric | null;
  window: BudgetWindow | null;
  windowKey: string | null;
  observed: number | null;
  limit: number | null;
  blocking: boolean;
  estimated: boolean;
  message: string;
}

export interface RunLogEntry {
  at: string;
  status: WorkflowStatus;
  durationSec: number;
  note: string;
  /** Shared automated-history/evidence run identity; absent on legacy rows. */
  runId: string | null;
  /** Trailing stderr lines of a failed run; null for successes + legacy rows. */
  errorTail: string | null;
  /** `budget` for a run a usage budget ended (it keeps status `error`);
   *  null otherwise (Rust `RunLogEntry.exit_kind`, Paperclip PR 8b). */
  exitKind: string | null;
}

export interface WorkflowDetail {
  workflow: Workflow;
  prompt: string;
  runLog: RunLogEntry[];
  /** The editor baseline (Rust `vault_workflows::edit_digest`): send it back
   *  as `WorkflowInput.editBase`. A file whose editor-owned fields changed on
   *  disk since rejects the save with `WORKFLOW_EDIT_CONFLICT: …`. */
  editBase: string;
}

/// One row of the Activity Ledger — a unified, newest-first feed merging the
/// agents' four append-only trails (heartbeat log, computer/m365 audit jsonl,
/// workflow run logs). `source` is one of heartbeat|workflow|computer|m365;
/// `deepLink` carries a workflow slug for workflow rows.
export interface LedgerEntry {
  id: string;
  at: string;
  source: 'heartbeat' | 'workflow' | 'computer' | 'm365';
  title: string;
  detail: string;
  ok: boolean;
  deepLink: string | null;
}

/** One file produced by a workflow, surfaced in the outputs view. */
export interface WorkflowOutput {
  path: string;      // vault-relative
  name: string;      // file stem
  format: string;    // 'html' | 'md' | other extension
  modified: string;  // RFC3339
}

export interface WorkflowInput {
  title: string;
  description: string;
  schedule: string;
  agent: string | null;
  model: string | null;
  cwd: string | null;
  outputGlob?: string | null;
  permissionMode: string;
  allowedTools: string[];
  disallowedTools: string[];
  prompt: string;
  /** `WorkflowDetail.editBase` from when the editor loaded the automation.
   *  Omitted by the new-automation form (nothing to compare against). */
  editBase?: string;
  /** Sent only when the user changed it (Paperclip PR 9): omitted keeps the
   *  file's `notify` as it is; `always` removes the key (the default). */
  notify?: 'always' | 'failures' | 'never';
  /** Paperclip PR 9b: sent only when the user changed it; omitted keeps the
   *  file's `skip_if_quiet`. A save that turns it on validates the watch. */
  skipIfQuiet?: boolean;
  /** Paperclip PR 9b: sent only when changed; an empty list removes it. */
  watch?: string[];
  /** OC13: a one-line shell command run before each scheduled occurrence;
   *  a non-zero exit skips it (`skipped_precheck`, no agent turn). Sent only
   *  when changed; '' removes it. Only this editor sets it: an agent's
   *  `automations.save` cannot, and a hand edit needs approval. */
  precheck?: string;
  /** Omitted preserves existing triggers; an empty list removes them. */
  triggers?: string[];
  /** Run inside this chat (overhaul F1): omitted keeps the binding; empty
   *  unbinds it (a new chat each run); a session key binds it. */
  inChat?: string;
  /** End time (G2), an ISO instant: sent only when changed; '' clears it.
   *  Must be in the future, and not set together with `afterRuns`. */
  ends?: string;
  /** End after this many finished runs (G2): sent only when changed; 0 clears it. */
  afterRuns?: number;
  /** Owner Mac (M4): omitted keeps it (a new automation gets this Mac);
   *  '' removes it; 'this' is this Mac; else a machine slug or id. */
  runsOn?: string;
}

/** Heartbeat tunables (Settings, Background work, Heartbeat). */
/// How many chats run at once (Rust `chat_limits::ChatLimits`, 2026-10-20):
/// agent turns across Hollow (1-25, default 10) and in one workspace's own
/// folder (1-10, default 5), and the subagents one chat runs at once (1-10,
/// default 6; 2026-11-12). A build worktree runs one turn at a time.
export interface ChatLimits {
  chatsAtOnce: number;
  chatsPerWorkspace: number;
  subagentsPerChat: number;
}

export interface HeartbeatConfig {
  workHoursStart: number;  // local hour [0..24), inclusive
  workHoursEnd: number;    // local hour (exclusive)
  minIntervalMin: number;  // minimum minutes between beats
  dailyBudget: number;     // max agentic beats per day
}

/// A task the heartbeat extracted from the vault/inbox, queued for approval
/// (the heartbeat's auto-extract queue). Flat wire shape — approving
/// creates the vault task. `status`: 'pending' | 'created' | 'discarded'.
export interface HeartbeatTask {
  id: string;
  created: string;
  status: string;
  title: string;
  due: string | null;
  priority: string | null;   // 'high' | 'medium' | 'low'
  note: string | null;
  sourceVaultPath: string | null;
}

export interface WorkflowRunResult {
  slug: string;
  status: WorkflowStatus;
  durationSec: number;
  output: string;
  error: string | null;
  receipt: EvidenceReceiptWire | null;
  evidenceError: string | null;
}

/** Backend → frontend streaming event during a workflow run. Keyed by
 *  slug so a global listener can fan events to per-workflow run logs. */
export type WorkflowEvent =
  | {
      type: 'started';
      slug: string;
      at: string;
      runId: string;
      recall: EvidenceRecallItemWire[];
      /** The run queue row's source (`cron` | `manual` | `trigger` | `retry`)
       *  and reason, e.g. `file changed: notes/a.md` (Paperclip PR 9). */
      source?: string;
      reason?: string;
      attempt?: number;
      queueId?: number | null;
      /** Merged plan decision 6: the move a claim made off the workflow's
       *  agent budget ceiling, the same record its receipt gets. Null or
       *  absent when the run kept its own agent. */
      switched?: ReceiptSwitchWire | null;
    }
  | { type: 'toolUse'; slug: string; name: string; input: unknown }
  | { type: 'assistantText'; slug: string; text: string }
  /** Paperclip PR 15b: a running run's activity, every 15 s, keyed by slug and
   *  the `started` event's run id. `lastEventAt` is its last stdout/stderr
   *  line (its spawn before the first one). Informational only. */
  | {
      type: 'activity';
      slug: string;
      runId: string;
      queueId: number | null;
      lastEventAt: string | null;
      progress: ProgressPhase | null;
    }
  | {
      type: 'done';
      slug: string;
      /** Paperclip PR 15b: the run (and queue row) this ends. */
      runId?: string;
      queueId?: number | null;
      status: WorkflowStatus;
      output: string;
      durationSec: number;
      error: string | null;
      nextRunAt: string | null;
      receipt: EvidenceReceiptWire | null;
      evidenceError: string | null;
    };

// ── Notifications ────────────────────────────────────────────────────────
//
// In-app notification center, backed by `vault/.notifications.jsonl`.
// Workflows fire one on each non-cancelled run; other code paths can
// surface notifications via `api.createNotification`. The toast stack
// and bell-icon dropdown share the same data — toasts pop on the
// `notification-event` Tauri channel, the center reads from `list`.

export type NotificationKind = 'info' | 'success' | 'warning' | 'error' | 'workflow' | string;

export interface Notification {
  id: string;
  title: string;
  body: string;
  kind: NotificationKind;
  source: string | null;
  actionLabel: string | null;
  /** Where clicking jumps to. Supported schemes:
   *    - `vault:<rel/path>` — open the vault note
   *    - `workflow:<slug>` — open the workflow detail pane
   *    - `https://` / `http://` — open in the user's browser
   */
  actionUrl: string | null;
  created: string;
  read: boolean;
}

export interface NotificationInput {
  title: string;
  body: string;
  kind: NotificationKind;
  source?: string | null;
  actionLabel?: string | null;
  actionUrl?: string | null;
  /** When true, also fire a native OS notification via
   *  `tauri-plugin-notification`. Off by default — caller opts in. */
  notifyNative?: boolean;
}

// ── Operator profile ──────────────────────────────────────────────────────
//
// The durable "how the user works" digest compiled from the vault's `me/`
// folder (Identity, Communication, Anti-patterns, Guardrails, Decisions). The chat
// composer injects `body` on the first message of a session so the agent
// knows the house style without being re-briefed. Settings shows `files` +
// `charCount` plus per-file review freshness so the user can see what's being
// prepended and which context needs an explicit review.

export type OperatorFreshnessStatus = 'fresh' | 'due' | 'overdue' | 'neverReviewed';

export interface OperatorFreshness {
  path: string;
  title: string;
  exists: boolean;
  /** Newer of the declared content-write time and filesystem mtime. */
  lastUpdated: string | null;
  /** Explicit human-review timestamp; this is the freshness clock. */
  lastReviewed: string | null;
  reviewCadenceDays: number;
  status: OperatorFreshnessStatus;
  /** Local calendar date (`YYYY-MM-DD`) when the next review is due. */
  dueAt: string | null;
  /** Negative only when overdue; null for never-reviewed files. */
  daysUntilDue: number | null;
}

export interface OperatorContext {
  /** Ready-to-inject markdown digest. Empty when no `me/` profile files exist. */
  body: string;
  /** Vault-relative paths that contributed, in include order. */
  files: string[];
  charCount: number;
  truncated: boolean;
  /** All five operator files, including missing and never-reviewed entries. */
  freshness: OperatorFreshness[];
}

// ── About you builder (profile_builder.rs) ──────────────────────────────
//
// Facts Hollow found in what the user's agents already know, questions for
// what they don't, and the five me/ notes as they stand. Kept outside the
// vault; nothing is written to me/ until the user adds or answers.

export interface ProfileSuggestion {
  id: string;
  /** `Identity.md` … `Decisions.md`. */
  file: string;
  /** Fills `- **Label:** (fill in)` when present (Name, Role, Preferred tone…). */
  label: string | null;
  /** Heading it goes under; null for the note's main list. */
  section: string | null;
  text: string;
  /** claude-instructions | claude-memory | codex-instructions | gemini-instructions | account | hollow-memory | conversations */
  source: string;
  createdAt: string;
}

export interface ProfileQuestion {
  id: string;
  file: string;
  /** How the answer is filed: `- **Label:** answer`. */
  label: string;
  section: string | null;
  question: string;
  why: string;
  createdAt: string;
}

export interface ProfileSource {
  id: string;
  label: string;
  count: number;
}

export interface ProfileFileView {
  path: string;
  title: string;
  exists: boolean;
  /** Filled-in bullets (placeholders left out), at most 12. */
  items: string[];
  /** `(fill in)`-style lines still waiting. */
  blanks: number;
}

export interface ProfileBuilderView {
  suggestions: ProfileSuggestion[];
  questions: ProfileQuestion[];
  sources: ProfileSource[];
  profile: ProfileFileView[];
  scannedAt: string | null;
  scanning: boolean;
  lastError: string | null;
  autoLearn: boolean;
}

// ── People ────────────────────────────────────────────────────────────────
//
// The vault `people/` stakeholder CRM. Each promoted person file carries a
// frontmatter snapshot (role/relationship/description) + working-notes body.
// The People tab reads the full note + recent mentions on demand.

export interface Person {
  name: string;
  role: string;
  relationship: string;
  description: string;
  /** Vault-relative path, e.g. "people/Jordan Lee.md". */
  path: string;
}

// ── Markdown preview windows ────────────────────────────────────────────────
//
// A `.md` file opened from the OS (file association → RunEvent::Opened → a
// md-preview-N window). `take_preview_file` returns PreviewFile;
// `save_preview_to_vault` returns SavedFile. Mirror of `md_open.rs`.

export interface PreviewFile {
  /** File name, e.g. "README.md". */
  name: string;
  /** Absolute source path (display only). */
  path: string;
  /** Markdown body (possibly truncated for display). */
  content: string;
  /** True if the source already lives under the vault root. */
  inVault: boolean;
  /** Vault-relative path when inVault is true (else null). */
  vaultRelPath: string | null;
  /** True if content was truncated to the display cap. */
  truncated: boolean;
}

export interface SavedFile {
  /** Vault-relative path of the saved copy, e.g. "imported/notes.md". */
  vaultRelPath: string;
}

// ── Chat preview pane ───────────────────────────────────────────────────────
//
// A file the chat agent created/edited, read back for the Chat tab's preview
// pane. Mirror of `types.rs::ChatPreviewFile` (backend `chat::preview_file`).
// The frontend layers the render mode (html / markdown / svg / code) on top
// of `kind` from the file extension.

/** A video token from `media_grant` (media.rs `MediaGrant`). */
export interface MediaGrant {
  token: string;
  mime: string;
  size: number;
  modifiedMs: number | null;
}

/** Read-only native PDF page pixels and bounded accessible text. */
export interface PdfPreviewPage { page: number; pages: number; width: number; height: number; dataUrl: string; text: string; textTruncated: boolean }

export interface ChatPreviewFile {
  /** "text" (utf-8 in `text`), "image" (`dataUrl` set), "video" (size and
   *  time only: it plays through `media_grant`), "pdf" (scoped page reader), or "binary". */
  kind: 'text' | 'image' | 'binary' | 'video' | 'pdf';
  text: string | null;
  dataUrl: string | null;
  size: number;
  /** Last-modified, ms since epoch (null when the FS doesn't report it). */
  modifiedMs: number | null;
  /** True when `text` was cut at the read cap. */
  truncated: boolean;
}

// ── Agent sessions ────────────────────────────────────────────────────────────
// `AgentSessionSummary`, `HarnessStint`, `SessionResumability`,
// `AgentSessionDetail`, `SessionRecordTurn` and `SessionPhase` are generated
// (see the top of this file); the rest mirror `src-tauri/src/types.rs` by
// hand. Hollow mints the session key; the harness's
// own id is per-stint metadata that may be absent entirely (Antigravity
// discards it and resumes with `--continue`), so every id field is nullable.

/** What `move_agent_session` did (Rust `session_store::SessionMove`). */
export interface SessionMove {
  key: string;
  /** The workspace it left; null for a vault-root chat. */
  from: string | null;
  to: string;
  /** Its cwd now. */
  cwd: string;
  /** False when it was already there. */
  moved: boolean;
  record: string | null;
  expiredCards: number;
  warnings: string[];
}

export interface ArtifactSessionLink {
  session: AgentSessionSummary | null;
  run: HistoryRun;
  /** `shared`: another chat in the same workspace folder changed it. */
  attribution: 'confirmed' | 'observed' | 'shared';
}

export interface ArtifactSessionContext {
  vaultPath: string;
  detail: AgentSessionDetail;
  permissionMode?: string | null;
  answers: Record<string, string>;
  receipts: EvidenceReceiptWire[];
}

export interface SessionStoreStatus {
  sessions: number;
  eventsRetained: number;
  bytes: number;
  oldestRetainedIso: string | null;
  /** Absolute path, surfaced so it is visibly outside the vault. */
  path: string;
}

/** Settings, Usage period, in local time (Paperclip PR 6). `last30`:
 *  the last 30 days, Usage only (Settings PR 8); budgets keep the first three. */
export type UsageWindow = 'day' | 'week' | 'month' | 'last30';
/** What a usage figure means (gate D2). Never a bare "$0". */
export type UsageCostStatus = 'reported' | 'estimated' | 'unpriced' | 'partial' | 'unknown';
/** `self-hosted`: billed by the GPU time of the person's own endpoint, never per token (Q8). */
export type UsageBilling = 'api' | 'subscription' | 'unknown' | 'self-hosted';

/**
 * One group of usage-ledger rows. Every run lands in exactly one of
 * metered / partial / unknown / notStarted / inProgress; `unpriced` is the
 * subset of metered with tokens but no price. Costs are null (never 0) when
 * no row of that kind exists, and reported and estimated are never mixed.
 */
export interface UsageBucket {
  key: string;
  label: string;
  runs: number;
  metered: number;
  partial: number;
  unknown: number;
  notStarted: number;
  inProgress: number;
  unpriced: number;
  /** Runs on a self-hosted model (Q8): billed by GPU time, never priced and
   *  never in `unpriced`. Their bucket is keyed `self-hosted:<served id>`. */
  gpuTime?: number;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  reportedCostUsd: number | null;
  estimatedCostUsd: number | null;
}

/**
 * One row of Hollow's price table (`usage_prices::Price`), USD per million
 * tokens. Used only where the agent can't price a run itself: a Claude
 * model it reports as unpriced, and every Codex run (overnight 9 A13).
 */
export interface ModelPrice {
  model: string;
  input: number;
  cacheWrite: number;
  cacheRead: number;
  output: number;
  source: string;
  asOf: string;
}

/** A price as Settings, Usage, Prices lists it: saved is the person's own row. */
export interface ModelPriceRow extends ModelPrice {
  saved: boolean;
}

/** Local recording/checkpoint day, matching the summary's row selection. */
export interface UsageDay {
  date: string;
  total: UsageBucket;
  byAgent: UsageBucket[];
  /** Exact night-queue direct chat runs only, excluding descendants/manual lanes. */
  nightByAgent: UsageBucket[];
}

export interface UsageSummary {
  window: UsageWindow;
  /** The agent tab the summary is filtered to (`claude`, `codex`,
   *  `self-hosted`, ...); null for every agent (Settings PR 8). Every list
   *  holds that agent's runs only, except `agents`. Optional: older fixtures. */
  agent?: string | null;
  /** Every agent tab with runs in the period, filter or not. */
  agents?: UsageBucket[];
  /** By the model a run reported, else the one it asked for; sums to `total`. */
  byModel?: UsageBucket[];
  /** By outcome (finished, stopped, failed, moved to another account...); sums to `total`. */
  outcomes?: UsageBucket[];
  /** The ten costliest conversations, keyed by session key (opens its chat). */
  topConversations?: UsageBucket[];
  /** The window's start, fixed-width UTC (`…Z`). */
  since: string;
  generatedAt: string;
  total: UsageBucket;
  bySurface: UsageBucket[];
  /** Workflow runs plus the teammate delegations they made, by slug. */
  byWorkflow: UsageBucket[];
  byAgent: UsageBucket[];
  /** By workspace ("none" is "No project"); sums to `total`. */
  byProject: UsageBucket[];
  /** Chat and delegate rows by the conversation's root, subagents rolled up. */
  byLead: UsageBucket[];
  /** Subagent conversations, top 50 by tokens; the rest in "other". */
  bySubagent: UsageBucket[];
  /** By hashed account label, or the agent's own login; sums to `total`. */
  byAccount: UsageBucket[];
  daily: UsageDay[];
  undatedRuns: number;
  nightAttributionError: string | null;
  /** The oldest row still kept: when recording began. */
  ledgerSince: string | null;
  /** The newest recording failure in the window, unless a row came after. */
  ledgerError: string | null;
  /**
   * OB07: summed `cost_floor_usd` of runs with tokens but no cost ("≥ $X more
   * in N incomplete runs"). A lower bound, never part of a known total; null
   * when no run has one.
   */
  partialFloorUsd?: number | null;
  /** How many runs that floor covers. */
  partialRuns?: number;
}

/** Where a Claude plan-usage window's numbers came from (Paperclip PR 8c). */
export type ClaudeQuotaSource = 'stream' | 'endpoint';

/** One Claude plan window, mirroring `claude_quota::ClaudeQuotaWindow`. */
export interface ClaudeQuotaWindow {
  /** `five_hour`, `seven_day`, `seven_day_opus`, `seven_day_sonnet` or `weekly_scoped:<model>`. */
  id: string;
  label: string;
  /** Percent of the window used. Can exceed 100 on overage. */
  usedPercent: number;
  /** RFC 3339 UTC, or null when unknown. */
  resetsAt: string | null;
  source: ClaudeQuotaSource;
  /** When Hollow saw this value (RFC 3339 UTC). */
  observedAt: string;
}

/**
 * Settings, Usage "Claude plan" card (Paperclip PR 8c). Display only; the
 * `claude_quota` command returns null to hide the card.
 */
export interface ClaudeQuota {
  source: ClaudeQuotaSource;
  observedAt: string;
  windows: ClaudeQuotaWindow[];
}

/** Claude account routing: always the pinned account, or spread by usage. */
export type ClaudeAccountRouting = 'single' | 'spread';
/**
 * `near`: at or over the spread ceiling; `limited`: out of plan allowance
 * until a window resets; `invalid`: Claude refused the token.
 */
export type ClaudeAccountStatus = 'ok' | 'near' | 'limited' | 'invalid' | 'missingToken' | 'unknown';

/** Mirrors `claude_accounts::WindowView`. */
export interface ClaudeAccountWindow {
  usedPercent: number;
  resetsAt: string | null;
}

/** Mirrors `claude_accounts::UsageView`. */
export interface ClaudeAccountUsage {
  fiveHour: ClaudeAccountWindow | null;
  sevenDay: ClaudeAccountWindow | null;
  status: string | null;
  overageDisabledReason: string | null;
  source: 'probe' | 'stream';
  observedAt: string;
}

/** Mirrors `claude_accounts::AccountView`. Never carries a token. */
export interface ClaudeAccount {
  id: string;
  label: string;
  enabled: boolean;
  status: ClaudeAccountStatus;
  note: string | null;
  usage: ClaudeAccountUsage | null;
  lastUsedAt: string | null;
  createdAt: string | null;
}

/** Settings, Claude accounts, mirroring `claude_accounts::AccountsView`. */
export interface ClaudeAccountsView {
  routing: ClaudeAccountRouting;
  /** The account `single` routing uses (the first added one by default). */
  pinned: string;
  ceilingPercent: number;
  /** R1: a chat turn whose account runs out resumes on another added
   *  account (Settings, Accounts, on by default). */
  failover: boolean;
  /** The account a new Claude run would use now; null when none of the
   *  added accounts can run, so runs use the Claude Code login. */
  nextAccount: string | null;
  /** The accounts added in Settings (Claude Code's own login is never listed). */
  accounts: ClaudeAccount[];
  /** Claude Code's own login as last seen (never a Keychain read), for the
   *  login row shown while a claude.ai connector set to Always pins chats
   *  to it (`claude_accounts::LoginUsageView`). */
  loginUsage?: ClaudeLoginUsage | null;
  error: string | null;
}

/** Mirrors `claude_accounts::LoginUsageView`. */
export interface ClaudeLoginUsage {
  fiveHour: ClaudeAccountWindow | null;
  sevenDay: ClaudeAccountWindow | null;
  observedAt: string;
}

/** Settings, Accounts, one row per non-Claude agent CLI
 *  (`provider_accounts::ProviderAccount`). */
export interface ProviderAccount {
  id: 'codex' | 'antigravity' | 'grok';
  label: string;
  installed: boolean;
  /** null when it could not be told. */
  signedIn: boolean | null;
  email: string | null;
  plan: string | null;
  note: string | null;
  signIn: 'browser' | 'terminal';
  canSignOut: boolean;
  /** Last passive reading from this login's own run; absent means unknown. */
  limits?: { observedAt: string; windows: import('./chatUsage').PlanWindow[] } | null;
}

export interface ClaudeAccountUpdate {
  id: string;
  label?: string | null;
  enabled?: boolean | null;
  /** A replacement token; never returned. */
  token?: string | null;
}

/** The finished chat turn's usage, from the `usage` chat event. */
export interface TurnUsage {
  costUsd: number | null;
  costStatus: UsageCostStatus;
  billing: UsageBilling;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  outputTokens: number | null;
  outcome?: string;
  model?: string | null;
  /** How the figures were obtained (`per_run`, `session_delta`, …). */
  basis?: string;
  /** Tokens in the context window after the turn's last message; null when
   *  the agent reports no figure (Antigravity). Codex's comes from its
   *  rollout file (overnight 9 A10). */
  contextTokens?: number | null;
  /** The window the agent reported with that figure (Codex); null when it is
   *  inferred from the model (`chatUsage.contextWindow`). */
  contextWindow?: number | null;
  /** A self-hosted run's endpoint, by name (Q8). */
  endpoint?: string | null;
}

// Repo features R6: `conversation_context_use` (src-tauri/src/context_use.rs),
// the usage ring's What's using it and Cache. Sizes only, never result text.
export interface ContextUse {
  /** The open stint's start: the window is the current agent session. */
  windowStartedAt: string | null;
  windowTurns: number;
  /** Window turns whose tool results all carry a size (older ones don't). */
  measuredTurns: number;
  /** A turn stopped recording steps: the figures are a floor. */
  atLeast: boolean;
  /** The 6 largest tools by bytes, then `other`. */
  byTool: { tool: string; bytes: number; calls: number }[];
  /** The 3 largest single results, largest first. */
  largest: { turn: number; tool: string; bytes: number; target: string | null }[];
  /** The last 10 turns, oldest first. */
  cache: ContextCacheTurn[];
  compactions: number;
}

export interface ContextCacheTurn {
  turn: number;
  inputTokens: number | null;
  cachedInputTokens: number | null;
  cachedShare: number | null;
  cold: boolean;
  cause: 'newSession' | 'idle' | 'model' | 'compacted' | 'unknown' | null;
}

/** `phase: 'queued'` or `'preparing'` with an empty `runId` is a chat turn still
 *  in the run queue (Paperclip PR 7b): it holds no lease yet, but it owns its
 *  key. `queued` means it truly can't start (a limit, memory, another run's
 *  folder); `preparing` means nothing holds it back (2026-11-16). */
export interface LiveAgentSession {
  key: string;
  agent: string;
  runId: string;
  phase: SessionPhase;
  phaseEnteredAt: string;
  failureReason: string | null;
  /** Paperclip PR 15b: when the running agent last printed a stdout or stderr
   *  line (its spawn before the first one); null before the spawn and for a
   *  queued turn. Backend surface only: no reload rehydration reads it yet. */
  lastEventAt?: string | null;
  /** Paperclip PR 15b: the progress phase of its latest tool call. */
  progress?: ProgressPhase | null;
}

/** Paperclip PR 15b: what kind of work a running agent is doing, read from
 *  its latest tool NAME only (Rust `run_activity::ProgressPhase`). */
export type ProgressPhase = 'researching' | 'usingTools' | 'workingWithFiles' | 'coordinating';

// `AgentSettings` (app-wide execution preferences; a blank model uses the
// CLI's own default) is generated; see the top of this file.
export interface AgentDescriptor {
  id: string;
  label: string;
  available: boolean;
  capabilities: {
    modelSelection: boolean;
    sessionResume: boolean;
    toolRestrictions: boolean;
    ownedMcp: boolean;
    /** Adapter can multiplex local turns on an account-scoped server. */
    sharedServer?: boolean;
    /** AC1 adapter flags (`agent::flags`, the one per-agent table). True only
     *  when Hollow's adapter does it today. A feature asks the flag, never
     *  the agent's name. Optional on read: an older backend sends none. */
    steer?: boolean;
    interrupt?: boolean;
    fork?: boolean;
    rollback?: boolean;
    queue?: boolean;
    pendingSurvivesRestart?: boolean;
    planProposals?: boolean;
    /** Why each false flag above is false, keyed by flag name, as a plain
     *  sentence naming the feature and the agent. Optional on read (`?? {}`). */
    unsupportedReasons?: Record<string, string>;
    structuredOutput: boolean;
    reportsContext?: boolean;
    reasoning?: boolean;
    /** Paperclip PR 14a: the effort levels this agent's CLI accepts (Claude
     *  `--effort`, Codex `model_reasoning_effort`); empty for none. Optional
     *  on read (`?? []`). */
    effortLevels?: string[];
    /** Merged plan slice 2: permission modes this agent can't run, each with
     *  the reason the pickers show (`agent::unavailable_mode`; Antigravity has
     *  no `auto`). Optional on read (`?? {}`); see src/permModes.ts. */
    unavailableModes?: Record<string, string>;
    /** Why the agent can't use the desktop tools in a chat (CU4); absent when it can. */
    desktopUnavailable?: string | null;
  };
}

/** One Doctor check (Cua CU10): a state, a line and a copyable fix. */
export interface ComputerCheck { id: string; label: string; state: 'ok' | 'warn' | 'fail' | 'unknown'; line: string; fix: string | null }

/** `computer_chat_state` (Cua CU8): one chat's desktop, for its chip and card. */
export interface ComputerEntry {
  id: number; atMs: number; tool: string; tier: string; target: string;
  outcome: 'ok' | 'failed' | 'refused'; detail: string; app: string | null; hasFrame: boolean;
}
export interface ComputerChatView { paused: boolean; takenOver: boolean; entries: ComputerEntry[]; frameId: number | null }

/** One app agents can't use on This Mac until the person allows it (CU6). */
export interface DeniedApp { label: string; allowed: boolean }

/** One host an agent added to its own deny list (ND1). Only the person removes it. */
export interface AgentDeniedHost { pattern: string; addedAt: string; by: string }

/** `computer_status` (Cua CU7): what the Computer control sheet shows. */
export interface ComputerStatus {
  enabled: boolean;
  sandboxEnabled: boolean;
  thisMacEnabled: boolean;
  pinned: boolean;
  target: 'sandboxVm' | 'thisMac' | null;
  version: string | null;
  brokerListening: boolean;
  liveSessions: number;
}

/** `codex_helper_status` (agent effectiveness F9, wave 4 · V1): whether the
 *  installed Codex runs Hollow's tool-free helpers. Rust
 *  `codex_probe::CodexHelperStatus`. */
export interface CodexHelperStatus {
  status: 'verified' | 'unverified' | 'missing' | 'unknown';
  /** What `codex --version` printed, when it ran. */
  version: string | null;
  /** The versions Hollow verified with its local check. */
  verifiedVersions: string[];
  /** `probed` or `cached`; null when nothing was read. */
  served: 'probed' | 'cached' | null;
  /** One sentence: the state and, when helpers can't run, the fallback. */
  message: string;
}

export interface AgentConversationTurn {
  reasoning?: string;
  id: string;
  role: 'user' | 'assistant';
  text: string;
  tools: { id: string; name: string; input: Record<string, unknown>; result: string | null; /** WD1: set when `result` is a preview (lean cold open). */ resultChars?: number; resultLines?: number; resultTurn?: number; isError: boolean; status: 'running' | 'done' | 'error'; /** AG7: the spawning call's id for a Claude subagent's call; absent otherwise. */ parentToolUseId?: string }[];
  agent: string;
  runId: string | null;
  startedAt: string;
  /** When the turn's run ended (assistant turns only, overnight 11-12 C8):
   *  the reply row's time when no receipt came back. Absent while running
   *  and from older builds and fixtures. */
  endedAt?: string | null;
  status: string | null;
  costUsd: number | null;
  error: string | null;
  receipt: import('./executionReceipt').EvidenceReceiptWire | null;
  transcriptIncomplete: boolean;
  /** The turn's result kind (Paperclip PR 14), assistant turns only; absent
   *  from older transcripts and from fixtures. */
  resultKind?: string | null;
  /** Stopped before its agent was spawned (Paperclip PR 14b); Continue is
   *  not offered for it. Absent from older builds and fixtures. */
  stoppedBeforeSpawn?: boolean;
  /** What the person said to this turn while it ran (mid-turn steering),
   *  assistant turns only; absent when nobody steered. */
  steers?: import('./chatSteer').SteerWire[];
  /** The files (not images) a user turn was sent with (B1a, ENG-104), from
   *  the store's `turns.attachments`; absent on replies and older turns. */
  files?: { name: string; path: string; size?: number }[];
}
export interface AgentConversation {
  summary: AgentSessionSummary;
  turns: AgentConversationTurn[];
  eventsRetained: boolean;
  permissionMode?: string | null;
}

// ── Structured question cards (Paperclip PR 13) ─────────────────────────────
// Mirrors `interaction_schema.rs` (the normalized request and the answer)
// and `ChatInteraction` / `ResolvedInteraction` in types.rs. The chat-event
// member lives in store/chatRuntime.ts's local ChatEventPayload union.

export type InteractionMode = 'questions' | 'confirmation' | 'checkbox' | 'verdicts';
export type InteractionAnswerMode = 'single' | 'multi' | 'text';
export type InteractionVerdict = 'approve' | 'reject' | 'defer';

export interface InteractionOption {
  id: string;
  label: string;
  description?: string;
  /** What choosing it does in the question's `scenario` (decision cards). */
  outcome?: string;
  recommended?: boolean;
}

export interface InteractionTextValidation {
  minLength?: number;
  maxLength?: number;
  /** Must match the WHOLE answer. */
  pattern?: string;
  inputType?: 'text' | 'number' | 'integer';
  minimum?: number;
  maximum?: number;
}

export interface InteractionQuestion {
  id: string;
  prompt: string;
  header?: string;
  /** One line under the question. Shown ahead of helpText. */
  reason?: string;
  /** One concrete scenario the options' outcomes are about (decision cards). */
  scenario?: string;
  helpText?: string;
  required: boolean;
  answerMode: InteractionAnswerMode;
  options: InteractionOption[];
  allowCustom: boolean;
  customLabel?: string;
  textValidation?: InteractionTextValidation;
}

export interface InteractionTarget { path: string }

export interface InteractionRequest {
  mode: InteractionMode;
  title?: string;
  description?: string;
  submitLabel?: string;
  supersedeOnUserMessage: boolean;
  questions?: InteractionQuestion[];
  confirmation?: {
    prompt: string;
    detailsMarkdown?: string;
    acceptLabel?: string;
    rejectLabel?: string;
    target?: InteractionTarget;
  };
  checkbox?: {
    prompt: string;
    detailsMarkdown?: string;
    options: InteractionOption[];
    defaultSelected: string[];
    minSelected?: number;
    maxSelected?: number;
    target?: InteractionTarget;
  };
  verdicts?: {
    prompt: string;
    detailsMarkdown?: string;
    items: { id: string; label: string; description?: string; previewMarkdown?: string }[];
    verdicts: InteractionVerdict[];
    requireReasonOn: InteractionVerdict[];
    allowBulkApprove: boolean;
    target?: InteractionTarget;
  };
}

export interface InteractionQuestionAnswer {
  selected?: string[];
  other?: string;
  text?: string;
}

/** Which fields apply depends on the request's mode: `answers` (questions),
 *  `accepted` + `note` (confirmation), `selected` (checkbox), `verdicts` +
 *  `note` (verdicts). */
export interface InteractionAnswer {
  answers?: Record<string, InteractionQuestionAnswer>;
  accepted?: boolean;
  note?: string;
  selected?: string[];
  verdicts?: Record<string, { verdict: InteractionVerdict; reason?: string }>;
}

export type InteractionStatus = 'pending' | 'stale' | 'answered' | 'superseded' | 'expired' | 'dismissed';

/** A request Hollow wrote for the person, sent unchanged (overnight 9 A1):
 *  `chat_send`'s `canned`. Its words settle nothing and teach nothing. */
export type CannedRequest = 'canvas' | 'canvasRefresh' | 'explain';

export interface ChatInteraction {
  id: string;
  sessionKey: string;
  turnOrd: number;
  runId: string;
  agent: string;
  /** The native session that asked: the answers go there when the
   *  conversation still resumes it, else as an ordinary message. */
  nativeSessionId: string | null;
  toolUseId: string;
  source: 'mcp' | 'fenced';
  mode: InteractionMode;
  request: InteractionRequest;
  targetPath: string | null;
  supersedeOnUserMessage: boolean;
  status: InteractionStatus;
  statusReason: string | null;
  supersededBy: string | null;
  answer: InteractionAnswer | null;
  createdAt: string;
  resolvedAt: string | null;
}

export interface ResolvedInteraction {
  interaction: ChatInteraction;
  /** Sent verbatim as the next turn. */
  prompt: string;
  /** Whether it goes into the native session that asked; false when the
   *  conversation has moved on to another, so it is an ordinary message. */
  resumes: boolean;
}

// ── Saved turn feedback (Paperclip PR 17) ──

export type FeedbackVote = 'useful' | 'notUseful';

/** `record_turn_feedback` input: the turn's exact run id, the vote and an
 *  optional reason. The snapshot is built by the backend from its stores. */
export interface TurnFeedbackInput {
  runId: string;
  vote: FeedbackVote;
  reason?: string | null;
}

/** What a vote saw, redacted then capped (answer at most 8,000 chars). */
export interface TurnFeedbackSnapshot {
  asked: string;
  answer: string;
  answerTruncated: boolean;
  tools: string[];
  receiptId: string | null;
  receiptStatus: string | null;
  verificationState: string | null;
  recall: string[];
  /** The turn row's stored status when the vote was saved. */
  turnStatus: string | null;
}

/** One saved vote (one per turn; a re-vote replaces it). */
export interface TurnFeedback {
  id: number;
  sessionKey: string;
  sessionTitle: string | null;
  turnOrd: number;
  runId: string;
  vote: FeedbackVote;
  reason: string | null;
  agent: string | null;
  model: string | null;
  snapshot: TurnFeedbackSnapshot;
  createdAt: string;
  updatedAt: string;
}

/** `disk_guard` reply (Rust `disk_guard::Report`, overnight D1). */
export interface DiskReport {
  /** The home volume's free space. */
  freeBytes: number | null;
  thresholdGb: number;
  /** Home or the vault's volume is under the threshold. */
  low: boolean;
  /** The vault's volume, only when it isn't home's (morning N6). */
  vault?: { path: string; freeBytes: number | null; low: boolean } | null;
  /** Largest first; `bytes` null when it couldn't be sized in time. */
  folders: Array<{ path: string; kind: 'cargo-target' | 'worktree' | 'cache'; bytes: number | null }>;
  cleanup: string[];
}

/** One gate run on the remote runner (Rust `remote_status::RunnerGate`). */
export interface RunnerGate {
  start: string;
  /** null while it runs, or when it was killed before it finished. */
  end: string | null;
  seconds: number | null;
  ok: boolean | null;
  exitCode: number | null;
  failedStep: string | null;
  commit: string | null;
  /** The worktree it ran for (the runner's folder name for it, `ri27-mm5`). */
  checkout: string | null;
}

/** `runner_status` reply (Rust `remote_status::RunnerStatus`, overnight N1):
 *  Settings, Devices, Runs on's Mac mini runner row. */
export interface RunnerStatus {
  host: string | null;
  state: 'none' | 'unchecked' | 'ready' | 'refusing' | 'unreachable' | 'error';
  checkedAt: string | null;
  millis: number | null;
  /** Why it isn't ready, with what to check. */
  message: string | null;
  machine: string | null;
  /** Its system, `darwin` or `linux` (a Linux runner, 2026-11-03); absent
   *  from an older Hollow. */
  os?: string | null;
  /** Its hollow-run-preflight line (`ok host=… free=243GB mem=92% rust=…`). */
  preflight: string | null;
  running: number | null;
  gateRunning: boolean;
  lastGate: RunnerGate | null;
  /** Who it belongs to and who else can log in there (remote runners 2);
   *  null until it answered a check. */
  owner?: RunnerOwner | null;
  /** Smart routing 3: hollow-slot's latest decisions and the runner's tools
   *  (Rust `build_slots::RouteStatus`), read fresh with each answer. */
  routing?: RouteStatus;
}

/** One line of hollow-slot's route-log (Rust `build_slots::RouteDecision`). */
export interface RouteDecision {
  at: string | null;
  /** `here`, or the runner's SSH host. */
  where: string;
  why: string;
  label: string;
  /** `auto`, `--auto`, `--here`, `--remote`, `--on`, `--send` or `Builds and tests`. */
  source: string;
}

/** Newest first; `caps` is the runner's last report
 *  (`rust=1.96.0 node=26.3.1 … postgres= …`; an empty value means none). */
export interface RouteStatus {
  decisions: RouteDecision[];
  caps: { host: string; at: string | null; line: string } | null;
}

/** A runner's owner as its check found it (Rust `runner_owner::RunnerOwner`,
 *  remote runners 2, 2026-11-03). */
export interface RunnerOwner {
  /** `you`: this Hollow recorded it; `other`: another Hollow did; `none`: no record there. */
  state: 'you' | 'other' | 'none';
  recordedAt: string | null;
  /** SSH keys its runner account lets in; null when they couldn't be read. */
  keys: number | null;
  /** Of those, how many are neither the owner's nor this Mac's. */
  otherKeys: number | null;
  /** This Hollow is the owner, but the record names another key than this Mac's now. */
  keyChanged: boolean;
  /** What to look at, worst first. */
  warnings: string[];
}

/** How a workspace's runner turns reach Google Cloud (Rust
 *  `runner_access::CloudAccess`, remote runners 2026-11-03). */
export type RunnerCloudAccess = 'off' | 'limited' | 'full' | 'service-account';

/** A workspace's Runs on → On the runner (Rust `runner_access::RunnerAccess`):
 *  both off until the person turns one on. */
export interface RunnerAccess {
  github: boolean;
  cloud: RunnerCloudAccess;
  serviceAccount?: string;
}

/** Settings, Devices, Remote runner, GitHub (Rust
 *  `runner_access::GithubStatus`): never the token itself. */
export interface RunnerGithubStatus {
  source: 'runner-token' | 'gh-login';
  tokenSaved: boolean;
  ghLogin: boolean;
}

/** Settings, Accounts, 1Password (Rust `onepassword::Status`, parity row
 *  7.5): never a secret, only references, states and fixed messages. */
export interface OnePasswordStatus {
  enabled: boolean;
  /** The `op` command line tool is on this Mac. */
  installed: boolean;
  /** The 1Password app is installed (`op` can borrow its sign-in). */
  desktopApp: boolean;
  cli: {
    state: 'off' | 'not-installed' | 'unchecked' | 'ready' | 'locked' | 'not-signed-in' | 'timed-out' | 'error';
    message: string;
    version: string | null;
  };
  slots: OnePasswordSlot[];
}

/** A secret a reference can stand in for (the GitHub runner token, the
 *  Linear API key, each Claude account). */
export interface OnePasswordSlot {
  key: string;
  label: string;
  reference: string | null;
  /** Why the last read through 1Password failed, null when it worked. */
  lastError: string | null;
}

/** The install build folders and which stay (Rust `install_targets::Plan`,
 *  overnight A5): Settings, Advanced, Disk space's preview (overnight H20). */
export interface InstallTargetsPlan {
  /** Newest first; `keep` says why one stays, null when it goes. */
  targets: Array<{ path: string; name: string; lastUsed: string | null; keep: string | null }>;
  /** The paths a prune deletes, newest first. */
  remove: string[];
  keepNewest: number;
  /** Binds the confirm to exactly `remove`. */
  hash: string;
  notes: string[];
  /** Folders an interrupted delete left aside; also in `remove`. */
  leftovers: string[];
}

/** One of Hollow's SQLite files (Rust `hollow_databases::DbFile`, repo
 *  features R7b): Settings, Advanced, Hollow's databases. */
export interface HollowDbFile {
  kind: 'sessions' | 'searchIndex' | 'embedIndex' | 'companion' | 'projectIndex';
  stem: string;
  path: string;
  owner: 'thisVault' | 'otherVault' | 'app';
  /** The owning vault's folder name, for another vault's file. */
  ownerName: string | null;
  bytes: number;
  walBytes: number;
  modified: string | null;
  schemaVersion: number | null;
  /** Check integrity only: `ok`, the first problems, or why it didn't finish. */
  quickCheck: string | null;
  /** Check integrity only: ordinary tables' row counts. */
  tables: Array<{ name: string; rows: number }> | null;
  error: string | null;
}

/** `hollow_databases` reply (Rust `hollow_databases::Report`). */
export interface HollowDatabases {
  folder: string;
  files: HollowDbFile[];
  /** What Disk space → Remove old indexes… would remove now. */
  staleCount: number;
  staleBytes: number;
}

/** Other vaults' old indexes (Rust `hollow_databases::Plan`): Disk space →
 *  Remove old indexes…'s preview (repo features R7b). */
export interface StaleIndexesPlan {
  folder: string;
  /** `why` is set for what isn't an ordinary old index: `left by an
   *  interrupted removal` for a `<db>.removing` tombstone. */
  files: Array<{ path: string; kind: HollowDbFile['kind']; ownerName: string; bytes: number; modified: string | null; why: string | null }>;
  bytes: number;
  /** Binds the confirm to exactly these files. */
  hash: string;
  /** Other vaults' files that stay, and why. */
  kept: Array<{ path: string; kind: HollowDbFile['kind']; ownerName: string; why: string }>;
}

/** What Remove old indexes… did (Rust `hollow_databases::Removed`). */
export interface StaleIndexesRemoved {
  removed: string[];
  failed: Array<[string, string]>;
  bytes: number;
}

// Move Hollow home (2026-11-12, Rust `move_home.rs`): the dry run's plan, the
// run's status and rollback. Settings, Sources, Hollow folder, Move Hollow
// home (`settings/MoveHomePanel.tsx`); agents use system.move_hollow_home_plan,
// system.move_hollow_home, system.move_hollow_home_status and
// system.rollback_move_home.

/** What a planned move is: the vault, the whole Projects folder, a folder
 *  beside the vault joining Projects, a `personal` folder or one going to
 *  Archive. */
export type MoveKind = 'vault' | 'projectsRoot' | 'project' | 'personal' | 'archive';

/** The dry run's and the run's options (`move_home_plan`, `move_home_start`). */
export interface MoveHomeOptions {
  includeVaultSiblings?: boolean;
  personal?: string[];
  launchAgents?: string[];
  scripts?: string[];
  allowOutsideHome?: boolean;
  budgetSecs?: number;
}

export interface PlannedMove {
  from: string;
  to: string;
  kind: MoveKind;
  bytes: number;
  files: number;
  /** Files under `from` that are in iCloud only (they must be downloaded
   *  before a copy move). */
  dataless: number;
  /** What those files weigh once downloaded. */
  datalessBytes: number;
  /** `copy`: a folder in iCloud, cloned out, compared, renamed into place,
   *  its original removed last. `rename`: a same-disk rename. */
  method: 'rename' | 'copy';
  /** The symlink at `to` the move replaces, if any. */
  replacesSymlink: string | null;
  /** True when `from` keeps a link to `to`. */
  compatSymlink: boolean;
  why: string;
}

/** A store in the support or app-data folder renamed to the new vault stem. */
export interface StoreRekey { from: string; to: string; what: string }
/** A Sessions DB column whose paths or workspace ids are rewritten. */
export interface ColumnRewrite { table: string; column: string; rows: number }
export interface FileRewrite { path: string; changes: number; what: string }
/** A project's workspace id that changes because its path moves. */
export interface IdChange { old: string; new: string; projectFrom: string; projectTo: string; hasWorkspace: boolean }
/** `winner` is the folder that keeps the name, when one does. */
export interface Collision { kind: string; path: string; detail: string; winner: string | null }
/** `top` is [folder, count] pairs, most first. */
export interface DatalessRoot { root: string; count: number; bytes: number; top: [string, number][]; sample: string[] }
export interface DatalessReport {
  total: number;
  /** What all the iCloud-only files weigh once downloaded. */
  bytes: number;
  /** iCloud-only folders among `total` (their contents show once downloaded). */
  dirs: number;
  /** iCloud-only files in folders that move by copy: download them first. */
  toDownload: number;
  toDownloadBytes: number;
  /** Rough seconds to download them (small files ~12 a second, large ~14 MB/s). */
  downloadSecs: number;
  /** Folders that move by copy, and roughly how long the copies take. */
  copyFolders: string[];
  copySecs: number;
  roots: DatalessRoot[];
  fileProviderDomain: string | null;
  partial: boolean;
}
/** `blocking` stops the run (a running chat); others are told. */
export interface Blocker { kind: string; detail: string; blocking: boolean }
export interface RepoRepair { repoFrom: string; repoTo: string; worktrees: number; broken: number; skipped: string | null }
export interface AgentStoreChange { agent: string; path: string; changes: number; dead: number; merges: number; ambiguous: string[] }
/** A LaunchAgent or script naming a moved path; changed only when `onPlan`. */
export interface LaunchItem { label: string; path: string; mentions: string[]; onPlan: boolean }

/** The dry run (`move_home_plan`, Rust `move_home::MoveHomePlan`). The run
 *  executes exactly this plan; `hash` binds the confirm to it. */
export interface MoveHomePlan {
  version: number;
  created: string;
  hash: string;
  /** False while something blocks the run (see `blockers`). */
  ready: boolean;
  /** True when a probe ran out of time and the counts are a floor. */
  partial: boolean;
  hollowHome: string;
  vaultFrom: string;
  vaultTo: string;
  oldStem: string;
  newStem: string;
  moves: PlannedMove[];
  mkdirs: string[];
  mappings: [string, string][];
  /** Folders beside the vault, listed in every dry run; they move only
   *  with `includeVaultSiblings`. */
  besideVault: string[];
  stores: StoreRekey[];
  columns: ColumnRewrite[];
  files: FileRewrite[];
  ids: IdChange[];
  orphans: string[];
  collisions: Collision[];
  dataless: DatalessReport;
  sameVolume: boolean;
  freeBytes: number;
  totalBytes: number;
  blockers: Blocker[];
  processes: string[];
  launchAgents: LaunchItem[];
  scripts: LaunchItem[];
  repos: RepoRepair[];
  agents: AgentStoreChange[];
  backup: { covered: boolean | null; detail: string };
  notes: string[];
  /** The readable report (Rust `describe(plan)`). */
  text: string[];
}

export interface MoveHomeStep { step: string; state: 'started' | 'done' | 'failed' | 'undone'; detail: string; at: string }
export interface MoveHomeCheck { name: string; ok: boolean; detail: string }

/** The journal of the last move (`move_home_status`). */
export interface MoveHomeStatus {
  state: 'none' | 'staged' | 'running' | 'done' | 'failed' | 'rolledBack';
  backupDir: string | null;
  steps: MoveHomeStep[];
  checks: MoveHomeCheck[];
  lastResult: string | null;
  canRollBack: boolean;
  /** Why a rollback would be refused now (new writes since the verify). */
  rollbackRefusal: string | null;
}

/** `move_home_start`'s reply; Hollow quits right after. */
export interface MoveHomeStarted { backupDir: string; planFile: string; helperPid: number; note: string }
/** `move_home_rollback`'s reply, staged the same two-process way. */
export interface MoveHomeRollback { staged: boolean; backupDir: string; note: string }

/** What a prune did (Rust `install_targets::Pruned`). */
export interface InstallTargetsPruned {
  removed: string[];
  /** [path, error] for each folder that couldn't be removed. */
  failed: Array<[string, string]>;
  kept: number;
}

/** Worktree hygiene (Rust `worktree_hygiene::Report`, overnight A9). */
export interface WorktreeHygieneReport {
  repos: Array<{
    repo: string;
    defaultBranch: string | null;
    worktrees: Array<{
      path: string; branch: string | null; workspace: string | null; merged: boolean; clean: boolean; unpushed: number;
      ignored: string[]; ignoredCount: number; inUse: boolean; removable: boolean; reason: string | null;
      /** OC6: generated folders an idle worktree that stays could shed (a dry report; nothing deletes them yet). */
      reclaim?: Array<{ path: string; bytes: number | null; idleDays: number }>;
    }>;
    localOnlyBranches: Array<{ name: string; commits: number; lastCommit: string }>;
    tildeFolders: Array<{ path: string; bytes: number | null }>;
    error: string | null;
  }>;
  /** Worktrees a clean removes, then `~` folders. */
  remove: string[];
  /** Binds the confirm to exactly `remove` and how each worktree goes. */
  hash: string;
  /** OC6: binds a later reclaim to exactly the listed folders (sizes left out). */
  reclaimHash?: string;
  /** OC6: the listed folders' total where sizes were read. */
  reclaimBytes?: number;
  tildeBytes: number;
  inUseUnknown: boolean;
}

/** What a worktree clean did (Rust `worktree_hygiene::Cleaned`). */
export interface WorktreesCleaned {
  removed: string[];
  failed: Array<[string, string]>;
  /** Due to go but not reached before the time budget ran out (PER-3). */
  remaining?: string[];
  complete?: boolean;
  /** Free bytes read from each volume before and after (statvfs, not du). */
  volumes?: Array<{ path: string; freeBefore: number; freeAfter: number; freed: number }>;
  freeBefore?: number;
  freeAfter?: number;
  freed?: number;
  elapsedMs?: number;
}

/** Overnight summary used by Home and Automations (Rust `overnight_summary::Summary`). */
export interface OvernightSummary {
  /** The evening's local date: the dismissal key. */
  night: string;
  from: string;
  to: string;
  ran: boolean;
  runs: number;
  conversations: number;
  succeeded: number;
  failed: Array<{ status: string; resultKind: string | null; failedChecks: number; at: string; sessionKey: string | null; sessionTitle: string | null; source: string }>;
  pullRequests: Array<{ repo: string | null; number: number; action: 'merged' | 'opened'; at: string; sessionKey: string; sessionTitle: string | null }>;
  commits: Array<{ repo: string; from: string; to: string; runs: number; at: string }>;
  waiting: { total: number; titles: string[] };
  report: string | null;
  /** Per handover chain (S06), costliest first; the card doesn't show them yet. */
  chains?: Array<{
    sessionKey: string; sessionTitle: string | null; chats: number; turns: number; stints?: number; merged: number;
    opened: number; costUsd: number; unknownCost: number; inProgress?: number; peakContextTokens: number | null;
    usdPerMergedPr: number | null;
  }>;
  /** The night's known cost (any row carrying one, partial runs included); `unknownCost` and `inProgress` rows add nothing to it. */
  costUsd?: number;
  /** Runs with a NULL-cost row, each once. */
  unknownCost?: number;
  /** NULL-cost rows with no run id (mostly Jev requests). */
  unknownCostCalls?: number;
  inProgress?: number;
  mergedPrs?: number;
  usdPerMergedPr?: number | null;
}

/** `settings_export` reply (Rust `settings_transfer::Exported`, overnight F7). */
export interface SettingsExported { path: string; count: number; skipped: string[] }
/** One change an import would make: shown values, old → new. */
/** `asks`: a confirm-tier key (what agents may do, what runs or spends on
 *  its own, accounts), listed first. */
export interface SettingsImportChange { key: string; summary: string; old: string; new: string; asks: boolean }
/** `settings_import_preview` reply; `hash` binds the apply to it. */
export interface SettingsImportPreview {
  changes: SettingsImportChange[];
  unchanged: number;
  ignored: Array<{ key: string; why: string }>;
  exportedAt: string | null;
  hash: string;
}
/** `settings_import_apply` reply. */
export interface SettingsImportApplied { applied: number; failed: Array<{ key: string; why: string }> }

/** `save_diagnostics` reply (Rust `diagnostics_bundle::Saved`). */
export interface DiagnosticsSaved {
  path: string;
  bytes: number;
  entries: number;
}

/** `export_feedback` reply. */
export interface FeedbackExport {
  jsonPath: string;
  markdownPath: string;
  count: number;
}

/** Jev ranking and duplicate-check settings, without the key (Rust `jev::Status`). */
export interface JevStatus { enabled: boolean; keyPresent: boolean; decisionsEnabled: boolean }
/** What the last Jev duplicate check did (Rust `jev_decide::Report`). */
/** One Jev duplicate check (Rust `jev_decide::Report`, generated): since
 *  agent effectiveness F11 (O3) it names its job and run and carries its Jev
 *  request; `jevDecisionsReport` answers the newest. */
export type { JevDecisionsReport } from './generated/ipc';
/** What the context prioritizer did for the last request (Rust `context_rank::Report`). */
/** A run's context manifest (agent effectiveness F11; Rust
 *  `run_manifest::RunContextView`, generated). Settings, Context shows the
 *  newest run's; `runContextManifest` reads any run's. */
export type { RunContextView, RunManifest } from './generated/ipc';

/** One agent's model choices (Rust `agent_models::ModelCatalog`). */
export interface AgentModelCatalog {
  /** `levels`: the effort levels a model takes when narrower than its agent's (Antigravity base models). */
  models: { id: string; label: string; levels?: string[] }[]; source: string; warning: string | null;
  /** When the agent last gave this list (RFC 3339), if known. */
  checkedAt: string | null;
  /** Claude only: each family's newest model (`sonnet` → `claude-sonnet-5-5`). */
  newestByFamily: Record<string, string>;
}

// ── Connected-project inventory (next-level P07) ────────────────────────
/** Mirrors `directory_connections::FileState`: what Hollow can do with a
 *  listed file. `tooLarge` is a text format over the 1 MB reader limit
 *  (listed, never read as empty); `extracted` is a PDF or Office file whose
 *  text Hollow reads itself (repo features R14a); `needsReader` is an image. */
export type ConnectedFileState = 'text' | 'extracted' | 'tooLarge' | 'needsReader';
/** Mirrors `directory_connections::Source`. */
export interface ConnectedFile {
  path: string;
  relative: string;
  projectId: string;
  size: number;
  readable: boolean;
  state: ConnectedFileState;
  /** Milliseconds since the epoch; 0 when unknown. */
  modified: number;
}
/** Mirrors `directory_connections::FilePage` (`connected_files`). */
export interface ConnectedFilePage {
  /** Listed files matching the query and filters. */
  total: number;
  offset: number;
  files: ConnectedFile[];
  /** The whole listed inventory, unfiltered. */
  indexed: number;
  /** Supported files counted but not listed. */
  omitted: number;
  /** First page of a query only: files a direct lookup found outside the
   *  listed inventory (a typed path, or a name in an over-share project). */
  direct: ConnectedFile[];
  refreshedAt: string;
}

// ── Page reviews (next-level P15; workspace_reviews.rs) ─────────────────
export interface ReviewSize { width: number; height: number }
/** One capture of a page: what it looked like at `capturedAt` in a
 *  `viewport` (CSS px). A capture, never a live view. */
export interface ReviewCapture {
  id: string; url: string; title: string; capturedAt: string;
  viewport: ReviewSize; imageSize: ReviewSize;
  /** The PNG, relative to the workspace folder. */
  image: string;
  /** The user's marks in the image's own pixels (annotate.ts `Mark`). */
  marks: unknown[];
  /** The capture this one was compared against. */
  parent: string | null;
  imageDataUrl: string | null;
}
export type ReviewVerdict = 'done' | 'notDone' | 'unclear';
/** A requested change (mark `mark` on `capture`) linked to the capture
 *  that shows its result, with the user's verdict. Newest per mark wins. */
export interface ReviewLink { capture: string; mark: number; result: string; status: ReviewVerdict; note: string; at: string }
export interface AnnotationSet {
  workspaceId: string; path: string; file: string; sha256: string; id: string; title: string;
  createdAt: string; updatedAt: string; captures: ReviewCapture[]; links: ReviewLink[];
}
export interface AnnotationSetSummary {
  path: string; file: string; id: string | null; title: string; updatedAt: string | null; url: string | null;
  captures: number; requested: number; done: number; error: string | null;
}
export interface NewReviewCapture {
  url: string; title: string; capturedAt: string; viewport: ReviewSize; imageSize: ReviewSize; marks: unknown[];
  /** Base64 PNG. */
  pngBase64: string;
}

/** Inline AI in the editor (overnight 10-11 B5, Rust `inline_ai::InlineAiInput`). */
export interface InlineAiInput {
  action: 'rewrite' | 'shorten' | 'expand' | 'fix' | 'translate' | 'ask';
  text: string;
  instruction?: string;
  context?: string;
  /** The editor's id for the request, so Dismiss can stop it (`inline_ai_cancel`). */
  requestId?: string;
}
export interface InlineAiResult { text: string }

/** The weekly review (overnight 10-11 A7, Rust `weekly_review`). */
export interface ReviewLine { text: string; path: string | null; sessionKey: string | null; day: string | null }
export interface WeeklyReview {
  week: string;
  monday: string;
  sunday: string;
  stats: { accepted: number; tasksDone: number; prsMerged: number; decisions: number; spendUsd: number | null };
  shipped: ReviewLine[];
  inProgress: ReviewLine[];
  slipped: ReviewLine[];
  decisions: ReviewLine[];
  kept: ReviewLine[];
  made: ReviewLine[];
  overdue: ReviewLine[];
  nextWeek: ReviewLine[];
  numbers: string[];
  spendByDay: { day: string; usd: number }[];
  spendByWorkspace: { workspace: string; title: string; usd: number }[];
  unpricedRuns: number;
}
export interface WeeklyTake { week: string; paragraph: string; priorities: { title: string; why: string; ask: string }[]; generatedAt: string }
export interface WeeklyReviewView { review: WeeklyReview; take: WeeklyTake | null; thisWeek: string }

/** Settings, Devices, Companion devices (companion apps T0, `companion_inbox.rs`). */
export interface CompanionDevice {
  deviceId: string;
  name: string;
  platform: string;
  publicKey: string;
  pairedAt: string;
  lastOpAt?: string | null;
}

export interface CompanionPairingCode {
  /** `XXXX-XXXX`, shown only here; Hollow keeps it in memory. */
  code: string;
  expiresAt: string;
}

/** Settings, Devices, Companion devices, Backend (companion apps W1, `companion_bridge.rs`). Never carries the token. */
export interface CompanionBackendStatus {
  url: string | null;
  connected: boolean;
  lastContactAt: string | null;
  lastError: string | null;
  /** The last mirror push that failed; changes still apply meanwhile. */
  mirrorError: string | null;
  mirroredTasks: number;
  lastPushAt: string | null;
  /** Documents the backend holds (v2 bridge); null until a v2 backend answered. */
  mirroredDocs: number | null;
  /** The last document push that failed; tasks and changes go on meanwhile. */
  docsError: string | null;
  /** False when the backend is too old for documents. */
  docsSupported: boolean | null;
  /** 'personal' or 'work', as last sent. */
  scope: string | null;
  /** scope.json couldn't be read. */
  scopeError: string | null;
}

/** What Settings, Devices, Companion devices, Test connection found (ST5, `companion_bridge.rs` `BackendTest`): one plain sentence. */
export interface CompanionBackendTest {
  ok: boolean;
  message: string;
  /** The backend's version, when it answered as one. */
  version: string | null;
}

/** One workspace in Settings, Devices, Companion devices, What the backend copies. */
export interface CompanionScopeWorkspace {
  id: string;
  title: string;
  status: string;
  /** Never copied in personal scope (a Notion project, `scope: work`, or a work name). */
  work: boolean;
  personal: boolean;
}

export interface CompanionScope {
  includeWorkData: boolean;
  seededAt: string | null;
  workspaces: CompanionScopeWorkspace[];
  /** scope.json couldn't be read: only tasks go up until it is saved again. */
  error: string | null;
}

export interface CompanionStatus {
  devices: CompanionDevice[];
  pairing: CompanionPairingCode | null;
  /** Ops waiting in `.hollow-sync/inbox/` (a run holds the vault). */
  waiting: number;
}

/** A phone paired with the phone chat API (`phone_access.rs`, docs/PHONE_CHAT_API.md). Never its token or the token's hash. */
export interface PhoneDevice {
  /** `ph-` and 8 hex digits. */
  deviceId: string;
  name: string;
  model: string | null;
  pairedAt: string;
  lastSeenAt: string | null;
  /** v1.1: the phone registered for push notifications (absent from an older Hollow). */
  push?: boolean;
  /** B4: the phone's approval keys, when it can answer cards (absent from an older Hollow). */
  approvals?: PhoneApprovals | null;
}

/** B4: a phone's approval keys, as Settings shows them. */
export interface PhoneApprovals {
  /** The 8 hex digits both screens show, `a1b2 c3d4`. */
  fingerprint: string;
  enrolledAt: string;
}

/** B4: the open approvals code window for one phone. */
export interface PhoneApprovalsCode {
  deviceId: string;
  code: string;
  expiresAt: string;
}

/** v1.1 push notifications (`phone_access/push.rs`), as Settings shows them. */
export interface PhonePush {
  /** `ready` (a key is imported), `noKey`, or `error` (the saved key can't be read; `lastError` says why). */
  state: 'ready' | 'noKey' | 'error';
  keyId: string | null;
  teamId: string | null;
  /** Paired phones with notifications on. */
  phones: number;
  lastSentAt: string | null;
  lastError: string | null;
  lastErrorAt: string | null;
}

/** The open pairing window, as Settings shows it. */
export interface PhonePairing {
  /** `XXXX-XXXX`, in memory only. */
  code: string;
  expiresAt: string;
  /** `hollow-companion://pair?url=…&code=…&mac=…`; null until Tailscale names this Mac. */
  link: string | null;
  /** The link as an SVG QR code (Rust's `qrcode` crate). */
  qrSvg: string | null;
}

/** Tailscale on this Mac, from `tailscale status --json` and `tailscale serve status --json`. */
export interface PhoneTailscale {
  installed: boolean;
  running: boolean;
  dnsName: string | null;
  login: string | null;
  /** Port 8443 proxies to the phone chat API on 127.0.0.1:47680. */
  published: boolean;
  /** Port 8443 is open to the internet (Funnel); Hollow never turns that on. */
  funnel: boolean;
  /** Port 8443 serves something else: its target. */
  other: string | null;
  /** Tailnet ports `tailscale serve --tcp` forwards straight to the phone chat API's port in the saved serve config. A
   *  raw forward passes on nothing of the caller, so Hollow can't tell its requests from this Mac's own: a problem Stop
   *  publishing removes. */
  tcpForwards: number[];
  /** The same from a `tailscale serve` still running in a terminal (started without --bg): only Control-C there ends
   *  it, so Stop publishing can't (third review, 2026-11-05). */
  foregroundForwards: number[];
  /** The same in a Tailscale service's config, each with the command that removes it when its name is plain. */
  serviceForwards: PhoneServiceForward[];
  error: string | null;
}

/** A raw TCP forward to the phone chat API in a Tailscale service's config. */
export interface PhoneServiceForward {
  /** `svc:<name>`. */
  service: string;
  port: number;
  /** `tailscale serve --service=<name> --tcp=<port> off`, for the person to run (the CLI's full path when no folder of
   *  their PATH holds `tailscale`); null for a name other than `svc:` and lower-case letters, digits and hyphens, which
   *  Hollow never puts in a command. */
  stop: string | null;
}

/** Settings, Devices, Companion devices, iPhone app (`phone_access_status`). */
export interface PhoneAccessStatus {
  /** False in a Hollow that doesn't run this vault's chats (another copy owns the run queue). */
  available: boolean;
  unavailableReason: string | null;
  enabled: boolean;
  listening: boolean;
  /** Why it isn't listening although on (the port is taken). */
  bindError: string | null;
  port: number;
  /** The record of paired phones couldn't be read. */
  recordError: string | null;
  tailscale: PhoneTailscale;
  /** `https://<MagicDNS name>:8443`, once Tailscale names this Mac. */
  baseUrl: string | null;
  macName: string;
  devices: PhoneDevice[];
  pairing: PhonePairing | null;
  /** v1.1: push notifications; null where this Hollow doesn't serve the iPhone app. */
  push?: PhonePush | null;
  /** M2: other Macs paired with this one (docs/phone-api/machines.md). */
  machines?: PhoneMachine[];
  /** M2: the open machine pairing window (Let another Mac connect). */
  machinePairing?: { code: string; expiresAt: string } | null;
  /** B4: the open approvals code window (Approvals from this iPhone), if any. */
  approvalsCode?: PhoneApprovalsCode | null;
  /** B4: seconds a card waits while a phone has approvals on. */
  approvalWaitSecs?: number;
  /** B4: phones whose keys were dropped for bad signatures: [device id, when]. */
  approvalsDropped?: [string, string][];
}

/** M2: another Mac paired with this one, as Settings lists it. */
export interface PhoneMachine {
  /** `mc-` and 8 hex digits: what Revoke takes. */
  deviceId: string;
  machineId: string | null;
  name: string;
  pairedAt: string;
  lastSeenAt: string | null;
}

/** Settings, ... , iPhone app → Keep Hollow running with the lid closed
 *  (`phone_lid_status`, `phone_access/lid.rs`). The words are `phoneLid.ts`'s. */
export interface PhoneLidStatus {
  /** False off macOS, or in a Hollow that doesn't serve the iPhone app. */
  available: boolean;
  unavailableReason: string | null;
  /** The helper is installed: the setting is on. */
  enabled: boolean;
  /** The installed helper's On battery too. */
  onBattery: boolean;
  /** The iPhone app is on. */
  iphoneOn: boolean;
  /** The iPhone app is on and a phone is paired. */
  ready: boolean;
  /** Hollow has armed the helper now. */
  armed: boolean;
  /** Hollow holds its idle-sleep assertion now. */
  holding: boolean;
  source: 'power' | 'battery' | 'unknown';
  batteryPercent: number | null;
  /** System sleep is disabled now; null when unread (with Hollow's marker,
   *  Settings counts that as off). */
  sleepDisabled: boolean | null;
  /** Hollow's helper turned system sleep off (its marker). */
  disabledByHollow: boolean;
  /** The lid is closed now. */
  lidClosed: boolean | null;
  /** The helper serves another copy of Hollow (that copy's path). */
  otherCopy: string | null;
  /** Seconds since the helper's last finished pass (root's
   *  `lid-closed-last-pass`); null when there is none. */
  lastPassSecs: number | null;
  /** Seconds since its restore check's last finished pass (root's
   *  `lid-closed-restore-last-pass`), the check that turns sleep back on
   *  unplugged; null when there is none. */
  lastRestoreSecs: number | null;
  /** Seconds since this Hollow stopped asking the helper for sleep off;
   *  null while it asks (armed, and on power or On battery too). */
  stoppedAskingSecs: number | null;
  /** The installed helper isn't the one this Hollow installs (a script or
   *  launchd keys differ, or its restore check or the helper itself is
   *  missing): Update the helper replaces both. */
  outdated: boolean;
}
