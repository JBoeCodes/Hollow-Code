import { describe, expect, it } from 'vitest';
import { foldRepeats, isWaitingFailure } from './activity';
import type { Notification } from './types';

const n = (id: string, title: string, at: string, over: Partial<Notification> = {}): Notification => ({ id, title, body: '', kind: 'error', source: 'morning-brief', actionLabel: null, actionUrl: null, created: at, read: true, ...over });

describe('activity folding (next-level P19)', () => {
  it('folds adjacent repeats and keeps a different failure separate', () => {
    const rows = foldRepeats([
      n('a', 'Morning brief failed', '2026-10-01T10:00:00Z'),
      n('b', 'Morning brief failed', '2026-10-01T09:00:00Z'),
      n('c', 'Morning brief failed', '2026-10-01T08:00:00Z'),
      n('d', 'Approval waiting', '2026-10-01T07:30:00Z', { kind: 'info', source: null }),
      n('e', 'Morning brief failed', '2026-10-01T07:00:00Z'),
    ]);
    expect(rows.map(r => [r.head.id, r.repeats, r.unread])).toEqual([['a', 3, false], ['d', 1, false], ['e', 1, false]]);
    expect(rows[0].firstAt).toBe('2026-10-01T08:00:00Z');
  });
  it('does not fold across the time window', () => {
    expect(foldRepeats([n('a', 'X', '2026-10-03T00:00:00Z'), n('b', 'X', '2026-10-01T00:00:00Z')]).length).toBe(2);
  });
  it('keeps different content, destinations and unread actions reachable', () => {
    for (const difference of [{ body: 'Different evidence' }, { actionUrl: 'vault:notes/another.md' }, { actionLabel: 'Inspect' }, { read: false }]) {
      const rows = foldRepeats([n('a', 'Report', '2026-10-01T10:00:00Z'), n('b', 'Report', '2026-10-01T09:00:00Z', difference)]);
      expect(rows.map(row => row.head.id)).toEqual(['a', 'b']);
    }
  });
  it('keeps each unread failure on its own row when asked (U3b bell history)', () => {
    const rows = foldRepeats([
      n('a', 'Morning brief failed', '2026-10-01T10:00:00Z'),
      n('b', 'Morning brief failed', '2026-10-01T09:00:00Z', { read: false }),
      n('c', 'Morning brief failed', '2026-10-01T08:30:00Z', { read: false }),
      n('d', 'Morning brief failed', '2026-10-01T08:00:00Z'),
      n('e', 'Morning brief failed', '2026-10-01T07:00:00Z'),
    ], undefined, isWaitingFailure);
    expect(rows.map(r => [r.head.id, r.repeats, r.unread])).toEqual([['a', 1, false], ['b', 1, true], ['c', 1, true], ['d', 2, false]]);
  });
  it('names only unread failures as waiting', () => {
    expect(isWaitingFailure({ read: false, kind: 'error' })).toBe(true);
    expect(isWaitingFailure({ read: true, kind: 'error' })).toBe(false);
    expect(isWaitingFailure({ read: false, kind: 'warning' })).toBe(false);
  });
});
