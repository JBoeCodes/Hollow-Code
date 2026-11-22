import { describe, expect, it } from 'vitest';
import { restorePreview } from './historyRun';
import type {
  ActivityEvent, ActivityGaps, ActivityInstall, ActivityResult, ActivityRun, ActivitySession, WorkspaceActivity,
  AgentSettings, BlockerWarning, BuildInfo, Canvas, CanvasRecorded, CanvasRevision, LibraryResult, NeedsYouItem, NeedsYouPage,
  ResultAcceptance, ResultCanvas, ReviewEvent, Task, TasksResponse, WorkResult, WorkWorkspace, Workflow,
  AgentSessionSummary, BriefStale, ContextDocHealth, HarnessStint, LatestResult, ReentryLastRun, RunQueueItem, SourceContext, WorkFile,
  WorkspaceDetail, WorkspaceDigest, WorkspaceReentry,
  AgentSessionDetail, SessionChanges, SessionRecordTurn, StartedBy, WorkspaceSessions,
  HistoryChange, HistoryRun, RunFlag,
} from './generated/ipc';

/**
 * One serialized IPC response per page (merged plan slice 6 item 7). The
 * fixtures are written by serde from Rust values (src-tauri/src/ts_bindings.rs,
 * which fails `cargo test --lib` when one is stale), so each test here checks
 * that what Rust sends has exactly the keys the generated type declares, and
 * reads a few fields through that type.
 */
const FIXTURES = import.meta.glob('../tests/fixtures/ipc/*.json', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

function fixture<T>(name: string): T {
  const text = FIXTURES[`../tests/fixtures/ipc/${name}.json`];
  if (text === undefined) throw new Error(`missing tests/fixtures/ipc/${name}.json`);
  return JSON.parse(text) as T;
}

/** `true` only when `K` lists every key of `T`: a key the list misses is a type error. */
type Complete<T, K extends readonly (keyof T)[]> = [Exclude<keyof T, K[number]>] extends [never] ? true : never;

/** Every key serde wrote across `values`, sorted. */
const keysOf = (...values: object[]) => [...new Set(values.flatMap(v => Object.keys(v)))].sort();
const sorted = (keys: readonly PropertyKey[]) => keys.map(String).sort();

const WORKSPACE_KEYS = ['id', 'title', 'status', 'path', 'cwd', 'modified', 'build', 'kind', 'mergedInto', 'archivedFrom', 'canvasTheme', 'pinned'] as const satisfies readonly (keyof WorkWorkspace)[];
const BUILD_KEYS = ['repo', 'worktree', 'branch', 'base', 'exists', 'runsOn'] as const satisfies readonly (keyof BuildInfo)[];
const TASKS_RESPONSE_KEYS = ['items', 'fetched', 'source', 'todoPath', 'lastSyncedAt', 'syncError'] as const satisfies readonly (keyof TasksResponse)[];
const TASK_KEYS = [
  'url', 'title', 'status', 'priority', 'due', 'effort', 'description', 'projectUrl', 'projectName', 'daysUntilDue',
  'bucket', 'vaultPath', 'labels', 'section', 'dueTime', 'durationMin', 'recurrence', 'subtasks', 'contentHash',
  'fingerprint', 'origin', 'doneWhen', 'activity', 'blockedBy', 'blocks', 'ready', 'unresolvedBlockers',
  'blockerWarnings', 'tracker', 'sourceLabel',
] as const satisfies readonly (keyof Task)[];
const BLOCKER_WARNING_KEYS = ['url', 'kind'] as const satisfies readonly (keyof BlockerWarning)[];
const RESULT_KEYS = [
  'path', 'name', 'title', 'kind', 'size', 'modified', 'sha256', 'review', 'acceptedSha256', 'acceptedAt', 'readyAt',
  'hasSnapshot', 'acceptance', 'history', 'origin', 'checks', 'canvas',
] as const satisfies readonly (keyof WorkResult)[];
/** `canvas` is omitted, not null, on every kind but `canvas` (Canvas C3 · S3). */
const PLAIN_RESULT_KEYS = RESULT_KEYS.filter(k => k !== 'canvas');
const RESULT_CANVAS_KEYS = ['id', 'revision'] as const satisfies readonly (keyof ResultCanvas)[];
const LIBRARY_KEYS = [...RESULT_KEYS, 'workspaceId', 'workspaceTitle'] as const satisfies readonly (keyof LibraryResult)[];
const ACCEPTANCE_KEYS = ['actor', 'sha256', 'bytes', 'runId', 'mode', 'basis', 'snapshot', 'at'] as const satisfies readonly (keyof ResultAcceptance)[];
const REVIEW_EVENT_KEYS = ['state', 'sha256', 'at', 'note', 'actor', 'runId'] as const satisfies readonly (keyof ReviewEvent)[];
const WORKFLOW_KEYS = [
  'slug', 'title', 'description', 'schedule', 'agent', 'model', 'cwd', 'outputGlob', 'routine', 'permissionMode',
  'allowedTools', 'disallowedTools', 'triggers', 'created', 'lastRunAt', 'lastStatus', 'lastError', 'lastDurationSec',
  'nextRunAt', 'misfireGraceMin', 'catchUp', 'skipIfQuiet', 'watch', 'notify', 'lastStartedAt', 'path', 'permissionHold',
  'budgetHold', 'scheduleProblem', 'inChat', 'ends', 'afterRuns', 'runsCounted', 'endedNote', 'runsOn', 'precheck',
] as const satisfies readonly (keyof Workflow)[];
const SETTINGS_KEYS = ['defaultAgent', 'models', 'efforts', 'codexNoOutputTimeoutMin', 'noOutputTimeoutMin', 'deadlineMin', 'canvases', 'compactWindow', 'whenOut', 'runsOn', 'unattendedRunsOn', 'buildsOn', 'appBuildsOn', 'runnerDefaultsFrozen', 'moveOnSleep', 'cooDispatcher', 'acp'] as const satisfies readonly (keyof AgentSettings)[];
const NEEDS_YOU_PAGE_KEYS = ['items', 'total', 'nextCursor', 'unavailable'] as const satisfies readonly (keyof NeedsYouPage)[];
const CANVAS_RECORDED_KEYS = ['outcome', 'matchedRevision', 'canvas'] as const satisfies readonly (keyof CanvasRecorded)[];
const CANVAS_KEYS = [
  'scope', 'workspaceId', 'sessionKey', 'id', 'title', 'kit', 'session', 'fenceSha256', 'createdAt', 'updatedAt',
  'revision', 'ask', 'height', 'html', 'sha256', 'path', 'file', 'htmlChanged', 'state', 'stateSavedAt', 'revisions',
] as const satisfies readonly (keyof Canvas)[];
const CANVAS_REVISION_KEYS = [
  'revision', 'fenceSha256', 'title', 'savedAt', 'ask', 'height', 'session', 'turnId',
] as const satisfies readonly (keyof CanvasRevision)[];
const NEEDS_YOU_ITEM_KEYS = [
  'id', 'kind', 'sourceId', 'title', 'detail', 'state', 'at', 'workspaceId', 'sessionKey', 'path', 'category', 'link',
] as const satisfies readonly (keyof NeedsYouItem)[];
const ACTIVITY_KEYS = ['workspaceId', 'asOf', 'events', 'total', 'gaps', 'nextCursor'] as const satisfies readonly (keyof WorkspaceActivity)[];
const ACTIVITY_EVENT_KEYS = [
  'id', 'kind', 'at', 'status', 'title', 'sessionKey', 'runId', 'receiptId', 'path', 'session', 'run', 'result', 'install',
] as const satisfies readonly (keyof ActivityEvent)[];
const ACTIVITY_GAPS_KEYS = ['runsWithoutReceipt', 'runsWithoutUsage', 'resultsWithoutReceipt', 'turnsTruncated', 'resultsTruncated', 'receiptsError', 'reviewsError', 'usageError'] as const satisfies readonly (keyof ActivityGaps)[];
const ACTIVITY_RUN_KEYS = [
  'source', 'sourceId', 'turn', 'startedAt', 'endedAt', 'agent', 'model', 'costUsd', 'costStatus', 'inputTokens', 'outputTokens',
  'files', 'outputs', 'checks', 'failedChecks', 'verification', 'resultKind', 'canvases',
] as const satisfies readonly (keyof ActivityRun)[];
const ACTIVITY_RESULT_KEYS = ['name', 'kind', 'exists', 'actor', 'revision', 'note'] as const satisfies readonly (keyof ActivityResult)[];
const ACTIVITY_SESSION_KEYS = ['agent', 'turns', 'lastActivityAt'] as const satisfies readonly (keyof ActivitySession)[];
const ACTIVITY_INSTALL_KEYS = ['step', 'commit', 'branch', 'summary', 'matchesRunning'] as const satisfies readonly (keyof ActivityInstall)[];
const WORKSPACE_DETAIL_KEYS = ['projectAgentKey', 'workspace', 'files', 'sessions', 'sources', 'sourcesOmitted', 'reentry', 'contextHealth'] as const satisfies readonly (keyof WorkspaceDetail)[];
const WORK_FILE_KEYS = ['path', 'name', 'kind', 'size', 'modified'] as const satisfies readonly (keyof WorkFile)[];
const SESSION_KEYS = [
  'key', 'title', 'agent', 'harnessSessionId', 'cwd', 'createdAt', 'lastActivityAt', 'endedAt', 'turnCount', 'parentKey', 'forkKind',
  'adopted', 'stints', 'resumable', 'imported',
] as const satisfies readonly (keyof AgentSessionSummary)[];
/** `imported` (H5a) is optional on the wire: absent for a chat started in Hollow, as in every fixture. */
const SESSION_WIRE_KEYS = SESSION_KEYS.filter(k => k !== 'imported');
const STINT_KEYS = ['agent', 'harnessSessionId', 'startedAt', 'endedAt', 'reason'] as const satisfies readonly (keyof HarnessStint)[];
const SOURCE_CONTEXT_KEYS = ['path', 'original', 'captured', 'freshness', 'coverage', 'includedBytes', 'totalBytes'] as const satisfies readonly (keyof SourceContext)[];
const REENTRY_KEYS = ['asOf', 'nextStep', 'openItems', 'openItemCount', 'lastRun', 'gaps', 'briefStale'] as const satisfies readonly (keyof WorkspaceReentry)[];
const LAST_RUN_KEYS = [
  'sessionKey', 'status', 'finishedAt', 'failedChecks', 'checkCount', 'unknownChecks', 'verification',
] as const satisfies readonly (keyof ReentryLastRun)[];
const BRIEF_STALE_KEYS = ['days', 'conversations'] as const satisfies readonly (keyof BriefStale)[];
const CONTEXT_HEALTH_KEYS = ['name', 'bytes', 'included', 'limit'] as const satisfies readonly (keyof ContextDocHealth)[];
const DIGEST_KEYS = [
  'workspaceId', 'reentry', 'latestResult', 'resultCount', 'awaitingReview', 'lastActivityAt', 'resultsError',
] as const satisfies readonly (keyof WorkspaceDigest)[];
const LATEST_RESULT_KEYS = ['path', 'title', 'kind', 'modified', 'review'] as const satisfies readonly (keyof LatestResult)[];
const RUN_QUEUE_KEYS = [
  'id', 'source', 'sourceId', 'label', 'detail', 'reason', 'status', 'priority', 'position', 'attempt', 'coalescedCount',
  'requestedAt', 'claimedAt', 'finishedAt', 'historyRunId', 'error', 'waitingSince', 'waitingHolder', 'heldReason',
  'cancelRequested', 'waitingReason', 'waitingSession', 'working',
] as const satisfies readonly (keyof RunQueueItem)[];
const WORKSPACE_SESSIONS_KEYS = ['projectAgentKey', 'workspaceId', 'total', 'sessions', 'changes', 'startedBy'] as const satisfies readonly (keyof WorkspaceSessions)[];
const SESSION_CHANGES_KEYS = ['key', 'additions', 'deletions'] as const satisfies readonly (keyof SessionChanges)[];
const STARTED_BY_KEYS = ['key', 'parent'] as const satisfies readonly (keyof StartedBy)[];
const SESSION_DETAIL_KEYS = ['summary', 'turns', 'eventsRetained'] as const satisfies readonly (keyof AgentSessionDetail)[];
const SESSION_TURN_KEYS = ['ord', 'runId', 'startedAt', 'endedAt', 'status', 'asked'] as const satisfies readonly (keyof SessionRecordTurn)[];
const HISTORY_RUN_KEYS = [
  'id', 'source', 'sourceId', 'startedAt', 'finishedAt', 'status', 'changes', 'flags', 'scope', 'overlaps',
] as const satisfies readonly (keyof HistoryRun)[];
const HISTORY_CHANGE_KEYS = [
  'vaultPath', 'changeType', 'beforeHash', 'afterHash', 'beforeSize', 'afterSize', 'text', 'notionMirror', 'beforeNotionId',
  'afterNotionId', 'beforeUncaptured', 'afterUncaptured', 'beforeCoverage', 'afterCoverage', 'restoredAt', 'restoredByRunId',
  'alsoChangedBy', 'unattributed', 'unverified',
] as const satisfies readonly (keyof HistoryChange)[];
const RUN_FLAG_KEYS = ['kind', 'path', 'changeType', 'detail', 'scan'] as const satisfies readonly (keyof RunFlag)[];

// Compile-time: each list names every key of its type. A missing key makes
// its entry `never`, and assigning `true` to it fails tsc.
const complete: [
  Complete<WorkWorkspace, typeof WORKSPACE_KEYS>,
  Complete<BuildInfo, typeof BUILD_KEYS>,
  Complete<TasksResponse, typeof TASKS_RESPONSE_KEYS>,
  Complete<Task, typeof TASK_KEYS>,
  Complete<BlockerWarning, typeof BLOCKER_WARNING_KEYS>,
  Complete<WorkResult, typeof RESULT_KEYS>,
  Complete<ResultCanvas, typeof RESULT_CANVAS_KEYS>,
  Complete<LibraryResult, typeof LIBRARY_KEYS>,
  Complete<ResultAcceptance, typeof ACCEPTANCE_KEYS>,
  Complete<ReviewEvent, typeof REVIEW_EVENT_KEYS>,
  Complete<Workflow, typeof WORKFLOW_KEYS>,
  Complete<AgentSettings, typeof SETTINGS_KEYS>,
  Complete<NeedsYouPage, typeof NEEDS_YOU_PAGE_KEYS>,
  Complete<NeedsYouItem, typeof NEEDS_YOU_ITEM_KEYS>,
  Complete<CanvasRecorded, typeof CANVAS_RECORDED_KEYS>,
  Complete<Canvas, typeof CANVAS_KEYS>,
  Complete<CanvasRevision, typeof CANVAS_REVISION_KEYS>,
  Complete<WorkspaceActivity, typeof ACTIVITY_KEYS>,
  Complete<ActivityEvent, typeof ACTIVITY_EVENT_KEYS>,
  Complete<ActivityGaps, typeof ACTIVITY_GAPS_KEYS>,
  Complete<ActivityRun, typeof ACTIVITY_RUN_KEYS>,
  Complete<ActivityResult, typeof ACTIVITY_RESULT_KEYS>,
  Complete<ActivitySession, typeof ACTIVITY_SESSION_KEYS>,
  Complete<ActivityInstall, typeof ACTIVITY_INSTALL_KEYS>,
  Complete<WorkspaceDetail, typeof WORKSPACE_DETAIL_KEYS>,
  Complete<WorkFile, typeof WORK_FILE_KEYS>,
  Complete<AgentSessionSummary, typeof SESSION_KEYS>,
  Complete<HarnessStint, typeof STINT_KEYS>,
  Complete<SourceContext, typeof SOURCE_CONTEXT_KEYS>,
  Complete<WorkspaceReentry, typeof REENTRY_KEYS>,
  Complete<ReentryLastRun, typeof LAST_RUN_KEYS>,
  Complete<BriefStale, typeof BRIEF_STALE_KEYS>,
  Complete<ContextDocHealth, typeof CONTEXT_HEALTH_KEYS>,
  Complete<WorkspaceDigest, typeof DIGEST_KEYS>,
  Complete<LatestResult, typeof LATEST_RESULT_KEYS>,
  Complete<RunQueueItem, typeof RUN_QUEUE_KEYS>,
  Complete<WorkspaceSessions, typeof WORKSPACE_SESSIONS_KEYS>,
  Complete<SessionChanges, typeof SESSION_CHANGES_KEYS>,
  Complete<StartedBy, typeof STARTED_BY_KEYS>,
  Complete<AgentSessionDetail, typeof SESSION_DETAIL_KEYS>,
  Complete<SessionRecordTurn, typeof SESSION_TURN_KEYS>,
  Complete<HistoryRun, typeof HISTORY_RUN_KEYS>,
  Complete<HistoryChange, typeof HISTORY_CHANGE_KEYS>,
  Complete<RunFlag, typeof RUN_FLAG_KEYS>,
] = [
  true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
  true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true,
];

describe('IPC fixtures match the generated types', () => {
  it('lists every key of each checked type', () => {
    expect(complete.every(Boolean)).toBe(true);
  });

  it('Work: list_workspaces', () => {
    const list = fixture<WorkWorkspace[]>('work');
    expect(keysOf(...list)).toEqual(sorted(WORKSPACE_KEYS));
    const [build, quick, merged] = list;
    expect(keysOf(build.build!)).toEqual(sorted(BUILD_KEYS));
    expect(build.build?.exists).toBe(true);
    // Optional on the wire: absent, never null.
    expect('kind' in build).toBe(false);
    expect('build' in quick).toBe(false);
    expect(quick.kind).toBe('quick');
    expect('mergedInto' in build).toBe(false);
    expect(merged.mergedInto).toBe('local-results-view');
  });

  it('Tasks: tasks', () => {
    const response = fixture<TasksResponse>('tasks');
    expect(keysOf(response)).toEqual(sorted(TASKS_RESPONSE_KEYS));
    const [task] = response.items;
    expect(keysOf(task)).toEqual(sorted(TASK_KEYS.filter(key => key !== 'tracker' && key !== 'sourceLabel')));
    expect('tracker' in task).toBe(false);
    expect(keysOf(...task.blockerWarnings)).toEqual(sorted(BLOCKER_WARNING_KEYS));
    expect(task.priority).toBe('P1');
    expect(task.bucket).toBe('thisweek');
    expect(task.daysUntilDue).toBe(2);
    expect(task.subtasks[0].done).toBe(true);
    expect(task.blockerWarnings[0].kind).toBe('missing');
  });

  it('Results: workspace_results', () => {
    const [result, canvas] = fixture<WorkResult[]>('results');
    expect(keysOf(result)).toEqual(sorted(PLAIN_RESULT_KEYS));
    expect(keysOf(canvas)).toEqual(sorted(RESULT_KEYS));
    expect(keysOf(canvas.canvas!)).toEqual(sorted(RESULT_CANVAS_KEYS));
    expect([canvas.kind, canvas.review, canvas.canvas?.revision]).toEqual(['canvas', 'changed', 3]);
    expect(keysOf(result.acceptance!)).toEqual(sorted(ACCEPTANCE_KEYS));
    expect(keysOf(...result.history)).toEqual(sorted(REVIEW_EVENT_KEYS));
    expect(result.review).toBe('accepted');
    expect(result.acceptance?.actor).toBe('person');
    expect(result.history.map(e => e.actor)).toEqual(['person', 'agent']);
    expect(result.checks.total).toBe(3);
    expect(result.origin?.runId).toBe('run-1');
  });

  it('Library: list_all_results (a result with its workspace, flattened)', () => {
    const [item] = fixture<LibraryResult[]>('library');
    expect(keysOf(item)).toEqual(sorted(LIBRARY_KEYS.filter(k => k !== 'canvas')));
    expect(item.workspaceTitle).toBe('Results view');
    expect(item.path).toBe('workspaces/local-results-view/outputs/brief.md');
  });

  it('Automations: list_workflows', () => {
    const [workflow] = fixture<Workflow[]>('automations');
    // scheduleProblem is sent only when the schedule can't be read (B04).
    expect(keysOf(workflow)).toEqual(sorted(WORKFLOW_KEYS.filter(k => !['scheduleProblem', 'inChat', 'ends', 'afterRuns', 'runsCounted', 'endedNote', 'runsOn', 'precheck'].includes(k))));
    expect(workflow.scheduleProblem).toBeUndefined();
    expect(workflow.permissionHold).toBe('review');
    expect(workflow.lastDurationSec).toBe(42);
    expect(workflow.skipIfQuiet).toBe(false);
  });

  it('Settings: get_agent_settings', () => {
    const settings = fixture<AgentSettings>('settings');
    expect(keysOf(settings)).toEqual(sorted(SETTINGS_KEYS));
    // An explicit null switches the Codex watchdog off; the wire keeps it.
    expect(settings.codexNoOutputTimeoutMin).toBeNull();
    expect(settings.noOutputTimeoutMin?.grok).toBeNull();
    expect(settings.deadlineMin?.codex).toBe(120);
    expect(settings.canvases).toBe('most');
    // OB22: the compaction window, absent when off (the default).
    expect(settings.compactWindow).toBe(400000);
  });

  it('Waiting on you: list_needs_you', () => {
    const page = fixture<NeedsYouPage>('needs-you');
    expect(keysOf(page)).toEqual(sorted(NEEDS_YOU_PAGE_KEYS));
    expect(keysOf(...page.items)).toEqual(sorted(NEEDS_YOU_ITEM_KEYS));
    expect(page.items.map(i => i.kind)).toEqual(['result', 'failure']);
    expect(page.nextCursor).toBe('opaque-cursor');
    expect(page.total).toBe(3);
  });

  it('Canvases: record_canvas (and read_canvas, which returns the inner Canvas)', () => {
    const recorded = fixture<CanvasRecorded>('canvas');
    expect(keysOf(recorded)).toEqual(sorted(CANVAS_RECORDED_KEYS));
    expect(keysOf(recorded.canvas)).toEqual(sorted(CANVAS_KEYS));
    expect(keysOf(...recorded.canvas.revisions)).toEqual(sorted(CANVAS_REVISION_KEYS));
    expect(recorded.outcome).toBe('revised');
    expect(recorded.canvas.scope).toBe('workspace');
    expect(recorded.canvas.revisions.at(-1)?.fenceSha256).toBe(recorded.canvas.fenceSha256);
    expect(recorded.canvas.state).toEqual({ tab: 'checks' });
  });

  it('Activity: workspace_activity', () => {
    const a = fixture<WorkspaceActivity>('activity');
    expect(keysOf(a)).toEqual(sorted(ACTIVITY_KEYS));
    expect(keysOf(...a.events)).toEqual(sorted(ACTIVITY_EVENT_KEYS));
    expect(keysOf(a.gaps)).toEqual(sorted(ACTIVITY_GAPS_KEYS));
    const [run, result, session, install] = a.events;
    expect(keysOf(run.run!)).toEqual(sorted(ACTIVITY_RUN_KEYS));
    expect(keysOf(result.result!)).toEqual(sorted(ACTIVITY_RESULT_KEYS));
    expect(keysOf(session.session!)).toEqual(sorted(ACTIVITY_SESSION_KEYS));
    expect(keysOf(install.install!)).toEqual(sorted(ACTIVITY_INSTALL_KEYS));
    expect(a.events.map(e => e.kind)).toEqual(['run', 'result', 'session', 'install']);
    expect(run.run?.canvases).toEqual({ count: 1, bytes: 6200 });
    expect(result.result?.actor).toBe('run');
    expect(install.install?.step).toBe('install');
    expect(a.total).toBe(9);
  });

  it('Workspace panel: read_workspace (and attach_workspace_source, apply_source_refresh)', () => {
    const d = fixture<WorkspaceDetail>('workspace');
    expect(keysOf(d)).toEqual(sorted(WORKSPACE_DETAIL_KEYS));
    expect(keysOf(...d.files)).toEqual(sorted(WORK_FILE_KEYS));
    expect(keysOf(...d.sessions)).toEqual(sorted(SESSION_WIRE_KEYS));
    expect(keysOf(...d.sessions[0].stints)).toEqual(sorted(STINT_KEYS));
    expect(keysOf(...d.sources)).toEqual(sorted(SOURCE_CONTEXT_KEYS));
    expect(keysOf(d.reentry)).toEqual(sorted(REENTRY_KEYS));
    expect(keysOf(d.reentry.lastRun!)).toEqual(sorted(LAST_RUN_KEYS));
    expect(keysOf(d.reentry.briefStale!)).toEqual(sorted(BRIEF_STALE_KEYS));
    expect(keysOf(...d.contextHealth)).toEqual(sorted(CONTEXT_HEALTH_KEYS));
    // Sent every time, null when unknown (the hand-written type had these optional).
    expect(d.sourcesOmitted).toBe(2);
    expect(d.files[1].modified).toBeNull();
    expect(d.reentry.lastRun?.unknownChecks).toBeNull();
    expect(d.files.map(f => f.kind)).toEqual(['context', 'output']);
    expect(d.sessions[0].resumable).toBe('native');
    expect(d.sessions[0].stints.map(s => s.agent)).toEqual(['claude', 'codex']);
    expect(d.contextHealth[1].included).toBeLessThan(d.contextHealth[1].bytes);
  });

  it('Work: workspace_digests', () => {
    const [counted, damaged] = fixture<WorkspaceDigest[]>('digests');
    expect(keysOf(damaged)).toEqual(sorted(DIGEST_KEYS));
    // Optional on the wire: `resultsError` and `briefStale` are absent, never null.
    expect(keysOf(counted)).toEqual(sorted(DIGEST_KEYS.filter(k => k !== 'resultsError')));
    expect(keysOf(counted.reentry)).toEqual(sorted(REENTRY_KEYS.filter(k => k !== 'briefStale')));
    expect(keysOf(counted.latestResult!)).toEqual(sorted(LATEST_RESULT_KEYS));
    expect(counted.latestResult?.review).toBe('ready');
    expect(damaged.latestResult).toBeNull();
    expect(damaged.resultsError).toBe('Results review state could not be read');
  });

  it('Run queue: list_run_queue (and cancel_queued_run, run-queue-event)', () => {
    const [chat, cron] = fixture<RunQueueItem[]>('run-queue');
    // `waitingReason` is null, never absent, on a row it doesn't apply to.
    expect(keysOf(chat)).toEqual(sorted(RUN_QUEUE_KEYS.filter(k => k !== 'working')));
    // `waitingSession` (P1-46) is left out when nothing names a holder.
    // `working` (2026-11-16) is left out unless the row reads Working.
    expect(keysOf(cron)).toEqual(sorted(RUN_QUEUE_KEYS.filter(k => k !== 'waitingSession' && k !== 'working')));
    expect(chat.waitingSession).toBe('s-20261005-0a1b2c3d');
    expect(cron.waitingReason).toBeNull();
    expect(chat.waitingReason).toBe('“Fix the panel” to finish (it has used this worktree for 12 min)');
    expect([chat.source, chat.status, cron.source, cron.status]).toEqual(['chat', 'queued', 'cron', 'done']);
    expect(chat.position).toBe(1);
  });

  it('Rail tree: workspace_sessions', () => {
    const [busy, quiet] = fixture<WorkspaceSessions[]>('workspace-sessions');
    expect(keysOf(busy)).toEqual(sorted(WORKSPACE_SESSIONS_KEYS));
    expect(keysOf(...busy.sessions)).toEqual(sorted(SESSION_WIRE_KEYS));
    expect(keysOf(...busy.changes)).toEqual(sorted(SESSION_CHANGES_KEYS));
    expect(keysOf(...busy.startedBy!)).toEqual(sorted(STARTED_BY_KEYS));
    // `changes` is sent every time (the hand-written type had it optional);
    // `startedBy` is absent, never empty, when nothing started another.
    expect(keysOf(quiet)).toEqual(sorted(WORKSPACE_SESSIONS_KEYS.filter(k => k !== 'startedBy')));
    expect(quiet.changes).toEqual([]);
    expect(busy.startedBy).toEqual([{ key: busy.sessions[1].key, parent: busy.sessions[0].key }]);
    expect(busy.changes[0]).toEqual({ key: busy.sessions[0].key, additions: 120, deletions: 8 });
    expect(busy.total).toBe(3);
  });

  it('One conversation: read_agent_session', () => {
    const d = fixture<AgentSessionDetail>('session-detail');
    expect(keysOf(d)).toEqual(sorted(SESSION_DETAIL_KEYS));
    expect(keysOf(d.summary)).toEqual(sorted(SESSION_WIRE_KEYS));
    expect(keysOf(...d.turns)).toEqual(sorted(SESSION_TURN_KEYS));
    expect(d.turns.map(t => t.ord)).toEqual([1, 2]);
    // A running turn: no end and no status yet, sent as null.
    expect([d.turns[1].endedAt, d.turns[1].status, d.turns[1].runId]).toEqual([null, null, null]);
    expect(d.eventsRetained).toBe(true);
  });

  it('History & recovery: get_history_run (and list_history_runs, the restores)', () => {
    const run = fixture<HistoryRun>('history');
    expect(keysOf(run)).toEqual(sorted(HISTORY_RUN_KEYS));
    expect(keysOf(...run.changes)).toEqual(sorted(HISTORY_CHANGE_KEYS));
    // `scan` is optional on the wire: absent when a flag has no scan result (CX7).
    expect(keysOf(...run.flags!)).toEqual(sorted(RUN_FLAG_KEYS.filter(k => k !== 'scan')));
    const [edited, large, mirror] = run.changes;
    // Optional on the wire: absent, never null or empty.
    for (const key of ['beforeNotionId', 'beforeUncaptured', 'restoredAt', 'alsoChangedBy', 'unattributed'] as const) expect(key in large).toBe(false);
    // Sent every time (the hand-written type had it optional).
    expect(large.notionMirror).toBe(false);
    expect([large.beforeCoverage, large.afterCoverage]).toEqual([{ state: 'absent' }, { state: 'uncaptured', reason: 'tooLarge' }]);
    expect([mirror.beforeCoverage?.state, edited.afterCoverage?.state]).toEqual(['unknown', 'captured']);
    expect(run.flags?.map(f => f.kind)).toEqual(['agentInstructions']);
    expect(run.scope).toBe('workspaces/local-results-view');
    expect(edited.alsoChangedBy).toEqual(run.overlaps);
    // Another chat's change in the shared folder (chat concurrency F1).
    expect(edited.unattributed).toBe(true);
    // A run closed by startup recovery: its changes are unknown (Q-R11).
    expect(edited.unverified).toBe(true);
    expect('unverified' in large).toBe(false);
    // The generated coverage union reads through historyRun.ts as before.
    expect(restorePreview(run.changes).map(item => item.path)).toEqual([mirror.vaultPath, large.vaultPath]);
  });
});
