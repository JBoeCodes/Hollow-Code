/**
 * Activity (next-level P19). Pure, unit-tested.
 *
 * Repeated failures fold into one row with a count ("Morning brief failed ·
 * 4 times since 7:00"), so one broken routine can't bury the item that needs
 * a decision. Folding only joins rows with the same kind, source, title,
 * body, destination and read state,
 * that are adjacent in time (within `windowMs`); a different failure in
 * between starts a new row. Unread wins: a group is unread if any member is.
 * A row `apart` matches never folds, either way: the bell's history lists
 * each unread failure on its own row, with its own Open and Mark read (U3b).
 */
import type { Notification } from './types';

export interface ActivityRow { head: Notification; repeats: number; ids: string[]; firstAt: string; unread: boolean }

/** An unread failure still waits in Needs you: listed bulk actions skip it. */
export function isWaitingFailure(n: Pick<Notification, 'read' | 'kind'>): boolean {
  return !n.read && n.kind === 'error';
}

export function foldRepeats(list: readonly Notification[], windowMs = 24 * 3_600_000, apart: (n: Notification) => boolean = () => false): ActivityRow[] {
  const out: ActivityRow[] = [];
  for (const n of list) {
    const last = out[out.length - 1];
    const same = last && !apart(n) && !apart(last.head) && last.head.kind === n.kind && last.head.source === n.source && last.head.title === n.title
      && last.head.body === n.body && last.head.actionUrl === n.actionUrl && last.head.actionLabel === n.actionLabel && last.head.read === n.read
      && Math.abs(Date.parse(last.firstAt) - Date.parse(n.created)) <= windowMs;
    if (same) { last.repeats++; last.ids.push(n.id); last.firstAt = n.created; last.unread ||= !n.read; }
    else out.push({ head: n, repeats: 1, ids: [n.id], firstAt: n.created, unread: !n.read });
  }
  return out;
}
