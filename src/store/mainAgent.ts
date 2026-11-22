import { createEffect, createRoot, createSignal, on, onCleanup } from 'solid-js';
import { api } from '../api';
import { activePage, backgroundChatCwd, canLeaveConversation, chatAgentSessionKey, chatBusy, chatCwd, flashFooter, requestChatFocus, setActivePage, shownPage, type PageId } from './runtime';
import { newWorkspaceSessionOpened, selectWorkspace, sessionWorking, setWorkspaceConversationVisible, setWorkspaces, showWorkOverview, workspaceDetail, workspaceId, workspaceSessions, workspaces } from './workspaces';
import { MAIN_AGENT_ID, MAIN_AGENT_TITLE, isMainAgent } from '../workspaceTypes';
import { orderChildren, reopenableMainSessions, todaysMainSession, type MainAgentAct, type MainAgentChild, type MainAgentReturn, type MainAgentStatus } from '../mainAgent';
import { runQueue } from './runQueue';
import { sessionState } from './sessionStates';
import { mainAgentAttention, observedProvider, presence } from '../mainAgentSpace';
import { rememberWindowLabels } from '../windowLabels';
import { latestRequest } from '../latestRequest';
import { agentLabel } from '../agentLabel';

// The Main agent's entry points (main agent M1, 2026-10-07): the top bar's
// orb runs `toggleMainAgent`; its system-wide key and the tray item run
// `openMainAgent` (they bring Hollow forward on it, never away from it).

/** The Main agent is what the window shows. */
export const mainAgentOpen = () => activePage() === 'today' && isMainAgent(workspaceId()) && !!workspaceDetail();

/** The conversation on screen is a COO one, wherever it was opened from (the
 *  Main agent page, Library, Conversations): its folder is the Main agent's.
 *  A card raised to the COO is answered there (2026-11-20). */
export const cooChatShown = () => {
  const cwd = chatCwd();
  return !!cwd && workspaces().some(w => isMainAgent(w) && w.cwd === cwd);
};

/** A Main agent conversation is queued or running (the orb's light trails),
 *  including today's first turn before the session list has caught up. */
export function mainAgentWorking(): boolean {
  if (mainAgentOpen() && chatBusy()) return true;
  const sessions = workspaceSessions()[MAIN_AGENT_ID]?.sessions ?? [];
  return sessions.some(s => sessionWorking(s.key));
}

/** One consistent attention source for the top bar and the page's header. */
function ownAttention() {
  const keys = new Set((workspaceSessions()[MAIN_AGENT_ID]?.sessions ?? []).map(s => s.key));
  if (mainAgentOpen() && chatAgentSessionKey()) keys.add(chatAgentSessionKey()!);
  const entries = [...keys].map(key => ({ key, state: sessionState(key) }));
  return { ...mainAgentAttention(entries), failedSession: entries.find(e => e.state === 'failed')?.key ?? null };
}

/** What the orb and the page say now, from state already held: no added
 *  poll or read. */
export const mainAgentNow = () => {
  const attention = ownAttention();
  const now = presence(mainAgentWorking(), mainAgentChildren(), mainAgentStatus(), attention.waitingCount > 0,
    { failed: attention.failed, waitingCount: attention.waitingCount, nightRunning: 0 });
  const own = (workspaceSessions()[MAIN_AGENT_ID]?.sessions ?? []).filter(s => sessionWorking(s.key));
  const agents: (string | null)[] = [...own.map(s => s.agent), ...mainAgentChildren().filter(c => c.live).map(c => c.agent)];
  // A new turn on the page may precede its listed summary. Do not
  // substitute the next-message selector or a child's provider for it.
  if (mainAgentOpen() && chatBusy() && !own.some(s => s.key === chatAgentSessionKey())) agents.push(null);
  const provider = mainAgentChildrenRead() === 'ready' ? observedProvider(agents) : null;
  // This tint describes the recorded working chats, not other night work.
  return { ...now, provider, providerLabel: agentLabel(provider), waitingCount: attention.waitingCount, failedSession: attention.failedSession };
};

const [returnTo, setReturnTo] = createSignal<MainAgentReturn | null>(null);

// ── The subagents it started (M2) ───────────────────────────────────────
/** The subagents the COO started (live first, then newest): its presence
 *  and the orb read them; the conversation's Subagents chip and tab list
 *  them (COO rebuild L3). */
export const [mainAgentChildren, setMainAgentChildren] = createSignal<MainAgentChild[]>([]);
export const [mainAgentChildrenRead, setMainAgentChildrenRead] = createSignal<'loading' | 'ready' | 'error'>('loading');
export const [mainAgentRowsLoading, setMainAgentRowsLoading] = createSignal(false);
/** How many of them are queued or running (the orb's count). */
export const liveChildCount = () => mainAgentChildren().filter(c => c.live).length;
/** Its day and what it did on its own (M3 to M5). */
export const [mainAgentStatus, setMainAgentStatus] = createSignal<MainAgentStatus | null>(null);
export const [mainAgentStatusError, setMainAgentStatusError] = createSignal<string | null>(null);
export const [mainAgentStatusLoading, setMainAgentStatusLoading] = createSignal(false);
const statusReads = latestRequest();
/** Independent of the child list: a failed child read cannot hide its day. */
export async function refreshMainAgentStatus() {
  if (!workspaces().some(w => isMainAgent(w))) return;
  const seq = statusReads.start();
  setMainAgentStatusLoading(true);
  try {
    const status = await api.mainAgentStatus();
    if (!status) throw new Error('Status is unavailable.');
    if (statusReads.isLatest(seq)) { setMainAgentStatus(status); setMainAgentStatusError(null); }
  } catch (error) {
    if (statusReads.isLatest(seq)) setMainAgentStatusError(String(error));
  } finally {
    if (statusReads.isLatest(seq)) setMainAgentStatusLoading(false);
  }
}
export const [mainAgentActs, setMainAgentActs] = createSignal<MainAgentAct[]>([]);
export const [mainAgentActsRead, setMainAgentActsRead] = createSignal<'loading' | 'ready' | 'error'>('loading');
const childReads = latestRequest();
let childTimer: ReturnType<typeof setTimeout> | null = null;
/** Re-read the list (debounced: the run queue changes in bursts). Only once
 *  the Main agent exists; it has started nothing before that. */
export function refreshMainAgentChildren(delayMs = 400) {
  if (!workspaces().some(w => isMainAgent(w))) return;
  if (childTimer) clearTimeout(childTimer);
  childTimer = setTimeout(async () => {
    childTimer = null;
    void refreshMainAgentStatus();
    const seq = childReads.start();
    setMainAgentRowsLoading(true);
    try {
      const [list, acts, windows] = await Promise.all([
        api.mainAgentChildren().catch(() => null), api.mainAgentActivity().catch(() => null),
        api.automationWindows().catch(() => null),
      ]);
      if (childReads.isLatest(seq)) {
        if (windows) rememberWindowLabels(windows.windows);
        if (acts) setMainAgentActs(acts);
        setMainAgentActsRead(acts ? 'ready' : 'error');
        setMainAgentChildrenRead(list ? 'ready' : 'error');
      }
      if (list && childReads.isLatest(seq)) {
        // Unchanged rows keep their object, so the list keeps its DOM (and focus).
        const old = new Map(mainAgentChildren().map(c => [c.queueId, c]));
        setMainAgentChildren(orderChildren(list).map(c => {
          const prev = old.get(c.queueId);
          return prev && JSON.stringify(prev) === JSON.stringify(c) ? prev : c;
        }));
      }
    } catch { /* keep the last list; the next queue change retries */ }
    finally { if (childReads.isLatest(seq)) setMainAgentRowsLoading(false); }
  }, delayMs);
}
createRoot(() => {
  // Every queue change can be a started chat starting or ending.
  createEffect(on(runQueue, () => refreshMainAgentChildren(), { defer: true }));
  createEffect(on(() => workspaces().some(w => isMainAgent(w)), has => { if (has) refreshMainAgentChildren(0); }));
  const refresh = () => { void refreshMainAgentStatus(); };
  if (typeof window !== 'undefined') {
    window.addEventListener('focus', refresh);
    window.addEventListener('hollow-main-agent-settings', refresh);
    onCleanup(() => { window.removeEventListener('focus', refresh); window.removeEventListener('hollow-main-agent-settings', refresh); });
  }
  createEffect(() => {
    const status = mainAgentStatus();
    if (!status) return;
    const now = Date.now();
    // One read at local midnight, when today's check-in count, spend and hard
    // stop reset (11-06; a cap reached last night read "paused" until a queue
    // event). Sooner, one read at a future check-in; one past due waits for
    // something new and check-in hours can end meanwhile, so one at the next
    // local hour. A stale response cannot schedule a loop: one timer per
    // status, at most one read an hour.
    const midnight = new Date(now); midnight.setHours(24, 0, 0, 500);
    const hour = new Date(now); hour.setMinutes(60, 0, 500);
    const next = Date.parse(status.nextCheckIn ?? '');
    const at = Number.isFinite(next) ? Math.min(midnight.getTime(), next > now ? next + 500 : hour.getTime()) : midnight.getTime();
    const timer = setTimeout(refresh, Math.min(at - now, 2_147_483_647));
    onCleanup(() => clearTimeout(timer));
  });
});
/** The Main agent conversation last on screen, and when it was first seen
 *  there: a thread started today counts as today's before the session list
 *  is refreshed, so pressing the orb again doesn't start a second one. */
let lastSeen: { key: string; at: string } | null = null;
createRoot(() => {
  createEffect(() => {
    const open = mainAgentOpen();
    // Leaving the Main agent by any other way (rail, palette, a page jump)
    // ends the trip: the orb then opens it again rather than "going back".
    if (!open) { setReturnTo(null); return; }
    const key = chatAgentSessionKey();
    // Only a conversation in the Main agent's own folder: while the window
    // switches, the global chat can still hold the previous workspace's key.
    if (!key || lastSeen?.key === key || chatCwd() !== workspaceDetail()?.workspace.cwd) return;
    // A listed conversation keeps its own start; an unlisted one is new.
    const listed = workspaceSessions()[MAIN_AGENT_ID]?.sessions.find(s => s.key === key);
    lastSeen = { key, at: listed?.createdAt ?? new Date().toISOString() };
  });
});

let opening = false;

/** Open today's Main agent conversation (creating the workspace on first
 *  use), remembering where you were so the orb can take you back. Already
 *  open, it only focuses the composer. */
export async function openMainAgent(): Promise<boolean> {
  if (mainAgentOpen()) { setWorkspaceConversationVisible(true); requestChatFocus(); return true; }
  if (opening) return false;
  if (!canLeaveConversation()) { flashFooter('Wait a moment: the current message is still being sent.', 4000); return false; }
  opening = true;
  // `shownPage`: Library → Conversations is `chat`, so the orb goes back to it, not to Notes (S4).
  const from: MainAgentReturn = { page: shownPage(), workspaceId: workspaceId(), key: chatAgentSessionKey() };
  try {
    let main;
    try { main = await api.ensureMainAgentWorkspace(); }
    catch (e) { flashFooter(`× Couldn't open the ${MAIN_AGENT_TITLE}: ${String(e)}`, 6000); return false; }
    if (!workspaces().some(w => isMainAgent(w))) setWorkspaces([...workspaces(), main]);
    let sessions = workspaceSessions()[MAIN_AGENT_ID]?.sessions;
    if (!sessions) {
      try { sessions = (await api.readWorkspace(MAIN_AGENT_ID)).sessions; } catch { sessions = []; }
    }
    const known = reopenableMainSessions(sessions, lastSeen, new Date(), backgroundChatCwd, main.cwd);
    const key = todaysMainSession(known, new Date(), sessionWorking);
    if (key) await selectWorkspace(MAIN_AGENT_ID, key);
    else if (!await newWorkspaceSessionOpened(MAIN_AGENT_ID)) return false;
    if (!mainAgentOpen()) return false;
    setWorkspaceConversationVisible(true); requestChatFocus();
    if (!isMainAgent(from.workspaceId) || from.page !== 'today') setReturnTo(from);
    return true;
  } finally { opening = false; }
}

/** The orb: open the Main agent, or from it go back to where you were
 *  (Your work when it was opened some other way). */
export async function toggleMainAgent() {
  if (!mainAgentOpen()) { await openMainAgent(); return; }
  const back = returnTo();
  setReturnTo(null);
  // The global chat now holds the Main agent's conversation: reopen the
  // workspace conversation you left, then the page you were on.
  if (back?.workspaceId && !isMainAgent(back.workspaceId)) {
    await selectWorkspace(back.workspaceId, back.key ?? undefined);
  } else {
    showWorkOverview();
  }
  if (back && back.page !== 'today') setActivePage(back.page as PageId);
}
