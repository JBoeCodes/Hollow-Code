import { mockConnectedFiles, mockContentIndexStatus, mockDirectoryState, mockDirectoryStatus, mockReadConnectedFile } from './mockDirectories';
import { workspaceFixture } from './mockWorkspaces';
import { mockConversationOrganization } from './mockConversationOrganization';
import { sha256Hex } from './sectionMarkdown';
import { ENFORCEMENT_COPY } from './budgets';
import { CHAT_LIMIT_MAX } from './chatLimits';
import { isoWeek } from './weeklyReview';
import { customIconFixture } from './mockCustomIcons';
import { buildFixture } from './mockBuild';
import { conflictCopiesFixture } from './mockConflictCopies';
import { syncClashesFixture } from './mockSyncClashes';
import { reviewsFixture } from './mockReviews';
import { recordingsFixture } from './mockRecordings';
import { rejectIfDamagedStore } from './mockDamagedStores';
import { mockAutomationWindows, mockSetAutomationWindows } from './mockAutomationWindows';
import { mockPhoneAccess, mockPhonePushTest } from './mockPhoneAccess';
import { mockPeerState, mockPeers } from './mockPeers';
import { mockPhoneLid } from './mockPhoneLid';
import { mockCleanWorktrees, mockDiskGuard, mockHollowDatabases, mockInstallTargets, mockPruneInstallTargets, mockRemoveStaleIndexes, mockStaleIndexes, mockWorktreeHygiene } from './mockDiskCleanup';
import { askCanvasFixtureTurns, isMockCanvasCommand, mockAskCanvasNeedsYou, mockCanvasCommand } from './mockCanvases';
import { isMockTrackerCommand, mockTrackerCommand } from './mockTracker';
import { isMockEventTriggersCommand, mockEventTriggersCommand } from './mockEventTriggers';
import { isMockMoveHomeCommand, mockMoveHomeCommand } from './mockMoveHome';
import { isMockContextLayersCommand, mockContextLayersCommand } from './mockContextLayers';
import { isMockSourcesCommand, mockSourcesCommand } from './mockSources';

import { isMockContextReviewCommand, mockContextReviewCommand } from './mockContextReview';
import { isMockFirstRunCommand, mockFirstRunCommand } from './mockFirstRun';

import type {
  ActivationInventory, AgentSessionSummary, Dashboard, Diagnostics, GitActivity, OperatorContext,
  ProfileBuilderView,
  Commitment, Goal, HistoryRun, LearningContext, LearningProposal, Notification,
  Project, SearchResult, SemanticStatus, SessionMeta, SetupStatus,
  ReviewInboxItem, RunQueueItem, Skill, Task, VaultGraph, VaultItem, VaultTree, Workflow, WorkflowRunResult,
  UsageBucket, UsageSummary, ClaudeQuota, ClaudeAccountsView, ModelPrice, ModelPriceRow,
  LiveAgentSession, SendAccepted, ChatInteraction, ResolvedInteraction, InteractionAnswer,
  BudgetDef, BudgetInput, BudgetStatus, BudgetsView, RunLogEntry, RepoSidecar,
  RunContextView,
} from './types';
import type { AgentToolPath, AgentToolPathEntry, ChainView, SubagentsView, SelfHostedEntry, SelfHostedJobKind, SelfHostedJobs, SelfHostedModels, SelfHostedTest, SelfHostedWake, SelfHostedWakeStatus, ShellPathRead, UpdateGoBackPlan, UpdatePerson, UpdatePublishOutcome, UpdatePublishStatus, UpdateSettings, UpdateState } from './generated/ipc';
import type { EvidenceReceiptWire } from './executionReceipt';
import { isWorkspaceReviewItem, WORKSPACE_REVIEW_KINDS } from './workspaceContent';
import type { NeedsYouItem, NeedsYouPage } from './workspaceTypes';
import { scenarioHook } from './mockScenarios';
import { WORKFLOW_REVIEW_REFUSAL_PREFIX } from './workflowPermissions';

const NOW = '2026-07-15T13:00:00-04:00';
/** The legacy Claude fixture's ask, just before NOW (Paperclip PR 17). */
const LEGACY_ASKED_AT = '2026-07-15T12:59:00-04:00';

/** A Settings, Usage bucket; every count not given is zero, costs null. */
function usageBucket(key: string, label: string, fields: Partial<UsageBucket>): UsageBucket {
  return {
    key, label, runs: 0, metered: 0, partial: 0, unknown: 0, notStarted: 0, inProgress: 0, unpriced: 0,
    inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, reportedCostUsd: null, estimatedCostUsd: null,
    ...fields,
  };
}

/**
 * Settings, Claude accounts. Module state so add / remove / routing round
 * trip within a page load; tokens are accepted and never stored or returned.
 */
let mockAccountsState: ClaudeAccountsView | null = null;
// One chat's desktop (CU8): a mock desktop with two actions, so the chip and
// card have something to show.
const MOCK_FRAME = 'data:image/svg+xml;utf8,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="320" height="200" fill="#1c2430"/><rect x="24" y="24" width="272" height="28" rx="6" fill="#2c3a4c"/><text x="40" y="43" font-size="14" fill="#cfd8e3" font-family="sans-serif">Notes</text></svg>');
let mockComputerHeld: 'pause' | 'takeover' | null = null;
let mockComputerEntries: Array<{ id: number; atMs: number; tool: string; tier: string; target: string; outcome: string; detail: string; app: string | null; hasFrame: boolean }> = [
  { id: 2, atMs: Date.now() - 4000, tool: 'click', tier: 'act', target: 'sandboxVm', outcome: 'ok', detail: 'ran', app: null, hasFrame: true },
  { id: 1, atMs: Date.now() - 9000, tool: 'get_window_state', tier: 'observe', target: 'sandboxVm', outcome: 'ok', detail: 'ran', app: null, hasFrame: true },
];
const mockComputerChat = () => ({ paused: mockComputerHeld !== null, takenOver: mockComputerHeld === 'takeover', entries: mockComputerEntries, frameId: mockComputerEntries.find(e => e.hasFrame)?.id ?? null });
// This Mac's default-denied apps (CU6), all blocked until allowed.
const mockDeniedAllowed: Record<string, boolean> = {};
let mockAgentDenied = [{ pattern: 'tracker.example.net', addedAt: '2026-11-20T01:00:00Z', by: 'chat s-20261120-mock' }];
const mockDeniedApps = () => ['Terminal', 'iTerm', 'Ghostty', 'Warp', 'Alacritty', 'kitty', 'WezTerm', 'Script Editor', 'System Settings', 'Keychain Access', 'Passwords', '1Password', 'Bitwarden', 'LastPass', 'Dashlane', 'KeePassXC', 'Cua Driver', 'Hollow']
  .map(label => ({ label, allowed: mockDeniedAllowed[label] === true }));
/** Settings, Models, Self-hosted models (Q4; `{app data}/self-hosted-models.json`
 *  in the app): kept in memory, empty at first like a new install. A smoke
 *  spec may seed it as JSON in `mockSelfHosted` (Q6). */
let mockSelfHosted: SelfHostedEntry[] = (() => {
  try { return JSON.parse(localStorage.getItem('mockSelfHosted') ?? '[]') as SelfHostedEntry[]; } catch { return []; }
})();
/** Q6: each entry's last wake, as `self_hosted_wake.rs` keeps it. */
const mockWakes = new Map<string, { startedAt: number; result: SelfHostedWake | null }>();
const MOCK_SELF_HOSTED_PRESET: SelfHostedEntry = {
  id: 'qwen', name: 'Qwen3.8 27B', baseUrl: 'https://qwen-27b-x6jmsblvea-uc.a.run.app', apiStyle: 'anthropic',
  servedModel: 'qwen3.8-27b', contextWindow: 131072, effortLevels: ['low', 'medium', 'xhigh'],
  auth: { kind: 'tokenCommand', program: 'gcloud', args: ['auth', 'print-identity-token'] },
  healthPath: '/health', enabled: true,
};
function selfHostedView(): SelfHostedModels {
  return {
    entries: mockSelfHosted.map(e => ({ ...e, effortLevels: [...e.effortLevels] })), preset: MOCK_SELF_HOSTED_PRESET,
    levels: ['low', 'medium', 'high', 'xhigh', 'max'], minContextWindow: 110000, maxContextWindow: 10000000, error: null, invalid: {},
  };
}
/** Its Background jobs (Q10; `{app data}/self-hosted-jobs.json` in the app):
 *  kind to entry id, kept in memory; nothing ever waits in the mock. */
let mockSelfHostedJobs: Partial<Record<SelfHostedJobKind, string>> = {};
const MOCK_JOB_KINDS: { kind: SelfHostedJobKind; label: string; about: string }[] = [
  { kind: 'extraction', label: 'Recording extraction', about: 'Promises, tasks, decisions and facts found in a kept recording transcript or a chat range.' },
  { kind: 'titles', label: 'Chat and session titles', about: "A new chat's model title after its first answer, and titles in the session list." },
  { kind: 'learning', label: 'Learning extraction', about: 'The learnings Hollow proposes after your own chat turns.' },
  { kind: 'meetings', label: 'Meeting recaps and prep packs', about: 'Prepare, Recap and the prep pack a check-in writes before a meeting.' },
  { kind: 'profile', label: 'About-you scan', about: 'The About-you scan that reads what your agents know about you.' },
  { kind: 'heartbeat', label: 'Heartbeat', about: 'The background check of your vault every few minutes.' },
];
function selfHostedJobsView(): SelfHostedJobs {
  return {
    kinds: MOCK_JOB_KINDS.map(k => {
      const entry = mockSelfHostedJobs[k.kind] ?? null;
      const saved = entry ? mockSelfHosted.find(e => e.id === entry) : undefined;
      const problem = !entry ? null
        : !saved ? `Skipped while there is no self-hosted model "${entry}" in Settings → Models → Self-hosted models: these jobs never run on another model instead.`
        : !saved.enabled ? `Skipped while ${saved.name} is switched off: these jobs never run on another model instead.`
        : null;
      return { ...k, entry, problem };
    }),
    waiting: [], held: [], error: null, queueError: null,
  };
}
// Updates (row U5): the updater client's state, kept in localStorage so a
// spec can seed a phase (`mockUpdateState`, a whole UpdateState) and read
// what the window asked (`mockUpdateInstalled`, `mockUpdateWaiting`).
function mockUpdateSettings(): UpdateSettings {
  return {
    channel: 'main', installOvernight: false, skippedVersion: null, dismissedVersion: null,
    lastCheckedAt: null, allowDowngradeOnce: false, allowDowngradeSinceMs: null,
    personName: null, allowedChannels: null, publisher: false, dismissedLocalBuild: null,
    ...JSON.parse(localStorage.getItem('mockUpdateSettings') ?? '{}'),
  };
}
function saveMockUpdateSettings(patch: Partial<UpdateSettings>): UpdateSettings {
  const next = { ...mockUpdateSettings(), ...patch };
  localStorage.setItem('mockUpdateSettings', JSON.stringify(next));
  return next;
}
/** Publish to Main (row U8). `mockPublishStatus` overrides fields; `mockPublishMain` is the live Main. */
function mockPublishStatus(pick: string | null): UpdatePublishStatus {
  const publisher = mockUpdateSettings().publisher;
  const base: UpdatePublishStatus = {
    publisher, runner: 'runner-host', available: publisher,
    unavailableReason: publisher ? null : "This Mac doesn't publish to Main. Turn on This Mac publishes to Main in Settings, Updates first.",
    nightly: null, main: null, builds: [], proposed: null, proposedWhy: null, notes: null,
    changesAhead: null, daysAhead: null, canWithdraw: false, previousMain: null, history: [], nudge: false,
  };
  if (!publisher) return base;
  const history = JSON.parse(localStorage.getItem('mockPublishHistory') ?? '["0.6.90"]') as string[];
  const main = history[history.length - 1] ?? null;
  const builds = ['0.6.104', '0.6.102', '0.6.100', '0.6.90'].map((version, i) => ({
    version, commit: String(i + 1).repeat(40), pubDate: `2026-11-${15 - i}T10:00:00Z`,
    ranHere: version === '0.6.100' || version === '0.6.102', newerThanMain: !main || Number(version.split('.')[2]) > Number(main.split('.')[2]),
  }));
  const proposed = pick ?? builds.find(b => b.ranHere && b.newerThanMain)?.version ?? null;
  return {
    ...base,
    nightly: { version: '0.6.104', commit: builds[0].commit, pubDate: builds[0].pubDate },
    main: main ? { version: main, commit: builds.find(b => b.version === main)?.commit ?? null, pubDate: '2026-11-01T10:00:00Z' } : null,
    builds, proposed, proposedWhy: proposed ? null : 'Main already has the newest kept build',
    notes: proposed ? { pullRequests: [{ number: 1470, title: 'Publish to Main', merge: 'abcdef12', mergedAt: '2026-11-15T09:00:00Z' }], whatsNew: [{ title: 'Publish to Main', what: 'Settings, Updates publishes a Nightly to Main.' }], commits: [], note: null } : null,
    changesAhead: 23, daysAhead: 4, nudge: true,
    canWithdraw: history.length > 1, previousMain: history.length > 1 ? history[history.length - 2] : null,
    history: history.map((version, i) => ({ version, action: 'publish', at: '2026-11-01T11:00:00Z', from: i ? history[i - 1] : null })),
    ...JSON.parse(localStorage.getItem('mockPublishStatus') ?? '{}'),
  };
}

function mockUpdateState(): UpdateState {
  const settings = mockUpdateSettings();
  const seeded = JSON.parse(localStorage.getItem('mockUpdateState') ?? 'null') as Partial<UpdateState> | null;
  const connected = localStorage.getItem('mockUpdateConnected') !== '0';
  const phase = seeded?.phase ?? { phase: 'upToDate' as const, skipped: settings.skippedVersion };
  return {
    runningVersion: '0.6.100', configured: true, note: null,
    ...seeded,
    phase,
    channel: settings.channel, connected,
    pendingConnect: localStorage.getItem('mockUpdatePendingConnect') === '1',
    waiting: localStorage.getItem('mockUpdateWaiting') === '1',
    goBackWaiting: localStorage.getItem('mockUpdateGoBackWaiting') === '1',
    dismissed: phase.phase === 'ready' && settings.dismissedVersion === phase.release.version,
    skippedVersion: settings.skippedVersion, downgradeAllowed: settings.allowDowngradeOnce,
    lastCheckedAt: settings.lastCheckedAt,
    allowedChannels: settings.allowedChannels ?? ['main', 'nightly'], personName: settings.personName,
  };
}
// Updates, People (row U9): `mockUpdatePeople` holds the list a spec can seed.
function mockUpdatePeople(): UpdatePerson[] {
  return JSON.parse(localStorage.getItem('mockUpdatePeople') ?? '[]') as UpdatePerson[];
}
function saveMockUpdatePerson(id: unknown, patch: Partial<UpdatePerson>): UpdatePerson {
  if (!mockUpdateSettings().publisher) throw "People is only on the publisher's Mac.";
  const people = mockUpdatePeople();
  const person = people.find(p => p.id === id);
  if (!person) throw 'The release builder said: no person has that id.';
  Object.assign(person, patch);
  localStorage.setItem('mockUpdatePeople', JSON.stringify(people));
  return person;
}
function mockGoBackPlan(): UpdateGoBackPlan {
  const state = mockUpdateState();
  return {
    available: true, disabledReason: null,
    runningVersion: state.runningVersion, runningInstalledAt: '2026-11-15T09:30:00.000Z',
    previous: { version: '0.6.99', installedAt: '2026-11-14T08:00:00.000Z' },
    schema: { verdict: 'compatible' }, snapshot: { at: '2026-11-15T09:29:58.000Z', files: 1, intact: true, problem: null },
    restoreAvailable: true, indexesOff: [], willSkip: state.runningVersion,
    skippedVersion: state.skippedVersion, waiting: state.goBackWaiting,
    ...JSON.parse(localStorage.getItem('mockGoBackPlan') ?? '{}'),
  };
}

function mockSelfHostedEntry(id: unknown): SelfHostedEntry {
  const entry = mockSelfHosted.find(e => e.id === id);
  if (!entry) throw `There is no self-hosted model with the id "${String(id)}".`;
  return entry;
}
/** Settings, Keyboard shortcuts (`{app data}/shortcuts.json` in the app). */
let mockShortcuts: { enabled: boolean; keys: Record<string, string>; leader: Record<string, string> } = { enabled: true, keys: {}, leader: {} };
function claudeAccountsFixture(): ClaudeAccountsView {
  if (mockAccountsState) return mockAccountsState;
  // Relative times, so resets always read as upcoming.
  const inHours = (h: number) => new Date(Date.now() + h * 3_600_000).toISOString();
  const observedAt = inHours(-0.1);
  mockAccountsState = {
    routing: 'spread', pinned: 'acct-0000cafe', ceilingPercent: 90, failover: true, nextAccount: 'acct-0000beef', error: null,
    accounts: [
      {
        id: 'acct-0000cafe', label: 'Main plan', enabled: true, status: 'near', note: null,
        usage: { fiveHour: { usedPercent: 93, resetsAt: inHours(2) }, sevenDay: { usedPercent: 61, resetsAt: inHours(70) }, status: 'allowed_warning', overageDisabledReason: null, source: 'probe', observedAt },
        lastUsedAt: inHours(-0.3), createdAt: '2026-07-09T12:00:00Z',
      },
      {
        id: 'acct-0000beef', label: 'Second Max plan', enabled: true, status: 'ok', note: null,
        usage: { fiveHour: { usedPercent: 12, resetsAt: inHours(4) }, sevenDay: { usedPercent: 34, resetsAt: inHours(120) }, status: 'allowed', overageDisabledReason: null, source: 'probe', observedAt },
        lastUsedAt: null, createdAt: '2026-07-10T12:00:00Z',
      },
    ],
  };
  return mockAccountsState;
}

/** Two stream-observed plan windows, as the recorded Claude runs printed them. */
function claudeQuotaFixture(): ClaudeQuota {
  const observedAt = '2026-07-15T16:55:00Z';
  return {
    source: 'stream',
    observedAt,
    windows: [
      {
        id: 'five_hour', label: '5-hour session', usedPercent: 101,
        resetsAt: '2026-07-15T19:10:00Z', source: 'stream', observedAt,
      },
      {
        id: 'seven_day', label: 'Weekly · all models', usedPercent: 30,
        resetsAt: '2026-07-19T07:00:00Z', source: 'stream', observedAt,
      },
    ],
  };
}

/**
 * Deterministic usage-ledger summary: a reported and an estimated Claude
 * cost kept apart, an unpriced Codex agent, a teammate delegation credited
 * to its automation, and one each of partial / didn't start / in progress
 * plus two unknown runs. Nothing here is ever a bare $0.
 */
/** Settings, Usage, Prices (overnight 9 A13): two of the built-in rows. */
const MOCK_BUILT_IN_PRICES: ModelPrice[] = [
  { model: 'claude-haiku-5-5', input: 0.1, cacheWrite: 0.2, cacheRead: 0.01, output: 0.5, source: 'Anthropic launch price, prompts up to 100K tokens (2026-10-07)', asOf: '2026-10-07' },
  { model: 'claude-opus-5-5', input: 4, cacheWrite: 8, cacheRead: 0.2, output: 20, source: 'Anthropic list price (claude-api model table cached 2026-09-25); cache write = 2× input (one-hour cache)', asOf: '2026-09-25' },
];
let mockSavedPrices: ModelPrice[] = [];

function mockPriceTable(): ModelPriceRow[] {
  const rows: ModelPriceRow[] = mockSavedPrices.map(p => ({ ...p, saved: true }));
  for (const p of MOCK_BUILT_IN_PRICES) if (!rows.some(r => r.model === p.model)) rows.push({ ...p, saved: false });
  return rows.sort((a, b) => a.model.localeCompare(b.model));
}

function usageSummaryFixture(period: string, agent?: string | null): UsageSummary {
  const all = usageSummaryAll(period);
  if (!agent) return all;
  // Settings PR 8: one agent tab. The mock keeps every list and swaps the total.
  const total = all.agents?.find(a => a.key === agent);
  return { ...all, agent, total: total ? { ...total, key: 'total', label: 'All runs' } : usageBucket('total', 'All runs', {}) };
}

function usageSummaryAll(period: string): UsageSummary {
  return {
    window: (period === 'week' || period === 'month' || period === 'last30' ? period : 'day'),
    agent: null,
    agents: [
      usageBucket('claude', 'Claude Code', {
        runs: 8, metered: 4, partial: 1, unknown: 1, notStarted: 1, inProgress: 1,
        inputTokens: 42_440, cachedInputTokens: 300_300, outputTokens: 4_620,
        reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
      usageBucket('codex', 'Codex', {
        runs: 3, metered: 2, unknown: 1, unpriced: 2,
        inputTokens: 15_800, cachedInputTokens: 112_600, outputTokens: 1_500,
      }),
    ],
    byModel: [
      usageBucket('claude-opus-4-7', 'claude-opus-4-7', { runs: 6, metered: 3, partial: 1, unknown: 1, inProgress: 1, inputTokens: 38_000, cachedInputTokens: 280_000, outputTokens: 4_200, reportedCostUsd: 0.42, estimatedCostUsd: 2.6 }),
      usageBucket('gpt-6-astra', 'gpt-6-astra', { runs: 3, metered: 2, unknown: 1, unpriced: 2, inputTokens: 15_800, cachedInputTokens: 112_600, outputTokens: 1_500 }),
      usageBucket('claude-sonnet-4-6', 'claude-sonnet-4-6', { runs: 2, metered: 1, notStarted: 1, inputTokens: 4_440, cachedInputTokens: 20_300, outputTokens: 420, estimatedCostUsd: 0.5 }),
    ],
    outcomes: [
      usageBucket('finished', 'Finished', { runs: 7, metered: 6, unknown: 1, inputTokens: 52_000, cachedInputTokens: 380_000, outputTokens: 5_600, reportedCostUsd: 0.42, estimatedCostUsd: 3.1 }),
      usageBucket('failed', 'Failed', { runs: 1, unknown: 1 }),
      usageBucket('moved', 'Moved to another account', { runs: 1, partial: 1, inputTokens: 6_240, cachedInputTokens: 32_900, outputTokens: 520 }),
      usageBucket('not_started', "Didn't start", { runs: 1, notStarted: 1 }),
      usageBucket('running', 'Still running', { runs: 1, inProgress: 1 }),
    ],
    topConversations: [
      usageBucket('mock-coo', 'COO', {
        runs: 4, metered: 2, partial: 1, unknown: 1, inputTokens: 21_400, cachedInputTokens: 180_300,
        outputTokens: 2_400, reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
    ],
    since: '2026-07-15T04:00:00.000Z',
    generatedAt: '2026-07-15T17:00:00.000Z',
    total: usageBucket('total', 'All runs', {
      runs: 11, metered: 6, partial: 1, unknown: 2, notStarted: 1, inProgress: 1, unpriced: 2,
      inputTokens: 58_240, cachedInputTokens: 412_900, outputTokens: 6_120,
      reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
    }),
    bySurface: [
      usageBucket('chat', 'Chat', {
        runs: 4, metered: 2, partial: 1, unknown: 1, inputTokens: 21_400, cachedInputTokens: 180_300,
        outputTokens: 2_400, reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
      usageBucket('workflow', 'Automations', {
        runs: 2, metered: 1, inProgress: 1, inputTokens: 18_900, cachedInputTokens: 120_000, outputTokens: 1_900,
      }),
      usageBucket('oneshot', 'Assistants', { runs: 2, metered: 1, notStarted: 1, inputTokens: 2_140, outputTokens: 320 }),
      usageBucket('delegate', 'Teammates', {
        runs: 2, metered: 1, unknown: 1, unpriced: 1, inputTokens: 11_849, cachedInputTokens: 11_904, outputTokens: 5,
      }),
      usageBucket('ingest', 'Imports', {
        runs: 1, metered: 1, unpriced: 1, inputTokens: 3_951, cachedInputTokens: 100_696, outputTokens: 1_495,
      }),
    ],
    byWorkflow: [
      usageBucket('morning-brief', 'Morning brief', {
        runs: 3, metered: 2, inProgress: 1, unpriced: 1,
        inputTokens: 30_749, cachedInputTokens: 131_904, outputTokens: 1_905,
      }),
    ],
    byAgent: [
      usageBucket('claude', 'Claude Code', {
        runs: 8, metered: 4, partial: 1, unknown: 1, notStarted: 1, inProgress: 1,
        inputTokens: 42_440, cachedInputTokens: 300_300, outputTokens: 4_620,
        reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
      usageBucket('codex', 'Codex', {
        runs: 3, metered: 2, unknown: 1, unpriced: 2,
        inputTokens: 15_800, cachedInputTokens: 112_600, outputTokens: 1_500,
      }),
    ],
    byProject: [
      usageBucket('mock-hollow', 'Hollow', {
        runs: 7, metered: 4, partial: 1, unknown: 1, inProgress: 1, inputTokens: 40_100, cachedInputTokens: 300_000,
        outputTokens: 4_000, reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
      usageBucket('mock-notes', 'Notes', { runs: 3, metered: 2, unknown: 1, inputTokens: 15_000, cachedInputTokens: 100_000, outputTokens: 1_500 }),
      usageBucket('none', 'No project', { runs: 1, metered: 1, notStarted: 0, inputTokens: 3_140, cachedInputTokens: 12_900, outputTokens: 620 }),
    ],
    byLead: [
      usageBucket('mock-coo', 'COO', {
        runs: 4, metered: 2, partial: 1, unknown: 1, inputTokens: 21_400, cachedInputTokens: 180_300,
        outputTokens: 2_400, reportedCostUsd: 0.42, estimatedCostUsd: 3.1,
      }),
    ],
    bySubagent: [
      usageBucket('mock-sub-1', 'Review the ledger', { runs: 2, metered: 2, inputTokens: 9_000, cachedInputTokens: 60_000, outputTokens: 900, estimatedCostUsd: 1.2 }),
      usageBucket('mock-sub-2', 'Check the budgets', { runs: 1, metered: 1, inputTokens: 4_000, cachedInputTokens: 20_000, outputTokens: 300, estimatedCostUsd: 0.4 }),
    ],
    byAccount: [
      usageBucket('login', 'Claude Code login', {
        runs: 5, metered: 3, partial: 1, unknown: 1, inputTokens: 30_440, cachedInputTokens: 200_300,
        outputTokens: 3_120, reportedCostUsd: 0.42,
      }),
      usageBucket('acct-0a1b2c3d4e5f', 'Work account', { runs: 3, metered: 3, inputTokens: 12_000, cachedInputTokens: 100_000, outputTokens: 1_500, estimatedCostUsd: 3.1 }),
      usageBucket('agent:codex', 'Codex (its own login)', { runs: 3, metered: 2, unknown: 1, unpriced: 2, inputTokens: 15_800, cachedInputTokens: 112_600, outputTokens: 1_500 }),
    ],
    daily: [{ date: '2026-07-15', total: usageBucket('2026-07-15', '2026-07-15', {runs:11,partial:1,unknown:2,inProgress:1,notStarted:1,unpriced:2,reportedCostUsd:0.42,estimatedCostUsd:3.1}), byAgent: [usageBucket('claude','Claude Code',{runs:8,reportedCostUsd:0.42,estimatedCostUsd:3.1}),usageBucket('codex','Codex',{runs:3,unpriced:2,unknown:1})], nightByAgent: [] }],
    undatedRuns: 0, nightAttributionError: null,
    ledgerSince: '2026-07-01T12:00:00.000Z',
    ledgerError: null,
  };
}
const mockWrittenNotes = new Map<string, string>();
/** Library rename, move and trash (P1-18): paths that left the vault list, and those a move created. */
const mockGoneNotes = new Set<string>();
const mockMovedNotes = new Set<string>();
/** A seeded list plus the notes moved into it; a seeded note moved back (Undo) is listed once. */
const withMovedNotes = <N extends { path: string; basename: string }>(seeded: N[]) =>
  [...seeded, ...[...mockMovedNotes].filter(path => !seeded.some(n => n.path === path)).map(path => ({ path, basename: path.split('/').pop()!.replace(/\.md$/, '') }))];
/** A workspace note as the mock workspaces see it (written this session, or
 *  saved in `mockWorkspaceNotes`): the curated canvas store hashes these. */
const mockWorkspaceNote = (path: string): string | null => {
  const written = mockWrittenNotes.get(path);
  if (written !== undefined) return written;
  try { return (JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>)[path] ?? null; } catch { return null; }
};
/** A change made outside Hollow (another editor, an agent, a sync), for the
 *  smoke tests: writes the mock file without an IPC call, so it is not in
 *  `__mockApiCalls`. The test then delivers `vault-changed` itself. */
try {
  (window as unknown as { __mockOutsideWrite?: (path: string, content: string, modified?: string) => void }).__mockOutsideWrite = (path, content, modified) => {
    mockWrittenNotes.set(path, content);
    if (path.startsWith('workspaces/')) {
      const stored = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>;
      stored[path] = content;
      localStorage.setItem('mockWorkspaceNotes', JSON.stringify(stored));
      if (modified) {
        const times = JSON.parse(localStorage.getItem('mockWorkspaceFileTimes') ?? '{}') as Record<string, string>;
        times[path] = modified;
        localStorage.setItem('mockWorkspaceFileTimes', JSON.stringify(times));
      }
    }
  };
} catch { /* non-browser context */ }

/** Paperclip PR 13: the seeded question cards (localStorage). */
/** `run_context_manifest` (agent effectiveness F11): a stored override
 *  (`mockRunContextManifest`, a RunContextView or `null`), else one local
 *  run with retrieval and no "when relevant" connectors. */
function mockRunContextManifest(now: string, runId?: string | null, sessionKey?: string | null): RunContextView | null {
  const stored = localStorage.getItem('mockRunContextManifest');
  if (stored) return JSON.parse(stored) as RunContextView | null;
  const id = runId || 'run-mock-latest';
  const hash = (c: string) => c.repeat(64);
  return {
    runId: id,
    source: 'chat',
    sessionKey: sessionKey || 'mock-session',
    finishedAt: now,
    manifest: {
      schemaVersion: 1,
      runId: id,
      layers: [
        { name: 'policy', bytes: 4200, sha256: hash('a') },
        { name: 'retrievalEvidence', bytes: 1800, sha256: hash('b') },
        { name: 'request', bytes: 42, sha256: hash('c') },
      ],
      retrieval: {
        engine: 'local',
        candidates: Array.from({ length: 8 }, (_, i) => ({
          path: `notes/mock-${i + 1}.md`, line: 1, source: 'note', coverage: 'excerpt', rank: i + 1,
          localScore: 2, kept: true, basis: 'local', bytes: 120, sha256: hash('d'),
        })),
        notices: [],
      },
      connectors: { engine: 'local', decisions: [] },
      jev: [],
      capabilities: { ownedServers: ['hollow_vault'], connectors: [] },
      fallbacks: [],
      preparation: { retrievalMs: 12, connectorsMs: 1 },
    },
    jevCalls: { sent: 0, notSent: 0, latencyMs: 0, outcomes: {}, costUsd: null },
    costRollup: { ownUsd: null, descendantsUsd: 0, totalUsd: null, knownUsd: 0, unknown: 1, descendants: [] },
  };
}

/** A seeded subagents view (`mockSubagents`), else none but the earlier
 *  chats (L8, as Rust `subagents::earlier_folder`): a project lead's
 *  (`s-project-…`) are its project's other chats, the COO's are the Chats
 *  folder's. */
function mockSubagents(parent: string, older = 0): SubagentsView {
  try {
    const seeded = JSON.parse(localStorage.getItem('mockSubagents') || '{}') as Record<string, SubagentsView>;
    // A seeded view may carry every older finished row; `older` of them are listed, as the backend does.
    if (seeded[parent]) {
      const v = seeded[parent];
      return { ...v, doneOlder: (v.doneOlder ?? []).slice(0, older), olderTotal: v.olderTotal ?? (v.doneOlder ?? []).length };
    }
  } catch { /* fall through */ }
  const at = new Date().toISOString();
  return { parent, parents: [parent], active: [], done: [], doneOlder: [], olderTotal: 0, earlier: mockEarlierChats(parent), running: 0, queued: 0, doneCount: 0, at };
}

function mockEarlierChats(parent: string): SubagentsView['earlier'] {
  try {
    const workspaces = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; cwd: string; kind?: string }[];
    const all = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as { key: string; cwd: string; title?: string | null; agent?: string; createdAt?: string; lastActivityAt?: string; archived?: boolean; forkKind?: string | null }[];
    const sessions = all.filter(s => !s.archived && s.forkKind !== 'subagent' && s.key !== parent);
    const lead = workspaces.find(w => !w.kind && `s-project-${sha256Hex(w.id).slice(0, 48)}` === parent);
    const main = workspaces.find(w => w.id === 'main' || w.kind === 'main');
    const isCoo = !!main && all.some(s => s.key === parent && s.cwd === main.cwd);
    const quick = workspaces.find(w => w.id === 'quick-chats' || w.kind === 'quick');
    const folder = lead?.cwd ?? (isCoo ? quick?.cwd : undefined);
    if (!folder) return [];
    return sessions.filter(s => s.cwd === folder)
      .sort((a, b) => String(b.lastActivityAt ?? '').localeCompare(String(a.lastActivityAt ?? '')))
      .map(s => ({
        key: s.key, current: s.key, title: s.title ?? null, agent: s.agent ?? 'claude', model: null, effort: null, account: null,
        status: 'done', queuePlace: null, waitingFor: null, startedAt: s.createdAt ?? NOW, endedAt: s.lastActivityAt ?? NOW, workingMs: 0, wallMs: 0,
        inputTokens: 0, cachedInputTokens: 0, outputTokens: 0, tokens: 0, costUsd: null, earlier: true, conversations: [s.key], trackerIssue: null, issue: null, freshFrom: null,
      }));
  } catch { return []; }
}

/** A seeded chain view (`mockChains`), else the conversation alone. */
function mockChain(key: string): ChainView {
  try {
    const seeded = JSON.parse(localStorage.getItem('mockChains') || '{}') as Record<string, ChainView>;
    if (seeded[key]) return seeded[key];
  } catch { /* fall through */ }
  return { key, current: key, currentTitle: null, main: false, earlier: [], forwards: [] };
}

function mockInteractions(): ChatInteraction[] {
  try {
    const raw = JSON.parse(localStorage.getItem('mockPendingInteractions') ?? '[]');
    return Array.isArray(raw) ? (raw as ChatInteraction[]) : [];
  } catch { return []; }
}
function writeMockInteractions(list: ChatInteraction[]) {
  localStorage.setItem('mockPendingInteractions', JSON.stringify(list));
}

/** A small stand-in for `interaction_schema::format_answers` (questions mode
 *  with option choices and text, each question by its whole prompt; other
 *  modes summarized). The real format is pinned by the Rust tests. */
function mockAnswerPair(lines: string[], question: string, value: string) {
  lines.push('', question.replace(/\s+/g, ' ').trim(), value);
}
function mockAnswerPrompt(card: ChatInteraction, answer: InteractionAnswer | undefined): string {
  const lines = ['Answered questions'];
  for (const q of card.request.questions ?? []) {
    const a = answer?.answers?.[q.id];
    const chosen = (q.options ?? []).filter(o => (a?.selected ?? []).includes(o.id)).map(o => o.label);
    if (a?.other) chosen.push(a.other);
    const value = q.answerMode === 'text' ? (a?.text?.trim() || 'No answer') : (chosen.join(', ') || 'No answer');
    mockAnswerPair(lines, q.prompt, value);
  }
  if (card.request.mode === 'confirmation' && card.request.confirmation) {
    mockAnswerPair(lines, card.request.confirmation.prompt, answer?.accepted ? 'Accepted' : 'Rejected');
  }
  return lines.join('\n');
}

/** Paperclip PR 7b: chat_send enqueues. Each mock send gets the next row id,
 *  so a smoke test can cancel it by id. */
const MOCK_CHAT_SESSION_KEY = 's-20260925-mock0001';
let mockQueueSeq = 0;
const mockChatRows = new Map<number, RunQueueItem>();

const project: Project = {
  url: 'vault://projects/hollow-3-0.md',
  name: 'Hollow 3.0',
  status: 'Active',
  description: 'Ship the 3.0 foundation, safety defaults, and daily cockpit.',
  created: '2026-07-07',
  vaultPath: 'projects/Hollow 3.0.md',
  priority: 'P1',
  targetDate: '2026-07-31',
  deployedAt: null,
  currentState: 'The safety foundation is partially implemented and still needs end-to-end verification.',
  desiredState: 'Hollow 3.0 ships with safe defaults and a dependable daily cockpit.',
  nonGoals: 'Rebuild every existing workspace in this project.',
  successEvidence: 'Smoke tests pass and the packaged app completes the core daily flow.',
  goalUrls: [],
};

const goal: Goal = {
  url: 'vault:goals/Dependable personal operating system.md',
  title: 'Dependable personal operating system',
  status: 'active',
  horizon: '2026',
  targetDate: '2026-12-31',
  why: 'Turn daily work into a trusted, recoverable operating loop.',
  successEvidence: 'Every active project has a clear outcome and recoverable execution trail.',
  created: NOW,
  vaultPath: 'goals/Dependable personal operating system.md',
};

const commitment: Commitment = {
  url: 'vault:commitments/send-jordan-release-notes.md',
  vaultPath: 'commitments/send-jordan-release-notes.md',
  title: 'Send Jordan the release notes',
  direction: 'owed_by_me',
  counterparty: 'Jordan',
  personPath: 'people/Jordan.md',
  projectUrl: project.url,
  due: '2026-07-13',
  status: 'open',
  channel: 'meeting',
  sourcePath: 'calendar/hollow-review.md',
  created: NOW,
  fulfilledAt: null,
  fingerprint: 'mock-commitment-fingerprint',
  context: 'Share the final verification summary after the build lands.',
};

/** The commitments ledger's demo (overnight 10-11 A5): both ways, overdue,
 *  undated, one snoozed, one done, and one found but not accepted. Kept in
 *  memory so `?mock=1` can triage them. */
const mockDay = (days: number) => { const d = new Date(); d.setDate(d.getDate() + days); return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`; };
const mockCommitment = (slug: string, over: Partial<Commitment>): Commitment => ({
  ...commitment, url: `vault:commitments/${slug}.md`, vaultPath: `commitments/${slug}.md`, fingerprint: `mock-${slug}`, projectUrl: null, personPath: null,
  sourcePath: null, context: '', channel: 'manual', ...over,
});
const mockCommitments: Commitment[] = [
  commitment,
  mockCommitment('q3-numbers-pat', { title: 'Q3 numbers for the board pack', counterparty: 'Pat Smith', due: mockDay(-2), channel: 'meeting', sourcePath: 'recordings/Weekly finance sync.md', context: 'Weekly finance sync (Oct 6): recordings/Weekly finance sync.md@00:14:32', evidence: "I'll get you the Q3 numbers before Friday's board pack." }),
  mockCommitment('atlas-pricing', { title: 'Atlas pricing sheet v2', counterparty: 'Ana Ruiz', due: mockDay(3) }),
  mockCommitment('vendor-export', { title: 'Vendor export for September', direction: 'owed_to_me', counterparty: 'Raj', due: mockDay(-1), channel: 'meeting', sourcePath: 'recordings/Vendor call.md', context: 'Vendor call (Oct 8): recordings/Vendor call.md@00:09:10', evidence: 'Raj: we can have the September export over to you by Friday.' }),
  mockCommitment('venue-list', { title: 'Venue shortlist for the offsite', direction: 'owed_to_me', counterparty: 'Ana Ruiz', due: null }),
  mockCommitment('tipflow-intro', { title: 'Intro to the TipFlow payroll contact', counterparty: 'Priya', due: mockDay(-4), snoozedUntil: mockDay(2) }),
  mockCommitment('hris-audit-notes', { title: 'HRIS audit notes', counterparty: 'Pat Smith', status: 'fulfilled', fulfilledAt: new Date().toISOString() }),
];

const learningProposal: LearningProposal = {
  id: 'proposal-learning-1',
  status: 'pending',
  createdAt: NOW,
  updatedAt: NOW,
  fingerprint: 'mock-learning-fingerprint',
  payload: {
    kind: 'preference',
    targetKind: 'operator',
    targetPath: 'me/Guardrails.md',
    subject: 'Release verification',
    statement: 'Verify the installed app, not only the source build.',
    rationale: 'The operator corrected a source-only completion claim.',
    evidenceExcerpt: 'Build, install, and verify the live app.',
    sourceRunId: 'run-chat-1',
  },
  error: null,
  version: 'v-mock-learning-1',
};

const reviewInboxItem: ReviewInboxItem = {
  ...learningProposal,
  kind: 'learning',
  status: 'pending',
  payload: learningProposal.payload as unknown as Record<string, unknown>,
  risk: 'local_write',
  source: 'chat',
  sourcePath: null,
  snoozedUntil: null,
};

/** A hard-stop budget pause card (Paperclip PR 8a): 1,200 of 1,000 tokens
 *  today. Its display payload only; the backend re-derives the pause. */
const budgetPauseItem: ReviewInboxItem = {
  id: 'review-budget-pause-1',
  kind: 'budget_pause',
  status: 'pending',
  createdAt: NOW,
  updatedAt: NOW,
  fingerprint: 'budget-pause:b-brief:2026-07-15t17:00:00.000z',
  payload: {
    budgetId: 'b-brief', episode: '2026-07-15T17:00:00.000Z', scope: 'workflow:morning-brief',
    scopeLabel: 'the “morning-brief” automation', metric: 'tokens', window: 'day',
    windowKey: '2026-07-15', windowEnd: '2026-07-16T04:00:00.000Z', limit: 1000, observed: 1200,
    pausedAt: '2026-07-15T17:00:00.000Z', suggestedLimit: 1800, estimated: false,
    enforcement: ENFORCEMENT_COPY,
  },
  risk: 'local_write',
  source: 'budget',
  sourcePath: null,
  snoozedUntil: null,
  error: null,
};

/** The review items every mock session starts with, shared by
 *  `list_review_inbox` and `resolve_review_item`. */
const DEFAULT_REVIEW_ITEMS = JSON.stringify([reviewInboxItem, budgetPauseItem]);

/** Settings, Usage budgets (`mockBudgets` round-trips the stored defs;
 *  `mockLedgerDays` sets how much history exists, default 9). */
function mockBudgetDefs(): BudgetDef[] {
  return JSON.parse(localStorage.getItem('mockBudgets') ?? JSON.stringify([{
    id: 'b-brief', scope: 'workflow:morning-brief', metric: 'tokens', window: 'day', limit: 1000,
    warnPercent: 80, hardStop: true, createdAt: NOW, updatedAt: NOW,
  }])) as BudgetDef[];
}

/** Observed usage per budget in the mock: 1,200 tokens / $0.42 today. */
function mockObserved(def: BudgetDef): number {
  return def.metric === 'tokens' ? 1200 : 0.42;
}

function budgetsViewFixture(): BudgetsView {
  const ledgerDays = Number(localStorage.getItem('mockLedgerDays') ?? '9');
  const suggestions = ledgerDays >= 7;
  const paused = JSON.parse(localStorage.getItem('mockBudgetPaused') ?? '["b-brief"]') as string[];
  const budgets: BudgetStatus[] = mockBudgetDefs().map((def): BudgetStatus => {
    const observed = mockObserved(def);
    const pause = paused.includes(def.id)
      ? { windowKey: '2026-07-15', windowEnd: '2026-07-16T04:00:00.000Z', episode: '2026-07-15T17:00:00.000Z',
        pausedAt: '2026-07-15T17:00:00.000Z', observed, limit: def.limit, held: false, reviewItemId: budgetPauseItem.id }
      : null;
    return {
      ...def,
      scopeLabel: def.scope === 'global' ? 'all usage' : def.scope.startsWith('workflow:') ? `the “${def.scope.slice(9)}” automation` : def.scope,
      windowKey: '2026-07-15', windowStart: '2026-07-15T04:00:00.000Z', windowEnd: '2026-07-16T04:00:00.000Z',
      observed, percent: observed / def.limit * 100,
      state: pause ? 'paused' : observed >= def.limit ? 'over' : observed >= def.limit * def.warnPercent / 100 ? 'warning' : 'ok',
      pause, runs: 3, unknownRuns: 0, partialRuns: 0, unpricedRuns: 0, cachedInputTokens: 5000,
      estimated: def.metric === 'costUsd', enforceability: 'ok', enforceabilityReason: null, scopeMissing: false,
      suggestedLimit: suggestions ? Math.ceil(observed * 1.5) : null, ledgerError: null,
    };
  });
  return {
    budgets, ledgerDays, suggestionsAvailable: suggestions,
    blocked: {
      allAutomations: false, heartbeat: false,
      workflows: budgets.filter(b => b.state === 'paused' && b.scope.startsWith('workflow:')).map(b => b.scope.slice(9)),
    },
    storeError: null, foreignEdit: false,
    enforcement: ENFORCEMENT_COPY,
  };
}

const activationInventory: ActivationInventory = {
  activeProjectCount: 1,
  planningProjectCount: 0,
  projectOutcomeGaps: [],
  alignment: {
    activeGoalCount: 1,
    activeProjectCount: 1,
    healthyGoalCount: 1,
    healthyProjectCount: 1,
    activeGoalsWithoutActiveProjects: [],
    activeProjectsWithoutGoals: [],
    brokenGoalUrls: [],
    projectsOnlyLinkedToInactiveGoals: [],
  },
  sourceCounts: { email: 0, chat: 0, meeting: 2, recording: 1 },
  commitmentCount: 1,
  learningCount: 1,
  pendingActivationProposals: 1,
  dismissedActivationFingerprints: 0,
};

const learnedContext: LearningContext = {
  path: 'learnings/use-vault.md',
  title: 'Use vault notes',
  targetKind: 'operator',
  targetPath: 'me/Guardrails.md',
  subject: 'Workspace ownership',
  statement: 'Keep tasks and projects in the Obsidian vault.',
  sourceRunId: 'run-chat-0',
  learnedAt: NOW,
  supersedes: 'learnings/tasks-in-notion.md',
  supersededBy: null,
};

// The correction it replaced (next-level P18: Memory shows superseded ones).
const outdatedContext: LearningContext = {
  path: 'learnings/tasks-in-notion.md',
  title: 'Tasks in Notion',
  targetKind: 'operator',
  targetPath: 'me/Guardrails.md',
  subject: 'Where tasks live',
  statement: 'Keep tasks in Notion only.',
  sourceRunId: 'run-chat-0',
  learnedAt: NOW,
  supersedes: null,
  supersededBy: 'learnings/use-vault.md',
};

const historyRun: HistoryRun = {
  id: 'run-workflow-1',
  source: 'workflow',
  sourceId: 'morning-brief',
  startedAt: '2026-07-07T07:00:00-04:00',
  finishedAt: '2026-07-07T07:01:00-04:00',
  status: 'error',
  changes: [{
    vaultPath: 'briefings/2026-07-07.md',
    changeType: 'create',
    beforeHash: null,
    afterHash: 'mock-after-hash',
    beforeSize: null,
    afterSize: 512,
    text: true,
    notionMirror: false,
  }, {
    vaultPath: 'projects/Hollow 3.0.md',
    changeType: 'modify',
    beforeHash: 'mock-project-before',
    afterHash: 'mock-project-after',
    beforeSize: 410,
    afterSize: 420,
    text: true,
    notionMirror: true,
    beforeNotionId: 'mock-project-page',
    afterNotionId: 'mock-project-page',
  }],
};

const workflowReceipt: EvidenceReceiptWire = {
  id: 'receipt-workflow-1',
  runId: historyRun.id,
  source: 'workflow',
  sourceId: 'morning-brief',
  startedAt: historyRun.startedAt,
  finishedAt: historyRun.finishedAt!,
  status: 'error',
  verificationState: 'failedAfterChange',
  files: [{ path: 'briefings/2026-07-07.md', changeType: 'create', attribution: 'confirmed', tool: 'Write' }],
  checks: [{ command: 'npm test', kind: 'test', outcome: 'failed', sequence: 4, excerpt: 'fixture failure' }],
  failures: [{ label: 'Bash', sequence: 4, excerpt: 'fixture failure' }],
  historyRunId: historyRun.id,
  // Paperclip PR 16: a linked project the brief also touched.
  repoChanges: [{
    root: '/Users/fixture/Projects/app',
    label: 'app',
    target: 'linkedProject',
    status: 'captured',
    headBefore: '1111111111111111111111111111111111111111',
    headAfter: '1111111111111111111111111111111111111111',
    files: [
      {
        path: 'src/lib.rs', changeType: 'modify', status: ' M', untracked: false, gitlink: false,
        additions: 3, deletions: 1, binary: false, oversized: false, patchTruncated: false,
        withheld: false, dirtyBefore: false, attribution: 'observed', tool: null,
      },
      {
        path: 'notes/todo.md', changeType: 'create', status: '??', untracked: true, gitlink: false,
        additions: 2, deletions: 0, binary: false, oversized: false, patchTruncated: false,
        withheld: false, dirtyBefore: false, attribution: 'confirmed', tool: 'Write',
      },
    ],
    totalFiles: 2,
    truncated: false,
  }],
  // Paperclip PR 15c: first output was never measured, so it is absent.
  phaseTimings: { queueWaitMs: 1_250, baselineSnapshotMs: 84, spawnMs: 2_300, turnMs: 125_000, repoCaptureMs: 140, finalizeDiffMs: 96, evidenceMs: 3 },
  // Repo features R5: the brief's cites, one past the end of its file.
  cites: {
    checked: 3,
    seen: 2,
    pastEnd: 1,
    items: [{ path: 'src/lib.rs', line: 412, status: 'pastEnd' }],
  },
};

/** The same run's repository sidecar, with patches (`read_history_repo_changes`). */
const workflowRepoSidecar: RepoSidecar = {
  schemaVersion: 1,
  runId: historyRun.id,
  capturedAt: historyRun.finishedAt!,
  repos: workflowReceipt.repoChanges!.map(repo => ({
    ...repo,
    files: repo.files.map(file => ({
      ...file,
      patch: file.path === 'src/lib.rs'
        ? 'diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,2 +1,4 @@\n-fn old() {}\n+fn brief() {}\n+fn more() {}\n+fn most() {}\n'
        : 'diff --git a/notes/todo.md b/notes/todo.md\nnew file mode 100644\n--- /dev/null\n+++ b/notes/todo.md\n@@ -0,0 +1,2 @@\n+- [ ] one\n+- [ ] two\n',
    })),
  })),
};

/** One multi-stint session: started on Claude, moved to Codex. That shape is
 *  the whole point of Hollow-owned identity, so the harness exercises it. */
/** `mockReplyWork=1` (overnight 11-12, C1 and C2): a finished reply that read
 *  three files in 1m 10s and wrote three, so the default conversation shows
 *  its file chips, its "Worked for 1m 10s" line and its row of actions. */
function replyWorkFixtureTurns(at: string) {
  const start = Date.parse(at);
  const started = new Date(start + 60_000).toISOString();
  const finished = new Date(start + 130_000).toISOString();
  const read = (id: string, path: string) => ({ id, name: 'Read', input: { file_path: path }, result: 'ok', isError: false, status: 'done' as const });
  const receipt: EvidenceReceiptWire = {
    id: 'receipt-reply-work-1', runId: 'run-reply-work-1', source: 'chat', sourceId: agentSession.key,
    startedAt: started, finishedAt: finished, status: 'success', verificationState: 'noChanges',
    files: ['brief.md', 'findings.md', 'handoff.md'].map(name => ({ path: `notes/delta/${name}`, changeType: 'modify', attribution: 'confirmed', tool: 'Edit' })),
    checks: [], failures: [], historyRunId: 'run-reply-work-1',
  };
  return [
    { id: 'reply-work-user', role: 'user', text: 'What do the Delta notes add?', tools: [], agent: 'claude', runId: 'run-reply-work-1', startedAt: started, transcriptIncomplete: false },
    {
      id: 'reply-work-answer', role: 'assistant', agent: 'claude', runId: 'run-reply-work-1', startedAt: started, status: 'success', transcriptIncomplete: false,
      tools: [read('rw-1', 'notes/delta/brief.md'), read('rw-2', 'notes/delta/findings.md'), read('rw-3', 'notes/delta/handoff.md')],
      text: 'They show how the earlier investigation was run and where its evidence came from.\n\n- `notes/delta/brief.md` records the original scope.\n- `notes/delta/findings.md` ties each conclusion to its evidence.\n- `notes/delta/handoff.md` keeps the open questions.',
      receipt,
    },
  ];
}

const agentSession: AgentSessionSummary = {
  key: 's-20260811-fixture1',
  title: 'Wire the session store',
  agent: 'codex',
  harnessSessionId: null,
  cwd: '/Users/fixture/vault',
  createdAt: '2026-08-11T09:00:00Z',
  lastActivityAt: '2026-08-11T09:42:00Z',
  endedAt: null,
  turnCount: 2,
  parentKey: null,
  forkKind: null,
  adopted: false,
  stints: [
    {
      agent: 'claude',
      harnessSessionId: 'claude-fixture-session',
      startedAt: '2026-08-11T09:00:00Z',
      endedAt: '2026-08-11T09:20:00Z',
      reason: 'agent changed',
    },
    {
      agent: 'codex',
      harnessSessionId: null,
      startedAt: '2026-08-11T09:20:00Z',
      endedAt: null,
      reason: null,
    },
  ],
  resumable: 'briefOnly',
};

const tasks: Task[] = [
  {
    url: 'vault://tasks/finish-artifact-safety.md',
    title: 'Finish artifact safety defaults',
    status: 'todo',
    priority: 'P1',
    due: '2026-07-15',
    effort: '45m',
    description: 'Make HTML previews offline and scriptless until explicitly elevated.',
    projectUrl: project.url,
    projectName: project.name,
    daysUntilDue: 0,
    bucket: 'today',
    vaultPath: 'tasks/Finish artifact safety defaults.md',
    labels: ['hollow', 'security'],
    section: 'Foundation',
    dueTime: '14:00',
    durationMin: 45,
    recurrence: null,
    subtasks: [{ text: 'Add policy controls', done: true }],
    contentHash: 'mock-sha-finish-artifact-safety',
    fingerprint: null,
    origin: null,
    // Read-only reserved sections (Paperclip PR 11) for the Tasks pane.
    doneWhen: 'HTML previews open offline and without scripts by default.',
    activity: '- 2026-09-01 Opened from the security review.',
    blockedBy: [],
    blocks: [],
    ready: true,
    unresolvedBlockers: [],
    blockerWarnings: [],
  },
  {
    url: 'vault://tasks/rebuild-today.md',
    title: 'Sketch the 3.0 daily cockpit',
    status: 'todo',
    priority: 'P2',
    due: '2026-07-15',
    effort: '1h',
    description: 'Define Now, Next, Waiting, Decisions, At Risk, and Recently Changed lanes.',
    projectUrl: project.url,
    projectName: project.name,
    daysUntilDue: -1,
    bucket: 'overdue',
    vaultPath: 'tasks/Sketch daily cockpit.md',
    labels: ['product'],
    section: 'Daily core',
    dueTime: null,
    durationMin: null,
    recurrence: null,
    subtasks: [],
    contentHash: 'mock-sha-rebuild-today',
    fingerprint: null,
    origin: null,
    doneWhen: null,
    activity: null,
    blockedBy: [],
    blocks: [],
    ready: true,
    unresolvedBlockers: [],
    blockerWarnings: [],
  },
];

/** The task list the mock edits: the demo fixture's when present (next-level). */
const mockTaskList = (): Task[] => (window as unknown as { __mockDemoTasks?: Task[] }).__mockDemoTasks ?? tasks;

const workflows: Workflow[] = [
  {
    slug: 'morning-brief',
    title: 'Morning Brief',
    description: 'Daily operating brief.',
    schedule: '0 0 7 * * * *',
    agent: 'claude',
    model: null,
    cwd: null,
    outputGlob: 'briefs/*.html',
    routine: 'morning-brief',
    permissionMode: 'default',
    allowedTools: [],
    disallowedTools: [],
    triggers: [],
    created: '2026-07-01',
    lastRunAt: '2026-07-07T07:01:00-04:00',
    lastStatus: 'error',
    lastError: 'Fixture failure for smoke coverage',
    lastDurationSec: 12,
    nextRunAt: '2026-07-08T07:00:00-04:00',
    misfireGraceMin: null,
    skipIfQuiet: false,
    watch: [],
    notify: null,
    lastStartedAt: '2026-07-07T07:00:48-04:00',
    path: 'workflows/morning-brief.md',
  },
];

const vaultTree: VaultTree = {
  projects: {
    kind: 'folder',
    children: {
      'Hollow 3.0.md': {
        kind: 'file',
        name: 'Hollow 3.0.md',
        path: 'projects/Hollow 3.0.md',
        mtime: NOW,
        size: 420,
      },
    },
  },
  tasks: {
    kind: 'folder',
    children: {
      'Finish artifact safety defaults.md': {
        kind: 'file',
        name: 'Finish artifact safety defaults.md',
        path: 'tasks/Finish artifact safety defaults.md',
        mtime: NOW,
        size: 320,
      },
    },
  },
};

const vaultItems: VaultItem[] = [
  {
    path: 'projects/Hollow 3.0.md',
    title: 'Hollow 3.0',
    kind: 'project',
    surface: 'projects',
    folder: 'projects',
    editable: true,
    hidden: false,
    trust: 'authored',
    mtime: NOW,
    size: 420,
    excerpt: '3.0 migration plan and implementation notes.',
    drift: null,
  },
  {
    path: 'loose/Untitled.md',
    title: 'Untitled',
    kind: 'unfiled',
    surface: 'vault',
    folder: 'loose',
    editable: true,
    hidden: false,
    trust: 'authored',
    mtime: NOW,
    size: 80,
    excerpt: 'Needs a home.',
    drift: 'Unknown folder: loose',
  },
];

const graph: VaultGraph = {
  nodes: [
    { id: 'projects/Hollow 3.0.md', label: 'Hollow 3.0', folder: 'projects', kind: 'project', inDegree: 0, outDegree: 1 },
    { id: 'tasks/Finish artifact safety defaults.md', label: 'Finish artifact safety defaults', folder: 'tasks', kind: 'task', inDegree: 1, outDegree: 0 },
    { id: 'people/Jordan.md', label: 'Jordan', folder: 'people', kind: 'person', inDegree: 0, outDegree: 0 },
    { id: 'loose/Untitled.md', label: 'Untitled', folder: 'loose', kind: 'unfiled', inDegree: 0, outDegree: 0 },
  ],
  edges: [{ source: 'projects/Hollow 3.0.md', target: 'tasks/Finish artifact safety defaults.md' }],
};

const dashboard: Dashboard = {
  sessions: [],
  sessionsTotal: 0,
  recentNotes: [{ name: 'Hollow 3.0.md', path: 'projects/Hollow 3.0.md', mtime: NOW, size: 420 }],
  tasks,
  tasksTotal: tasks.length,
  overdueCount: 1,
  todayCount: 1,
  activeProjects: [project],
  projectsTotal: 1,
  tasksFetched: NOW,
  projectsFetched: NOW,
  timestamp: NOW,
};

const setupStatus: SetupStatus = {
  configured: true,
  configPath: '/tmp/hollow/config.json',
  configError: null,
  vaultPath: '/tmp/hollow/vault',
  vaultExists: true,
};

const diagnostics: Diagnostics = {
  vaultPath: '/tmp/hollow/vault',
  vaultExists: true,
  configPath: '/tmp/hollow/config.json',
  claudeBinary: '/usr/local/bin/claude',
  debugLogDir: '/tmp/hollow/logs',
  debugLogEnabled: false,
  debugLogFiles: 0,
};

/** The persisted About you switch: never set, so on (the backend default). */
let operatorSwitch = { enabled: true, chosen: false };

const operatorContext: OperatorContext = {
  body: '# Operator\n\n## me/Identity.md\n\n- **Name:** Jordan\n- **Role:** Operator\n\nUse direct, pragmatic language.',
  files: ['me/Identity.md'],
  charCount: 101,
  truncated: false,
  freshness: [
    { path: 'me/Identity.md', title: 'Identity', exists: true, lastUpdated: '2026-06-01', lastReviewed: '2026-03-01', reviewCadenceDays: 90, status: 'overdue', dueAt: '2026-05-30', daysUntilDue: -44 },
    { path: 'me/Communication.md', title: 'Communication', exists: true, lastUpdated: '2026-06-01', lastReviewed: '2026-04-20', reviewCadenceDays: 90, status: 'due', dueAt: '2026-07-19', daysUntilDue: 6 },
    { path: 'me/Anti-patterns.md', title: 'Anti-patterns', exists: true, lastUpdated: '2026-06-15', lastReviewed: '2026-06-15', reviewCadenceDays: 60, status: 'fresh', dueAt: '2026-08-14', daysUntilDue: 32 },
    { path: 'me/Guardrails.md', title: 'Guardrails', exists: true, lastUpdated: '2026-06-01', lastReviewed: null, reviewCadenceDays: 90, status: 'neverReviewed', dueAt: null, daysUntilDue: null },
    { path: 'me/Decisions.md', title: 'Decisions', exists: false, lastUpdated: null, lastReviewed: null, reviewCadenceDays: 30, status: 'neverReviewed', dueAt: null, daysUntilDue: null },
  ],
};

// About you builder. Starts never scanned, so opening Settings, Personalization
// auto-reads; the scan fills two facts and one question. Mutated by the
// accept / dismiss / answer mocks like the real store.
const profileBuilder: ProfileBuilderView = {
  suggestions: [],
  questions: [],
  sources: [],
  profile: [
    { path: 'me/Identity.md', title: 'Identity', exists: true, items: ['Name: Jordan'], blanks: 2 },
    { path: 'me/Communication.md', title: 'Communication', exists: true, items: [], blanks: 3 },
    { path: 'me/Anti-patterns.md', title: 'Anti-patterns', exists: true, items: [], blanks: 2 },
    { path: 'me/Guardrails.md', title: 'Guardrails', exists: true, items: ['Never delete or move files outside this vault.'], blanks: 0 },
    { path: 'me/Decisions.md', title: 'Decisions', exists: false, items: [], blanks: 0 },
  ],
  scannedAt: null,
  scanning: false,
  lastError: null,
  // `mockProfileBuilderLearn` = 'off' starts with Keep learning off (P0-11).
  autoLearn: (() => { try { return localStorage.getItem('mockProfileBuilderLearn') !== 'off'; } catch { return true; } })(),
};

function profileBuilderSnapshot(): ProfileBuilderView {
  return JSON.parse(JSON.stringify(profileBuilder)) as ProfileBuilderView;
}

function fileProfile(file: string, line: string) {
  const entry = profileBuilder.profile.find(p => p.path === `me/${file}`);
  if (!entry) return;
  entry.exists = true;
  entry.items.push(line);
  if (entry.blanks > 0) entry.blanks -= 1;
}

const gitActivity: GitActivity = {
  days: [{ date: '2026-07-07', count: 3 }],
  totalCommits: 3,
  activeDays: 1,
  currentStreak: 1,
  longestStreak: 1,
  reposScanned: 1,
  start: '2026-07-01',
  end: '2026-07-07',
};

// The bell panel groups by day and dates rows relative to now, so these are
// built off the clock rather than fixed strings. One unread error (the badge's
// attention state), one unread routine row (the quiet count), a spread of
// sources for the row glyphs, and one `vault:` target absent from
// `vault_notes_list` so the moved-target CTA is visible.
function agoIso(minutes: number): string {
  return new Date(Date.now() - minutes * 60_000).toISOString();
}

// `mockNotificationsExtra` (smoke tests) adds rows on top, newest first.
const notifications: Notification[] = [
  ...(JSON.parse(localStorage.getItem('mockNotificationsExtra') ?? '[]') as Notification[]),
  {
    id: 'mock-notif-1', title: 'Morning Brief wrote briefings/morning.html',
    body: '2 files written · 41s', kind: 'success', source: 'morning-brief',
    actionLabel: 'Open brief', actionUrl: 'vault:briefings/morning.html',
    created: agoIso(45), read: false,
  },
  {
    id: 'mock-notif-2', title: 'Close my day failed',
    body: 'claude exited with status 1. Run claude, then /login.',
    kind: 'error', source: 'close-my-day',
    actionLabel: 'Open routine', actionUrl: 'workflow:close-my-day',
    created: agoIso(180), read: false,
  },
  {
    id: 'mock-notif-3', title: 'Ship the 3.0 safety defaults is 4 days overdue',
    body: 'P1 · due 2026-07-11', kind: 'warning', source: 'nudge',
    actionLabel: 'Open task', actionUrl: 'vault:tasks/ship-safety-defaults.md',
    created: agoIso(600), read: true,
  },
  {
    id: 'mock-notif-4', title: 'Two projects have no next action',
    body: 'Hollow 3.0 and Vault cleanup both stalled.', kind: 'info', source: 'heartbeat',
    actionLabel: 'Open project', actionUrl: 'vault:projects/Hollow 3.0.md',
    created: agoIso(1560), read: true,
  },
  {
    id: 'mock-notif-5', title: 'New meeting recordings',
    body: '1 recording synced.', kind: 'info', source: 'plaud',
    actionLabel: null, actionUrl: null,
    created: agoIso(6000), read: true,
  },
];

/** Skills made with New skill in this page load (`write_user_skill`). */
const mockCreatedSkills: Skill[] = [];
/** Their SKILL.md as Rust writes it, so Edit skill reads back what it saved (P1-57). */
const mockSkillFiles = new Map<string, string>();
/** Local notes on shared skills (repo ideas 10-27 #2), this page load only. */
const mockSkillNotes = new Map<string, string>();

/**
 * Mutations the smoke harness fires and deliberately treats as no-ops: they
 * return nothing, and the fixture has no state for them to change. Each entry
 * is a conscious decision, unlike the prefix-matching heuristic this replaced,
 * which quietly swallowed 108 of 209 commands.
 */
const NO_OP_COMMANDS = new Set<string>([
  // The Workflows page's Stop (Paperclip PR 15b found it unmocked); the
  // harness has no running workflow process to stop.
  'cancel_workflow',
  // The window tells the phone chat API the mode for new chats and each
  // chat's mode choice; the harness has no phone to send with them.
  'phone_access_new_chat_mode',
  'phone_access_chat_modes',
  // Reveal a build workspace's worktree in Finder: nothing to open in a browser.
  'chat_preview_open',
  'set_active_theme',
  'set_vault_mcp_enabled',
  'set_code_context_mcp_enabled',
  'set_tasks_mcp_enabled',
  'set_app_mcp_enabled',
  'set_semantic_search_enabled',
  'set_nudges_enabled',
  'set_teamwork_enabled',
  'set_computer_control_enabled',
  'set_heartbeat_enabled',
  'set_heartbeat_drafts_enabled',
  'set_heartbeat_config',
  'set_notify_native',
  'set_notify_native_enabled',
  // Paperclip PR 22: the foreground notification scope (Workflows.tsx).
  'set_foreground_notification_scope',
  // Paperclip PR 21: an Allow/Deny click on an approval card. Had no mock
  // before; the approval smoke test counts its calls.
  'approval_respond',
  'set_agent_role',
  'set_minimize_to_tray',
  'set_window_vibrancy',
  // 0.4 (#1661): the dock badge follows the unseen-finished count; the coordinator
  // sets it once at mount (0 clears a stale one), so every page calls it.
  'set_dock_badge',
  'clear_debug_logs',
  'prevent_close',
]);

/**
 * Commands the harness has hit with no fixture behind them. Recorded as well
 * as thrown, because plenty of call sites catch their own IPC errors — a throw
 * alone can be swallowed and leave the gap invisible. `tests/smoke` asserts
 * this list is empty, so an unmocked command fails the suite either way.
 */
declare global {
  interface Window {
    __mockApiMisses?: string[];
    /** Per-command invoke counts. The only way to assert that a fetch was
     *  dropped, deduplicated or debounced: "this command ran once, not nine
     *  times" is not observable from the DOM. */
    __mockApiCalls?: Record<string, number>;
    /** Every `create_task` input, in order (Paperclip PR 11 smoke tests). */
    __mockCreatedTasks?: unknown[];
    /** A `create_task` title that fails once, with a plain string as Tauri
     *  rejects (Paperclip PR 11's ProjectModal retry smoke test). */
    __mockFailCreateTaskOnce?: string;
    /** A command that rejects once, with a plain string as Tauri does (plan
     *  completion L1: a failed handoff, e.g. `ensure_quick_chats_workspace`). */
    __mockFailCommandOnce?: string;
    /** Hold the next answer to `command` (H6 follow-up): it is read when
     *  called, as the backend would, and delivered once `release` settles;
     *  `delivered` turns true as it is handed back. Lets a smoke test land a
     *  read after a save without a fixed delay. */
    __mockHoldOnce?: { command: string; release: Promise<unknown>; delivered?: boolean };
  }
}

function recordMiss(command: string): void {
  try {
    const misses = (window.__mockApiMisses ??= []);
    if (!misses.includes(command)) misses.push(command);
  } catch { /* non-browser context */ }
}

/** `window.__mockHoldOnce`, taken if it is for `command`. */
function takeHold(command: string): NonNullable<Window['__mockHoldOnce']> | undefined {
  try {
    const hold = window.__mockHoldOnce;
    if (hold?.command !== command) return undefined;
    window.__mockHoldOnce = undefined;
    return hold;
  } catch { return undefined; /* non-browser context */ }
}

function recordCall(command: string): void {
  try {
    const calls = (window.__mockApiCalls ??= {});
    calls[command] = (calls[command] ?? 0) + 1;
  } catch { /* non-browser context */ }
}

/** Workflows the smoke test marks refused (`mockRefusedWorkflows`) carry the
 *  backend's `permissionHold`, as `list_workflows` reports it. */
function withPermissionHolds(list: Workflow[]): Workflow[] {
  const refused = JSON.parse(localStorage.getItem('mockRefusedWorkflows') ?? '[]') as string[];
  // Paperclip PR 8a: `mockBudgetHeld` slugs carry the backend's budgetHold.
  const held = JSON.parse(localStorage.getItem('mockBudgetHeld') ?? '[]') as string[];
  // Paperclip PR 8b: `mockBudgetStopped` slugs last ended at a budget limit.
  const stopped = JSON.parse(localStorage.getItem('mockBudgetStopped') ?? '[]') as string[];
  // Overnight S10: `mockWorkflowSchedules` {slug: schedule} puts fixtures on a window.
  const schedules = JSON.parse(localStorage.getItem('mockWorkflowSchedules') ?? '{}') as Record<string, string>;
  return list.map(fixture => {
    const override = schedules[fixture.slug];
    const base = fixture;
    const workflow = override
      ? { ...base, schedule: override, ...mockScheduleProblem(override) }
      : base;
    const withPermission = refused.includes(workflow.slug) ? { ...workflow, permissionHold: 'review' as const } : workflow;
    const withBudget = held.includes(workflow.slug)
      ? { ...withPermission, budgetHold: 'Budget paused: this automation reached today\'s token limit (1,200 tokens of 1,000 tokens).' }
      : withPermission;
    return stopped.includes(workflow.slug)
      ? { ...withBudget, lastStatus: 'error' as const, lastError: MOCK_BUDGET_STOP }
      : withBudget;
  });
}

/** Owner Mac (M4): `mockWorkflowsExtra` adds automations (each a partial
 *  over the Morning Brief fixture, with its own slug), and `mockWorkflowRunsOn`
 *  {slug: runs_on} sets owners. An editor save that sends `runsOn` writes it
 *  there (`this` as this Mac's id, '' removes), as the backend would. */
function mockWorkflowList(): Workflow[] {
  const extra = JSON.parse(localStorage.getItem('mockWorkflowsExtra') ?? '[]') as Partial<Workflow>[];
  const owners = JSON.parse(localStorage.getItem('mockWorkflowRunsOn') ?? '{}') as Record<string, string>;
  const all = [...workflows, ...extra.map(w => ({ ...workflows[0], routine: null, lastStatus: null, lastError: null, ...w }) as Workflow)];
  return withPermissionHolds(all).map(w => (owners[w.slug] ? { ...w, runsOn: owners[w.slug] } : w));
}

function mockSaveWorkflow(args: Record<string, unknown> | undefined): Workflow {
  const input = (args?.input ?? {}) as Partial<Workflow> & { runsOn?: string };
  // The last input as the editor sent it, for a spec to read.
  (window as unknown as { __mockLastWorkflowInput?: unknown }).__mockLastWorkflowInput = structuredClone(input);
  const list = mockWorkflowList();
  const before = list.find(w => w.slug === args?.originalSlug) ?? list[0];
  if (input.runsOn !== undefined) {
    const owners = JSON.parse(localStorage.getItem('mockWorkflowRunsOn') ?? '{}') as Record<string, string>;
    const owner = input.runsOn.trim() === 'this' ? mockPeerState().thisMac.machineId : input.runsOn.trim();
    if (owner) owners[before.slug] = owner; else delete owners[before.slug];
    localStorage.setItem('mockWorkflowRunsOn', JSON.stringify(owners));
    input.runsOn = owner || undefined;
  }
  return { ...before, ...input } as Workflow;
}

/** B04: what `list_workflows` reports for a schedule `validate_schedule`
 *  refuses. The mock's rule is rougher than the backend's: anything that is
 *  not `manual`, a `window:` schedule or six or seven space-separated cron
 *  fields (the year is optional).
 *  The text is the `cron` crate's own. */
function mockScheduleProblem(schedule: string): { scheduleProblem?: string } {
  const readable = schedule === 'manual' || schedule.startsWith('window:') || [6, 7].includes(schedule.trim().split(/\s+/).length);
  return readable ? {} : { scheduleProblem: 'Invalid expression: Invalid cron expression.' };
}

/** The error of a run a usage budget stopped (Rust `budgets::stop_notice`). */
const MOCK_BUDGET_STOP = 'Budget limit reached: the “morning-brief” automation reached its daily token limit (1,200 tokens of 1,000 tokens) during this run, so Hollow stopped it.';

/** The Morning Brief receipt, with `outsideEffects` per
 *  `localStorage.mockOutsideEffects` (plan completion L2): unset is an older
 *  receipt (unknown), `none` recorded nothing, `notion` a Notion page it
 *  created plus a `git push` and two shell commands Hollow can't see into. */
function mockWorkflowReceipt(): EvidenceReceiptWire {
  const sharedRecord = localStorage.getItem('mockRunRecord');
  if (sharedRecord) return { ...JSON.parse(sharedRecord), id: workflowReceipt.id, runId: workflowReceipt.runId, historyRunId: workflowReceipt.historyRunId, source: 'workflow', sourceId: 'morning-brief' } as EvidenceReceiptWire;
  const mode = localStorage.getItem('mockOutsideEffects');
  if (mode === 'none') return { ...workflowReceipt, outsideEffects: {} };
  if (mode === 'notion') {
    return {
      ...workflowReceipt,
      outsideEffects: {
        recorded: [
          { kind: 'connector', label: 'Notion · create page “Morning brief 2026-07-07”', tool: 'mcp__notion__notion-create-pages', outcome: 'done', key: 'mock-key-1', sequence: 2 },
          { kind: 'shell', label: 'git push · git push origin main', tool: 'Bash', outcome: 'uncertain', sequence: 3 },
        ],
        unknown: [{ kind: 'shell', label: 'Shell commands', count: 2, examples: ['python3 scripts/brief.py'] }],
      },
    };
  }
  return workflowReceipt;
}

function mockRunLog(slug: string): RunLogEntry[] {
  if (slug === 'morning-brief' && localStorage.getItem('mockRunRecordNoLatest')) return [
    { at: '2026-10-25T12:00:00Z', status: 'error', durationSec: 1, note: 'Evidence was unavailable', runId: 'missing-receipt', errorTail: null, exitKind: null },
    { at: historyRun.finishedAt!, status: 'success', durationSec: 12, note: 'Earlier run', runId: historyRun.id, errorTail: null, exitKind: null },
  ];
  const stopped = JSON.parse(localStorage.getItem('mockBudgetStopped') ?? '[]') as string[];
  if (stopped.includes(slug)) {
    return [{ at: historyRun.finishedAt!, status: 'error', durationSec: 9, note: MOCK_BUDGET_STOP, runId: historyRun.id, errorTail: null, exitKind: 'budget' }];
  }
  return [{ at: historyRun.finishedAt!, status: 'error', durationSec: 12, note: 'Fixture failure for smoke coverage', runId: historyRun.id, errorTail: 'API Error: 401 Invalid authentication\nclaude exited exit status: 1', exitKind: null }];
}

/** The "needs you" query in the browser mock (needs_you.rs): the fixture's
 *  results to review (`mockWorkspaces`), the review inbox's pending items,
 *  unread failure notifications, and `mockNeedsYouExtra` (seeded approvals,
 *  questions or conflicts). Newest first, ties by id; offsets stand in for
 *  the keyset cursor. `mockNeedsYouUnavailable` names unreadable sources. */
async function mockNeedsYou(args: Record<string, unknown> | undefined): Promise<NeedsYouPage> {
  const results = (workspaceFixture('list_needs_you', { kinds: ['result'], limit: 200 }, mockWrittenNotes)?.value as NeedsYouPage | undefined)?.items ?? [];
  const reviewItems = (JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[])
    .filter(item => isWorkspaceReviewItem(item) && (item.status === 'pending' || item.status === 'applying'))
    .map((item): NeedsYouItem => {
      const label = WORKSPACE_REVIEW_KINDS[item.kind as keyof typeof WORKSPACE_REVIEW_KINDS];
      const title = ['subject', 'title', 'projectName'].map(k => item.payload[k]).find(v => typeof v === 'string' && v.trim()) as string | undefined;
      return {
        id: `review:${item.id}`, kind: 'review', sourceId: item.id, title: title ?? label, detail: label, state: item.status,
        at: item.createdAt, workspaceId: null, sessionKey: null, path: item.sourcePath, category: item.kind, link: null,
      };
    });
  const failures = notifications.filter(n => n.kind === 'error' && !n.read).map((n): NeedsYouItem => ({
    id: `failure:${n.id}`, kind: 'failure', sourceId: n.id, title: n.title, detail: n.body || null, state: 'unread', at: n.created,
    workspaceId: null, sessionKey: null, path: n.actionUrl?.startsWith('vault:') ? n.actionUrl.slice(6) : null, category: n.source, link: n.actionUrl,
  }));
  const extra = JSON.parse(localStorage.getItem('mockNeedsYouExtra') ?? '[]') as NeedsYouItem[];
  const kinds = args?.kinds as string[] | null | undefined;
  const canvases = !kinds || kinds.includes('canvas') ? await mockAskCanvasNeedsYou(agentSession.key) : [];
  const categories = args?.categories as string[] | null | undefined;
  const all = [...results, ...reviewItems, ...failures, ...canvases, ...extra]
    .filter(item => !kinds || kinds.includes(item.kind))
    .filter(item => !categories || categories.includes(item.category ?? ''))
    .sort((a, b) => (a.at && b.at ? b.at.localeCompare(a.at) : a.at ? -1 : b.at ? 1 : 0) || a.id.localeCompare(b.id));
  const start = Math.min(Number(args?.cursor ?? 0), all.length);
  const limit = Math.min(200, Math.max(1, Number(args?.limit ?? 50)));
  const items = all.slice(start, start + limit);
  const end = start + items.length;
  const unavailable = JSON.parse(localStorage.getItem('mockNeedsYouUnavailable') ?? '[]') as string[];
  return { items, total: all.length, nextCursor: end < all.length ? String(end) : null, unavailable };
}

/** Settings, Advanced, Folders agents can run tools from (A12). localStorage
 *  `mockAgentToolPath` = 'failed' | 'pending' shows that note instead of a
 *  read shell PATH (no shell folders, as in the app). */
function mockAgentToolPath(): AgentToolPath {
  const mode = localStorage.getItem('mockAgentToolPath');
  const shell: ShellPathRead = mode === 'failed' ? { state: 'failed', detail: 'timed out after 5000 ms' }
    : mode === 'pending' ? { state: 'pending', detail: null }
    : { state: 'read', detail: null };
  const shellDirs: AgentToolPathEntry[] = shell.state === 'read' ? [
    { dir: '/Users/you/google-cloud-sdk/bin', from: 'shell', exists: true },
    { dir: '/Users/you/.bun/bin', from: 'shell', exists: true },
    { dir: '/Users/you/old-tools/bin', from: 'shell', exists: false },
    // A stalled network folder whose check didn't answer in time (Spare 1).
    { dir: '/Volumes/share/bin', from: 'shell', exists: null },
  ] : [];
  return {
    entries: [
      { dir: '/Users/you/.cargo/bin', from: 'hollow', exists: true },
      { dir: '/Users/you/.local/bin', from: 'hollow', exists: true },
      { dir: '/opt/homebrew/bin', from: 'hollow', exists: true },
      { dir: '/usr/local/bin', from: 'hollow', exists: false },
      ...shellDirs,
      { dir: '/usr/bin', from: 'inherited', exists: true },
      { dir: '/bin', from: 'inherited', exists: true },
      { dir: '/usr/sbin', from: 'system', exists: true },
      { dir: '/sbin', from: 'system', exists: true },
    ],
    shell,
  };
}

/** The CFO's policy in the preview (COO rebuild L7). */
function mockCfoSettings(): { allow: { agent: string; model?: string }[]; effortMode: 'automatic' | 'fixed'; reservePercent: number; extraUsageUntil?: string; projectBudgets: { workspaceId: string; fiveHourTokens?: number; weekTokens?: number }[]; subagentBudget: { tokens?: number; minutes?: number } } {
  const base = { projectBudgets: [], subagentBudget: {} };
  return { ...base, ...JSON.parse(localStorage.getItem('mockCfoSettings') ?? '{"allow":[],"effortMode":"automatic","reservePercent":20,"projectBudgets":[],"subagentBudget":{}}') };
}

function mockCfoView() {
  const settings = mockCfoSettings();
  const ceiling = 100 - settings.reservePercent;
  const open = !!settings.extraUsageUntil && new Date(settings.extraUsageUntil).getTime() > Date.now();
  const seeded = JSON.parse(localStorage.getItem('mockCfoAccounts') ?? 'null') as { agent: string; account: string; label: string; fiveHourPercent: number | null; weeklyPercent: number | null; resetsAt?: string | null }[] | null;
  const rows = seeded ?? [
    { agent: 'claude', account: 'acct-1', label: 'Personal', fiveHourPercent: 42, weeklyPercent: 18, resetsAt: new Date(Date.now() + 2 * 3600_000).toISOString() },
    { agent: 'codex', account: 'login', label: 'Codex login', fiveHourPercent: null, weeklyPercent: null, resetsAt: null },
  ];
  const accounts = rows.map(row => {
    const used = Math.max(row.fiveHourPercent ?? 0, row.weeklyPercent ?? 0);
    const measured = row.fiveHourPercent !== null || row.weeklyPercent !== null;
    const room = !measured || used < ceiling || (used >= 100 && open);
    const note = !measured ? 'Not measured yet; its first run measures it' : room ? 'Room for unattended work' : used >= 100 ? 'Out of included usage; extra usage needs your approval' : `Past the reserve (${ceiling}% line) in its 5-hour window`;
    return { agent: row.agent, account: row.account, label: row.label, fiveHourPercent: row.fiveHourPercent, weeklyPercent: row.weeklyPercent, fiveHourResetsAt: row.resetsAt ?? null, weeklyResetsAt: null, room, roomAt: room ? null : row.resetsAt ?? null, note };
  });
  return {
    settings, poolCeilingPercent: ceiling, extraUsageOpen: open, accounts,
    levels: [['conversation', 'low'], ['question', 'low'], ['report', 'medium'], ['review', 'medium'], ['research', 'high'], ['plan', 'high'], ['code', 'high']].map(([kind, level]) => ({ kind, level })),
    reserveChoices: [0, 10, 20, 30, 40, 50],
    // Each budgeted project's usage now: 60% of the 5-hour figure and 20% of
    // the weekly one, unless localStorage `mockCfoUsage` names a project's own.
    projectUsage: settings.projectBudgets.map(budget => {
      const own = (JSON.parse(localStorage.getItem('mockCfoUsage') ?? '{}') as Record<string, { fiveHourTokens: number; weekTokens: number }>)[budget.workspaceId];
      const fiveHourTokens = own?.fiveHourTokens ?? Math.round((budget.fiveHourTokens ?? 1_000_000) * 0.6);
      const weekTokens = own?.weekTokens ?? Math.round((budget.weekTokens ?? 5_000_000) * 0.2);
      const reached = (budget.fiveHourTokens !== undefined && fiveHourTokens >= budget.fiveHourTokens) || (budget.weekTokens !== undefined && weekTokens >= budget.weekTokens);
      return { workspaceId: budget.workspaceId, fiveHourTokens, weekTokens, reached };
    }),
  };
}

export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const hold = takeHold(command);
  if (hold) {
    const value = await mockInvoke<T>(command, args);
    await hold.release;
    hold.delivered = true;
    return value;
  }
  recordCall(command);
  if (command === 'agent_models' && args?.agent === 'codex' && localStorage.getItem('mockCodexCatalogError')) {
    localStorage.removeItem('mockCodexCatalogError');
    throw new Error('Synthetic Codex catalog unavailable');
  }
  try {
    if (window.__mockFailCommandOnce === command) { window.__mockFailCommandOnce = undefined; throw `mock ${command} failure`; }
  } catch (e) { if (typeof e === 'string') throw e; /* non-browser */ }
  rejectIfDamagedStore(command);
  // Named whole-app scenarios (mockScenarios.ts): a delay or a rejection.
  if (window.__mockScenario) await scenarioHook(command);
  if (command.endsWith('_custom_icon') || command === 'select_app_icon_preset' || command === 'generate_logo_options') {
    const iconFixture = await customIconFixture(command, args);
    if (iconFixture) return iconFixture.value as T;
  }
  // M2's peer link (`mockPeers.ts`): no other Mac until a spec seeds one.
  if (command.startsWith('peer_') || command.startsWith('chat_move_') || command === 'chat_offline_rows' || command === 'chat_continue_offline') return mockPeers(command, args) as T;
  const recFixture = recordingsFixture(command, args);
  if (recFixture) return recFixture.value as T;
  if (command === 'list_needs_you') return await mockNeedsYou(args) as T;
  if (command === 'companion_backend_status' && localStorage.getItem('mockCompanionBackend')) return JSON.parse(localStorage.getItem('mockCompanionBackend')!) as T;
  // The Main agent's day (asks 10-11 A2): seed `mockMainAgentStatus` to see
  // its watching / paused states; with none it has nothing to report.
  if (command === 'main_agent_status') return JSON.parse(localStorage.getItem('mockMainAgentStatus') ?? 'null') as T;
  // Its limits (Settings, Background work, Main agent): opt-in, so the section
  // keeps its old shape unless a spec seeds `mockMainAgentSettings` (A07):
  // unseeded it answers null, which every reader treats as unread (Budgets'
  // built-in rows read it on every Usage open since P1-07).
  if (command === 'get_main_agent_settings') return JSON.parse(localStorage.getItem('mockMainAgentSettings') ?? 'null') as T;
  if (command === 'set_main_agent_settings' && localStorage.getItem('mockMainAgentSettings')) {
    localStorage.setItem('mockMainAgentSettings', JSON.stringify(args?.settings));
    return args?.settings as T;
  }
  // How many chats run at once (Settings, Background work, 2026-10-20): the
  // defaults until a spec saves others (`mockChatLimits`).
  // A value saved before a field existed reads as that field's default.
  if (command === 'get_chat_limits') return { chatsAtOnce: 10, chatsPerWorkspace: 5, subagentsPerChat: 6, ...JSON.parse(localStorage.getItem('mockChatLimits') ?? '{}') } as T;
  if (command === 'set_chat_limits') {
    // Whole numbers only, like the native u32 fields.
    const l = args?.limits as { chatsAtOnce: unknown; chatsPerWorkspace: unknown; subagentsPerChat: unknown };
    const whole = (n: unknown, max: number) => typeof n === 'number' && Number.isInteger(n) && n >= 1 && n <= max;
    if (!whole(l.chatsAtOnce, CHAT_LIMIT_MAX.chatsAtOnce)) throw `Run 1 to ${CHAT_LIMIT_MAX.chatsAtOnce} chats at once.`;
    if (!whole(l.chatsPerWorkspace, CHAT_LIMIT_MAX.chatsPerWorkspace)) throw `Run 1 to ${CHAT_LIMIT_MAX.chatsPerWorkspace} chats at once in one workspace.`;
    if (!whole(l.subagentsPerChat, CHAT_LIMIT_MAX.subagentsPerChat)) throw `Run 1 to ${CHAT_LIMIT_MAX.subagentsPerChat} subagents at once from one chat.`;
    localStorage.setItem('mockChatLimits', JSON.stringify(l));
    return l as T;
  }
  // The Main agent's started chats (its presence reads them): none in the mock.
  if (command === 'main_agent_children') return [] as T;
  if (command === 'main_agent_activity') return [] as T;
  // The fixture conversation is in no workspace: "waiting on you" opens it
  // on the Chat page by its exact key (C4 A3).
  if (command === 'session_target' && args?.key === agentSession.key) return { workspaceId: null, session: agentSession } as T;
  if (command === 'canvas_item_source' || command === 'canvas_plan_items') {
    const { mockCanvasItems } = await import('./canvas/mockItems');
    return await mockCanvasItems(command, args ?? {}, mockWorkspaceNote, (path, content) => {
      mockWrittenNotes.set(path, content);
      const stored = JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}') as Record<string, string>;
      stored[path] = content;
      localStorage.setItem('mockWorkspaceNotes', JSON.stringify(stored));
    }) as T;
  }
  if (isMockCanvasCommand(command)) return await mockCanvasCommand(command, args) as T;
  if (isMockTrackerCommand(command)) return mockTrackerCommand(command, args ?? {}) as T;
  if (isMockEventTriggersCommand(command)) return mockEventTriggersCommand(command, args ?? {}) as T;
  // Move Hollow home (2026-11-12): fixtures and knobs in mockMoveHome.ts.
  if (isMockMoveHomeCommand(command)) return mockMoveHomeCommand(command, args ?? {}) as T;
  // Settings, Context (CX17): fixture and knobs in mockContextLayers.ts.
  if (isMockContextLayersCommand(command)) return mockContextLayersCommand(command, args ?? {}) as T;
  // Settings, Sources (H4b): fixture and knobs in mockSources.ts.
  if (isMockSourcesCommand(command)) return mockSourcesCommand(command, args ?? {}) as T;

  // Context scan (CX19) and the review card (CX20): fixture and knobs in mockContextReview.ts.
  if (isMockContextReviewCommand(command)) return mockContextReviewCommand(command, args ?? {}) as T;
  // First-run setup (H6) and session import (H5a): fixture and knobs in mockFirstRun.ts.
  if (isMockFirstRunCommand(command)) return mockFirstRunCommand(command, args ?? {}) as T;

  const workFixture = workspaceFixture(command, args, mockWrittenNotes);
  const organized = mockConversationOrganization(command, args, agentSession);
  if (organized) {
    if (command === 'organize_conversation' && (args?.change as { action?: string })?.action === 'settle') writeMockInteractions(mockInteractions().filter(i => i.sessionKey !== (args?.change as { key: string }).key));
    return organized.value as T;
  }
  if (workFixture) return workFixture.value as T;
  const build = buildFixture(command, args);
  if (build) return build.value as T;
  const copies = conflictCopiesFixture(command, args);
  if (copies) return copies.value as T;
  const clashes = syncClashesFixture(command, args);
  if (clashes) return clashes.value as T;
  const reviews = reviewsFixture(command, args, mockWrittenNotes);
  if (reviews) return reviews.value as T;
  switch (command) {
    // Connected projects (next-level P07): src/mockDirectories.ts holds the
    // inventory fixture; the status never carries the file list.
    case 'directories_status': return mockDirectoryStatus(mockDirectoryState(NOW)) as T;
    case 'refresh_directories': return mockDirectoryStatus(mockDirectoryState(NOW), new Date().toISOString()) as T;
    case 'connected_files': return mockConnectedFiles(mockDirectoryState(NOW), (args ?? {}) as any) as T;
    // Repo features R14b: the content index, off until switched on, then done.
    case 'connected_index_status': return mockContentIndexStatus(mockDirectoryState(NOW)) as T;
    case 'set_connected_index_enabled': {
      if ((args as { enabled?: boolean } | undefined)?.enabled) localStorage.setItem('mockContentIndexOn', '1');
      else localStorage.removeItem('mockContentIndexOn');
      return mockContentIndexStatus(mockDirectoryState(NOW)) as T;
    }
    case 'build_identity': return (localStorage.getItem('mockBuildIdentity') ? JSON.parse(localStorage.getItem('mockBuildIdentity')!) : { version: '0.5.3', commit: 'f1x7ure0000000000000000000000000000000000', shortCommit: 'f1x7ure', dirty: false, generatedAt: NOW, sourceFingerprint: 'mock', inputCount: 0, text: 'Hollow 0.5.3 · commit f1x7ure · built fixture · source mock…' }) as T;
    case 'set_directory_exclusion': {
      // Synthetic: record the Hollow-side exclusion; matching files drop out
      // when the inventory is read (mockDirectoryState filters them).
      const stored = JSON.parse(localStorage.getItem('mockDirectories') ?? 'null');
      const { files: _files, ...d } = mockDirectoryState(NOW);
      const rel = String((args as any)?.relative ?? '').replace(/\/$/, ''); const on = Boolean((args as any)?.excluded);
      d.coverage = d.coverage ?? { projects: [], scanLimited: false, maxFiles: 200000, maxProjectFiles: 50000, excluded: [] };
      d.coverage.excluded = on ? [...new Set([...d.coverage.excluded, rel])].sort() : d.coverage.excluded.filter((r: string) => r !== rel);
      localStorage.setItem('mockDirectories', JSON.stringify(stored?.files ? { ...d, files: stored.files } : d));
      return mockDirectoryStatus(mockDirectoryState(NOW)) as T;
    }
    case 'connect_hollow_home': {
      const home=String(args?.homePath??'').replace(/^~(?=\/|$)/,'/Users/you');
      if (!home.startsWith('/')) throw new Error('Use an absolute folder path');
      const projects=String(args?.existingProjects||`${home}/Projects`), vault=String(args?.existingVault||`${home}/Vault`);
      if (args?.existingProjects && home.startsWith(`${projects}/`)) throw new Error(`The Hollow folder can't be inside the folder you're bringing in (${projects}).`);
      if (projects===vault || projects.startsWith(`${vault}/`) || vault.startsWith(`${projects}/`)) throw new Error('The Projects and Vault folders must not overlap');
      const c={version:1,pendingRestart:true,homePath:home,projectsPath:projects,vaultPath:vault,refreshedAt:NOW,projects:[],files:[],warnings:[]};
      localStorage.setItem('mockDirectories',JSON.stringify(c));return c as T;
    }
    case 'list_connectors': {
      const connectors=JSON.parse(localStorage.getItem('mockConnectors')??'[]');
      const discovered=[
        {name:'linear',source:'claude',transport:'http',command:'',url:'https://mcp.linear.app/mcp',iconUrl:'',inLibrary:connectors.some((c:{id:string})=>c.id==='linear'),unsupported:null,note:null},
        {name:'Superhuman Mail',source:'claudeai',transport:'claudeAi',command:'',url:'https://mcp.mail.superhuman.com/mcp',iconUrl:'',inLibrary:connectors.some((c:{id:string})=>c.id==='superhuman_mail'),unsupported:null,note:null},
        {name:'Microsoft 365',source:'claudeai',transport:'claudeAi',command:'',url:'https://microsoft365.mcp.claude.com/mcp',iconUrl:'',inLibrary:connectors.some((c:{id:string})=>c.id==='microsoft_365'),unsupported:null,note:'Sign in to it on claude.ai (Settings → Connectors) first'},
        // A spec may add more tiles to the catalog (the Settings height spec seeds 15).
        ...(JSON.parse(localStorage.getItem('mockDiscovered')??'[]') as Array<Record<string,unknown>>),
      ];
      // Built-in connectors, opt-in so older specs keep their rows (mockBuiltins=1).
      const m365=localStorage.getItem('mockM365');
      const builtin=(id:string,name:string,kind:'m365'|'plaud',ready:boolean,line:string)=>({...{id,name,description:'',transport:'builtin',command:'',args:[],url:'',serverId:'',iconUrl:'',envKeys:[],headerKeys:[],agents:['claude','codex'],chat:true,automations:false,mode:'relevant',autoApprove:true,enabled:true,origin:'hollow',updatedAt:NOW},...connectors.find((c:{id:string})=>c.id===id),builtin:{ready,kind,line}});
      const builtins=localStorage.getItem('mockBuiltins')==='1'?[
        builtin('hollow_m365','Microsoft 365','m365',!!m365,m365?`Signed in to Microsoft as ${m365}. Works on every Claude account and in Codex.`:'Sign in to Microsoft once, here in Hollow.'),
      ]:[];
      // The built-in Plaud (overnight S8) is opt-in too (mockPlaudBuiltin=1), so specs counting rows keep them.
      if(localStorage.getItem('mockPlaudBuiltin')==='1')builtins.push(builtin('hollow_plaud','Plaud','plaud',localStorage.getItem('mockPlaudMcp')==='1',localStorage.getItem('mockPlaudMcp')==='1'?"Runs Plaud's MCP server on its own Plaud sign-in, on every Claude account and in Codex.":"Sign in to Plaud once, here in Hollow. Plaud's MCP server keeps its own sign-in, separate from Recordings'."));
      const own=connectors.filter((c:{id:string})=>!c.id.startsWith('hollow_'));
      return {connectors:[...own,...builtins],discovered,problems:[],claudeaiError:null,claudeaiLogin:{email:'you@example.com',organization:'Example Team'}} as T;
    }
    case 'm365_sign_in': localStorage.setItem('mockM365','you@example.com');localStorage.removeItem('mockCalendarSignIn');return {signedIn:true,account:'you@example.com',files:!args?.calendarOnly,problem:null} as T;
    case 'm365_sign_out': localStorage.removeItem('mockM365');return undefined as T;
    case 'plaud_mcp_sign_in': localStorage.setItem('mockPlaudMcp','1');return {signedIn:true,installed:true,already:false} as T;
    case 'plaud_mcp_sign_out': localStorage.removeItem('mockPlaudMcp');return {signedIn:false,installed:true,already:false} as T;
    // mockCalendarSignIn: Microsoft refused the stored sign-in, which m365_status reports as its problem (m365_auth::status).
    case 'm365_status': {const a=localStorage.getItem('mockM365');return {signedIn:!!a,account:a,files:!!a&&localStorage.getItem('mockM365CalendarOnly')!=='1',problem:a&&localStorage.getItem('mockCalendarSignIn')?'AADSTS50173: the grant has expired':null} as T;}
    case 'save_connector': {
      const input=args?.input as Record<string,any>;const all=JSON.parse(localStorage.getItem('mockConnectors')??'[]');
      if(!String(input.name??'').trim())throw new Error('Give the connector a name (up to 60 characters)');
      const id=input.id||String(input.name).toLowerCase().replace(/[^a-z0-9]+/g,'-').replace(/^-|-$/g,'');
      const view={id,name:input.name,description:input.description??'',transport:input.transport,command:input.command??'',args:input.args??[],url:input.url??'',serverId:input.serverId??'',iconUrl:all.find((c:{id:string})=>c.id===id)?.iconUrl??'',envKeys:Object.keys(input.env??{}),headerKeys:Object.keys(input.headers??{}),agents:input.agents??[],chat:input.chat,automations:input.automations,mode:input.mode,autoApprove:input.autoApprove,enabled:input.enabled,origin:all.find((c:{id:string})=>c.id===id)?.origin??'hollow',updatedAt:NOW,oauth:!!input.oauth,oauthClientId:input.oauthClientId??'',oauthClientSecretSet:!!input.oauthClientId&&(input.oauthClientSecret===undefined?!!all.find((c:{id:string})=>c.id===id)?.oauthClientSecretSet:!!input.oauthClientSecret),keepClaudeai:!!input.keepClaudeai,remote:input.oauth?(all.find((c:{id:string})=>c.id===id)?.remote??{signedIn:false,problem:null}):undefined};
      localStorage.setItem('mockConnectors',JSON.stringify([...all.filter((c:{id:string})=>c.id!==id),view]));return view as T;
    }
    case 'check_connectors': {
      const all=JSON.parse(localStorage.getItem('mockConnectors')??'[]');
      // Stamped like the backend (next-level P16): when, and as which login.
      const at={checkedAt:new Date().toISOString(),login:localStorage.getItem('mockClaudeAiCheckLogin')??'you@example.com in Example Team'};
      return all.filter((c:{transport:string})=>c.transport==='claudeAi').map((c:{id:string})=>c.id==='microsoft_365'
        ?{id:c.id,state:'needsSignIn',detail:'OAuth token has been invalidated. Re-authentication is required.',...at}
        :{id:c.id,state:'connected',detail:null,...at}) as T;
    }
    case 'connector_use_hollow_sign_in': {
      const all=JSON.parse(localStorage.getItem('mockConnectors')??'[]');const src=all.find((c:{id:string})=>c.id===args?.id);
      if(!src||src.transport!=='claudeAi')throw new Error("Only a claude.ai connector can move to Hollow's sign-in");
      const name=String(src.name).replace(/ for Claude$/i,'');const id=name.toLowerCase().replace(/[^a-z0-9]+/g,'_');
      const view={...src,id,name,transport:'http',serverId:'',agents:['claude','codex'],origin:'hollow',oauth:true,oauthClientId:'',remote:{signedIn:false,problem:null}};
      localStorage.setItem('mockConnectors',JSON.stringify([...all,view]));return view as T;
    }
    case 'remote_connector_sign_in': case 'remote_connector_sign_out': {
      const on=command==='remote_connector_sign_in';const all=JSON.parse(localStorage.getItem('mockConnectors')??'[]');
      localStorage.setItem('mockConnectors',JSON.stringify(all.map((c:{id:string})=>c.id===args?.id?{...c,remote:{signedIn:on,problem:null}}:c)));
      return (on?{signedIn:true,problem:null}:undefined) as T;
    }
    case 'delete_connector': localStorage.setItem('mockConnectors',JSON.stringify(JSON.parse(localStorage.getItem('mockConnectors')??'[]').filter((c:{id:string})=>c.id!==args?.id)));return undefined as T;
    case 'import_connector': {
      const all=JSON.parse(localStorage.getItem('mockConnectors')??'[]');
      const claudeai=args?.source==='claudeai';
      const view={id:claudeai?String(args?.name).toLowerCase().replace(/[^a-z0-9]+/g,'_'):String(args?.name),name:String(args?.name),description:'',transport:claudeai?'claudeAi':'http',command:'',args:[],url:claudeai?(args?.name==='Microsoft 365'?'https://microsoft365.mcp.claude.com/mcp':'https://mcp.mail.superhuman.com/mcp'):'https://mcp.linear.app/mcp',serverId:claudeai?'mcpsrv_mock':'',iconUrl:'',envKeys:[],headerKeys:[],agents:claudeai?['claude']:['claude','codex'],chat:true,automations:false,mode:'always',autoApprove:true,enabled:true,origin:String(args?.source),updatedAt:NOW};
      localStorage.setItem('mockConnectors',JSON.stringify([...all,view]));return view as T;
    }
    case 'jev_status': return JSON.parse(localStorage.getItem('mockJev')??'{"enabled":false,"keyPresent":false}') as T;
    case 'save_jev_settings': {
      const prev=JSON.parse(localStorage.getItem('mockJev')??'{"enabled":false,"keyPresent":false}');
      const next={enabled:Boolean(args?.enabled),keyPresent:prev.keyPresent||Boolean(args?.apiKey),decisionsEnabled:args?.decisionsEnabled === undefined ? Boolean(prev.decisionsEnabled) : Boolean(args.decisionsEnabled)};
      if((next.enabled||next.decisionsEnabled)&&!next.keyPresent)throw new Error('Add a Jev API key before switching it on.');
      localStorage.setItem('mockJev',JSON.stringify(next));return next as T;
    }
    case 'remove_jev_key': localStorage.setItem('mockJev','{"enabled":false,"keyPresent":false}');return {enabled:false,keyPresent:false,decisionsEnabled:false} as T;
    case 'test_jev_key': return undefined as T;
    case 'jev_decisions_report': { const r = localStorage.getItem('mockJevDecisionsReport'); return (r ? JSON.parse(r) : null) as T; }
    case 'run_context_manifest': return mockRunContextManifest(NOW, args?.runId as string | null | undefined, args?.sessionKey as string | null | undefined) as T;
    case 'read_connected_file': return mockReadConnectedFile(mockDirectoryState(NOW), String(args?.path ?? '')) as T;
    case 'setup_status': return {
      ...setupStatus,
      configured: localStorage.getItem('mockSetupConfigured') !== 'false',
      ...JSON.parse(localStorage.getItem('mockSetupIdentity') ?? '{}'),
    } as T;
    case 'create_profile_notes':
      localStorage.setItem('mockCreateProfileNotes', '1');
      return { created: 5, skipped: 0, errors: [] } as T;
    case 'setup_restart': {
      localStorage.setItem('mockRestartConfirmedKeys', JSON.stringify(args?.confirmedKeys ?? null));
      const late = localStorage.getItem('mockRestartNewWork');
      if (late && args?.confirmedKeys) {
        localStorage.removeItem('mockRestartNewWork');
        localStorage.setItem('mockExitRunningWork', late);
        throw 'RESTART_WORK_CHANGED: Running work changed; confirm again';
      }
      localStorage.setItem('mockRestartCompleted', '1');
      return undefined as T;
    }
    case 'update_get_state': return mockUpdateState() as T;
    case 'update_get_settings': return mockUpdateSettings() as T;
    case 'update_set_channel':
      if (args?.channel !== mockUpdateSettings().channel) saveMockUpdateSettings({ channel: args?.channel as UpdateSettings['channel'], allowDowngradeOnce: true, allowDowngradeSinceMs: Date.now(), dismissedVersion: null });
      return mockUpdateState() as T;
    case 'update_set_overnight': return saveMockUpdateSettings({ installOvernight: Boolean(args?.enabled) }) as T;
    case 'update_check_now':
      saveMockUpdateSettings({ lastCheckedAt: NOW });
      return mockUpdateState() as T;
    case 'update_connect':
      if (!/^[A-Za-z0-9_-]{43}$/.test(String(args?.token ?? '').trim())) throw "That isn't an updates token.";
      localStorage.setItem('mockUpdateConnected', '1');
      return { connected: true } as T;
    case 'update_confirm_connect':
      if (localStorage.getItem('mockUpdatePendingConnect') !== '1') throw 'No new updates token is waiting.';
      localStorage.removeItem('mockUpdatePendingConnect');
      localStorage.setItem('mockUpdateConnected', '1');
      return mockUpdateState() as T;
    case 'update_discard_connect':
      localStorage.removeItem('mockUpdatePendingConnect');
      return mockUpdateState() as T;
    case 'update_disconnect':
      localStorage.setItem('mockUpdateConnected', '0');
      return { connected: false } as T;
    case 'update_dismiss':
      saveMockUpdateSettings({ dismissedVersion: String(args?.version ?? '') });
      return mockUpdateState() as T;
    case 'update_install_now': {
      const phase = mockUpdateState().phase;
      if (phase.phase !== 'ready') throw 'No verified update is ready to install.';
      const work = JSON.parse(localStorage.getItem('mockExitRunningWork') ?? '[]') as Array<string | { key: string; name: string }>;
      const running = work.map(item => typeof item === 'string' ? { key: item, name: item } : item);
      const confirmed = args?.confirmedKeys as string[] | null | undefined;
      if (running.length && !confirmed) return { outcome: 'workRunning', work: running } as T;
      if (confirmed && running.some(w => !confirmed.includes(w.key))) return { outcome: 'workChanged', work: running } as T;
      localStorage.setItem('mockUpdateInstalled', phase.release.version);
      return { outcome: 'started', version: phase.release.version } as T;
    }
    case 'update_install_when_idle':
      if (mockUpdateState().phase.phase !== 'ready') throw 'No verified update is ready to install.';
      localStorage.setItem('mockUpdateWaiting', '1');
      return mockUpdateState() as T;
    case 'update_cancel_wait':
      localStorage.removeItem('mockUpdateWaiting');
      localStorage.removeItem('mockUpdateGoBackWaiting');
      return mockUpdateState() as T;
    case 'update_last_result': return JSON.parse(localStorage.getItem('mockUpdateLastResult') ?? 'null') as T;
    case 'update_ack_last_result':
      localStorage.removeItem('mockUpdateLastResult');
      return undefined as T;
    case 'update_publish_status': return mockPublishStatus((args?.pick as string | null) ?? null) as T;
    case 'update_publish_main': {
      const status = mockPublishStatus(null);
      const version = String(args?.version ?? '');
      const refuse = (code: string, reason: string): UpdatePublishOutcome => ({ outcome: 'refused', code, reason });
      if (!status.publisher) return refuse('not-publisher', status.unavailableReason ?? '') as T;
      if ((args?.expectedMain ?? null) !== (status.main?.version ?? null)) return refuse('moved', `Main is ${status.main?.version ?? 'empty'} now`) as T;
      if (status.main?.version === version) return refuse('unchanged', `Main already serves ${version}`) as T;
      const build = status.builds.find(b => b.version === version);
      if (!build) return refuse('no-record', `${version} isn't a kept build`) as T;
      if (args?.commit !== build.commit) return refuse('commit', `${version} is another commit`) as T;
      const history = status.history.map(h => h.version);
      localStorage.setItem('mockPublishHistory', JSON.stringify([...history, version]));
      return { outcome: 'published', version, from: status.main?.version ?? null, at: NOW, historyWarning: null } as T;
    }
    case 'update_withdraw_main': {
      const status = mockPublishStatus(null);
      if (!status.publisher) return { outcome: 'refused', code: 'not-publisher', reason: status.unavailableReason ?? '' } as T;
      if (args?.expected && args.expected !== status.main?.version) return { outcome: 'refused', code: 'moved', reason: `Main is ${status.main?.version} now` } as T;
      if (!status.canWithdraw || !status.main || !status.previousMain) return { outcome: 'refused', code: 'no-previous', reason: 'There is no earlier Main to go back to' } as T;
      localStorage.setItem('mockPublishHistory', JSON.stringify(status.history.map(h => h.version).slice(0, -1)));
      return { outcome: 'withdrawn', version: status.previousMain, from: status.main.version, at: NOW, historyWarning: null } as T;
    }
    case 'update_set_publisher': return saveMockUpdateSettings({ publisher: Boolean(args?.enabled) }) as T;
    case 'update_offer_local_build': {
      // Row U10: the workspace's finished build becomes the Local build.
      const build = JSON.parse(localStorage.getItem(`mockAppBuild:${String(args?.id ?? '')}`) ?? 'null') as { state?: string; branch?: string; commit?: string; receipt?: { buildId: string } | null } | null;
      if (build?.state !== 'built') throw 'Build the app first';
      const log = JSON.parse(localStorage.getItem('mockBuildLog') ?? '[]'); log.push({ command, args }); localStorage.setItem('mockBuildLog', JSON.stringify(log));
      const release = { version: '0.6.100', notes: 'Brings 1 pull request since the installed build.\n#771 Name the installed release', pubDate: null, channel: mockUpdateSettings().channel, commit: build.commit ?? null, downgrade: false,
        local: { workspaceId: String(args?.id), branch: build.branch ?? 'hollow/work', buildId: build.receipt?.buildId ?? 'build', dirty: false } };
      localStorage.setItem('mockUpdateState', JSON.stringify({ phase: { phase: 'ready', release } }));
      saveMockUpdateSettings({ dismissedLocalBuild: null });
      return mockUpdateState() as T;
    }
    case 'exit_running_work': return (JSON.parse(localStorage.getItem('mockExitRunningWork') ?? '[]') as Array<string | {key: string; name: string}>).map(item => typeof item === 'string' ? {key: item, name: item} : item) as T;
    case 'pending_quit_request': return JSON.parse(localStorage.getItem('mockPendingQuit') ?? 'null') as T;
    case 'resolve_quit': {
      if (args?.choice === 'quit' && localStorage.getItem('mockQuitNewWork')) {
        const next = { id: 'quit-new', names: ['Polish lane B', 'New automation'] };
        localStorage.removeItem('mockQuitNewWork');
        localStorage.setItem('mockPendingQuit', JSON.stringify(next));
        return next as T;
      }
      localStorage.setItem('mockQuitChoice', String(args?.choice));
      localStorage.removeItem('mockPendingQuit');
      return null as T;
    }
    case 'diagnostics': return diagnostics as T;
    case 'dashboard': return dashboard as T;
    // Agenda (next-level P10): fixture events with an all-day offsite, an
    // overlap, a cancellation and a moved recurring occurrence, relative to
    // today. (Focus blocks have no frontend reader since the 11-10 sweep.)
    case 'calendar_source_status':
    case 'set_calendar_source': {
      if (command === 'set_calendar_source') localStorage.setItem('mockAgendaSource', JSON.stringify(args?.settings));
      if (command === 'set_calendar_source' && args?.requestAccess) localStorage.setItem('mockCalendarAccessRequests', String(Number(localStorage.getItem('mockCalendarAccessRequests') ?? 0) + 1));
      const settings = JSON.parse(localStorage.getItem('mockAgendaSource') ?? '{"source":"microsoft","calendarIds":null}');
      return { settings, permission: settings.source === 'mac' ? (localStorage.getItem('mockMacCalendarPermission') ?? 'granted') : 'notRequested',
        calendars: settings.source === 'mac' ? [{ id: 'work', title: 'Work', account: 'Example account' }, { id: 'personal', title: 'Personal', account: 'On My Mac' }] : [], error: null } as T;
    }
    case 'open_calendar_settings': localStorage.setItem('mockCalendarSettingsOpened', 'true'); return null as T;
    case 'agenda_events': {
      const settings = JSON.parse(localStorage.getItem('mockAgendaSource') ?? '{"source":"microsoft","calendarIds":null}');
      if (settings.source === 'off') return { connected: false, provider: 'off', status: 'Calendar is off', fetchedAt: new Date().toISOString(), events: [], complete: true, error: null, source: 'Off' } as T;
      if (settings.source === 'mac' && localStorage.getItem('mockMacCalendarPermission') === 'denied') return { connected: false, provider: 'mac', permission: 'denied', status: 'Calendar access is denied', fetchedAt: new Date().toISOString(), events: [], complete: false, error: null, source: 'Calendars on this Mac' } as T;
      if (localStorage.getItem('mockCalendarOff')) return { connected: false, fetchedAt: new Date().toISOString(), events: [], error: null, source: 'Microsoft 365 (not signed in)' } as T;
      // localStorage mockCalendarSignIn: Microsoft refused the stored sign-in (agenda::events_with's stored problem; overnight H6).
      if (localStorage.getItem('mockCalendarSignIn')) return { connected: true, fetchedAt: new Date().toISOString(), events: [], complete: false, partial: null, error: 'Microsoft 365 needs signing in again in Hollow → Settings → Connectors (AADSTS50173)', source: 'Microsoft 365' } as T;
      const at = (dayOffset: number, h: number, m = 0) => { const d = new Date(); d.setDate(d.getDate() + dayOffset); d.setHours(h, m, 0, 0); return d.toISOString(); };
      const allDay = (dayOffset: number, days: number) => { const d = new Date(); d.setDate(d.getDate() + dayOffset); d.setHours(0, 0, 0, 0); const e = new Date(d); e.setDate(e.getDate() + days); return [d.toISOString(), e.toISOString()]; };
      const [os, oe] = allDay(2, 2);
      const events = [
        { id: 'ev-standup', title: 'Ops standup', start: at(0, 9), end: at(0, 9, 15), allDay: false, cancelled: false, location: 'Teams', showAs: 'busy', organizer: 'Dana Kim' },
        { id: 'ev-review', title: 'Q3 review prep', start: at(0, 11), end: at(0, 12), allDay: false, cancelled: false, location: 'Room 4B', showAs: 'busy', organizer: 'You' },
        { id: 'ev-vendor', title: 'Vendor security call', start: at(0, 11, 30), end: at(0, 12, 30), allDay: false, cancelled: false, showAs: 'tentative', organizer: 'Vendor' },
        { id: 'ev-cancelled', title: 'Garage 14 walkthrough', start: at(0, 14), end: at(0, 15), allDay: false, cancelled: true, showAs: 'free', organizer: 'Facilities' },
        { id: 'ev-standup-moved', title: 'Ops standup (moved)', start: at(1, 15), end: at(1, 15, 15), allDay: false, cancelled: false, showAs: 'busy', organizer: 'Dana Kim' },
        { id: 'ev-offsite', title: 'Leadership offsite', start: os, end: oe, allDay: true, cancelled: false, showAs: 'oof', organizer: 'CEO' },
      ];
      // localStorage mockCalendarPartial: the read stopped early (merged plan slice 5).
      const partial = localStorage.getItem('mockCalendarPartial') ? 'page 2 failed: Microsoft Graph 503 Service Unavailable' : null;
      return { connected: true, fetchedAt: new Date(Date.now() - 120_000).toISOString(), timeZone: 'America/New_York', events: settings.source === 'mac' && settings.calendarIds?.length === 0 ? [] : events, complete: !partial, partial, error: null, source: settings.source === 'mac' ? 'Calendars on this Mac · read only' : 'Microsoft 365 · read only' } as T;
    }
    case 'ingest_run': {
      const src = args?.source as { kind: string; url?: string } | undefined;
      if (localStorage.getItem('mockIngestFail')) throw 'Couldn’t fetch that page (offline).';
      const log = JSON.parse(localStorage.getItem('mockIngestLog') ?? '[]'); log.push(src); localStorage.setItem('mockIngestLog', JSON.stringify(log));
      return { targetDir: 'sources/web', createdCount: 1, summary: `Saved ${src?.url ?? 'source'}` } as T;
    }
    case 'tasks': return { items: (window as unknown as { __mockDemoTasks?: Task[] }).__mockDemoTasks ?? (localStorage.getItem('mockTasks') ? JSON.parse(localStorage.getItem('mockTasks')!) as Task[] : tasks), fetched: NOW, source: 'mock', todoPath: null, lastSyncedAt: null, syncError: null } as T;
    case 'projects': return { items: [project], fetched: NOW, source: 'mock', lastSyncedAt: null, syncError: null } as T;
    case 'list_goals': return [goal] as T;
    case 'goal_alignment': return {
      activeGoalCount: 1,
      activeProjectCount: 1,
      healthyGoalCount: 0,
      healthyProjectCount: 0,
      activeGoalsWithoutActiveProjects: [{ goalUrl: goal.url, goalTitle: goal.title }],
      activeProjectsWithoutGoals: [{ projectUrl: project.url, projectName: project.name }],
      brokenGoalUrls: [],
      projectsOnlyLinkedToInactiveGoals: [],
    } as T;
    case 'weekly_review': {
      // The weekly review's demo (overnight 10-11 A7).
      const thisWeek = isoWeek(new Date());
      const week = String(args?.week ?? thisWeek);
      const [year, number] = week.split('-W').map(Number);
      const mon = new Date(year, 0, 4, 12);
      mon.setDate(mon.getDate() - (mon.getDay() + 6) % 7 + (number - 1) * 7);
      const iso = (x: Date) => `${x.getFullYear()}-${String(x.getMonth() + 1).padStart(2, '0')}-${String(x.getDate()).padStart(2, '0')}`;
      const at = (n: number) => { const x = new Date(mon); x.setDate(mon.getDate() + n); return iso(x); };
      const line = (text: string, path: string | null = null, sessionKey: string | null = null) => ({ text, path, sessionKey, day: null });
      return {
        thisWeek,
        take: { week, paragraph: 'A Atlas week: the Q3 pricing sheet and the board pack moved, and the overnight lanes merged eight features. The vendor addendum slipped and two commitments to Pat are overdue.', generatedAt: NOW,
          priorities: [
            { title: 'Finish the Q3 board pack', why: 'Due Monday; Pat is waiting on the numbers', ask: 'Help me finish the Q3 board pack for Pat' },
            { title: 'Send Raj the signed addendum', why: 'Slipped twice this week', ask: 'Draft the cover note for the signed vendor addendum to Raj' },
            { title: 'Review the overnight PRs by hand', why: 'Eight merged, none checked natively', ask: 'List what I need to check by hand from last night' },
          ] },
        review: {
          week, monday: at(0), sunday: at(6),
          stats: { accepted: 4, tasksDone: 9, prsMerged: 8, decisions: 2, spendUsd: 42.18 },
          shipped: [line('Accepted “Q3 pricing sheet v2” in Atlas', 'workspaces/atlas/outputs/q3-pricing.md'), line('Done: Call Pat about the floor price', 'tasks/call-pat.md'), line('Merged example-org/hollow-code #526 (Commitments ledger)', null, 's-a'), line('Merged example-org/hollow-code #528 (Ask your vault)', null, 's-a')],
          inProgress: [line('Atlas: 6 runs in 3 conversations', null, 's-b')],
          slipped: [line('Send the vendor addendum (due Wed)', 'tasks/addendum.md')],
          decisions: [line('Q3 floor price is $14 a unit (Pat Smith)', 'decisions/q3-floor.md'), line('Overnight lanes run two toolchain slots', 'decisions/lanes.md')],
          kept: [line('You → Pat Smith: HRIS audit notes', 'commitments/hris-audit-notes.md')],
          made: [line('You → Ana Ruiz: Atlas pricing sheet v2', 'commitments/atlas-pricing.md')],
          overdue: [line('You → Pat Smith: Q3 numbers for the board pack (due Thu)', 'commitments/q3-numbers-pat.md')],
          nextWeek: [line('Due Mon: Board pack', 'tasks/board-pack.md'), line('Due Wed: TipFlow push file reconcile', 'tasks/tipflow.md')],
          numbers: ['14 conversations active, 37 runs (2 failed)', 'Hours saved (estimate): 6.5 h from 4 accepted runs'],
          spendByDay: [3.2, 8.4, 6.1, 9.9, 7.3, 4.0, 3.28].map((usd, n) => ({ day: at(n), usd })),
          spendByWorkspace: [{ workspace: 'hollow', title: 'hollow', usd: 24.1 }, { workspace: 'atlas', title: 'Atlas', usd: 11.6 }, { workspace: 'tipflow', title: 'TipFlow', usd: 6.48 }],
          unpricedRuns: 0,
        },
      } as T;
    }
    case 'list_commitments': return mockCommitments.map(c => ({ ...c })) as T;
    case 'list_private_memories': return JSON.parse(localStorage.getItem('mockPrivateMemories') ?? '[]') as T;
    case 'list_skill_lessons': return JSON.parse(localStorage.getItem('mockSkillLessons') ?? '[]') as T;
    case 'resolve_skill_lesson': {
      const items=JSON.parse(localStorage.getItem('mockSkillLessons') ?? '[]') as {id:string;status:string}[];
      const item=items.find(i=>i.id===args?.id); if(!item || item.status!=='pending') throw new Error('Skill proposal is no longer pending.');
      if(localStorage.getItem('mockSkillLessonChanged')==='1') throw new Error('Skill changed. Read it fresh and propose again.');
      item.status=args?.action==='accept'?'accepted':'rejected';localStorage.setItem('mockSkillLessons',JSON.stringify(items));return item as T;
    }
    case 'undo_skill_lesson': {
      const items=JSON.parse(localStorage.getItem('mockSkillLessons') ?? '[]') as {id:string;status:string}[];
      const item=items.find(i=>i.id===args?.id); if(!item || item.status!=='accepted') throw new Error('Only a saved skill lesson can be undone.');
      item.status='pending';localStorage.setItem('mockSkillLessons',JSON.stringify(items));return item as T;
    }
    case 'promote_private_memory': {
      const sources = JSON.parse(localStorage.getItem('mockPrivateMemories') ?? '[]') as {id:string;text:string;expectedHash:string}[];
      const source = sources.find(s => s.id === args?.id);
      if (!source || source.expectedHash !== args?.expectedHash) throw new Error('Private memory changed. Refresh before promoting.');
      const statement = String(args?.statement ?? '').trim();
      if (statement.length < 12 || !source.text.includes(statement)) throw new Error('Choose an exact passage from the private memory.');
      const scope = args?.scope;
      const targetPath = scope === 'you' ? 'me/Decisions.md' : scope === 'project' ? `workspaces/${args?.workspaceId}/WORKSPACE.md` : 'learnings/';
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const old = items.find(i => i.kind === 'learning' && i.payload.statement === statement && i.payload.targetPath === targetPath);
      if (old) return old as T;
      const item = {...learningProposal,kind:'learning',risk:'local_write',source:'private-memory',id:`review-private-${Date.now()}`,status:'pending',createdAt:new Date().toISOString(),updatedAt:new Date().toISOString(),payload:{...learningProposal.payload,kind:'private-memory-promotion',subject:'Promote private memory',statement,evidenceExcerpt:statement,targetKind:scope === 'you' ? 'operator' : scope === 'project' ? 'workspace' : 'knowhow',targetPath,learnedPath:null,batchId:null}};
      items.push(item as unknown as ReviewInboxItem); localStorage.setItem('mockReviewItems',JSON.stringify(items)); return item as T;
    }
    case 'list_learning_proposals': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      return items.filter(item => item.kind === 'learning').map(item => ({...item,version:item.version ?? `v-mock-${item.updatedAt}`,payload:{...learningProposal.payload,...item.payload}})) as T;
    }
    // Bulk learning review: the same transitions as the backend, on the
    // mock review items (each id on its own; Undo clears the batch, so a
    // second Undo fails).
    case 'resolve_learning_proposals_batch': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const action = args?.action as string;
      if (action !== 'accept' && action !== 'reject') throw `unknown learning resolution: ${action}`;
      const reason = typeof args?.reason === 'string' && args.reason.trim() ? args.reason.trim() : undefined;
      if (reason && action === 'accept') throw 'a reason goes with Reject, not Accept';
      if (reason && reason.length > 500) throw 'the reason is too long (max 500 characters)';
      const ids = [...new Set((args?.ids as string[] | undefined) ?? [])];
      if (!ids.length) throw 'no learning proposals selected';
      // A row's Save with the person's edited words: one Accept only.
      const edited = typeof args?.editedStatement === 'string' ? args.editedStatement.trim() : undefined;
      if (edited !== undefined && (action !== 'accept' || ids.length !== 1)) throw 'edited words go with the Save of one learning';
      const batchId = `learning-batch-mock-${Date.now().toString(36)}`;
      const results = ids.map(id => {
        const item = items.find(candidate => candidate.id === id && candidate.kind === 'learning');
        if (!item) return { id, ok: false, status: null, error: `learning proposal not found: ${id}` };
        // Blank edited words fail like the backend's validation, unchanged.
        if (edited === '') return { id, ok: false, status: null, error: 'learning subject and statement are required' };
        if (item.status !== 'pending') return { id, ok: false, status: null, error: `review item ${id} is ${item.status}` };
        item.status = action === 'accept' ? 'accepted' : 'rejected';
        item.payload = { ...item.payload, ...(edited !== undefined ? { statement: edited } : {}), batchId, ...(reason ? { rejectReason: reason } : {}) };
        return { id, ok: true, status: item.status, error: null };
      });
      localStorage.setItem('mockReviewItems', JSON.stringify(items));
      return { batchId: results.some(r => r.ok) ? batchId : null, action, items: results } as T;
    }
    case 'undo_learning_batch': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const batchId = args?.batchId as string;
      const members = items.filter(item => item.kind === 'learning' && item.payload.batchId === batchId);
      if (!members.length) throw `nothing to undo: bulk review ${batchId} was already undone`;
      for (const item of members) {
        const rest = { ...item.payload };
        for (const key of ['batchId', 'rejectReason', 'learnedPath']) delete rest[key];
        item.payload = rest;
        item.status = 'pending';
      }
      localStorage.setItem('mockReviewItems', JSON.stringify(items));
      return { batchId, action: 'undo', items: members.map(item => ({ id: item.id, ok: true, status: 'pending', error: null })) } as T;
    }
    case 'list_learning_context': return (args?.includeSuperseded ? [learnedContext, outdatedContext] : [learnedContext]) as T;
    case 'memory_maintenance': return (JSON.parse(localStorage.getItem('mockMemoryMaintenance') ?? 'null') ?? {
      entries: [learnedContext,outdatedContext].map((c,i) => ({key:c.path,kind:'note',line:null,ages:true,proposalId:null,path:c.path,status:i ? 'archived' : 'active',pinned:false,uses:i ? 0 : 3,lastUsed:null,expiresOn:null,expectedHash:`mock-${i}`})),
      coreBytes:1200,coreBudgetBytes:16000,consolidationProposal:null,
    }) as T;
    case 'edit_memory_maintenance': {
      const view = await mockInvoke<import('./types').MemoryMaintenance>('memory_maintenance', {});
      const entry = view.entries.find(e => e.key === args?.key);
      if(!entry || entry.expectedHash !== args?.expectedHash) throw new Error('This learning changed. Refresh and review it again; nothing was changed.');
      if(args?.action === 'pin') {entry.pinned=true;entry.status='active';}
      else if(args?.action === 'unpin') entry.pinned=false;
      else if(args?.action === 'restore') {entry.status='active';entry.expiresOn=null;}
      else if(args?.action === 'expiry') entry.expiresOn=(args.date as string | null) ?? null;
      entry.expectedHash += '-next';
      localStorage.setItem('mockMemoryMaintenance',JSON.stringify(view));return view as T;
    }
    case 'list_review_inbox': {
      // Filtered by status and kind as the backend does (Decisions' Snoozed list asks for 'snoozed', P1-13).
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      return items.filter(i => (!args?.status || i.status === args.status) && (!args?.kind || i.kind === args.kind)) as T;
    }
    case 'resolve_review_item': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const item = items.find(candidate => candidate.id === args?.id);
      if (!item || !isWorkspaceReviewItem(item)) throw new Error('Unsupported review item');
      // A budget pause re-checks the new limit like the backend, and refuses
      // with a STRING, as Tauri rejects (never an Error object).
      if (item.kind === 'budget_pause' && args?.action === 'accept') {
        const newLimit = Number((args?.editedPayload as Record<string, unknown> | undefined)?.newLimit);
        const observed = Number(item.payload.observed);
        if (!Number.isFinite(newLimit) || newLimit <= observed) {
          throw `New limit must be above this day's usage (${observed} tokens).`;
        }
        localStorage.setItem('mockBudgetPaused', '[]');
      }
      item.status = args?.action === 'accept' ? 'accepted' : 'rejected';
      item.updatedAt = new Date().toISOString();
      if (args?.editedPayload) item.payload = args.editedPayload as Record<string, unknown>;
      localStorage.setItem('mockReviewItems', JSON.stringify(items));
      return item as T;
    }
    // P0-10: the Undo after one Reject, as the backend: only a rejection, once.
    case 'undo_review_reject': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const item = items.find(candidate => candidate.id === args?.id);
      if (!item) throw `review item not found: ${String(args?.id)}`;
      if (item.status !== 'rejected') throw `review item ${item.id} is ${item.status}, not a rejection to undo`;
      if (item.kind === 'budget_pause' || item.kind === 'companion_old_op') throw `a ${item.kind} rejection can't be undone here`;
      if (item.updatedAt !== args?.expectedUpdatedAt) throw `review item ${item.id} changed since it was rejected`;
      item.status = 'pending';
      item.updatedAt = new Date().toISOString();
      localStorage.setItem('mockReviewItems', JSON.stringify(items));
      return item as T;
    }
    // P1-13: snooze until a time, and Wake now / a snooze's Undo, as the backend.
    case 'snooze_review_item':
    case 'wake_review_item': {
      const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? DEFAULT_REVIEW_ITEMS) as ReviewInboxItem[];
      const item = items.find(candidate => candidate.id === args?.id);
      if (!item) throw `review item not found: ${String(args?.id)}`;
      const waking = command === 'wake_review_item';
      if (item.status !== (waking ? 'snoozed' : 'pending')) throw `review item ${item.id} is ${item.status}`;
      item.status = waking ? 'pending' : 'snoozed';
      item.snoozedUntil = waking ? null : String(args?.until);
      item.updatedAt = new Date().toISOString();
      localStorage.setItem('mockReviewItems', JSON.stringify(items));
      return item as T;
    }
    case 'activation_inventory': return activationInventory as T;
    case 'list_artifact_sessions': {
      const path = String(args?.path ?? '');
      if (localStorage.getItem('mockArtifactSessionsError')) throw new Error('Session store unavailable');
      if (localStorage.getItem('mockArtifactSessionsEmpty')) return [] as T;
      return [{
        session: { ...agentSession, harnessSessionId: 'codex-artifact-thread', resumable: 'native' },
        run: { ...historyRun, id: 'run-artifact-1', source: 'chat', sourceId: 'codex-artifact-thread', status: 'success',
          changes: [{ ...historyRun.changes[1], vaultPath: path }] },
        attribution: 'observed',
      }] as T;
    }
    case 'read_artifact_session_context': return {
      vaultPath: '/Users/fixture/vault',
      detail: {
        summary: { ...agentSession, harnessSessionId: 'codex-artifact-thread', resumable: 'native' },
        turns: [{ ord: 1, runId: 'run-artifact-1', startedAt: NOW, endedAt: NOW,
          status: 'success', asked: 'Clarify the project outcome and next action.' }],
        eventsRetained: true,
      },
      answers: { '1': 'Updated the project outcome. Next: verify the daily cockpit.' },
      receipts: [],
    } as T;
    case 'list_agent_sessions': return (Boolean(args?.archived) === (localStorage.getItem('mockArchivedSession') === 'true') ? [agentSession] : []) as T;
    case 'archive_agent_session': {
      const org = JSON.parse(localStorage.getItem('mockConversationOrganization') ?? '{}') as Record<string, { pinned: boolean; snoozedUntil: string | null; settled: boolean; reason: string | null; revision: number }>;
      const row = org[String(args?.key)];
      if (args?.archived && row?.pinned) throw 'Unpin this conversation first';
      if (row) { row.snoozedUntil = null; row.reason = null; row.settled = false; row.revision++; localStorage.setItem('mockConversationOrganization', JSON.stringify(org)); }
      localStorage.setItem('mockArchivedSession', String(args?.archived));
      // Archiving expires the conversation's open question cards, as the
      // backend does in the same transaction (Paperclip PR 13).
      if (args?.archived) writeMockInteractions(mockInteractions().filter(i => i.sessionKey !== args?.key));
      return undefined as T;
    }
    // Question cards (Paperclip PR 13), seeded through localStorage
    // `mockPendingInteractions` (an array of ChatInteraction). Refusals throw
    // strings, as Tauri does. `mockReopenClosed` makes reopen refuse.
    case 'list_pending_interactions': {
      // `chain`: a current conversation also holds its chain's earlier
      // chats' open cards (one Main conversation, 2026-11-07).
      const key = String(args?.sessionKey ?? '');
      const chain = args?.chain ? mockChain(key) : null;
      const keys = new Set([key, ...(chain && chain.current === key ? chain.earlier.map(e => e.key) : [])]);
      return mockInteractions().filter(i => keys.has(i.sessionKey) && (i.status === 'pending' || i.status === 'stale')) as T;
    }
    // A conversation's chain (one Main conversation, 2026-11-07), seeded
    // through localStorage `mockChains`: { [key]: ChainView }. Unseeded, a
    // conversation is its own current one with nothing earlier.
    case 'conversation_chain':
      return mockChain(String(args?.key ?? '')) as T;
    // A conversation's subagents (L2), seeded through localStorage
    // `mockSubagents`: { [parent]: SubagentsView }. Unseeded, none.
    case 'session_subagents':
      return mockSubagents(String(args?.parent ?? ''), Number(args?.older ?? 0)) as T;
    case 'resolve_interaction': {
      const all = mockInteractions();
      const card = all.find(i => i.id === args?.id);
      if (!card || card.status !== 'pending') throw 'INTERACTION_CLOSED: this question card is not open';
      // Another native session (or none) answers as an ordinary message, each
      // question in full, as the backend does since 2026-11-04.
      const resumes = !!card.nativeSessionId && card.nativeSessionId === args?.nativeSessionId;
      localStorage.setItem('mockLastResolveArgs', JSON.stringify(args));
      const answered = { ...card, status: 'answered' as const, answer: (args?.answer ?? null) as ChatInteraction['answer'] };
      writeMockInteractions(all.map(i => (i.id === card.id ? answered : i)));
      const result: ResolvedInteraction = { interaction: answered, prompt: mockAnswerPrompt(card, args?.answer as InteractionAnswer | undefined), resumes };
      return result as T;
    }
    case 'dismiss_interaction': {
      const all = mockInteractions();
      const card = all.find(i => i.id === args?.id);
      if (!card || (card.status !== 'pending' && card.status !== 'stale')) throw 'INTERACTION_CLOSED: this question card is not open';
      const dismissed = { ...card, status: 'dismissed' as const };
      writeMockInteractions(all.map(i => (i.id === card.id ? dismissed : i)));
      return dismissed as T;
    }
    case 'reopen_interaction': {
      if (localStorage.getItem('mockReopenClosed')) throw 'INTERACTION_CLOSED: a turn already started after this answer';
      const all = mockInteractions();
      const card = all.find(i => i.id === args?.id);
      if (!card || card.status !== 'answered') throw 'INTERACTION_CLOSED: this question card is not answered';
      const reopened = { ...card, status: 'pending' as const, answer: null };
      writeMockInteractions(all.map(i => (i.id === card.id ? reopened : i)));
      return reopened as T;
    }
    case 'set_disk_threshold': {
      const gb = Number(args?.thresholdGb);
      if (!Number.isInteger(gb) || gb < 1 || gb > 10_000) throw new Error(`Warn under 1 to 10,000 GB, not ${String(args?.thresholdGb)}. Nothing changed.`);
      localStorage.setItem('mockDiskThreshold', String(gb));
      return gb as T;
    }
    case 'disk_threshold': return Number(localStorage.getItem('mockDiskThreshold') ?? 50) as T;
    case 'automation_windows': return mockAutomationWindows() as T;
    case 'set_automation_windows': return mockSetAutomationWindows(args) as T;
    // Disk space and its two cleanups (overnight H20): mockDiskCleanup.ts.
    case 'disk_guard': return mockDiskGuard() as T;
    case 'agent_tool_path': return mockAgentToolPath() as T;
    case 'install_targets': return mockInstallTargets() as T;
    case 'prune_install_targets': return mockPruneInstallTargets(args) as T;
    case 'worktree_hygiene': return mockWorktreeHygiene() as T;
    case 'clean_worktrees': return mockCleanWorktrees(args) as T;
    // Repo features R7b: Hollow's databases and Remove old indexes….
    case 'hollow_databases': return mockHollowDatabases(args) as T;
    case 'stale_indexes': return mockStaleIndexes() as T;
    case 'remove_stale_indexes': return mockRemoveStaleIndexes(args) as T;
    case 'overnight_summary': {
      // Quiet by default so Home's smoke shots don't move; localStorage
      // `mockOvernight=1` shows a sample night.
      const sample = localStorage.getItem('mockOvernight') === '1';
      return {
        night: '2026-10-09', from: '2026-10-10T00:00:00.000Z', to: '2026-10-10T12:00:00.000Z',
        ran: sample, runs: sample ? 12 : 0, conversations: sample ? 2 : 0, succeeded: sample ? 10 : 0,
        failed: sample ? [{ status: 'error', resultKind: 'error', failedChecks: 0, at: '2026-10-10T06:00:00Z', sessionKey: 'mock-lane-b', sessionTitle: 'Overnight lane B', source: 'chat' }] : [],
        pullRequests: sample ? [{ repo: 'example-org/hollow-code', number: 451, action: 'merged', at: '2026-10-10T06:37:00Z', sessionKey: 'mock-lane-b', sessionTitle: 'Overnight lane B' }] : [],
        commits: [], waiting: { total: sample ? 2 : 0, titles: [] }, report: null,
      } as T;
    }
    case 'result_export_markdown': {
      return { target: String(args?.target ?? '/mock/exports/Result.md'), sources: 2 } as T;
    }
    case 'settings_export': {
      return { path: String(args?.path ?? '/mock/exports/hollow-settings.json'), count: 42, skipped: [] } as T;
    }
    case 'settings_import_preview': {
      return {
        changes: [{ key: 'theme', summary: 'The colour theme.', old: 'light', new: 'dark', asks: false }],
        unchanged: 40,
        ignored: [{ key: 'jev.apiKey', why: 'a secret: never imported from a file' }],
        exportedAt: '2026-10-10T10:00:00Z',
        hash: 'mock-settings-hash',
      } as T;
    }
    case 'settings_import_apply': {
      return { applied: args?.expectedHash === 'mock-settings-hash' ? 1 : 0, failed: [] } as T;
    }
    case 'save_diagnostics': {
      const dir = String(args?.dir ?? '/mock/exports');
      return { path: `${dir}/hollow-diagnostics-20261010-030405.zip`, bytes: 18432, entries: 6 } as T;
    }
    // CP1 turn checkpoints: none by default, as the backend answers for a chat
    // outside its own isolated worktree (`turn_checkpoints::isolation`'s text
    // for a folder outside git); `mockTurnCheckpoints` (a TurnCheckpoints JSON)
    // gives a chat some.
    case 'turn_checkpoints': return JSON.parse(localStorage.getItem('mockTurnCheckpoints') ?? 'null')
      ?? { isolated: false, reason: 'This chat\'s folder is not a git worktree.', turns: [] } as T;
    // CH1 Branch from here: `mockBranchKey` names the chat it makes (seed it in
    // `mockWorkspaceSessions` to have it open), `mockBranchInfo` ({ [key]: BranchInfo })
    // is each branch's record for its notice.
    case 'branch_chat': return { key: localStorage.getItem('mockBranchKey') ?? 's-mock-branch', title: null, through: 1, turns: 2, checkpoint: null, refName: null, changedSince: [] } as T;
    case 'chat_branch_info': return ((JSON.parse(localStorage.getItem('mockBranchInfo') ?? '{}') as Record<string, unknown>)[String(args?.key ?? '')] ?? null) as T;
    case 'restore_turn_checkpoint': {
      const ord = Number(args?.ord ?? 1);
      return { ord, through: ord, restored: [], removed: [], branchResetTo: null, safetyRef: `refs/hollow/safety/mock-${ord}`, text: `Restored files to before turn ${ord}.` } as T;
    }
    // `mockAgentConversations` ({ [key]: AgentConversation }) answers per
    // conversation (a subagent's session view, COO rebuild L3).
    case 'read_agent_conversation': return (JSON.parse(localStorage.getItem('mockAgentConversations') ?? '{}') as Record<string, unknown>)[String(args?.key ?? '')] as T
      ?? JSON.parse(localStorage.getItem('mockAgentConversation') ?? 'null') ?? {
      summary: agentSession, eventsRetained: true,
      turns: [
        { id: 'fixture-user', role: 'user', text: 'Wire the session store', tools: [], agent: 'codex', runId: 'run-fixture-1', startedAt: agentSession.createdAt, transcriptIncomplete: false },
        { id: 'fixture-answer', role: 'assistant', text: 'The shared conversation record is ready.', tools: [], agent: 'codex', runId: 'run-fixture-1', startedAt: agentSession.createdAt, status: 'success', transcriptIncomplete: false },
        ...(localStorage.getItem('mockAskCanvasFixture') === '1' ? await askCanvasFixtureTurns(agentSession.createdAt, agentSession.key) : []),
        ...(localStorage.getItem('mockReplyWork') === '1' ? replyWorkFixtureTurns(agentSession.createdAt) : []),
      ],
    } as T;
    case 'read_agent_session': return {
      summary: agentSession,
      turns: [{
        ord: 1,
        runId: 'run-fixture-1',
        startedAt: '2026-08-11T09:00:00Z',
        endedAt: '2026-08-11T09:01:12Z',
        status: 'success',
        asked: 'Wire the session store',
      }],
      eventsRetained: true,
    } as T;
    // Nothing in flight in the harness: the rail's Active lane must render
    // correctly from a null, which is the common case. `mockChatQueued`
    // reports a chat turn still waiting in the run queue (Paperclip PR 7b).
    case 'agent_session_live': {
      // P1a PR 2: `mockPausedChat` names the one chat the backend holds paused.
      const pausedKey = localStorage.getItem('mockPausedChat');
      if (pausedKey && args?.key === pausedKey) {
        const paused: LiveAgentSession = { key: pausedKey, agent: 'claude', runId: '', phase: 'paused', phaseEnteredAt: NOW, failureReason: null, lastEventAt: null, progress: null };
        return paused as T;
      }
      if (localStorage.getItem('mockChatQueued') !== 'true') return null as T;
      const queued: LiveAgentSession = { key: MOCK_CHAT_SESSION_KEY, agent: 'claude', runId: '', phase: 'queued', phaseEnteredAt: NOW, failureReason: null, lastEventAt: null, progress: null };
      return queued as T;
    }
    // Settings, Usage, Value (master plan #6). `__mockValueNoneAccepted`
    // (G4 spec): a month with no accepted run, so hours saved stay hidden.
    case 'summary':
      if ((window as unknown as { __mockValueNoneAccepted?: boolean }).__mockValueNoneAccepted) {
        const none = { runs: 3, accepted: 0, costUsd: 1.25, unknownCostRuns: 0, savedMinutes: 0, savedHoursEstimate: 0, unbaselinedAccepted: 0 };
        return { month: '2026-11', totals: none, groups: [{ key: { department: 'AI', workKind: 'code' }, totals: none }] } as T;
      }
      return {
        month: '2026-10',
        totals: { runs: 4, accepted: 2, costUsd: 2.5, unknownCostRuns: 1, savedMinutes: 90, savedHoursEstimate: 1.5, unbaselinedAccepted: 1 },
        groups: [
          { key: { department: 'Finance', workKind: 'data' }, totals: { runs: 2, accepted: 1, costUsd: 2.5, unknownCostRuns: 0, savedMinutes: 90, savedHoursEstimate: 1.5, unbaselinedAccepted: 0 } },
          { key: { department: 'HR', workKind: 'admin' }, totals: { runs: 1, accepted: 1, costUsd: 0, unknownCostRuns: 1, savedMinutes: 0, savedHoursEstimate: 0, unbaselinedAccepted: 1 } },
        ],
      } as T;
    // Flight recorder (master plan #15).
    case 'run_steps':
      return { sessionKey: MOCK_CHAT_SESSION_KEY, turn: 1, runId: String(args?.runId ?? ''), live: false, steps: [], firstError: null, fedBy: [], accounts: [] } as T;
    case 'get_baselines':
      return { schemaVersion: 1, baselines: { data: { minutes: 90, setAt: NOW } }, runOverrides: {} } as T;
    case 'set_baseline': {
      const input = (args?.input ?? {}) as { workKind?: string; minutes?: number | null };
      const baselines: Record<string, unknown> = { data: { minutes: 90, setAt: NOW } };
      if (input.workKind) {
        if (input.minutes == null) delete baselines[input.workKind];
        else baselines[input.workKind] = { minutes: input.minutes, setAt: NOW };
      }
      return { schemaVersion: 1, baselines, runOverrides: {} } as T;
    }
    // Settings, Usage, Prices (overnight 9 A13): the built-in rows plus
    // the session's saved ones; a saved row replaces a built-in one.
    case 'price_table':
      return mockPriceTable() as T;
    case 'set_price': {
      const { saved: _saved, ...price } = args?.price as ModelPriceRow;
      if (localStorage.getItem('mockSetPriceError')) throw new Error('Synthetic price save failure');
      mockSavedPrices = mockSavedPrices.filter(p => p.model !== price.model);
      if (!args?.remove) mockSavedPrices.push(price);
      return mockPriceTable() as T;
    }
    case 'usage_summary': {
      const period = String(args?.window);
      localStorage.setItem('mockLastUsageWindow', period);
      if (localStorage.getItem('mockUsageError')) throw new Error('Synthetic usage read failure');
      const override = localStorage.getItem('mockUsageSummary');
      return (override ? JSON.parse(override) : usageSummaryFixture(period, args?.agent as string | null | undefined)) as T;
    }
    // Settings, Usage "Claude plan" card (Paperclip PR 8c). Built from the
    // recorded rate_limit_event shape (a 1.01 fraction is 101 %, overage);
    // localStorage.mockClaudeQuota = 'none' hides the card.
    case 'claude_quota': {
      if (localStorage.getItem('mockClaudeQuota') === 'none') return null as T;
      return claudeQuotaFixture() as T;
    }
    // Settings, Accounts, other providers (provider_accounts.rs).
    case 'provider_accounts':
    case 'provider_sign_in':
    case 'provider_sign_out':
      return ([
        { id: 'codex', label: 'ChatGPT (Codex)', installed: true, signedIn: command !== 'provider_sign_out' || args?.id !== 'codex', email: 'me@example.com', plan: 'ChatGPT Pro', note: null, signIn: 'browser', canSignOut: true },
        { id: 'antigravity', label: 'Antigravity', installed: true, signedIn: null, email: null, plan: null, note: 'Antigravity signs in inside its own app: open Terminal, run agy, and choose your Google account.', signIn: 'terminal', canSignOut: false },
        { id: 'grok', label: 'Grok', installed: true, signedIn: true, email: 'me@x.com', plan: null, note: null, signIn: 'browser', canSignOut: true },
      ]) as T;
    case 'claude_accounts':
    case 'refresh_claude_accounts':
      return claudeAccountsFixture() as T;
    case 'add_claude_account': {
      const v = claudeAccountsFixture();
      const label = String(args?.label ?? '').trim();
      const token = String(args?.token ?? '').replace(/\s+/g, '');
      if (!token.startsWith('sk-ant-oat')) throw "That doesn't look like a Claude Code token.";
      const id = `acct-${String(v.accounts.length).padStart(8, '0')}`;
      v.accounts = [...v.accounts, {
        id, label, enabled: true, status: 'unknown', note: null, usage: null,
        lastUsedAt: null, createdAt: new Date().toISOString(),
      }];
      return { ...v } as T;
    }
    case 'update_claude_account': {
      const v = claudeAccountsFixture();
      const input = args?.input as { id: string; label?: string | null; enabled?: boolean | null };
      v.accounts = v.accounts.map(a => a.id !== input.id ? a : {
        ...a, label: input.label ?? a.label, enabled: input.enabled ?? a.enabled,
      });
      return { ...v } as T;
    }
    case 'remove_claude_account': {
      const v = claudeAccountsFixture();
      v.accounts = v.accounts.filter(a => a.id !== args?.id);
      if (v.pinned === args?.id) v.pinned = v.accounts[0]?.id ?? '';
      return { ...v } as T;
    }
    case 'set_claude_account_routing': {
      const v = claudeAccountsFixture();
      v.routing = args?.routing as ClaudeAccountsView['routing'];
      if (args?.pinned) v.pinned = String(args.pinned);
      if (args?.ceilingPercent != null) v.ceilingPercent = Number(args.ceilingPercent);
      v.nextAccount = v.routing === 'single' ? v.pinned : 'acct-0000beef';
      return { ...v } as T;
    }
    // Settings, Models, Self-hosted models (Q4): the main rules of
    // self_hosted_registry::normalize, enough for the form's errors.
    case 'self_hosted_models': return selfHostedView() as T;
    case 'save_self_hosted_model': {
      const entry = args?.entry as SelfHostedEntry;
      const exists = mockSelfHosted.some(e => e.id === entry.id);
      if (!/^[a-z0-9][a-z0-9-]{0,31}$/.test(entry.id)) throw 'The id must be 1 to 32 characters: lowercase letters, digits and hyphens, starting with a letter or digit.';
      if (!entry.name) throw 'The name must be 1 to 80 characters.';
      if (!/^https:\/\/./.test(entry.baseUrl) && !/^http:\/\/(127\.0\.0\.1|localhost|\[::1\])(:\d+)?(\/|$)/.test(entry.baseUrl)) throw 'The address must start with https://, or http:// for this Mac (localhost or 127.0.0.1).';
      if (/^https?:\/\/[^/]*@/.test(entry.baseUrl)) throw "The address can't hold a user name or password (user@host).";
      if (/[?#]/.test(entry.baseUrl)) throw "The address can't have a query (?) or a fragment (#).";
      if (entry.contextWindow < 110000 || entry.contextWindow > 10000000) throw "The context window must be between 110,000 and 10,000,000 tokens: Claude Code compacts at 100,000 tokens at the earliest, and a Hollow chat turn's first call is about 52,000.";
      if (!entry.effortLevels.length) throw 'Choose at least one effort level.';
      if (args?.edit !== true && exists) throw `Another self-hosted model already uses the id "${entry.id}".`;
      if (args?.edit === true && !exists) throw `There is no self-hosted model with the id "${entry.id}" to edit.`;
      const saved = { ...entry, baseUrl: entry.baseUrl.replace(/\/+$/, ''), healthPath: entry.healthPath || '/health' };
      mockSelfHosted = exists ? mockSelfHosted.map(e => (e.id === entry.id ? saved : e)) : [...mockSelfHosted, saved];
      mockWakes.delete(entry.id);
      return selfHostedView() as T;
    }
    case 'remove_self_hosted_model':
      mockSelfHostedEntry(args?.id);
      mockSelfHosted = mockSelfHosted.filter(e => e.id !== args?.id);
      mockWakes.delete(String(args?.id));
      return selfHostedView() as T;
    case 'set_self_hosted_model_enabled': {
      const entry = mockSelfHostedEntry(args?.id);
      mockSelfHosted = mockSelfHosted.map(e => (e === entry ? { ...e, enabled: args?.enabled === true } : e));
      mockWakes.delete(entry.id);
      return selfHostedView() as T;
    }
    case 'test_self_hosted_model': {
      const entry = mockSelfHostedEntry(args?.id);
      // Long enough for the smoke spec to see the row's controls wait.
      await new Promise(resolve => setTimeout(resolve, 1500));
      const result: SelfHostedTest = {
        ok: true, healthStatus: 200, healthMs: 4200, modelsStatus: 200, modelListed: true, reportedWindow: 131072,
        windowMismatch: entry.contextWindow !== 131072, firstByteMs: 812, status: 200, error: null,
      };
      return result as T;
    }
    // A self-hosted chat's start chip (Q6): a short "cold start", and the
    // backend's refusals for a missing or switched-off entry.
    case 'wake_self_hosted_model': {
      const entry = mockSelfHosted.find(e => e.id === args?.id);
      if (!entry) throw `There is no self-hosted model "${String(args?.id)}". Add it in Settings → Models → Self-hosted models.`;
      if (!entry.enabled) throw `${entry.name} is switched off. Switch it on in Settings → Models → Self-hosted models.`;
      const startedAt = Date.now();
      mockWakes.set(entry.id, { startedAt, result: null });
      await new Promise(resolve => setTimeout(resolve, 1500));
      const result: SelfHostedWake = { status: 200, ms: Date.now() - startedAt, error: null, shared: false, startedMsAgo: Date.now() - startedAt };
      mockWakes.set(entry.id, { startedAt, result });
      return result as T;
    }
    case 'self_hosted_wake_status': {
      const last = mockWakes.get(String(args?.id));
      const status: SelfHostedWakeStatus | null = last ? { startedMsAgo: Date.now() - last.startedAt, result: last.result } : null;
      return status as T;
    }
    // Self-hosted models Q10: Background jobs, as self_hosted_jobs::set_at checks.
    case 'self_hosted_jobs': return selfHostedJobsView() as T;
    case 'set_self_hosted_job': {
      const kind = args?.kind as SelfHostedJobKind;
      const entry = typeof args?.entry === 'string' ? args.entry : null;
      if (entry) {
        const saved = mockSelfHosted.find(e => e.id === entry);
        const label = MOCK_JOB_KINDS.find(k => k.kind === kind)?.label ?? kind;
        if (!saved) throw `${label} can't go there: there is no self-hosted model "${entry}" in Settings → Models → Self-hosted models. Nothing changed.`;
        if (!saved.enabled) throw `${label} can't go there: ${saved.name} is switched off. Nothing changed.`;
        mockSelfHostedJobs = { ...mockSelfHostedJobs, [kind]: entry };
      } else {
        const next = { ...mockSelfHostedJobs };
        delete next[kind];
        mockSelfHostedJobs = next;
      }
      return selfHostedJobsView() as T;
    }
    case 'run_self_hosted_jobs_now': throw 'Nothing is waiting for this model.';
    case 'run_self_hosted_job_as_now':
    case 'retry_self_hosted_job':
    case 'discard_self_hosted_job': throw "That job isn't waiting any more.";
    // Settings, Keyboard shortcuts: kept in memory, like the real file.
    case 'get_shortcut_settings': return { ...mockShortcuts, keys: { ...mockShortcuts.keys }, leader: { ...mockShortcuts.leader }, warnings: [] } as T;
    case 'set_shortcut_settings': {
      const next = args?.settings as { enabled: boolean; keys: Record<string, string>; leader?: Record<string, string> };
      mockShortcuts = { enabled: next.enabled !== false, keys: { ...(next.keys ?? {}) }, leader: { ...(next.leader ?? {}) } };
      return { ...mockShortcuts, keys: { ...mockShortcuts.keys }, leader: { ...mockShortcuts.leader }, warnings: [] } as T;
    }
    case 'suspend_shortcuts': return [] as T;
    case 'set_menu_enabled': return undefined as T;
    case 'after_summoned_capture': { const w = window as unknown as { __mockAfterCapture?: boolean[] }; (w.__mockAfterCapture ??= []).push(!!args?.saved); return undefined as T; }
    case 'set_claude_account_failover': {
      const v = claudeAccountsFixture();
      v.failover = args?.enabled === true;
      return { ...v } as T;
    }
    // Settings, Usage budgets (Paperclip PR 8a). Refusals are STRINGS, as
    // Tauri rejects.
    case 'get_budgets': return budgetsViewFixture() as T;
    case 'save_budget': {
      const input = args?.input as BudgetInput;
      if (input.metric === 'costUsd' && (input.scope === 'agent:codex' || input.scope === 'agent:antigravity')) {
        throw "Codex reports no price of its own (any cost is an estimate at a price you entered), so a spending budget on Codex alone isn't offered. Use a token budget instead.";
      }
      const defs = mockBudgetDefs();
      if (defs.some(d => d.id !== input.id && d.scope === input.scope && d.metric === input.metric && d.window === input.window)) {
        throw `There is already a ${input.window} ${input.metric === 'tokens' ? 'token' : 'spending'} budget for ${input.scope}.`;
      }
      const def: BudgetDef = {
        id: input.id || `b-mock-${defs.length + 1}`, scope: input.scope, metric: input.metric, window: input.window,
        limit: input.limit, warnPercent: input.warnPercent ?? 80, hardStop: input.hardStop ?? true, createdAt: NOW, updatedAt: NOW,
      };
      const next = defs.some(d => d.id === def.id) ? defs.map(d => (d.id === def.id ? def : d)) : [...defs, def];
      localStorage.setItem('mockBudgets', JSON.stringify(next));
      return def as T;
    }
    case 'delete_budget': {
      localStorage.setItem('mockBudgets', JSON.stringify(mockBudgetDefs().filter(d => d.id !== args?.id)));
      return undefined as T;
    }
    case 'resume_budget': {
      const def = mockBudgetDefs().find(d => d.id === args?.id);
      if (!def) throw 'That budget no longer exists.';
      const newLimit = Number(args?.newLimit);
      if (!Number.isFinite(newLimit) || newLimit <= mockObserved(def)) {
        throw `New limit must be above this ${def.window}'s usage (${mockObserved(def)}).`;
      }
      localStorage.setItem('mockBudgets', JSON.stringify(mockBudgetDefs().map(d => (d.id === def.id ? { ...d, limit: newLimit } : d))));
      localStorage.setItem('mockBudgetPaused', '[]');
      return undefined as T;
    }
    case 'list_history_runs':
    case 'list_note_history': return [historyRun, { ...historyRun, id: 'run-artifact-1', source: 'chat', sourceId: 'codex-artifact-thread', status: 'success', changes: [{ ...historyRun.changes[1], vaultPath: String(args?.path || project.vaultPath) }] }] as T;
    case 'list_evidence_receipts':
      // Opt-in (Paperclip PR 17): a chat receipt the legacy Claude view joins
      // to its bubble by TIMING, to prove such a join is never used as the
      // identity of a saved vote.
      if (args?.source === 'chat' && localStorage.getItem('mockLegacyChatReceipts')) {
        return [{
          ...workflowReceipt, id: 'receipt-legacy-1', runId: 'run-legacy-1', source: 'chat',
          sourceId: 'legacy-claude-session', startedAt: LEGACY_ASKED_AT, finishedAt: LEGACY_ASKED_AT,
          status: 'success', verificationState: 'noChanges', files: [], checks: [], failures: [],
          historyRunId: 'run-legacy-1',
        }] as T;
      }
      return (
        args?.source === 'workflow' && args?.sourceId === 'morning-brief' ? [mockWorkflowReceipt()] : []
      ) as T;
    // Plan completion L2: one run's cost roll-up (`run_cost`). Unknown when
    // `localStorage.mockRunCostUnknown` is set, never $0.
    case 'run_cost': return (localStorage.getItem('mockRunCostUnknown')
      ? { ownUsd: null, descendantsUsd: 0, totalUsd: null, knownUsd: 0, unknown: 1, descendants: [] }
      : { ownUsd: 0.18, descendantsUsd: 0, totalUsd: 0.18, knownUsd: 0.18, unknown: 0, descendants: [] }) as T;
    // Repo features R6: the usage ring's What's using it and Cache.
    // Quiet in the mock: smoke pins the elapsed line, and three finished
    // chats of a kind are what turns the usual time on.
    case 'job_timing': return { workKind: 'code', typicalMs: null } as T;
    case 'conversation_context_use': return {
      windowStartedAt: NOW, windowTurns: 3, measuredTurns: 3, atLeast: false,
      byTool: [{ tool: 'Read', bytes: 96_000, calls: 7 }, { tool: 'Bash', bytes: 41_000, calls: 5 }, { tool: 'Grep', bytes: 6_200, calls: 3 }],
      largest: [{ turn: 2, tool: 'Read', bytes: 48_000, target: 'chat.rs' }, { turn: 3, tool: 'Bash', bytes: 22_000, target: 'cargo' }, { turn: 1, tool: 'Read', bytes: 9_000, target: 'README.md' }],
      cache: [
        { turn: 1, inputTokens: 38_000, cachedInputTokens: 0, cachedShare: 0, cold: true, cause: 'newSession' },
        { turn: 2, inputTokens: 4_000, cachedInputTokens: 60_000, cachedShare: 0.94, cold: false, cause: null },
        { turn: 3, inputTokens: 70_000, cachedInputTokens: 2_000, cachedShare: 0.03, cold: true, cause: 'idle' },
      ],
      compactions: 0,
    } as T;
    case 'read_history_version': return (args?.version === 'before' ? null : '# Morning brief\n\nFixture output.') as T;
    case 'read_history_repo_changes': return (args?.runId === historyRun.id ? workflowRepoSidecar : null) as T;
    case 'history_storage_status': return {
      runCount: 1,
      activeRunCount: 0,
      objectCount: 1,
      totalBytes: 1024,
      objectBytes: 512,
      maxRuns: 200,
      maxAgeDays: 90,
      maxBytes: 536_870_912,
      oldestRunAt: historyRun.startedAt,
      newestRunAt: historyRun.finishedAt,
    } as T;
    case 'list_sessions': return (localStorage.getItem('mockLegacySession') ? [{
      project: 'vault', projectDir: '-mock-vault', sessionId: 'legacy-claude-session',
      title: 'Earlier Claude conversation', firstPrompt: 'Earlier request', messageCount: 2,
      lastActivity: NOW, sizeBytes: 100, cwd: '/Users/fixture/vault', gitBranch: null,
    }] : []) as SessionMeta[] as T;
    case 'read_session': return {
      project: 'vault', sessionId: 'legacy-claude-session',
      turns: localStorage.getItem('mockLegacyChatReceipts')
        // With a user turn, so rehydrateSessionTurns has a segment to join to.
        ? [
          { role: 'user', text: 'Earlier request', timestamp: LEGACY_ASKED_AT, toolUses: [] },
          { role: 'assistant', text: 'Earlier Claude answer', timestamp: NOW, toolUses: [] },
        ]
        : [{ role: 'assistant', text: 'Earlier Claude answer', timestamp: NOW, toolUses: [] }],
    } as T;
    case 'vault_tree': return vaultTree as T;
    case 'vault_items': return ((window as unknown as { __mockDemoLibrary?: VaultItem[] }).__mockDemoLibrary ? [...vaultItems, ...(window as unknown as { __mockDemoLibrary: VaultItem[] }).__mockDemoLibrary] : vaultItems) as T;
    // `mockVaultNotes` (localStorage) seeds a whole vault index, e.g. a real vault's titles for a Library capture.
    // A note renamed, moved or trashed in Library (P1-18) leaves the list (`mockGoneNotes`); its new path joins it.
    case 'vault_notes_list': if ((window as unknown as { __mockDemoLibrary?: VaultItem[] }).__mockDemoLibrary) return withMovedNotes((window as unknown as { __mockDemoLibrary: VaultItem[] }).__mockDemoLibrary.filter(n => !mockGoneNotes.has(n.path)).map(n => ({ path: n.path, basename: n.path.split('/').pop()!.replace(/\.md$/, ''), title: n.title, summary: n.excerpt }))) as T;
      if (localStorage.getItem('mockVaultNotes')) return withMovedNotes((JSON.parse(localStorage.getItem('mockVaultNotes')!) as { path: string; basename: string }[]).filter(n => !mockGoneNotes.has(n.path))) as T; return [{ path: 'projects/Hollow 3.0.md', basename: 'Hollow 3.0' }, ...[...mockWrittenNotes.keys()].filter(path => path !== 'projects/Hollow 3.0.md').map(path=>({path,basename:path.split('/').pop()!.replace(/\.md$/, '')}))].filter(n => !mockGoneNotes.has(n.path) && !n.path.startsWith('.trash/')) as T;
    case 'vault_move_note': {
      const from = String(args?.from ?? ''), to = String(args?.to ?? '');
      if (mockWrittenNotes.has(to)) throw new Error(`destination exists: ${to}`);
      const content = mockWrittenNotes.get(from) ?? '# Hollow 3.0\n\nFixture vault note.';
      mockWrittenNotes.delete(from); mockMovedNotes.delete(from); mockGoneNotes.add(from);
      mockWrittenNotes.set(to, content); mockGoneNotes.delete(to); if (!to.startsWith('.trash/')) mockMovedNotes.add(to);
      return { path: to, content, mtime: NOW, size: content.length } as T;
    }
    case 'vault_delete_note': {
      const path = String(args?.path ?? '');
      const trashed = `.trash/${Date.now()}-0-${path.split('/').pop()}`;
      mockWrittenNotes.set(trashed, mockWrittenNotes.get(path) ?? '# Hollow 3.0\n\nFixture vault note.');
      mockWrittenNotes.delete(path); mockMovedNotes.delete(path); mockGoneNotes.add(path);
      return trashed as T;
    }
    case 'read_work_file':
    case 'vault_note': {
      const notePath = String(args?.path ?? 'projects/Hollow 3.0.md');
      const noteContent = mockWrittenNotes.get(notePath) ?? '# Hollow 3.0\n\nFixture vault note.';
      return { path: notePath, content: noteContent, mtime: NOW, size: noteContent.length, revision: noteContent } as T;
    }
    case 'vault_create_note': {
      const path = String(args?.path ?? 'notes/new.md');
      if (mockWrittenNotes.has(path) || vaultItems.some(note => note.path === path)) throw new Error('A note already exists at this path');
      const content = String(args?.content ?? '');
      mockWrittenNotes.set(path, content);
      return { path, content, mtime: NOW, size: content.length } as T;
    }
    case 'vault_save_note_cas': {
      const path = String(args?.path ?? '');
      const content = String(args?.content ?? '');
      const current = mockWrittenNotes.get(path);
      const conflict = current !== undefined && current !== args?.baseline;
      if (!conflict) mockWrittenNotes.set(path, content);
      return { conflict, note: { path, content: conflict ? current : content, mtime: NOW, size: (conflict ? current! : content).length } } as T;
    }
    case 'vault_open_in_obsidian':
      localStorage.setItem('mockObsidianOpen', JSON.stringify(args));
      return undefined as T;
    // P1-41: mockObsidianMissing='true' plays a Mac with no obsidian:// handler.
    case 'obsidian_available': return (localStorage.getItem('mockObsidianMissing') !== 'true') as T;
    // 5.2: the card's git rows: a dirty repo on main with one other branch; mockGitCard records each call for specs.
    case 'git_card_status': return { repo: true, root: '/mock/repo', branch: 'main', head: 'abc1234', upstream: 'origin/main', ahead: 1, behind: 0, linkedWorktree: false, files: [{ path: 'src/a.ts', status: 'modified', additions: 3, deletions: 1 }], moreFiles: 0, additions: 3, deletions: 1, blocked: null } as T;
    case 'pr_card_view': return { detail: { number: 7, title: 'Add the thing', state: 'open', url: 'https://github.com/o/r/pull/7', draft: false, reviewDecision: 'REVIEW_REQUIRED', mergeState: 'BLOCKED', head: 'topic', base: 'main', checks: [{ name: 'gate', state: 'success', url: 'https://github.com/o/r/actions/1' }, { name: 'lint', state: 'pending', url: null }], moreChecks: 0, comments: [{ author: 'ann', kind: 'comment', state: null, body: 'Looks good to me.', createdAt: '2026-11-20T10:00:00Z', url: 'https://github.com/o/r/pull/7#c1' }], moreComments: 0 }, note: null } as T;
    // 7.3: Projects → Pull requests. One PR and one issue; the mock records each triage call for specs.
    case 'github_list': return ((args?.query as { kind?: string } | undefined)?.kind === 'issue'
      ? { kind: 'issue', repo: 'o/r', more: false, note: null, rows: [{ number: 12, title: 'Crash on launch (ENG-12)', state: 'open', url: 'https://github.com/o/r/issues/12', draft: false, author: 'ann', labels: [{ name: 'bug', color: 'd73a4a' }], assignees: [], updatedAt: '2026-11-20T10:00:00Z', comments: 1, reviewDecision: null, checks: null, head: '', linear: [{ key: 'ENG-12', title: 'Fix the lot map', url: 'https://linear.app/x/issue/ENG-12', tracked: true }] }] }
      : { kind: 'pr', repo: 'o/r', more: false, note: null, rows: [{ number: 7, title: 'Add the thing', state: 'open', url: 'https://github.com/o/r/pull/7', draft: false, author: 'ann', labels: [{ name: 'enhancement', color: 'a2eeef' }], assignees: ['bob'], updatedAt: '2026-11-20T10:00:00Z', comments: 1, reviewDecision: 'REVIEW_REQUIRED', checks: 'pending', head: 'topic', linear: [] }] }) as T;
    case 'github_detail': return (args?.kind === 'issue'
      ? { detail: { kind: 'issue', repo: 'o/r', item: { number: 12, title: 'Crash on launch (ENG-12)', state: 'open', url: 'https://github.com/o/r/issues/12', draft: false, reviewDecision: null, mergeState: null, head: '', base: '', checks: [], moreChecks: 0, comments: [{ author: 'cy', kind: 'comment', state: null, body: 'Seen on 0.5.15 too.', createdAt: '2026-11-20T10:30:00Z', url: 'https://github.com/o/r/issues/12#c1' }], moreComments: 0 }, body: 'It crashes right after the splash screen.', author: 'ann', labels: [{ name: 'bug', color: 'd73a4a' }], assignees: [], createdAt: '2026-11-19T10:00:00Z', updatedAt: '2026-11-20T10:00:00Z', linear: [{ key: 'ENG-12', title: 'Fix the lot map', url: 'https://linear.app/x/issue/ENG-12', tracked: true }] }, note: null }
      : { detail: { kind: 'pr', repo: 'o/r', item: { number: 7, title: 'Add the thing', state: 'open', url: 'https://github.com/o/r/pull/7', draft: false, reviewDecision: 'REVIEW_REQUIRED', mergeState: 'BLOCKED', head: 'topic', base: 'main', checks: [{ name: 'gate', state: 'success', url: 'https://github.com/o/r/actions/1' }, { name: 'lint', state: 'pending', url: null }], moreChecks: 0, comments: [{ author: 'ann', kind: 'comment', state: null, body: 'Looks good to me.', createdAt: '2026-11-20T10:00:00Z', url: 'https://github.com/o/r/pull/7#c1' }], moreComments: 0 }, body: 'Adds the thing.', author: 'ann', labels: [{ name: 'enhancement', color: 'a2eeef' }], assignees: ['bob'], createdAt: '2026-11-19T10:00:00Z', updatedAt: '2026-11-20T10:00:00Z', linear: [] }, note: null }) as T;
    case 'github_triage': localStorage.setItem('mockGithubTriage', JSON.stringify({ command, args })); return { message: `Done: ${(args?.triage as { action?: string } | undefined)?.action} #${(args?.triage as { number?: number } | undefined)?.number}`, repo: 'o/r' } as T;
    // 7.2: the terminal drawer in the browser mock: a shell that prints nothing; a plan chat is refused as the backend does.
    case 'terminal_open': return (String(args?.perm) === 'plan' ? Promise.reject('The terminal is off for this chat. Plan mode reads and changes nothing, so a shell would do more than the chat is allowed to. Switch the chat to a mode that runs commands without a folder sandbox (Auto, Approve for me or Bypass) to use it.') : { id: `term-mock-${Date.now()}`, cwd: String(args?.cwd ?? '/mock/repo'), shell: '/bin/zsh', pid: 4242 }) as T;
    case 'terminal_gate': return (String(args?.perm) === 'plan' ? 'The terminal is off for this chat.' : null) as T;
    case 'terminal_write': case 'terminal_resize': case 'terminal_close': case 'terminal_adopt_chat': return undefined as T;
    case 'git_card_diff': return { diff: 'diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,3 @@\n keep\n-old\n+new\n+more\n', truncated: false, truncatedFiles: [] } as T;
    case 'git_card_range_files': return [{ path: 'src/a.ts', status: 'modified', additions: 2, deletions: 1 }] as T;
    case 'git_card_branches': return { current: 'main', branches: ['main', 'topic'] } as T;
    case 'git_card_suggest_message': return 'Update a.ts\n\n- update src/a.ts (+3 -1)' as T;
    case 'git_card_new_worktree': localStorage.setItem('mockGitCard', JSON.stringify({ command, args })); return '/mock/worktrees/repo/topic' as T;
    case 'git_card_switch': case 'git_card_commit_push': localStorage.setItem('mockGitCard', JSON.stringify({ command, args })); return { repo: true, root: '/mock/repo', branch: 'main', head: 'abc1234', upstream: 'origin/main', ahead: 0, behind: 0, linkedWorktree: false, files: [], moreFiles: 0, additions: 0, deletions: 0, blocked: null } as T;
    // 5.1: the card's Open in menu: Finder and one editor; an open is recorded for specs.
    case 'open_in_targets': return [{ id: 'finder', label: 'Finder' }, { id: 'vscode', label: 'Visual Studio Code' }] as T;
    case 'open_in_open': localStorage.setItem('mockOpenIn', JSON.stringify(args)); return undefined as T;
    case 'vault_backlinks': return [] as T;
    case 'vault_search': return [] as SearchResult[] as T;
    case 'semantic_search': return [] as SearchResult[] as T;
    case 'semantic_search_status': return { enabled: false, indexed: 0, total: 0 } as SemanticStatus as T;
    case 'vault_graph': return graph as T;
    case 'list_skills': return [{
      name: 'release-notes',
      source: 'shared',
      sourceKind: 'user',
      plugin: null,
      path: '/tmp/hollow/skills/release-notes/SKILL.md',
      description: 'Draft release notes from the run log.',
      version: null,
      writable: true,
    },
    // Synthetic duplicate across ownership (audit H13): shared + provider copy.
    { name: 'humanizer', source: 'shared', sourceKind: 'user', plugin: null, path: '/Users/fixture/.agents/skills/humanizer/SKILL.md', description: 'Remove signs of AI-generated writing from text.', version: null, writable: true },
    { name: 'humanizer', source: 'provider:claude', sourceKind: 'user', plugin: null, path: '/Users/fixture/.claude/skills/humanizer/SKILL.md', description: 'Remove signs of AI-generated writing from text.', version: null, writable: false },
    { name: 'pdf', source: 'plugin:claude/fixture-market/document-skills@1.2.0', sourceKind: 'plugin', plugin: 'document-skills', path: '/Users/fixture/.claude/plugins/cache/fixture-market/document-skills/1.2.0/skills/pdf/SKILL.md', description: 'Read, fill and combine PDF files.', version: '1.2.0', writable: false },
    ...mockCreatedSkills,
    ] as Skill[] as T;
    // NV3: a synthetic portability table (the backend's words in `skill_portability.rs`).
    case 'skill_portability_table': {
      const path = args?.path ? String(args.path) : null;
      const rule = (id: string, channel: 'hollow' | 'native', effect: string, what: string, sentence: string, verifiedOn: string | null, status: string, unverifiedReason: string | null = null) => ({ id, channel, effect, what, sentence, verifiedOn, unverifiedReason, status });
      const desc = rule('desc-cap-160', 'hollow', 'degraded', 'Index description cut at 160 characters', 'The turn\'s skill index cuts the description to 160 characters, so the agent sees only its start.', null, 'hollow');
      const native = (id: string, what: string) => rule(id, 'native', 'unknown', what, 'Not verified for this version.', null, 'unverified', 'fixture: not verified');
      const agents = [['claude', 'Claude', '2.1.325', null], ['codex', 'Codex', '0.169.0', null], ['grok', 'Grok', '1.0.79', '1.0.46'], ['antigravity', 'Antigravity', '1.5.9', '1.2.15']] as const;
      return {
        skill: path ? 'release-notes' : null,
        agents: agents.map(([agent, label, installed, checked]) => {
          const rules = [desc, ...(checked ? [rule('inline-8k-own-tools', 'hollow', 'degraded', 'Over 8 KB: opened by path', 'Over 8 KB the skill\'s text does not go inline; this agent has no read_skill and opens it by path.', checked, 'notVerifiedOnThisVersion')] : []), native(`native-${agent}`, `${label}\'s own loader`)];
          const long = path?.includes('long');
          return { agent, label, installed, rules, stale: !!checked, summary: path ? (long ? 'degraded' : 'loads') : null,
            findings: path ? [...(long ? [desc] : []), native(`native-${agent}`, '')].map(r => ({ rule: r.id, channel: r.channel, effect: r.effect, sentence: r.sentence, verifiedOn: r.verifiedOn, status: r.status })) : [] };
        }),
      } as T;
    }
    // Library → Skills → New skill (X21's stale-refresh smoke creates one,
    // then the list refresh fails once): kept for this page load only.
    case 'write_user_skill': {
      const name = String(args?.name ?? '');
      const existing = mockCreatedSkills.findIndex(s => s.name === name);
      if (args?.createOnly !== false && (existing >= 0 || ['release-notes','humanizer'].includes(name))) throw new Error(`SKILL_EXISTS: /${name} already exists`);
      const skill = { name, source: 'shared', sourceKind: 'user', plugin: null, path: `/tmp/hollow/skills/${name}/SKILL.md`, description: String(args?.description ?? ''), version: null, writable: true } as Skill;
      if (existing >= 0) mockCreatedSkills[existing] = skill;
      else mockCreatedSkills.push(skill);
      mockSkillFiles.set(skill.path, `---\nname: ${name}\ndescription: ${skill.description || name}\n---\n\n${String(args?.body ?? '').trimEnd()}\n`);
      return skill as T;
    }
    case 'read_skill': return (mockSkillFiles.get(String(args?.path ?? '')) ?? '# release-notes\n\nFixture skill body.') as T;
    // Skill import: `mockSkillImportName` picks the staged name (default
    // pdf-tools; release-notes clashes with the fixture above);
    // `mockSkillImportError` makes the preview refuse with that text.
    case 'preview_skill_import': {
      // SD2: a rename (the refusal's suggested name) gets past the refusal.
      const renameTo = typeof args?.renameTo === 'string' ? args.renameTo : null;
      const error = localStorage.getItem('mockSkillImportError');
      if (error && !renameTo) throw error;
      const name = renameTo ?? localStorage.getItem('mockSkillImportName') ?? 'pdf-tools';
      const clash = name === 'release-notes';
      return {
        ...(renameTo ? { renamedFrom: 'stitch::generate-design', rewritten: [{ path: 'SKILL.md', count: 3 }] } : {}),
        token: '0123456789abcdef0123456789abcdef',
        name,
        description: 'Read and fill PDF forms.',
        files: [
          { path: 'SKILL.md', size: 1830, script: false },
          { path: 'references/forms.md', size: 5120, script: false },
          { path: 'scripts/fill.py', size: 2410, script: true },
        ],
        totalBytes: 9360,
        hasScripts: true,
        skipped: ['__MACOSX/pdf-tools/._SKILL.md'],
        folder: name,
        existing: clash ? `/tmp/hollow/skills/${name}/SKILL.md` : null,
        canReplace: clash,
        keepBothName: `${name}-2`,
      } as T;
    }
    case 'commit_skill_import': {
      const name = localStorage.getItem('mockSkillImportName') ?? 'pdf-tools';
      const keep = args?.onClash === 'keep-both';
      const installed = keep ? `${name}-2` : name;
      localStorage.setItem('mockSkillImportCommitted', JSON.stringify(args ?? {}));
      return {
        skill: { name: installed, source: 'shared', sourceKind: 'user', plugin: null, path: `/tmp/hollow/skills/${installed}/SKILL.md`, description: 'Read and fill PDF forms.', version: null, writable: true },
        files: 3,
        hasScripts: true,
        replaced: args?.onClash === 'replace',
        renamedFrom: keep ? name : null,
      } as T;
    }
    case 'discard_skill_import': return undefined as T;
    // Repo ideas 10-27 #2: release-notes has an import record (from a git
    // checkout) with one file changed outside Hollow; every other shared
    // skill has none. `mockSkillUpdateError` makes Check for update refuse.
    case 'skill_provenance': case 'set_skill_note': {
      const name = String(args?.name ?? '');
      if (command === 'set_skill_note') {
        const note = String(args?.note ?? '').trim();
        if (note) mockSkillNotes.set(name, note); else mockSkillNotes.delete(name);
      }
      const record = name === 'release-notes' ? {
        from: { kind: 'path', path: '/Users/fixture/src/notes-skills/release-notes' },
        origin: { url: 'https://github.com/fixture/notes-skills.git', commit: 'a701c3d3b144aaaaaaaaaaaaaaaaaaaaaaaaaaaa', subdir: 'release-notes' },
        files: { 'SKILL.md': 'aa', 'scripts/draft.py': 'bb' },
        importedAt: '2026-10-27T14:05:00Z',
      } : null;
      return { name, record, changed: record ? { added: [], removed: [], modified: ['scripts/draft.py'] } : null, error: null, canCheck: !!record, note: mockSkillNotes.get(name) ?? null } as T;
    }
    case 'stage_skill_update': {
      const error = localStorage.getItem('mockSkillUpdateError');
      if (error) throw error;
      const name = String(args?.name ?? '');
      return {
        token: 'fedcba9876543210fedcba9876543210', name, description: 'Draft release notes from the run log.',
        files: [{ path: 'SKILL.md', size: 900, script: false }, { path: 'scripts/draft.py', size: 1400, script: true }, { path: 'templates/notes.md', size: 300, script: false }],
        totalBytes: 2600, hasScripts: true, skipped: [], folder: null,
        existing: `/tmp/hollow/skills/${name}/SKILL.md`, canReplace: true, keepBothName: `${name}-2`,
        update: {
          changes: { added: ['templates/notes.md'], removed: [], modified: ['scripts/draft.py'] }, scriptsChanged: ['scripts/draft.py'],
          diffs: [
            { path: 'scripts/draft.py', script: true, text: '  import sys\n- print(open(sys.argv[1]).read())\n+ print(open(sys.argv[1]).read().strip())\n', truncated: false, listedOnly: false },
            { path: 'templates/notes.md', script: false, text: '+ # Release notes\n+ {{items}}\n', truncated: false, listedOnly: false },
          ],
        },
      } as T;
    }
    case 'list_workflows': return mockWorkflowList() as T;
    case 'read_workflow': return {
      workflow: mockWorkflowList().find(w => w.slug === args?.slug) ?? mockWorkflowList()[0],
      prompt: 'Create the daily brief and write it to briefs/*.html.',
      runLog: mockRunLog(withPermissionHolds(workflows)[0].slug),
      // Changes with every mocked save, like the real digest of the saved
      // fields, so a smoke test can catch an editor that reuses a stale base.
      editBase: `mock-edit-base-${localStorage.getItem('mockWorkflowSaves') ?? '0'}`,
    } as T;
    case 'list_workflow_outputs': return [] as T;
    // next-level P17: an editor save without the hygiene spec's seam returns
    // the fixture with the input applied (nothing is persisted).
    case 'write_workflow': return mockSaveWorkflow(args) as T;
    // Run queue (Paperclip PR 7): seed `localStorage.mockRunQueue` with
    // RunQueueItem rows. Cancel mirrors `run_queue::cancel` by status: a
    // queued cron row is only flagged (its occurrence is bookkept at claim),
    // another queued row ends cancelled, a claimed row is flagged to stop,
    // and a running heartbeat refuses. Tauri rejects with plain strings.
    case 'list_run_queue': return JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]') as T;
    // Chat rows created by the mocked chat_send (Paperclip PR 7b) live in
    // `mockChatRows`, not the seeded fixture, and follow the same rules: a
    // queued chat turn ends cancelled with its text cleared, as
    // `run_queue::finish` clears a chat row's detail.
    case 'cancel_queued_run': {
      const items = JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]') as RunQueueItem[];
      const item = items.find(row => row.id === Number(args?.id)) ?? mockChatRows.get(Number(args?.id));
      if (!item) {
        // A running subagent's row (`mockSubagents`, overnight 11-13 C9):
        // Stop goes by its queue row and ends that conversation's turn.
        const views = Object.values(JSON.parse(localStorage.getItem('mockSubagents') ?? '{}') as Record<string, { active?: { current: string; queueId?: number | null; status: string }[] }>);
        const sub = views.flatMap(v => v.active ?? []).find(r => r.queueId === Number(args?.id) && r.status === 'running');
        if (!sub) throw `There is no queued run ${String(args?.id)}.`;
        localStorage.setItem('mockCancelledChatKey', sub.current);
        return { id: Number(args?.id), source: 'chat', sourceId: sub.current, status: 'running', cancelRequested: true } as unknown as T;
      }
      if (item.source === 'chat' && item.status === 'queued') {
        item.status = 'cancelled';
        item.finishedAt = NOW;
        item.error = 'cancelled before it started';
        item.position = null;
        item.detail = null;
        return item as T;
      }
      if (item.status === 'running' && item.source === 'heartbeat') {
        throw 'The heartbeat cannot be stopped once it starts.';
      }
      if ((item.status === 'queued' && item.source === 'cron') || item.status === 'claimed') {
        item.cancelRequested = true;
      } else if (item.status === 'queued') {
        item.status = 'cancelled';
        item.finishedAt = NOW;
        item.error = 'cancelled before it started';
        item.position = null;
      }
      localStorage.setItem('mockRunQueue', JSON.stringify(items));
      return item as T;
    }
    case 'run_workflow': {
      const slug = String(args?.slug ?? '');
      // Plan completion L2: what the last Run now asked for (a guarded retry
      // carries `retryOf`), for the smoke spec.
      localStorage.setItem('mockLastRunWorkflowArgs', JSON.stringify(args ?? {}));
      const refused = JSON.parse(localStorage.getItem('mockRefusedWorkflows') ?? '[]') as string[];
      // Tauri rejects invokes with plain strings, so the mock throws one too.
      if (refused.includes(slug)) {
        throw `${WORKFLOW_REVIEW_REFUSAL_PREFIX} “${slug}” was changed outside Hollow's automation editor (Permission mode: acceptEdits → bypassPermissions). It will not run until you approve the change in Automations → Permission changes.`;
      }
      // Paperclip PR 22: `localStorage.mockCancelledWorkflowRun = '1'` makes
      // Run now come back stopped, as a run the user cancelled mid-flight does;
      // `'throw'` rejects it with the run permit's expected-cancellation text
      // (a plain string, as Tauri rejects), which trackTask must show neutrally.
      if (localStorage.getItem('mockCancelledWorkflowRun') === 'throw') {
        throw `${slug} was stopped while waiting for a background agent slot`;
      }
      if (localStorage.getItem('mockCancelledWorkflowRun') === '1') {
        return {
          slug, status: 'cancelled', durationSec: 1, output: '', error: null, receipt: null, evidenceError: null,
        } satisfies WorkflowRunResult as T;
      }
      return {
        slug, status: 'success', durationSec: 1, output: 'Fixture run complete.', error: null, receipt: null, evidenceError: null,
      } satisfies WorkflowRunResult as T;
    }
    case 'list_people': return [{
      name: 'Jordan',
      role: 'Operator',
      relationship: 'Owner',
      description: 'Prefers direct, pragmatic updates. Cadence: weekly check-in. Next: review 3.0 slice.',
      path: 'people/Jordan.md',
    }] as T;
    case 'git_activity': return gitActivity as T;
    case 'list_activity_ledger': return [] as T;
    case 'backup_health': return {
      status: args?.managedElsewhere ? 'protected' : 'partial',
      summary: args?.managedElsewhere
        ? 'At least one explicit backup signal is present. Hollow has not run or verified a restore.'
        : 'A possible backup signal exists, but Hollow cannot treat it as proof of protection.',
      signals: [
        { id: 'cloud-path', label: 'Cloud-synced location', state: 'partial', detail: 'The vault is under a OneDrive-looking path; sync success is not verified.' },
        { id: 'git', label: 'Git remote', state: 'missing', detail: 'No enclosing repository with a configured remote was found.' },
        { id: 'time-machine', label: 'Time Machine', state: 'missing', detail: 'macOS does not report a configured Time Machine destination.' },
        { id: 'managed', label: 'Managed elsewhere', state: args?.managedElsewhere ? 'confirmed' : 'missing', detail: args?.managedElsewhere ? 'You marked this vault as protected by another backup system on this Mac.' : 'No machine-local acknowledgement of another backup system.' },
      ],
    } as T;
    case 'list_heartbeat_actions':
    case 'list_notifications': return { items: notifications.map(n => ({ ...n })), total: notifications.length } as T;
    // Scoped, and a failure still waiting in Review stays (P0-08).
    case 'mark_notifications_read': {
      const ids = new Set((args?.ids as string[] | undefined) ?? []);
      let n = 0;
      for (const row of notifications) if (ids.has(row.id) && !row.read && row.kind !== 'error') { row.read = true; n++; }
      return n as T;
    }
    case 'delete_notifications': {
      const ids = new Set((args?.ids as string[] | undefined) ?? []);
      const before = notifications.length;
      const keep = notifications.filter(row => !ids.has(row.id) || (!row.read && row.kind === 'error'));
      notifications.length = 0;
      notifications.push(...keep);
      return (before - keep.length) as T;
    }
    case 'unread_notification_count': return notifications.filter(n => !n.read).length as T;
    // The panel refetches after every row action, so these mutate the fixture
    // — otherwise read and delete would visibly undo themselves.
    case 'mark_notification_read': {
      const row = notifications.find(n => n.id === String(args?.id ?? ''));
      if (row) row.read = true;
      return undefined as T;
    }
    case 'delete_notification': {
      const i = notifications.findIndex(n => n.id === String(args?.id ?? ''));
      if (i >= 0) notifications.splice(i, 1);
      return undefined as T;
    }
    // IPC deserialization always gives the real app a fresh object graph.
    // Mirror that here so Solid resources invalidate row children after an
    // explicit review instead of retaining text nodes from the old fixture.
    // The About you switch the backend persists (operator-context.json).
    case 'operator_context_switch': return { ...operatorSwitch } as T;
    case 'set_operator_context_enabled':
      operatorSwitch = { enabled: Boolean(args?.enabled), chosen: true };
      return undefined as T;
    case 'operator_context': return {
      ...operatorContext,
      files: [...operatorContext.files],
      freshness: operatorContext.freshness.map((item) => ({ ...item })),
    } as T;
    case 'profile_builder_view': return profileBuilderSnapshot() as T;
    case 'profile_builder_scan': {
      await new Promise(resolve => setTimeout(resolve, 150));
      profileBuilder.scannedAt = new Date().toISOString();
      profileBuilder.sources = [
        { id: 'claude-instructions', label: 'Claude Code instructions', count: 1 },
        { id: 'claude-memory', label: 'Claude memories', count: 12 },
        { id: 'conversations', label: 'Recent requests', count: 40 },
      ];
      if (!profileBuilder.suggestions.length && !localStorage.getItem('mockProfileBuilderDone')) {
        const at = profileBuilder.scannedAt;
        profileBuilder.suggestions = [
          { id: 'pf-role', file: 'Identity.md', label: 'Role', section: null, text: 'Runs payroll and HR systems at Acme', source: 'claude-memory', createdAt: at },
          { id: 'pf-answer-first', file: 'Communication.md', label: null, section: 'Always', text: 'Lead with the answer, then the reasoning', source: 'claude-instructions', createdAt: at },
        ];
        profileBuilder.questions = [
          { id: 'pq-timezone', file: 'Identity.md', label: 'Timezone', section: 'Context that always helps', question: 'What timezone do you work in?', why: 'So agents schedule and date things right', createdAt: at },
        ];
      }
      return profileBuilderSnapshot() as T;
    }
    case 'profile_builder_accept': {
      const i = profileBuilder.suggestions.findIndex(x => x.id === args?.id);
      if (i < 0) throw 'That suggestion is gone; look again.';
      const [s] = profileBuilder.suggestions.splice(i, 1);
      const text = typeof args?.text === 'string' && args.text.trim() ? args.text.trim() : s.text;
      fileProfile(s.file, s.label ? `${s.label}: ${text}` : text);
      return profileBuilderSnapshot() as T;
    }
    case 'profile_builder_accept_all': {
      for (const s of profileBuilder.suggestions.splice(0)) fileProfile(s.file, s.label ? `${s.label}: ${s.text}` : s.text);
      return profileBuilderSnapshot() as T;
    }
    case 'profile_builder_dismiss':
      profileBuilder.suggestions = profileBuilder.suggestions.filter(x => x.id !== args?.id);
      return profileBuilderSnapshot() as T;
    case 'profile_builder_answer': {
      const q = profileBuilder.questions.find(x => x.id === args?.id);
      if (!q) throw 'That question is gone; look again.';
      const answer = String(args?.answer ?? '').trim();
      if (!answer) throw 'Type an answer first.';
      profileBuilder.questions = profileBuilder.questions.filter(x => x.id !== q.id);
      fileProfile(q.file, `${q.label}: ${answer}`);
      return profileBuilderSnapshot() as T;
    }
    case 'profile_builder_skip_question':
      profileBuilder.questions = profileBuilder.questions.filter(x => x.id !== args?.id);
      return profileBuilderSnapshot() as T;
    case 'profile_builder_set_auto_learn':
      profileBuilder.autoLearn = !!args?.enabled;
      return profileBuilderSnapshot() as T;
    case 'mark_operator_profile_reviewed': {
      const item = operatorContext.freshness.find((entry) => entry.path === args?.path);
      if (!item || !item.exists) throw new Error(`Operator profile not found: ${String(args?.path ?? '')}`);
      const now = new Date().toISOString();
      const due = new Date(now);
      due.setUTCDate(due.getUTCDate() + item.reviewCadenceDays);
      Object.assign(item, {
        lastReviewed: now,
        status: 'fresh' as const,
        dueAt: due.toISOString().slice(0, 10),
        daysUntilDue: item.reviewCadenceDays,
      });
      return { ...item } as T;
    }
    case 'codex_helper_status': return { status: 'verified', version: 'codex-cli 0.153.4', verifiedVersions: ['codex-cli 0.146.0', 'codex-cli 0.153.4'], served: 'probed', message: 'Codex helpers run tool-free on codex-cli 0.153.4, a version Hollow verified with its local check.' } as T;
    // Reads that had no fixture, so browser preview threw on them (overnight
    // S5): empty answers, so each surface shows its empty state.
    case 'records_decisions': return JSON.parse(localStorage.getItem('mockDecisionRecords') ?? '[]') as T;
    case 'records_set_decision_status': {
      const rows = JSON.parse(localStorage.getItem('mockDecisionRecords') ?? '[]');
      const row = rows.find((r: {path: string}) => r.path === args?.path);
      if (!row) throw 'Decision not found';
      row.status = args?.status;
      localStorage.setItem('mockDecisionRecords', JSON.stringify(rows));
      return row as T;
    }
    // History & recovery's restore: a seeded
    // `mockRestoreRefusal` is the error it throws (the live-run guard).
    case 'restore_history_run': {
      const refusal = localStorage.getItem('mockRestoreRefusal');
      if (refusal) throw new Error(refusal);
      return { ...historyRun, id: String(args?.runId ?? historyRun.id) } as T;
    }
    // The chats a restart interrupted: none, unless a scenario seeds them.
    case 'list_interrupted_chats': return JSON.parse(localStorage.getItem('mockInterruptedChats') ?? '[]') as T;
    // P1-40: each asked-for chat still offered is queued (and leaves the
    // offer); an agent-started one is refused in words, as the backend does.
    // `mockResumedChats` keeps every queued key, for specs.
    case 'resume_interrupted_chats': {
      const asked = (args?.chats ?? []) as { sessionKey: string }[];
      const offered = JSON.parse(localStorage.getItem('mockInterruptedChats') ?? '[]') as { sessionKey: string; agentStarted?: boolean }[];
      const out = asked.map(({ sessionKey }) => {
        const chat = offered.find(c => c.sessionKey === sessionKey);
        if (!chat) return { sessionKey, queued: false, reason: 'No longer interrupted (it has a newer turn, is archived or too old).' };
        if (chat.agentStarted) return { sessionKey, queued: false, reason: 'An agent started this chat: open it and press Continue.' };
        return { sessionKey, queued: true, reason: null };
      });
      const queued = out.filter(o => o.queued).map(o => o.sessionKey);
      localStorage.setItem('mockInterruptedChats', JSON.stringify(offered.filter(c => !queued.includes(c.sessionKey))));
      localStorage.setItem('mockResumedChats', JSON.stringify([...JSON.parse(localStorage.getItem('mockResumedChats') ?? '[]') as string[], ...queued]));
      return out as T;
    }
    case 'meeting_recap': {
      if (localStorage.getItem('mockRecapError')) throw new Error(localStorage.getItem('mockRecapError')!);
      const meetingId = String(args?.meetingId);
      const subject = 'Finished planning meeting';
      if (!args?.recordingId && !args?.regenerate) return { kind: 'needsRecording', subject, day: String(args?.around).slice(0, 10), candidates: [{ id: 'recap-fixture', name: 'Planning recording', date: String(args?.around).slice(0, 10), duration: '12m' }] } as T;
      const path = 'workspaces/demo/outputs/recaps/planning.md';
      const markdown = '# Recap: Planning\n\n## Decisions\n\n- Review the plan [00:05]\n\n## Source\n\nPlanning recording · recap-fixture. The transcript stays in Plaud.';
      if (args?.expectedHash && args.expectedHash !== mockWrittenNotes.get(path)) throw new Error('The note changed.');
      mockWrittenNotes.set(path, markdown);
      return { kind: 'recap', meetingId, subject, start: args?.around, path, workspaceId: 'demo', markdown, contentHash: markdown, edited: false, kept: false, queued: args?.regenerate ? 0 : 1, dropped: 0 } as T;
    }
    // Prep packs (overnight 10-11 A4).
    case 'prep_pack': {
      const subject = 'Atlas pricing review';
      const markdown = `---\ntype: prep-pack\n---\n\n# Prep: ${subject}\n\nSunday 11 October, 2:00 PM · organized by Pat Smith · Teams\n\n## Who's coming\n\n- **Pat Smith**, CFO · [note](people/Pat%20Smith.md)\n  - You owe Pat Smith: Q3 numbers (due 2026-10-11)\n- **Ana Ruiz**\n\n## Related work\n\n- Workspace “Atlas”; next step: Finalize Q3 pricing; 3 open plan items [2]\n\n## Open questions\n\n- Is the Q3 floor price final?\n\n## Talking points\n\n- Bring the Q3 numbers you owe Pat [3]\n- Confirm the pricing next step before the board pack [2]\n\n## Sources\n\n1. Pat Smith (people note) · \`people/Pat Smith.md\`\n2. Atlas · \`workspaces/atlas/WORKSPACE.md\`\n3. Commitment · \`commitments/q3-numbers.md\`\n`;
      const path = 'workspaces/atlas/outputs/prep/2026-10-11-atlas-pricing-review.md';
      if (args?.expectedHash && args.expectedHash !== mockWrittenNotes.get(path)) throw new Error('The note changed.');
      mockWrittenNotes.set(path, markdown);
      return { meetingId: String(args?.meetingId ?? ''), subject, start: new Date(Date.now() + 3600_000).toISOString(), path, workspaceId: 'atlas', markdown, contentHash: markdown, edited: false, kept: !args?.regenerate } as T;
    }
    // Push-to-talk (overnight 10-11 B7): ready, so the mic can be seen; the
    // store types a sample sentence on a timer (no microphone in a browser).
    case 'dictation_status':
    case 'dictation_authorize':
      return (localStorage.getItem('mockDictationOff')
        ? { state: 'unavailable', locale: 'en-US', reason: 'On-device recognition isn’t available for en-US on this Mac, so dictation is off. Hollow never sends your voice to a network service.' }
        : { state: 'ready', locale: 'en-US', reason: null }) as T;
    case 'dictation_start': return Date.now() as T;
    case 'dictation_stop': return null as T;
    case 'open_dictation_settings': return undefined as T;
    // The triage inbox (overnight 10-11 B6): a few arrivals and suggestions
    // matched on workspace title words, so the view can be seen.
    case 'triage_arrivals': {
      const ago = (h: number) => new Date(Date.now() - h * 3600_000).toISOString();
      const state = JSON.parse(localStorage.getItem('mockTriageState') ?? '{"dismissed":{},"filed":{}}');
      return {
        imports: [
          { id: 'import:regional-parking-demand', title: 'Regional parking demand, Q3', path: 'library/imports/regional-parking-demand', files: ['library/imports/regional-parking-demand/index.md', 'library/imports/regional-parking-demand/occupancy.md'], fileCount: 2, at: ago(3), excerpt: 'Occupancy by region for the quarter, with the September close and the operations review figures by lot.' },
          { id: 'import:hiring-benchmarks', title: 'Hiring benchmarks for float supervisors', path: 'library/imports/hiring-benchmarks', files: ['library/imports/hiring-benchmarks/index.md'], fileCount: 1, at: ago(30), excerpt: 'Pay bands and time-to-fill for float supervisors, for the Q4 hiring plan.' },
        ],
        tasks: [
          { id: 'task:tasks/Confirm Q3 lot counts with Dana.md', path: 'tasks/Confirm Q3 lot counts with Dana.md', title: 'Confirm Q3 lot counts with Dana', from: 'Notion', at: ago(5), project: 'Q3 regional operations review', due: null },
        ],
        state, unavailable: [],
      } as T;
    }
    case 'triage_suggest': {
      const list = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string; title: string; kind?: string; status: string }[];
      const out: Record<string, unknown> = {};
      for (const item of (args?.items ?? []) as { id: string; text: string }[]) {
        const text = item.text.toLowerCase();
        let best: { w: { id: string; title: string }; hits: string[] } | null = null;
        for (const w of list.filter(x => !x.kind && x.status !== 'archived')) {
          const hits = w.title.toLowerCase().split(/[^a-z0-9]+/).filter(t => t.length >= 5 && text.includes(t));
          if (hits.length && (!best || hits.length > best.hits.length)) best = { w, hits };
        }
        if (best) out[item.id] = { workspaceId: best.w.id, title: best.w.title, score: Math.min(0.9, 0.3 + 0.2 * best.hits.length), reasons: text.includes(best.w.title.toLowerCase()) ? [`names the workspace ${best.w.title}`] : [`shared terms: ${best.hits.slice(0, 3).join(', ')}`] };
      }
      return out as T;
    }
    case 'triage_mark': {
      const state = JSON.parse(localStorage.getItem('mockTriageState') ?? '{"dismissed":{},"filed":{}}');
      const id = String(args?.id); const at = new Date().toISOString();
      state.tasked ??= {};
      delete state.dismissed[id]; delete state.filed[id]; delete state.tasked[id];
      if (args?.action === 'task') state.tasked[id] = { task: String(args?.taskRef), at };
      if (args?.action === 'dismiss') state.dismissed[id] = at;
      if (args?.action === 'file') state.filed[id] = { workspaceId: String(args?.workspaceId), at };
      localStorage.setItem('mockTriageState', JSON.stringify(state));
      return state as T;
    }
    // Today (overnight 10-11 A3): a written brief, so the view can be seen.
    case 'today_brief_cached':
    case 'today_brief': {
      const date = String(args?.date ?? '');
      const brief = { date, generatedAt: new Date().toISOString(), path: `workspaces/main/outputs/today/${date}.md`,
        paragraph: 'Three meetings today; the one that matters is the 2:00 Atlas pricing review with Pat, where the Q3 numbers you owe him are still open. Two tasks slipped past due, and last night\'s lanes merged four pull requests.',
        recommendations: [
          { title: 'Prepare the Atlas pricing review', why: '2:00 PM with Pat; the Q3 numbers are open', ask: 'Prepare me for the 2:00 Atlas pricing review with Pat' },
          { title: 'Draft the Q3 numbers for Pat', why: 'You owe them to him, due today', ask: 'Draft the Q3 numbers summary I owe Pat, from the Atlas workspace' },
          { title: 'Close the TipFlow push-file task', why: 'Overdue since Tuesday', ask: 'Start a chat in tipflow to reconcile last week\'s push file' },
        ] };
      return (command === 'today_brief' ? brief : { brief, firstName: 'Alex' }) as T;
    }
    // Extraction model (overnight A11), per agent, kept in localStorage.
    case 'get_extraction_settings': {
      const chosen = localStorage.getItem('mockExtractionModel') ?? '';
      return { agent: 'claude', chosen, asks: chosen || 'claude-haiku-5-5', retitleChats: localStorage.getItem('mockRetitleChats') === '1' } as T;
    }
    case 'get_learning_settings':
      return { suggestions: localStorage.getItem('mockLearningSuggestions') ?? 'important' } as T;
    case 'set_learning_suggestions': {
      const mode = String(args?.mode ?? '');
      if (!['off', 'important', 'all'].includes(mode)) throw new Error(`unknown learning suggestions \`${mode}\` (off, important or all)`);
      localStorage.setItem('mockLearningSuggestions', mode);
      return { suggestions: mode } as T;
    }
    case 'set_chat_retitle': {
      localStorage.setItem('mockRetitleChats', args?.enabled ? '1' : '0');
      const chosen = localStorage.getItem('mockExtractionModel') ?? '';
      return { agent: 'claude', chosen, asks: chosen || 'claude-haiku-5-5', retitleChats: !!args?.enabled } as T;
    }
    case 'set_extraction_settings': {
      const model = String(args?.model ?? '').trim();
      if (/\s/.test(model) || model.length > 200) throw new Error('A model id has no spaces and is at most 200 characters. Nothing changed.');
      localStorage.setItem('mockExtractionModel', model);
      return { agent: String(args?.agent ?? 'claude'), chosen: model, asks: model || 'claude-haiku-5-5', retitleChats: localStorage.getItem('mockRetitleChats') === '1' } as T;
    }
    case 'agent_models': return { models: args?.agent === 'claude' ? [{id:'opus',label:'Opus 5.5'},{id:'fable',label:'Fable 5.1'},{id:'sonnet',label:'Sonnet 5.5'},{id:'haiku',label:'Haiku 5.5'},{id:'claude-sonnet-5',label:'Sonnet 5 · pinned'}] : args?.agent === 'codex' ? [{id:'gpt-6-sol',label:'GPT-6-Sol'},{id:'gpt-6-astra',label:'GPT-6-Astra'}] : args?.agent === 'grok' ? [{id:'grok-4.7',label:'grok-4.7 · default'},{id:'grok-4.6',label:'grok-4.6'}] : [{id:'agy-default',label:'Antigravity default'}], source: args?.agent === 'claude' ? 'Claude Code 2.1.296\'s model list' : 'Preview model catalog', warning: null, checkedAt: new Date().toISOString(), newestByFamily: args?.agent === 'claude' ? { opus: 'claude-opus-5-5', fable: 'claude-fable-5-1', sonnet: 'claude-sonnet-5-5', haiku: 'claude-haiku-5-5' } : {} } as T;
    // The CFO (COO rebuild L7): `mockCfoSettings` persists the policy and
    // `mockCfoAccounts` seeds the accounts Settings, Usage lists.
    case 'cfo_settings': return mockCfoSettings() as T;
    case 'cfo_view': return mockCfoView() as T;
    case 'cfo_save_settings': {
      // A patch: only the fields one panel owns change.
      const patch = (args?.patch ?? {}) as Record<string, unknown>;
      if (Number(patch.reservePercent) > 90) throw new Error('CFO_SETTINGS: the reserve is at most 90%');
      const next: Record<string, unknown> = { ...mockCfoSettings() };
      for (const key of ['allow', 'effortMode', 'reservePercent', 'projectBudgets', 'subagentBudget']) if (key in patch) next[key] = patch[key];
      localStorage.setItem('mockCfoSettings', JSON.stringify(next));
      return mockCfoView() as T;
    }
    case 'cfo_approve_extra_usage': {
      const hours = Number(args?.hours ?? 0);
      if (hours > 72) throw new Error('CFO_SETTINGS: approve extra usage for at most 72 hours at a time');
      const next: Record<string, unknown> = { ...mockCfoSettings() };
      if (hours > 0) next.extraUsageUntil = new Date(Date.now() + hours * 3600_000).toISOString(); else delete next.extraUsageUntil;
      localStorage.setItem('mockCfoSettings', JSON.stringify(next));
      return mockCfoView() as T;
    }
    case 'agent_cli_status': return mockCliRows(!!args?.refresh) as T;
    case 'update_agent_cli': return mockCliUpdate(String(args?.agent), !!args?.now) as T;
    case 'restore_agent_cli': return mockCliRestore(String(args?.agent)) as T;
    case 'cancel_agent_cli_update': { const row = mockCli.find(r => r.agent === args?.agent)!; row.state = 'idle'; row.message = null; row.tone = null; return { ...row } as T; }
    case 'set_agent_cli_policy': { const row = mockCli.find(r => r.agent === args?.agent)!; row.policy = String(args?.policy); return row.policy as T; }
    // The Mac mini runner (2026-11-01): `mockRemoteRunner` names one.
    case 'remote_runner': return (localStorage.getItem('mockRemoteRunner') ?? null) as T;
    // Overnight N1: the runner's state; `mockRunnerStatus` overrides it.
    case 'runner_status': return mockRunnerStatus(!!args?.fresh) as T;
    // Remote runners 2 (2026-11-03): name or turn off the runner, check a
    // typed host, record this Hollow as owner (`mockRunnerOwner`).
    case 'runner_set': {
      const host = typeof args?.host === 'string' && args.host.trim() ? args.host.trim() : null;
      if (host) localStorage.setItem('mockRemoteRunner', host); else localStorage.removeItem('mockRemoteRunner');
      mockRunnerChecked = null;
      return host as T;
    }
    case 'runner_check_host': {
      const host = String(args?.host ?? '').trim();
      if (!/^[A-Za-z0-9._@-]+$/.test(host) || host.startsWith('-')) return { ...mockRunnerStatus(false), host, state: 'error', message: `${host} is not an SSH host name.` } as T;
      if (host.startsWith('down')) return { ...mockRunnerStatus(false), host, state: 'unreachable', message: `${host} didn't answer: ssh: connect to host ${host} port 22: Operation timed out.` } as T;
      return { ...mockRunnerStatus(false), ...mockRunnerReady(), host, owner: mockOwner('none') } as T;
    }
    case 'runner_record_owner': localStorage.setItem('mockRunnerOwner', 'you'); return mockRunnerStatus(true) as T;
    // Remote runners 3 (2026-11-03): Set up a runner from scratch. A short
    // plan per path; Check finds the first steps done, Apply makes a step
    // done (`mockSetupDone`), Create needs the estimate it was shown.
    case 'runner_setup_plan': return mockSetupPlan(args?.params as import('./runnerSetup').RunnerSetupParams) as T;
    case 'runner_setup_check': {
      const plan = mockSetupPlan(args?.params as import('./runnerSetup').RunnerSetupParams);
      if (plan.problems.length) throw new Error(plan.problems.join(' '));
      return plan.steps.map((s, i) => mockSetupState(s.id, i < 1)) as T;
    }
    case 'runner_setup_apply': {
      const step = String(args?.step ?? '');
      const plan = mockSetupPlan(args?.params as import('./runnerSetup').RunnerSetupParams);
      const params = plan.params;
      if (plan.problems.length) throw new Error(plan.problems.join(' '));
      if (step === 'cloud') throw new Error('The machine is created with Create, after its monthly estimate.');
      mockSetupDone.add(step);
      if (step === 'use') localStorage.setItem('mockRemoteRunner', params.alias);
      return mockSetupState(step, true) as T;
    }
    case 'runner_cloud_estimate': {
      const plan = mockSetupPlan(args?.params as import('./runnerSetup').RunnerSetupParams);
      if (!plan.estimate) throw new Error('Pick a machine type from the list.');
      return plan.estimate as T;
    }
    case 'runner_cloud_create': {
      const plan = mockSetupPlan(args?.params as import('./runnerSetup').RunnerSetupParams);
      if (plan.problems.length) throw new Error(plan.problems.join(' '));
      if (plan.estimate?.totalCents !== Number(args?.estimateCents)) throw new Error("The estimate you confirmed isn't this machine's. Look at the estimate again, then Create.");
      mockSetupDone.add('cloud');
      return mockSetupState('cloud', true) as T;
    }
    // Phase 3 (2026-11-02): a vault workspace's own Runs on, `mockWorkspaceRunsOn`.
    case 'workspace_runs_on': return ((JSON.parse(localStorage.getItem('mockWorkspaceRunsOn') ?? '{}') as Record<string, string>)[String(args?.id)] ?? null) as T;
    case 'workspace_set_runs_on': {
      const all = JSON.parse(localStorage.getItem('mockWorkspaceRunsOn') ?? '{}') as Record<string, string>;
      if (args?.runsOn) all[String(args.id)] = String(args.runsOn); else delete all[String(args?.id)];
      localStorage.setItem('mockWorkspaceRunsOn', JSON.stringify(all));
      return ((args?.runsOn as string | null) ?? null) as T;
    }
    // Remote runners (2026-11-03): `mockRunnerAccess` (per workspace) and
    // `mockRunnerGithub` (the GitHub source and whether a token is saved).
    case 'runner_access_get': return ((JSON.parse(localStorage.getItem('mockRunnerAccess') ?? '{}') as Record<string, unknown>)[String(args?.id)] ?? { github: false, cloud: 'off' }) as T;
    case 'runner_access_set': {
      const all = JSON.parse(localStorage.getItem('mockRunnerAccess') ?? '{}') as Record<string, unknown>;
      const access = { ...(args?.access as Record<string, unknown>) };
      if (access.cloud !== 'service-account') delete access.serviceAccount;
      all[String(args?.id)] = access;
      localStorage.setItem('mockRunnerAccess', JSON.stringify(all));
      return access as T;
    }
    case 'runner_github_status': return mockRunnerGithub() as T;
    case 'runner_github_save_token': {
      const token = String(args?.token ?? '').trim();
      if (!/^(github_pat_|ghp_)[A-Za-z0-9_]+$/.test(token)) throw new Error("That doesn't look like a GitHub personal access token (it starts with github_pat_ for a fine-grained one).");
      return mockRunnerGithub({ tokenSaved: true }) as T;
    }
    case 'runner_github_clear_token': return mockRunnerGithub({ tokenSaved: false }) as T;
    case 'runner_github_set_source': return mockRunnerGithub({ source: args?.source === 'gh-login' ? 'gh-login' : 'runner-token' }) as T;
    // 1Password secrets (parity row 7.5): `mockOnePassword` (the switch and the references; off by default).
    case 'onepassword_status': return mockOnePassword() as T;
    case 'onepassword_check': {
      const on = mockOnePassword();
      return (on.enabled ? mockOnePassword({}, { state: 'ready', message: '1Password answered: the command line tool is signed in.', version: '2.31.1' }) : on) as T;
    }
    case 'onepassword_set_enabled': return mockOnePassword({ enabled: Boolean(args?.enabled) }) as T;
    case 'onepassword_set_reference': {
      const reference = String(args?.reference ?? '').trim();
      if (!/^op:\/\/[^/\s][^/]*\/[^/]+\/[^/]+(\/[^/]+)?$/.test(reference)) throw new Error("That isn't a 1Password reference. It looks like op://Vault/Item/field.");
      return mockOnePassword({}, undefined, { [String(args?.key)]: reference }) as T;
    }
    case 'onepassword_clear_reference': return mockOnePassword({}, undefined, { [String(args?.key)]: null }) as T;
    case 'get_agent_settings': return JSON.parse(localStorage.getItem('mockAgentSettings') ?? '{"defaultAgent":null,"models":{}}') as T;
    case 'set_agent_settings': localStorage.setItem('mockAgentSettings', JSON.stringify(args?.settings)); return args?.settings as T;
    case 'computer_status': return { enabled: true, sandboxEnabled: true, thisMacEnabled: false, pinned: false, target: null, version: null, brokerListening: true, liveSessions: 0 } as T;
    case 'computer_stop_all': return 0 as T;
    case 'computer_doctor': return [
      { id: 'switch', label: 'Computer control', state: 'ok', line: 'On.', fix: null },
      { id: 'broker', label: 'Hollow’s desktop link', state: 'ok', line: 'Listening.', fix: null },
      { id: 'pin', label: 'Cua Driver', state: 'fail', line: 'Not set up: no pinned version.', fix: 'Hollow never installs the Driver itself. Put computer-engine.json in Hollow’s support folder.' },
      { id: 'space', label: 'Free disk space', state: 'ok', line: '120 GB free.', fix: null },
      { id: 'driver-permissions', label: 'Driver permissions', state: 'unknown', line: 'Not checked: Hollow can read what the Driver was granted only once it runs.', fix: 'In System Settings, Privacy and Security, give CuaDriver Accessibility and Screen Recording.' },
    ] as T;
    case 'computer_chat_state': return mockComputerChat() as T;
    case 'computer_chat_frame': return (mockComputerChat().frameId == null ? null : MOCK_FRAME) as T;
    case 'computer_chat_control': { const a = String(args?.action); mockComputerHeld = a === 'takeover' ? 'takeover' : a === 'pause' ? 'pause' : null; if (a === 'stop') mockComputerEntries = []; return mockComputerChat() as T; }
    case 'computer_denied_apps': return mockDeniedApps() as T;
    case 'host_scope_agent_denied': return mockAgentDenied as T;
    case 'host_scope_remove_agent_deny': mockAgentDenied = mockAgentDenied.filter(e => e.pattern !== String(args?.pattern)); return mockAgentDenied as T;
    case 'computer_set_app_allowed': mockDeniedAllowed[String(args?.label)] = args?.allowed === true; return mockDeniedApps() as T;
    case 'agent_descriptors': return ['claude', 'codex', 'antigravity', 'grok'].map(id => ({ id, label: id === 'claude' ? 'Claude' : id === 'codex' ? 'Codex' : id === 'grok' ? 'Grok' : 'Antigravity', available: id !== 'antigravity', capabilities: { reportsContext: true, modelSelection: true, sessionResume: id !== 'antigravity', toolRestrictions: id === 'claude', ownedMcp: id === 'claude' || id === 'codex', structuredOutput: id !== 'antigravity', steer: id === 'claude' || id === 'codex', interrupt: true, fork: false, rollback: false, queue: true, pendingSurvivesRestart: true, unsupportedReasons: { ...(id === 'claude' || id === 'codex' ? {} : { steer: `${(id === 'claude' ? 'Claude' : id === 'codex' ? 'Codex' : id === 'grok' ? 'Grok' : 'Antigravity')} can't take a message mid-turn. Queue it for after, or Stop and send restarts it with your message.` }), fork: `Hollow can't branch a ${(id === 'claude' ? 'Claude' : id === 'codex' ? 'Codex' : id === 'grok' ? 'Grok' : 'Antigravity')} conversation yet: the adapter doesn't wire a session fork.`, rollback: `Hollow can't rewind a ${(id === 'claude' ? 'Claude' : id === 'codex' ? 'Codex' : id === 'grok' ? 'Grok' : 'Antigravity')} conversation yet: the adapter doesn't wire a session rollback.` }, effortLevels: ['low', 'medium', 'high', 'xhigh', 'max'], unavailableModes: {} } })) as T;
    case 'agent_available': return ['claude', 'codex'] as T;
    // A chat's file link takes the first place that holds the file
    // (2026-11-04). Every absolute path exists here, as its preview read
    // does, unless `mockMissingFiles` (path endings) says it doesn't.
    case 'chat_locate_file': {
      const missing = JSON.parse(localStorage.getItem('mockMissingFiles') ?? '[]') as string[];
      const paths = (args?.paths ?? []) as string[];
      (window as unknown as { __locateCalls?: string[][] }).__locateCalls = [...((window as unknown as { __locateCalls?: string[][] }).__locateCalls ?? []), paths];
      return (paths.find(p => p.startsWith('/') && !missing.some(end => p.endsWith(end))) ?? null) as T;
    }
    // The last try for a link no folder of the chat holds (2026-11-10): what
    // `mockFindFiles` (tail → absolute paths) says the vault holds, else none.
    case 'chat_find_file': {
      const found = JSON.parse(localStorage.getItem('mockFindFiles') ?? '{}') as Record<string, string[]>;
      (window as unknown as { __findCalls?: unknown[] }).__findCalls = [...((window as unknown as { __findCalls?: unknown[] }).__findCalls ?? []), args];
      return (found[String(args?.tail ?? '')] ?? []) as T;
    }
    // The side panel's Files tab reads what the agent wrote.
    case 'chat_preview_file': {
      const fixtures = JSON.parse(localStorage.getItem('mockPreviewFiles') ?? '{}') as Record<string, { text: string; delay?: number }>;
      const fixture = fixtures[String(args?.path ?? '')];
      if (fixture) {
        if (fixture.delay) await new Promise(resolve => setTimeout(resolve, fixture.delay));
        return { kind: 'text', text: fixture.text, dataUrl: null, size: fixture.text.length, modifiedMs: null, truncated: false } as T;
      }
      // A vault file by its absolute path (a project's notes and Results in
      // the Files tab, COO rebuild L4): the mock workspaces' own text. An
      // image there is still an image (Library's Results thumbnails).
      const vaultRel = String(args?.path ?? '').startsWith(`${setupStatus.vaultPath}/workspaces/`) && !/\.(png|jpe?g|gif|webp)$/i.test(String(args?.path))
        ? String(args?.path).slice(setupStatus.vaultPath!.length + 1) : null;
      if (vaultRel) {
        const note = await mockInvoke<{ content: string }>('read_work_file', { path: vaultRel });
        return { kind: 'text', text: note.content, dataUrl: null, size: note.content.length, modifiedMs: null, truncated: false } as T;
      }
      const name = String(args?.path ?? '').split('/').pop() ?? 'file';
      // `mockPreviewRefusal` (a path-ending -> error map in localStorage)
      // rejects with that plain string, as Tauri does (file-load failures).
      const refused = Object.entries(JSON.parse(localStorage.getItem('mockPreviewRefusal') ?? '{}') as Record<string, string>)
        .find(([end]) => String(args?.path ?? '').endsWith(end));
      if (refused) throw refused[1];
      if (/\.(png|jpe?g|gif|webp)$/i.test(name)) {
        const previews = JSON.parse(localStorage.getItem('mockImagePreviews') ?? '{}') as Record<string, string | null>;
        const path = String(args?.path ?? '');
        if (path in previews && previews[path] === null) throw new Error('Image unavailable');
        return { kind: 'image', text: null, dataUrl: previews[path] ?? 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=', size: 68, modifiedMs: Date.now(), truncated: false } as T;
      }
      if (/\.(mmd|mermaid)$/i.test(name)) {
        const text = 'flowchart LR\nstart[Start] --> finish[Finish]';
        return { kind: 'text', text, dataUrl: null, size: text.length, modifiedMs: Date.now(), truncated: false } as T;
      }
      // Videos play through media_grant (the smoke routes /__media__/*).
      if (/\.pdf$/i.test(name)) return { kind: 'pdf', text: null, dataUrl: null, size: 4096, modifiedMs: Date.now(), truncated: false } as T;
      if (/\.(mp4|m4v|mov)$/i.test(name)) return { kind: 'video', text: null, dataUrl: null, size: 48_213, modifiedMs: Date.now(), truncated: false } as T;
      // Office files are binary here; office_preview shows their text (B1b).
      if (/\.(docx|xlsx|pptx)$/i.test(name)) return { kind: 'binary', text: null, dataUrl: null, size: 24_576, modifiedMs: Date.now(), truncated: false } as T;
      const text = `# ${name.replace(/\.md$/, '')}\n\nMock file written by the agent.`;
      return { kind: 'text', text, dataUrl: null, size: text.length, modifiedMs: Date.now(), truncated: false } as T;
    }
    // A video token (media.rs). `mockMediaRefusal` (a path -> code map in
    // localStorage) makes a grant fail with that code, as the backend would.
    // A Word, Excel or PowerPoint file's text (office_preview.rs, B1b): a
    // sample per type by extension. `mockOfficePreviewError` (a path-ending
    // -> message map in localStorage) rejects with that message.
    case 'office_preview': {
      const path = String(args?.path ?? '');
      const refused = Object.entries(JSON.parse(localStorage.getItem('mockOfficePreviewError') ?? '{}') as Record<string, string>)
        .find(([end]) => path.endsWith(end));
      if (refused) throw refused[1];
      const empty = { title: '', paragraphs: [] as string[], columns: [] as string[], rows: [] as { number: number; cells: string[] }[] };
      if (/\.docx$/i.test(path)) {
        return { kind: 'word', truncated: [], parts: [{ ...empty, paragraphs: ['Quarterly report', 'Revenue grew 12% on the quarter, led by the North region.', 'Next steps: hire two engineers and open the Austin office.'] }] } as T;
      }
      if (/\.xlsx$/i.test(path)) {
        return { kind: 'excel', truncated: ['sheet "Q3": first 200 rows with cells only'], parts: [
          { ...empty, title: 'Q3', columns: ['A', 'B', 'C'], rows: [
            { number: 1, cells: ['Region', 'Revenue', 'Margin'] },
            { number: 2, cells: ['North', '1200', '0.18'] },
            { number: 3, cells: ['South', '950', '0.21'] },
            { number: 4, cells: ['Total', '2150', '0.195'], formulas: ['', 'SUM(B2:B3)', 'AVERAGE(C2:C3)'] },
          ] },
          { ...empty, title: 'Notes', columns: ['A'], rows: [{ number: 1, cells: ['Figures in thousands'] }] },
        ] } as T;
      }
      if (/\.pptx$/i.test(path)) {
        return { kind: 'powerpoint', truncated: [], parts: [
          { ...empty, title: 'Slide 1', paragraphs: ['Launch plan', 'Owner: Sam'] },
          { ...empty, title: 'Slide 2', paragraphs: ['Timeline', 'Beta in March, launch in May'] },
        ] } as T;
      }
      throw 'This file is not a Word, Excel or PowerPoint file.';
    }
    case 'pdf_preview_page': {
      const path = String(args?.path ?? '');
      const page = Number(args?.page ?? 0);
      const fixture = JSON.parse(localStorage.getItem('mockPdfPreviews') ?? '{}')[path] ?? {};
      if (fixture.delay) await new Promise(resolve => setTimeout(resolve, fixture.delay));
      if (fixture.error) throw new Error(fixture.error);
      const pages = fixture.pages ?? 2;
      if (page < 0 || page >= pages) throw new Error('That PDF page is no longer available. Retry from the first page.');
      return { page, pages, width: 612, height: 792, dataUrl: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=', text: `${path.split('/').pop()}, page ${page + 1}`, textTruncated: false } as T;
    }
    case 'media_grant': {
      const path = String(args?.path ?? '');
      const refusals = JSON.parse(localStorage.getItem('mockMediaRefusal') ?? '{}') as Record<string, string>;
      if (refusals[path]) throw new Error(refusals[path]);
      const n = Number(localStorage.getItem('mockMediaGrants') ?? '0') + 1;
      localStorage.setItem('mockMediaGrants', String(n));
      const sizes = JSON.parse(localStorage.getItem('mockMediaSizes') ?? '{}') as Record<string, number>;
      return { token: `${n.toString(16).padStart(32, '0')}`, mime: 'video/mp4', size: sizes[path] ?? 48_213, modifiedMs: 1_760_000_000_000 } as T;
    }
    // A poster frame. The backend answers a JPEG data URL; any image works
    // for the <img> here, so this is a 1×1 PNG.
    case 'media_poster': {
      const refusals = JSON.parse(localStorage.getItem('mockMediaRefusal') ?? '{}') as Record<string, string>;
      if (refusals[String(args?.path ?? '')]) throw new Error(refusals[String(args?.path)]);
      return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6LJ8AAAAASUVORK5CYII=' as T;
    }
    case 'media_release': {
      const released = JSON.parse(localStorage.getItem('mockMediaReleased') ?? '[]') as string[];
      localStorage.setItem('mockMediaReleased', JSON.stringify([...released, String(args?.token)]));
      return undefined as T;
    }
    // Claude Code's own MCP-server and plugin lists (`claude mcp list`,
    // `claude plugin list`), as the composer's Commands menu runs them.
    case 'claude_admin': {
      const sub = Array.isArray(args?.args) ? String(args.args[0]) : '';
      return (sub === 'mcp' ? 'notion: npx -y @notionhq/notion-mcp-server - ✓ Connected' : 'No plugins installed.') as T;
    }
    case 'chat_preflight': {
      const cwd = typeof args?.cwd === 'string' ? args.cwd : null;
      const missing = localStorage.getItem('mockMissingChatCwd');
      if (cwd && cwd === missing) {
        // One recovery per configured stale path. The next explicit send is a
        // fresh vault turn and should pass preflight.
        localStorage.removeItem('mockMissingChatCwd');
        return cwd as T;
      }
      return null as T;
    }
    // Useful and Not useful under a reply (overnight 11-12, C2): saved votes
    // in `mockTurnFeedback` ({ [runId]: TurnFeedback }); `mockFeedbackError`
    // refuses a save with that string, as the backend's FEEDBACK_* codes.
    case 'list_feedback': {
      const saved = JSON.parse(localStorage.getItem('mockTurnFeedback') ?? '{}') as Record<string, import('./types').TurnFeedback>;
      const ids = (args?.runIds as string[] | null | undefined) ?? null;
      return Object.values(saved).filter(row => !ids || ids.includes(row.runId)) as T;
    }
    case 'record_turn_feedback': {
      const refuse = localStorage.getItem('mockFeedbackError');
      if (refuse) throw refuse;
      const input = args?.input as import('./types').TurnFeedbackInput;
      const saved = JSON.parse(localStorage.getItem('mockTurnFeedback') ?? '{}') as Record<string, import('./types').TurnFeedback>;
      const before = saved[input.runId];
      const row: import('./types').TurnFeedback = {
        id: before?.id ?? Object.keys(saved).length + 1, sessionKey: agentSession.key, sessionTitle: null, turnOrd: 1, runId: input.runId,
        vote: input.vote, reason: input.reason ?? (before?.vote === input.vote ? before.reason : null), agent: 'claude', model: null,
        snapshot: { asked: '', answer: '', answerTruncated: false, tools: [], receiptId: null, receiptStatus: null, verificationState: null, recall: [], turnStatus: 'success' },
        createdAt: before?.createdAt ?? NOW, updatedAt: new Date().toISOString(),
      };
      saved[input.runId] = row;
      localStorage.setItem('mockTurnFeedback', JSON.stringify(saved));
      return row as T;
    }
    // Side chat (7.6): one fixture side chat per parent; a send is recorded for the smoke spec.
    case 'side_chat_open': {
      const parent = String(args?.parentKey ?? '');
      const key = `${parent}-side`;
      return { key, title: 'Side chat', agent: 'claude', harnessSessionId: null, cwd: '/Users/fixture/vault/workspaces/alpha-1', createdAt: '2026-11-20T12:00:00Z', lastActivityAt: '2026-11-20T12:00:00Z', endedAt: null, turnCount: 0, parentKey: parent, forkKind: 'side', adopted: false, stints: [], resumable: 'none' } as T;
    }
    case 'side_chat_list': return [] as T;
    case 'side_chat_send': {
      localStorage.setItem('mockLastSideChatArgs', JSON.stringify(args));
      return { sessionKey: String(args?.key ?? ''), queueId: ++mockQueueSeq, queuePosition: 1, phase: 'running' } as T;
    }
    case 'chat_send': {
      localStorage.setItem('mockChatSendCalled', 'true');
      localStorage.setItem('mockLastChatArgs', JSON.stringify(args));
      localStorage.setItem('mockLastChatPrompt', String(args?.prompt ?? ''));
      // A send that fails before anything is queued (Paperclip PR 13's
      // answer-preservation smoke test). Tauri rejects with the string.
      const sendError = localStorage.getItem('mockChatSendError');
      if (sendError) throw sendError;
      // Mid-turn steering: a send held for after the running turn.
      if (args?.after && localStorage.getItem('mockChatHold')) {
        const held: SendAccepted = { sessionKey: String(args?.sessionKey ?? MOCK_CHAT_SESSION_KEY), queueId: 0, queuePosition: 0, phase: 'queued', waitingFor: 'the running turn to end', heldId: `held-mock-${++mockQueueSeq}` };
        localStorage.setItem('mockLastHeldId', held.heldId!);
        return held as T;
      }
      // Paperclip PR 7b: the send enqueues and returns at once. The harness
      // emits no events of its own, so the turn stays queued until a test
      // delivers runStarted / cancelled / done through the event bridge.
      // `mockChatMintKeys` mints a key per new conversation, as the backend
      // does (build programme PR B4's parallel-conversation smoke test).
      const minted = localStorage.getItem('mockChatMintKeys') ? `s-20260927-${String(mockQueueSeq + 1).padStart(8, '0')}` : MOCK_CHAT_SESSION_KEY;
      const sessionKey = typeof args?.sessionKey === 'string' && args.sessionKey ? args.sessionKey : minted;
      const waitingFor = localStorage.getItem('mockChatWaitingFor');
      const queueId = ++mockQueueSeq;
      mockChatRows.set(queueId, {
        id: queueId, source: 'chat', sourceId: sessionKey, label: 'Chat', detail: String(args?.prompt ?? '').slice(0, 8000),
        reason: 'chat', status: 'queued', priority: 0, position: 1, attempt: 0, coalescedCount: 0,
        requestedAt: NOW, claimedAt: null, finishedAt: null, historyRunId: null, error: null,
        waitingSince: waitingFor ? NOW : null, waitingHolder: waitingFor, heldReason: null, cancelRequested: false, waitingReason: null,
        // Like the backend: a queued chat nothing holds back reads Working.
        ...(waitingFor ? {} : { working: true }),
      });
      const accepted: SendAccepted = { sessionKey, queueId, queuePosition: 1, phase: 'queued', waitingFor };
      return accepted as T;
    }
    // The harness has no process to stop and emits no events; a test that
    // needs the turn to end delivers cancelled + done itself.
    case 'chat_cancel':
      localStorage.setItem('mockCancelledChatKey', String(args?.sessionKey));
      return undefined as T;
    // P1a: the harness has no running turn to pause; it records the ask, and
    // a test that needs the turn to end delivers paused + done itself.
    case 'chat_pause':
      localStorage.setItem('mockPausedChatKey', String(args?.sessionKey));
      return true as T;
    // P1b, P1c: `mockPausedChat` is the one paused chat; Pause all asks
    // nothing (no running turn here); its resume time is
    // `mockPausedResumeAt` (unix seconds).
    case 'chat_pause_all':
      localStorage.setItem('mockPauseAll', 'true');
      return [] as T;
    case 'chat_resume_all': {
      const key = localStorage.getItem('mockPausedChat');
      if (key) localStorage.removeItem('mockPausedChat');
      return { resumed: key ? [key] : [], refused: [], skipped: [] } as T;
    }
    case 'chat_paused_list': {
      const key = localStorage.getItem('mockPausedChat');
      const at = Number(localStorage.getItem('mockPausedResumeAt'));
      return (key ? [{ sessionKey: key, nativeSessionId: 'native-mock', agent: 'claude', pausedAt: Math.floor(Date.now() / 1000), note: 'Was using Bash.', reason: 'user', resumeAt: at > 0 ? at : null }] : []) as T;
    }
    case 'chat_set_resume_at': {
      const key = String(args?.sessionKey ?? '');
      if (localStorage.getItem('mockPausedChat') !== key) throw new Error('This chat isn’t paused. Nothing changed.');
      const at = typeof args?.at === 'number' ? args.at : null;
      if (at === null) localStorage.removeItem('mockPausedResumeAt'); else localStorage.setItem('mockPausedResumeAt', String(at));
      return { sessionKey: key, nativeSessionId: 'native-mock', agent: 'claude', pausedAt: Math.floor(Date.now() / 1000), note: 'Was using Bash.', reason: 'user', resumeAt: at } as T;
    }
    case 'chat_resume_paused': {
      const sessionKey = String(args?.sessionKey ?? '');
      localStorage.setItem('mockResumedChatKey', sessionKey);
      if (localStorage.getItem('mockPausedChat') === sessionKey) localStorage.removeItem('mockPausedChat');
      const accepted: SendAccepted = { sessionKey, queueId: Date.now(), queuePosition: 1, phase: 'queued', waitingFor: null };
      return accepted as T;
    }
    // Mid-turn steering. `mockSteerRefuse` (a STEER_* code) makes the steer
    // refuse; the harness emits the turn's events itself.
    case 'chat_steer': {
      localStorage.setItem('mockLastSteer', JSON.stringify(args));
      const refuse = localStorage.getItem('mockSteerRefuse');
      if (refuse) throw refuse;
      const steer = { id: String(args?.clientId ?? 'steer-mock'), text: String(args?.text ?? '').trim(), at: new Date().toISOString(), afterTools: 0, state: 'sent' };
      const event = { type: 'steer', sessionKey: String(args?.sessionKey ?? ''), steer, conversationKey: String(args?.sessionKey ?? '') };
      return event as T;
    }
    case 'chat_steer_support': {
      // The notes in the backend's words (`steering::support_table`).
      const native = (localStorage.getItem('mockSteerNative') ?? 'claude').split(',');
      const labels: Record<string, string> = { claude: 'Claude', codex: 'Codex', grok: 'Grok', antigravity: 'Antigravity' };
      return Object.entries(labels).map(([agent, label]) => ({
        agent,
        support: native.includes(agent) ? 'native' : 'none',
        note: native.includes(agent) ? `${label} takes your message at its next step, in this turn.` : `${label} can't take a message mid-turn. Queue it for after, or Stop and send restarts it with your message.`,
      })) as T;
    }
    case 'chat_ack_held_failed': return true as T;
    case 'chat_cancel_held': {
      localStorage.setItem('mockLastCancelHeld', JSON.stringify(args));
      return true as T;
    }
    case 'chat_save_attachment': {
      localStorage.setItem('mockAttachmentCwd', String(args?.cwd ?? 'vault'));
      // C5: a named file keeps a cleaned stem and extension, as the backend's.
      const name = String(args?.name ?? '');
      if (!name) return '/mock/vault/.hollow-attachments/paste.png' as T;
      localStorage.setItem('mockAttachmentName', name);
      const dot = name.lastIndexOf('.');
      const ext = dot > 0 && /^[a-z0-9]{1,10}$/i.test(name.slice(dot + 1)) ? `.${name.slice(dot + 1).toLowerCase()}` : '';
      const stem = (ext ? name.slice(0, dot) : name).replace(/[^A-Za-z0-9._-]+/g, '-').replace(/^[.-]+|[.-]+$/g, '').slice(0, 48) || 'file';
      return `/mock/vault/.hollow-attachments/${stem}-1${ext}` as T;
    }
    // Hollow's own Plaud sign-in (Recordings) follows `mockPlaudOff`, as Recordings' list does (P1-20).
    case 'plaud_connected':
    case 'plaud_ping': return !localStorage.getItem('mockPlaudOff') as T;
    case 'plaud_logout': localStorage.setItem('mockPlaudOff', '1'); return undefined as T;
    case 'plaud_login': localStorage.removeItem('mockPlaudOff'); return undefined as T;
    // The real command returns the login item's resulting state, which the
    // Startup switch shows (a NO_OP undefined left it with no aria-checked).
    case 'set_run_on_startup': localStorage.setItem('mockRunOnStartup', args?.enabled ? '1' : '0'); return Boolean(args?.enabled) as T;
    case 'run_on_startup_status': return (localStorage.getItem('mockRunOnStartup') !== '0') as T;
    case 'tasks_mcp_enabled': return true as T;
    case 'app_mcp_enabled': return true as T;
    case 'list_custom_themes': return JSON.parse(localStorage.getItem('mockCustomThemes') ?? '[]') as T;
    case 'read_custom_theme': {
      const themes = JSON.parse(localStorage.getItem('mockCustomThemes') ?? '[]') as import('./types').CustomTheme[];
      const theme = themes.find(t => t.name === args?.name);
      if (!theme) throw new Error("That theme isn't on this Mac");
      return theme as T;
    }
    // Settings PR 5: the theme sheet's Save and Delete (the height spec saves a theme).
    case 'write_custom_theme': {
      const themes = JSON.parse(localStorage.getItem('mockCustomThemes') ?? '[]') as Array<Record<string, unknown>>;
      const name = String(args?.name ?? 'New Theme');
      const source = String(args?.content ?? '');
      const pick = (key: string, fallback: string) => source.match(new RegExp(`^${key}\\s*=?\\s*"?(#[0-9a-fA-F]{3,8})`, 'm'))?.[1] ?? fallback;
      const saved = { name, path: `/tmp/hollow/themes/${name}.theme`, source, isDark: /^is_dark\s*=?\s*(true|yes)/m.test(source), accent: pick('accent', '#c8761a'), foreground: pick('foreground', '#1c1408'), background: pick('background', '#f4ecd8'), cursor: null, selectionForeground: null, selectionBackground: null, colors: [], success: null, warning: null, error: null };
      localStorage.setItem('mockCustomThemes', JSON.stringify([...themes.filter(t => t.name !== name), saved]));
      return saved as T;
    }
    case 'delete_custom_theme': {
      const themes = JSON.parse(localStorage.getItem('mockCustomThemes') ?? '[]') as Array<Record<string, unknown>>;
      localStorage.setItem('mockCustomThemes', JSON.stringify(themes.filter(t => t.name !== args?.name)));
      return undefined as T;
    }
    case 'themes_dir_path': return '/tmp/hollow/themes' as T;
    case 'mark_all_notifications_read': {
      const n = notifications.filter(row => !row.read).length;
      for (const row of notifications) row.read = true;
      return n as T;
    }
    case 'clear_notifications': {
      const n = notifications.length;
      notifications.length = 0;
      return n as T;
    }
    case 'clear_debug_logs':
    case 'run_nudge_scan':
    case 'run_heartbeat_now': return 0 as T;
    case 'set_workspace_source': return undefined as T;
    case 'artifact_put':
      throw new Error('artifact protocol unavailable in browser mock');
    // The backend fetch of a remote image (remote_image.rs). The harness never
    // touches the network: every URL comes back as the same 40×30 grey PNG,
    // and the last URL asked for is kept for the click-to-load smoke test.
    case 'fetch_remote_image':
      localStorage.setItem('mockLastRemoteImage', String(args?.url ?? ''));
      return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAKElEQVR4nO3NMQEAAAwCIPsnM5YxtgMKkB6JWCwWi8VisVgsFov/xgPORv0XHYdCmQAAAABJRU5ErkJggg==' as T;
    case 'parse_quick_add': return {
      title: String(args?.input ?? ''),
      priority: null,
      due: null,
      dueTime: null,
      durationMin: null,
      recurrence: null,
      project: null,
      labels: [],
    } as T;
    // Inline AI (overnight 10-11 B5): a canned suggestion after a short wait.
    case 'inline_ai': {
      const input = (args?.input ?? {}) as { action?: string; text?: string; instruction?: string };
      await new Promise(r => setTimeout(r, 700));
      const text = String(input.text ?? '');
      const out = input.action === 'shorten' ? text.split(/(?<=[.!?])\s/)[0]
        : input.action === 'fix' ? text.replace(/\bteh\b/g, 'the')
        : input.action === 'ask' && !text.trim() ? `| Owner | Due |\n| --- | --- |\n| Dana | Friday |`
        : `${text.replace(/\s+$/, '')} (clearer, with the September close called out)`;
      return { text: out } as T;
    }
    case 'inline_ai_cancel': return false as T;
    case 'cancel_logo_options': return false as T;
    case 'task_assist':
      return { suggestions: ['Outline the change', 'Check it with a test'], text: 'Outline the change' } as T;
    case 'create_task': {
      // Recorded so a smoke test can assert the idempotency key and origin
      // each create carries (Paperclip PR 11).
      try {
        const input = args?.input as { title?: string } | undefined;
        if (window.__mockFailCreateTaskOnce && input?.title === window.__mockFailCreateTaskOnce) {
          window.__mockFailCreateTaskOnce = undefined;
          throw 'mock create_task failure';
        }
        (window.__mockCreatedTasks ??= []).push(args?.input);
      } catch (e) {
        if (typeof e === 'string') throw e;
        /* non-browser */
      }
      // A create whose idempotency key an earlier create carried returns that
      // task (as vault_tasks does), across reloads.
      const key = (args?.input as { idempotencyKey?: string } | undefined)?.idempotencyKey;
      if (key) {
        const keys = JSON.parse(localStorage.getItem('mockTaskKeys') ?? '{}') as Record<string, string>;
        if (keys[key]) return { url: keys[key], id: keys[key].split('/').pop()!, existing: true } as T;
        const url = `vault://tasks/new-${Object.keys(keys).length + 1}.md`;
        keys[key] = url; localStorage.setItem('mockTaskKeys', JSON.stringify(keys));
        return { url, id: url.split('/').pop()!, existing: false } as T;
      }
      return { url: 'vault://tasks/new.md', id: 'new', existing: false } as T;
    }
    case 'update_task': {
      const input = (args?.input ?? {}) as Partial<Task> & { url?: string; expected?: string };
      const task = mockTaskList().find(candidate => candidate.url === input.url);
      if (!task) throw new Error(`Task not found: ${String(input.url ?? '')}`);
      // A stale compare-and-swap baseline. Thrown as a plain STRING on
      // purpose: Tauri rejects invoke() that way, so a caller reading
      // `e.message` shows "undefined"/"failed" here exactly as in the app.
      if (typeof input.expected === 'string' && input.expected && input.expected !== task.contentHash) {
        throw `TASK_CONFLICT: ${task.vaultPath} changed on disk after it was opened.`;
      }
      if (typeof input.due === 'string') task.due = input.due || null;
      if (typeof input.title === 'string' && input.title.trim()) task.title = input.title.trim();
      if (typeof input.priority === 'string' || input.priority === null) task.priority = (input.priority || null) as Task['priority'];
      if (typeof input.dueTime === 'string') task.dueTime = input.dueTime || null;
      if (typeof input.durationMin === 'number') task.durationMin = input.durationMin || null;
      task.contentHash += '+';
      return { url: task.url, contentHash: task.contentHash } as T;
    }
    case 'set_task_status':
    case 'set_task_priority': {
      const task = mockTaskList().find(candidate => candidate.url === args?.url);
      if (!task) throw new Error(`Task not found: ${String(args?.url ?? '')}`);
      if (localStorage.getItem('mockTaskWriteFail') === task.url) throw `TASK_CONFLICT: ${task.vaultPath} changed on disk after it was opened.`;
      // Compare-and-swap like vault_tasks (review F01): a stale baseline writes nothing.
      if (typeof args?.expected === 'string' && args.expected && args.expected !== task.contentHash) {
        throw `TASK_CONFLICT: ${task.vaultPath} changed on disk after it was opened.`;
      }
      // A recurring task completes into its next occurrence once (as vault_tasks does).
      if (command === 'set_task_status' && task.recurrence && /^(done|delivered|completed)$/i.test(String(args?.status ?? '')) && task.due) {
        const next = new Date(`${task.due.slice(0, 10)}T12:00:00`); next.setDate(next.getDate() + 7);
        task.due = `${next.getFullYear()}-${String(next.getMonth() + 1).padStart(2, '0')}-${String(next.getDate()).padStart(2, '0')}`;
      } else if (command === 'set_task_status') task.status = String(args?.status ?? '');
      else task.priority = (args?.priority ?? null) as Task['priority'];
      task.contentHash += '+';
      return task.contentHash as T;
    }
    // The side panel's browser: no native webview here, so the fixture keeps
    // each tab's address and logs every call (`mockBrowserLog`) for the specs.
    case 'browser_open': case 'browser_show': case 'browser_go': case 'browser_url': case 'browser_close': case 'browser_snapshot': case 'browser_inspect': case 'browser_page_text': case 'browser_grant_tab': case 'browser_revoke_tab': case 'browser_tab_grants': {
      const log = JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]') as { command: string; args: unknown }[];
      log.push({ command, args: args ?? {} });
      localStorage.setItem('mockBrowserLog', JSON.stringify(log.slice(-200)));
      const tabs = JSON.parse(localStorage.getItem('mockBrowserTabs') ?? '{}') as Record<string, string>;
      const id = String(args?.tabId ?? '');
      // A spec can make an address fail to open (`mockBrowserOpenFail` = a substring of it; next-level P15).
      const failOn = localStorage.getItem('mockBrowserOpenFail');
      if (command === 'browser_open' && failOn && String(args?.url ?? '').includes(failOn)) throw new Error('The page could not be opened: the server did not answer.');
      if (command === 'browser_open') { tabs[id] = String(args?.url ?? ''); localStorage.setItem('mockBrowserTabs', JSON.stringify(tabs)); }
      if (command === 'browser_close') { delete tabs[id]; localStorage.setItem('mockBrowserTabs', JSON.stringify(tabs)); }
      if (command === 'browser_url') { if (!(id in tabs)) throw new Error('That browser tab is closed.'); return tabs[id] as T; }
      // A 40×30 grey PNG, and a button under every point.
      if (command === 'browser_snapshot') return 'iVBORw0KGgoAAAANSUhEUgAAACgAAAAeCAIAAADRv8uKAAAAKElEQVR4nO3NMQEAAAwCIPsnM5YxtgMKkB6JWCwWi8VisVgsFov/xgPORv0XHYdCmQAAAABJRU5ErkJggg==' as T;
      // Repo ideas 10-27 #5: shared tabs live in `mockTabGrants`.
      if (command === 'browser_grant_tab' || command === 'browser_revoke_tab' || command === 'browser_tab_grants') {
        const grants = (JSON.parse(localStorage.getItem('mockTabGrants') ?? '[]') as import('./tabGrants').TabGrant[]).filter(g => command === 'browser_tab_grants' || (g.tabId !== id && (command === 'browser_revoke_tab' || g.sessionKey !== String(args?.sessionKey))));
        if (command === 'browser_grant_tab') grants.push({ tabId: id, sessionKey: String(args?.sessionKey ?? ''), title: String(args?.title ?? ''), grantedAt: new Date().toISOString() });
        if (command !== 'browser_tab_grants') localStorage.setItem('mockTabGrants', JSON.stringify(grants));
        return (command === 'browser_grant_tab' ? grants[grants.length - 1] : command === 'browser_tab_grants' ? grants : undefined) as T;
      }
      if (command === 'browser_page_text') return { url: tabs[id] ?? '', title: 'Fixture page', text: 'Start free trial\nPlan\tPrice\nTeam\t$12', truncated: false } as T;
      if (command === 'browser_inspect') {
        const points = (args?.points ?? []) as unknown[];
        return { url: tabs[id] ?? '', title: 'Fixture page', elements: points.map(() => ({ tag: 'button', id: '', classes: 'cta', text: 'Start free trial', label: '', href: '', selector: 'main > button.cta' })) } as T;
      }
      return undefined as T;
    }
    // Canvas C6 P1: the bridge refusal log; the harness keeps what it was
    // sent for the smoke tests to read (`__mockBridgeRefusals`).
    case 'record_canvas_bridge_refusal': {
      const w = window as unknown as { __mockBridgeRefusals?: unknown[] };
      (w.__mockBridgeRefusals ??= []).push(args?.refusal);
      return undefined as T;
    }
    // Companion devices (companion apps T0): nothing paired, no code open.
    case 'companion_status':
    case 'companion_revoke_device':
      return { devices: [], pairing: null, waiting: 0 } as T;
    case 'companion_pairing_start':
      return { code: 'K7M3-Q9TX', expiresAt: new Date(Date.now() + 600_000).toISOString() } as T;
    case 'companion_pairing_cancel':
      return undefined as T;
    // The phone chat API's Settings section (2026-11-04): off, nothing paired,
    // Tailscale running and unpublished (`mockPhoneAccess.ts`).
    case 'phone_access_status':
    case 'phone_access_set_enabled':
    case 'phone_access_pairing_start':
    case 'phone_access_pairing_cancel':
    case 'phone_access_revoke':
    case 'phone_access_machine_pairing_start':
    case 'phone_access_machine_pairing_cancel':
    case 'phone_access_approvals_start':
    case 'phone_access_approvals_cancel':
    case 'phone_access_approvals_off':
    case 'phone_access_approvals_wait':
    case 'phone_access_publish':
    case 'phone_access_unpublish':
    case 'phone_push_import_key':
    case 'phone_push_remove_key':
      return mockPhoneAccess(command, args) as T;
    case 'phone_push_test':
      return mockPhonePushTest() as T;
    // Keep Hollow running with the lid closed (2026-11-05): off, on power,
    // following the iPhone app's mock (`mockPhoneLid.ts`).
    case 'phone_lid_status':
    case 'phone_lid_set':
      return mockPhoneLid(command, args) as T;
    // Companion backend (W1): none set up, unless a scenario seeds one
    // (`mockCompanionBackend`, e.g. the disconnected scenario).
    case 'companion_backend_status':
    case 'companion_backend_set':
    case 'companion_backend_repush':
    case 'companion_backend_claim':
      return { url: null, connected: false, lastContactAt: null, lastError: null, mirrorError: null, mirroredTasks: 0, lastPushAt: null,
               mirroredDocs: null, docsError: null, docsSupported: null, scope: null, scopeError: null } as T;
    // Test connection (ST5): nothing is set up in the plain mock, so a blank
    // address has nothing to test and a typed one needs its token; with one,
    // it answers as a working backend.
    case 'companion_backend_test': {
      const url = String(args?.url ?? '').trim();
      if (!url) throw new Error('No backend is set up.');
      if (!String(args?.token ?? '').trim()) throw new Error("Paste the backend's applier token to test this address.");
      const host = (() => { try { return new URL(url).hostname; } catch { return url; } })();
      return { ok: true, message: `Connected: the backend at ${host} accepted the token. It has 0 paired devices and 0 shared tasks.`, version: '0.4.2' } as T;
    }
    // Companion backend v2: what the backend copies (personal by default).
    case 'companion_scope_get':
    case 'companion_scope_set':
      return {
        includeWorkData: false,
        seededAt: '2026-10-13T03:30:00Z',
        error: null,
        workspaces: [
          { id: 'local-hollow', title: 'hollow', status: 'active', work: false, personal: true },
          { id: 'jbmine-1', title: 'jbmine', status: 'active', work: false, personal: false },
          { id: 'notion-34db', title: 'Claims AI System', status: 'Idea', work: true, personal: false },
        ],
      } as T;
    case 'take_linear_links': return [] as T;
    // Per-chat effort lives in Rust (a2f011ae9): the window reads every chat's level at start.
    case 'chat_efforts_list': return [] as T;
    case 'chat_effort_set': return undefined as T;
    case 'chat_efforts_import': return (Array.isArray(args?.entries) ? args.entries.length : 0) as T;
    case 'update_people_list':
      if (!mockUpdateSettings().publisher) throw "People is only on the publisher's Mac.";
      return { people: mockUpdatePeople() } as T;
    case 'update_people_add': {
      if (!mockUpdateSettings().publisher) throw "People is only on the publisher's Mac.";
      const name = String(args?.name ?? '').trim();
      const channels = (['main', 'nightly'] as const).filter(c => (args?.channels as string[] | undefined)?.includes(c));
      if (!name) throw 'A person needs a name.';
      if (!channels.length) throw 'Choose at least one channel for a new person.';
      const person: UpdatePerson = { id: `p-${String(mockUpdatePeople().length + 1).padStart(12, '0')}`, name, channels: [...channels], revoked: false, createdAt: NOW };
      localStorage.setItem('mockUpdatePeople', JSON.stringify([...mockUpdatePeople(), person]));
      return { person, installLink: `https://updates.example.com/install/${'M'.repeat(43)}` } as T;
    }
    case 'update_people_revoke': return saveMockUpdatePerson(args?.id, { revoked: true }) as T;
    case 'update_people_set_channels':
      return saveMockUpdatePerson(args?.id, { channels: (['main', 'nightly'] as const).filter(c => (args?.channels as string[] | undefined)?.includes(c)) }) as T;
    // Updates, Go back (row U6): a spec seeds `mockGoBackPlan` (a whole or
    // partial UpdateGoBackPlan) and reads `mockUpdateWentBack`.
    case 'update_go_back_plan': return mockGoBackPlan() as T;
    case 'update_go_back': {
      const plan = mockGoBackPlan();
      const restore = Boolean(args?.restoreData);
      if (!plan.available || !plan.previous) return { outcome: 'refused', reason: plan.disabledReason ?? 'There is no previous version.' } as T;
      if (restore && !plan.restoreAvailable) return { outcome: 'refused', reason: "There's no intact copy of your chats from before the update to put back." } as T;
      if (!restore && plan.schema?.verdict === 'needsRestore') return { outcome: 'refused', reason: 'Choose Go back and restore data from before the update.' } as T;
      const work = JSON.parse(localStorage.getItem('mockExitRunningWork') ?? '[]') as Array<string | { key: string; name: string }>;
      const running = work.map(item => typeof item === 'string' ? { key: item, name: item } : item);
      const confirmed = args?.confirmedKeys as string[] | null | undefined;
      if (running.length && !confirmed) return { outcome: 'workRunning', work: running } as T;
      if (confirmed && running.some(w => !confirmed.includes(w.key))) return { outcome: 'workChanged', work: running } as T;
      localStorage.setItem('mockUpdateWentBack', JSON.stringify({ version: plan.previous.version, restoreData: restore }));
      saveMockUpdateSettings({ skippedVersion: plan.runningVersion });
      return { outcome: 'started', version: plan.previous.version } as T;
    }
    case 'update_go_back_when_idle': {
      const plan = mockGoBackPlan();
      if (!plan.available) throw plan.disabledReason ?? 'There is no previous version.';
      localStorage.removeItem('mockUpdateWaiting');
      localStorage.setItem('mockUpdateGoBackWaiting', '1');
      return mockUpdateState() as T;
    }
    case 'update_unskip':
      saveMockUpdateSettings({ skippedVersion: null });
      return mockUpdateState() as T;
    default:
      if (NO_OP_COMMANDS.has(command)) return undefined as T;
      recordMiss(command);
      throw new Error(
        `mockApi: no fixture for '${command}'. Add a case to src/mockApi.ts, ` +
        `or list it in NO_OP_COMMANDS if it is a mutation the harness can ignore.`,
      );
  }
}

// Overnight N1: the runner's state for Settings, Runs on. No runner without
// `mockRemoteRunner`; `mockRunnerStatus` (JSON) replaces the answer.
let mockRunnerChecked: string | null = null;
let mockRunnerChecks = 0;
/** Remote runners (2026-11-03): the GitHub source in `mockRunnerGithub`. */
function mockRunnerGithub(change: Partial<import('./types').RunnerGithubStatus> = {}): import('./types').RunnerGithubStatus {
  const saved = JSON.parse(localStorage.getItem('mockRunnerGithub') ?? '{}') as Partial<import('./types').RunnerGithubStatus>;
  const next = { source: 'runner-token' as const, tokenSaved: false, ghLogin: true, ...saved, ...change };
  localStorage.setItem('mockRunnerGithub', JSON.stringify(next));
  return next;
}

/** 1Password secrets: the saved switch and references, shaped as the Rust status (never a secret). */
function mockOnePassword(change: { enabled?: boolean } = {}, cli?: import('./types').OnePasswordStatus['cli'], refs: Record<string, string | null> = {}): import('./types').OnePasswordStatus {
  const saved = JSON.parse(localStorage.getItem('mockOnePassword') ?? '{}') as { enabled?: boolean; refs?: Record<string, string> };
  const enabled = change.enabled ?? saved.enabled ?? false;
  const all: Record<string, string> = { ...(saved.refs ?? {}) };
  for (const [key, ref] of Object.entries(refs)) { if (ref) all[key] = ref; else delete all[key]; }
  localStorage.setItem('mockOnePassword', JSON.stringify({ enabled, refs: all }));
  const slots = [['runner.github.token', 'GitHub runner token'], ['tracker:linear', 'Linear API key']] as const;
  return {
    enabled, installed: true, desktopApp: true,
    cli: cli ?? (enabled
      ? { state: 'unchecked', message: 'Press Check to see whether 1Password answers.', version: null }
      : { state: 'off', message: 'Off. Secrets come from the Keychain.', version: null }),
    slots: slots.map(([key, label]) => ({ key, label, reference: all[key] ?? null, lastError: null })),
  };
}

/** Remote runners 2: an owner view for `state`, with `mockRunnerOtherKeys` extra keys. */
function mockOwner(state: string): import('./types').RunnerOwner {
  const others = Number(localStorage.getItem('mockRunnerOtherKeys') ?? 0);
  const warnings: string[] = [];
  if (others) warnings.push(`${others} other SSH key${others === 1 ? '' : 's'} can log in as its runner account (~/.ssh/authorized_keys there): whoever holds ${others === 1 ? 'it' : 'one'} can run as that account and read what runs there, your turns' files and credentials included. Remove any you don't recognise.`);
  if (state === 'other') warnings.push("Another Hollow recorded itself as this runner's owner (2026-10-01). A runner should serve one person: if that wasn't you from another Mac, stop using it here and check who can log in there.");
  return { state: state === 'you' || state === 'other' ? state : 'none', recordedAt: state === 'none' ? null : '2026-11-03T19:00:00Z', keys: 1 + others, otherKeys: others, keyChanged: false, warnings };
}

/** Remote runners 3: steps Apply made done in this mock session. */
const mockSetupDone = new Set<string>();

function mockSetupState(id: string, done: boolean): import('./runnerSetup').RunnerSetupStepState {
  const ok = done || mockSetupDone.has(id);
  return { id, state: ok ? 'done' : 'missing', summary: ok ? 'Done.' : 'Not set up yet.', details: ok ? [] : ['Apply sets it up.'], needsByHand: false, ran: [] };
}

/** A short Set up plan, in the shape of runner_setup.rs's: defaults filled, the fields' problems, the steps and the estimate. */
function mockSetupPlan(given: import('./runnerSetup').RunnerSetupParams | undefined): import('./runnerSetup').RunnerSetupPlan {
  const g = given ?? ({} as import('./runnerSetup').RunnerSetupParams);
  const cloud = g.path === 'cloud';
  const alias = g.alias || (cloud ? 'hollow-gcp' : localStorage.getItem('mockRemoteRunner') || 'hollow-runner');
  const params = {
    path: cloud ? 'cloud' as const : 'mac' as const, admin: g.admin ?? '', alias, account: g.account || 'hollowworker', keyFile: g.keyFile || `~/.ssh/id_ed25519_${alias}`,
    hostName: g.hostName || (g.admin?.includes('@') ? g.admin.split('@')[1] : ''), project: g.project ?? '', zone: g.zone || 'us-central1-a',
    machineType: g.machineType || 'e2-standard-8', diskGb: g.diskGb || 200, idleMinutes: g.idleMinutes || 30, hoursPerMonth: g.hoursPerMonth || 160,
    billingAccount: g.billingAccount ?? '', budgetUsd: g.budgetUsd || 100, instance: g.instance || 'hollow-runner', serviceAccount: g.serviceAccount || 'hollow-runner',
  };
  const types = [
    { id: 'e2-standard-4', label: '4 vCPU, 16 GB', hourlyUsd: 0.134012, arm: false },
    { id: 'e2-standard-8', label: '8 vCPU, 32 GB', hourlyUsd: 0.268024, arm: false },
    { id: 't2a-standard-8', label: '8 vCPU, 32 GB, Arm', hourlyUsd: 0.308, arm: true },
  ];
  const problems: string[] = [];
  if (!cloud && !/^[A-Za-z0-9._@-]+$/.test(params.admin)) problems.push('The admin login is an SSH destination for an admin account there, like admin@your-mac.local.');
  if (cloud && !/^[a-z][a-z0-9-]{4,28}[a-z0-9]$/.test(params.project)) problems.push('The project id is 6 to 30 lowercase letters, digits and dashes, starting with a letter.');
  if (cloud && !/^[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}$/.test(params.billingAccount)) problems.push('The billing account id looks like 0X0X0X-0X0X0X-0X0X0X (gcloud billing accounts list).');
  const ssh = 'ssh -a -x -o BatchMode=yes -o ConnectTimeout=10 -o ControlPath=none';
  const admin = params.admin || 'admin@your-mac.local';
  const cmd = (on: import('./runnerSetup').RunnerSetupOn, who: string, shown: string, sudo = false) => ({ on, who, shown, sudo });
  const step = (id: string, title: string, why: string, changes: string[], confirm: string | null, check: import('./runnerSetup').RunnerSetupCmd[], apply: import('./runnerSetup').RunnerSetupCmd[]) =>
    ({ id, title, why, changes, confirm, check, apply, byHand: [] as import('./runnerSetup').RunnerSetupCmd[], byHandNote: null });
  const use = step('use', 'Use it', `Hollow checks ${alias}, records itself as its owner, sends one test call there through hollow-slot --on ${alias}, and only then names it as this Mac's runner.`, ['names this Mac\'s runner'], null,
    [cmd('hollow', 'Hollow', 'Hollow reads ~/.config/hollow/slot-remote: which runner this Mac uses.')],
    [cmd('local', 'this Mac', `hollow-slot --on ${alias} sh -c 'uname -sm; node --version; cargo --version'`), cmd('hollow', 'Hollow', `Hollow writes ${alias} on its own line in ~/.config/hollow/slot-remote.`)]);
  const steps = cloud ? [
    step('gcloud', 'Google Cloud sign-in', `Setup runs gcloud on this Mac as you, in project ${params.project || 'your-project'}.`, [], null, [cmd('gcloud', 'gcloud on this Mac', "gcloud config list account '--format=value(core.account)'")], []),
    step('cloud', 'The machine', `A Debian 12 VM, ${params.machineType}, with no public address, reached only through IAP.`, ['creates a VM (billed)', 'creates a budget alert'], 'This creates the VM.',
      [cmd('gcloud', 'gcloud on this Mac', `gcloud compute instances describe ${params.instance} --project=${params.project} --zone=${params.zone} '--format=value(status)'`)],
      [cmd('gcloud', 'gcloud on this Mac', `gcloud compute instances create ${params.instance} --project=${params.project} --zone=${params.zone} --machine-type=${params.machineType} --no-address`)]),
    use,
  ] : [
    step('admin', 'Admin login', `Setup logs in to ${admin} to make the runner account and change settings there.`, [], null,
      [cmd('admin', `${admin} (admin login)`, `${ssh} ${admin} 'id -un; sw_vers -productVersion; uname -m; sudo -n true 2>/dev/null && echo sudo=yes || echo sudo=no'`)], []),
    step('account', 'A hidden standard user', `Work runs as ${params.account}, a standard user hidden from the login window.`, ['creates a user'], `This creates the user ${params.account} on that Mac.`,
      [cmd('admin', `${admin} (admin login)`, `${ssh} ${admin} 'dscl . -read /Users/${params.account} UniqueID IsHidden'`)],
      [cmd('admin', `${admin} (admin login, sudo -n)`, `${ssh} ${admin} 'sudo -n sysadminctl -addUser ${params.account}'`, true)]),
    step('sleep', 'No sleep, and an SSH keepalive', 'A runner that sleeps drops the work on it.', ['changes sleep settings', 'changes sshd'], 'This changes sleep settings and sshd on that Mac.',
      [cmd('admin', `${admin} (admin login)`, `${ssh} ${admin} 'pmset -g'`)],
      [cmd('admin', `${admin} (admin login, sudo -n)`, `${ssh} ${admin} 'sudo -n pmset -a sleep 0 disksleep 0 autorestart 1'`, true)]),
    use,
  ];
  const type = types.find(t => t.id === params.machineType);
  const estimate = cloud && type ? (() => {
    // In millionths, rounded to cents once, as runner_setup.rs does.
    const cents = (micro: number) => Math.floor((micro + 5_000) / 10_000);
    const compute = Math.round(type.hourlyUsd * 1e6) * params.hoursPerMonth, disk = params.diskGb * 100_000, nat = 1_400 * params.hoursPerMonth + 3_650_000;
    const total = cents(compute) + cents(disk) + cents(nat);
    return { machineType: type.id, machineLabel: type.label, hourlyUsd: type.hourlyUsd, hours: params.hoursPerMonth, computeUsd: cents(compute) / 100, diskGb: params.diskGb, diskUsd: cents(disk) / 100,
      natUsd: cents(nat) / 100, totalUsd: total / 100, totalCents: total, region: params.zone.replace(/-[a-z]$/, ''), pricesRegion: 'us-central1', pricesTaken: '2026-11-03' };
  })() : null;
  return { params, problems, notes: [], steps, estimate, machineTypes: types };
}

function mockRunnerReady(): Partial<import('./types').RunnerStatus> {
  return { state: 'ready', checkedAt: new Date().toISOString(), millis: 900, machine: 'runner', preflight: 'ok host=runner free=243GB mem=92% rust=1.96.0 node=v26.3.1', running: 0 };
}

function mockRunnerStatus(fresh: boolean): import('./types').RunnerStatus {
  const host = localStorage.getItem('mockRemoteRunner');
  const none = { host, state: 'none' as const, checkedAt: null, millis: null, message: null, machine: null, preflight: null, running: null, gateRunning: false, lastGate: null, owner: null };
  if (!host) return none;
  // Smart routing 3: hollow-slot's latest decisions and the runner's tools.
  const minutesAgo = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
  const routing: import('./types').RouteStatus = {
    decisions: [
      { at: minutesAgo(2), where: host, why: 'gate, usually 3 min; this Mac 72% free', label: 'gate', source: 'auto' },
      { at: minutesAgo(5), where: 'here', why: 'vitest on named files, usually 2 s; sending it to the Mac mini would take about as long', label: 'vitest on named files', source: 'auto' },
      { at: minutesAgo(9), where: 'here', why: 'asked for with --here', label: 'npm run tauri:rebuild', source: '--here' },
    ],
    caps: { host, at: minutesAgo(2), line: 'rust=1.96.0 node=26.3.1 uv=0.13.5 python3=3.14.6 postgres=17.11 playwright=chromium-1243 gcloud=yes slots=0/2 cargo=free' },
  };
  const custom = localStorage.getItem('mockRunnerStatus');
  if (custom) return { ...none, routing, ...(JSON.parse(custom) as Partial<import('./types').RunnerStatus>), host };
  if (fresh) { mockRunnerChecked = new Date().toISOString(); mockRunnerChecks += 1; }
  if (!mockRunnerChecked) return { ...none, state: 'unchecked', routing };
  return {
    // Each check takes 0.1 s longer, so a spec can tell checks apart.
    ...none, routing, state: 'ready', checkedAt: mockRunnerChecked, millis: 1000 + 100 * mockRunnerChecks, machine: 'build-mac',
    preflight: 'ok host=build-mac free=243GB mem=92% rust=1.96.0 node=v26.3.1 browser=chromium-1228', running: 1,
    owner: mockOwner(localStorage.getItem('mockRunnerOwner') ?? 'none'),
    lastGate: { start: new Date(Date.now() - 20 * 60_000).toISOString(), end: new Date(Date.now() - 17 * 60_000).toISOString(), seconds: 151, ok: true, exitCode: 0, failedStep: null, commit: '4c5517b304bb29619c90acd925a00b13539e4e37', checkout: 'ri27-mm5' },
  };
}

// Settings, Agent updates (preview): Claude current, Codex one behind and
// busy in a chat, Antigravity unable to check, Grok behind.
const mockCli: import('./generated/ipc').AgentCliStatus[] = [
  { agent: 'claude', label: 'Claude', installed: true, version: '2.1.296', latest: '2.1.296', updateAvailable: false, method: "Claude Code's installer", updateCommand: 'claude update', canUpdate: true, canCheck: true, previousVersion: null, canRestore: false, safeWhileRunning: true, policy: 'automatic', lastChecked: new Date().toISOString(), state: 'idle', inUse: 2, message: null, tone: null, knownBad: null, latestKnownBad: null },
  { agent: 'codex', label: 'Codex', installed: true, version: '0.153.4', latest: '0.162.1', updateAvailable: true, method: 'npm (global)', updateCommand: 'npm install -g @openai/codex@0.162.1', canUpdate: true, canCheck: true, previousVersion: null, canRestore: false, safeWhileRunning: false, policy: 'automatic', lastChecked: new Date().toISOString(), state: 'idle', inUse: 0, message: null, tone: null, knownBad: null, latestKnownBad: null },
  { agent: 'antigravity', label: 'Antigravity', installed: true, version: '1.3.4', latest: null, updateAvailable: false, method: "Antigravity's updater", updateCommand: 'agy update', canUpdate: true, canCheck: false, previousVersion: null, canRestore: false, safeWhileRunning: false, policy: 'automatic', lastChecked: null, state: 'idle', inUse: 0, message: null, tone: null, knownBad: null, latestKnownBad: null },
  { agent: 'grok', label: 'Grok', installed: true, version: '1.0.46', latest: '1.0.52', updateAvailable: true, method: "Grok's updater", updateCommand: 'grok update', canUpdate: true, canCheck: true, previousVersion: null, canRestore: false, safeWhileRunning: false, policy: 'ask', lastChecked: new Date().toISOString(), state: 'idle', inUse: 1, message: null, tone: null, knownBad: null, latestKnownBad: null },
];
function mockCliRows(refresh: boolean) {
  if (refresh) for (const row of mockCli) if (row.canCheck) row.lastChecked = new Date().toISOString();
  // A spec can put Codex's latest on the known-bad list (repo ideas 10-27 #9).
  const knownBad = localStorage.getItem('mockCliKnownBad');
  return mockCli.map(row => ({ ...row, latestKnownBad: row.agent === 'codex' && row.updateAvailable ? knownBad : row.latestKnownBad }));
}
function mockCliUpdate(agent: string, now: boolean) {
  const row = mockCli.find(r => r.agent === agent)!;
  if (row.inUse > 0 && !(now && row.safeWhileRunning)) {
    Object.assign(row, { state: 'waiting', tone: 'warning', message: `Waiting until nothing is using ${row.label} (${row.inUse} running now); it updates then.` });
  } else if (row.updateAvailable || !row.canCheck) {
    const from = row.version;
    Object.assign(row, { previousVersion: from, version: row.latest ?? row.version, updateAvailable: false, state: 'idle', tone: 'success', message: `Updated ${row.label} from ${from} to ${row.latest ?? row.version}.`, canRestore: row.agent !== 'antigravity' });
  } else {
    Object.assign(row, { state: 'idle', tone: 'success', message: `${row.label} is up to date (${row.version}).` });
  }
  return { ...row };
}
function mockCliRestore(agent: string) {
  const row = mockCli.find(r => r.agent === agent)!;
  const from = row.version;
  Object.assign(row, { version: row.previousVersion, previousVersion: from, updateAvailable: true, tone: 'success', message: `Restored ${row.label} ${row.previousVersion}, from ${from}.` });
  return { ...row };
}
