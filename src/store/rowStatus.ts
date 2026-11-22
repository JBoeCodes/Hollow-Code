// The facts behind a chat row's status marks (t3code parity 3.3): which ask is open, whether a
// plan awaits review, unread turns, and the linked pull request. The rules are pure in
// `rowStatus.ts`. Nothing here polls: every input is a signal that already updates from events.
import { createEffect, createMemo, createRoot, createSignal } from 'solid-js';
import { chatAgentSessionKey } from './chatRuntime';
import { pendingApprovals } from './runtime';
import { waitingList } from './waitingOnYou';
import { projectWorkspaces, railGitStates } from './workspaces';
import { visitedSeen, visitsVersion } from '../chatVisitMark';
import { unreadCount, rowMarks, type RowMark } from '../rowStatus';
import { sessionState } from './sessionStates';
import type { AgentSessionSummary } from '../types';

/** Chats with a plan proposal nobody has looked at: this window only, cleared
 *  when the chat is opened (the card is then on screen). */
const [planKeys, setPlanKeys] = createSignal<ReadonlySet<string>>(new Set());
export function markPlanReady(key: string) { setPlanKeys(prev => prev.has(key) ? prev : new Set(prev).add(key)); }

const { asks } = createRoot(() => {
  createEffect(() => {
    const open = chatAgentSessionKey();
    if (open && planKeys().has(open)) setPlanKeys(prev => { const next = new Set(prev); next.delete(open); return next; });
  });
  // Approval wins over input when a chat has both.
  const asks = createMemo(() => {
    const out = new Map<string, 'approval' | 'input'>();
    for (const i of waitingList.items()) {
      if (!i.sessionKey) continue;
      if (i.kind === 'approval') out.set(i.sessionKey, 'approval');
      else if ((i.kind === 'canvas' || (i.kind === 'question' && i.state === 'pending')) && !out.has(i.sessionKey)) out.set(i.sessionKey, 'input');
    }
    // An approval a subagent asks also shows on its parent's row.
    for (const a of pendingApprovals()) {
      if (a.sessionKey) out.set(a.sessionKey, 'approval');
      if (a.subagent?.parentKey) out.set(a.subagent.parentKey, 'approval');
      // A card raised to the COO is answered there: its row says so too.
      if (a.routedTo?.cooKey && a.kind !== 'm365') out.set(a.routedTo.cooKey, 'approval');
    }
    return out;
  });
  return { asks };
});

/** Build-workspace folders to their pull request, from the sidebar's git states. */
const prByCwd = createRoot(() => createMemo(() => {
  const out = new Map<string, { number: number; state: string | null }>();
  const states = railGitStates();
  for (const w of projectWorkspaces()) {
    const g = states[w.id];
    if (w.cwd && g?.prNumber) out.set(w.cwd, { number: g.prNumber, state: g.prState });
  }
  return out;
}));

/** One chat's marks. `merged` is the pull request an automatic settlement recorded. */
export function chatRowMarks(session: AgentSessionSummary, here: boolean, merged?: { number: number } | null): RowMark[] {
  visitsVersion();
  const pr = prByCwd().get(session.cwd) ?? (merged ? { number: merged.number, state: 'merged' } : null);
  return rowMarks({
    state: sessionState(session.key),
    ask: asks().get(session.key) ?? null,
    planReady: planKeys().has(session.key),
    unread: unreadCount(visitedSeen(session.key), session.turnCount),
    pr,
    current: here,
  });
}

/** What the sidebar's filters read about one chat. */
export function chatRailFacts(session: AgentSessionSummary, here: boolean): { state: ReturnType<typeof sessionState>; marks: RowMark['id'][] } {
  return { state: sessionState(session.key), marks: chatRowMarks(session, here, null).map(m => m.id) };
}
