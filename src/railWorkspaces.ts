/**
 * Which workspaces the rail lists, in order. Only workspaces you have had a
 * conversation in, never every workspace (frontier F4 listed every active one;
 * the user reverted that on 2026-09-29, because a long list pushed the rail's
 * bottom bar off screen and buried the ones in use):
 *   1. workspaces used here (a conversation started or opened), most recent first;
 *   2. then any other workspace with saved conversations, most recently
 *      modified first (covers conversations started before the used list
 *      existed, or from another window).
 * "Hide from sidebar" takes a workspace out until a conversation is started
 * or picked in it again. Nothing on disk changes either way. An archived
 * workspace is never listed (P1-38): Unarchive brings it back.
 */
import type { SessionState } from './sessionIndicator';

/** Partition before capping: attention and the current row always remain
 * visible. Working rows have one home, outside Recent, in source order. */
export function activityGroups<T>(rows: readonly T[], state: (row: T) => SessionState, current: (row: T) => boolean, limit: number): { active: T[]; working: T[] } {
  const attention: T[] = [], recent: T[] = [], working: T[] = [];
  let idleCount = 0;
  for (const row of rows) {
    const status = state(row);
    if (status === 'waiting' || status === 'failed' || status === 'interrupted' || status === 'paused') attention.push(row);
    else if (current(row)) recent.push(row);
    else if (status === 'running' || status === 'queued') working.push(row);
    else if (idleCount++ < limit) recent.push(row);
  }
  return { active: [...attention, ...recent], working };
}

export function railWorkspaceOrder<W extends { id: string; modified: string; status?: string }>(
  all: readonly W[], used: readonly string[], hidden: readonly string[], withSessions: readonly string[] = [],
): W[] {
  const byId = new Map(all.filter(w => w.status?.trim().toLowerCase() !== 'archived').map(w => [w.id, w]));
  const skip = new Set(hidden);
  const out: W[] = [];
  const seen = new Set<string>();
  for (const id of used) {
    const w = byId.get(id);
    if (!w || skip.has(id) || seen.has(id)) continue;
    seen.add(id); out.push(w);
  }
  const talked = new Set(withSessions);
  const rest = [...byId.values()]
    .filter(w => talked.has(w.id) && !seen.has(w.id) && !skip.has(w.id))
    .sort((a, b) => (b.modified || '').localeCompare(a.modified || ''));
  return [...out, ...rest];
}

/** The rail's workspace groups (next-level P02; Pinned group 2026-10-21):
 *  `pinned`, the pinned workspaces in pin order, whether or not they have a
 *  conversation (pins are the user's own choice, so they are never
 *  filtered), then `rest`, the conversation order above without them, so a
 *  pinned workspace is listed once. Pins that no longer exist are skipped. */
export function railGroups<W extends { id: string }>(railed: readonly W[], all: readonly W[], pinned: readonly string[]): { pinned: W[]; rest: W[] } {
  const byId = new Map(all.map(w => [w.id, w]));
  const out: W[] = [];
  const seen = new Set<string>();
  for (const id of pinned) {
    const w = byId.get(id);
    if (w && !seen.has(id)) { seen.add(id); out.push(w); }
  }
  const rest: W[] = [];
  for (const w of railed) if (!seen.has(w.id)) { seen.add(w.id); rest.push(w); }
  return { pinned: out, rest };
}
