/**
 * The quiet row under a reply (overnight 11-12, lane C2): copy, Useful and
 * Not useful, Read aloud, Retry on the latest reply, More, and when it
 * finished ("Sunday 11:27 PM"). Pure (`import type` erases), so the node
 * vitest suite can load it; `components/TurnFooter.tsx` draws the row.
 *
 * The thumbs save to the session store's `turn_feedback` table through
 * `record_turn_feedback` (the commands D1 kept when the Votes list left on
 * 11-08). That table is where Hollow already learns from a reply: the
 * friction loop (`friction.rs`) counts each Not useful vote, with its
 * reason, as a signal, and the run ledger reads both votes. No Votes page
 * comes back.
 */
import type { FeedbackVote } from './types';

/** The row's buttons, in order. */
export type ReplyRowButton = 'copy' | 'useful' | 'notUseful' | 'read' | 'retry' | 'more';

/** Which buttons the row shows: Copy always; the thumbs only on a reply with
 *  an exact run id (a vote is saved against it, nothing else); Read aloud
 *  when the reply has words and the WebView has a voice; Retry only where
 *  ChatBody offers it (the latest finished reply); More when it has items. */
export function replyRowButtons(o: { canVote: boolean; canRead: boolean; canRetry: boolean; hasMore: boolean }): ReplyRowButton[] {
  const out: ReplyRowButton[] = ['copy'];
  if (o.canVote) out.push('useful', 'notUseful');
  if (o.canRead) out.push('read');
  if (o.canRetry) out.push('retry');
  if (o.hasMore) out.push('more');
  return out;
}

/** A settled assistant reply with words and an exact run id, not the run
 *  still in flight, takes a vote. */
export function canVote(turn: { role: string; text: string; partial?: boolean; runId?: string }, liveRunId: string | null): boolean {
  if (turn.role !== 'assistant' || turn.partial || !turn.text.trim() || !turn.runId) return false;
  return turn.runId !== liveRunId;
}

/** When a finished reply finished, from the best time it has (overnight
 *  11-12, C8): its receipt's finish, else the reply turn's own `at` (a
 *  saved turn's end, `chatHydrate.ts` `savedTurnAt`, or the moment a live
 *  reply's result came, `markReply`). A reply without a receipt (an older
 *  run, a turn whose receipt wasn't written) still shows its time. */
export function replyFinishedAt(receipt: { finishedAt?: string } | null | undefined, at: string | undefined): string | undefined {
  for (const t of [receipt?.finishedAt, at]) if (t && Number.isFinite(Date.parse(t))) return t;
  return undefined;
}

/** When a reply finished, the way the row says it: `11:27 PM` today,
 *  `Sunday 11:27 PM` within the last week, `Nov 3, 11:27 PM` before that
 *  (with the year when it isn't this year). Null for a missing or bad time. */
export function replyTime(at: string | undefined, now = Date.now(), locale?: string): string | null {
  const t = Date.parse(at ?? '');
  if (!Number.isFinite(t)) return null;
  const when = new Date(t);
  const today = new Date(now);
  const time = when.toLocaleTimeString(locale, { hour: 'numeric', minute: '2-digit' });
  if (when.toDateString() === today.toDateString()) return time;
  const midnight = new Date(today.getFullYear(), today.getMonth(), today.getDate()).getTime();
  // Six days back from this morning: a weekday never names two dates.
  if (t >= midnight - 6 * 86_400_000 && t < midnight) {
    return `${when.toLocaleDateString(locale, { weekday: 'long' })} ${time}`;
  }
  const date = when.toLocaleDateString(locale, when.getFullYear() === today.getFullYear()
    ? { month: 'short', day: 'numeric' }
    : { month: 'short', day: 'numeric', year: 'numeric' });
  return `${date}, ${time}`;
}

/** What a thumb click saves: the clicked vote; a reason only with Not useful. */
export function voteInput(runId: string, vote: FeedbackVote, reason?: string | null): { runId: string; vote: FeedbackVote; reason: string | null } {
  const why = vote === 'notUseful' ? (reason ?? '').trim().slice(0, 2_000) : '';
  return { runId, vote, reason: why || null };
}

/** The short line after a vote is saved. */
export function voteSavedText(vote: FeedbackVote, withReason = false): string {
  if (vote === 'useful') return 'Thanks: marked useful';
  return withReason ? 'Saved: not useful, with your reason' : 'Saved: not useful';
}

/** A refused vote in plain words. Tauri rejects with the backend's string,
 *  which starts with a `FEEDBACK_*` code (`turn_feedback.rs`). */
export function voteErrorText(e: unknown): string {
  const message = e instanceof Error ? e.message : String(e);
  const code = /^(FEEDBACK_[A-Z_]+):\s*/.exec(message);
  switch (code?.[1]) {
    case 'FEEDBACK_NO_TURN': return 'Not saved: this reply has no saved turn record.';
    case 'FEEDBACK_TURN_RUNNING': return 'Not saved: this reply is still being written. Try again when it finishes.';
    case 'FEEDBACK_AMBIGUOUS_RUN': return 'Not saved: this reply’s run is recorded on more than one turn.';
    default: return `Not saved: ${code ? message.slice(code[0].length) : message}`;
  }
}
