// Generated from the Rust IPC types by src-tauri/src/ts_bindings.rs (ts-rs).
// Do not edit by hand: change the Rust type, then from src-tauri/ run
//   HOLLOW_REGENERATE_IPC=1 cargo test --lib ts_bindings
// `cargo test --lib` fails while this file is stale.

// hollow_lib::peer_client::Accepted
export type Accepted = {
  accepted: boolean,
  state: string,
  queueId: number,
  waitingFor: string | null,
  forwardedTo: string | null,
};

// hollow_lib::ts_bindings::ActivityActor
export type ActivityActor = "run" | "chat" | "person" | "agent";

// hollow_lib::workspace_activity::ActivityCanvases
export type ActivityCanvases = {
  count: number,
  bytes: number,
};

// hollow_lib::workspace_activity::ActivityEvent
export type ActivityEvent = {
  id: string,
  kind: ActivityKind,
  at: string,
  status: ActivityStatus,
  title: string,
  sessionKey: string | null,
  runId: string | null,
  receiptId: string | null,
  path: string | null,
  session: ActivitySession | null,
  run: ActivityRun | null,
  result: ActivityResult | null,
  install: ActivityInstall | null,
};

// hollow_lib::workspace_activity::ActivityGaps
export type ActivityGaps = {
  runsWithoutReceipt: number,
  runsWithoutUsage: number,
  resultsWithoutReceipt: number,
  turnsTruncated: boolean,
  resultsTruncated: boolean,
  receiptsError: string | null,
  reviewsError: string | null,
  usageError: string | null,
};

// hollow_lib::workspace_activity::ActivityInstall
export type ActivityInstall = {
  step: "build" | "install",
  commit: string | null,
  branch: string | null,
  summary: string | null,
  matchesRunning: boolean | null,
};

// hollow_lib::ts_bindings::ActivityKind
export type ActivityKind = "session" | "run" | "result" | "install";

// hollow_lib::workspace_activity::ActivityResult
export type ActivityResult = {
  name: string,
  kind: string,
  exists: boolean,
  actor: ActivityActor | null,
  revision: number | null,
  note: string | null,
};

// hollow_lib::workspace_activity::ActivityRun
export type ActivityRun = {
  source: string,
  sourceId: string | null,
  turn: number | null,
  startedAt: string | null,
  endedAt: string | null,
  agent: string | null,
  model: string | null,
  costUsd: number | null,
  costStatus: string | null,
  inputTokens: number | null,
  outputTokens: number | null,
  files: number | null,
  outputs: number | null,
  checks: number | null,
  failedChecks: number | null,
  verification: string | null,
  resultKind: string | null,
  canvases: ActivityCanvases | null,
};

// hollow_lib::workspace_activity::ActivitySession
export type ActivitySession = {
  agent: string,
  turns: number,
  lastActivityAt: string,
};

// hollow_lib::ts_bindings::ActivityStatus
export type ActivityStatus = "started" | "running" | "succeeded" | "failed" | "stopped" | "interrupted" | "unknown" | "created" | "modified" | "deleted" | "added" | "edited" | "pending" | "draft" | "ready" | "accepted" | "building" | "built" | "installing" | "installed";

// hollow_lib::subagents::activity::ActivityStep
export type ActivityStep = {
  name: string,
  target: string,
  status: string,
};

// hollow_lib::agent_cli::AgentCliStatus
export type AgentCliStatus = {
  agent: string,
  label: string,
  installed: boolean,
  version: string | null,
  latest: string | null,
  updateAvailable: boolean,
  method: string,
  updateCommand: string | null,
  canUpdate: boolean,
  canCheck: boolean,
  previousVersion: string | null,
  canRestore: boolean,
  safeWhileRunning: boolean,
  policy: string,
  lastChecked: string | null,
  state: string,
  inUse: number,
  message: string | null,
  tone: string | null,
  knownBad: string | null,
  latestKnownBad: string | null,
};

// hollow_lib::types::AgentSessionDetail
export type AgentSessionDetail = {
  summary: AgentSessionSummary,
  turns: Array<SessionRecordTurn>,
  eventsRetained: boolean,
};

// hollow_lib::types::AgentSessionSummary
export type AgentSessionSummary = {
  key: string,
  title: string | null,
  agent: string,
  harnessSessionId: string | null,
  cwd: string,
  createdAt: string,
  lastActivityAt: string,
  endedAt: string | null,
  turnCount: number,
  parentKey: string | null,
  forkKind: string | null,
  adopted: boolean,
  stints: Array<HarnessStint>,
  resumable: SessionResumability,
  imported?: ImportedProvenance,
};

// hollow_lib::agent_settings::AgentSettings
export type AgentSettings = {
  defaultAgent: string | null,
  models: { [key in string]: string },
  efforts?: { [key in string]: string },
  codexNoOutputTimeoutMin?: number | null,
  noOutputTimeoutMin?: { [key in string]: number | null },
  deadlineMin?: { [key in string]: number },
  canvases?: CanvasFrequency,
  compactWindow?: number,
  whenOut?: string,
  runsOn?: string,
  unattendedRunsOn?: string,
  buildsOn?: string,
  appBuildsOn?: string,
  runnerDefaultsFrozen?: boolean,
  moveOnSleep?: boolean,
  cooDispatcher?: string,
  acp?: { [key in string]: boolean },
};

// hollow_lib::shell_path::AgentToolPath
export type AgentToolPath = {
  entries: Array<AgentToolPathEntry>,
  shell: ShellPathRead,
};

// hollow_lib::shell_path::AgentToolPathEntry
export type AgentToolPathEntry = {
  dir: string,
  from: ToolPathFrom,
  exists: boolean | null,
};

// hollow_lib::audit::AuditEntry
export type AuditEntry = {
  id: string,
  at: string,
  kind: string,
  sessionKey: string | null,
  parentKey: string | null,
  rootKey: string | null,
  askedBy: string | null,
  agent: string | null,
  model: string | null,
  effort: string | null,
  account: string | null,
  workspaceId: string | null,
  issueId: string | null,
  issueKey: string | null,
  action: string | null,
  fields: Record<string, unknown> | null,
  outcome: string | null,
  detail: string | null,
  costUsd: number | null,
  tokensIn: number | null,
  tokensOut: number | null,
  workingMs: number | null,
  requestKey: string | null,
  prevHash: string,
  hash: string,
};

// hollow_lib::schedule_windows::AutomationWindows
export type AutomationWindows = {
  windows: Array<WindowView>,
  overnightCapUsd: number,
  timeZone: string,
  nightPlan?: NightPlan,
};

// hollow_lib::types::BlockerWarning
export type BlockerWarning = {
  url: string,
  kind: BlockerWarningKind,
};

// hollow_lib::ts_bindings::BlockerWarningKind
export type BlockerWarningKind = "missing" | "cancelled" | "recurring" | "cycle";

// hollow_lib::workspace_reentry::BriefStale
export type BriefStale = {
  days: number,
  conversations: number,
};

// hollow_lib::build_workspaces::BuildInfo
export type BuildInfo = {
  repo: string,
  worktree: string,
  branch: string,
  base: string,
  exists: boolean,
  runsOn?: string,
};

// hollow_lib::workspace_canvases::Canvas
export type Canvas = {
  scope: CanvasScope,
  workspaceId: string | null,
  sessionKey: string | null,
  id: string,
  title: string,
  kit: string | null,
  session: string | null,
  fenceSha256: string,
  createdAt: string,
  updatedAt: string,
  revision: number,
  ask: boolean,
  height: number | null,
  html: string,
  sha256: string,
  path: string | null,
  file: string | null,
  htmlChanged: boolean,
  state: unknown,
  stateSavedAt: string | null,
  revisions: Array<CanvasRevision>,
};

// hollow_lib::ts_bindings::CanvasFrequency
export type CanvasFrequency = "off" | "helps" | "most";

// hollow_lib::canvas_items::CanvasItemSource
export type CanvasItemSource = {
  canvas: Canvas,
  project: string | null,
  projectUrl: string | null,
};

// hollow_lib::canvas_items::CanvasPlanItems
export type CanvasPlanItems = {
  workspaceId: string,
  id: string,
  revision: number,
  heading: string,
  rows: Array<[number, string]>,
  expectedPreview: string | null,
};

// hollow_lib::ts_bindings::CanvasRecordOutcome
export type CanvasRecordOutcome = "created" | "revised" | "unchanged" | "retired";

// hollow_lib::workspace_canvases::CanvasRecorded
export type CanvasRecorded = {
  outcome: CanvasRecordOutcome,
  matchedRevision: number,
  canvas: Canvas,
};

// hollow_lib::workspace_canvases::CanvasRevision
export type CanvasRevision = {
  revision: number,
  fenceSha256: string,
  title: string,
  savedAt: string,
  ask: boolean,
  height: number | null,
  session: string | null,
  turnId: string | null,
};

// hollow_lib::workspace_canvases::CanvasRevisionHtml
export type CanvasRevisionHtml = {
  revision: number,
  fenceSha256: string,
  title: string,
  html: string,
};

// hollow_lib::ts_bindings::CanvasScope
export type CanvasScope = "workspace" | "session";

// hollow_lib::workspace_canvases::CanvasStateSaved
export type CanvasStateSaved = {
  sha256: string,
  stateSavedAt: string | null,
  revision: number,
};

// hollow_lib::workspace_canvases::CanvasSummary
export type CanvasSummary = {
  id: string,
  title: string,
  revision: number,
  updatedAt: string | null,
  fenceSha256: string | null,
  path: string | null,
  file: string | null,
  error: string | null,
};

// hollow_lib::cfo::accounts::CfoAccount
export type CfoAccount = {
  agent: string,
  account: string,
  label: string,
  fiveHourPercent: number | null,
  weeklyPercent: number | null,
  fiveHourResetsAt: string | null,
  weeklyResetsAt: string | null,
  room: boolean,
  roomAt: string | null,
  note: string,
};

// hollow_lib::cfo::settings::AllowEntry
export type CfoAllowEntry = {
  agent: string,
  model?: string,
};

// hollow_lib::cfo::Decision
export type CfoDecision = {
  agent: string,
  model?: string,
  effort?: string,
  effortSource: CfoEffortSource,
  account?: string,
  kind: CfoJobKind,
  reason: string,
};

// hollow_lib::cfo::settings::EffortMode
export type CfoEffortMode = "automatic" | "fixed";

// hollow_lib::cfo::effort::EffortSource
export type CfoEffortSource = "you" | "agent" | "cfo" | "fixed";

// hollow_lib::cfo::job::JobKind
export type CfoJobKind = "conversation" | "question" | "plan" | "research" | "code" | "review" | "report";

// hollow_lib::cfo::CfoKindLevel
export type CfoKindLevel = {
  kind: CfoJobKind,
  level: string,
};

// hollow_lib::cfo::settings::Patch
export type CfoPatch = {
  allow?: Array<CfoAllowEntry>,
  effortMode?: CfoEffortMode,
  reservePercent?: number,
  projectBudgets?: Array<CfoProjectBudget>,
  subagentBudget?: CfoSubagentBudget,
};

// hollow_lib::cfo::settings::ProjectBudget
export type CfoProjectBudget = {
  workspaceId: string,
  fiveHourTokens?: number,
  weekTokens?: number,
};

// hollow_lib::cfo::CfoProjectUsage
export type CfoProjectUsage = {
  workspaceId: string,
  fiveHourTokens: number,
  weekTokens: number,
  reached: boolean,
};

// hollow_lib::cfo::settings::CfoSettings
export type CfoSettings = {
  allow: Array<CfoAllowEntry>,
  effortMode: CfoEffortMode,
  reservePercent: number,
  extraUsageUntil?: string,
  projectBudgets: Array<CfoProjectBudget>,
  subagentBudget: CfoSubagentBudget,
};

// hollow_lib::cfo::settings::SubagentBudget
export type CfoSubagentBudget = {
  tokens?: number,
  minutes?: number,
};

// hollow_lib::cfo::CfoView
export type CfoView = {
  settings: CfoSettings,
  poolCeilingPercent: number,
  extraUsageOpen: boolean,
  accounts: Array<CfoAccount>,
  levels: Array<CfoKindLevel>,
  reserveChoices: Array<number>,
  projectUsage: Array<CfoProjectUsage>,
};

// hollow_lib::chat_chain::ChainLink
export type ChainLink = {
  key: string,
  title: string | null,
  createdAt: string,
};

// hollow_lib::chat_chain::ChainView
export type ChainView = {
  key: string,
  current: string,
  currentTitle: string | null,
  main: boolean,
  earlier: Array<ChainLink>,
  forwards: Array<ForwardNote>,
};

// hollow_lib::chat_move_trigger::ChatMoveResult
export type ChatMoveResult = {
  key: string,
  outcome: MoveOutcome,
  to: string | null,
  message: string,
  continued: boolean,
};

// hollow_lib::sync_engine::clash::ClashResolved
export type ClashResolved = {
  path: string,
  text: string,
  open: boolean,
};

// hollow_lib::sync_engine::clash::ClashSide
export type ClashSide = {
  lines: Array<string>,
  totalLines: number,
  unreadable: string | null,
};

// hollow_lib::conflict_copies::ConflictCopy
export type ConflictCopy = {
  copy: string,
  original: string,
  identical: boolean,
  copyModified: string,
  originalModified: string,
  copySize: number,
  originalSize: number,
  originalProtected: boolean,
};

// hollow_lib::conflict_copies::ConflictPair
export type ConflictPair = {
  copy: string,
  original: string,
  copyText: string,
  originalText: string,
  copyHash: string,
  originalHash: string,
};

// hollow_lib::ts_bindings::ConnectionKind
export type ConnectionKind = "project" | "people" | "link" | "source" | "task" | "terms" | "chat";

// hollow_lib::workspace_connections::ConnectionReason
export type ConnectionReason = {
  kind: ConnectionKind,
  detail: string,
  weight: number,
};

// hollow_lib::context_rank::Basis
export type ConnectorBasis = "named" | "jev" | "local" | "local-fallback" | "unscored" | "beyond-limit" | "cached";

// hollow_lib::context_rank::ConnectorDecision
export type ConnectorDecision = {
  id: string,
  attach: boolean,
  basis: ConnectorBasis,
  score?: number,
  reason?: string,
};

// hollow_lib::context_rank::ConnectorSelection
export type ConnectorSelection = {
  engine: string,
  decisions: Array<ConnectorDecision>,
  note?: string,
};

// hollow_lib::workspaces::ContextDocHealth
export type ContextDocHealth = {
  name: string,
  bytes: number,
  included: number,
  limit: number,
};

// hollow_lib::context_review::ContextReviewAccepted
export type ContextReviewAccepted = {
  id: string,
  layer: ContextReviewLayer,
  target: string | null,
  written: boolean,
  prUrl: string | null,
  prStatus: string | null,
  prReason: string | null,
};

// hollow_lib::context_review::ContextReviewChoice
export type ContextReviewChoice = {
  layer: ContextReviewLayer,
  target: string | null,
  disabled: "no-personal-root" | "no-org-root" | "no-machine" | "target-exists" | "refused" | "not-a-project-file" | "org-only" | "stale" | null,
  detail: string | null,
  outward: boolean,
};

// hollow_lib::context_review::ContextReviewFlag
export type ContextReviewFlag = {
  rule: string,
  group: string,
  severity: "warn" | "block",
  line: number,
};

// hollow_lib::context_review::ContextReviewItem
export type ContextReviewItem = {
  id: string,
  source: "scan" | "org",
  agent: string | null,
  kind: string,
  scope: string | null,
  rootLabel: string,
  path: string,
  relPath: string,
  size: number,
  modified: number | null,
  excerpt: string | null,
  withheld: string | null,
  secretLike: boolean,
  flags: Array<ContextReviewFlag>,
  scanNotes: Array<string>,
  duplicateOf: string | null,
  chat: string | null,
  proposedLayer: ContextReviewLayer,
  sort: ContextReviewSort | null,
  choices: Array<ContextReviewChoice>,
  blocked: "secret" | "block-flag" | "unread" | null,
  blockedRule: string | null,
  skipped: boolean,
};

// hollow_lib::context_review::ContextReviewLayer
export type ContextReviewLayer = "personal" | "project" | "org" | "home" | "stale";

// hollow_lib::context_review::ContextReviewList
export type ContextReviewList = {
  scanId: string | null,
  items: Array<ContextReviewItem>,
  accepted: number,
};

// hollow_lib::context_review::ContextReviewSort
export type ContextReviewSort = {
  layer: ContextReviewLayer,
  rule: string,
  reason: string,
};

// hollow_lib::extract::ExtractionSettings
export type ExtractionSettings = {
  models: { [key in string]: string },
  retitleChats: boolean,
};

// hollow_lib::extract::ExtractionView
export type ExtractionView = {
  agent: string,
  chosen: string,
  asks: string | null,
  retitleChats: boolean,
};

// hollow_lib::chat_chain::ForwardNote
export type ForwardNote = {
  from: string,
  to: string,
  at: string,
  preview: string,
};

// hollow_lib::types::HarnessStint
export type HarnessStint = {
  agent: string,
  harnessSessionId: string | null,
  startedAt: string,
  endedAt: string | null,
  reason: string | null,
};

// hollow_lib::automated_history::HistoryChange
export type HistoryChange = {
  vaultPath: string,
  changeType: string,
  beforeHash: string | null,
  afterHash: string | null,
  beforeSize: number | null,
  afterSize: number | null,
  text: boolean,
  notionMirror: boolean,
  beforeNotionId?: string,
  afterNotionId?: string,
  beforeUncaptured?: boolean,
  afterUncaptured?: boolean,
  beforeCoverage?: HistoryCoverage,
  afterCoverage?: HistoryCoverage,
  restoredAt?: string,
  restoredByRunId?: string,
  alsoChangedBy?: Array<string>,
  unattributed?: boolean,
  unverified?: boolean,
};

// hollow_lib::automated_history::Coverage
export type HistoryCoverage = { "state": "absent" } | { "state": "captured" } | { "state": "uncaptured", reason: HistoryUncapturedReason | null, } | { "state": "unknown" };

// hollow_lib::automated_history::HistoryRun
export type HistoryRun = {
  id: string,
  source: string,
  sourceId: string | null,
  startedAt: string,
  finishedAt: string | null,
  status: string,
  changes: Array<HistoryChange>,
  flags?: Array<RunFlag>,
  scope?: string,
  overlaps?: Array<string>,
};

// hollow_lib::automated_history::HistoryStorageStatus
export type HistoryStorageStatus = {
  runCount: number,
  activeRunCount: number,
  objectCount: number,
  totalBytes: number,
  objectBytes: number,
  maxRuns: number,
  maxAgeDays: number,
  maxBytes: number,
  oldestRunAt: string | null,
  newestRunAt: string | null,
};

// hollow_lib::automated_history::UncapturedReason
export type HistoryUncapturedReason = "tooLarge" | "unreadable" | "excluded";

// hollow_lib::chat_move::HomeRow
export type HomeRow = {
  key: string,
  state: string,
  peer: string,
  at: string,
  waiting: boolean,
};

// hollow_lib::types::ImportedProvenance
export type ImportedProvenance = {
  source: string,
  label: string,
  externalId: string,
  version: number,
  originalCreatedAt: string | null,
  originalLastAt: string | null,
  importedAt: string,
  updatedAt: string | null,
  note: string | null,
  keepOutOfSearch: boolean,
  sourceCwd: string | null,
  importId: string,
};

// hollow_lib::restart_resume::InterruptedChat
export type InterruptedChat = {
  sessionKey: string,
  title: string,
  agent: string,
  workspaceId: string | null,
  askedAt: string,
  model: string | null,
  permissionMode: string | null,
  agentStarted: boolean,
};

// hollow_lib::jev_decide::Report
export type JevDecisionsReport = {
  at: string,
  engine: string,
  subject: string,
  checked: number,
  dropped: number,
  flagged: number,
  note: string | null,
  job: string,
  runId: string | null,
  request: ManifestJevUse | null,
};

// hollow_lib::workspace_results::LatestResult
export type LatestResult = {
  path: string,
  title: string,
  kind: string,
  modified: string | null,
  review: ResultReview,
};

// hollow_lib::vault_learning::LearningBatchItem
export type LearningBatchItem = {
  id: string,
  ok: boolean,
  status: string | null,
  error: string | null,
};

// hollow_lib::vault_learning::LearningBatchOutcome
export type LearningBatchOutcome = {
  batchId: string | null,
  action: string,
  items: Array<LearningBatchItem>,
};

// hollow_lib::vault_learning::LearningSettings
export type LearningSettings = {
  suggestions: Suggestions,
};

// hollow_lib::workspace_results::LibraryResult
export type LibraryResult = {
  workspaceId: string,
  workspaceTitle: string,
  path: string,
  name: string,
  title: string,
  kind: string,
  size: number,
  modified: string | null,
  sha256: string | null,
  review: ResultReview,
  acceptedSha256: string | null,
  acceptedAt: string | null,
  readyAt: string | null,
  hasSnapshot: boolean,
  acceptance: ResultAcceptance | null,
  history: Array<ReviewEvent>,
  origin: ResultOrigin | null,
  checks: ResultChecks,
  canvas?: ResultCanvas,
};

// hollow_lib::commands::peer_api::MachineRef
export type MachineRef = {
  machineId: string,
  slug: string | null,
  name: string,
};

// hollow_lib::run_manifest::AgentReads
export type ManifestAgentReads = {
  readSource: number,
  readSourceFailed: number,
  readSourceQueried: number,
  searchVault: number,
  notesOpened?: Array<string>,
};

// hollow_lib::run_manifest::AttachedFileRecord
export type ManifestAttachedFile = {
  path: string,
  coverage: string,
  lines: number,
  totalBytes: number,
  includedBytes: number,
  sha256: string,
};

// hollow_lib::run_manifest::AttachedSourcesRecord
export type ManifestAttachedSources = {
  budget: number,
  used: number,
  reader: string,
  terms: number,
  bytesSearched: number,
  chunksTotal: number,
  chunksRelevant: number,
  chunksUnrecorded: number,
  trimmed: boolean,
  files: Array<ManifestAttachedFile>,
  chunks: Array<ManifestChunkDecision>,
  ms: number,
};

// hollow_lib::run_manifest::AutoMemoryRecord
export type ManifestAutoMemory = {
  dir: string,
  exists: boolean,
  memoryMdBytes: number,
  memoryMdLines: number,
  notes: number,
};

// hollow_lib::run_manifest::BudgetRecord
export type ManifestBudget = {
  provider: string,
  total: number,
  configuredChannel: string,
  used: number,
  over: boolean,
  layers: Array<ManifestBudgetLayer>,
};

// hollow_lib::run_manifest::BudgetLayer
export type ManifestBudgetLayer = {
  name: string,
  tier: string,
  wanted: number,
  granted: number,
};

// hollow_lib::run_manifest::Capabilities
export type ManifestCapabilities = {
  ownedServers: Array<string>,
  connectors: Array<string>,
  bashOutputMaxChars?: number,
  compactWindow?: number,
  compactions?: number,
};

// hollow_lib::run_manifest::ChunkDecision
export type ManifestChunkDecision = {
  path: string,
  fromLine: number,
  toLine: number,
  score: number,
  kept: boolean,
  bytes: number,
  reason?: string,
};

// hollow_lib::run_manifest::CompilerRecord
export type ManifestCompiler = {
  entry: string,
  session: string,
  resetReason?: string,
  resetParts?: Array<string>,
  profile: ManifestProfile,
  continuity: ManifestContinuity,
  taskState: ManifestTaskState,
  profileDelta: ManifestProfileDelta,
  visualGuide?: ManifestVisualGuide,
  budget?: ManifestBudget,
  stableLayers?: ManifestStableLayers,
};

// hollow_lib::run_manifest::ContinuityRecord
export type ManifestContinuity = {
  sent: boolean,
  available: number,
  entries: number,
  bytes: number,
  continuedFrom: number,
  seeded: number,
};

// hollow_lib::run_manifest::CostRollup
export type ManifestCostRollup = {
  ownUsd: number | null,
  descendantsUsd: number | null,
  totalUsd: number | null,
  knownUsd: number,
  unknown: number,
  gpuTime?: number,
  descendants: Array<ManifestDescendant>,
};

// hollow_lib::run_manifest::DelegationRecord
export type ManifestDelegation = {
  kind: "consultation" | "childTask",
  tool: string,
  agent: string,
  childSession?: string,
  childQueueId?: number,
  packetBytes: number,
  packetSha256: string,
  constraints: number,
  sources: number,
  fromTaskState: number,
  omitted: number,
  budgetSeconds?: number,
  budgetUsd?: number,
  deadlineSeconds?: number,
  at: string,
  outcome?: "ok" | "timeout" | "error" | "cancelled",
  durationMs?: number,
  returnedBytes?: number,
  incomplete?: boolean,
};

// hollow_lib::run_manifest::Descendant
export type ManifestDescendant = {
  kind: "consultation" | "childTask" | "helper",
  agent?: string,
  status: string,
  costUsd: number | null,
  purpose?: string,
  childSession?: string,
  childQueueId?: number,
  childRunId?: string,
  stop?: string,
};

// hollow_lib::run_manifest::Fallback
export type ManifestFallback = {
  stage: string,
  detail: string,
};

// hollow_lib::run_manifest::JevCalls
export type ManifestJevCalls = {
  sent: number,
  notSent: number,
  latencyMs: number,
  outcomes: { [key in string]: number },
  costUsd: number | null,
};

// hollow_lib::run_manifest::JevUse
export type ManifestJevUse = {
  use: string,
  outcome: string,
  items: number,
  latencyMs: number | null,
  costUsd: number | null,
};

// hollow_lib::run_manifest::Layer
export type ManifestLayer = {
  name: string,
  bytes: number,
  sha256: string,
};

// hollow_lib::run_manifest::LifecycleRecord
export type ManifestLifecycle = {
  history: boolean,
  cue?: string,
  decisions: Array<ManifestLifecycleDecision>,
  unrecorded: number,
};

// hollow_lib::run_manifest::LifecycleDecision
export type ManifestLifecycleDecision = {
  route: string,
  path: string,
  status: string,
  action: string,
  current?: string,
  reason: string,
};

// hollow_lib::run_manifest::NativeFileRecord
export type ManifestNativeFile = {
  scope: string,
  path: string,
  bytes: number,
};

// hollow_lib::run_manifest::NativeMemoryRecord
export type ManifestNativeMemory = {
  configDir: string,
  instructions: Array<ManifestNativeFile>,
  instructionBytes: number,
  autoMemory: ManifestAutoMemory,
  skillListing: ManifestSkillListing,
};

// hollow_lib::run_manifest::PhaseTiming
export type ManifestPhaseTiming = {
  name: string,
  startMs: number | null,
  durationMs: number | null,
};

// hollow_lib::run_manifest::PrepDeadlineRecord
export type ManifestPrepDeadline = {
  budgetMs: number,
  cut: Array<string>,
};

// hollow_lib::run_manifest::PrepTimeline
export type ManifestPrepTimeline = {
  origin: string,
  totalMs: number | null,
  phases: Array<ManifestPhaseTiming>,
};

// hollow_lib::run_manifest::Preparation
export type ManifestPreparation = {
  retrievalMs?: number,
  connectorsMs?: number,
  timeline?: ManifestPrepTimeline,
  deadline?: ManifestPrepDeadline,
  codexInventory?: string,
};

// hollow_lib::run_manifest::ProfileRecord
export type ManifestProfile = {
  enabled: boolean,
  sent: boolean,
  bytes: number,
  sha256?: string,
  files: number,
  truncated: boolean,
};

// hollow_lib::run_manifest::ProfileDeltaRecord
export type ManifestProfileDelta = {
  baseline: string,
  sent: boolean,
  added: number,
  removed: number,
  files: Array<string>,
  bytes: number,
};

// hollow_lib::run_manifest::RetrievalStats
export type ManifestRetrievalStats = {
  prefilter: string,
  candidateSource?: string,
  candidates?: number,
  notesEligible: number,
  notesRead: number,
  notesMatched: number,
  bytesRead: number,
  bytesScored: number,
  stopped?: string,
  projectFilesRead: number,
  pool: number,
  budget: number,
  semantic: string,
  semanticReason?: string,
  semanticHits: number,
  semanticMs?: number,
  ms: number,
};

// hollow_lib::run_manifest::SkillIndexed
export type ManifestSkillIndexed = {
  name: string,
  source: string,
  reason: string,
  score: number,
  bytes: number,
  inline: boolean,
};

// hollow_lib::run_manifest::SkillListingRecord
export type ManifestSkillListing = {
  count: number,
  bytes: number,
};

// hollow_lib::run_manifest::SkillRead
export type ManifestSkillRead = {
  name?: string,
  source?: string,
  via: string,
  ok: boolean,
};

// hollow_lib::run_manifest::SkillsRecord
export type ManifestSkills = {
  reader: string,
  available: number,
  indexed: Array<ManifestSkillIndexed>,
  leftOut: number,
  selectedMissing: number,
  read: Array<ManifestSkillRead>,
};

// hollow_lib::run_manifest::StableLayersRecord
export type ManifestStableLayers = {
  configured: string,
  workspace: string,
  reason?: string,
  fullBytes: number,
  sentBytes: number,
  unchanged: number,
  changed: number,
  excerptsReferenced: number,
  resumedSinceFull: number,
  heldLayers?: Array<string>,
};

// hollow_lib::run_manifest::TaskStateRecord
export type ManifestTaskState = {
  revision: number,
  throughTurn: number,
  items: number,
  sent: boolean,
  bytes: number,
  merged: number,
  reground?: string,
};

// hollow_lib::run_manifest::VisualGuideRecord
export type ManifestVisualGuide = {
  delivery: string,
  cue?: string,
  file: boolean,
  bytes: number,
  sha256: string,
};

// hollow_lib::workspace_merge::MergeFile
export type MergeFile = {
  from: string,
  to: string,
  renamed: boolean,
};

// hollow_lib::workspace_merge::MergeOutcome
export type MergeOutcome = {
  plan: MergePlan,
  expiredCards: number,
  warnings: Array<string>,
};

// hollow_lib::workspace_merge::MergePlan
export type MergePlan = {
  from: string,
  fromTitle: string,
  into: string,
  intoTitle: string,
  files: Array<MergeFile>,
  sessions: Array<MergeSession>,
  left: Array<string>,
  keepsWorktree: boolean,
  hash: string,
  blocked: string | null,
};

// hollow_lib::workspace_merge::MergeSession
export type MergeSession = {
  key: string,
  title: string,
  archived: boolean,
};

// hollow_lib::chat_move_trigger::MoveOutcome
export type MoveOutcome = "moved" | "kept" | "waiting" | "refused" | "busy";

// hollow_lib::needs_you::NeedsYouItem
export type NeedsYouItem = {
  id: string,
  kind: NeedsYouKind,
  sourceId: string,
  title: string,
  detail: string | null,
  state: string,
  at: string | null,
  workspaceId: string | null,
  sessionKey: string | null,
  path: string | null,
  category: string | null,
  link: string | null,
};

// hollow_lib::ts_bindings::NeedsYouKind
export type NeedsYouKind = "approval" | "question" | "review" | "result" | "failure" | "conflict" | "canvas" | "interrupted" | "copies" | "clash" | "lesson_memory";

// hollow_lib::needs_you::NeedsYouPage
export type NeedsYouPage = {
  items: Array<NeedsYouItem>,
  total: number,
  nextCursor: string | null,
  unavailable: Array<string>,
};

// hollow_lib::schedule_windows::NightJobView
export type NightJobView = {
  id: string,
  label: string,
  window: string,
  at: string,
};

// hollow_lib::schedule_windows::NightPlan
export type NightPlan = {
  start: string,
  end: string,
  jobs: Array<NightJobView>,
};

// hollow_lib::chat_offline::OfflineRow
export type OfflineRow = {
  offlineKey: string,
  key: string,
  peer: string,
  state: string,
  note: string | null,
  partly: boolean,
};

// hollow_lib::commands::peer_api::PeerChatDetail
export type PeerChatDetail = {
  home: MachineRef,
  detail: SessionDetail,
};

// hollow_lib::commands::peer_api::PeerChatEvent
export type PeerChatEvent = {
  machineId: string,
  key: string | null,
  kind: string,
  data: unknown,
};

// hollow_lib::commands::peer_api::PeerFailure
export type PeerFailure = {
  code: PeerFailureCode,
  message: string,
  state: PeerState,
  attemptsLeft: number | null,
};

// hollow_lib::commands::peer_api::PeerFailureCode
export type PeerFailureCode = "offline" | "pairing-needed" | "update-needed" | "forbidden" | "pairing-closed" | "wrong-code" | "not-found" | "busy" | "question-closed" | "bad-request" | "too-large" | "misdirected" | "unavailable" | "local" | "internal";

// hollow_lib::commands::peer_api::PeerMachine
export type PeerMachine = {
  machineId: string,
  slug: string | null,
  name: string,
  sshHosts: Array<string>,
  remotePort: number,
  pairedAt: string,
  lastSeenAt: string | null,
  lastVersion: string | null,
  state: PeerState,
  message: string | null,
  checkedAt: string | null,
  alias: string | null,
  moves: boolean,
};

// hollow_lib::commands::peer_api::PeerMachinesView
export type PeerMachinesView = {
  thisMac: MachineRef | null,
  machines: Array<PeerMachine>,
  suggestedAlias: string | null,
  moveTo: string | null,
  sleepTo: string | null,
};

// hollow_lib::commands::peer_api::PeerSessionRow
export type PeerSessionRow = {
  home: MachineRef,
  key: string,
  title: string | null,
  agent: string,
  workspace: RemoteWorkspace | null,
  createdAt: string,
  lastActivityAt: string,
  turnCount: number,
  state: string,
  needsAnswer: boolean,
  preview: string | null,
  runsOn: string | null,
  movedTo: string | null,
  movedFrom: string | null,
};

// hollow_lib::commands::peer_api::PeerSessionsPage
export type PeerSessionsPage = {
  home: MachineRef,
  sessions: Array<PeerSessionRow>,
  truncated: boolean,
};

// hollow_lib::peer_client::PeerState
export type PeerState = "this-mac" | "online" | "connecting" | "offline" | "pairing-needed" | "update-needed";

// hollow_lib::stakeholder_sim::PersonaView
export type PersonaView = {
  role: string,
  title: string,
  path: string,
  exists: boolean,
  acceptance: string,
  sha256: string | null,
  evidence: number,
  validEvidence: number,
  text: string | null,
  cut: boolean,
};

// hollow_lib::plan_rows::PlanRowToggled
export type PlanRowToggled = {
  outcome: "written" | "unchanged",
  revision: string,
  line: number,
  lineSha256: string,
  done: boolean,
};

// hollow_lib::project_portfolio::PortfolioTracked
export type PortfolioTracked = {
  provider: string,
  projectId: string,
  url: string | null,
};

// hollow_lib::project_portfolio::PortfolioUpdate
export type PortfolioUpdate = {
  workspaceId: string,
  values: Record<string, string | string[]>,
  expectedRevision: string,
};

// hollow_lib::ts_bindings::Priority
export type Priority = "P1" | "P2" | "P3" | "P4";

// hollow_lib::project_portfolio::ProjectPortfolio
export type ProjectPortfolio = {
  workspaceId: string,
  path: string,
  exists: boolean,
  title: string,
  status: string | null,
  targetDate: string | null,
  description: string,
  tracked: PortfolioTracked | null,
  text: Record<string, string>,
  lists: Record<string, string[]>,
  revision: string,
};

// hollow_lib::workspace_reentry::LastRun
export type ReentryLastRun = {
  sessionKey: string,
  status: string | null,
  finishedAt: string | null,
  failedChecks: number | null,
  checkCount: number | null,
  unknownChecks: number | null,
  verification: string | null,
};

// hollow_lib::peer_client::RemoteLive
export type RemoteLive = {
  key: string,
  ord: number,
  gen: number,
  startedAt: string,
  text: string,
  truncated: boolean,
  tools: Array<unknown>,
};

// hollow_lib::peer_client::RemoteSession
export type RemoteSession = {
  key: string,
  title: string | null,
  agent: string,
  workspace: RemoteWorkspace | null,
  createdAt: string,
  lastActivityAt: string,
  turnCount: number,
  state: string,
  needsAnswer: boolean,
  preview: string | null,
  runsOn: string | null,
  movedTo: string | null,
  movedFrom: string | null,
};

// hollow_lib::peer_client::RemoteWorkspace
export type RemoteWorkspace = {
  id: string,
  title: string,
  kind: string,
};

// hollow_lib::subagents::activity::ReportedProgress
export type ReportedProgress = {
  completed: number,
  total: number,
};

// hollow_lib::workspace_results::Acceptance
export type ResultAcceptance = {
  actor: "person",
  sha256: string,
  bytes: number,
  runId: string | null,
  mode: string | null,
  basis: string,
  snapshot: boolean,
  at: string,
};

// hollow_lib::workspace_results::ResultCanvas
export type ResultCanvas = {
  id: string,
  revision: number | null,
};

// hollow_lib::workspace_results::ResultChecks
export type ResultChecks = {
  total: number | null,
  failed: number | null,
  unknown: number | null,
};

// hollow_lib::workspace_results::ResultOrigin
export type ResultOrigin = {
  sessionKey: string | null,
  runId: string,
  finishedAt: string,
  status: string,
  verification: string,
};

// hollow_lib::ts_bindings::ResultReview
export type ResultReview = "draft" | "ready" | "accepted" | "changed";

// hollow_lib::restart_resume::ResumeOutcome
export type ResumeOutcome = {
  sessionKey: string,
  queued: boolean,
  reason: string | null,
};

// hollow_lib::run_manifest::RetrievalCandidate
export type RetrievalCandidate = {
  path: string,
  line: number,
  source: string,
  coverage: string,
  rank: number,
  localScore: number,
  jevScore?: number,
  semanticScore?: number,
  kept: boolean,
  basis: string,
  reason?: string,
  bytes: number,
  sha256: string,
  cited?: boolean,
};

// hollow_lib::run_manifest::RetrievalRecord
export type RetrievalRecord = {
  scope?: string,
  scopeBasis?: string,
  workspace?: string,
  engine: string,
  candidates: Array<RetrievalCandidate>,
  notices: Array<string>,
  stats?: ManifestRetrievalStats,
  queryBasis?: string,
  citedChecked?: boolean,
};

// hollow_lib::ts_bindings::ReviewActor
export type ReviewActor = "person" | "agent";

// hollow_lib::workspace_results::ReviewEvent
export type ReviewEvent = {
  state: string,
  sha256: string | null,
  at: string,
  note: string | null,
  actor: ReviewActor | null,
  runId: string | null,
};

// hollow_lib::run_manifest::RunContextView
export type RunContextView = {
  runId: string,
  source: string,
  sessionKey?: string,
  finishedAt: string,
  manifest: RunManifest,
  afterRun?: Array<JevDecisionsReport>,
  jevCalls: ManifestJevCalls,
  delegations?: Array<ManifestDelegation>,
  costRollup: ManifestCostRollup,
};

// hollow_lib::run_guard::RunFlag
export type RunFlag = {
  kind: RunFlagKind,
  path: string,
  changeType: string | null,
  detail: string | null,
  scan?: string,
};

// hollow_lib::ts_bindings::RunFlagKind
export type RunFlagKind = "workflow" | "workflowSettings" | "operatorProfile" | "agentInstructions" | "agentConfig" | "stateFile" | "historyManifest" | "sharedDoc";

// hollow_lib::run_manifest::RunManifest
export type RunManifest = {
  schemaVersion: number,
  runId: string,
  layers: Array<ManifestLayer>,
  retrieval?: RetrievalRecord,
  attachedSources?: ManifestAttachedSources,
  lifecycle?: ManifestLifecycle,
  connectors?: ConnectorSelection,
  jev: Array<ManifestJevUse>,
  capabilities: ManifestCapabilities,
  fallbacks: Array<ManifestFallback>,
  preparation: ManifestPreparation,
  compiler?: ManifestCompiler,
  agentReads?: ManifestAgentReads,
  dedup?: Array<JevDecisionsReport>,
  skills?: ManifestSkills,
  nativeMemory?: ManifestNativeMemory,
  recallMeaning?: string,
  cfo?: CfoDecision,
};

// hollow_lib::types::RunQueueEvent
export type RunQueueEvent = {
  kind: RunQueueEventKind,
  item: RunQueueItem,
};

// hollow_lib::ts_bindings::RunQueueEventKind
export type RunQueueEventKind = "enqueued" | "coalesced" | "waiting" | "running" | "finished" | "cancelled" | "requeued" | "held";

// hollow_lib::types::RunQueueItem
export type RunQueueItem = {
  id: number,
  source: RunQueueSource,
  sourceId: string | null,
  label: string,
  detail: string | null,
  reason: string,
  status: RunQueueStatus,
  priority: number,
  position: number | null,
  attempt: number,
  coalescedCount: number,
  requestedAt: string,
  claimedAt: string | null,
  finishedAt: string | null,
  historyRunId: string | null,
  error: string | null,
  waitingSince: string | null,
  waitingHolder: string | null,
  heldReason: string | null,
  cancelRequested: boolean,
  waitingReason: string | null,
  waitingSession?: string,
  working?: boolean,
};

// hollow_lib::ts_bindings::RunQueueSource
export type RunQueueSource = "restrictedResume" | "chat" | "cron" | "manual" | "trigger" | "retry" | "heartbeat" | "ingest";

// hollow_lib::ts_bindings::RunQueueStatus
export type RunQueueStatus = "paused" | "queued" | "claimed" | "running" | "done" | "failed" | "cancelled" | "skipped" | "coalesced" | "interrupted";

// hollow_lib::self_hosted_registry::SelfHostedAuth
export type SelfHostedAuth = { "kind": "tokenCommand", program: string, args: Array<string>, } | { "kind": "none" };

// hollow_lib::self_hosted_registry::SelfHostedEntry
export type SelfHostedEntry = {
  id: string,
  name: string,
  baseUrl: string,
  apiStyle: string,
  servedModel: string,
  contextWindow: number,
  effortLevels: Array<string>,
  auth: SelfHostedAuth,
  healthPath: string,
  enabled: boolean,
};

// hollow_lib::self_hosted_jobs::SelfHostedHeldJob
export type SelfHostedHeldJob = {
  id: string,
  kind: SelfHostedJobKind,
  label: string,
  entry: string,
  name: string | null,
  queuedAt: string,
  attempts: number,
  lastError: string | null,
  gaveUp: boolean,
  running: boolean,
  canRunAsNow: boolean,
  canTryAgain: boolean,
  onlyOn: string | null,
};

// hollow_lib::self_hosted_jobs::JobKind
export type SelfHostedJobKind = "extraction" | "titles" | "learning" | "meetings" | "profile" | "heartbeat";

// hollow_lib::self_hosted_jobs::SelfHostedJobRow
export type SelfHostedJobRow = {
  kind: SelfHostedJobKind,
  label: string,
  about: string,
  entry: string | null,
  problem: string | null,
};

// hollow_lib::self_hosted_jobs::SelfHostedJobs
export type SelfHostedJobs = {
  kinds: Array<SelfHostedJobRow>,
  waiting: Array<SelfHostedWaiting>,
  held: Array<SelfHostedHeldJob>,
  error: string | null,
  queueError: string | null,
};

// hollow_lib::self_hosted_registry::SelfHostedModels
export type SelfHostedModels = {
  entries: Array<SelfHostedEntry>,
  preset: SelfHostedEntry,
  levels: Array<string>,
  minContextWindow: number,
  maxContextWindow: number,
  error: string | null,
  invalid: { [key in string]: string },
};

// hollow_lib::self_hosted_jobs::SelfHostedRetry
export type SelfHostedRetry = {
  started: boolean,
  underWay: boolean,
  entry: string,
  name: string | null,
  jobs: SelfHostedJobs,
};

// hollow_lib::self_hosted_registry::SelfHostedTest
export type SelfHostedTest = {
  ok: boolean,
  healthStatus: number | null,
  healthMs: number | null,
  modelsStatus: number | null,
  modelListed: boolean | null,
  reportedWindow: number | null,
  windowMismatch: boolean,
  firstByteMs: number | null,
  status: number | null,
  error: string | null,
};

// hollow_lib::self_hosted_jobs::SelfHostedWaiting
export type SelfHostedWaiting = {
  entry: string,
  name: string | null,
  count: number,
  oldestAt: string,
  draining: boolean,
  lastError: string | null,
};

// hollow_lib::self_hosted_wake::SelfHostedWake
export type SelfHostedWake = {
  status: number | null,
  ms: number,
  error: string | null,
  shared: boolean,
  startedMsAgo: number,
};

// hollow_lib::self_hosted_wake::SelfHostedWakeStatus
export type SelfHostedWakeStatus = {
  startedMsAgo: number,
  result: SelfHostedWake | null,
};

// hollow_lib::types::SendAccepted
export type SendAccepted = {
  sessionKey: string,
  queueId: number,
  queuePosition: number,
  phase: SessionPhase,
  waitingFor: string | null,
  heldId?: string,
  forwardedFrom?: string,
  forwardedTitle?: string,
};

// hollow_lib::workspaces::SessionChanges
export type SessionChanges = {
  key: string,
  additions: number,
  deletions: number,
};

// hollow_lib::peer_client::SessionDetail
export type SessionDetail = {
  session: RemoteSession,
  permissionMode: string | null,
  turns: Array<unknown>,
  hasMore: boolean,
  live: RemoteLive | null,
};

// hollow_lib::ts_bindings::SessionPhase
export type SessionPhase = "idle" | "queued" | "preparing" | "running" | "pausing" | "paused" | "cancelling" | "failed";

// hollow_lib::types::SessionRecordTurn
export type SessionRecordTurn = {
  ord: number,
  runId: string | null,
  startedAt: string,
  endedAt: string | null,
  status: string | null,
  asked: string | null,
};

// hollow_lib::ts_bindings::SessionResumability
export type SessionResumability = "native" | "briefOnly" | "none";

// hollow_lib::shell_path::ShellPathRead
export type ShellPathRead = {
  state: ShellPathState,
  detail: string | null,
};

// hollow_lib::shell_path::ShellPathState
export type ShellPathState = "read" | "pending" | "failed";

// hollow_lib::workspaces::SourceContext
export type SourceContext = {
  path: string,
  original: string | null,
  captured: string | null,
  freshness: string,
  coverage: string,
  includedBytes: number,
  totalBytes: number,
};

// hollow_lib::sources::SourceLabel
export type SourceLabel = {
  kind: SourceLabelKind,
  text: string,
  providerName: string | null,
  url: string | null,
  openLabel: string | null,
  since: string | null,
  wasTracked: string | null,
  wasTrackedUrl: string | null,
};

// hollow_lib::sources::LabelKind
export type SourceLabelKind = "tracked" | "stored" | "readOnly" | "unreachable";

// hollow_lib::sources::Binding
export type SourcesBinding = {
  role: SourcesRole,
  owner: SourcesOwnerKind,
  inherited: boolean,
  chosen: boolean,
  workspace: string | null,
  provider: string | null,
  providerName: string | null,
  teamId: string | null,
  teamKey: string | null,
  trackerProject: string | null,
  wasTracked: string | null,
  readOnly: string | null,
  reachable: boolean,
  unreachableSince: string | null,
  problems: Array<string>,
};

// hollow_lib::sources::migration::PlannedFile
export type SourcesMigrationFile = {
  path: string,
  action: string,
  why: string,
};

// hollow_lib::sources::migration::Outcome
export type SourcesMigrationOutcome = {
  plan: SourcesMigrationPlan,
  record: SourcesMigrationRecord | null,
  resumed: boolean,
  note: string,
};

// hollow_lib::sources::migration::Phase
export type SourcesMigrationPhase = "applying" | "applied" | "rolledBack";

// hollow_lib::sources::migration::Plan
export type SourcesMigrationPlan = {
  target: SourcesMigrationTarget,
  from: SourcesOwnerKind,
  nothingToDo: boolean,
  refused: string | null,
  files: Array<SourcesMigrationFile>,
  configChange: string | null,
  generationFrom: number,
  generationTo: number,
  storedInHollowAfter: boolean,
  queuedEdits: number,
  heldEdits: number,
  conflicts: number,
  mirroredTasks: number,
  mirroredProjects: number,
  trackerWrites: number,
  notes: Array<string>,
};

// hollow_lib::sources::migration::Record
export type SourcesMigrationRecord = {
  id: string,
  phase: SourcesMigrationPhase,
  target: SourcesMigrationTarget,
  from: SourcesOwnerKind,
  createdAt: string,
  vault: string,
  generationFrom: number,
  generationTo: number,
  storedBefore: boolean,
  storedAfter: boolean,
  ledgerStep: boolean,
  ledgerExisted: boolean,
  ledgerBeforeSha: string | null,
  ledgerAfterSha: string | null,
  configStep: boolean,
  configBeforeSha: string | null,
  configAfterSha: string | null,
  rolledBackAt: string | null,
  notes: Array<string>,
};

// hollow_lib::sources::migration::Status
export type SourcesMigrationStatus = {
  switches: Array<SourcesMigrationRecord>,
  unfinished: string | null,
};

// hollow_lib::sources::migration::Target
export type SourcesMigrationTarget = {
  workspace: string | null,
  to: SourcesOwnerKind,
};

// hollow_lib::sources::OwnerKind
export type SourcesOwnerKind = "vault" | "tracker";

// hollow_lib::sources::Reference
export type SourcesReference = {
  connector: string,
  label: string | null,
};

// hollow_lib::sources::ReferenceRow
export type SourcesReferenceRow = {
  reference: SourcesReference,
  name: string | null,
  inLibrary: boolean,
  label: SourceLabel,
};

// hollow_lib::sources::Role
export type SourcesRole = "tasks" | "knowledge" | "personal";

// hollow_lib::sources::RoleRow
export type SourcesRoleRow = {
  role: SourcesRole,
  binding: SourcesBinding,
  label: SourceLabel,
  lastRead: string | null,
  queuedEdits: number,
  heldEdits: number,
  conflicts: number,
  problems: Array<string>,
};

// hollow_lib::sources::SourcesStatus
export type SourcesStatus = {
  rows: Array<SourcesRoleRow>,
  workspaces: Array<SourcesWorkspaceRow>,
  references: Array<SourcesReferenceRow>,
  generation: number,
  storedInHollow: boolean,
  unfinishedMigration: string | null,
  trackerTaskKeys: Array<string>,
  sentTaskKeys: Array<string>,
  trackerProjectKeys: Array<string>,
  sentProjectKeys: Array<string>,
  problems: Array<string>,
};

// hollow_lib::sources::Update
export type SourcesUpdate = {
  tasksOwner: SourcesOwnerKind | null,
  references: Array<SourcesReference> | null,
};

// hollow_lib::sources::WorkspaceRow
export type SourcesWorkspaceRow = {
  id: string,
  title: string | null,
  binding: SourcesBinding,
  label: SourceLabel,
};

// hollow_lib::workspaces::StartedBy
export type StartedBy = {
  key: string,
  parent: string,
};

// hollow_lib::types::SubTask
export type SubTask = {
  text: string,
  done: boolean,
};

// hollow_lib::subagents::activity::SubagentActivity
export type SubagentActivity = {
  current: ActivityStep | null,
  recent: Array<ActivityStep>,
  progress: ReportedProgress | null,
  outcome: string | null,
  artifacts: Array<string>,
  reportedPrs: Array<string>,
};

// hollow_lib::subagents::activity::SubagentGroup
export type SubagentGroup = {
  id: string,
  title: string,
  parts: number,
};

// hollow_lib::subagents::SubagentRow
export type SubagentRow = {
  key: string,
  current: string,
  title: string | null,
  agent: string,
  model: string | null,
  effort: string | null,
  account: string | null,
  status: SubagentStatus,
  queuePlace: number | null,
  waitingFor: string | null,
  startedAt: string,
  endedAt: string | null,
  workingMs: number,
  wallMs: number,
  inputTokens: number,
  cachedInputTokens: number,
  outputTokens: number,
  tokens: number,
  costUsd: number | null,
  earlier: boolean,
  conversations: Array<string>,
  trackerIssue: string | null,
  issue: WorkIssueRef | null,
  freshFrom: string | null,
  parentKey?: string,
  group?: SubagentGroup,
  activity?: SubagentActivity,
  queueId?: number,
  startedBy?: "lead" | "main" | "chat" | "you",
};

// hollow_lib::ts_bindings::SubagentStatus
export type SubagentStatus = "running" | "queued" | "done" | "failed" | "stopped";

// hollow_lib::subagents::SubagentsView
export type SubagentsView = {
  parent: string,
  parents: Array<string>,
  active: Array<SubagentRow>,
  done: Array<SubagentRow>,
  doneOlder: Array<SubagentRow>,
  olderTotal: number,
  earlier: Array<SubagentRow>,
  running: number,
  queued: number,
  doneCount: number,
  at: string,
};

// hollow_lib::vault_learning::Suggestions
export type Suggestions = "off" | "important" | "all";

// hollow_lib::sync_engine::clash::SyncClash
export type SyncClash = {
  root: string,
  path: string,
  copy: string,
  mineMachine: string,
  theirsMachine: string,
  at: string,
  replaceable: boolean,
  editing: boolean,
  changed: boolean,
  mine: ClashSide | null,
  theirs: ClashSide | null,
  onlyMine: number,
  onlyTheirs: number,
};

// hollow_lib::types::Task
export type Task = {
  url: string,
  title: string,
  status: string,
  priority: Priority | null,
  due: string | null,
  effort: string | null,
  description: string,
  projectUrl: string | null,
  projectName: string | null,
  daysUntilDue: number | null,
  bucket: TaskBucket,
  vaultPath: string | null,
  labels: Array<string>,
  section: string | null,
  dueTime: string | null,
  durationMin: number | null,
  recurrence: string | null,
  subtasks: Array<SubTask>,
  contentHash: string,
  tracker?: TrackerTaskMeta,
  sourceLabel?: SourceLabel,
  fingerprint: string | null,
  origin: string | null,
  doneWhen: string | null,
  activity: string | null,
  blockedBy: Array<string>,
  blocks: Array<string>,
  ready: boolean,
  unresolvedBlockers: Array<string>,
  blockerWarnings: Array<BlockerWarning>,
};

// hollow_lib::ts_bindings::TaskBucket
export type TaskBucket = "overdue" | "today" | "thisweek" | "later" | "undated" | "done";

// hollow_lib::types::TasksResponse
export type TasksResponse = {
  items: Array<Task>,
  fetched: string | null,
  source: string,
  todoPath: string | null,
  lastSyncedAt: string | null,
  syncError: string | null,
};

// hollow_lib::shell_path::ToolPathFrom
export type ToolPathFrom = "hollow" | "shell" | "inherited" | "system";

// hollow_lib::tracker::TrackerComment
export type TrackerComment = {
  id: string,
  author: string | null,
  body: string,
  createdAt: string,
  editedAt: string | null,
  parentId: string | null,
};

// hollow_lib::tracker::ipc::TrackerConflict
export type TrackerConflict = {
  field: TrackerField,
  local: string,
  remote: string,
  at: string,
};

// hollow_lib::tracker::TrackerCycle
export type TrackerCycle = {
  id: string,
  number: number,
  name: string | null,
  startsAt: string,
  endsAt: string,
};

// hollow_lib::tracker::ipc::TrackerEdit
export type TrackerEdit = {
  id: string,
  values: Record<string, unknown>,
  expected: Record<string, unknown>,
};

// hollow_lib::tracker::Field
export type TrackerField = "title" | "description" | "status" | "priority" | "assignee" | "labels" | "project" | "due" | "parent" | "estimate" | "cycle" | "milestone";

// hollow_lib::tracker::ipc::TrackerFridayStatus
export type TrackerFridayStatus = {
  date: string,
  digestPath: string | null,
  heraldLine: string,
  projects: Array<TrackerStatusNote>,
  copies: Array<TrackerStatusCopy>,
};

// hollow_lib::tracker::TrackerHistory
export type TrackerHistory = {
  events: Array<TrackerHistoryEvent>,
  more: boolean,
};

// hollow_lib::tracker::TrackerHistoryEvent
export type TrackerHistoryEvent = {
  id: string,
  at: string,
  actor: string | null,
  kind: TrackerHistoryKind,
  from: string | null,
  to: string | null,
  fromKind: TrackerStateKind | null,
  toKind: TrackerStateKind | null,
  added: Array<string>,
  removed: Array<string>,
};

// hollow_lib::tracker::HistoryKind
export type TrackerHistoryKind = "state" | "assignee" | "priority" | "labels" | "project" | "title" | "description";

// hollow_lib::tracker::TrackerIssue
export type TrackerIssue = {
  id: string,
  key: string,
  url: string,
  title: string,
  description: string,
  state: TrackerState,
  assignee: TrackerUser | null,
  priority: number,
  due: string | null,
  projectId: string | null,
  labels: Array<string>,
  parentId: string | null,
  teamId: string,
  updatedAt: string,
  archivedAt: string | null,
  trashed: boolean,
  estimate: number | null,
  cycle: TrackerCycle | null,
  milestone: TrackerMilestone | null,
  creator: TrackerUser | null,
  createdAt: string,
  completedAt?: string,
  children: Array<string>,
  relations: Array<TrackerRelation>,
  links: Array<TrackerLink>,
  attachments: Array<TrackerLink>,
  comments: Array<TrackerComment>,
};

// hollow_lib::tracker::ipc::TrackerIssueDetail
export type TrackerIssueDetail = {
  issue: TrackerIssue,
  project: TrackerProjectRow | null,
  path: string | null,
  workspaceId: string | null,
  lock: TrackerLock | null,
  writableFields: Array<TrackerField>,
  conflicts: Array<TrackerConflict>,
  pending: Array<TrackerField>,
  vaultNotes: string | null,
};

// hollow_lib::tracker::work::TrackerIssueWork
export type TrackerIssueWork = {
  runs: Array<WorkRun>,
  audit: Array<AuditEntry>,
};

// hollow_lib::tracker::settings::KeyState
export type TrackerKeyState = "missing" | "saved" | "unreadable";

// hollow_lib::tracker::TrackerLabel
export type TrackerLabel = {
  id: string,
  name: string,
  color: string | null,
  group: string | null,
};

// hollow_lib::tracker::TrackerLink
export type TrackerLink = {
  id: string,
  title: string,
  url: string,
  source: string | null,
  createdAt: string | null,
};

// hollow_lib::tracker::ipc::TrackerLock
export type TrackerLock = {
  kind: TrackerLockKind,
  fields: Array<string>,
};

// hollow_lib::tracker::ipc::TrackerLockKind
export type TrackerLockKind = "linked" | "remoteMissing" | "moved" | "unreadable" | "staleRelations" | "outOfScope";

// hollow_lib::tracker::TrackerMilestone
export type TrackerMilestone = {
  id: string,
  name: string,
  targetDate: string | null,
  projectId: string | null,
};

// hollow_lib::tracker::ipc::TrackerNewProject
export type TrackerNewProject = {
  name: string,
  description: string | null,
  leadId: string | null,
  targetDate: string | null,
  workspaceId: string | null,
  teamKey: string | null,
};

// hollow_lib::tracker::ipc::TrackerNewTask
export type TrackerNewTask = {
  title: string,
  description: string | null,
  stateId: string | null,
  priority: number | null,
  assigneeId: string | null,
  labels: Array<string>,
  projectId: string | null,
  due: string | null,
  parentId: string | null,
  estimate: number | null,
  cycleId: string | null,
  milestoneId: string | null,
  workspaceId: string | null,
  teamKey: string | null,
};

// hollow_lib::tracker::ProjectHealth
export type TrackerProjectHealth = "onTrack" | "atRisk" | "offTrack";

// hollow_lib::tracker::ipc::TrackerProjectOption
export type TrackerProjectOption = {
  id: string,
  name: string,
};

// hollow_lib::tracker::ipc::TrackerProjectRow
export type TrackerProjectRow = {
  id: string,
  key: string | null,
  name: string,
  statusName: string,
  statusKind: string,
  lead: string | null,
  targetDate: string | null,
  progress: number | null,
  issueCount: number,
  openCount: number,
  url: string,
  workspaceId: string | null,
  teamKey: string | null,
};

// hollow_lib::tracker::ipc::TrackerProviderOption
export type TrackerProviderOption = {
  id: string,
  displayName: string,
  keychainItem: boolean,
  transports: Array<string>,
};

// hollow_lib::tracker::RateBudget
export type TrackerRateBudget = {
  requestsRemaining: number | null,
  requestsLimit: number | null,
  complexityRemaining: number | null,
  complexityLimit: number | null,
  resetAt: string | null,
};

// hollow_lib::tracker::TrackerRelation
export type TrackerRelation = {
  kind: TrackerRelationKind,
  key: string,
};

// hollow_lib::tracker::RelationKind
export type TrackerRelationKind = "blocks" | "blockedBy" | "related" | "duplicateOf";

// hollow_lib::tracker::ipc::TrackerRunState
export type TrackerRunState = "off" | "needsSetup" | "blocked" | "ready" | "syncing" | "error";

// hollow_lib::tracker::settings::TrackerSettingsUpdate
export type TrackerSettingsUpdate = {
  provider: string | null,
  pollSeconds: number | null,
  teamId: string | null,
  teamKey: string | null,
  personalTeamId: string | null,
  personalTeamKey: string | null,
  transport: string | null,
  trackAgentWork: boolean | null,
  fridayStatus: boolean | null,
  postStatusUpdates: boolean | null,
};

// hollow_lib::tracker::TrackerState
export type TrackerState = {
  id: string,
  name: string,
  kind: TrackerStateKind,
  position: number,
  color: string | null,
};

// hollow_lib::tracker::StateKind
export type TrackerStateKind = "backlog" | "unstarted" | "started" | "completed" | "canceled";

// hollow_lib::tracker::ipc::TrackerStatus
export type TrackerStatus = {
  state: TrackerRunState,
  provider: string | null,
  displayName: string | null,
  providers: Array<TrackerProviderOption>,
  teamId: string | null,
  teamKey: string | null,
  personalTeamId: string | null,
  personalTeamKey: string | null,
  transport: string,
  pollSeconds: number,
  trackAgentWork: boolean,
  fridayStatus: boolean,
  postStatusUpdates: boolean,
  key: TrackerKeyState,
  missing: string | null,
  blockedBy: string | null,
  lastSync: string | null,
  nextSync: string | null,
  lastError: string | null,
  rate: TrackerRateBudget | null,
  outbox: number,
  problems: Array<string>,
  restartNeeded: boolean,
  engine: boolean,
};

// hollow_lib::tracker::ipc::TrackerStatusCopy
export type TrackerStatusCopy = {
  workspaceId: string,
  projectId: string,
  posted: boolean,
  note: string,
};

// hollow_lib::tracker::ipc::TrackerStatusNote
export type TrackerStatusNote = {
  workspaceId: string,
  title: string,
  projectId: string,
  path: string,
  done: number,
  inProgress: number,
  blocked: number,
  next: number,
  health: TrackerProjectHealth,
};

// hollow_lib::tracker::ipc::TrackerSyncOutcome
export type TrackerSyncOutcome = {
  pulled: number,
  pushed: number,
  conflicts: number,
  droppedOutOfScope: number,
  errors: Array<string>,
  finishedAt: string,
};

// hollow_lib::tracker::ipc::TrackerTaskMeta
export type TrackerTaskMeta = {
  provider: string,
  id: string,
  key: string | null,
  url: string | null,
  statusType: TrackerStateKind | null,
  priority: number,
  lock: TrackerLock,
};

// hollow_lib::tracker::TrackerTeam
export type TrackerTeam = {
  id: string,
  key: string,
  name: string,
};

// hollow_lib::tracker::ipc::TrackerTeamMeta
export type TrackerTeamMeta = {
  team: TrackerTeam,
  states: Array<TrackerState>,
  labels: Array<TrackerLabel>,
  members: Array<TrackerUser>,
  cycles: Array<TrackerCycle>,
  milestones: Array<TrackerMilestone>,
  projects: Array<TrackerProjectOption>,
  viewer: TrackerUser | null,
};

// hollow_lib::tracker::TrackerUser
export type TrackerUser = {
  id: string,
  name: string,
  email: string | null,
  displayName?: string,
};

// hollow_lib::tracker::ipc::TrackerView
export type TrackerView = {
  provider: string | null,
  displayName: string | null,
  teamKey: string | null,
  teamKeys: Array<string>,
  lastSync: string | null,
  syncError: string | null,
  states: Array<TrackerState>,
  rows: Array<TrackerViewRow>,
  projects: Array<TrackerProjectRow>,
  total: number,
  note: string | null,
};

// hollow_lib::tracker::ipc::TrackerViewQuery
export type TrackerViewQuery = {
  stateKinds: Array<TrackerStateKind> | null,
  assigneeId: string | null,
  projectId: string | null,
  label: string | null,
  cycleId: string | null,
  search: string | null,
  includeClosed: boolean,
  includeArchived: boolean,
  teamKey: string | null,
  limit: number | null,
};

// hollow_lib::tracker::ipc::TrackerViewRow
export type TrackerViewRow = {
  id: string,
  key: string,
  title: string,
  statusName: string,
  statusKind: TrackerStateKind,
  priority: number,
  labels: Array<string>,
  project: string | null,
  projectId: string | null,
  due: string | null,
  assignee: string | null,
  assigneeId: string | null,
  estimate: number | null,
  cycle: string | null,
  milestone: string | null,
  teamKey: string | null,
  updatedAt: string,
  url: string,
  path: string | null,
  lock: TrackerLock | null,
  pending: boolean,
  conflict: boolean,
  archived: boolean,
  sourceLabel?: SourceLabel,
};

// hollow_lib::tracker::ipc::TrackerWriteOutcome
export type TrackerWriteOutcome = {
  id: string | null,
  key: string | null,
  path: string | null,
  queued: boolean,
  note: string,
};

// hollow_lib::workspaces::TrashedWorkspace
export type TrashedWorkspace = {
  id: string,
  title: string,
  trashedAt: string,
  blocked?: string,
};

// hollow_lib::workspace_merge::UnmergeOutcome
export type UnmergeOutcome = {
  from: string,
  into: string,
  files: number,
  sessions: number,
  sessionKeys: Array<string>,
  warnings: Array<string>,
};

// hollow_lib::update_client::state::Channel
export type UpdateChannel = "main" | "nightly";

// hollow_lib::update_client::ipc::UpdateConnected
export type UpdateConnected = {
  connected: boolean,
};

// hollow_lib::update_client::goback_run::UpdateGoBackOutcome
export type UpdateGoBackOutcome = { "outcome": "started", version: string, } | { "outcome": "workRunning", work: Array<UpdateWork>, } | { "outcome": "workChanged", work: Array<UpdateWork>, } | { "outcome": "refused", reason: string, };

// hollow_lib::update_client::goback::UpdateGoBackPlan
export type UpdateGoBackPlan = {
  available: boolean,
  disabledReason: string | null,
  runningVersion: string,
  runningInstalledAt: string | null,
  previous: UpdatePreviousBuild | null,
  schema: UpdateSchemaVerdict | null,
  snapshot: UpdateSnapshotInfo | null,
  restoreAvailable: boolean,
  indexesOff: Array<string>,
  willSkip: string,
  skippedVersion: string | null,
  waiting: boolean,
};

// hollow_lib::update_client::install::UpdateInstallOutcome
export type UpdateInstallOutcome = { "outcome": "started", version: string, } | { "outcome": "workRunning", work: Array<UpdateWork>, } | { "outcome": "workChanged", work: Array<UpdateWork>, };

// hollow_lib::update_client::last::UpdateLastResult
export type UpdateLastResult = { "kind": "updated", version: string, notes: string, channel: UpdateChannel, at: string, local: UpdateLocalBuild | null, } | { "kind": "wentBack", attempted: string, restored: string | null, reason: string, at: string, manual: boolean, dataRestored: boolean, } | { "kind": "notWentBack", target: string, reason: string | null, at: string, } | { "kind": "notInstalled", attempted: string, reason: string | null, at: string, };

// hollow_lib::update_client::local::UpdateLocalBuild
export type UpdateLocalBuild = {
  workspaceId: string,
  branch: string,
  buildId: string,
  dirty: boolean,
};

// hollow_lib::update_client::goback::UpdateLostData
export type UpdateLostData = {
  since: string,
  chats: number,
  messages: number,
  summary: string,
};

// hollow_lib::update_client::people_admin::UpdatePeopleList
export type UpdatePeopleList = {
  people: Array<UpdatePerson>,
};

// hollow_lib::update_client::people_admin::UpdatePerson
export type UpdatePerson = {
  id: string,
  name: string,
  channels: Array<UpdateChannel>,
  revoked: boolean,
  createdAt: string,
};

// hollow_lib::update_client::people_admin::UpdatePersonAdded
export type UpdatePersonAdded = {
  person: UpdatePerson,
  installLink: string,
};

// hollow_lib::update_client::state::UpdatePhase
export type UpdatePhase = { "phase": "notSetUp", reason: string, } | { "phase": "idle" } | { "phase": "checking" } | { "phase": "upToDate", skipped: string | null, } | { "phase": "available", release: UpdateRelease, } | { "phase": "downloading", release: UpdateRelease, percent: number | null, } | { "phase": "ready", release: UpdateRelease, } | { "phase": "installing", release: UpdateRelease, } | { "phase": "goingBack", version: string, } | { "phase": "failed", reason: string, };

// hollow_lib::update_client::goback::UpdatePreviousBuild
export type UpdatePreviousBuild = {
  version: string,
  installedAt: string | null,
};

// hollow_lib::update_client::publish::UpdatePublishBuild
export type UpdatePublishBuild = {
  version: string,
  commit: string | null,
  pubDate: string | null,
  ranHere: boolean,
  newerThanMain: boolean,
};

// hollow_lib::update_client::publish::UpdatePublishCommit
export type UpdatePublishCommit = {
  sha: string,
  subject: string,
};

// hollow_lib::update_client::publish::UpdatePublishHistoryEntry
export type UpdatePublishHistoryEntry = {
  version: string,
  action: string,
  at: string | null,
  from: string | null,
};

// hollow_lib::update_client::publish::UpdatePublishLive
export type UpdatePublishLive = {
  version: string,
  commit: string | null,
  pubDate: string | null,
};

// hollow_lib::update_client::publish::UpdatePublishNotes
export type UpdatePublishNotes = {
  pullRequests: Array<UpdatePublishPr>,
  whatsNew: Array<UpdatePublishWhatsNew>,
  commits: Array<UpdatePublishCommit>,
  note: string | null,
};

// hollow_lib::update_client::publish::UpdatePublishOutcome
export type UpdatePublishOutcome = { "outcome": "published", version: string, from: string | null, at: string, historyWarning: string | null, } | { "outcome": "withdrawn", version: string, from: string, at: string, historyWarning: string | null, } | { "outcome": "refused", code: string, reason: string, };

// hollow_lib::update_client::publish::UpdatePublishPr
export type UpdatePublishPr = {
  number: number,
  title: string,
  merge: string,
  mergedAt: string,
};

// hollow_lib::update_client::publish::UpdatePublishStatus
export type UpdatePublishStatus = {
  publisher: boolean,
  runner: string | null,
  available: boolean,
  unavailableReason: string | null,
  nightly: UpdatePublishLive | null,
  main: UpdatePublishLive | null,
  builds: Array<UpdatePublishBuild>,
  proposed: string | null,
  proposedWhy: string | null,
  notes: UpdatePublishNotes | null,
  changesAhead: number | null,
  daysAhead: number | null,
  canWithdraw: boolean,
  previousMain: string | null,
  history: Array<UpdatePublishHistoryEntry>,
  nudge: boolean,
};

// hollow_lib::update_client::publish::UpdatePublishWhatsNew
export type UpdatePublishWhatsNew = {
  title: string,
  what: string,
};

// hollow_lib::update_client::state::UpdateRelease
export type UpdateRelease = {
  version: string,
  notes: string,
  pubDate: string | null,
  channel: UpdateChannel,
  commit: string | null,
  downgrade: boolean,
  local: UpdateLocalBuild | null,
};

// hollow_lib::update_client::goback::UpdateSchemaVerdict
export type UpdateSchemaVerdict = { "verdict": "compatible" } | { "verdict": "needsRestore", newer: Array<UpdateStoreVersion>, lost: UpdateLostData | null, summary: string, };

// hollow_lib::update_client::settings::UpdateSettings
export type UpdateSettings = {
  channel: UpdateChannel,
  installOvernight: boolean,
  skippedVersion: string | null,
  dismissedVersion: string | null,
  lastCheckedAt: string | null,
  allowDowngradeOnce: boolean,
  allowDowngradeSinceMs: number | null,
  personName: string | null,
  allowedChannels: Array<UpdateChannel> | null,
  publisher: boolean,
  dismissedLocalBuild: string | null,
};

// hollow_lib::update_client::goback::UpdateSnapshotInfo
export type UpdateSnapshotInfo = {
  at: string,
  files: number,
  intact: boolean,
  problem: string | null,
};

// hollow_lib::update_client::state::UpdateState
export type UpdateState = {
  phase: UpdatePhase,
  runningVersion: string,
  channel: UpdateChannel,
  configured: boolean,
  connected: boolean,
  pendingConnect: boolean,
  waiting: boolean,
  goBackWaiting: boolean,
  dismissed: boolean,
  skippedVersion: string | null,
  downgradeAllowed: boolean,
  lastCheckedAt: string | null,
  note: string | null,
  allowedChannels: Array<UpdateChannel>,
  personName: string | null,
};

// hollow_lib::update_client::goback::UpdateStoreVersion
export type UpdateStoreVersion = {
  store: string,
  file: string,
  current: number,
  known: number | null,
};

// hollow_lib::update_client::install::UpdateWork
export type UpdateWork = {
  key: string,
  name: string,
};

// hollow_lib::schedule_windows::WindowDef
export type WindowDef = {
  name: string,
  label: string,
  days: Array<string>,
  start: string,
  end: string,
};

// hollow_lib::schedule_windows::WindowSettings
export type WindowSettings = {
  windows: Array<WindowDef>,
  overnightCapUsd: number,
};

// hollow_lib::schedule_windows::WindowView
export type WindowView = {
  name: string,
  label: string,
  days: Array<string>,
  start: string,
  end: string,
  period: string,
  openNow: boolean,
  nextStart?: string,
  nextStarts?: Array<string>,
};

// hollow_lib::workspaces::WorkFile
export type WorkFile = {
  path: string,
  name: string,
  kind: "context" | "source" | "output" | "session",
  size: number,
  modified: string | null,
};

// hollow_lib::tracker::work::WorkIssueRef
export type WorkIssueRef = {
  id: string,
  key: string,
  title: string | null,
  stateName: string | null,
  stateKind: TrackerStateKind | null,
};

// hollow_lib::workspace_results::WorkResult
export type WorkResult = {
  path: string,
  name: string,
  title: string,
  kind: string,
  size: number,
  modified: string | null,
  sha256: string | null,
  review: ResultReview,
  acceptedSha256: string | null,
  acceptedAt: string | null,
  readyAt: string | null,
  hasSnapshot: boolean,
  acceptance: ResultAcceptance | null,
  history: Array<ReviewEvent>,
  origin: ResultOrigin | null,
  checks: ResultChecks,
  canvas?: ResultCanvas,
};

// hollow_lib::tracker::work::WorkRun
export type WorkRun = {
  sessionKey: string,
  title: string | null,
  agent: string,
  model: string | null,
  effort: string | null,
  status: SubagentStatus,
  startedAt: string,
  endedAt: string | null,
  workingMs: number,
  tokens: number,
  costUsd: number | null,
  linkedBy: string,
  accepted: boolean,
};

// hollow_lib::workspaces::Workspace
export type WorkWorkspace = {
  id: string,
  title: string,
  status: string,
  path: string,
  cwd: string,
  modified: string,
  build?: BuildInfo,
  kind?: "quick" | "main" | "tracker",
  mergedInto?: string,
  archivedFrom?: string,
  canvasTheme?: string,
  pinned?: number,
};

// hollow_lib::types::Workflow
export type Workflow = {
  slug: string,
  title: string,
  description: string,
  schedule: string,
  agent: string | null,
  model: string | null,
  cwd: string | null,
  outputGlob: string | null,
  routine: string | null,
  permissionMode: string,
  allowedTools: Array<string>,
  disallowedTools: Array<string>,
  triggers: Array<string>,
  inChat?: string,
  runsOn?: string,
  created: string | null,
  lastRunAt: string | null,
  lastStatus: string | null,
  lastError: string | null,
  lastDurationSec: number | null,
  nextRunAt: string | null,
  misfireGraceMin: number | null,
  catchUp?: string,
  scheduleProblem?: string,
  skipIfQuiet: boolean,
  watch: Array<string>,
  precheck?: string,
  notify: string | null,
  lastStartedAt: string | null,
  ends?: string,
  afterRuns?: number,
  runsCounted?: number,
  endedNote?: string,
  path: string,
  permissionHold?: "review" | "unavailable",
  budgetHold?: string,
};

// hollow_lib::workspace_activity::WorkspaceActivity
export type WorkspaceActivity = {
  workspaceId: string,
  asOf: string,
  events: Array<ActivityEvent>,
  total: number,
  gaps: ActivityGaps,
  nextCursor: string | null,
};

// hollow_lib::workspace_connections::WorkspaceConnections
export type WorkspaceConnections = {
  edges: Array<WorkspaceEdge>,
  generatedAt: string,
  tookMs: number,
  cached: boolean,
};

// hollow_lib::workspaces::WorkspaceDetail
export type WorkspaceDetail = {
  projectAgentKey?: string,
  workspace: WorkWorkspace,
  files: Array<WorkFile>,
  sessions: Array<AgentSessionSummary>,
  sources: Array<SourceContext>,
  sourcesOmitted: number,
  reentry: WorkspaceReentry,
  contextHealth: Array<ContextDocHealth>,
};

// hollow_lib::workspace_results::WorkspaceDigest
export type WorkspaceDigest = {
  workspaceId: string,
  reentry: WorkspaceReentry,
  latestResult: LatestResult | null,
  resultCount: number,
  awaitingReview: number,
  lastActivityAt: string | null,
  resultsError?: string,
};

// hollow_lib::workspace_connections::WorkspaceEdge
export type WorkspaceEdge = {
  a: string,
  b: string,
  weight: number,
  reasons: Array<ConnectionReason>,
};

// hollow_lib::workspace_reentry::WorkspaceReentry
export type WorkspaceReentry = {
  asOf: string,
  nextStep: string | null,
  openItems: Array<string>,
  openItemCount: number | null,
  lastRun: ReentryLastRun | null,
  gaps: Array<string>,
  briefStale?: BriefStale,
};

// hollow_lib::workspaces::WorkspaceSessions
export type WorkspaceSessions = {
  projectAgentKey?: string,
  workspaceId: string,
  total: number,
  sessions: Array<AgentSessionSummary>,
  changes: Array<SessionChanges>,
  startedBy?: Array<StartedBy>,
};
