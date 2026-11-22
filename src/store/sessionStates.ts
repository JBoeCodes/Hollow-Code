// Every conversation's live state for the one indicator family (asks 10-11
// A1/A2). One memo over signals that already update from events (chat
// events, the run queue, approvals, the "waiting on you" list); nothing here
// polls. The rules are pure in `sessionIndicator.ts`.
import { createEffect, createMemo, createRoot, createSignal } from 'solid-js';
import { activePage, backgroundChatKeys, chatAgentSessionKey, chatBusy, conversationHistoryShown, pendingApprovals } from './runtime';
import { runQueue } from './runQueue';
import { waitingList } from './waitingOnYou';
import { missionFeed } from './missionControl';
import { workspaceId, workspaceSessions } from './workspaces';
import { pausedChats, probePausedRows } from './chatPause';
import { projectRollup, sessionStateIndex, type SessionRole, type SessionRollup, type SessionState, type SessionStateIndex } from '../sessionIndicator';

/** Kinds in "waiting on you" that pause a conversation until the person answers. */
const PAUSING_KINDS = new Set(['approval', 'question', 'canvas']);

/** Failures already looked at (key → failure token), kept on this Mac so a
 *  restart doesn't bring them back. The newest SEEN_MAX only. */
const SEEN_KEY = 'hollow.seenFailures';
const SEEN_MAX = 200;
function readSeen(): ReadonlyMap<string, string> {
  try {
    const raw = JSON.parse(localStorage.getItem(SEEN_KEY) ?? '[]') as unknown;
    return new Map(Array.isArray(raw) ? raw.filter((e): e is [string, string] => Array.isArray(e) && typeof e[0] === 'string' && typeof e[1] === 'string') : []);
  } catch { return new Map(); }
}
const [seenFailures, setSeenFailures] = createSignal<ReadonlyMap<string, string>>(readSeen());
function markSeen(key: string, token: string) {
  setSeenFailures(prev => {
    const next = new Map(prev);
    next.delete(key);
    next.set(key, token);
    while (next.size > SEEN_MAX) next.delete(next.keys().next().value!);
    try { localStorage.setItem(SEEN_KEY, JSON.stringify([...next])); } catch { /* private mode: this window only */ }
    return next;
  });
}

/** Two indexes say the same thing: a streamed token changes the feed but no
 *  conversation's state, and must not re-render every mark. */
function sameIndex(a: SessionStateIndex, b: SessionStateIndex): boolean {
  const same = (x: Map<string, string>, y: Map<string, string>) => x.size === y.size && [...x].every(([k, v]) => y.get(k) === v);
  return same(a.states, b.states) && same(a.failures, b.failures);
}

const [runStarts, setRunStarts] = createSignal<ReadonlyMap<string, number>>(new Map());

const { index, rollups } = createRoot(() => {
  const idx = createMemo(() => sessionStateIndex({
    onScreen: { key: chatAgentSessionKey(), busy: chatBusy() },
    background: backgroundChatKeys(),
    queue: runQueue(),
    waitingKeys: [
      ...pendingApprovals().map(a => a.sessionKey),
      ...waitingList.items().filter(i => PAUSING_KINDS.has(i.kind) && !(i.kind === 'question' && i.state !== 'pending')).map(i => i.sessionKey),
    ],
    feed: missionFeed(),
    seenFailures: seenFailures(),
    paused: Object.keys(pausedChats()),
  }), undefined, { equals: sameIndex });
  // P1a PR 2: after a reload no `paused` event has been seen; a chat whose
  // newest queue row ended cancelled is asked about once (`probePausedRows`).
  createEffect(() => probePausedRows(runQueue()));
  // Opening a failed conversation shows its error there; the mark has done
  // its job, so it clears (until a later turn fails). Only while it is on
  // screen: the key outlives a trip to another page.
  createEffect(() => {
    const onScreen = (activePage() === 'today' && !!workspaceId()) || conversationHistoryShown();
    const key = onScreen ? chatAgentSessionKey() : null;
    const token = key ? idx().failures.get(key) : undefined;
    if (key && token && seenFailures().get(key) !== token) markSeen(key, token);
  });
  // Every workspace's roll-up, once per change (the map reads it each frame).
  const ups = createMemo(() => {
    const out = new Map<string, SessionRollup>();
    const states = idx().states;
    for (const [id, group] of Object.entries(workspaceSessions())) {
      // The row counts every session in the project: the lead, its
      // subagents and any other chat there (running indicator, 11-11).
      const lead = group?.projectAgentKey;
      const listed = group?.sessions ?? [];
      const entries: { title: string; state: SessionState; role: SessionRole }[] = listed.map(s => ({ title: s.title ?? '', state: states.get(s.key) ?? 'idle', role: (s.key === lead ? 'lead' : s.forkKind === 'subagent' ? 'subagent' : 'chat') as SessionRole }));
      // The lead's turn may sit outside the newest window: count it anyway.
      if (lead && !listed.some(s => s.key === lead)) entries.push({ title: 'Lead', state: states.get(lead) ?? 'idle', role: 'lead' });
      const r = projectRollup(entries);
      if (r) out.set(id, r);
    }
    return out;
  });
  // When each conversation was first seen running (the sidebar's live
  // timer): the moment this window saw it start, or open if it was already
  // running. Dropped when it stops, so a later turn starts at zero.
  createEffect(() => {
    const states = idx().states;
    setRunStarts(prev => {
      const next = new Map<string, number>();
      let changed = false;
      for (const [key, state] of states) {
        if (state !== 'running') continue;
        const at = prev.get(key) ?? Date.now();
        if (!prev.has(key)) changed = true;
        next.set(key, at);
      }
      return changed || next.size !== prev.size ? next : prev;
    });
  });
  return { index: idx, rollups: ups };
});

/** Every conversation's state now (the announcer diffs successive snapshots). */
export const sessionStatesNow = (): ReadonlyMap<string, SessionState> => index().states;

/** One conversation's state now. */
export function sessionState(key: string | null | undefined): SessionState {
  return (key && index().states.get(key)) || 'idle';
}

/** A workspace's sessions rolled up (A1): null when every one is idle. */
export function workspaceRollup(id: string): SessionRollup | null {
  return rollups().get(id) ?? null;
}

/** When a running conversation was first seen running in this window, or
 *  null when it is not running. */
export function runningSince(key: string | null | undefined): number | null {
  return (key && runStarts().get(key)) || null;
}

/** The earliest running conversation's start in a workspace, or null when
 *  none of its conversations runs. */
export function workspaceRunningSince(id: string): number | null {
  const group = workspaceSessions()[id];
  let earliest: number | null = null;
  const keys = new Set<string>(group?.sessions.map(s => s.key) ?? []);
  if (group?.projectAgentKey) keys.add(group.projectAgentKey);
  for (const key of keys) {
    const at = runStarts().get(key);
    if (at !== undefined && (earliest === null || at < earliest)) earliest = at;
  }
  return earliest;
}
