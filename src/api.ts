import type { SteerSupportRow, SteerWire } from './chatSteer';
import type { PauseReason } from './chatStop';
import type { PrView } from './prPanelModel';
import type { GhDetailView, GhList, GhQuery, Triage, TriageDone } from './githubModel';
import type { CardGitBranches, CardGitFile, CardGitStatus } from './cardGitModel';
import type { CannedRequest, InlineAiInput, InlineAiResult } from './types';
import type { AgendaEventsView, CalendarSettings, CalendarSourceStatus } from './agendaTypes';
import type { RecordingDetail, RecordingLink, RecordingList } from './recordingTypes';
import type { DecisionRecord } from './recordTypes';
import type { RunnerCloudEstimate, RunnerSetupParams, RunnerSetupPlan, RunnerSetupStepState } from './runnerSetup';
import type { ProjectScript } from './projectScripts';
import type { AppBuildStatus, BuildDiff, BuildStatus, SelfInstallReceipt, SelfInstallResult, ServiceView, WorkResult, ResultReview, LibraryResult, CreatedBuildWorkspace, RailGitState, RepoCandidate, ScratchInfo, AdoptReport, ClonedRepo, WorkWorkspace, WorkspaceDetail, WorkspaceIcon, WorkspaceSessions, NeedsYouItem, NeedsYouPage, SessionSearchPage, SessionTarget } from './workspaceTypes';
import { invoke as tauriInvoke } from '@tauri-apps/api/core';
import type { ConnectedFilePage, ConnectedFileState, ConnectedIndexStatus, PortabilityTable } from './types';
import type { AgentToolPath, BuildInfo, CanvasFrequency, LearningBatchOutcome, SelfHostedEntry, SelfHostedJobKind, SelfHostedJobs, SelfHostedModels, SelfHostedRetry, SelfHostedTest, SelfHostedWake, SelfHostedWakeStatus } from './generated/ipc';
import type { ConnectorInput, ConnectorStatus, ConnectorView, ConnectorsLibraryView, M365Status, PlaudMcpStatus, RemoteStatus } from './connectorsLibrary';
import type {
  AgentSettings, AgentDescriptor, BacklinkSource, CreatedTask, CustomTheme, CustomThemeMeta,
  Dashboard, Diagnostics, DiagnosticsSaved, DiskReport, OnePasswordStatus, OvernightSummary, RunnerAccess, RunnerGithubStatus, RunnerStatus,
  InstallTargetsPlan, InstallTargetsPruned, WorktreeHygieneReport, WorktreesCleaned,
  HollowDatabases, StaleIndexesPlan, StaleIndexesRemoved,
  SettingsExported, SettingsImportApplied, SettingsImportPreview,
  IngestOpts, IngestResult, IngestSource,
  Notification, NotificationInput, OperatorContext, OperatorFreshness,
  ProfileBuilderView,
  NoteContent, NoteRef, PreviewFile, SavedFile, ChatPreviewFile, PdfPreviewPage, MediaGrant, VaultGraph, VaultItem, VaultKind, SaveNoteResult,
  Project, SearchResult, SemanticStatus, ParsedQuickAdd, AssistInput, AssistResult,
  HistoryRun, HistoryStorageStatus, RepoSidecar,
  LearningProposal, LearningContext,
  ReviewInboxItem, ReviewItemKind, ReviewItemStatus, ActivationInventory, CompanionStatus, CompanionPairingCode, CompanionBackendStatus, CompanionBackendTest, CompanionScope, PhoneAccessStatus,
  Commitment, WeeklyReviewView,
  SetupStatus, ProfileNotesResult,
  WorkspaceSourceUpdate, WorkspaceSyncSummary, CreateTaskFields, UpdateTaskFields, UpdatedTask,
  ArtifactSessionLink, ArtifactSessionContext,
  AgentConversation, AgentSessionSummary, AgentSessionDetail, LiveAgentSession, SessionMove,
  UsageSummary, UsageWindow, ClaudeQuota, ModelPrice, ModelPriceRow,
  ClaudeAccountsView, ClaudeAccountUpdate, ClaudeAccountRouting, ProviderAccount,
  BudgetsView, BudgetInput, BudgetDef,
  SendAccepted, ChatInteraction, ResolvedInteraction, InteractionAnswer, Skill, TasksResponse, VaultTree,
  Workflow, WorkflowDetail, WorkflowInput, WorkflowRunResult, WorkflowOutput, RunQueueItem,
  HeartbeatConfig, LedgerEntry, BackupHealth,
  JevStatus, RunContextView, JevDecisionsReport,
} from './types';
import type { PhoneLidStatus } from './types';
import type { AgentDeniedHost, ComputerChatView, ComputerCheck, ComputerStatus, DeniedApp } from './types';
import type { ChainView, SubagentsView } from './generated/ipc';
import type { EvidenceReceiptWire } from './executionReceipt';
import type { OperatorSwitchState } from './operatorSwitch';
import { MOCK_BUILD, isBrowserMockMode } from './mockMode';
import { noteIpc } from './launchIpc';
import type { PageInspection } from './annotate';
import type { PageText } from './sharedPage';
import type { ShortcutSettings, ShortcutsView } from './shortcuts';

/** Where a browser tab sits in the main window, in CSS pixels. */
export interface BrowserBounds { x: number; y: number; width: number; height: number }

// Thin typed wrappers around Tauri invoke. Field shapes are guaranteed
// by the Rust side (#[derive(Serialize)] with rename_all = "camelCase").
// The browser mock loads only in a dev or `--mode mock` build: `MOCK_BUILD` is
// the literal `false` in production, so the import and the fixtures behind it
// never reach the shipped bundle (see mockMode.ts). The load starts as soon as
// this module does; once it lands, calls go straight to `mockInvoke`, so its
// synchronous side effects (call counts, `mockBuildLog`, localStorage writes)
// happen inside the call, as they did when the import was static. Smoke specs
// read that state right after the UI settles, and an `import().then` hop per
// call made them race it.
type MockApi = typeof import('./mockApi');
let mockApi: MockApi | undefined;
const mockApiLoad: Promise<MockApi> | undefined = MOCK_BUILD && isBrowserMockMode()
  ? import('./mockApi').then(m => (mockApi = m))
  : undefined;
const rawInvoke = mockApiLoad
  ? <T>(command: string, args?: Record<string, unknown>): Promise<T> =>
      mockApi ? mockApi.mockInvoke<T>(command, args) : mockApiLoad.then(m => m.mockInvoke<T>(command, args))
  : tauriInvoke;
/** Every call is tallied for the launch IPC budget. Exported for lazy API modules;
 * components use typed wrappers, never this function directly. */
export const invoke = <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
  noteIpc(command, args);
  return rawInvoke<T>(command, args);
};

export interface DirectoryConnection {
  /** `refreshedAt` is the last SUCCESSFUL refresh. */
  version: number; pendingRestart: boolean; homePath: string; projectsPath: string; vaultPath: string; refreshedAt: string;
  projects: {id: string; path: string; title: string}[];
  /** The file inventory no longer travels here (next-level P07): page it with `api.connectedFiles`. */
  warnings: string[];
  /** Structured coverage (audit H03); absent in a manifest written before it. */
  coverage?: SourceCoverage;
}
export interface BuildIdentity { version: string; commit: string; shortCommit: string; dirty: boolean; generatedAt: string; sourceFingerprint: string; inputCount: number; text: string }
export interface SourceCoverage {
  projects: { id: string; title: string; indexed: number; found: number; capped: boolean; folders: { relative: string; files: number }[];
    /** P07 (absent in an older manifest): seen but not listed, and listed but unread. */
    omitted?: number; tooLarge?: number; needsReader?: number; extracted?: number }[];
  /** The walk stopped at its entry limit: counts are lower bounds. */
  scanLimited: boolean; maxFiles: number; maxProjectFiles: number;
  /** Folders Hollow leaves out, relative to the Projects folder. */
  excluded: string[];
  /** P07 totals (absent in an older manifest): listed, seen, seen-not-listed, listed-not-read. */
  indexed?: number; available?: number; omitted?: number; tooLarge?: number; needsReader?: number;
  /** R14a: listed PDF and Office files Hollow reads (absent before it). */
  extracted?: number;
  /** The listed inventory reached its safety bound. */
  inventoryLimited?: boolean;
  /** When the listed inventory last changed. */
  changedAt?: string;
}
/** Which canvas store (Canvas C3): a workspace, or a conversation outside one. */
export interface CanvasScopeArgs { workspaceId: string | null; sessionKey: string | null }
const scopeArgs = (scope: CanvasScopeArgs) => ({ workspaceId: scope.workspaceId, sessionKey: scope.sessionKey });
/** Self-hosted models Q6: Settings' save, switch and remove of an entry,
 *  told to the chats' start chip (`store/selfHostedWake.ts`), which re-reads
 *  the entries and starts that entry's wake state over, mounted or not. */
const selfHostedListeners = new Set<(id: string) => void>();
export function onSelfHostedChange(listener: (id: string) => void): () => void {
  selfHostedListeners.add(listener);
  return () => { selfHostedListeners.delete(listener); };
}
function selfHostedChanged<T>(id: string, call: Promise<T>): Promise<T> {
  return call.then(value => { for (const listener of selfHostedListeners) listener(id); return value; });
}

/** A paused chat as `chat_pause::PausedChat` serializes it (P1b, P1c). */
export interface PausedChatRecord { sessionKey: string; nativeSessionId: string; agent: string; pausedAt: number; note: string; reason: PauseReason; resumeAt: number | null }

/** CP1 (`turn_checkpoints.rs`): one turn's checkpoint pair and what it changed. */
export interface TurnCheckpoint { ord: number; runId: string | null; before: string; after: string | null; files: string[]; mergedIn: string[] }
export interface TurnCheckpoints { isolated: boolean; reason: string | null; turns: TurnCheckpoint[] }
export interface RestoredCheckpoint { ord: number; through: number; restored: string[]; removed: string[]; branchResetTo: string | null; safetyRef: string; text: string }

/** H5a (`session_import`): Claude Code, Codex and Pi conversations from their folders. */
export type ImportSourceId = 'claude-code' | 'codex' | 'chatgpt' | 'claudeai' | 'gemini' | 'pi';
export type ImportStatusId = 'new' | 'alreadyImported' | 'updated' | 'newVersion' | 'hollowOwn' | 'subagent' | 'unsupported';
export interface ImportSource { source: ImportSourceId; label: string; defaultRoot: string | null; /** An export file the person picks (ChatGPT, Gemini): no default folder. */ export: boolean; exists: boolean }
export interface ImportEntry { id: string; externalId: string; title: string; createdAt: string | null; lastAt: string | null; messageCount: number; turnCount: number; size: number; projectFolder: string | null; destination: string; status: ImportStatusId; version?: number; existingKey?: string; selected: boolean; sidechainLines: number; malformedLines: number; truncated: boolean; shortenedTurns: number; contentHash: string; /** ChatGPT: branches of the conversation not imported. */ otherBranches?: number; /** Gemini: "Grouped by time". */ grouping?: string }
export interface ImportManifest { importId: string; source: ImportSourceId; sourceLabel: string; root: string; createdAt: string; state: string; conversations: ImportEntry[]; counts: Record<'new' | 'alreadyImported' | 'updated' | 'newVersion' | 'hollowOwn' | 'subagent' | 'unsupported', number>; foundFiles: number; capped: boolean; stopped?: string; symlinksSkipped: number; unreadable: number; unsupported?: string; note?: string; export?: ImportExportInfo }
export interface ImportExportInfo { name: string; fingerprint: string; shards: number; malformed: number; duplicates: number; damaged: string[]; otherFiles: string[]; otherFileCount: number; nestedSkipped: number; linksSkipped: number }
export interface ImportJob { importId: string; source: ImportSourceId; state: string; done: number; total: number; error?: string }
export interface ImportBatch { importId: string; source: string; conversations: number; importedAt: string; keepOutOfSearch: boolean }
export interface ImportPreview { id: string; title: string; turns: { at: string | null; asked: string; reply: string; tools: string[] }[]; totalTurns: number; sidechainLines: number; otherBranches?: number; /** Gemini: "Grouped by time". */ grouping?: string }
export interface ImportCommitResult { importId: string; imported: number; updated: number; newVersions: number; alreadyImported: number; skipped: number; failed: { id: string; title: string; error: string }[]; sessionKeys: string[]; stagingDeleted: boolean }
export interface NativeResumeOffer { available: boolean; reason: string | null; cliVersion: string | null; cwd: string | null }
export interface ImportContinuedNative extends ImportContinued { resumeSessionId: string }
export interface ImportContinued { key: string; title: string; parentKey: string; agent: string; model: string | null; workspaceId: string; cwd: string; mode: string; effort: string | null }

export const api = {
  readCustomIcon: (target: import('./customIcons').IconTarget, includeOriginal = false) => invoke<import('./customIcons').CustomIcon | null>('read_custom_icon', { target, includeOriginal }),
  prepareCustomIcon: (original: import('./customIcons').IconOriginal) => invoke<import('./customIcons').PreparedIcon>('prepare_custom_icon', { original }),
  saveCustomIcon: (target: import('./customIcons').IconTarget, original: import('./customIcons').IconOriginal, rendered: string, editor: typeof import('./customIcons').UPLOAD_EDITOR, expected: string | null) => invoke<import('./customIcons').CustomIcon | null>('save_custom_icon', { target, original, rendered, editor, expected }),
  resetCustomIcon: (target: import('./customIcons').IconTarget, expected: string | null) => invoke<import('./customIcons').CustomIcon | null>('reset_custom_icon', { target, expected }),
  selectAppIconPreset: (preset: import('./appIconPresets').AppIconPreset, expected: string | null) => invoke<import('./customIcons').CustomIcon | null>('select_app_icon_preset', { preset, expected }),
  /** Generate a logo (overhaul H2): one tool-free call on `agent` for up to four checked SVG options. Writes nothing. */
  generateLogoOptions: (agent: string, target: import('./customIcons').IconTarget, request: string, requestId?: string) => invoke<import('./logoGenerate').LogoOptions>('generate_logo_options', { agent, target, request, requestId }),
  /** Closing the icon dialog: stop that Generate a logo call. `true` when it was still running. */
  cancelLogoOptions: (requestId: string) => invoke<boolean>('cancel_logo_options', { requestId }),
  canvasExport: (request: import('./canvas/canvasExport').CanvasExportRequest) => invoke<import('./canvas/canvasExport').ExportedFile>('canvas_export', { request }),
  /** Repo ideas 2026-10-27 #4: Measure on the picture's two captures. */
  canvasMeasure: (request: import('./canvas/canvasExport').CanvasMeasureRequest) => invoke<import('./canvas/canvasExport').CanvasMeasureCaptures>('canvas_measure', { request }),
  directoriesStatus: () => invoke<DirectoryConnection>('directories_status'),
  /** The build this binary was compiled from (version, commit, fingerprint). */
  buildIdentity: () => invoke<BuildIdentity>('build_identity'),
  connectHollowHome: (homePath: string, existingProjects: string, existingVault: string) => invoke<DirectoryConnection>('connect_hollow_home', {homePath, existingProjects, existingVault}),
  refreshDirectories: () => invoke<DirectoryConnection>('refresh_directories'),
  /** Hollow-side exclusion: leave a Projects-folder folder out of the index (or bring it back) and rescan. */
  setDirectoryExclusion: (relative: string, excluded: boolean) => invoke<DirectoryConnection>('set_directory_exclusion', { relative, excluded }),
  readConnectedFile: (path: string) => invoke<string>('read_connected_file', {path}),
  /** The workspace to open for a connected project (overnight 10-11 Y1): made on first open. */
  openProjectWorkspace: (id: string) => invoke<string>('open_project_workspace', { id }),
  /** One page of the connected-project inventory (P07): every word of `query` in the path, name matches first. */
  connectedFiles: (opts: { query?: string; projectId?: string; fileState?: ConnectedFileState; offset?: number; limit?: number } = {}) => invoke<ConnectedFilePage>('connected_files', opts),
  /** Repo features R14b: the project content index's switch and progress. */
  connectedIndexStatus: () => invoke<ConnectedIndexStatus>('connected_index_status'),
  /** Off deletes the index; on rebuilds it in the background. */
  setConnectedIndexEnabled: (enabled: boolean) => invoke<ConnectedIndexStatus>('set_connected_index_enabled', { enabled }),
  readWorkFile: (path: string) => invoke<NoteContent>('read_work_file', { path }),
  listWorkspaces: () => invoke<WorkWorkspace[]>('list_workspaces'),
  workspaceIcons: () => invoke<WorkspaceIcon[]>('workspace_icons'),
  /** `refresh` re-reads the claude.ai connector list (otherwise a five-minute copy). */
  listConnectors: (refresh = false) => invoke<ConnectorsLibraryView>('list_connectors', { refresh }),
  saveConnector: (input: ConnectorInput) => invoke<ConnectorView>('save_connector', { input }),
  /** Sign-in status of the library's claude.ai connectors (asks Anthropic's proxy). */
  checkConnectors: () => invoke<ConnectorStatus[]>('check_connectors'),
  m365SignIn: (calendarOnly = false) => invoke<M365Status>('m365_sign_in', { calendarOnly }),
  m365SignOut: () => invoke<void>('m365_sign_out'),
  m365Status: () => invoke<M365Status>('m365_status'),
  /** The built-in Plaud connector: Plaud's MCP server's own sign-in (its `login` tool opens the browser; waits up to 5 minutes). */
  plaudMcpSignIn: () => invoke<PlaudMcpStatus>('plaud_mcp_sign_in'),
  plaudMcpSignOut: () => invoke<PlaudMcpStatus>('plaud_mcp_sign_out'),
  deleteConnector: (id: string) => invoke<void>('delete_connector', { id }),
  importConnector: (source: string, name: string) => invoke<ConnectorView>('import_connector', { source, name }),
  /** Add Hollow's own sign-in for a claude.ai connector (five asks B2). */
  connectorUseHollowSignIn: (id: string) => invoke<ConnectorView>('connector_use_hollow_sign_in', { id }),
  /** Sign a remote connector in with Hollow's own sign-in (opens the browser, waits up to 5 minutes). */
  remoteConnectorSignIn: (id: string) => invoke<RemoteStatus>('remote_connector_sign_in', { id }),
  remoteConnectorSignOut: (id: string) => invoke<void>('remote_connector_sign_out', { id }),
  jevStatus: () => invoke<JevStatus>('jev_status'),
  saveJevSettings: (enabled: boolean, apiKey: string | null, decisionsEnabled?: boolean) => invoke<JevStatus>('save_jev_settings', { enabled, apiKey, decisionsEnabled }),
  /** The newest Jev duplicate check, or with `runId` that run's (F11, O3). */
  jevDecisionsReport: (runId?: string | null) => invoke<JevDecisionsReport | null>('jev_decisions_report', { runId: runId ?? null }),
  // Saved chat canvases (Canvas C3). `scope` names the store: a workspace's
  // outputs/canvases/ (its sessionKey is recorded as where a revision came
  // from), or a conversation's app-data store when workspaceId is null.
  listCanvases: (scope: CanvasScopeArgs) => invoke<import('./generated/ipc').CanvasSummary[]>('list_canvases', scopeArgs(scope)),
  readCanvas: (scope: CanvasScopeArgs, id: string) => invoke<import('./generated/ipc').Canvas | null>('read_canvas', { ...scopeArgs(scope), id }),
  readCanvasRevision: (scope: CanvasScopeArgs, id: string, revision: number) => invoke<import('./generated/ipc').CanvasRevisionHtml>('read_canvas_revision', { ...scopeArgs(scope), id, revision }),
  /** Save a finished hollow-canvas fence body. Refused with `CANVAS_CHANGED:` when the sidecar is no longer `expectedSha256` (null: none was read). A body that is a kept revision is `unchanged`, one that aged out of the kept 30 is `retired` (nothing written; render the fence itself), and a body of a pruned conversation outside a workspace is refused with `CANVAS_RETIRED:` (nothing written). */
  recordCanvas: (scope: CanvasScopeArgs, body: string, expectedSha256: string | null, opts?: { turnId?: string | null; kit?: string | null }) => invoke<import('./generated/ipc').CanvasRecorded>('record_canvas', { ...scopeArgs(scope), body, turnId: opts?.turnId ?? null, kit: opts?.kit ?? null, expectedSha256 }),
  /** The canvas's UI state (≤16 KB JSON; null clears). Refused with `CANVAS_CHANGED:` when stale. */
  saveCanvasState: (scope: CanvasScopeArgs, id: string, canvasState: unknown, expectedSha256: string) => invoke<import('./generated/ipc').CanvasStateSaved>('save_canvas_state', { ...scopeArgs(scope), id, canvasState, expectedSha256 }),
  /** Log a bridge message a canvas frame's host refused (Canvas C6 P1): kind, size, reason, ids; never the payload. Bounded app-data file, rate-limited. */
  recordCanvasBridgeRefusal: (refusal: import('./canvasBridge').BridgeRefusal) => invoke<void>('record_canvas_bridge_refusal', { refusal }),
  // Page reviews (next-level P15): saved Annotate sets in outputs/reviews/.
  listAnnotationSets: (id: string) => invoke<import('./types').AnnotationSetSummary[]>('list_annotation_sets', { id }),
  readAnnotationSet: (id: string, path: string) => invoke<import('./types').AnnotationSet>('read_annotation_set', { id, path }),
  /** A new outputs/reviews/<slug>.annotations.json (+ its capture PNG); never replaces a file. */
  saveAnnotationSet: (id: string, input: { title?: string; capture: import('./types').NewReviewCapture }) => invoke<import('./types').AnnotationSet>('save_annotation_set', { id, input }),
  /** Refused with `REVIEW_CHANGED:` when the file is no longer `expectedSha256`. */
  addAnnotationCapture: (id: string, path: string, expectedSha256: string, capture: import('./types').NewReviewCapture, links: { capture: string; mark: number; status: import('./types').ReviewVerdict; note?: string }[]) =>
    invoke<import('./types').AnnotationSet>('add_annotation_capture', { id, path, expectedSha256, capture, links }),
  removeJevKey: () => invoke<JevStatus>('remove_jev_key'),
  testJevKey: () => invoke<void>('test_jev_key'),
  /** A run's context manifest (agent effectiveness F11): with `runId`, that
   *  run's or none; else with `sessionKey`, that conversation's newest or
   *  none; with neither, the newest run's. Sizes, hashes and decisions only,
   *  never prompt text; `null` when there is none. */
  /** A turn's steps (master plan F6 step spine), by its run id. */
  runSteps: (runId: string) => invoke<import('./flightRecorder').StepsView>('run_steps', { runId, session: null, turn: null }),
  /** Retry from after `step` (0: the start): a new conversation, files put
   *  back through History first when needed (master plan F6/#15). */
  retryRunFromStep: (runId: string, step: number, mode?: string) =>
    invoke<{ sessionKey: string; queueId: number; restoredBy: string | null; filesPutBack: number; mode: string }>('retry_run_from_step', { runId, step, mode: mode ?? null }),
  runContextManifest: (runId?: string | null, sessionKey?: string | null) =>
    invoke<RunContextView | null>('run_context_manifest', { runId: runId ?? null, sessionKey: sessionKey ?? null }),
  createWorkspace: (title: string, goal: string) => invoke<WorkWorkspace>('create_workspace', { title, goal }),
  /** The built-in Quick chats workspace, created on first use. */
  ensureQuickChatsWorkspace: () => invoke<WorkWorkspace>('ensure_quick_chats_workspace'),
  ensureMainAgentWorkspace: () => invoke<WorkWorkspace>('ensure_main_agent_workspace'),
  /** A person restores defaults only when no standing-orders file exists. */
  createMainStandingOrders: () => invoke<boolean>('create_main_standing_orders'),
  mainAgentChildren: () => invoke<import('./mainAgent').MainAgentChild[]>('main_agent_children'),
  mainAgentStatus: () => invoke<import('./mainAgent').MainAgentStatus>('main_agent_status'),
  mainAgentActivity: () => invoke<import('./mainAgent').MainAgentAct[]>('main_agent_activity'),
  automationWindows: () => invoke<import('./generated/ipc').AutomationWindows>('automation_windows'),
  /** Save the named windows and the overnight cap (Settings, Background work
   *  windows, overnight S10). User-only: never an agent action. */
  setAutomationWindows: (settings: import('./generated/ipc').WindowSettings) => invoke<import('./generated/ipc').AutomationWindows>('set_automation_windows', { settings }),
  /** Settings, Advanced, Extraction model (overnight A11). */
  getExtractionSettings: () => invoke<import('./generated/ipc').ExtractionView>('get_extraction_settings'),
  setExtractionSettings: (args: { agent: string; model: string }) => invoke<import('./generated/ipc').ExtractionView>('set_extraction_settings', args),
  /** Settings, Chat, Retitle new chats (morning N5). */
  setChatRetitle: (enabled: boolean) => invoke<import('./generated/ipc').ExtractionView>('set_chat_retitle', { enabled }),
  // Settings, Memory, Learning suggestions (fewer learnings, 10-11).
  getLearningSettings: () => invoke<import('./generated/ipc').LearningSettings>('get_learning_settings'),
  setLearningSuggestions: (mode: import('./generated/ipc').Suggestions) => invoke<import('./generated/ipc').LearningSettings>('set_learning_suggestions', { mode }),
  /** Conflict copies (overnight A10): the list, one pair's texts and hashes, and the person's keep-one. */
  conflictCopies: () => invoke<import('./generated/ipc').ConflictCopy[]>('conflict_copies'),
  conflictCopyPair: (copy: string) => invoke<import('./generated/ipc').ConflictPair>('conflict_copy_pair', { copy }),
  resolveConflictCopy: (args: { copy: string; keepCopy: boolean; copyHash: string; originalHash: string }) => invoke<string>('resolve_conflict_copy', args),
  dismissConflictCopy: (copy: string) => invoke<string>('dismiss_conflict_copy', { copy }),
  undismissConflictCopy: (copy: string) => invoke<void>('undismiss_conflict_copy', { copy }),
  /** "Delete the memory X?" (build run L2): the person's Delete or Keep, and a Delete's Undo. */
  answerLessonMemory: (sourceId: string, del: boolean) => invoke<string>('answer_lesson_memory', { sourceId, delete: del }),
  restoreLessonMemory: (sourceId: string) => invoke<string>('restore_lesson_memory', { sourceId }),
  getMainAgentSettings: () => invoke<import('./mainAgent').MainAgentSettings>('get_main_agent_settings'),
  setMainAgentSettings: async (settings: import('./mainAgent').MainAgentSettings) => {
    const saved = await invoke<import('./mainAgent').MainAgentSettings>('set_main_agent_settings', { settings });
    if (typeof window !== 'undefined') window.dispatchEvent(new Event('hollow-main-agent-settings'));
    return saved;
  },
  /** Settings, Chat, how many chats run at once (2026-10-20). */
  getChatLimits: () => invoke<import('./types').ChatLimits>('get_chat_limits'),
  setChatLimits: (limits: import('./types').ChatLimits) => invoke<import('./types').ChatLimits>('set_chat_limits', { limits }),
  /** A workspace with its own git worktree and branch in `repo`; its conversations run there. */
  createBuildWorkspace: (input: { title: string; goal: string; repo: string; branch?: string; base?: string }) => invoke<CreatedBuildWorkspace>('create_build_workspace', { input }),
  /** Remove a build workspace's worktree (the branch is kept). Without `force`, unsaved work rejects with `WORKTREE_UNSAVED:`. */
  removeBuildWorktree: (id: string, force: boolean) => invoke<void>('remove_build_worktree', { id, force }),
  buildRepoCandidates: () => invoke<RepoCandidate[]>('build_repo_candidates'),
  /** A quick chat's scratch folder; `create` makes it first (4.4). */
  quickChatScratch: (key: string, create: boolean) => invoke<ScratchInfo>('quick_chat_scratch', { key, create }),
  /** Turn into project: the chat's scratch files move into the project's sources/chat-<key>. */
  adoptChatScratch: (key: string, workspaceId: string) => invoke<AdoptReport>('adopt_chat_scratch', { key, workspaceId }),
  /** Clone an https or ssh address into the Projects folder (4.4). */
  cloneRepository: (url: string, name?: string) => invoke<ClonedRepo>('clone_repository', { url, name }),
  /** A project's named scripts (4.5, A10). */
  listProjectScripts: (workspaceId: string) => invoke<ProjectScript[]>('list_project_scripts', { workspaceId }),
  /** Replace a project's scripts; the backend validates every field and answers what it saved. */
  saveProjectScripts: (workspaceId: string, scripts: ProjectScript[]) => invoke<ProjectScript[]>('save_project_scripts', { workspaceId, scripts: scripts.map(({ id, name, command, chord }) => ({ id, name, command, chord: chord || undefined })) }),
  /** Run one script (the person's click or chord): it starts as a service in the project's folder, if its command is still the one shown. */
  runProjectScript: (workspaceId: string, scriptId: string, expectedCommand: string) => invoke<ServiceView>('run_project_script', { workspaceId, scriptId, expectedCommand }),
  /** Changes pane (Build PR B2): the worktree's branch against its base. */
  buildStatus: (id: string) => invoke<BuildStatus>('build_status', { id }),
  /** Unified diff against the merge-base, for one file or all (redacted, capped). */
  buildDiff: (id: string, path?: string) => invoke<BuildDiff>('build_diff', { id, path: path ?? null }),
  /** `git add -A` + commit. Refused while an agent run holds the vault. */
  buildCommit: (id: string, message: string) => invoke<BuildStatus>('build_commit', { id, message }),
  buildPush: (id: string) => invoke<BuildStatus>('build_push', { id }),
  /** `gh pr create`, pushing first when the branch has no upstream; the PR URL. */
  buildOpenPr: (id: string, title: string, body: string) => invoke<string>('build_open_pr', { id, title, body }),
  /** Runs on (the remote runner): the build workspace's own choice, or null to follow Settings. */
  buildSetRunsOn: (id: string, runsOn: 'this-mac' | 'runner' | null) => invoke<BuildInfo>('build_set_runs_on', { id, runsOn }),
  /** The remote runner Runs on's runner choice uses, or null when none is set up. */
  remoteRunner: () => invoke<string | null>('remote_runner'),
  /** The runner's state (overnight N1): `fresh` checks again over ssh (up to 30 s), else the last check's answer. */
  runnerStatus: (fresh = false) => invoke<RunnerStatus>('runner_status', { fresh }),
  /** Remote runners 2: name this Mac's runner (null turns it off); the runner now in use. */
  runnerSet: (host: string | null) => invoke<string | null>('runner_set', { host }),
  /** The read-only check of a typed SSH host before it is named the runner. */
  runnerCheckHost: (host: string) => invoke<RunnerStatus>('runner_check_host', { host }),
  /** Record this Hollow as the runner's owner there; the fresh status. */
  runnerRecordOwner: () => invoke<RunnerStatus>('runner_record_owner'),
  /** Remote runners 3: Set up's steps for the typed fields (defaults filled, what isn't valid, each step's commands). */
  runnerSetupPlan: (params: RunnerSetupParams) => invoke<RunnerSetupPlan>('runner_setup_plan', { params }),
  /** Set up → Check: every step's read-only check, in order. */
  runnerSetupCheck: (params: RunnerSetupParams) => invoke<RunnerSetupStepState[]>('runner_setup_check', { params }),
  /** Set up → a step's Apply (after its confirm), then its check. */
  runnerSetupApply: (params: RunnerSetupParams, step: string) => invoke<RunnerSetupStepState>('runner_setup_apply', { params, step }),
  /** The Google Cloud machine's monthly estimate. */
  runnerCloudEstimate: (params: RunnerSetupParams) => invoke<RunnerCloudEstimate>('runner_cloud_estimate', { params }),
  /** Create the Google Cloud machine; runs nothing unless `estimateCents` is the estimate for these fields. */
  runnerCloudCreate: (params: RunnerSetupParams, estimateCents: number) => invoke<RunnerSetupStepState>('runner_cloud_create', { params, estimateCents }),
  workspaceRunsOn: (id: string) => invoke<string | null>('workspace_runs_on', { id }),
  workspaceSetRunsOn: (id: string, runsOn: 'this-mac' | 'runner' | null) => invoke<string | null>('workspace_set_runs_on', { id, runsOn }),
  /** Remote runners (2026-11-03): what a workspace's runner turns may reach (GitHub, Google Cloud). */
  runnerAccess: (id: string) => invoke<RunnerAccess>('runner_access_get', { id }),
  runnerAccessSet: (id: string, access: RunnerAccess) => invoke<RunnerAccess>('runner_access_set', { id, access }),
  /** Settings, Devices, Remote runner, GitHub: the credential's source; never the token. */
  runnerGithubStatus: () => invoke<RunnerGithubStatus>('runner_github_status'),
  runnerGithubSaveToken: (token: string) => invoke<RunnerGithubStatus>('runner_github_save_token', { token }),
  runnerGithubClearToken: () => invoke<RunnerGithubStatus>('runner_github_clear_token'),
  runnerGithubSetSource: (source: RunnerGithubStatus['source']) => invoke<RunnerGithubStatus>('runner_github_set_source', { source }),
  /** 1Password as an optional second secret path (parity row 7.5). Status runs nothing; check runs op. */
  onePasswordStatus: () => invoke<OnePasswordStatus>('onepassword_status'),
  onePasswordCheck: () => invoke<OnePasswordStatus>('onepassword_check'),
  onePasswordSetEnabled: (enabled: boolean) => invoke<OnePasswordStatus>('onepassword_set_enabled', { enabled }),
  onePasswordSetReference: (key: string, reference: string) => invoke<OnePasswordStatus>('onepassword_set_reference', { key, reference }),
  onePasswordClearReference: (key: string) => invoke<OnePasswordStatus>('onepassword_clear_reference', { key }),
  /** Update from <base> (P1-46): fetch and rebase; a conflict aborts. */
  buildUpdateFromBase: (id: string) => invoke<BuildStatus>('build_update_from_base', { id }),
  /** Install from Hollow (Build PR B5): the app build of a Hollow worktree. */
  appBuildStatus: (id: string) => invoke<AppBuildStatus>('app_build_status', { id }),
  appBuildBrings: (id: string) => invoke<import('./workspaceTypes').InstallBringsReport>('app_build_brings', { id }),
  /** `npm run tauri:bundle` as a background job; progress on `self-install-event`. */
  startAppBuild: (id: string) => invoke<AppBuildStatus>('start_app_build', { id }),
  cancelAppBuild: (id: string) => invoke<void>('cancel_app_build', { id }),
  selfInstallResult: () => invoke<SelfInstallResult | null>('self_install_result'),
  exitRunningWork: () => invoke<import('./workspaceTypes').RunningWork[]>('exit_running_work'),
  pendingQuitRequest: () => invoke<import('./workspaceTypes').QuitRequest | null>('pending_quit_request'),
  resolveQuit: (id: string, choice: 'quit' | 'tray' | 'cancel') => invoke<import('./workspaceTypes').QuitRequest | null>('resolve_quit', { id, choice }),
  selfInstallReceipt: () => invoke<SelfInstallReceipt | null>('self_install_receipt'),
  // Updates (row U5, update_client): every change also arrives as
  // onNative<UpdateState>('update-state', …); the after-restart notice as
  // onNative<UpdateLastResult>('update-last-result', …). The token goes in
  // once (updateConnect) and is never returned.
  updateGetState: () => invoke<import('./generated/ipc').UpdateState>('update_get_state'),
  updateGetSettings: () => invoke<import('./generated/ipc').UpdateSettings>('update_get_settings'),
  /** A switch allows one older build of the new channel, then checks it. */
  updateSetChannel: (channel: import('./generated/ipc').UpdateChannel) => invoke<import('./generated/ipc').UpdateState>('update_set_channel', { channel }),
  updateSetOvernight: (enabled: boolean) => invoke<import('./generated/ipc').UpdateSettings>('update_set_overnight', { enabled }),
  updateCheckNow: () => invoke<import('./generated/ipc').UpdateState>('update_check_now'),
  updateConnect: (token: string) => invoke<import('./generated/ipc').UpdateConnected>('update_connect', { token }),
  /** A Connect Hollow link brought a different token than the stored one (state.pendingConnect): Use the new token / Keep the current one. */
  updateConfirmConnect: () => invoke<import('./generated/ipc').UpdateState>('update_confirm_connect'),
  updateDiscardConnect: () => invoke<import('./generated/ipc').UpdateState>('update_discard_connect'),
  updateDisconnect: () => invoke<import('./generated/ipc').UpdateConnected>('update_disconnect'),
  /** Not now: hides this version's button until a newer one is ready. */
  updateDismiss: (version: string) => invoke<import('./generated/ipc').UpdateState>('update_dismiss', { version }),
  /** Restart to update. Running work is an answer (workRunning / workChanged), not an error; pass the keys the restart dialog showed. */
  updateInstallNow: (confirmedKeys?: string[] | null) => invoke<import('./generated/ipc').UpdateInstallOutcome>('update_install_now', { confirmedKeys: confirmedKeys ?? null }),
  /** Update when chats finish: waits with no expiry, in memory only. */
  updateInstallWhenIdle: () => invoke<import('./generated/ipc').UpdateState>('update_install_when_idle'),
  updateCancelWait: () => invoke<import('./generated/ipc').UpdateState>('update_cancel_wait'),
  updateLastResult: () => invoke<import('./generated/ipc').UpdateLastResult | null>('update_last_result'),
  updateAckLastResult: () => invoke<void>('update_ack_last_result'),
  // Publish to Main (row U8, update_client/publish.rs). Status asks the
  // release builder through the runner (one ssh call, seconds): call it when
  // the card opens and on Refresh, never on a timer. Publish and Withdraw
  // answer an outcome; a refusal is an outcome, not an error.
  updatePublishStatus: (pick?: string | null) => invoke<import('./generated/ipc').UpdatePublishStatus>('update_publish_status', { pick: pick ?? null }),
  /** Publish: the person's click is the approval. Pass the commit the card showed for that build and the Main it showed (null: none yet). */
  updatePublishMain: (version: string, commit: string, expectedMain: string | null) => invoke<import('./generated/ipc').UpdatePublishOutcome>('update_publish_main', { version, commit, expectedMain }),
  /** Withdraw: pass the Main version the card showed; refused (`moved`) when Main changed since. */
  updateWithdrawMain: (expected: string | null) => invoke<import('./generated/ipc').UpdatePublishOutcome>('update_withdraw_main', { expected }),
  updateSetPublisher: (enabled: boolean) => invoke<import('./generated/ipc').UpdateSettings>('update_set_publisher', { enabled }),
  /** Changes, Hollow app: Show in Update button. Offers the workspace's finished build as the Local build again (row U10); installing stays the Update button's. */
  updateOfferLocalBuild: (id: string) => invoke<import('./generated/ipc').UpdateState>('update_offer_local_build', { id }),
  // Results and Your work digests (next-level P02/P05).
  workspaceResults: (id: string) => invoke<WorkResult[]>('workspace_results', { id }),
  // Overnight F6: save a Result as Markdown with its sources (user-only).
  resultExportMarkdown: (id: string, path: string, target: string) =>
    invoke<{ target: string; sources: number }>('result_export_markdown', { id, path, target }),
  setResultReview: (id: string, path: string, review: Exclude<ResultReview, 'changed'>, expectedSha256?: string | null, note?: string) => invoke<WorkResult>('set_result_review', { id, path, review, expectedSha256: expectedSha256 ?? null, note: note ?? null }),
  resultSnapshot: (id: string, path: string, sha256: string) => invoke<string | null>('result_snapshot', { id, path, sha256 }),
  /** Library's All: the same list, but a damaged review store is an error and stays where it is. */
  listAllResultsPeek: (kind?: string | null, limit?: number) => invoke<LibraryResult[]>('list_all_results_peek', { kind: kind ?? null, limit: limit ?? null }),
  listAllResults: (kind?: string | null, limit?: number) => invoke<LibraryResult[]>('list_all_results', { kind: kind ?? null, limit: limit ?? null }),
  /** Everything waiting on the person, paged (B·F15): pass `nextCursor` back as `cursor`. */
  listNeedsYou: (cursor?: string | null, limit?: number, kinds?: NeedsYouItem['kind'][], categories?: readonly string[]) => invoke<NeedsYouPage>('list_needs_you', { cursor: cursor ?? null, limit: limit ?? null, kinds: kinds ?? null, categories: categories ?? null }),
  /** The chats the last quit interrupted mid-turn that this launch offers to resume (overnight A1). */
  listInterruptedChats: () => invoke<import('./generated/ipc').InterruptedChat[]>('list_interrupted_chats'),
  /** The person's Resume ticked: each ticked chat still offered gets the Continue text as a queued turn, with its own
   *  Canvases choice, its own Ultracode switch (P1-10) and its own effort (Settings PR 7), as its composer would send them. */
  resumeInterruptedChats: (chats: { sessionKey: string; canvases: string | null; ultracode: boolean; effort: string | null }[]) =>
    invoke<import('./generated/ipc').ResumeOutcome[]>('resume_interrupted_chats', { chats }),
  /** One conversation by its exact key, archived and old ones included; null when none has it. */
  sessionTarget: (key: string) => invoke<SessionTarget | null>('session_target', { key }),
  /** Scrubbed full-text conversation search, preferring this workspace, then work chats, then recency. */
  searchSessions: (query: string, cursor?: string | null, limit?: number, workspaceId?: string | null) => invoke<SessionSearchPage>('search_sessions', { query, cursor: cursor ?? null, limit: limit ?? null, workspaceId: workspaceId ?? null }),
  // Agenda (next-level P10): read-only calendar events.
  /** A complete read of the same window in the last minute is reused unless `fresh` (Refresh). */
  agendaEvents: (from: string, to: string, fresh?: boolean) => invoke<AgendaEventsView>('agenda_events', { from, to, fresh: fresh ?? null }),
  calendarSourceStatus: () => invoke<CalendarSourceStatus>('calendar_source_status'),
  setCalendarSource: (settings: CalendarSettings, requestAccess = false) => invoke<CalendarSourceStatus>('set_calendar_source', { settings, requestAccess }),
  openCalendarSettings: () => invoke<void>('open_calendar_settings'),
  // Recordings (next-level P11): on demand through Hollow's Plaud sign-in.
  /** A list read in the last two minutes is reused unless `fresh` (Refresh). */
  listRecordings: (days?: number, fresh?: boolean) => invoke<RecordingList>('list_recordings', { days: days ?? null, fresh: fresh ?? null }),
  readRecording: (id: string) => invoke<RecordingDetail>('read_recording', { id }),
  recordingAudioUrl: (id: string) => invoke<string>('recording_audio_url', { id }),
  setRecordingLink: (id: string, link: { workspaceId?: string | null; eventId?: string | null; eventTitle?: string | null; name?: string | null }) => invoke<RecordingLink | null>('set_recording_link', { id, workspaceId: link.workspaceId ?? null, eventId: link.eventId ?? null, eventTitle: link.eventTitle ?? null, name: link.name ?? null }),
  keepRecordingTranscript: (id: string, workspaceId: string) => invoke<string>('keep_recording_transcript', { id, workspaceId }),
  /** Master plan F4: decision records; a workspace scope also returns global ones. */
  recordsDecisions: (scope?: string, status?: string, query?: string) => invoke<DecisionRecord[]>('records_decisions', { scope, status, query }),
  recordsSetDecisionStatus: (path: string, status: 'settled' | 'revisit') => invoke<DecisionRecord>('records_set_decision_status', { path, status }),
  ackSelfInstallResult: () => invoke<void>('ack_self_install_result'),
  /** Services that outlive a turn (Build PR B6), all or one worktree's. */
  // The side panel's browser (browser_pane.rs): one native webview per tab.
  /** Resolves to the load's `gen` (each `browser-event` carries the one that sent it); 0 when a newer load replaced it. */
  browserOpen: (tabId: string, url: string, bounds: BrowserBounds) => invoke<number | undefined>('browser_open', { tabId, url, bounds }),
  browserShow: (tabId: string | null, bounds: BrowserBounds | null) => invoke<void>('browser_show', { tabId, bounds }),
  /** A reload resolves to its load's `gen`, as browserOpen; the rest to 0. */
  browserGo: (tabId: string, action: 'back' | 'forward' | 'reload' | 'stop') => invoke<number | undefined>('browser_go', { tabId, action }),
  browserUrl: (tabId: string) => invoke<string>('browser_url', { tabId }),
  browserClose: (tabId: string) => invoke<void>('browser_close', { tabId }),
  // Annotate: a PNG (base64) of the tab, and the element under each viewport point.
  browserSnapshot: (tabId: string) => invoke<string>('browser_snapshot', { tabId }),
  browserInspect: (tabId: string, points: [number, number][]) => invoke<PageInspection>('browser_inspect', { tabId, points }),
  /** Annotate → Include page text (repo ideas 2026-10-27 #3). */
  browserPageText: (tabId: string) => invoke<PageText>('browser_page_text', { tabId }),
  /** Repo ideas 2026-10-27 #5: share a Preview tab with the open chat (read
   *  only, until its turn ends), stop sharing it, and list what is shared. */
  browserGrantTab: (tabId: string, sessionKey: string, title: string) => invoke<import('./tabGrants').TabGrant>('browser_grant_tab', { tabId, sessionKey, title }),
  browserRevokeTab: (tabId: string) => invoke<void>('browser_revoke_tab', { tabId }),
  browserTabGrants: () => invoke<import('./tabGrants').TabGrant[]>('browser_tab_grants'),
  listServices: (cwd?: string) => invoke<ServiceView[]>('list_services', { cwd: cwd ?? null }),
  stopService: (cwd: string, name: string) => invoke<void>('stop_service', { cwd, name }),
  serviceLog: (cwd: string, name: string, lines?: number) => invoke<string>('service_log', { cwd, name, lines: lines ?? null }),
  readWorkspace: (id: string) => invoke<WorkspaceDetail>('read_workspace', { id }),
  /** `pinned`: the sidebar's chat pins, listed past each workspace's newest 25. */
  workspaceSessions: (pinned?: readonly string[]) => invoke<WorkspaceSessions[]>('workspace_sessions', { pinned: pinned?.length ? [...pinned] : null }),
  /** Branch and PR state of these build workspaces for the rail (cached two minutes in Rust). */
  buildRailStates: (ids: string[], fresh: string[] = []) => invoke<RailGitState[]>('build_rail_states', { ids, fresh }),
  /** Rename and/or set status (`archived` archives and unpins; refused for
   *  imported mirror workspaces), and/or pin (`pinned`), unpin or move a pin
   *  (`pinPosition`, 1 = top; any workspace but archived and built-in ones). */
  updateWorkspace: (id: string, update: { title?: string; status?: string; pinned?: boolean; pinPosition?: number }) =>
    invoke<WorkWorkspace>('update_workspace', { id, title: update.title ?? null, status: update.status ?? null, pinned: update.pinned ?? null, pinPosition: update.pinPosition ?? null }),
  setWorkspaceCanvasTheme: (id: string, theme: string | null) => invoke<WorkWorkspace>('set_workspace_canvas_theme', { id, theme }),
  /** Move the workspace folder into the vault's `.trash/`. */
  trashWorkspace: (id: string) => invoke<void>('trash_workspace', { id }),
  /** Move the newest trashed copy of workspace `id` back; refused while the id is taken (P1-41). */
  restoreWorkspace: (id: string) => invoke<WorkWorkspace>('restore_workspace', { id }),
  /** Workspaces in the vault's `.trash/`, newest first (Home's Recently deleted). */
  trashedWorkspaces: () => invoke<import('./generated/ipc').TrashedWorkspace[]>('trashed_workspaces'),
  /** What merging `from` into `into` would move (every file and conversation) and the hash that binds the confirm (master plan #18). */
  previewWorkspaceMerge: (from: string, into: string) => invoke<import('./generated/ipc').MergePlan>('preview_workspace_merge', { from, into }),
  /** Merge `from` into `into`; refused unless the plan still hashes to `hash`. `from` is archived (nothing deleted) with a record that Undo merge reads. */
  mergeWorkspace: (from: string, into: string, hash: string) => invoke<import('./generated/ipc').MergeOutcome>('merge_workspace', { from, into, hash }),
  /** Archive older entries (V5a): one shared doc of workspace `workspace`. `dry_run` (default) writes nothing and returns sizes before and after, what moves and the doc's `hash`; `apply` needs that hash as `expectedHash` and `today` (the dry run's day, so it moves exactly what was shown) and moves nothing when the doc changed since. */
  archiveSharedDocs: (workspace: string, doc: import('./archiveOlderEntries').SharedDocName, mode: 'dry_run' | 'apply' = 'dry_run', expectedHash?: string, today?: string) =>
    invoke<import('./archiveOlderEntries').SharedDocArchive>('archive_shared_docs', { workspace, doc, mode, expectedHash: expectedHash ?? null, today: today ?? null }),
  /** Undo the merge recorded in workspace `from`: files, conversations, unedited appended sections and status go back. */
  undoWorkspaceMerge: (from: string) => invoke<import('./generated/ipc').UnmergeOutcome>('undo_workspace_merge', { from }),
  /** Tick (`done`) or untick one PLAN.md row from the Plan view: the row `{line, sha256}` from `planRowId`, in a PLAN.md whose bytes still hash to `expectedRevision`. Refused with `PLAN_CHANGED:` (nothing written) when the file or the line moved on. The person's click only (user-only). */
  togglePlanRow: (args: { workspaceId: string; line: number; lineSha256: string; expectedRevision: string; done: boolean }) => invoke<import('./generated/ipc').PlanRowToggled>('toggle_plan_row', args),
  attachWorkspaceSource: (id: string, source: string) => invoke<WorkspaceDetail>('attach_workspace_source', { id, source }),
  /** Compare snapshot `path` (`sources/<name>`) with its original by content (plan L3 X5; user-only, reads only). */
  previewSourceRefresh: (id: string, path: string) => invoke<import('./workspaceTypes').SourceRefreshPreview>('preview_source_refresh', { id, path }),
  /** Replace the snapshot with the original the preview showed; `SOURCE_CHANGED:` when either side moved on (user-only). */
  applySourceRefresh: (id: string, path: string, expectedOriginSha: string, expectedSnapshotSha: string) => invoke<WorkspaceDetail>('apply_source_refresh', { id, path, expectedOriginSha, expectedSnapshotSha }),
  // Settings / Diagnostics surface — paths, binary location, runtime
  // toggles. Cheap to call; the Settings page refetches it on mount.
  diagnostics: () => invoke<Diagnostics>('diagnostics'),

  // Setup. setupStatus gates the boot branch in main.tsx (onboarding vs.
  // shell). createProfileNotes adds any missing me/ starter, never overwriting.
  setupStatus: () => invoke<SetupStatus>('setup_status'),
  createProfileNotes: () => invoke<ProfileNotesResult>('create_profile_notes'),
  setupRestart: (confirmedKeys?: string[]) => invoke<void>('setup_restart', { confirmedKeys }),
  setWorkspaceSource: (update: WorkspaceSourceUpdate) =>
    invoke<void>('set_workspace_source', { update }),

  dashboard: () => invoke<Dashboard>('dashboard'),

  // Operator profile — the durable house-rules digest compiled from the
  // vault's me/ folder. Injected into the first message of a chat session
  // (and workflow runs) so the agent knows the user's style + settled
  // decisions without being re-briefed. Cheap: reads a few small files.
  operatorContext: () => invoke<OperatorContext>('operator_context'),
  // Settings, Personalization builder (profile_builder.rs). Every call returns
  // the refreshed view; a scan runs one tool-free agent pass (up to ~2 min).
  profileBuilderView: () => invoke<ProfileBuilderView>('profile_builder_view'),
  profileBuilderScan: () => invoke<ProfileBuilderView>('profile_builder_scan'),
  profileBuilderAccept: (id: string, text?: string) =>
    invoke<ProfileBuilderView>('profile_builder_accept', { id, text: text ?? null }),
  profileBuilderAcceptAll: () => invoke<ProfileBuilderView>('profile_builder_accept_all'),
  profileBuilderDismiss: (id: string) => invoke<ProfileBuilderView>('profile_builder_dismiss', { id }),
  profileBuilderAnswer: (id: string, answer: string) =>
    invoke<ProfileBuilderView>('profile_builder_answer', { id, answer }),
  profileBuilderSkipQuestion: (id: string) => invoke<ProfileBuilderView>('profile_builder_skip_question', { id }),
  profileBuilderSetAutoLearn: (enabled: boolean) =>
    invoke<ProfileBuilderView>('profile_builder_set_auto_learn', { enabled }),
  // Explicit human review: updates only `last_reviewed` frontmatter on one of
  // the five allowlisted operator files and returns its recomputed status.
  markOperatorProfileReviewed: (path: string) =>
    invoke<OperatorFreshness>('mark_operator_profile_reviewed', { path }),
  // The About you switch: the backend persists it and is its source of
  // truth (chats and workflow runs read it). The window adopts it at launch
  // (operatorSwitch.ts) and sends only an explicit change.
  operatorContextSwitch: () => invoke<OperatorSwitchState>('operator_context_switch'),
  setOperatorContextEnabled: (enabled: boolean) =>
    invoke<void>('set_operator_context_enabled', { enabled }),

  // Tasks (vault-local; a tracker-linked workspace's issues sync through the
  // tracker). lastSyncedAt/syncError report sync freshness: a sync failure
  // never fails the read, the mirrors still serve.
  tasks: () => invoke<TasksResponse>('tasks'),
  /** Returns the written file's `contentHash` (the next CAS baseline). With
   *  `expected` (the hash this caller last saw or wrote) the write is
   *  compare-and-swap and a newer edit rejects with `TASK_CONFLICT: …`
   *  (review F01). */
  setTaskStatus: (url: string, status: string, expected?: string) =>
    invoke<string>('set_task_status', { url, status, expected: expected || null }),
  /** Returns the written file's `contentHash`; `expected` as for setTaskStatus. */
  setTaskPriority: (url: string, priority: string | null, expected?: string) =>
    invoke<string>('set_task_priority', { url, priority, expected: expected || null }),
  createTask: (input: CreateTaskFields) =>
    invoke<CreatedTask>('create_task', { input }),
  /** Task edit. Send only changed fields: omit a field to leave it unchanged;
   *  empty string / empty array clears it. Mirrors Rust `UpdateTaskInput`
   *  (camelCase). With `expected` (the loaded `contentHash`) the write is
   *  compare-and-swap; a stale baseline rejects with the plain string
   *  `TASK_CONFLICT: …`. Show it through `taskErrorText`. */
  updateTask: (input: UpdateTaskFields) =>
    invoke<UpdatedTask>('update_task', { input }),
  /** Delete a task. Returns the file's raw markdown so the UI can offer Undo. */
  deleteTask: (url: string) => invoke<string>('delete_task', { url }),
  /** Re-create a deleted task file at its original path (Undo of deleteTask). */
  restoreTask: (url: string, content: string) =>
    invoke<void>('restore_task', { url, content }),
  /** Parse a natural-language quick-add line into structured fields. */
  parseQuickAdd: (input: string) =>
    invoke<ParsedQuickAdd>('parse_quick_add', { input }),
  /** AI assist for a task: action is 'breakdown' | 'next_steps' | 'rephrase'. */
  // Inline AI in the editor (overnight 10-11 B5): one tool-free model call;
  // the editor shows the reply as a suggestion. Writes nothing.
  inlineAi: (input: InlineAiInput) => invoke<InlineAiResult>('inline_ai', { input }),
  /** Dismiss on a pending request stops its model call (overnight S3). */
  inlineAiCancel: (requestId: string) => invoke<boolean>('inline_ai_cancel', { requestId }),
  taskAssist: (input: AssistInput) =>
    invoke<AssistResult>('task_assist', { input }),

  // Projects (vault-local; tracker-linked ones sync through the tracker)
  projects: () => invoke<{ items: Project[]; fetched: string | null; source: string; lastSyncedAt: string | null; syncError: string | null }>('projects'),
  setProjectStatus: (url: string, status: string) =>
    invoke<void>('set_project_status', { url, status }),

  // One durable approval inbox across learning, activation, communication,
  // heartbeat, and meeting proposals.
  listReviewInbox: (status?: ReviewItemStatus, kind?: ReviewItemKind) =>
    invoke<ReviewInboxItem[]>('list_review_inbox', { status, kind }),
  // `expectedFingerprint` binds an approval to the card the user confirmed;
  // required to accept a `workflow_permissions` review.
  resolveReviewItem: (id: string, action: 'accept' | 'reject', editedPayload?: Record<string, unknown>, expectedFingerprint?: string, expectedVersion?: string) =>
    invoke<ReviewInboxItem>('resolve_review_item', { id, action, editedPayload, expectedFingerprint, expectedVersion }),
  /** The Undo after one Reject (P0-10): the same item back to pending, only
   *  while it is still the rejection the Reject returned (`updatedAt`). */
  undoReviewReject: (id: string, expectedUpdatedAt: string) => invoke<ReviewInboxItem>('undo_review_reject', { id, expectedUpdatedAt }),
  // Companion devices (companion apps T0): the person's own Settings clicks.
  companionStatus: () => invoke<CompanionStatus>('companion_status'),
  companionPairingStart: () => invoke<CompanionPairingCode>('companion_pairing_start'),
  companionPairingCancel: () => invoke<void>('companion_pairing_cancel'),
  companionRevokeDevice: (deviceId: string) => invoke<CompanionStatus>('companion_revoke_device', { deviceId }),
  companionBackendStatus: () => invoke<CompanionBackendStatus>('companion_backend_status'),
  companionBackendSet: (url: string, token: string) => invoke<CompanionBackendStatus>('companion_backend_set', { url, token }),
  companionBackendRepush: () => invoke<CompanionBackendStatus>('companion_backend_repush'),
  companionBackendClaim: (url: string, code: string) => invoke<CompanionBackendStatus>('companion_backend_claim', { url, code }),
  /** Test a backend without saving: the typed URL and token, or the saved connection when the URL is blank. */
  companionBackendTest: (url: string, token: string) => invoke<CompanionBackendTest>('companion_backend_test', { url, token }),
  companionScopeGet: () => invoke<CompanionScope>('companion_scope_get'),
  companionScopeSet: (includeWorkData: boolean, personalWorkspaces: string[]) =>
    invoke<CompanionScope>('companion_scope_set', { includeWorkData, personalWorkspaces }),
  // The phone chat API (2026-11-04, docs/PHONE_CHAT_API.md): Settings,
  // Companion devices → iPhone app, the person's own clicks in the main window.
  /** `fresh` (Check again) reads Tailscale now, not its copy from the last ten seconds. */
  phoneAccessStatus: (fresh = false) => invoke<PhoneAccessStatus>('phone_access_status', fresh ? { fresh } : undefined),
  phoneAccessSetEnabled: (enabled: boolean) => invoke<PhoneAccessStatus>('phone_access_set_enabled', { enabled }),
  phoneAccessPairingStart: () => invoke<PhoneAccessStatus>('phone_access_pairing_start'),
  phoneAccessPairingCancel: () => invoke<PhoneAccessStatus>('phone_access_pairing_cancel'),
  phoneAccessRevoke: (deviceId: string) => invoke<PhoneAccessStatus>('phone_access_revoke', { deviceId }),
  phoneAccessApprovalsStart: (deviceId: string) => invoke<PhoneAccessStatus>('phone_access_approvals_start', { deviceId }),
  phoneAccessApprovalsCancel: () => invoke<PhoneAccessStatus>('phone_access_approvals_cancel'),
  phoneAccessApprovalsOff: (deviceId: string) => invoke<PhoneAccessStatus>('phone_access_approvals_off', { deviceId }),
  phoneAccessApprovalsWait: (seconds: number) => invoke<PhoneAccessStatus>('phone_access_approvals_wait', { seconds }),
  phoneAccessPublish: () => invoke<PhoneAccessStatus>('phone_access_publish'),
  phoneAccessUnpublish: () => invoke<PhoneAccessStatus>('phone_access_unpublish'),
  /** The mode Settings, Models gives new chats, which a phone's New chat starts in. */
  phoneAccessNewChatMode: (mode: string, agent?: string) => invoke<void>('phone_access_new_chat_mode', { mode, agent }),
  /** Each chat's mode choice (`[key, mode]`, oldest first), which a phone's send
   *  to that chat runs in: one just made, or the whole record (`replace`); `plan`
   *  is this window's fallback for chats it holds no choice for. */
  phoneAccessChatModes: (choices: [string, string][], replace: boolean, plan: boolean) =>
    invoke<void>('phone_access_chat_modes', { choices, replace, plan }),
  /** v1.1: import the APNs key (.p8) into the Keychain; the Key ID comes from Apple's file name when not given. */
  phonePushImportKey: (path: string, keyId: string | null, teamId: string | null) =>
    invoke<PhoneAccessStatus>('phone_push_import_key', { path, keyId, teamId }),
  phonePushRemoveKey: () => invoke<PhoneAccessStatus>('phone_push_remove_key'),
  /** Send a test notification to every paired iPhone with notifications on; Hollow's sentence for what went. */
  phonePushTest: () => invoke<string>('phone_push_test'),
  /** Keep Hollow running with the lid closed (2026-11-05, `phone_access/lid.rs`):
   *  its reading, and the switch and On battery too, which ask macOS for the
   *  person's password. */
  phoneLidStatus: () => invoke<PhoneLidStatus>('phone_lid_status'),
  phoneLidSet: (enabled: boolean, onBattery: boolean) => invoke<PhoneLidStatus>('phone_lid_set', { enabled, onBattery }),
  snoozeReviewItem: (id: string, until: string) =>
    invoke<ReviewInboxItem>('snooze_review_item', { id, until }),
  /** Wake now / a snooze's Undo (P1-13): the snoozed item back to pending. */
  wakeReviewItem: (id: string) => invoke<ReviewInboxItem>('wake_review_item', { id }),
  activationInventory: () => invoke<ActivationInventory>('activation_inventory'),

  // First-class commitments and approval-gated outbound promise detection.
  // The weekly review (overnight 10-11 A7).
  weeklyReview: (week?: string) => invoke<WeeklyReviewView>('weekly_review', { week: week ?? null }),
  listCommitments: () => invoke<Commitment[]>('list_commitments'),

  // Approved learning and bounded trusted recall.
  listLearningProposals: () => invoke<LearningProposal[]>('list_learning_proposals'),
  listSkillLessons: () => invoke<{id:string;status:string;error?:string;payload:{name:string;description:string;lesson:string;quote:string;sourceRunId:string;expectedHash?:string|null}}[]>('list_skill_lessons'),
  resolveSkillLesson: (id:string, action:'accept'|'reject') => invoke<unknown>('resolve_skill_lesson',{id,action}),
  undoSkillLesson: (id:string) => invoke<unknown>('undo_skill_lesson',{id}),
  listPrivateMemories: (workspaceId: string) => invoke<{id: string; provider: string; path: string; text: string; expectedHash: string}[]>('list_private_memories', {workspaceId}),
  promotePrivateMemory: (workspaceId: string, id: string, expectedHash: string, scope: string, statement: string) => invoke<LearningProposal>('promote_private_memory', {workspaceId,id,expectedHash,scope,statement}),
  // Bulk review (the person's click only; no app action): each proposal is
  // reported on its own, and `batchId` is the one Undo for the batch. A
  // row's Save is a one-item batch, with `editedStatement` when the person
  // changed the words, so an edited Save has the same Undo. A row's Save
  // sends the `version` it read (ME1): edited words that changed meanwhile
  // are refused, and edited words need it.
  resolveLearningProposalsBatch: (ids: string[], action: 'accept' | 'reject', reason?: string, editedStatement?: string, expectedVersion?: string) =>
    invoke<LearningBatchOutcome>('resolve_learning_proposals_batch', { ids, action, reason, editedStatement, expectedVersion }),
  undoLearningBatch: (batchId: string) => invoke<LearningBatchOutcome>('undo_learning_batch', { batchId }),
  listLearningContext: (includeSuperseded = false) => invoke<LearningContext[]>('list_learning_context', { includeSuperseded }),
  memoryMaintenance: () => invoke<import('./types').MemoryMaintenance>('memory_maintenance'),
  editMemoryMaintenance: (key: string, expectedHash: string, action: 'pin' | 'unpin' | 'restore' | 'expiry', date?: string) => invoke<import('./types').MemoryMaintenance>('edit_memory_maintenance', { key, expectedHash, action, date: date || null }),
  supersedeLearningContext: (oldPath: string, newPath: string) => invoke<void>('supersede_learning_context', { oldPath, newPath }),

  // Run-scoped automated-write history and durable evidence receipts.
  listArtifactSessions: (path: string) => invoke<ArtifactSessionLink[]>('list_artifact_sessions', { path }),
  readArtifactSessionContext: (key: string) => invoke<ArtifactSessionContext>('read_artifact_session_context', { key }),
  listHistoryRuns: (limit = 100) => invoke<HistoryRun[]>('list_history_runs', { limit }),
  listNoteHistory: (path: string, limit = 100) => invoke<HistoryRun[]>('list_note_history', { path, limit }),
  readHistoryVersion: (runId: string, path: string, version: 'before' | 'after') =>
    invoke<string | null>('read_history_version', { runId, path, version }),
  restoreHistoryChange: (runId: string, path: string, force = false) =>
    invoke<HistoryRun>('restore_history_change', { runId, path, force }),
  /** A meeting's prep pack (overnight 10-11 A4): the kept one, or written now (`regenerate` writes again). */
  prepPack: (meetingId: string, regenerate: boolean, expectedHash?: string) => invoke<import('./store/prepPack').PrepPack>('prep_pack', { meetingId, regenerate, expectedHash }),
  meetingRecap: (meetingId: string, around: string, regenerate: boolean, recordingId?: string, expectedHash?: string) => invoke<import('./store/prepPack').RecapOutcome>('meeting_recap', { meetingId, around, regenerate, recordingId, expectedHash }),
  /** Today (overnight 10-11 A3): today's cached brief and the person's first name; nothing generated. */
  todayBriefCached: (date: string) => invoke<import('./todayBrief').TodayCached>('today_brief_cached', { date }),
  /** Write today's brief from the day's facts (once a day unless `regenerate`). */
  todayBrief: (date: string, facts: string, regenerate: boolean) => invoke<import('./todayBrief').TodayBrief>('today_brief', { date, facts, regenerate }),
  /** Push-to-talk (overnight 10-11 B7): on-device dictation's status, macOS's own prompt, start (a session id) and stop. */
  dictationStatus: () => invoke<import('./dictation').DictationStatus>('dictation_status'),
  dictationAuthorize: () => invoke<import('./dictation').DictationStatus>('dictation_authorize'),
  dictationStart: () => invoke<number>('dictation_start'),
  dictationStop: (session: number) => invoke<void>('dictation_stop', { session }),
  /** P1-55: System Settings, Privacy, Microphone or Speech Recognition. */
  openDictationSettings: (pane: 'microphone' | 'speech') => invoke<void>('open_dictation_settings', { pane }),
  /** The triage inbox (overnight 10-11 B6): recent imports and tasks other sources made, and what the person dismissed or filed. */
  triageArrivals: () => invoke<import('./triage').TriageArrivals>('triage_arrivals'),
  /** Each row's suggested workspace (deterministic, no model); rows below the threshold are absent. */
  triageSuggest: (items: { id: string; text: string }[]) => invoke<Record<string, import('./triage').TriageSuggestion>>('triage_suggest', { items }),
  /** Dismiss, file (into `workspaceId`) or restore an inbox row; ids only, nothing else changes. */
  /** `task` links the row to the task made from it (`taskRef`, `vault:tasks/…`), so it leaves the inbox. */
  triageMark: (id: string, action: 'dismiss' | 'file' | 'restore' | 'task', workspaceId: string | null, taskRef: string | null = null) => invoke<import('./triage').TriageState>('triage_mark', { id, action, workspaceId, taskRef }),
  restoreHistoryRun: (runId: string, force = false) => invoke<HistoryRun>('restore_history_run', { runId, force }),
  historyStorageStatus: () => invoke<HistoryStorageStatus>('history_storage_status'),
  clearHistory: () => invoke<void>('clear_automated_history'),
  listEvidenceReceipts: (source?: string, sourceId?: string, limit = 100) =>
    invoke<EvidenceReceiptWire[]>('list_evidence_receipts', { source, sourceId, limit }),
  // Paperclip PR 16: a run's repository changes with patches (its sidecar), or null.
  readHistoryRepoChanges: (runId: string) =>
    invoke<RepoSidecar | null>('read_history_repo_changes', { runId }),

  // Vault
  vaultTree: () => invoke<VaultTree>('vault_tree'),
  vaultSearch: (query: string, kinds?: VaultKind[]) => invoke<SearchResult[]>('vault_search', { query, kinds }),
  // Local semantic search (candle embeddings, fully on-device). Same
  // SearchResult shape as vaultSearch; empty when the feature is off or the
  // vault isn't indexed yet, so callers fall back to the keyword path.
  semanticSearch: (query: string, limit?: number, kinds?: VaultKind[]) =>
    invoke<SearchResult[]>('semantic_search', { query, limit, kinds }),
  semanticSearchStatus: () => invoke<SemanticStatus>('semantic_search_status'),
  setSemanticSearchEnabled: (enabled: boolean) => invoke<void>('set_semantic_search_enabled', { enabled }),
  /** The semantic vault index (classified VaultItems). Backs the Vault lenses,
   *  the cleanup queue, and richer agent context. Newest first. */
  vaultItems: () => invoke<VaultItem[]>('vault_items'),
  vaultNote: (path: string) => invoke<NoteContent>('vault_note', { path }),
  vaultCreateNote: (path: string, content: string) =>
    invoke<NoteContent>('vault_create_note', { path, content }),
  vaultOpenInObsidian: (path?: string, daily?: boolean) =>
    invoke<void>('vault_open_in_obsidian', { path, daily }),
  /** Whether an app on this Mac opens obsidian:// links (P1-41). */
  obsidianAvailable: () => invoke<boolean>('obsidian_available'),
  // 5.2: the chat card's git rows, keyed by the chat's folder (any git folder, no build binding).
  gitCardStatus: (cwd: string) => invoke<CardGitStatus>('git_card_status', { cwd }),
  gitCardBranches: (cwd: string) => invoke<CardGitBranches>('git_card_branches', { cwd }),
  gitCardSwitch: (cwd: string, branch: string, create: boolean) => invoke<CardGitStatus>('git_card_switch', { cwd, branch, create }),
  gitCardNewWorktree: (cwd: string, branch: string) => invoke<string>('git_card_new_worktree', { cwd, branch }),
  gitCardSuggestMessage: (cwd: string) => invoke<string>('git_card_suggest_message', { cwd }),
  gitCardCommitPush: (cwd: string, message: string, push: boolean) => invoke<CardGitStatus>('git_card_commit_push', { cwd, message, push }),
  // 5.4: the PR tab: the folder's branch PR with checks and comments, through gh (read-only).
  prCardView: (cwd: string) => invoke<PrView>('pr_card_view', { cwd }),
  // 7.3: Projects → Pull requests: a repository's PRs or issues through gh; triage is the person's click after the page's confirm dialog.
  githubList: (cwd: string, query: GhQuery) => invoke<GhList>('github_list', { cwd, query }),
  githubDetail: (cwd: string, kind: 'pr' | 'issue', number: number) => invoke<GhDetailView>('github_detail', { cwd, kind, number }),
  githubTriage: (cwd: string, triage: Triage) => invoke<TriageDone>('github_triage', { cwd, triage }),
  // 7.2: the terminal drawer's shells (person-driven; never agent tools). Open checks the chat's mode, the folder and scrubs the environment.
  terminalOpen: (chat: string, agent: string | null, perm: string, cwd: string, rows: number, cols: number) => invoke<{ id: string; cwd: string; shell: string; pid: number }>('terminal_open', { chat, agent, perm, cwd, rows, cols }),
  terminalGate: (chat: string, agent: string | null, perm: string) => invoke<string | null>('terminal_gate', { chat, agent, perm }),
  terminalWrite: (id: string, data: string) => invoke<void>('terminal_write', { id, data }),
  terminalResize: (id: string, rows: number, cols: number) => invoke<void>('terminal_resize', { id, rows, cols }),
  terminalClose: (id: string) => invoke<void>('terminal_close', { id }),
  terminalAdoptChat: (from: string, to: string) => invoke<void>('terminal_adopt_chat', { from, to }),
  // 5.3: the Diff tab. No range: the folder against HEAD (with untracked files); with `from` and `to` commits: one turn.
  gitCardDiff: (cwd: string, path: string | null, from: string | null, to: string | null) => invoke<BuildDiff>('git_card_diff', { cwd, path, from, to }),
  gitCardRangeFiles: (cwd: string, from: string, to: string) => invoke<CardGitFile[]>('git_card_range_files', { cwd, from, to }),
  // 5.1: the chat card's Open in menu: Finder plus the editors installed on this Mac.
  openInTargets: () => invoke<{ id: string; label: string }[]>('open_in_targets'),
  openInOpen: (path: string, target: string) => invoke<void>('open_in_open', { path, target }),
  // Compare-and-swap autosave: write only if the file still equals `baseline`,
  // else return a conflict with the on-disk note (reload / keep-mine).
  vaultSaveNoteCas: (path: string, content: string, baseline: string) =>
    invoke<SaveNoteResult>('vault_save_note_cas', { path, content, baseline }),
  vaultMoveNote: (from: string, to: string) =>
    invoke<NoteContent>('vault_move_note', { from, to }),
  /** Moves the note to the vault's `.trash/`; resolves to its path there, for Undo (P1-18). */
  vaultDeleteNote: (path: string) => invoke<string>('vault_delete_note', { path }),
  vaultRevealInFinder: (path: string) => invoke<void>('vault_reveal_in_finder', { path }),
  vaultNotesList: () => invoke<NoteRef[]>('vault_notes_list'),
  vaultGraph: () => invoke<VaultGraph>('vault_graph'),
  vaultBacklinks: (path: string) => invoke<BacklinkSource[]>('vault_backlinks', { path }),

  // Chat (spawns the selected agent CLI headless and streams 'chat-event' events)
  // A missing persisted cwd is resolved before the composer is consumed so a
  // context-dependent reply is never auto-run as a fresh vault chat.
  chatPreflight: (cwd?: string) => invoke<string | null>('chat_preflight', { cwd }),
  // `sessionKey` is Hollow's own session identity, echoed back from the
  // previous turn's `runStarted`. Null on a fresh conversation — the backend
  // mints one and reports it.
  // Paperclip PR 7b: the send ENQUEUES and resolves at once with the queue
  // position; it no longer waits for the turn. `clientTurnId` is a nonce the
  // backend echoes in runStarted / error / cancelled / done so the composer
  // can match events to this send even when runStarted wins the race.
  // `ultracode` is the chat's Track U switch: this turn runs at ultracode
  // instead of the agent's saved effort. `canvases` is the chat's own
  // Canvases choice (Canvas C3 wave 2 C4); null follows the setting.
  // `continuedFrom` names the earlier conversations this one continues
  // (Continue from summary, a folder-vanished reset), oldest first; the
  // backend bundles their recorded turns on a fresh native session
  // (agent effectiveness F4, `chatContinuedFrom.ts`).
  // `effort` (Settings PR 7): this chat's own reasoning effort, queued with
  // the turn and never saved to Settings; null runs at the app default.
  // `canned`: a request Hollow wrote that the person sent unchanged (overnight 9 A1).
  // `after` (mid-turn steering): while this conversation's turn runs, hold
  // the send for after it (`queue`) or stop it first (`stop`); the reply then
  // carries `heldId`, and a `heldSent` chat-event says when it went.
  chatSend: (prompt: string, sessionId: string | null, cwd?: string, model?: string, permissionMode?: string, agent?: string, sessionKey?: string | null, clientTurnId?: string, ultracode?: boolean, canvases?: CanvasFrequency | null, continuedFrom?: readonly string[], canned?: CannedRequest | null, after?: 'queue' | 'stop' | null, effort?: string | null) =>
    invoke<SendAccepted>('chat_send', { prompt, sessionId, sessionKey, cwd, model, permissionMode, agent, clientTurnId, ultracode: ultracode ?? false, canvases: canvases ?? null, continuedFrom: continuedFrom?.length ? [...continuedFrom] : null, canned: canned ?? null, after: after ?? null, effort: effort || null }),
  // Mid-turn steering: the person's message into the running turn of that
  // conversation, taken at the agent's next step. Rejects with a STEER_*
  // code (chatSteer.ts `steerRefusalNotice` words it, and says whether it
  // goes as the next message).
  chatSteer: (sessionKey: string, text: string, clientId: string) =>
    invoke<{ type: 'steer'; sessionKey: string; steer: SteerWire }>('chat_steer', { sessionKey, text, clientId }),
  chatSteerSupport: () => invoke<SteerSupportRow[]>('chat_steer_support'),
  chatAckHeldFailed: (sessionKey: string, heldId: string) => invoke<boolean>('chat_ack_held_failed', { sessionKey, heldId }),
  chatCancelHeld: (sessionKey: string, heldId: string) => invoke<boolean>('chat_cancel_held', { sessionKey, heldId }),
  // Stops the turn of that session: a queued one is taken out of the queue,
  // a running one is stopped. No key = whatever turn is running.
  // A chat's own reasoning effort lives in Rust (the session store), so runs
  // Hollow starts itself (automations, wake-ups, a handover successor, a
  // resume) use it too. The window keeps a cache: read once at start.
  chatEffortsList: () => invoke<[string, string][]>('chat_efforts_list'),
  chatEffortSet: (sessionKey: string, effort: string | null) => invoke<void>('chat_effort_set', { sessionKey, effort }),
  chatEffortsImport: (entries: [string, string][]) => invoke<number>('chat_efforts_import', { entries }),
  chatCancel: (sessionKey?: string | null) => invoke<void>('chat_cancel', { sessionKey: sessionKey ?? null }),
  // Side chat (t3code parity 7.6, `side_chat.rs`): a second chat beside the one
  // on screen. `sideChatOpen` is the chat's newest side chat (a new one when it
  // has none, or `fresh` asks and the newest has a turn); the parent is only
  // read. `sideChatSend` queues a question: seeded from the parent on its
  // first turn, in no stronger a mode than the parent's (read only unless
  // `sameAccess`). Side chats are not in `listAgentSessions`.
  sideChatOpen: (parentKey: string, fresh?: boolean) => invoke<AgentSessionSummary>('side_chat_open', { parentKey, fresh: fresh ?? false }),
  sideChatList: (parentKey: string) => invoke<AgentSessionSummary[]>('side_chat_list', { parentKey }),
  sideChatSend: (key: string, prompt: string, clientTurnId: string, sameAccess: boolean) =>
    invoke<SendAccepted>('side_chat_send', { key, prompt, clientTurnId, sameAccess }),
  // P1a: pause a running chat at its next safe point (true when it reached a
  // running turn), and resume a paused one on its stored native session.
  chatPause: (sessionKey: string) => invoke<boolean>('chat_pause', { sessionKey }),
  chatResumePaused: (sessionKey: string) => invoke<SendAccepted>('chat_resume_paused', { sessionKey }),
  // P1b: Pause all (the keys asked) and Resume all; P1c: every paused chat
  // with its note, reason and resume time, and set or clear (null) a time
  // (unix seconds; a past time resumes now and answers null).
  chatPauseAll: () => invoke<string[]>('chat_pause_all'),
  chatResumeAll: () => invoke<{ resumed: string[]; refused: [string, string][]; skipped: string[] }>('chat_resume_all'),
  chatPausedList: () => invoke<PausedChatRecord[]>('chat_paused_list'),
  chatSetResumeAt: (sessionKey: string, at: number | null) => invoke<PausedChatRecord | null>('chat_set_resume_at', { sessionKey, at }),
  // Agent sessions: Hollow-owned identity over every harness. The rail
  // switches to these in a later phase; Phase 1 records and exposes them.
  listAgentSessions: (limit?: number, archived?: boolean) =>
    invoke<AgentSessionSummary[]>('list_agent_sessions', { limit, archived }),
  /** The window's cold open of a chat (WD1: long tool results come as
   *  previews with `resultChars`; readToolResult brings one whole). */
  readAgentConversation: (key: string) => invoke<AgentConversation>('read_agent_conversation', { key }),
  /** WD1: one stored tool call's full result (a previewed row expanding). */
  readToolResult: (key: string, ord: number, toolId: string) => invoke<string | null>('read_tool_result', { key, ord, toolId }),
  /** CP1: a chat's turn checkpoints (empty with a reason outside its own isolated worktree). */
  turnCheckpoints: (key: string) => invoke<TurnCheckpoints>('turn_checkpoints', { key }),
  /** CP1: Restore to before this turn (the person's click; refused while a turn runs). */
  restoreTurnCheckpoint: (key: string, ord: number) => invoke<RestoredCheckpoint>('restore_turn_checkpoint', { key, ord }),
  archiveAgentSession: (key: string, archived: boolean) => invoke<void>('archive_agent_session', { key, archived }),
  renameAgentSession: (key: string, title: string) => invoke<void>('rename_agent_session', { key, title }),
  /** Regenerate title: a new model title from the first request; the new title. */
  regenerateChatTitle: (key: string) => invoke<string>('regenerate_chat_title', { key }),
  /** Move a conversation to another workspace (refused while it has a turn in flight). */
  moveAgentSession: (key: string, workspaceId: string) => invoke<SessionMove>('move_agent_session', { key, workspaceId }),
  // H5a session import: the sheet (lane IMPORT-UI) reads the sources, starts an inspect, polls status, lists, previews, commits or cancels.
  sessionImportSources: () => invoke<ImportSource[]>('session_import_sources'),
  inspectSessionImport: (source: ImportSourceId, root?: string | null) => invoke<{ importId: string; root: string }>('inspect_session_import', { source, root: root ?? null }),
  sessionImportStatus: (importId?: string | null) => invoke<{ jobs: ImportJob[]; batches: ImportBatch[] }>('session_import_status', { importId: importId ?? null }),
  listSessionImport: (importId: string) => invoke<ImportManifest>('list_session_import', { importId }),
  previewSessionImport: (importId: string, entry: string) => invoke<ImportPreview>('preview_session_import', { importId, entry }),
  commitSessionImport: (importId: string, request?: { ids?: string[] | null; destination?: string | null; keepOutOfSearch?: boolean }) => invoke<ImportCommitResult>('commit_session_import', { importId, request: request ?? null }),
  cancelSessionImport: (importId: string) => invoke<boolean>('cancel_session_import', { importId }),
  /** N26 Continue in Hollow: a new chat linked to imported `key`; the imported record stays read-only. */
  continueImportedSession: (key: string, request: { agent: string; model?: string | null; workspaceId?: string | null; mode?: string | null; effort?: string | null }) => invoke<ImportContinued>('continue_imported_session', { key, request }),
  /** H8: can Continue fork the source Claude Code session? `reason` says why not. */
  nativeResumeOffer: (key: string) => invoke<NativeResumeOffer>('native_resume_offer', { key }),
  /** H8: Continue by native fork-resume; the composer sends `resumeSessionId` as the first turn's resume id. Refuses with the reason; use `continueImportedSession` then. */
  continueImportedSessionNative: (key: string, request?: { model?: string | null; mode?: string | null; effort?: string | null }) => invoke<ImportContinuedNative>('continue_imported_session_native', { key, request: request ?? null }),
  /** N25: Keep out of agent search for one import batch. */
  setSessionImportSearch: (importId: string, keepOut: boolean) => invoke<number>('set_session_import_search', { importId, keepOut }),
  // Structured question cards (Paperclip PR 13). Refusals are plain strings
  // prefixed INTERACTION_CLOSED / _STALE / _INVALID.
  /** With `chain`, a current conversation's list also holds the open cards
   *  of the chats before it in its chain (one Main conversation, 11-07). */
  listPendingInteractions: (sessionKey: string, chain = false) =>
    invoke<ChatInteraction[]>('list_pending_interactions', { sessionKey, chain }),
  /** A conversation's chain: the current one, earlier ones, forward notes. */
  conversationChain: (key: string) => invoke<ChainView>('conversation_chain', { key }),
  /** L2: a conversation's subagents (the side panel's Subagents tab). */
  sessionSubagents: (parent: string, older = 0) => invoke<SubagentsView>('session_subagents', { parent, older }),
  resolveInteraction: (id: string, answer: InteractionAnswer, nativeSessionId: string | null) =>
    invoke<ResolvedInteraction>('resolve_interaction', { id, answer, nativeSessionId }),
  dismissInteraction: (id: string) => invoke<ChatInteraction>('dismiss_interaction', { id }),
  reopenInteraction: (id: string) => invoke<ChatInteraction>('reopen_interaction', { id }),
  readAgentSession: (key: string) =>
    invoke<AgentSessionDetail>('read_agent_session', { key }),
  // With a key, that conversation's (P1a: phase `paused` from its record when
  // it is paused); without one, any conversation's running turn.
  agentSessionLive: (key?: string | null) => invoke<LiveAgentSession | null>('agent_session_live', key ? { key } : undefined),
  // Settings, Usage: the usage ledger for today / this week / this month /
  // the last 30 days, every agent or one agent tab's (Settings PR 8).
  usageSummary: (window: UsageWindow, agent?: string | null) => invoke<UsageSummary>('usage_summary', { window, agent: agent ?? null }),
  // Settings, Usage, Prices (overnight 9 A13): the table as it applies,
  // and saving or removing the person's own row (a confirm action for agents).
  priceTable: () => invoke<ModelPriceRow[]>('price_table'),
  setPrice: (price: ModelPrice, remove = false) => invoke<ModelPriceRow[]>('set_price', { price, remove }),
  valueSummary: (month?: string) => invoke<import('./valueLedger').ValueSummary>('summary', { month: month ?? null, groupBy: null }),
  getValueBaselines: () => invoke<import('./valueLedger').Baselines>('get_baselines'),
  setValueBaseline: (input: { workKind?: string; runId?: string; minutes: number | null; note?: string }) =>
    invoke<import('./valueLedger').Baselines>('set_baseline', { input }),
  // Settings, Usage "Claude plan" card (display only). null hides it; `force`
  // is Refresh, which may ask Claude Code's usage endpoint (once a minute).
  claudeQuota: (force: boolean) => invoke<ClaudeQuota | null>('claude_quota', { force }),
  // Settings, Claude accounts: several Claude plans, one chosen per run.
  // Tokens go in, never come back out.
  claudeAccounts: () => invoke<ClaudeAccountsView>('claude_accounts'),
  addClaudeAccount: (label: string, token: string) =>
    invoke<ClaudeAccountsView>('add_claude_account', { label, token }),
  updateClaudeAccount: (input: ClaudeAccountUpdate) =>
    invoke<ClaudeAccountsView>('update_claude_account', { input }),
  removeClaudeAccount: (id: string) => invoke<ClaudeAccountsView>('remove_claude_account', { id }),
  setClaudeAccountRouting: (routing: ClaudeAccountRouting, pinned: string | null, ceilingPercent: number | null) =>
    invoke<ClaudeAccountsView>('set_claude_account_routing', { routing, pinned, ceilingPercent }),
  setClaudeAccountFailover: (enabled: boolean) =>
    invoke<ClaudeAccountsView>('set_claude_account_failover', { enabled }),
  // Settings, Models, Self-hosted models (self-hosted models Q4,
  // self_hosted_registry.rs): every change returns the whole list. `edit`
  // false adds (a new id), true replaces the entry with its id. Test may
  // wait minutes while a cold model starts.
  selfHostedModels: () => invoke<SelfHostedModels>('self_hosted_models'),
  saveSelfHostedModel: (entry: SelfHostedEntry, edit: boolean) =>
    selfHostedChanged(entry.id, invoke<SelfHostedModels>('save_self_hosted_model', { entry, edit })),
  removeSelfHostedModel: (id: string) => selfHostedChanged(id, invoke<SelfHostedModels>('remove_self_hosted_model', { id })),
  setSelfHostedModelEnabled: (id: string, enabled: boolean) =>
    selfHostedChanged(id, invoke<SelfHostedModels>('set_self_hosted_model_enabled', { id, enabled })),
  testSelfHostedModel: (id: string) => invoke<SelfHostedTest>('test_self_hosted_model', { id }),
  // A self-hosted chat's start chip (Q6, self_hosted_wake.rs): wakes the
  // model on a keystroke, send or Start now; may wait minutes on a cold
  // start. One wake per entry per 2 minutes; a call meanwhile shares it.
  wakeSelfHostedModel: (id: string) => invoke<SelfHostedWake>('wake_self_hosted_model', { id }),
  // The entry's last wake (its age and answer), from memory: no model call.
  selfHostedWakeStatus: (id: string) => invoke<SelfHostedWakeStatus | null>('self_hosted_wake_status', { id }),
  // Its Background jobs (self-hosted models Q10, self_hosted_jobs.rs): which
  // model each kind of Hollow's own calls uses (`null`: As now), and Run them
  // now, which wakes the model and runs what waits for it.
  selfHostedJobs: () => invoke<SelfHostedJobs>('self_hosted_jobs'),
  setSelfHostedJob: (kind: SelfHostedJobKind, entry: string | null) =>
    invoke<SelfHostedJobs>('set_self_hosted_job', { kind, entry }),
  runSelfHostedJobsNow: (entry: string) => invoke<SelfHostedJobs>('run_self_hosted_jobs_now', { entry }),
  // A held job (its kind back to As now): only this click sends it there.
  runSelfHostedJobAsNow: (id: string) => invoke<SelfHostedJobs>('run_self_hosted_job_as_now', { id }),
  // Try again on a job that gave up: its attempts cleared; `started` says
  // whether its model's run began now or the next run takes it.
  retrySelfHostedJob: (id: string) => invoke<SelfHostedRetry>('retry_self_hosted_job', { id }),
  discardSelfHostedJob: (id: string) => invoke<SelfHostedJobs>('discard_self_hosted_job', { id }),
  // Settings, Keyboard shortcuts (shortcuts.rs): saved in app data, applied
  // to the menu bar and the system-wide quick capture / ingest keys.
  getShortcutSettings: () => invoke<ShortcutsView>('get_shortcut_settings'),
  setShortcutSettings: (settings: ShortcutSettings) => invoke<ShortcutsView>('set_shortcut_settings', { settings }),
  /** While recording a key: pause every shortcut (nothing saved), then restore. */
  suspendShortcuts: (suspended: boolean) => invoke<string[]>('suspend_shortcuts', { suspended }),
  /** Grey out Stop Running Turn, Back and Forward in the menu bar (P1-49). */
  setMenuEnabled: (items: Record<string, boolean>) => invoke<void>('set_menu_enabled', { items }),
  /** After an Add sheet ⌘⌥⇧T / ⌘⌥⇧I or their tray items opened (S5 review):
   *  saved, hide Hollow again where it came from (a no-op when it was in
   *  front); an import that failed after the sheet closed, bring it forward. */
  afterSummonedCapture: (saved: boolean) => invoke<void>('after_summoned_capture', { saved }),
  refreshClaudeAccounts: () => invoke<ClaudeAccountsView>('refresh_claude_accounts'),
  // Settings, Accounts: the other agent CLIs' own sign-ins (Hollow stores none).
  providerAccounts: () => invoke<ProviderAccount[]>('provider_accounts'),
  providerSignIn: (id: string) => invoke<ProviderAccount[]>('provider_sign_in', { id }),
  providerSignOut: (id: string) => invoke<ProviderAccount[]>('provider_sign_out', { id }),
  // Settings, Usage budgets (Paperclip PR 8a). Every write emits
  // `budget-changed`; resume never starts a run (the pump claims the next).
  getBudgets: () => invoke<BudgetsView>('get_budgets'),
  saveBudget: (input: BudgetInput) => invoke<BudgetDef>('save_budget', { input }),
  deleteBudget: (id: string) => invoke<void>('delete_budget', { id }),
  resumeBudget: (id: string, newLimit: number) => invoke<void>('resume_budget', { id, newLimit }),
  // Which agent CLIs (claude / codex / antigravity / grok) are installed on this machine.
  agentSettings: () => invoke<AgentSettings>('get_agent_settings'),
  setAgentSettings: (settings: AgentSettings) => invoke<AgentSettings>('set_agent_settings', { settings }),
  agentDescriptors: () => invoke<AgentDescriptor[]>('agent_descriptors'),
  /** Whether the installed Codex runs Hollow's tool-free helpers (F9). */
  codexHelperStatus: () => invoke<import('./types').CodexHelperStatus>('codex_helper_status'),
  /** Model choices for one agent's picker; custom IDs stay valid. */
  agentModels: (agent: string, refresh = false) => invoke<import('./types').AgentModelCatalog>('agent_models', { agent, refresh }),
  // The CFO (COO rebuild L7): Settings, Models (Effort, Models the CFO may
  // use) and Settings, Usage (Reserve, Extra usage). Person-only.
  cfoSettings: () => invoke<import('./generated/ipc').CfoSettings>('cfo_settings'),
  /** Saves only the fields one panel owns (Agents: allow, effortMode; Usage:
   *  reservePercent), so the two never overwrite each other. */
  cfoSaveSettings: (patch: import('./generated/ipc').CfoPatch) => invoke<import('./generated/ipc').CfoView>('cfo_save_settings', { patch }),
  cfoView: () => invoke<import('./generated/ipc').CfoView>('cfo_view'),
  /** Hours 1 to 72 approve extra usage for unattended work; 0 withdraws it. */
  cfoApproveExtraUsage: (hours: number) => invoke<import('./generated/ipc').CfoView>('cfo_approve_extra_usage', { hours }),
  // Settings, Accounts, Agent updates.
  agentCliStatus: (refresh = false) => invoke<import('./generated/ipc').AgentCliStatus[]>('agent_cli_status', { refresh }),
  updateAgentCli: (agent: string, now = false) => invoke<import('./generated/ipc').AgentCliStatus>('update_agent_cli', { agent, now }),
  restoreAgentCli: (agent: string) => invoke<import('./generated/ipc').AgentCliStatus>('restore_agent_cli', { agent }),
  cancelAgentCliUpdate: (agent: string) => invoke<import('./generated/ipc').AgentCliStatus>('cancel_agent_cli_update', { agent }),
  setAgentCliPolicy: (agent: string, policy: 'automatic' | 'ask' | 'off') => invoke<string>('set_agent_cli_policy', { agent, policy }),
  agentAvailable: () => invoke<string[]>('agent_available'),
  // Save a pasted/dropped image or file (base64, no data: prefix) → absolute
  // path, written inside `cwd` (the chat working dir) so Claude can Read it
  // freely. `name` is a file's original name (C5): the saved name keeps a
  // cleaned stem and extension of it. An image passes none.
  chatSaveAttachment: (data: string, mime: string, cwd?: string, name?: string) =>
    invoke<string>('chat_save_attachment', name == null ? { data, mime, cwd } : { data, mime, cwd, name }),
  chatDiscover: (cwd?: string) => invoke<string[]>('chat_discover', { cwd }),
  // Resolve an inline chat approval card (Allow/Deny) — unblocks the MCP
  // subprocess waiting on the Rust approval broker.
  // A Decline may carry the words the person typed; they go to the agent.
  approvalRespond: (id: string, approved: boolean, reason?: string) =>
    invoke<void>('approval_respond', { id, approved, reason }),
  // Read a file the agent created/edited for the Chat preview pane (absolute
  // path; text capped, images as data URLs).
  // The first of `paths` that is a file (2026-11-04): a chat's file link tries each place it may be.
  chatLocateFile: (paths: string[]) => invoke<string | null>('chat_locate_file', { paths }),
  /** Files whose path ends with `tail` under every project folder and repo (`roots`), then the vault (2026-11-10). */
  chatFindFile: (tail: string, roots: string[]) => invoke<string[]>('chat_find_file', { tail, roots }),
  chatPreviewFile: (path: string) => invoke<ChatPreviewFile>('chat_preview_file', { path }),
  pdfPreviewPage: (path: string, page: number, sessionKey?: string | null) => invoke<PdfPreviewPage>('pdf_preview_page', { path, page, sessionKey: sessionKey ?? null }),
  // Open a previewed artifact in the OS default app (reveal=false) or reveal
  // it in Finder (reveal=true). Says which it did: an open of a folder, app,
  // executable or script (decided on the symlink-resolved target) reveals
  // instead (overnight H4).
  // `withApp: 'quicktime'` opens an mp4/m4v/mov in QuickTime Player.
  chatPreviewOpen: (path: string, reveal: boolean, withApp?: 'quicktime') =>
    invoke<'opened' | 'revealed'>('chat_preview_open', { path, reveal, with: withApp ?? null }),
  // A token for playing one video through the `hollow-media` scheme (media.rs).
  // Refusals reject with a stable code (mediaKinds.ts `refusalOf`). Never log
  // or send the token anywhere: it is the only key to the file.
  mediaGrant: (path: string, sessionKey?: string | null) =>
    invoke<MediaGrant>('media_grant', { path, sessionKey: sessionKey ?? null }),
  mediaRelease: (token: string) => invoke<void>('media_release', { token }),
  // A video's first frame as a `data:image/jpeg` URL (cached; same path rules).
  mediaPoster: (path: string, sessionKey?: string | null) =>
    invoke<string>('media_poster', { path, sessionKey: sessionKey ?? null }),
  claudeAdmin: (args: string[], cwd?: string) =>
    invoke<string>('claude_admin', { args, cwd }),

  // Skills (~/.claude/skills + plugin/marketplace SKILL.md)
  listSkills: () => invoke<Skill[]>('list_skills'),
  /** NV3: whether each agent loads a skill (its `SKILL.md` path), per profile rule and CLI version; no path lists the rules alone. */
  skillPortabilityTable: (path?: string | null) => invoke<PortabilityTable>('skill_portability_table', { path: path ?? null }),
  readSkill: (path: string) => invoke<string>('read_skill', { path }),
  writeUserSkill: (name: string, description: string, body: string, createOnly = true) =>
    invoke<Skill>('write_user_skill', { name, description, body, createOnly }),
  deleteUserSkill: (name: string) =>
    invoke<void>('delete_user_skill', { name }),
  /** Import step 1: check and stage a .skill/.zip file, a skill folder or a
   *  SKILL.md (a picked path, or a drop's bytes); nothing is installed yet. */
  previewSkillImport: (source: import('./types').SkillImportSource, renameTo?: string) =>
    invoke<import('./types').SkillImportPreview>('preview_skill_import', renameTo ? { source, renameTo } : { source }),
  /** Import step 2: install the staged skill into ~/.agents/skills. */
  commitSkillImport: (token: string, onClash: import('./types').SkillImportClash) =>
    invoke<import('./types').SkillImportOutcome>('commit_skill_import', { token, onClash }),
  discardSkillImport: (token: string) =>
    invoke<void>('discard_skill_import', { token }),
  /** Repo ideas 10-27 #2: a shared skill's import record, changes outside
   *  Hollow and local note. */
  skillProvenance: (name: string) =>
    invoke<import('./types').SkillProvenance>('skill_provenance', { name }),
  /** Check for update…: stage the recorded source again (fetches an https
   *  origin only now) for Import's card; nothing is replaced yet. */
  stageSkillUpdate: (name: string) =>
    invoke<import('./types').SkillImportPreview>('stage_skill_update', { name }),
  /** Keep (or with null, drop) a shared skill's local note. */
  setSkillNote: (name: string, note: string | null) =>
    invoke<import('./types').SkillProvenance>('set_skill_note', { name, note }),

  /** Stash a sandboxed artifact document for the artifact:// protocol.
   *  The served response carries the selected CSP, so safe previews stay
   *  scriptless/offline and Interactive can run in installed builds. */
  artifactPut: (parts: string[], mode: string) => invoke<string>('artifact_put', { parts, mode }),
  // A remote image as a `data:` URL (remote_image.rs): the main window's CSP
  // allows no remote images, so a click-to-load placeholder and the app's own
  // icons (connector favicons, GitHub avatars) fetch through the backend.
  fetchRemoteImage: (url: string) => invoke<string>('fetch_remote_image', { url }),
  // Diagnostics: wipe `~/.cache/hollow/*.log`. Returns the number of
  // files removed. Debug logs are only written when HOLLOW_DEBUG_LOG=1
  // is set in the environment at launch — this lets users clear any
  // they accumulated in a prior debug session.
  clearDebugLogs: () => invoke<number>('clear_debug_logs'),
  // Overnight C6: a redacted diagnostics zip into a folder outside the vault.
  saveDiagnostics: (dir: string) => invoke<DiagnosticsSaved>('save_diagnostics', { dir }),
  // Settings export/import (overnight F7): the person's own, user-only.
  settingsExport: (path: string) => invoke<SettingsExported>('settings_export', { path }),
  settingsImportPreview: (path: string) => invoke<SettingsImportPreview>('settings_import_preview', { path }),
  settingsImportApply: (path: string, expectedHash: string) =>
    invoke<SettingsImportApplied>('settings_import_apply', { path, expectedHash }),
  // Overnight C8: Home's card about last night (runs, failures, PRs, commits, waiting).
  overnightSummary: () => invoke<OvernightSummary>('overnight_summary'),
  // Overnight D1: free space and Hollow's largest folders (read-only; du takes a few seconds).
  diskGuard: () => invoke<DiskReport>('disk_guard'),
  // A12: the PATH every agent gets, folder by folder with its source, and
  // whether the login shell's PATH was read (Settings, Advanced; read-only).
  agentToolPath: () => invoke<AgentToolPath>('agent_tool_path'),
  /** Settings, Advanced, Disk space, Warn under (morning N6). */
  setDiskThreshold: (thresholdGb: number) => invoke<number>('set_disk_threshold', { thresholdGb }),
  /** The saved Warn under, without measuring (P1-22). */
  diskThreshold: () => invoke<number>('disk_threshold'),
  // Settings, Advanced, Disk space's two cleanups (overnight H20): the
  // person's click, after the preview's confirm. Each delete is bound to the
  // previewed list's hash, so a list that changed since refuses and deletes
  // nothing. Agents reach the same through build.prune_install_targets and
  // build.clean_worktrees, each with its own card.
  installTargets: (keep: number | null = null) => invoke<InstallTargetsPlan>('install_targets', { keep }),
  pruneInstallTargets: (keep: number | null, expectedHash: string) => invoke<InstallTargetsPruned>('prune_install_targets', { keep, expectedHash }),
  worktreeHygiene: (repo: string | null = null) => invoke<WorktreeHygieneReport>('worktree_hygiene', { repo }),
  cleanWorktrees: (repo: string | null, expectedHash: string) => invoke<WorktreesCleaned>('clean_worktrees', { repo, expectedHash }),
  // Repo features R7b: Hollow's databases (Settings, Advanced; read-only) and
  // Disk space's third cleanup, Remove old indexes… (hash-bound like the two
  // above; agents: system.remove_stale_indexes with its card).
  hollowDatabases: (check = false) => invoke<HollowDatabases>('hollow_databases', { check }),
  staleIndexes: () => invoke<StaleIndexesPlan>('stale_indexes'),
  removeStaleIndexes: (expectedHash: string) => invoke<StaleIndexesRemoved>('remove_stale_indexes', { expectedHash }),

  // Plaud connector: sign-in through the `plaud` CLI's own OAuth (`plaud
  // login`; tokens at ~/.plaud/tokens.json). Recordings stay in Plaud.
  plaudConnected: () => invoke<boolean>('plaud_connected'),
  plaudLogin: () => invoke<void>('plaud_login'),
  plaudLogout: () => invoke<void>('plaud_logout'),
  plaudPing: () => invoke<boolean>('plaud_ping'),

  // Universal ingestion. `source` is one of file/url/text; `opts`
  // carries the cognitive anchors (domain/purpose). Claude decides how
  // many notes the source becomes. Streams progress via the
  // `ingest-event` Tauri channel; resolves with the final IngestResult.
  ingestRun: (source: IngestSource, opts: IngestOpts) =>
    invoke<IngestResult>('ingest_run', { source, opts }),

  // Custom themes — kitty-style `*.theme` palette files under
  // `~/Library/Application Support/Hollow/themes/`. The frontend
  // derives the full set of design tokens from the parsed file.
  listCustomThemes: () => invoke<CustomThemeMeta[]>('list_custom_themes'),
  readCustomTheme: (name: string) => invoke<CustomTheme>('read_custom_theme', { name }),
  writeCustomTheme: (name: string, content: string) =>
    invoke<CustomTheme>('write_custom_theme', { name, content }),
  deleteCustomTheme: (name: string) => invoke<void>('delete_custom_theme', { name }),
  revealThemesFolder: () => invoke<void>('reveal_themes_folder'),
  themesDirPath: () => invoke<string>('themes_dir_path'),

  // Workflows — Claude-driven automations stored under vault/workflows/.
  // The scheduler tokio task fires due cron-scheduled workflows once per
  // minute; runWorkflow runs a workflow on demand. Both paths stream
  // progress via the `workflow-event` Tauri event channel keyed by slug.
  listWorkflows: () => invoke<Workflow[]>('list_workflows'),
  readWorkflow: (slug: string) =>
    invoke<WorkflowDetail>('read_workflow', { slug }),
  listWorkflowOutputs: (slug: string) =>
    invoke<WorkflowOutput[]>('list_workflow_outputs', { slug }),
  writeWorkflow: (input: WorkflowInput, originalSlug?: string | null) =>
    invoke<Workflow>('write_workflow', { input, originalSlug: originalSlug ?? null }),
  deleteWorkflow: (slug: string) =>
    invoke<void>('delete_workflow', { slug }),
  /** Run now; with `retryOf` (plan completion L2), "Run again without
   *  repeating" that failed run: Hollow refuses repeating its completed
   *  outside actions and tells the agent what already happened. */
  runWorkflow: (slug: string, retryOf?: string | null) =>
    invoke<WorkflowRunResult>('run_workflow', retryOf ? { slug, retryOf } : { slug }),
  /** One run's cost with its descendants' (`run_cost`): null totals are
   *  unknown, never $0. */
  runCost: (runId: string) =>
    invoke<import('./generated/ipc').ManifestCostRollup>('run_cost', { runId }),
  /** Repo features R6: what a conversation's context is spent on (the usage
   *  ring's What's using it and Cache), read when the section opens. */
  conversationContextUse: (key: string) =>
    invoke<import('./types').ContextUse>('conversation_context_use', { key }),
  /** Usual length of a finished chat of this folder's work kind
   *  (`job_timing`). `typicalMs` is null until three of that kind have
   *  finished. Not how long this chat has taken, and not hours saved. */
  jobTiming: (cwd: string) =>
    invoke<{ workKind: string | null; typicalMs: number | null }>('job_timing', { cwd }),
  cancelWorkflow: (slug: string) =>
    invoke<void>('cancel_workflow', { slug }),
  // Run queue (Paperclip PR 7): live rows then the newest `recent` finished
  // ones; cancel maps by status (skip a queued cron occurrence, cancel other
  // queued rows, stop a claimed or running workflow).
  listRunQueue: (recent?: number) =>
    invoke<RunQueueItem[]>('list_run_queue', { recent: recent ?? null }),
  cancelQueuedRun: (id: number) =>
    invoke<RunQueueItem>('cancel_queued_run', { id }),

  // Notifications — in-app notification center, backed by
  // vault/.notifications.jsonl. The toast stack listens on the
  // `notification-event` Tauri event for new entries; the bell-icon
  // dropdown calls `listNotifications` on open and refreshes when the
  // event fires.
  // The newest 200 and the file's total (P0-08: "Newest 200 of N").
  listNotifications: () => invoke<{ items: Notification[]; total: number }>('list_notifications'),
  // Scoped to the ids a list shows (P0-08); a failure still waiting in
  // Review is never marked read or removed by these.
  markNotificationsRead: (ids: string[]) => invoke<number>('mark_notifications_read', { ids }),
  deleteNotifications: (ids: string[]) => invoke<number>('delete_notifications', { ids }),
  unreadNotificationCount: () => invoke<number>('unread_notification_count'),
  markNotificationRead: (id: string) =>
    invoke<void>('mark_notification_read', { id }),
  markAllNotificationsRead: () =>
    invoke<number>('mark_all_notifications_read'),
  deleteNotification: (id: string) =>
    invoke<void>('delete_notification', { id }),
  clearNotifications: () => invoke<number>('clear_notifications'),
  createNotification: (input: NotificationInput) =>
    invoke<Notification>('create_notification', { input }),
  setNotifyNativeEnabled: (enabled: boolean) =>
    invoke<void>('set_notify_native_enabled', { enabled }),
  // Paperclip PR 22: the notification scope the user is watching
  // (`workflow:<slug>` while that workflow is open in Automations, null
  // otherwise). A notification opening exactly that scope skips its native
  // banner while the main window is focused.
  setForegroundNotificationScope: (scope: string | null) =>
    invoke<void>('set_foreground_notification_scope', { scope }),
  // 0.4: a chat finished or needs you. A native banner only (no notification record),
  // honouring the macOS system notifications switch; true when it was shown.
  announceChatNative: (title: string, body: string) =>
    invoke<boolean>('announce_chat_native', { title, body }),
  // 0.4: the dock badge count (0 clears it). macOS only; elsewhere a no-op.
  setDockBadge: (count: number) =>
    invoke<void>('set_dock_badge', { count }),
  // Keep-running-on-close: gate whether closing the main window hides to the
  // tray (true) or quits the app (false).
  setMinimizeToTray: (enabled: boolean) =>
    invoke<void>('set_minimize_to_tray', { enabled }),
  // Launch-at-login. set returns the resulting state (the OS login item is the
  // source of truth); status reads the current state.
  setRunOnStartup: (enabled: boolean) =>
    invoke<boolean>('set_run_on_startup', { enabled }),
  runOnStartupStatus: () => invoke<boolean>('run_on_startup_status'),
  // Proactive nudges: gate the background vault scan that fires
  // meeting-soon / frog / cold-project notifications.
  setNudgesEnabled: (enabled: boolean) =>
    invoke<void>('set_nudges_enabled', { enabled }),
  runNudgeScan: () => invoke<number>('run_nudge_scan'),
  // Silent agentic heartbeat — periodic autonomous review/action.
  // `switched`: the person turned it, so off also drops beats waiting for a
  // self-hosted model (self-hosted models Q10 follow-up); the start-up sync
  // passes false.
  setHeartbeatEnabled: (enabled: boolean, switched = false) =>
    invoke<void>('set_heartbeat_enabled', { enabled, switched }),
  setHeartbeatDraftsEnabled: (enabled: boolean) =>
    invoke<void>('set_heartbeat_drafts_enabled', { enabled }),
  setHeartbeatConfig: (cfg: HeartbeatConfig) =>
    invoke<void>('set_heartbeat_config', {
      workHoursStart: cfg.workHoursStart,
      workHoursEnd: cfg.workHoursEnd,
      minIntervalMin: cfg.minIntervalMin,
      dailyBudget: cfg.dailyBudget,
    }),
  runHeartbeatNow: () => invoke<number>('run_heartbeat_now'),
  // Teamwork: gate whether Claude chat/workflow runs attach the team-server.
  setTeamworkEnabled: (enabled: boolean) =>
    invoke<void>('set_teamwork_enabled', { enabled }),
  // Teammate role presets — ride to the team MCP server via spawn env.
  setAgentRole: (agent: string, role: string, brief: string) =>
    invoke<void>('set_agent_role', { agent, role, brief }),
  // Computer control: gate whether Claude chat runs attach the desktop-verbs
  // MCP server (open URLs/apps + confirm-gated AppleScript reads/quits).
  // The two target switches (sandbox Mac, This Mac's screen) ride the same
  // call, so the launch IPC budget is unchanged (CU7).
  setComputerControlEnabled: (enabled: boolean, sandbox?: boolean, thisMac?: boolean) =>
    invoke<void>('set_computer_control_enabled', { enabled, sandbox, thisMac }),
  // The Computer control sheet's status and its one action (CU7).
  computerStatus: () => invoke<ComputerStatus>('computer_status'),
  computerStopAll: () => invoke<number>('computer_stop_all'),
  // The Doctor's checks, each with a copyable fix; reads and installs nothing (CU10).
  computerDoctor: () => invoke<ComputerCheck[]>('computer_doctor'),
  // One chat's desktop: its state and newest frame, and the person's controls (CU8).
  computerChatState: (scope: string) => invoke<ComputerChatView>('computer_chat_state', { scope }),
  computerChatFrame: (scope: string, id?: number) => invoke<string | null>('computer_chat_frame', { scope, id }),
  computerChatControl: (scope: string, action: string) => invoke<ComputerChatView>('computer_chat_control', { scope, action }),
  // This Mac's default-denied apps and the person's allowances (CU6).
  computerDeniedApps: () => invoke<DeniedApp[]>('computer_denied_apps'),
  computerSetAppAllowed: (label: string, allowed: boolean) =>
    invoke<DeniedApp[]>('computer_set_app_allowed', { label, allowed }),
  // Hosts agents denied for themselves (ND1): the person lists and removes them.
  hostScopeAgentDenied: () => invoke<AgentDeniedHost[]>('host_scope_agent_denied'),
  hostScopeRemoveAgentDeny: (pattern: string) => invoke<AgentDeniedHost[]>('host_scope_remove_agent_deny', { pattern }),
  // Ask your vault: gate whether Claude chat runs attach the read-only
  // `hollow_vault` search server (search_vault grounds answers in the vault).
  setVaultMcpEnabled: (enabled: boolean) =>
    invoke<void>('set_vault_mcp_enabled', { enabled }),
  // Look up code (CC2): gate the read-only `hollow_code_context` server.
  setCodeContextMcpEnabled: (enabled: boolean) =>
    invoke<void>('set_code_context_mcp_enabled', { enabled }),
  // Task tools (Paperclip PR 12): whether chats and automations get the
  // hollow_tasks server. Persisted by the backend (agent-tools.json).
  setTasksMcpEnabled: (enabled: boolean) =>
    invoke<void>('set_tasks_mcp_enabled', { enabled }),
  tasksMcpEnabled: () => invoke<boolean>('tasks_mcp_enabled'),
  // Hollow app tools (PR A1): whether chats and automations get the
  // hollow_app server. Persisted by the backend (agent-tools.json).
  setAppMcpEnabled: (enabled: boolean) =>
    invoke<void>('set_app_mcp_enabled', { enabled }),
  appMcpEnabled: () => invoke<boolean>('app_mcp_enabled'),
  // Activity Ledger: unified newest-first feed of what the agents did
  // (heartbeat + computer/m365 audit + workflow run logs).
  listActivityLedger: (limit?: number) =>
    invoke<LedgerEntry[]>('list_activity_ledger', { limit }),
  backupHealth: (managedElsewhere = false) =>
    invoke<BackupHealth>('backup_health', { managedElsewhere }),
  // Liquid Glass: apply/clear the native macOS window vibrancy
  // (NSVisualEffectView) behind the transparent main window. `dark` selects
  // the material appearance. No-op on non-macOS.
  setWindowVibrancy: (enabled: boolean, dark: boolean) =>
    invoke<void>('set_window_vibrancy', { enabled, dark }),

  // Markdown preview windows — opened by the OS for *.md files (file
  // association → RunEvent::Opened → a md-preview-N window). Each window
  // identifies its file by its own label (getCurrentWindow().label).
  takePreviewFile: (label: string) =>
    invoke<PreviewFile>('take_preview_file', { label }),
  savePreviewToVault: (label: string) =>
    invoke<SavedFile>('save_preview_to_vault', { label }),
  // Updates, People (row U9, update_client/people_admin.rs): the publisher's
  // Mac only (UpdateSettings.publisher). Each change runs on the release
  // builder over ssh and can take seconds. updatePeopleAdd's installLink
  // holds the new person's token: show it once with Copy, never store it.
  // updateSetPublisher (the switch) sits with Publish to Main above.
  updatePeopleList: () => invoke<import('./generated/ipc').UpdatePeopleList>('update_people_list'),
  updatePeopleAdd: (name: string, channels: import('./generated/ipc').UpdateChannel[]) => invoke<import('./generated/ipc').UpdatePersonAdded>('update_people_add', { name, channels }),
  updatePeopleRevoke: (id: string) => invoke<import('./generated/ipc').UpdatePerson>('update_people_revoke', { id }),
  /** [] leaves the person with no channel (their checks then say so). */
  updatePeopleSetChannels: (id: string, channels: import('./generated/ipc').UpdateChannel[]) => invoke<import('./generated/ipc').UpdatePerson>('update_people_set_channels', { id, channels }),

  // Updates, Go back (row U6, update_client/goback_run.rs). None is an
  // agent action. updateGoBack answers `refused {reason}` instead of
  // throwing when Go back can't happen; running work is
  // `workRunning`/`workChanged`, as updateInstallNow's.
  updateGoBackPlan: () => invoke<import('./generated/ipc').UpdateGoBackPlan>('update_go_back_plan'),
  /** restoreData: "Go back and restore data from before the update" (plan.schema.verdict === 'needsRestore'). */
  updateGoBack: (restoreData: boolean, confirmedKeys?: string[] | null) =>
    invoke<import('./generated/ipc').UpdateGoBackOutcome>('update_go_back', { restoreData, confirmedKeys: confirmedKeys ?? null }),
  /** Go back when chats finish; updateCancelWait cancels it (state.goBackWaiting). */
  updateGoBackWhenIdle: (restoreData: boolean) =>
    invoke<import('./generated/ipc').UpdateState>('update_go_back_when_idle', { restoreData }),
  /** Skipping 0.6.n: Undo. A check follows. */
  updateUnskip: () => invoke<import('./generated/ipc').UpdateState>('update_unskip'),
};
