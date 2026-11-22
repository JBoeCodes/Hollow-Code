// Paused chats (P1a PR 2): which conversations are paused (their turn ended
// at a safe point keeping its native session, `chat_pause`) and which have a
// pause asked and not reached yet. Keyed by session key, so the sidebar,
// Mission Control and the composer read one source. Fed by the `paused`
// chat event (store/chatRuntime.ts), by the backend's record when a
// conversation is opened (`agent_session_live(key)` phase `paused`), and by
// a probe of chat rows the run queue shows cancelled (a paused turn's queue
// row is `cancelled`). Imports only the API, so chatRuntime and
// sessionStates can both import it without a cycle.
import { createSignal } from 'solid-js';
import { api } from '../api';
import { latestRequest } from '../latestRequest';
import { pausedRowsToProbe, type PausedChat } from '../chatStop';

export type { PausedChat } from '../chatStop';

/** Paused conversations by key: the note and when (unix seconds). */
export const [pausedChats, setPausedChats] = createSignal<Readonly<Record<string, PausedChat>>>({});
/** Conversations with a pause asked that hasn't reached its safe point. */
export const [pausingChats, setPausingChats] = createSignal<ReadonlySet<string>>(new Set());

/** The `at` of the pause the person cleared (Resume), by key: a `paused`
 *  event no newer than it (a background chat's buffer replayed after a
 *  Resume) must not bring it back. It is that event's own `at`, never the
 *  clock: a clock stamp dropped a quick re-pause inside the same second
 *  (review fix 2). */
const clearedAt = new Map<string, number>();

export function chatPausedFor(key: string | null | undefined): PausedChat | null {
  return (key && pausedChats()[key]) || null;
}

export function chatPausingFor(key: string | null | undefined): boolean {
  return !!key && pausingChats().has(key);
}

/** Mark `key` paused (a `paused` event or the backend's record). An event
 *  older than this window's own clear of it is ignored. `more` (P1b to P1d):
 *  why, and when it resumes on its own; left out, what is known stays. */
export function markChatPaused(key: string, note: string, at: number, more: Pick<PausedChat, 'reason' | 'resumeAt'> = {}) {
  if (!key) return;
  const cleared = clearedAt.get(key);
  if (cleared !== undefined && at <= cleared) return;
  setPausedChats(cur => ({ ...cur, [key]: { ...cur[key], note, at, ...more } }));
  setChatPausing(key, false);
}

/** P1c: `key`'s resume time changed (set, or cleared with null). */
export function setPausedResumeAt(key: string, resumeAt: number | null) {
  const cur = pausedChats()[key];
  if (!cur) return;
  setPausedChats(all => ({ ...all, [key]: { ...cur, resumeAt } }));
}

/** `key` is no longer paused. `byPerson`: the person resumed or sent, so a
 *  replayed `paused` event from before now is ignored (see `clearedAt`). */
export function clearChatPaused(key: string | null | undefined, byPerson = false) {
  if (!key) return;
  const cur = pausedChats()[key];
  if (byPerson && cur) clearedAt.set(key, cur.at);
  if (!cur) return;
  setPausedChats(cur => {
    const next = { ...cur };
    delete next[key];
    return next;
  });
}

export function setChatPausing(key: string, on: boolean) {
  if (pausingChats().has(key) === on) return;
  setPausingChats(cur => {
    const next = new Set(cur);
    if (on) next.add(key); else next.delete(key);
    return next;
  });
}

const reads = new Map<string, ReturnType<typeof latestRequest>>();

/** Ask the backend whether `key` is paused (its record, as phase `paused`)
 *  and follow it: opening a conversation shows Resume after a reload. A
 *  failed read changes nothing. */
export async function refreshChatPaused(key: string | null | undefined): Promise<void> {
  if (!key) return;
  let req = reads.get(key);
  if (!req) { req = latestRequest(); reads.set(key, req); }
  const generation = req.start();
  try {
    const live = await api.agentSessionLive(key);
    if (!req.isLatest(generation)) return;
    if (live?.phase === 'paused') {
      const at = Date.parse(live.phaseEnteredAt);
      markChatPaused(key, chatPausedFor(key)?.note ?? '', Number.isFinite(at) ? Math.floor(at / 1000) : 0);
    } else if (live?.phase === 'pausing') {
      setChatPausing(key, true);
    } else {
      // Running, queued or idle: not paused (the record clears when any
      // turn of the chat is admitted), and no pause is being asked (the
      // backend reports `pausing` while one is): a stale flag goes too.
      clearChatPaused(key);
      setChatPausing(key, false);
    }
  } catch { /* the state stays as it was */ }
}

/** Chats whose Resume is in flight (review fix 1): a second click, from the
 *  composer, Mission Control or `/resume`, does nothing until it answers. */
export const [resumingChats, setResumingChats] = createSignal<ReadonlySet<string>>(new Set<string>());
export function chatResumingFor(key: string | null | undefined): boolean {
  return !!key && resumingChats().has(key);
}
/** Take `key`'s Resume slot: false when one is already in flight. */
export function beginResume(key: string): boolean {
  if (resumingChats().has(key)) return false;
  setResumingChats(cur => new Set(cur).add(key));
  return true;
}
export function endResume(key: string) {
  setResumingChats(cur => { const next = new Set(cur); next.delete(key); return next; });
}

/** Queue rows already probed, so each cancelled chat row is asked once. */
const probedRows = new Set<number>();

/** After a reload this window has seen no `paused` event: ask about each
 *  chat whose newest run-queue row is `cancelled` (how a paused turn's row
 *  ends), once per row. The queue keeps a short history, so this is bounded. */
export function probePausedRows(rows: readonly { id: number; source: string; sourceId?: string | null; status: string }[]) {
  for (const { id, key } of pausedRowsToProbe(rows, probedRows)) {
    probedRows.add(id);
    void refreshChatPaused(key);
  }
}

/** Tests: forget everything. */
export function resetChatPauseState() {
  setPausedChats({});
  setPausingChats(new Set<string>());
  setResumingChats(new Set<string>());
  clearedAt.clear();
  probedRows.clear();
  reads.clear();
}
