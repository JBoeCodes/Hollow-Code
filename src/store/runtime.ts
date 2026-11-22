import { currentPage } from '../navigation';
import { windowAtMost } from '../windowLayout';
import { guardNavigation } from './navigationGuard';
import { editCurrent, keepLibraryView, nav, navQuiet, quietly, setNav, step, visit, workspaceNav, type NavEntry } from './navHistory';
import { libraryMode, placeLibraryView } from './libraryModeState';
import { directPageTarget, openSettingsPageTarget } from './settingsPageTarget';
export { openSettingsPageTarget };
import { withViewTransition } from '../viewTransitions';
import { resolveSettingsTarget, type SettingsTarget } from '../settingsResolve';
import { createSignal, createResource } from 'solid-js';
import { createStore } from 'solid-js/store';
import { listen } from '../nativeEvents';
import { api } from '../api';
import { OPERATOR_SWITCH_KEY, adoptOperatorSwitch, cachedOperatorSwitch } from '../operatorSwitch';
import { localDateKey, errMsg, parseFrontmatter } from '../utils';
import { kindLabel, TRUST_META } from '../vaultKinds';
import { passFromEvent, type SyncPass } from '../syncPass';
import { commitmentRevision, refreshCommitments } from '../commitments';
import { refreshReviewInbox, requestSuggestions } from '../reviewInbox';
import { refetchApprovals } from '../reviewQueue';
import { STREAK_ACTION_SCHEME, streakActionSlug } from '../streakAction';
import { isWorkspaceNotification, isWorkspaceVaultItem, isWorkspaceGraphNode, filterWorkspaceTree, suggestionContext, SUGGESTION_CONTEXTS, type SuggestionContext } from '../workspaceContent';
import { preserveDuringRefetch, unlessErrored } from '../stableResource';
import type { CannedRequest, VaultItem, VaultKind, HeartbeatConfig } from '../types';
import { isBudgetStopError } from '../budgets';
import type { BacklinkSource, Commitment, NoteContent, NoteRef, Task, VaultChange, VaultGraph } from '../types';
import { suppressToast } from '../toastPolicy';
import type { ExecutionReceipt } from '../executionReceipt';
// Domain controllers that used to live in this file (plan L3 X19,
// 2026-10-14), moved verbatim. Re-exported below so every `store/runtime`
// import keeps working. chatRuntime.ts and chatApprovals.ts import a few
// bindings back from here, only inside functions and callbacks (see the cycle
// note at the top of chatRuntime.ts); the other four import nothing from here.
import { showConfirm } from './overlays';
import { flashFooter, flashWithUndo } from './snackbars';
import { addNotificationToast, dismissNotificationToast } from './notificationToasts';
export { windowAway, setWindowAway } from './noticeTimers';
export * from './notificationToasts';
import { trackTask } from './activityDock';
import { openWorkView } from './destinations';
import { suppressPanelAnimation } from './panelAnimation';
import { AGENT_LABELS, chatAgent, chatCwd, chatModel, chatPermissionMode, chatSessionId, loadAgentsAvailable, type ChatAgent } from './chatRuntime';
export * from './overlays';
export * from './plaud';
export * from './snackbars';
export * from './activityDock';
export * from './chatRuntime';
export * from './chatApprovals';

export type PageId = 'today' | 'review' | 'tasks' | 'vault' | 'ingest' | 'chat' | 'workflows' | 'settings';

// ── Navigation
//
// `setActivePage` is a wrapper over the underlying signal setter — it
// also pushes onto a back/forward history stack so the user can
// navigate with mouse buttons (3 = back, 4 = forward), Cmd+[/Cmd+], or
// future UI affordances. The stack mirrors browser behavior: forward
// history past the cursor is truncated when you navigate somewhere new.
const [activePage, _setActivePage] = createSignal<PageId>('today');
export { activePage };
/** Library → Conversations is on screen: the old Conversation history page
 *  (S4, 2026-11-03), which page id `chat` still opens. Its unscoped composer
 *  is the only conversation outside a workspace with a whole page. */
export const conversationHistoryShown = () => activePage() === 'vault' && libraryMode() === 'conversations';
/** The page on screen by the id that opens it: `chat` for Conversation
 *  history, so Back and Forward step to it and callers that went there can
 *  tell they arrived. */
export const shownPage = (): PageId => conversationHistoryShown() ? 'chat' : activePage();
/** The Library step on screen keeps the tab it shows as it is left, so Back
 *  reopens that tab rather than the one Conversations was opened from (S4). */
const keepLibraryStep = () => { if (activePage() === 'vault') setNav(s => keepLibraryView(s, libraryMode())); };

// The entries and how they step are store/navHistory.ts's (P1-42): each
// workspace and conversation opened is a step, so ⌘[ goes back through them.

export const canNavBack = () => nav().cursor > 0;
export const canNavForward = () => nav().cursor < nav().entries.length - 1;

export const [libraryCollectionRequest, setLibraryCollectionRequest] = createSignal<string | null>(null);
export function setActivePage(p: PageId) {
  if (currentPage(p) !== activePage()) {
    return guardNavigation(() => applyActivePage(p));
  }
  applyActivePage(p);
}

function applyActivePage(p: PageId) {
  // A page change never animates panels into place (0.2): held until two frames after paint.
  suppressPanelAnimation();
  // Graph, People and Calendar were cut as pages (overnight 11-08 S3).
  // Import still lands in Library on every file.
  if (p === 'ingest') setLibraryCollectionRequest('all');
  if (!navQuiet()) keepLibraryStep();
  // Conversation history (`chat`) is Library → Conversations since S4.
  placeLibraryView(p);
  p = currentPage(p);
  _setActivePage(p);
  // An expanded rail plus a page-local aside leaves narrow windows with
  // no usable document pane. Treat choosing a destination as dismissal of the
  // mobile navigation drawer; the user can always reopen it with Cmd+B.
  if (windowAtMost('compact') && !sidebarCollapsed()) {
    setSidebarCollapsed(true);
  }
  if (navQuiet()) return;
  // Your work's page records which workspace and conversation it shows.
  setNav(s => visit(s, p === 'today' ? { page: p, ...workspaceNav().current() } : { page: shownPage() }));
}

export function navBack() {
  return guardNavigation(() => stepNav(-1));
}
export function navForward() {
  return guardNavigation(() => stepNav(1));
}
function stepNav(by: -1 | 1) {
  keepLibraryStep();
  const next = step(nav(), by);
  if (!next) return;
  // The cursor moves now (a second ⌘[ steps further); the page change
  // animates (overnight 10-11 B1).
  setNav(next.stack);
  withViewTransition(() => applyNavEntry(next.entry));
}

function applyNavEntry(entry: NavEntry) {
  quietly(() => {
    // A `chat` step is Library → Conversations; another Library step shows
    // the tab it kept, or leaves Conversations.
    placeLibraryView(entry.page, entry.libraryView);
    // The page first, unguarded (Back already passed the guard, and this may
    // run later, in the view transition's callback).
    _setActivePage(currentPage(entry.page));
    // A workspace, a conversation or the overview: the workspace store
    // reopens it (once it loads, its page change matches this entry), and
    // drops an open still loading for another entry.
    workspaceNav().restore(entry);
    // Bumping the signal (even to the same value) reopens the note —
    // possible because `openVaultNoteRequest` is declared `equals: false`.
    if (entry.notePath) setOpenVaultNoteRequest(entry.notePath);
  });
}
export const [sidebarCollapsed, setSidebarCollapsed] = createSignal(
  (() => {
    if (typeof window !== 'undefined' && window.matchMedia('(max-width: 860px)').matches) return true;
    try {
      const saved = localStorage.getItem('sidebarCollapsed');
      if (saved != null) return saved === 'true';
      return typeof window !== 'undefined' && window.matchMedia('(max-width: 860px)').matches;
    } catch { return false; }
  })()
);

// ── Resizable sidebars (with bounds + localStorage persistence)
const readNum = (key: string, fallback: number) => {
  try {
    const v = localStorage.getItem(key);
    if (!v) return fallback;
    const n = parseInt(v, 10);
    return Number.isFinite(n) ? n : fallback;
  } catch { return fallback; }
};
export const VAULT_ASIDE_MIN = 220;
export const VAULT_ASIDE_MAX = 560;
export const CHAT_ASIDE_MIN = 220;
export const CHAT_ASIDE_MAX = 480;
export const CHAT_PANEL_MIN = 320;
export const CHAT_PANEL_MAX = 720;
export const [vaultAsideWidth, setVaultAsideWidth] = createSignal(readNum('vaultAsideWidth', 320));
export const [chatAsideWidth, setChatAsideWidth] = createSignal(readNum('chatAsideWidth', 280));
export const [chatPanelWidth, setChatPanelWidth] = createSignal(readNum('chatPanelWidth', 420));

export function persistVaultAsideWidth() {
  try { localStorage.setItem('vaultAsideWidth', String(vaultAsideWidth())); } catch { /* ignore */ }
}
export function persistChatAsideWidth() {
  try { localStorage.setItem('chatAsideWidth', String(chatAsideWidth())); } catch { /* ignore */ }
}
export function persistChatPanelWidth() {
  try { localStorage.setItem('chatPanelWidth', String(chatPanelWidth())); } catch { /* ignore */ }
}
// The Preview pane's width (run 15 P1), set by dragging its left edge; it
// keeps the side panel's key, so a width dragged there carries over. null =
// the stylesheet's default; double-click the edge to reset. The page beside
// it always keeps SIDE_PANE_CHAT_MIN.
export const SIDE_PANE_MIN = 320;
export const SIDE_PANE_CHAT_MIN = 360;
function readSidePaneWidth(): number | null {
  try {
    const v = Number(localStorage.getItem('sidePaneWidth'));
    return Number.isFinite(v) && v >= SIDE_PANE_MIN ? v : null;
  } catch { return null; }
}
export const [sidePaneWidth, setSidePaneWidth] = createSignal<number | null>(readSidePaneWidth());
export function persistSidePaneWidth() {
  try {
    const w = sidePaneWidth();
    if (w == null) localStorage.removeItem('sidePaneWidth'); else localStorage.setItem('sidePaneWidth', String(Math.round(w)));
  } catch { /* ignore */ }
}
// Right-side chat panel — lets the user keep Claude open while on any page.
// Shares all chat state (session, turns, input) with Conversation history;
// auto-hides while that is shown (conversationHistoryShown) to avoid showing
// the same conversation twice on screen.
export const [chatPanelOpen, setChatPanelOpen] = createSignal(
  (() => {
    try { return localStorage.getItem('chatPanelOpen') === 'true'; } catch { return false; }
  })()
);
export function toggleChatPanel() {
  setChatPanelOpen(v => {
    const next = !v;
    try { localStorage.setItem('chatPanelOpen', String(next)); } catch { /* ignore */ }
    return next;
  });
}
export function closeChatPanel() {
  setChatPanelOpen(false);
  try { localStorage.setItem('chatPanelOpen', 'false'); } catch { /* ignore */ }
}

// True while the window is unfocused or hidden — the same state that drives
// `body.is-away`, set from the single detector App.tsx owns. Exposed as a
// signal because CSS's `--ambient-play` pause cannot reach a JS-driven
// animation loop: the 3D graph's WebGL rAF reads this to bank itself.
// Shared with notice countdowns through noticeTimers.ts (re-exported above).

// A monotonically-incrementing focus request. ChatBody subscribes via a
// createEffect and focuses its textarea on every bump — using a counter
// (not a boolean) means repeated requests always re-fire even when the
// value would otherwise be unchanged. `focusChatPanel()` opens the panel
// if it's closed and Conversation history isn't shown (where the panel is
// the only ChatBody surface), then requests focus regardless. In
// Conversation history the panel is hidden but its ChatBody is mounted, so
// the same request lands there.
export const [chatFocusRequest, setChatFocusRequest] = createSignal(0);
/** A message the workspace composer sends as soon as it shows the
 *  conversation in `cwd` and is idle (Your work's Ask anything starts a quick
 *  chat with it, 2026-09-30). The normal send path, not a second one.
 *  `preset` sends it as a ship button does, leaving the person's draft in the
 *  composer, and `sessionKey` (when given) holds it for that conversation
 *  only (Make a canvas from this, overnight 10-11/12 F4). `keepFocus` leaves
 *  focus where it is once it is sent, instead of in the composer: a section's
 *  Make a canvas view keeps it on its button (A02 review, 2026-10-17).
 *  `canned` marks words Hollow wrote (a canvas request, overnight 9 A1).
 *  `done` hears whether the composer's send queued a turn (Ask's Across all
 *  my work keeps its words unless it did, run 15 P3). */
export const [pendingChatSend, setPendingChatSend] = createSignal<{ text: string; cwd: string; preset?: boolean; sessionKey?: string | null; keepFocus?: boolean; canned?: CannedRequest; done?: (ok: boolean) => void } | null>(null);
let chatFocusRequestedAt = 0;
export function requestChatFocus() { chatFocusRequestedAt = Date.now(); setChatFocusRequest(n => n + 1); }
/** A focus request made just before a composer mounted (a lazily loaded
 *  workspace conversation opened from Your work) still lands on it. */
export const chatFocusRequestedWithin = (ms: number) => Date.now() - chatFocusRequestedAt < ms;
export function focusChatPanel() {
  if (!conversationHistoryShown() && !chatPanelOpen()) {
    setChatPanelOpen(true);
    try { localStorage.setItem('chatPanelOpen', 'true'); } catch { /* ignore */ }
  }
  requestChatFocus();
}

// Tasks' view, filters, grouping and order live with the page
// (`trackerViews.ts`, overhaul B2); the saved-filter store, List/Board
// layout and Projects tab signals went with the Notion-era Tasks (11-10).

// Pure query evaluation lives in taskQuery.ts (unit-testable, no store
// boot); re-exported here so existing `from './store'` imports keep working.
export { matchesTaskQuery } from '../taskQuery';

// Message integrations are retired; discard their obsolete browser caches.
for (const key of ['teamsBriefCache', 'slackBriefCache', 'inboxAsideWidth', 'm365WriteEnabled']) {
  try { localStorage.removeItem(key); } catch { /* storage unavailable */ }
}

// Auto-inject a snapshot of the current page (active tab + visible data)
// into each chat prompt. Defaults ON so Claude immediately knows where the
// user is and what they're looking at without the user having to restate it.
export const [chatContextEnabled, setChatContextEnabled] = createSignal(
  (() => {
    try {
      const v = localStorage.getItem('chatContextEnabled');
      return v === null ? true : v === 'true';
    } catch { return true; }
  })()
);
export function toggleChatContext() {
  setChatContextEnabled(v => {
    const next = !v;
    try { localStorage.setItem('chatContextEnabled', String(next)); } catch { /* ignore */ }
    return next;
  });
}

// Operator-context injection: the durable "how the user works" digest
// (compiled from the vault me/ folder) goes into the first turn of each new
// native chat session (the backend's context compiler adds it, for every
// entry point) and into workflow runs. Where page-context tells the agent
// what's on screen, this tells it the house style + settled decisions so it
// stops needing a re-brief each session.
//
// The backend persists the switch and is its source of truth (see
// operatorSwitch.ts): the window shows its cached copy (off when missing)
// until the backend answers at launch, adopts the backend's value, and
// pushes only an explicit change.
const switchStorage = (() => { try { return localStorage; } catch { return null; } })();
export const [operatorContextEnabled, setOperatorContextEnabledSignal] = createSignal(
  cachedOperatorSwitch(switchStorage)
);
let operatorSwitchChanged = false;
export function setOperatorContextEnabled(v: boolean) {
  operatorSwitchChanged = true;
  setOperatorContextEnabledSignal(v);
  try { localStorage.setItem(OPERATOR_SWITCH_KEY, String(v)); } catch { /* ignore */ }
  // An explicit change: the backend persists it for chats, workflow runs
  // and later launches.
  api.setOperatorContextEnabled(v).catch(() => { /* not in tauri */ });
}
export function toggleOperatorContext() { setOperatorContextEnabled(!operatorContextEnabled()); }
// At launch, adopt the backend's persisted value (never push the cached
// copy: a lost key would turn an "off" back on). A change made while this
// was in flight wins.
void adoptOperatorSwitch({
  get: () => api.operatorContextSwitch(),
  set: v => api.setOperatorContextEnabled(v),
  storage: switchStorage,
}).then(v => { if (v !== null && !operatorSwitchChanged) setOperatorContextEnabledSignal(v); });

/// Delete a task with a one-click Undo. Optimistically drops it from the cached
/// list, deletes the task through the active backend, then shows an Undo
/// snackbar. Vault-local tasks restore the file byte-for-byte.
export async function deleteTask(task: Task) {
  const snap = tasksData();
  const title = task.title || 'task';
  if (snap) mutateTasks({ ...snap, items: snap.items.filter(t => t.url !== task.url) });

  let content: string;
  try {
    content = await trackTask(`Delete task: ${title}`, () => api.deleteTask(task.url), { detail: 'Vault delete' });
  } catch (e: any) {
    if (snap) mutateTasks(snap); // roll back the optimistic removal
    flashFooter(`× delete failed: ${e?.message || e}`, 3000);
    return;
  }
  Promise.resolve(refetchTasks()).catch(() => {});

  flashWithUndo({
    message: `Deleted “${trimTitle(title, 48)}”`,
    kind: 'delete',
    undo: async () => {
      try {
        await trackTask(`Restore task: ${title}`, () => api.restoreTask(task.url, content), { detail: 'Vault undo' });
        Promise.resolve(refetchTasks()).catch(() => {});
            } catch (e: any) {
        flashFooter(`× restore failed: ${e?.message || e}`, 3500);
      }
    },
  });
}

export function toggleSidebar() {
  setSidebarCollapsed(v => {
    const next = !v;
    try { localStorage.setItem('sidebarCollapsed', String(next)); } catch { /* ignore */ }
    return next;
  });
}

/** Search chats asked for (L1, 11-12): the top bar's search icon or ⌘⇧K.
 *  The sidebar (Rail.tsx) opens its search, showing itself first if hidden;
 *  closing it puts focus back on `returnTo`, or on the top bar's icon. */
export const [searchChatsRequest, setSearchChatsRequest] = createSignal<{ n: number; returnTo: HTMLElement | null }>({ n: 0, returnTo: null });
export function requestSearchChats(returnTo: HTMLElement | null = null) {
  setSearchChatsRequest(r => ({ n: r.n + 1, returnTo }));
}

// ── Reactive data resources
//
// Every resource is lazy: gated by an enable signal that the relevant page
// flips on mount, so opening the app doesn't fan out to disk for pages the
// user hasn't visited yet.
const [_tasksEnabled, setTasksEnabled] = createSignal(false);
const [_projectsEnabled, setProjectsEnabled] = createSignal(false);
const [_vaultTreeEnabled, setVaultTreeEnabled] = createSignal(false);
const [_skillsEnabled, setSkillsEnabled] = createSignal(false);
const [_workflowsEnabled, setWorkflowsEnabled] = createSignal(false);
const [_ledgerEnabled, setLedgerEnabled] = createSignal(false);

const [_tasksData, { refetch: refetchTasks, mutate: mutateTasks }] =
  createResource(_tasksEnabled, () => api.tasks());
const [_projectsData, { refetch: refetchProjects, mutate: mutateProjects }] =
  createResource(_projectsEnabled, () => api.projects());
const [_vaultTree, { refetch: refetchVaultTree }] =
  createResource(_vaultTreeEnabled, () => api.vaultTree().then(tree => filterWorkspaceTree(tree)));
// The semantic vault index — every file classified into a VaultItem. Backs the
// Vault tab's lenses + cleanup queue and richer agent context. Shares the vault
// tree's enable flag (the Vault page mount + the background warmer flip it on)
// and refetches on any vault change.
const [_vaultItemsData, { refetch: refetchVaultItems }] =
  createResource(_vaultTreeEnabled, () => api.vaultItems().then(items => items.filter(isWorkspaceVaultItem)));
const [_skillsList, { refetch: refetchSkills }] =
  createResource(_skillsEnabled, () => api.listSkills());
const [_workflowsList, { refetch: refetchWorkflows }] =
  createResource(_workflowsEnabled, () => api.listWorkflows());
// Activity Ledger — the unified newest-first feed of what the agents did
// (heartbeat + computer/m365 audit + workflow run logs). Lazy like its
// siblings (Today flips it on mount; the background warmer enables it too).
// Re-read via `refetchLedger()` after a vault change touches a trail.
const [_activityLedger, { refetch: refetchLedger }] =
  createResource(_ledgerEnabled, () => api.listActivityLedger(60));
export {
  refetchTasks, mutateTasks, refetchProjects, mutateProjects,
  refetchVaultTree, refetchVaultItems, refetchSkills, refetchWorkflows,
  refetchLedger,
};
export const tasksData = preserveDuringRefetch(_tasksData);
export const projectsData = preserveDuringRefetch(_projectsData);
export const vaultTree = preserveDuringRefetch(_vaultTree);
export const vaultItemsData = preserveDuringRefetch(_vaultItemsData);
export const skillsList = preserveDuringRefetch(_skillsList);
export const workflowsList = preserveDuringRefetch(_workflowsList);
export const activityLedger = preserveDuringRefetch(_activityLedger);
export function enableLedgerResource(): void { setLedgerEnabled(true); }

// ── Usage budgets (Paperclip PR 8a) — Settings, Usage's BudgetsPanel. Lazy
// (the panel enables it on mount); read through preserveDuringRefetch so a
// `budget-changed` refetch never re-suspends the Settings page.
const [_budgetsEnabled, setBudgetsEnabled] = createSignal(false);
const [_budgetsView, { refetch: refetchBudgets }] =
  createResource(_budgetsEnabled, () => api.getBudgets());
export { refetchBudgets };
export const budgetsView = preserveDuringRefetch(_budgetsView);
export function enableBudgetsResource(): void { setBudgetsEnabled(true); }

/// Open Automations with Waiting on you filtered to its budget pauses: the
/// target of a pause notification and of Since last visit's budget rows.
export function openBudgetPauses(): void {
  setWorkflowsTab('workflows');
  setActivePage('workflows');
  refreshReviewInbox();
  requestSuggestions('budgets');
}

// Budgets or pauses changed (a save, a resume, a pause opened by a claim or a
// finished run): refresh the panel, every review panel and the automation
// chips (Workflow.budgetHold).
listen('budget-changed', () => {
  if (_budgetsEnabled()) void refetchBudgets();
  refreshReviewInbox();
  if (_workflowsEnabled()) void refetchWorkflows();
}).catch(() => { /* not in tauri */ });

/// Page-mount hooks turn these on; once enabled, they stay enabled for the
/// life of the app session (refetch* still works to re-pull). This means a
/// drag from Tasks → Projects sidebar still has data even after the user
/// navigates away from those pages.
export function enableTasksResource()     { setTasksEnabled(true); }
export function enableProjectsResource()  { setProjectsEnabled(true); }
export function enableVaultTreeResource() { setVaultTreeEnabled(true); }
export function enableSkillsResource()    { setSkillsEnabled(true); }
export function enableWorkflowsResource() { setWorkflowsEnabled(true); }

// ── Local suggestions — the review queue snapshot lives in ../reviewQueue
// (re-exported here), so the "waiting on you" surface's state can reload it
// without importing the store.
export { reviewQueueItems, refetchApprovals, enableApprovalsResource, approvalsPendingCount, reviewQueueFailed, reloadReviewQueue } from '../reviewQueue';
// ── Commitments — one snapshot of the promise ledger for every surface.
// Shared by Today and the ledger, avoiding duplicate folder reads. Same
// shape as the approvals queue above: gated so it stays cold until a surface
// asks for it, then keyed on the shared `commitmentRevision` token so a modal
// save, a status flip, or the vault watcher refreshes every reader at once.
// People.tsx still keeps a fourth copy for its relationship compass — it
// should read this too.
const [_commitmentsEnabled, setCommitmentsEnabled] = createSignal(false);
// A failed read keeps its error and the last good list (P0-04): an empty
// list in its place told Today and its brief "No commitments due".
const [_commitmentsError, setCommitmentsError] = createSignal<string | null>(null);
export const commitmentsError = _commitmentsError;
let lastCommitments: Commitment[] = [];
export const [commitmentsList, { refetch: refetchCommitments }] =
  createResource<Commitment[], number>(
    () => (_commitmentsEnabled() ? commitmentRevision() + 1 : false),
    () => api.listCommitments().then(
      list => { lastCommitments = list; setCommitmentsError(null); return list; },
      e => { setCommitmentsError(errMsg(e)); return lastCommitments; },
    ),
  );
export function enableCommitmentsResource() { setCommitmentsEnabled(true); }

// ── Vault graph — the Observatory's payload, hoisted out of the page.
// It lived in Graph.tsx, so every nav away disposed it and every return
// re-serialized the whole node/edge set before anything could paint. Lives
// here so the map survives navigation; deliberately NOT wired to the vault
// watcher, because a sync burst would re-lay-out the graph under the user's
// hands. The refresh button is the only re-read.
const [_graphEnabled, setGraphEnabled] = createSignal(false);
const [_vaultGraphData, { refetch: refetchVaultGraph }] =
  createResource<VaultGraph, boolean>(_graphEnabled, () => api.vaultGraph().then(graph => {
    const nodes = graph.nodes.filter(isWorkspaceGraphNode);
    const ids = new Set(nodes.map(node => node.id));
    return { nodes, edges: graph.edges.filter(edge => ids.has(edge.source) && ids.has(edge.target)) };
  }));
export { refetchVaultGraph };
export const vaultGraphData = preserveDuringRefetch(_vaultGraphData);
export function enableGraphResource() { setGraphEnabled(true); }

export type StartupWarmupStatus = 'scheduled' | 'running' | 'done' | 'error';
export interface StartupWarmupStep {
  id: string;
  label: string;
  tier: 'critical' | 'daily' | 'idle' | 'deferred';
  status: StartupWarmupStatus;
  scheduledAt: string;
  startedAt: string | null;
  finishedAt: string | null;
  durationMs: number | null;
  error: string | null;
}

export const [startupWarmupSteps, setStartupWarmupSteps] = createSignal<StartupWarmupStep[]>([]);

let warmupStarted = false;

function scheduleWarmupStep(step: Omit<StartupWarmupStep, 'status' | 'scheduledAt' | 'startedAt' | 'finishedAt' | 'durationMs' | 'error'>) {
  const row: StartupWarmupStep = {
    ...step,
    status: 'scheduled',
    scheduledAt: new Date().toISOString(),
    startedAt: null,
    finishedAt: null,
    durationMs: null,
    error: null,
  };
  setStartupWarmupSteps(prev => [...prev, row]);
}

async function runWarmupStep(id: string, fn: () => void | Promise<void>) {
  const started = performance.now();
  const startedAt = new Date().toISOString();
  setStartupWarmupSteps(prev => prev.map(step =>
    step.id === id ? { ...step, status: 'running', startedAt } : step,
  ));
  try {
    await fn();
    const finishedAt = new Date().toISOString();
    setStartupWarmupSteps(prev => prev.map(step =>
      step.id === id
        ? { ...step, status: 'done', finishedAt, durationMs: Math.round(performance.now() - started) }
        : step,
    ));
  } catch (err) {
    const finishedAt = new Date().toISOString();
    setStartupWarmupSteps(prev => prev.map(step =>
      step.id === id
        ? {
          ...step,
          status: 'error',
          finishedAt,
          durationMs: Math.round(performance.now() - started),
          error: errMsg(err),
        }
        : step,
    ));
  }
}

function scheduleAfter(delayMs: number, id: string, fn: () => void | Promise<void>) {
  window.setTimeout(() => { void runWarmupStep(id, fn); }, delayMs);
}

function scheduleIdle(id: string, fn: () => void | Promise<void>, timeout = 2500) {
  const requestIdle = (window as any).requestIdleCallback as
    | undefined
    | ((cb: () => void, opts?: { timeout: number }) => number);
  if (requestIdle) requestIdle(() => { void runWarmupStep(id, fn); }, { timeout });
  else scheduleAfter(timeout, id, fn);
}

/// Staged lazy-resource warm-up, in three tiers keyed to what the user can
/// actually see:
///
///   1. `badges` — everything the rail's numbers and Today's deck read.
///      Released immediately: these are small reads, and a delay here just
///      means the rail lies for that long. It used to be split across a
///      250ms tier and a 900ms tier, with the workflow list stranded four
///      and a half seconds back in a "deferred" tier even though the rail's
///      failure badge (`!N`) is derived from it — so a routine that failed
///      overnight stayed invisible for the first 4.5s of every launch.
///   2. `vault-index` — the whole-vault walk (tree + semantic index + the
///      wikilink note list). By far the largest payload here, so it keeps
///      its own idle slot rather than landing alongside anything else.
///   3. `deferred` — data only a specific page or panel shows. Nothing on
///      screen at boot depends on it.
///
/// `Today.onMount` has already started the critical daily path (calendar,
/// tasks and approvals) before this parent onMount
/// fires; the enables below are idempotent, and repeating them here is what
/// keeps the rail honest when boot lands on some other page. Each resource
/// still owns its own loading/error state — this signal only reports when
/// the scheduler released each tier.
export function warmUpAllInBackground(): void {
  if (warmupStarted) return;
  warmupStarted = true;

  // The workspace shell needs no calendar, project, people, graph or legacy
  // transcript inventory at boot. Each supporting view enables its own data.
  scheduleWarmupStep({ id: 'agents', label: 'Available agents', tier: 'daily' });
  void runWarmupStep('agents', async () => { await loadAgentsAvailable(); });
}

// ── Operator profile (the vault me/ folder digest)
// Cheap local fs read, no network, no dependency on other resources — safe to
// fetch eagerly so the chat composer has the house rules ready for the first
// message. Settings reads it to show what's being injected; the chat composer
// reads `.body`. Refetch after the user edits a me/ file from the Vault tab.
// Read through `preserveDuringRefetch`: a `me/` file change refetches it, and a
// raw read under Settings' page Suspense swapped the page for its loading
// fallback, so Settings jumped to the top while you read or edited (11-11).
const [_operatorProfile, { refetch: refetchOperatorProfile }] =
  createResource(() => api.operatorContext());
export const operatorProfile = preserveDuringRefetch(_operatorProfile);
export { refetchOperatorProfile };

// ── Notes index for wikilink autocomplete + resolution
export const [noteIndex, setNoteIndex] = createSignal<NoteRef[]>([]);
export async function refreshNoteIndex() {
  setNoteIndex((await api.vaultNotesList()).filter(isWorkspaceVaultItem));
}

// ── Vault editor state
//
// `baseline` is the on-disk content as Hollow last knew it (set on open + after
// a successful save + on a live-reload). The autosave is a compare-and-swap
// against it (data safety Tier 0.2): if the file changed underneath us (an
// Obsidian edit), the save returns a conflict instead of clobbering, and
// `conflict` holds the on-disk version so the editor can offer reload / keep-mine.
export const [vaultStore, setVaultStore] = createStore<{
  current: NoteContent | null;
  dirty: boolean;
  baseline: string;
  conflict: NoteContent | null;
}>({ current: null, dirty: false, baseline: '', conflict: null });

// ── Backlinks for the current vault note. Auto-refetches whenever the
// open note's path changes. Exposed at module scope so `pageContext`
// can read it synchronously — the in-app Claude sees who links to
// the note the user is editing without needing a tool call.
export const [currentBacklinks] = createResource<BacklinkSource[], string>(
  () => vaultStore.current?.path,
  async (path) => (path ? await api.vaultBacklinks(path) : []),
);

// ── Cross-page "new note" trigger (Cmd+Shift+N from anywhere navigates to Vault
//    and opens the create form). Vault.tsx subscribes via createEffect+on.
export const [newNoteRequest, setNewNoteRequest] = createSignal(0);
export const [newNoteTemplateRequest, setNewNoteTemplateRequest] = createSignal<string | null>(null);
export function requestNewNote(template?: string) { setNewNoteTemplateRequest(template ?? null); setNewNoteRequest(c => c + 1); }

// ─────────────────────────────────────────────────────────────
// Cross-page "open this vault note" trigger. Anyone (Today's
// calendar wikilinks, etc.) can call requestOpenVaultNote
// to switch to Vault and open a specific markdown file.
// ─────────────────────────────────────────────────────────────

// `equals: false` so re-opening the same path still fires the effect —
// needed for back/forward to restore a vault entry whose path equals
// the currently-cached request value.
export const [openVaultNoteRequest, setOpenVaultNoteRequest] = createSignal<string | null>(null, { equals: false });
export function requestOpenVaultNote(path: string) {
  setOpenVaultNoteRequest(path);
  if (navQuiet()) return;
  // Annotate the current history entry with the note path so back/forward
  // can restore it. We only stamp the *current* (most recent) entry —
  // setActivePage('vault') is expected to be called just before this
  // for cross-page opens, and we don't want to retroactively rewrite
  // older entries.
  setNav(s => editCurrent(s, e => e.page === 'vault' && e.notePath !== path ? { ...e, notePath: path } : null));
}

/// Navigate to the Vault tab and open a vault-relative note path. The single
/// "open this in the vault" entry point for tasks/projects/etc. — the vault is
/// the only link target (Notion/Obsidian deep-links were removed). No-ops on a
/// null/empty path so callers can pass an optional `vaultPath` directly.
export function openVaultNote(relPath: string | null | undefined): void {
  if (!relPath) return;
  setActivePage('vault');
  requestOpenVaultNote(relPath);
}

// Cross-page jump into the Vault History lens. Receipts use this instead of
// opening the current note so their changed-path links land on the exact
// recovery surface (and, when available, the exact durable run).
export interface VaultHistoryRequest {
  path?: string;
  runId?: string;
}
export const [openVaultHistoryRequest, setOpenVaultHistoryRequest] = createSignal<VaultHistoryRequest | null>(null, { equals: false });
export function openVaultHistory(path?: string, runId?: string): void {
  setActivePage('vault');
  setOpenVaultHistoryRequest({
    ...(path ? { path } : {}),
    ...(runId ? { runId } : {}),
  });
}

// ─────────────────────────────────────────────────────────────
// Browser / clipboard helpers
// ─────────────────────────────────────────────────────────────

/// Shared open-in-browser helper used by the auth flow and by the
/// "Open in browser" button on the banner. Throws so callers can
/// surface the error rather than silently swallowing it.
export async function openInBrowser(url: string): Promise<void> {
  const { openUrl } = await import('@tauri-apps/plugin-opener');
  await openUrl(url);
}

// ─────────────────────────────────────────────────────────────
// Tracker passes refresh already-loaded vault resources even when only
// ledger state changed, so no note write will wake the vault watcher.
export const [trackerSyncPass, setTrackerSyncPass] = createSignal<SyncPass | null>(null);
listen<unknown>('tracker-sync-updated', e => {
  setTrackerSyncPass(passFromEvent(e.payload, Date.now()));
  if (tasksData() !== undefined) refetchTasks();
  if (projectsData() !== undefined) refetchProjects();
}).catch(() => {/* not in tauri */});

// LIVE VAULT UPDATES: the vault is Hollow's local operating surface, even
// when a remote source owns a domain. Obsidian, embedded agents, sync jobs, and
// any text editor can all write to the same folders. The Rust FS watcher
// (`vault_watch.rs`) emits a debounced `vault-changed` with the set of top-level
// folders touched + the changed paths; we map those to the exact resources to
// refetch so the UI reflects the change without a `Cmd+R`. App-made writes also
// trip this — the refetch just re-reads what we wrote (idempotent), and the
// open-note reload compares content so it never disrupts the editor on an
// autosave.
// ─────────────────────────────────────────────────────────────

/// Live-reload the open vault note when it's the one that changed on disk.
/// Never clobbers unsaved edits (`dirty`), and only swaps the buffer when the
/// on-disk content genuinely differs — so our own autosave write (or a no-op
/// touch) doesn't reset the CodeMirror cursor/scroll.
async function maybeReloadOpenNote(changedPaths: string[]): Promise<void> {
  const cur = vaultStore.current;
  if (!cur || vaultStore.dirty) return;
  if (!changedPaths.includes(cur.path)) return;
  try {
    const fresh = await api.vaultNote(cur.path);
    if (
      vaultStore.current?.path === cur.path &&
      !vaultStore.dirty &&
      fresh.content !== vaultStore.current.content
    ) {
      // The buffer wasn't dirty, so adopting the disk version is safe — and it
      // becomes the new CAS baseline so a later autosave compares against it.
      setVaultStore({ current: fresh, dirty: false, baseline: fresh.content, conflict: null });
    }
  } catch {
    // The note may have been deleted/renamed out from under us — the tree
    // refetch reflects the removal; leave the stale buffer viewable.
  }
}

// Coalesce vault-changed bursts. The backend already debounces raw FS
// events (300ms), but a long sync (recording backfill) emits a
// burst per quiet window and each used to trigger the full refetch
// fan-out below. Leading-edge: the first burst refetches immediately so
// live updates stay instant; bursts landing inside the window merge into
// one trailing pass.
const VAULT_CHANGE_COALESCE_MS = 300;
let pendingVaultChange: { folders: Set<string>; paths: string[] } | null = null;
let vaultChangeTimer: ReturnType<typeof setTimeout> | null = null;
let lastVaultFlushAt = 0;

function handleVaultChange(change: VaultChange): void {
  if (!pendingVaultChange) pendingVaultChange = { folders: new Set(), paths: [] };
  for (const f of change?.folders ?? []) pendingVaultChange.folders.add(f);
  for (const p of change?.paths ?? []) pendingVaultChange.paths.push(p);
  if (vaultChangeTimer) return; // a trailing flush is already scheduled
  const since = Date.now() - lastVaultFlushAt;
  if (since >= VAULT_CHANGE_COALESCE_MS) flushVaultChange();
  else vaultChangeTimer = setTimeout(flushVaultChange, VAULT_CHANGE_COALESCE_MS - since);
}

function flushVaultChange(): void {
  vaultChangeTimer = null;
  lastVaultFlushAt = Date.now();
  const merged = pendingVaultChange;
  pendingVaultChange = null;
  if (!merged) return;
  const folders = merged.folders;
  const paths = merged.paths;

  // The Vault tab's tree / search / graph and the wikilink+backlink index
  // mirror the whole tree, so any change refreshes them. The semantic index
  // (lenses + cleanup queue) reclassifies from the same files.
  refetchVaultTree();
  refetchVaultItems();
  if (noteIndex().length) refreshNoteIndex().catch(() => { /* retain the current index on transient errors */ });
  maybeReloadOpenNote(paths);

  if (folders.has('tasks')) refetchTasks();
  if (folders.has('projects')) refetchProjects();
  if (folders.has('commitments')) refreshCommitments();

  if (folders.has('workflows'))  {
    refetchWorkflows();
    // A hand edit may have opened, refreshed or retired a permission review.
    refreshReviewInbox();
  }
  // The operator-profile digest is compiled from the me/ folder.
  if (folders.has('me')) refetchOperatorProfile();
}

listen<VaultChange>('vault-changed', (e) => {
  if (e?.payload) handleVaultChange(e.payload);
}).catch(() => {/* not in tauri */});

// Outbound-send promise detection mutates the dot-prefixed review store, which
// the vault watcher intentionally ignores. The backend emits this explicit
// event so every mounted commitment panel refreshes without polling.
listen('commitment-proposals-changed', () => refreshCommitments()).catch(() => {/* not in tauri */});

// hollow_tasks propose_tasks (Paperclip PR 12) queues agent-task suggestions
// in the same dot-prefixed review store; the broker emits this so the Task
// suggestions panel refreshes mid-run without a notification.
listen('review-queue-changed', () => refreshReviewInbox()).catch(() => {/* not in tauri */});

// ─────────────────────────────────────────────────────────────
// NOTIFICATIONS — in-app notification center backed by the
// Rust `notifications` module (vault/.notifications.jsonl). The
// toast stack and bell-icon dropdown both read from `notifications`
// and react to the `notification-event` channel. Settings toggles
// gate the in-app toast pop and the native OS notification; the
// persistent center entry always lands so the user has a history.

const [_notificationsEnabled, setNotificationsEnabled] = createSignal(false);
/// How many notifications the file holds; `notifications` is the newest 200.
export const [notificationTotal, setNotificationTotal] = createSignal(0);
export const [notifications, { refetch: refetchNotifications }] =
  createResource(_notificationsEnabled, () => api.listNotifications().then(page => { setNotificationTotal(page.total); return page.items.filter(isWorkspaceNotification); }));

export const [unreadNotificationCount, setUnreadNotificationCount] = createSignal(0);

/// Mount hook — flips the gate on so the dropdown has data once the
/// user clicks the bell. Also called by `warmUpAllInBackground` so the
/// initial unread count is correct without waiting for a click.
export function enableNotificationsResource() { setNotificationsEnabled(true); }

export async function refetchUnreadNotificationCount(): Promise<void> {
  try { setUnreadNotificationCount(await api.unreadNotificationCount()); }
  catch { /* ignore — Tauri not ready, etc. */ }
}

export async function markNotificationRead(id: string): Promise<void> {
  try {
    await api.markNotificationRead(id);
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× mark read failed: ${errMsg(e)}`, 2400);
  }
}

export async function markAllNotificationsRead(): Promise<void> {
  try {
    await api.markAllNotificationsRead();
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× mark all read failed: ${errMsg(e)}`, 2400);
  }
}

/// The bell history's Mark listed read and Clear listed (P0-08, U3b): only
/// the ids it lists, and never a failure still waiting in Needs you.
export async function markNotificationsRead(ids: string[]): Promise<void> {
  try {
    await api.markNotificationsRead(ids);
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× mark read failed: ${errMsg(e)}`, 2400);
  }
}

export async function deleteNotifications(ids: string[]): Promise<void> {
  try {
    await api.deleteNotifications(ids);
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× clear failed: ${errMsg(e)}`, 2400);
  }
}

export async function deleteNotification(id: string): Promise<void> {
  try {
    await api.deleteNotification(id);
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× delete failed: ${errMsg(e)}`, 2400);
  }
}

export async function clearAllNotifications(): Promise<void> {
  // It deletes the whole file, so it asks first (P0-13).
  // The count only once the bell's history is read; before, no number.
  const n = notifications.state === 'ready' ? notificationTotal() : null;
  const ok = await showConfirm({
    title: 'Clear every notification?',
    body: `This deletes ${n === null ? 'every entry' : n === 1 ? 'the one entry' : `all ${n} entries`} in the bell's notification history, including failures. It can't be undone; the next event starts a new history.`,
    confirmLabel: 'Clear all', danger: true, focusCancel: true,
  });
  if (!ok) return;
  try {
    await api.clearNotifications();
    await Promise.all([refetchNotifications(), refetchUnreadNotificationCount()]);
  } catch (e) {
    flashFooter(`× clear failed: ${errMsg(e)}`, 2400);
  }
}

/// Settings toggles. `notifyInApp` controls the top-right toast stack;
/// `notifyNative` controls whether workflow completions also fire a
/// native macOS notification. Persistent record (center) is always
/// written — these only affect surface delivery.
export const [notifyInApp, setNotifyInAppSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('notifyInApp') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setNotifyInApp(v: boolean) {
  setNotifyInAppSignal(v);
  try { localStorage.setItem('notifyInApp', String(v)); } catch { /* ignore */ }
}
export const [notifyNative, setNotifyNativeSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('notifyNative') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setNotifyNative(v: boolean) {
  setNotifyNativeSignal(v);
  try { localStorage.setItem('notifyNative', String(v)); } catch { /* ignore */ }
  // Sync to the backend so workflow-fired notifications honor the
  // toggle — workflows::finalize otherwise hardcodes notify_native=true.
  api.setNotifyNativeEnabled(v).catch(() => { /* not in tauri */ });
}
// Push the persisted value into the backend at boot so a restart
// doesn't silently re-enable native pops when the user previously
// turned them off.
api.setNotifyNativeEnabled(notifyNative()).catch(() => { /* not in tauri */ });

/// Proactive nudges: the background vault scan (meeting soon
/// unanswered, frogs carried 3+ days, projects gone cold) feeding the
/// notification center. Persisted; default on; synced to the backend
/// AtomicBool the scan loop checks each tick.
export const [nudgesEnabled, setNudgesEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('nudgesEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setNudgesEnabled(v: boolean) {
  setNudgesEnabledSignal(v);
  try { localStorage.setItem('nudgesEnabled', String(v)); } catch { /* ignore */ }
  api.setNudgesEnabled(v).catch(() => { /* not in tauri */ });
}
api.setNudgesEnabled(nudgesEnabled()).catch(() => { /* not in tauri */ });

/// Local semantic search (candle embeddings, on-device). UNLIKE the other
/// automation toggles this defaults OFF — enabling it starts the background
/// backfill that embeds the vault, which is opt-in for cost + privacy reasons
/// (inference is on-device, but the user chooses when to index their notes).
/// Persisted; synced to the backend AtomicBool the worker + semantic_search read.
export const [semanticSearchEnabled, setSemanticSearchEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return localStorage.getItem('semanticSearchEnabled') === 'true'; }
    catch { return false; }
  })()
);
export function setSemanticSearchEnabled(v: boolean) {
  setSemanticSearchEnabledSignal(v);
  try { localStorage.setItem('semanticSearchEnabled', String(v)); } catch { /* ignore */ }
  api.setSemanticSearchEnabled(v).catch(() => { /* not in tauri */ });
}
api.setSemanticSearchEnabled(semanticSearchEnabled()).catch(() => { /* not in tauri */ });

/// Keep Hollow running when the window closes. On → closing the main window
/// hides it to the menu-bar tray (today's behavior, pairs with run-at-login +
/// the heartbeat); off → closing quits the app. Persisted; default on; synced
/// to the backend AtomicBool the `lib.rs` close handler reads.
export const [minimizeToTray, setMinimizeToTraySignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('minimizeToTray') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setMinimizeToTray(v: boolean) {
  setMinimizeToTraySignal(v);
  try { localStorage.setItem('minimizeToTray', String(v)); } catch { /* ignore */ }
  api.setMinimizeToTray(v).catch(() => { /* not in tauri */ });
}
api.setMinimizeToTray(minimizeToTray()).catch(() => { /* not in tauri */ });

/// Launch Hollow at login. Unlike the other toggles this is NOT a localStorage
/// mirror — the OS login item is the source of truth, so the signal is seeded
/// from the backend at boot and the setter reconciles to whatever the OS
/// actually reports (so a refused enable, e.g. MDM-restricted login items,
/// never leaves the UI falsely "on"). Per the everything-on default, the FIRST
/// launch with this feature enables it once (guarded by a one-time marker) so
/// Hollow comes up in the tray on later logins; after that the user's choice
/// sticks.
export const [runOnStartup, setRunOnStartupSignal] = createSignal<boolean>(false);
/// Returns the native call's promise (settled, never rejected), so the app
/// bridge's queued-settings drain acknowledges only after it finished.
export function setRunOnStartup(v: boolean): Promise<void> {
  return api.setRunOnStartup(v)
    .then((actual) => { setRunOnStartupSignal(actual); })
    // A refusal (a review copy of Hollow, an MDM-restricted login item) says
    // why; outside Tauri there is nothing to say (plan completion L5).
    .catch((e) => { if (typeof e === 'string' && e) flashFooter(`× ${e}`); });
}
void (async () => {
  try {
    const seeded = (() => {
      try { return localStorage.getItem('autostartSeeded') === '1'; } catch { return false; }
    })();
    if (!seeded) {
      const actual = await api.setRunOnStartup(true); // everything-on: enable once
      setRunOnStartupSignal(actual);
      try { localStorage.setItem('autostartSeeded', '1'); } catch { /* ignore */ }
    } else {
      setRunOnStartupSignal(await api.runOnStartupStatus());
    }
  } catch { /* not in tauri */ }
})();

/// Silent agentic heartbeat — periodic background review where Claude acts /
/// surfaces autonomously. Persisted; **default on** (the everything-on
/// posture); synced to the backend AtomicBool the loop checks each tick.
export const [heartbeatEnabled, setHeartbeatEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('heartbeatEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setHeartbeatEnabled(v: boolean) {
  setHeartbeatEnabledSignal(v);
  try { localStorage.setItem('heartbeatEnabled', String(v)); } catch { /* ignore */ }
  api.setHeartbeatEnabled(v, true).catch(() => { /* not in tauri */ });
}
api.setHeartbeatEnabled(heartbeatEnabled()).catch(() => { /* not in tauri */ });

/// Heartbeat sub-toggle: when off, the heartbeat runs notify-only (it won't
/// write drafts into the vault). Default on per the full-autonomy choice; turn
/// off for the most conservative posture.
export const [heartbeatDraftsEnabled, setHeartbeatDraftsEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('heartbeatDraftsEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setHeartbeatDraftsEnabled(v: boolean) {
  setHeartbeatDraftsEnabledSignal(v);
  try { localStorage.setItem('heartbeatDraftsEnabled', String(v)); } catch { /* ignore */ }
  api.setHeartbeatDraftsEnabled(v).catch(() => { /* not in tauri */ });
}
api.setHeartbeatDraftsEnabled(heartbeatDraftsEnabled()).catch(() => { /* not in tauri */ });

/// Heartbeat tunables — work-hours window, min spacing, daily budget.
/// Persisted as JSON; synced to the backend at boot + on change.
const HEARTBEAT_CONFIG_DEFAULT: HeartbeatConfig = {
  workHoursStart: 7, workHoursEnd: 19, minIntervalMin: 60, dailyBudget: 10,
};
export const [heartbeatConfig, setHeartbeatConfigSignal] = createSignal<HeartbeatConfig>(
  (() => {
    try {
      const raw = localStorage.getItem('heartbeatConfig');
      if (raw) return { ...HEARTBEAT_CONFIG_DEFAULT, ...JSON.parse(raw) };
    } catch { /* ignore */ }
    return HEARTBEAT_CONFIG_DEFAULT;
  })()
);
export function setHeartbeatConfig(cfg: HeartbeatConfig) {
  setHeartbeatConfigSignal(cfg);
  try { localStorage.setItem('heartbeatConfig', JSON.stringify(cfg)); } catch { /* ignore */ }
  api.setHeartbeatConfig(cfg).catch(() => { /* not in tauri */ });
}
api.setHeartbeatConfig(heartbeatConfig()).catch(() => { /* not in tauri */ });

/// Agent teamwork: when on, a Claude chat/workflow can delegate subtasks to
/// the Codex/Antigravity teammate CLIs (image gen, web browsing, coding) via
/// the in-app MCP team-server. Persisted; default on. Synced to the backend
/// flag that gates whether the team server is attached to Claude runs.
export const [teamworkEnabled, setTeamworkEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('teamworkEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setTeamworkEnabled(v: boolean) {
  setTeamworkEnabledSignal(v);
  try { localStorage.setItem('teamworkEnabled', String(v)); } catch { /* ignore */ }
  api.setTeamworkEnabled(v).catch(() => { /* not in tauri */ });
}
// Push the persisted value at boot so the backend flag matches the UI.
api.setTeamworkEnabled(teamworkEnabled()).catch(() => { /* not in tauri */ });

/// Teammate role presets — name each delegate by function ("Codex =
/// Engineer") with optional standing instructions that ride ahead of every
/// delegated task. Persisted; pushed to the backend, which carries them to
/// the team MCP server via env on the next chat/workflow spawn.
export interface AgentRolePreset { role: string; brief: string }
export const DEFAULT_AGENT_ROLES: Record<ChatAgent, AgentRolePreset> = {
  claude: { role: 'Collaborator', brief: '' },
  codex: { role: 'Engineer', brief: '' },
  antigravity: { role: 'Designer & web researcher', brief: '' },
  grok: { role: 'Researcher', brief: '' },
};
export const [agentRoles, setAgentRolesSignal] = createSignal<Record<ChatAgent, AgentRolePreset>>(
  (() => {
    try {
      const raw = localStorage.getItem('agentRoles');
      if (raw) return { ...DEFAULT_AGENT_ROLES, ...JSON.parse(raw) };
    } catch { /* fall through */ }
    return DEFAULT_AGENT_ROLES;
  })()
);
export function setAgentRole(agent: ChatAgent, preset: AgentRolePreset) {
  const next = { ...agentRoles(), [agent]: preset };
  setAgentRolesSignal(next);
  try { localStorage.setItem('agentRoles', JSON.stringify(next)); } catch { /* ignore */ }
  api.setAgentRole(agent, preset.role, preset.brief).catch(() => { /* not in tauri */ });
}
// Push persisted presets at boot so the first spawn already carries them.
for (const [agent, preset] of Object.entries(agentRoles())) {
  api.setAgentRole(agent, preset.role, preset.brief).catch(() => { /* not in tauri */ });
}

/// Computer control: when on, the embedded Claude chat gets vetted desktop
/// verbs via the in-app computer-MCP server — open URLs/apps immediately
/// (Tier 1) and read browser tabs / quit apps behind a native Allow/Deny
/// dialog (Tier 2, AppleScript). Persisted; default on. Synced to the
/// backend flag that gates whether the server (and the matching Bash
/// osascript/open deny rules) is attached to chat Claude runs.
export const [computerControlEnabled, setComputerControlEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('computerControlEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
// The two target switches (Cua CU7): the sandbox Mac (default on) and This
// Mac's screen (default off). Persisted; sent with the Basic switch.
const boolPref = (key: string, fallback: boolean): boolean => {
  try { const v = localStorage.getItem(key); return v === null ? fallback : v === 'true'; }
  catch { return fallback; }
};
export const [computerSandboxEnabled, setComputerSandboxSignal] = createSignal<boolean>(boolPref('computerSandboxEnabled', true));
export const [computerThisMacEnabled, setComputerThisMacSignal] = createSignal<boolean>(boolPref('computerThisMacEnabled', false));
const pushComputer = () =>
  api.setComputerControlEnabled(computerControlEnabled(), computerSandboxEnabled(), computerThisMacEnabled()).catch(() => { /* not in tauri */ });
export function setComputerControlEnabled(v: boolean) {
  setComputerControlEnabledSignal(v);
  try { localStorage.setItem('computerControlEnabled', String(v)); } catch { /* ignore */ }
  pushComputer();
}
export function setComputerSandboxEnabled(v: boolean) {
  setComputerSandboxSignal(v);
  try { localStorage.setItem('computerSandboxEnabled', String(v)); } catch { /* ignore */ }
  pushComputer();
}
export function setComputerThisMacEnabled(v: boolean) {
  setComputerThisMacSignal(v);
  try { localStorage.setItem('computerThisMacEnabled', String(v)); } catch { /* ignore */ }
  pushComputer();
}
// Push the persisted values at boot so the backend flags match the UI (one call).
pushComputer();

/// Ask your vault: when on, the embedded Claude chat gets the in-app
/// vault-MCP server — a read-only `search_vault` tool that keyword-searches
/// the whole vault and returns ranked snippets, so the agent grounds answers
/// in the user's own notes before reaching for general knowledge. Read-only +
/// non-mutating, so no approval. Persisted; default on. Chat-only.
export const [vaultMcpEnabled, setVaultMcpEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('vaultMcpEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setVaultMcpEnabled(v: boolean) {
  setVaultMcpEnabledSignal(v);
  try { localStorage.setItem('vaultMcpEnabled', String(v)); } catch { /* ignore */ }
  api.setVaultMcpEnabled(v).catch(() => { /* not in tauri */ });
}
// Push the persisted value at boot so the backend flag matches the UI.
api.setVaultMcpEnabled(vaultMcpEnabled()).catch(() => { /* not in tauri */ });

/// Look up code (CC2): whether Claude and Codex chats and workflows in a git
/// checkout get the read-only `hollow_code_context` server (map, symbols,
/// callers, callees, impact, read). Persisted; default on.
export const [codeContextMcpEnabled, setCodeContextMcpEnabledSignal] = createSignal<boolean>(
  (() => {
    try { return (localStorage.getItem('codeContextMcpEnabled') ?? 'true') === 'true'; }
    catch { return true; }
  })()
);
export function setCodeContextMcpEnabled(v: boolean) {
  setCodeContextMcpEnabledSignal(v);
  try { localStorage.setItem('codeContextMcpEnabled', String(v)); } catch { /* ignore */ }
  api.setCodeContextMcpEnabled(v).catch(() => { /* not in tauri */ });
}
api.setCodeContextMcpEnabled(codeContextMcpEnabled()).catch(() => { /* not in tauri */ });

/// Task tools (Paperclip PR 12): whether Claude and Codex chats and
/// automations get the `hollow_tasks` server. Unlike the toggles above, the
/// BACKEND owns this switch (agent-tools.json, loaded before the run queue
/// starts, since an automation can run before this webview boots), so the
/// signal reads it at boot and never persists to localStorage.
export const [tasksMcpEnabled, setTasksMcpEnabledSignal] = createSignal<boolean>(true);
export function setTasksMcpEnabled(v: boolean) {
  const previous = tasksMcpEnabled();
  setTasksMcpEnabledSignal(v);
  api.setTasksMcpEnabled(v).catch((e) => {
    setTasksMcpEnabledSignal(previous);
    flashFooter(`× Task tools: ${errMsg(e)}`);
  });
}
api.tasksMcpEnabled()
  .then(setTasksMcpEnabledSignal)
  .catch(() => { /* not in tauri */ });

/// Hollow app tools (PR A1): whether Claude and Codex chats and automations
/// get the `hollow_app` server. Backend-owned like the task tools.
export const [appMcpEnabled, setAppMcpEnabledSignal] = createSignal<boolean>(true);
export function setAppMcpEnabled(v: boolean) {
  const previous = appMcpEnabled();
  setAppMcpEnabledSignal(v);
  api.setAppMcpEnabled(v).catch((e) => {
    setAppMcpEnabledSignal(previous);
    flashFooter(`× Hollow app tools: ${errMsg(e)}`);
  });
}
api.appMcpEnabled()
  .then(setAppMcpEnabledSignal)
  .catch(() => { /* not in tauri */ });

/// In-flight toast slots. Each entry is the persisted Notification plus
/// an auto-dismiss timer the toast component manages. Capped at 5 visible
/// at a time — newer pops push older ones up the stack.
import type { Notification as NotificationT } from '../types';

function pushNotificationToast(n: NotificationT) {
  if (!notifyInApp()) return;
  if (suppressToast(n)) return;
  addNotificationToast(n);
}

/// Click handler shared by the toast, Recent activity's rows and the
/// "waiting on you" surface's failure rows (which carry only the
/// notification's id and link). Routes by the notification's action_url
/// scheme — `workflow:`, `vault:`, or http(s).
export function activateNotification(n: Pick<NotificationT, 'id' | 'read' | 'actionUrl'>): void {
  // Mark read first so the badge updates immediately; the navigation is
  // best-effort. Both happen in parallel — the user shouldn't wait for
  // an IPC round-trip before the view changes.
  if (!n.read) markNotificationRead(n.id);
  dismissNotificationToast(n.id);

  const url = n.actionUrl;
  if (!url) return;
  const streakSlug = streakActionSlug(url);
  if (url.startsWith('budget-pauses:')) {
    // A usage budget paused automations (Paperclip PR 8a).
    openBudgetPauses();
  } else if (streakSlug) {
    // A failure streak (Paperclip PR 9): open Automations with the workflow
    // selected and Waiting on you on its failing automations. The card was
    // queued after the workflow file's write, so refresh the queue too.
    setWorkflowsTab('workflows');
    setActivePage('workflows');
    refreshReviewInbox();
    requestSuggestions('automationHealth');
    requestOpenWorkflow(streakSlug);
  } else if (url.startsWith('workflow-permissions:')) {
    // A paused automation: open Automations with the workflow selected and
    // Waiting on you on its permission changes.
    const slug = url.slice('workflow-permissions:'.length);
    setWorkflowsTab('workflows');
    setActivePage('workflows');
    refreshReviewInbox();
    requestSuggestions('automations');
    if (slug) requestOpenWorkflow(slug);
  } else if (url.startsWith('workflow:')) {
    const slug = url.slice('workflow:'.length);
    // Force the Workflows sub-tab (not Skills) so the detail pane that
    // consumes workflowOpenRequest is actually mounted to receive it.
    setWorkflowsTab('workflows');
    setActivePage('workflows');
    requestOpenWorkflow(slug);
  } else if (url.startsWith('vault:')) {
    const rel = url.slice('vault:'.length);
    setActivePage('vault');
    requestOpenVaultNote(rel);
  } else if (url.startsWith('heartbeat-actions:')) {
    // Legacy action URLs remain useful after the standalone page retirement.
    refetchApprovals();
    requestSuggestions('tasks');
    setActivePage('tasks');
  } else if (url.startsWith('mission-control:')) {
    // Pause all's summary (P1b): Running, where the paused chats and Resume
    // all are.
    openWorkView('activity');
  } else if (url.startsWith('peer-chat:')) {
    // A routed turn ended on another Mac (M3a): open that chat there.
    const [machine, key] = url.slice(10).split('/');
    if (machine && key) void import('./peerChat').then(m => m.openRemoteChat(machine, key));
  } else if (url.startsWith('settings:')) {
    // Generic deep-link to a Settings section (e.g. "settings:heartbeat").
    const section = url.slice('settings:'.length) || 'connectors';
    openSettingsFromOutside(section);
  } else if (url.startsWith('http://') || url.startsWith('https://')) {
    // Defer to the OS — Tauri's opener plugin via window.open is fine
    // for external URLs.
    try { window.open(url, '_blank'); } catch { /* ignore */ }
  }
}

/// Cross-page "open this workflow" signal. Workflows.tsx subscribes to
/// this so that clicking a workflow notification jumps the user straight
/// to the right detail pane.
export const [workflowOpenRequest, setWorkflowOpenRequest] = createSignal<{ slug: string; nonce: number } | null>(null);
export function requestOpenWorkflow(slug: string) {
  setWorkflowOpenRequest({ slug, nonce: Date.now() });
}

/// Cross-page "open this task in Tasks" signal. Tasks.tsx consumes it and
/// selects the task's detail pane.
/// A request with `path` names the task by its vault file instead (a
/// "waiting on you" item may know only that).
export const [taskOpenRequest, setTaskOpenRequest] = createSignal<{ url: string; path?: string; nonce: number } | null>(null);
export function requestOpenTask(url: string) {
  setActivePage('tasks');
  setTaskOpenRequest({ url, nonce: Date.now() });
}
export function requestOpenTaskAtPath(path: string) {
  setActivePage('tasks');
  setTaskOpenRequest({ url: '', path, nonce: Date.now() });
}

/// Deep-link to the Workflows tab with a specific workflow's detail pane open.
/// Used by the Today Activity ledger's workflow rows. Forces the Workflows
/// sub-tab (not Skills) so the pane that consumes `workflowOpenRequest` is
/// mounted to receive it.
export function openWorkflowBySlug(slug: string) {
  setWorkflowsTab('workflows');
  setActivePage('workflows');
  requestOpenWorkflow(slug);
}

/// Cross-page "open this Settings section" signal. Settings.tsx subscribes
/// so a notification (e.g. the heartbeat approval-queue "Review" CTA) can
/// jump straight to the right section instead of landing on the default one.
export const [settingsSectionRequest, setSettingsSectionRequest] = createSignal<{ id: string; nonce: number; control?: string } | null>(null);
/** Open a Settings section; `control` (a visible label from settingsSections'
 *  controls) scrolls to and highlights that control (audit H06). */
export function requestOpenSettingsSection(id: string, control?: string) {
  // Every id goes through the one resolver (Settings PR 1): a retired id
  // lands where it lives now, and a target that moved to another page opens it.
  // An empty control means none, so an alias id still lands on its label.
  const target = resolveSettingsTarget(id, control || undefined);
  // Settings showing: it gets the request, so its jump asks about unsaved
  // changes before leaving for the other page. Elsewhere it opens directly.
  if (target.kind === 'page') {
    if (activePage() !== 'settings') { void openSettingsPageTarget(target); return; }
    setSettingsSectionRequest({ id, nonce: Date.now(), ...(control ? { control } : {}) });
    return;
  }
  setSettingsSectionRequest({ id: target.section, nonce: Date.now(), ...(target.label ? { control: target.label } : {}) });
}

/** A link or agent step into Settings from another page. A target that moved
 *  to a Library tab or Automations opens there directly: no Settings step in
 *  Back, and no Settings mount first. From Settings itself it keeps the
 *  section request, so Settings' own jump asks about unsaved changes. Returns
 *  the resolved target, whether it opened directly, and a promise of whether it
 *  was accepted (a guard may refuse), so a caller names where the person landed
 *  only once it did. */
export function openSettingsFromOutside(id: string, control?: string): { target: SettingsTarget; opened: Promise<boolean>; direct: boolean } {
  const direct = directPageTarget(id, control, activePage() === 'settings');
  if (direct) return { target: direct, opened: openSettingsPageTarget(direct), direct: true };
  const target = resolveSettingsTarget(id, control || undefined);
  if (activePage() === 'settings') { requestOpenSettingsSection(id, control); return { target, opened: Promise.resolve(true), direct: false }; }
  // The section request is part of the guarded jump (overnight 11-13 S6): an
  // unsaved draft answered Keep editing leaves none pending for a later
  // Settings open, and `opened` says the jump was refused.
  let accepted = false;
  const asked = guardNavigation(() => { accepted = true; setActivePage('settings'); requestOpenSettingsSection(id, control); });
  const opened = asked instanceof Promise ? asked.then(() => accepted, () => false) : Promise.resolve(accepted);
  return { target, opened, direct: false };
}

/// Which sub-tab the Workflows page shows. Skills left it (Settings PR 4:
/// Library, Skills is their one home), so `skills` is never set any more and
/// `openSkills` opens Library; the value stays only so older callers compile.
export type WorkflowsTab = 'workflows' | 'skills';
export const [workflowsTab, setWorkflowsTab] = createSignal<WorkflowsTab>('workflows');

/// Open Library, Skills (Settings PR 4: Skills left Settings and the old
/// Workflows sub-tab). Palette skill rows, the chat header and Settings'
/// links come here.
export function openSkills() {
  void openSettingsPageTarget({ page: 'vault', tab: 'skills' });
}

/// Cross-page "open this skill" signal — mirrors workflowOpenRequest.
/// SkillsView consumes it so a palette skill row can land on the named
/// skill's detail pane instead of the bare Skills list.
export const [skillOpenRequest, setSkillOpenRequest] = createSignal<{ name: string; nonce: number } | null>(null);
export function requestOpenSkill(name: string) {
  setSkillOpenRequest({ name, nonce: Date.now() });
}

// Listener: refetch + pop a toast on every new notification.
listen<NotificationT>('notification-event', (e) => {
  if (!isWorkspaceNotification(e.payload)) return;
  // Defensive — if we get a malformed payload, skip rather than crash.
  if (!e.payload || !e.payload.id) return;
  if (_notificationsEnabled()) refetchNotifications();
  refetchUnreadNotificationCount();
  // A heartbeat "drafted N replies / extracted N tasks" notification means
  // the suggestion queue just grew — refresh contextual chat data.
  if (e.payload.actionUrl?.startsWith('heartbeat-actions:')) refetchApprovals();
  // A paused automation or a flagged run changed the (dot-prefixed, unwatched)
  // review queue: refresh every mounted review panel.
  // So did a failure streak opening (`workflow-failures:`), a finished
  // run's own notification (`workflow:`), whose success closed its streak,
  // and a usage budget pausing automations (`budget-pauses:`, PR 8a).
  const action = e.payload.actionUrl ?? '';
  if (action.startsWith('workflow-permissions:') || action.startsWith(STREAK_ACTION_SCHEME)
    || action.startsWith('workflow:') || action.startsWith('budget-pauses:')
    || e.payload.source === 'run-guard') refreshReviewInbox();
  pushNotificationToast(e.payload);
}).catch(() => { /* not in tauri */ });

// Boot — pull the unread count once at startup so the bell badge is
// accurate before the user clicks anything.
refetchUnreadNotificationCount();

// ─────────────────────────────────────────────────────────────
// PAGE CONTEXT — derives a compact snapshot of whatever tab the
// user is currently on (counts, top items, currently-open note)
// so the chat panel can prepend it to each prompt. Returns null
// when there's no useful context to share (chat page itself,
// graph view, ingest, settings).
// ─────────────────────────────────────────────────────────────

export interface PageContext {
  /** Short label for the pill ("Tasks", "Projects", "Calendar today"). */
  label: string;
  /** One-line summary shown in the pill ("12 overdue, 5 today"). */
  summary: string;
  /** Full markdown body prepended to the prompt. */
  body: string;
}

function trimTitle(s: string, n = 80): string {
  const oneLine = s.replace(/\s+/g, ' ').trim();
  return oneLine.length > n ? oneLine.slice(0, n - 1) + '…' : oneLine;
}

/** A one-line "why this note exists" per kind, for agent context. */

function chatContext(): PageContext | null {
  // The chat input prepends page-context, so when the user is *on* the
  // chat page there's no other tab to summarize. Instead, surface
  // Claude's own configuration — it's useful for Claude to know what
  // session/model/permission scope it's operating under — plus the
  // most recent past sessions so "what was I working on yesterday"
  // doesn't require a tool call.
  const sid = chatSessionId();
  const cwd = chatCwd();
  const model = chatModel();
  const mode = chatPermissionMode();
  const lines = [
    'The user is on the **Chat** tab — talking to the selected agent.',
    '',
    `Agent: ${AGENT_LABELS[chatAgent()]}`,
    `Session: ${sid ? sid : '(new — no session id yet)'}`,
    `Cwd: ${cwd ?? '(vault root)'}`,
    `Model: ${model}`,
    `Permission mode: ${mode}`,
  ];
  return {
    label: 'Chat',
    summary: `${model}/${mode}`,
    body: lines.join('\n'),
  };
}

// Set by the Workflows output view so chat page-context can include the
// workflow output the user is currently viewing (e.g. the morning brief).
// `null` when nothing is selected. `format` lets the builder text-strip HTML.
export const [activeBrief, setActiveBrief] =
  createSignal<{ date: string; content: string; format?: 'html' | 'md' } | null>(null);

/** Collapse an HTML brief to readable plain text for the chat page-context —
 *  drop <script>/<style> blocks, strip tags, decode the common entities. */

export function pageContext(): PageContext | null {
  // Only Conversation history (Library → Conversations) mounts the unscoped
  // composer; every workspace conversation carries its own workspace context.
  return conversationHistoryShown() ? chatContext() : null;
}
