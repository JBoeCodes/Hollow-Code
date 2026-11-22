/**
 * Copy for the neutral notice a stopped (or paused, P1a) chat turn leaves in
 * the transcript, and the pure pause helpers the store uses.
 *
 * The backend emits `{ type: 'cancelled', stop }` after it has ended every
 * process it could identify from the turn. `stop` says what it established:
 * `'verified'` (none of them remains), `'unconfirmed'` (a scan failed or a
 * process survived), or `null` when the turn was stopped before its agent
 * started. Dependency-free on purpose so the node vitest suite can load it.
 */
export type ChatStopOutcome = 'verified' | 'unconfirmed' | null | undefined;

export function cancelledNotice(stop: ChatStopOutcome): string {
  return stop === 'unconfirmed'
    ? 'Stopped. Hollow could not confirm that every process from this turn exited.'
    : 'Stopped.';
}

// ── Pause (P1a PR 2) ───────────────────────────────────────────────────────
// A paused turn ended at a safe point keeping its native session; the backend
// emits `{ type: 'paused', sessionKey, note, at }` after its receipt and
// before `done`. Neutral, like a stop: never an error.

/** Why a chat is paused (`chat_pause::PauseReason`): the person's Pause,
 *  Pause all (P1b) or a usage limit (P1d). */
export type PauseReason = 'user' | 'all' | 'usage_limit';

/** A paused conversation: the backend's one-line note and when (unix s);
 *  P1b to P1d add why and when it resumes on its own (unix s, or none). */
export interface PausedChat { note: string; at: number; reason?: PauseReason; resumeAt?: number | null }

/** The transcript's notice for a paused turn: "Paused: <note>". */
export function pausedNotice(note: string | null | undefined): string {
  const line = (note ?? '').trim().replace(/[.\s]+$/, '');
  return line ? `Paused: ${line}. Resume picks it up from here.` : 'Paused. Resume picks it up from here.';
}

/** What the person sees as the resume turn's message: the backend sends
 *  `chat_pause::RESUME_PROMPT` verbatim, so the bubble shows the same words. */
export const RESUME_PROMPT = 'Continue where you left off; the chat was paused.';

/** The chat rows to ask about after a reload (`probePausedRows`): each
 *  conversation's newest row, when it ended `cancelled` and wasn't asked
 *  about before. */
export function pausedRowsToProbe(
  rows: readonly { id: number; source: string; sourceId?: string | null; status: string }[],
  probed: ReadonlySet<number>,
): { id: number; key: string }[] {
  const newest = new Map<string, { id: number; status: string }>();
  for (const row of rows) {
    if (row.source !== 'chat' || !row.sourceId) continue;
    const prev = newest.get(row.sourceId);
    if (!prev || row.id > prev.id) newest.set(row.sourceId, { id: row.id, status: row.status });
  }
  return [...newest].filter(([, row]) => row.status === 'cancelled' && !probed.has(row.id)).map(([key, row]) => ({ id: row.id, key }));
}
