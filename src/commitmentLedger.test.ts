import { describe, expect, it } from 'vitest';
import {
  addDays, changeOf, isOverdue, isSnoozed, ledgerCommand, ledgerCounts, ledgerGroups, ledgerRows, matchesQuery, owesLine, snoozeChoices, sourceName,
} from './commitmentLedger';
import type { Commitment } from './types';

const c = (title: string, over: Partial<Commitment> = {}): Commitment => ({
  url: `vault:commitments/${title}.md`, vaultPath: `commitments/${title}.md`, title, direction: 'owed_by_me', counterparty: 'Dana Lee', personPath: null,
  projectUrl: null, due: null, status: 'open', channel: 'meeting', sourcePath: null, created: '2026-10-01T09:00:00Z', fulfilledAt: null, fingerprint: title, context: '',
  ...over,
});

const TODAY = '2026-10-11'; // a Sunday

describe('commitments ledger (overnight 10-11 A5)', () => {
  const list = [
    c('Late deck', { due: '2026-10-09' }),
    c('Budget', { due: '2026-10-14' }),
    c('Board pack', { due: '2026-10-30' }),
    c('Intro', {}),
    c('Their export', { direction: 'owed_to_me', counterparty: 'Raj', due: '2026-10-10' }),
    c('Snoozed call', { due: '2026-10-05', snoozedUntil: '2026-10-13' }),
    c('Woke today', { due: '2026-10-08', snoozedUntil: TODAY }),
    c('Sent', { status: 'fulfilled', fulfilledAt: '2026-10-10T10:00:00Z' }),
    c('Dropped', { status: 'cancelled', created: '2026-10-02T09:00:00Z' }),
  ];

  it('a snooze holds until its day, and only on open ones', () => {
    expect(isSnoozed(list[5], TODAY)).toBe(true);
    expect(isSnoozed(list[6], TODAY)).toBe(false);
    expect(isSnoozed({ ...list[5], status: 'fulfilled' }, TODAY)).toBe(false);
    expect(isOverdue(list[5], TODAY)).toBe(false);
    expect(isOverdue(list[6], TODAY)).toBe(true);
  });

  it('counts awake ones each way, overdue, snoozed and closed', () => {
    expect(ledgerCounts(list, TODAY)).toEqual({ owedByMe: 5, owedToMe: 1, overdue: 3, snoozed: 1, closed: 2 });
  });

  it('orders open rows by due date, undated last, and groups them', () => {
    const rows = ledgerRows(list, { scope: 'owed_by_me', status: 'open', query: '', today: TODAY });
    expect(rows.map(r => r.title)).toEqual(['Woke today', 'Late deck', 'Budget', 'Board pack', 'Intro']);
    const groups = ledgerGroups(rows, 'open', TODAY);
    expect(groups.map(g => [g.key, g.rows.map(r => r.title)])).toEqual([
      ['overdue', ['Woke today', 'Late deck']], ['week', ['Budget']], ['later', ['Board pack']], ['undated', ['Intro']],
    ]);
  });

  it('shows every direction for All, snoozed by wake day, closed newest first', () => {
    expect(ledgerRows(list, { scope: 'all', status: 'open', query: '', today: TODAY }).length).toBe(6);
    expect(ledgerRows(list, { scope: 'all', status: 'snoozed', query: '', today: TODAY }).map(r => r.title)).toEqual(['Snoozed call']);
    expect(ledgerRows(list, { scope: 'all', status: 'closed', query: '', today: TODAY }).map(r => r.title)).toEqual(['Sent', 'Dropped']);
    expect(ledgerGroups([], 'closed', TODAY)).toEqual([]);
  });

  it('filters by every word across title, person and their note', () => {
    expect(matchesQuery(c('Budget'), 'dana budget')).toBe(true);
    expect(matchesQuery(c('Budget', { personPath: 'people/Dana Lee.md' }), 'people/dana')).toBe(true);
    expect(matchesQuery(c('Budget'), 'raj')).toBe(false);
    expect(ledgerRows(list, { scope: 'all', status: 'open', query: 'raj', today: TODAY }).map(r => r.title)).toEqual(['Their export']);
  });

  it('offers tomorrow, next Monday and a week on', () => {
    expect(snoozeChoices(TODAY).map(s => s.until)).toEqual(['2026-10-12', '2026-10-12', '2026-10-18']);
    // From a Monday, next Monday is a week on.
    expect(snoozeChoices('2026-10-12')[1].until).toBe('2026-10-19');
    expect(snoozeChoices('2026-10-16')[1].until).toBe('2026-10-19');
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01');
  });

  it('words the debt and names the source', () => {
    expect(owesLine(c('x'))).toBe('You owe Dana Lee');
    expect(owesLine(c('x', { direction: 'owed_to_me', counterparty: 'Raj' }))).toBe('Raj owes you');
    expect(sourceName(c('x', { sourcePath: 'workspaces/ops/sources/Vendor call.md' }))).toBe('Vendor call');
    expect(sourceName(c('x'))).toBe('');
  });

  it('remembers what a change replaced, for undo', () => {
    expect(changeOf(list[5], 'Marked done')).toEqual({ url: list[5].url, title: 'Snoozed call', did: 'Marked done', status: 'open', snoozedUntil: '2026-10-13', fulfilledAt: null });
    // A9: undoing a reopen puts the original stamp back.
    expect(changeOf({ ...list[5], status: 'fulfilled', fulfilledAt: '2026-10-01T09:00:00Z' }, 'Reopened').fulfilledAt).toBe('2026-10-01T09:00:00Z');
    expect(changeOf(list[0], 'Dropped').snoozedUntil).toBe('');
  });

  it('takes bare keys only, so ⌘, ⌃ and ⌥ chords pass through to the app and macOS', () => {
    expect(ledgerCommand({ key: 'd' })).toBe('done');
    expect(ledgerCommand({ key: 'j' })).toBe('down');
    expect(ledgerCommand({ key: 'ArrowUp' })).toBe('up');
    expect(ledgerCommand({ key: 'Escape' })).toBe('close');
    expect(ledgerCommand({ key: 'd', metaKey: true })).toBeNull();
    expect(ledgerCommand({ key: 'k', ctrlKey: true })).toBeNull();
    expect(ledgerCommand({ key: 'a', altKey: true })).toBeNull();
    expect(ledgerCommand({ key: 'q' })).toBeNull();
  });

  it('drops with Delete, never x: x selects in Tasks, so it is not destructive here (P1-15)', () => {
    expect(ledgerCommand({ key: 'x' })).toBeNull();
    expect(ledgerCommand({ key: 'Delete' })).toBe('drop');
    expect(ledgerCommand({ key: 'Backspace' })).toBe('drop');
    expect(ledgerCommand({ key: 'u' })).toBe('undo');
  });
});
