// Desktop finish announcements (0.4, 2026-11-20): the pure rules. A chat that finishes or
// needs you gets a native banner and counts toward the dock badge. Nothing here touches
// the window or the backend; ThreadNotificationCoordinator.tsx feeds it and acts on it.
import type { SessionState } from './sessionIndicator';

export type ChatEventKind = 'finished' | 'needs-you' | 'failed';
export interface ChatEvent { key: string; kind: ChatEventKind }

/** States that mean a turn is in flight: leaving one for idle is a finish. */
const IN_FLIGHT = new Set<SessionState>(['running', 'queued']);
/** States the person has to act on. They count toward the dock badge. */
const NEEDS_YOU = new Set<SessionState>(['waiting', 'failed']);

/** What changed between two snapshots of every chat's state. The first snapshot
 *  (`prev` null) announces nothing, so opening the app never replays old news. */
export function chatEvents(prev: ReadonlyMap<string, SessionState> | null, next: ReadonlyMap<string, SessionState>): ChatEvent[] {
  if (!prev) return [];
  const out: ChatEvent[] = [];
  const keys = new Set([...prev.keys(), ...next.keys()]);
  for (const key of keys) {
    const before = prev.get(key) ?? 'idle';
    const after = next.get(key) ?? 'idle';
    if (before === after) continue;
    if (after === 'waiting') out.push({ key, kind: 'needs-you' });
    else if (after === 'failed' && IN_FLIGHT.has(before)) out.push({ key, kind: 'failed' });
    else if (after === 'idle' && IN_FLIGHT.has(before)) out.push({ key, kind: 'finished' });
  }
  return out;
}

export interface QuietContext {
  /** The Settings switch. */
  enabled: boolean;
  /** Hollow is the frontmost, focused window. */
  focused: boolean;
  /** The chat on screen right now, if any. */
  onScreenKey: string | null;
  /** When each chat was last announced (ms), for the per-chat gap. */
  lastShown: ReadonlyMap<string, number>;
  now: number;
  /** Subagent chats never announce: their lead does. */
  isSubagent: (key: string) => boolean;
}

/** Least time between two banners for one chat, so a turn that flaps does not nag. */
export const MIN_GAP_MS = 20_000;

/** The quiet rules. A chat you are looking at while Hollow is focused is never
 *  announced; a subagent never is; one chat is not announced twice inside the gap. */
export function shouldAnnounce(event: ChatEvent, ctx: QuietContext): boolean {
  if (!ctx.enabled) return false;
  if (ctx.isSubagent(event.key)) return false;
  if (ctx.focused && ctx.onScreenKey === event.key) return false;
  const last = ctx.lastShown.get(event.key);
  if (last !== undefined && ctx.now - last < MIN_GAP_MS) return false;
  return true;
}

const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1).trimEnd()}…` : s);

/** One banner for a burst of events: the chat's own title for one, a count for several.
 *  Plain words, no dashes. Null for no events. */
export function bannerFor(events: readonly ChatEvent[], titleOf: (key: string) => string): { title: string; body: string } | null {
  if (events.length === 0) return null;
  const name = (key: string) => clip(titleOf(key).trim() || 'A chat', 60);
  if (events.length === 1) {
    const [e] = events;
    const body = e.kind === 'finished' ? 'Finished.' : e.kind === 'needs-you' ? 'Waiting on you.' : 'Stopped with an error.';
    return { title: name(e.key), body };
  }
  const needs = events.filter(e => e.kind !== 'finished').length;
  const done = events.length - needs;
  const parts = [needs ? `${needs} need${needs === 1 ? 's' : ''} you` : '', done ? `${done} finished` : ''].filter(Boolean);
  const names = events.slice(0, 2).map(e => name(e.key)).join(', ');
  return { title: `${events.length} chats: ${parts.join(', ')}`, body: events.length > 2 ? `${names} and ${events.length - 2} more.` : `${names}.` };
}

/** The dock badge: chats waiting on you or failed, plus finished chats not yet opened. */
export function badgeCount(states: ReadonlyMap<string, SessionState>, unseenFinished: ReadonlySet<string>, isSubagent: (key: string) => boolean = () => false): number {
  let n = 0;
  for (const [key, state] of states) if (NEEDS_YOU.has(state) && !isSubagent(key)) n++;
  for (const key of unseenFinished) if (!NEEDS_YOU.has(states.get(key) ?? 'idle') && !isSubagent(key)) n++;
  return Math.min(n, 99);
}
