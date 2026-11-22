// Settings, Chat, Chats at once (2026-10-20): the rows' pure model,
// so the node vitest suite can load it. The panel is
// `settings/ChatLimitsRows.tsx`; the backend `chat_limits.rs` checks again.
import type { ChatLimits } from './types';

/** Each limit's range, as the backend's `ChatLimits::validated`. */
export const CHAT_LIMIT_MAX: Record<keyof ChatLimits, number> = { chatsAtOnce: 60, chatsPerWorkspace: 25, subagentsPerChat: 20 };

/** A typed limit as a whole number in 1..max, or the error to show beside
 *  the input (fractions, 0, over the top and empty are all refused). */
export function parseChatLimit(text: string, max: number): { value: number } | { error: string } {
  const trimmed = text.trim();
  const n = Number(trimmed);
  if (trimmed === '' || !Number.isInteger(n) || n < 1 || n > max) return { error: `Enter a whole number from 1 to ${max}.` };
  return { value: n };
}

/** Saves one at a time, each built on the last state the backend
 *  acknowledged, so a second change made before the first reply never sends
 *  the first one's stale value back (a lowered limit stays lowered). A
 *  refused save re-reads the backend in the same line (delta review): the
 *  re-read can't land after, and undo, a later save. A failed re-read keeps
 *  the acknowledged state. `onSaved` gets every acknowledged state with the
 *  `tag` of the save it settles (`null` for the first load) and whether
 *  that save was refused; the panel's [`chatLimitDrafts`] decides what the
 *  inputs show. A callback that throws is logged and never stops the line. */
export function serialSaver<T extends object, Tag = null>(
  send: (next: T) => Promise<T>,
  read: () => Promise<T>,
  onSaved: (saved: T, tag: Tag | null, refused: boolean) => void,
  onError: (error: unknown) => void,
) {
  let acknowledged: T | null = null;
  let chain: Promise<void> = Promise.resolve();
  const safely = (call: () => void) => {
    try { call(); } catch (e) { console.error('[chat limits]', e); }
  };
  const reread = async (tag: Tag | null, refused: boolean) => {
    try { acknowledged = await read(); } catch { /* keep what was acknowledged */ }
    const now = acknowledged;
    if (now) safely(() => onSaved(now, tag, refused));
  };
  const enqueue = (step: () => Promise<void>) => {
    chain = chain.then(step).catch(e => console.error('[chat limits]', e));
    return chain;
  };
  return {
    /** What the backend last said. */
    set(saved: T) { acknowledged = saved; },
    /** Read the backend in line with the saves (the first load). */
    load(): Promise<void> {
      return enqueue(() => reread(null, false));
    },
    save(patch: Partial<T>, tag: Tag | null = null): Promise<void> {
      return enqueue(async () => {
        if (!acknowledged) return;
        let saved: T;
        try {
          saved = await send({ ...acknowledged, ...patch });
        } catch (e) {
          safely(() => onError(e));
          await reread(tag, true);
          return;
        }
        acknowledged = saved;
        safely(() => onSaved(saved, tag, false));
      });
    },
  };
}

/** Which edit a submission carried, per field (its edit revision). */
export type CarriedEdits = Partial<Record<keyof ChatLimits, number>>;

/** What each input shows (second delta review): per field, an edit
 *  revision bumped on every keystroke and every committed edit, the
 *  revision and draft the last submission carried, and the newest revision
 *  an answer has settled (saved, or refused and re-read). On every
 *  acknowledged answer a field takes the acknowledged value unless it has a
 *  newer edit still unsettled (typed but not committed, or submitted and
 *  pending); so an untouched field follows a change made elsewhere, and a
 *  refusal puts a field back only when the refused edit is still its
 *  latest, whatever the values. */
export function chatLimitDrafts() {
  const keys: Array<keyof ChatLimits> = ['chatsAtOnce', 'chatsPerWorkspace', 'subagentsPerChat'];
  const zero = () => ({ chatsAtOnce: 0, chatsPerWorkspace: 0, subagentsPerChat: 0 });
  const rev: Record<keyof ChatLimits, number> = zero();
  const submitted: Record<keyof ChatLimits, number> = zero();
  const settled: Record<keyof ChatLimits, number> = zero();
  const draft: Partial<ChatLimits> = {};
  return {
    /** A keystroke in `key` (not committed yet): an answer leaves it be. */
    typing(key: keyof ChatLimits) { rev[key] += 1; },
    /** A committed edit of `key` to `value`: what its submission carries. */
    edit(key: keyof ChatLimits, value: number): CarriedEdits {
      rev[key] += 1;
      submitted[key] = rev[key];
      draft[key] = value;
      return { [key]: rev[key] };
    },
    /** Typing in `key` was refused before it was sent (not a whole number
     *  in range): back to its last submission. */
    abandon(key: keyof ChatLimits) { rev[key] = submitted[key]; },
    /** The value `key` should show, given the acknowledged one: a
     *  submission still pending shows its draft. */
    shown(key: keyof ChatLimits, acknowledged: number): number {
      return submitted[key] > settled[key] ? (draft[key] ?? acknowledged) : acknowledged;
    },
    /** An acknowledged answer settling `carried` (`{}` for a plain read):
     *  the fields to set to the acknowledged values. */
    settle(carried: CarriedEdits, acknowledged: ChatLimits): Partial<ChatLimits> {
      for (const key of keys) settled[key] = Math.max(settled[key], carried[key] ?? 0);
      const out: Partial<ChatLimits> = {};
      for (const key of keys) if (rev[key] <= settled[key]) out[key] = acknowledged[key];
      return out;
    },
  };
}
