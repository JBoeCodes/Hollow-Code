// Review's one keymap (P1-13): a shared letter means the same thing on a
// waiting row, a review card and Incoming, or nothing there. x dismisses (a
// notification: mark read; an Incoming row: its Reject, Mark read or
// Dismiss), a accepts, Shift-R rejects a card (with P0-10's Undo). e edits
// and z snoozes a card; Incoming's own e (file), t, 1–9 and Enter live in
// `triage.ts`. Deleting a notification is its button only. Arrows and j/k
// move between rows everywhere and aren't listed here.

export type ReviewKeyAction = 'dismiss' | 'accept' | 'reject' | 'edit' | 'snooze';
export type ReviewKeySurface = 'notification' | 'card' | 'incoming';
export const REVIEW_KEY_SURFACES: readonly ReviewKeySurface[] = ['notification', 'card', 'incoming'];

/** The DOM event Incoming's a and x send a review card to accept or reject it
 *  (detail 'accept' | 'reject'). Not a faked key press, so it doesn't depend
 *  on the card's own keymap (where reject is Shift-R). */
export const REVIEW_CARD_ACTION = 'review-card-action';

const KEYMAP: Record<ReviewKeySurface, Partial<Record<string, ReviewKeyAction>>> = {
  notification: { x: 'dismiss' },
  card: { a: 'accept', 'Shift-R': 'reject', e: 'edit', z: 'snooze' },
  // The shared letters only; `triageActions` names the same keys.
  incoming: { x: 'dismiss', a: 'accept' },
};

/** What a key does on that surface, or null when it does nothing there. */
export function reviewKey(surface: ReviewKeySurface, e: { key: string; shiftKey: boolean }): ReviewKeyAction | null {
  const id = e.shiftKey ? `Shift-${e.key.toUpperCase()}` : e.key;
  return KEYMAP[surface][id] ?? null;
}

/** When a snooze ends: 09:00 local the next morning. Before 05:00 the night
 *  isn't over yet, so it is this morning's 09:00. */
export function snoozeUntil(now: Date): Date {
  const wake = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 9, 0, 0, 0);
  if (now.getHours() >= 5) wake.setDate(wake.getDate() + 1);
  return wake;
}

/** "tomorrow 09:00", "today 09:00" or "Sat, Oct 31 09:00", for the snooze
 *  toast and the Snoozed list. */
export function wakeLabel(until: Date, now = new Date()): string {
  const time = until.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  if (until.toDateString() === now.toDateString()) return `today ${time}`;
  if (until.toDateString() === tomorrow.toDateString()) return `tomorrow ${time}`;
  return `${until.toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' })} ${time}`;
}
