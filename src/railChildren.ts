// Child chats as nested rows under their lead (t3code parity 3.6, B15). A subagent's chat is a
// session whose `forkKind` is `subagent` and whose `parentKey` names the chat that started it.
// In the sidebar it nests under that chat instead of being a row of its own. Pure.
import type { AgentSessionSummary } from './types';
import type { SessionState } from './sessionIndicator';
import { SESSION_STATE_ORDER } from './sessionIndicator';

export interface ChildSplit {
  /** Sessions that stay rows of their own: not a subagent, or its parent is not listed. */
  top: AgentSessionSummary[];
  /** A parent's listed subagent chats, oldest first (the order they were started in). */
  kids: Map<string, AgentSessionSummary[]>;
}

export const isSubagentChat = (s: AgentSessionSummary) => s.forkKind === 'subagent' && !!s.parentKey;

export function splitChildren(sessions: readonly AgentSessionSummary[]): ChildSplit {
  const keys = new Set(sessions.map(s => s.key));
  const top: AgentSessionSummary[] = [];
  const kids = new Map<string, AgentSessionSummary[]>();
  for (const s of sessions) {
    if (isSubagentChat(s) && s.parentKey !== s.key && keys.has(s.parentKey!)) {
      const list = kids.get(s.parentKey!) ?? [];
      list.push(s);
      kids.set(s.parentKey!, list);
    } else top.push(s);
  }
  for (const list of kids.values()) list.sort((a, b) => a.createdAt.localeCompare(b.createdAt));
  // A cycle (a names b, b names a) is reachable from no top row: those chats stay rows of their own.
  const reached = new Set<string>();
  const walk = (key: string) => { for (const c of kids.get(key) ?? []) if (!reached.has(c.key)) { reached.add(c.key); walk(c.key); } };
  for (const t of top) walk(t.key);
  for (const s of sessions) if (!reached.has(s.key) && !top.includes(s) && !top.some(t => t.key === s.key)) {
    top.push(s);
    for (const list of kids.values()) { const i = list.indexOf(s); if (i >= 0) list.splice(i, 1); }
  }
  return { top, kids };
}

/** Every chat nested under `key` at any depth, in start order level by level (for the deepest
 *  indent, which lists them flat rather than hiding them). */
export function flatDescendants(key: string, kids: ReadonlyMap<string, readonly AgentSessionSummary[]>): AgentSessionSummary[] {
  const out: AgentSessionSummary[] = [], seen = new Set<string>([key]);
  let level = [...(kids.get(key) ?? [])];
  while (level.length) {
    const next: AgentSessionSummary[] = [];
    for (const s of level) if (!seen.has(s.key)) { seen.add(s.key); out.push(s); next.push(...(kids.get(s.key) ?? [])); }
    level = next;
  }
  return out;
}

/** The most urgent state among the chats nested under `key` (not its own). */
export function descendantState(key: string, kids: ReadonlyMap<string, readonly AgentSessionSummary[]>, state: (key: string) => SessionState): SessionState {
  let best: SessionState = 'idle';
  for (const c of flatDescendants(key, kids)) {
    const s = state(c.key);
    if (s === 'idle') continue;
    if (best === 'idle' || SESSION_STATE_ORDER.indexOf(s) < SESSION_STATE_ORDER.indexOf(best)) best = s;
  }
  return best;
}

/** The most urgent state among a chat's own and its children's (all levels), so a parent whose
 *  subagent works or waits does not read as idle while the child sits inside a folded group. */
export function groupState(key: string, kids: ReadonlyMap<string, readonly AgentSessionSummary[]>, state: (key: string) => SessionState, seen: Set<string> = new Set()): SessionState {
  if (seen.has(key)) return 'idle';
  seen.add(key);
  let best = state(key);
  for (const child of kids.get(key) ?? []) {
    const s = groupState(child.key, kids, state, seen);
    if (s === 'idle') continue;
    if (best === 'idle' || SESSION_STATE_ORDER.indexOf(s) < SESSION_STATE_ORDER.indexOf(best)) best = s;
  }
  return best;
}

/** How many chats are nested under `key`, all levels. */
export function childCount(key: string, kids: ReadonlyMap<string, readonly AgentSessionSummary[]>, seen: Set<string> = new Set()): number {
  if (seen.has(key)) return 0;
  seen.add(key);
  let n = 0;
  for (const child of kids.get(key) ?? []) n += 1 + childCount(child.key, kids, seen);
  return n;
}

/** The words for a fold's button: "Show 2 subagent chats". */
export function childFoldLabel(count: number, open: boolean): string {
  return `${open ? 'Hide' : 'Show'} ${count} subagent chat${count === 1 ? '' : 's'}`;
}
