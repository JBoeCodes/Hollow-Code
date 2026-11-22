// The commitments ledger (overnight 10-11 A5): what I owe and what I'm owed.
// The screen was cut (overnight 11-08 S3). Pure: nothing here touches the store or the vault.
import type { Commitment } from './types';

export type LedgerScope = 'owed_by_me' | 'owed_to_me' | 'all';
export type LedgerStatus = 'open' | 'snoozed' | 'closed';

export { isAwake, isSnoozed } from './commitmentAwake';
import { isAwake, isSnoozed } from './commitmentAwake';

export function isOverdue(c: Commitment, today: string): boolean {
  return isAwake(c, today) && !!c.due && c.due.slice(0, 10) < today;
}

/** Which Status view shows `c`: Open, Snoozed or Closed. */
export function ledgerStatusOf(c: Commitment, today: string): LedgerStatus {
  return c.status !== 'open' ? 'closed' : isSnoozed(c, today) ? 'snoozed' : 'open';
}

export interface LedgerCounts { owedByMe: number; owedToMe: number; overdue: number; snoozed: number; closed: number }

export function ledgerCounts(list: readonly Commitment[], today: string): LedgerCounts {
  const awake = list.filter(c => isAwake(c, today));
  return {
    owedByMe: awake.filter(c => c.direction === 'owed_by_me').length,
    owedToMe: awake.filter(c => c.direction === 'owed_to_me').length,
    overdue: awake.filter(c => isOverdue(c, today)).length,
    snoozed: list.filter(c => isSnoozed(c, today)).length,
    closed: list.filter(c => c.status !== 'open').length,
  };
}

/** Whether a commitment matches the filter text: every word in its title,
 *  the other party, or their people note's path, case-insensitively. */
export function matchesQuery(c: Commitment, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = `${c.title} ${c.counterparty} ${c.personPath ?? ''}`.toLowerCase();
  return words.every(w => hay.includes(w));
}

/** The rows for a scope, status and filter, in reading order: open ones by
 *  due date (undated last), snoozed by wake day, closed newest first. */
export function ledgerRows(list: readonly Commitment[], opts: { scope: LedgerScope; status: LedgerStatus; query: string; today: string }): Commitment[] {
  const { scope, status, query, today } = opts;
  const inStatus = (c: Commitment) =>
    status === 'open' ? isAwake(c, today) : status === 'snoozed' ? isSnoozed(c, today) : c.status !== 'open';
  const rows = list.filter(c => inStatus(c) && (scope === 'all' || c.direction === scope) && matchesQuery(c, query));
  const byTitle = (a: Commitment, b: Commitment) => a.title.localeCompare(b.title) || a.url.localeCompare(b.url);
  if (status === 'snoozed') return rows.sort((a, b) => (a.snoozedUntil ?? '').localeCompare(b.snoozedUntil ?? '') || byTitle(a, b));
  if (status === 'closed') return rows.sort((a, b) => (b.fulfilledAt ?? b.created).localeCompare(a.fulfilledAt ?? a.created) || byTitle(a, b));
  return rows.sort((a, b) => (a.due ?? '9999').localeCompare(b.due ?? '9999') || byTitle(a, b));
}

export type LedgerGroupKey = 'overdue' | 'week' | 'later' | 'undated' | 'snoozed' | 'closed';
export interface LedgerGroup { key: LedgerGroupKey; label: string; rows: Commitment[] }

/** `YYYY-MM-DD` plus `days` (local calendar days). */
export function addDays(day: string, days: number): string {
  const d = new Date(`${day}T12:00:00`);
  d.setDate(d.getDate() + days);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const dd = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${dd}`;
}

/** Rows (already in `ledgerRows` order) under their headings: open ones as
 *  Overdue, This week (the next 7 days), Later and No date; empty groups
 *  are left out. */
export function ledgerGroups(rows: readonly Commitment[], status: LedgerStatus, today: string): LedgerGroup[] {
  if (status === 'snoozed') return rows.length ? [{ key: 'snoozed', label: 'Snoozed', rows: [...rows] }] : [];
  if (status === 'closed') return rows.length ? [{ key: 'closed', label: 'Closed', rows: [...rows] }] : [];
  const week = addDays(today, 7);
  const groups: LedgerGroup[] = [
    { key: 'overdue', label: 'Overdue', rows: [] },
    { key: 'week', label: 'This week', rows: [] },
    { key: 'later', label: 'Later', rows: [] },
    { key: 'undated', label: 'No date', rows: [] },
  ];
  for (const c of rows) {
    const due = c.due?.slice(0, 10);
    const g = !due ? 3 : due < today ? 0 : due <= week ? 1 : 2;
    groups[g].rows.push(c);
  }
  return groups.filter(g => g.rows.length);
}

export interface SnoozeChoice { key: '1' | '2' | '3'; label: string; until: string }

/** Snooze choices from `today`: tomorrow, next Monday (a week on from a
 *  Monday), and a week from today. */
export function snoozeChoices(today: string): SnoozeChoice[] {
  const dow = new Date(`${today}T12:00:00`).getDay();
  const toMonday = ((8 - dow) % 7) || 7;
  return [
    { key: '1', label: 'Tomorrow', until: addDays(today, 1) },
    { key: '2', label: 'Next Monday', until: addDays(today, toMonday) },
    { key: '3', label: 'In a week', until: addDays(today, 7) },
  ];
}

/** "Fri Oct 16" for a `YYYY-MM-DD`. */
export function shortDay(day: string): string {
  const d = new Date(`${day.slice(0, 10)}T12:00:00`);
  return Number.isNaN(d.getTime()) ? day : d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
}

/** Who owes whom, in words. */
export function owesLine(c: Pick<Commitment, 'direction' | 'counterparty'>): string {
  return c.direction === 'owed_by_me' ? `You owe ${c.counterparty}` : `${c.counterparty} owes you`;
}

/** The source's name for the citation: its file name without `.md`. */
export function sourceName(c: Commitment): string {
  if (!c.sourcePath) return '';
  const base = c.sourcePath.split('/').pop() ?? c.sourcePath;
  return base.replace(/\.md$/i, '');
}

/** What a change did, so `u` can put it back. */
/** The commitment as it was before a change, enough to undo it; `fulfilledAt`
 *  puts a fulfilled one's original stamp back (overnight 10-11/12 A9). */
export interface LedgerChange { url: string; title: string; did: string; status: Commitment['status']; snoozedUntil: string; fulfilledAt: string | null }

export function changeOf(c: Commitment, did: string): LedgerChange {
  return { url: c.url, title: c.title, did, status: c.status, snoozedUntil: c.snoozedUntil ?? '', fulfilledAt: c.fulfilledAt ?? null };
}

/** The ledger's keys, while it has focus and nothing is being typed in.
 *  Bare letters only: anything with ⌘, ⌃ or ⌥ passes through to the app and
 *  macOS. */
export type LedgerCommand =
  | 'down' | 'up' | 'open' | 'done' | 'drop' | 'snooze' | 'undo' | 'new' | 'search'
  | 'source' | 'person' | 'accept' | 'reject' | 'scope1' | 'scope2' | 'scope3' | 'close';

export function ledgerCommand(e: { key: string; metaKey?: boolean; ctrlKey?: boolean; altKey?: boolean }): LedgerCommand | null {
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  switch (e.key) {
    case 'j': case 'ArrowDown': return 'down';
    case 'k': case 'ArrowUp': return 'up';
    case 'Enter': return 'open';
    case 'd': return 'done';
    // Delete, not x: x selects a row on the Tasks tab beside this one, so it
    // isn't destructive here (P1-15).
    case 'Delete': case 'Backspace': return 'drop';
    case 's': return 'snooze';
    case 'u': return 'undo';
    case 'n': return 'new';
    case '/': return 'search';
    case 'o': return 'source';
    case 'p': return 'person';
    case 'a': return 'accept';
    case 'r': return 'reject';
    case '1': return 'scope1';
    case '2': return 'scope2';
    case '3': return 'scope3';
    case 'Escape': return 'close';
    default: return null;
  }
}
