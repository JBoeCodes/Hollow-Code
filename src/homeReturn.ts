/**
 * How a time reads ("12m ago"), for Results, Needs you and file views.
 * The return rows this module once picked for Home (next-level P02) went
 * with the unmounted HomeReturnRow (11-10). Pure, unit-tested.
 */
import { formatRelative } from './formatTime';

/** Compact relative time, then an absolute date after seven days. */
export function relativeWhen(iso: string | null | undefined, now = Date.now()): string {
  return formatRelative(Date.parse(iso ?? ''), now);
}
