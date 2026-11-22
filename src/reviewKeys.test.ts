import { describe, expect, it } from 'vitest';
import { REVIEW_KEY_SURFACES, reviewKey, snoozeUntil, type ReviewKeyAction } from './reviewKeys';
import triageSource from './triage.ts?raw';

// P1-13: one keymap across Review's waiting rows, its cards and Incoming. A
// letter means one thing wherever it works, and nothing else anywhere.
describe('Review keys', () => {
  const press = (key: string, shiftKey = false) => ({ key, shiftKey });

  it('gives each letter one meaning on every surface', () => {
    const meanings = new Map<string, Set<ReviewKeyAction>>();
    for (const surface of REVIEW_KEY_SURFACES) {
      for (const [key, shift] of [['x', false], ['a', false], ['R', true], ['r', false], ['e', false], ['z', false]] as const) {
        const action = reviewKey(surface, press(key, shift));
        if (!action) continue;
        const id = `${shift ? 'Shift-' : ''}${key.toLowerCase()}`;
        meanings.set(id, (meanings.get(id) ?? new Set()).add(action));
      }
    }
    for (const [key, actions] of meanings) expect([key, [...actions]]).toEqual([key, [...actions].slice(0, 1)]);
  });

  it('x dismisses, a accepts, Shift-R rejects', () => {
    expect(reviewKey('notification', press('x'))).toBe('dismiss');
    expect(reviewKey('card', press('a'))).toBe('accept');
    expect(reviewKey('card', press('R', true))).toBe('reject');
    expect(reviewKey('card', press('e'))).toBe('edit');
    expect(reviewKey('card', press('z'))).toBe('snooze');
  });

  it('r alone no longer rejects a card or reads a notification', () => {
    expect(reviewKey('card', press('r'))).toBeNull();
    expect(reviewKey('notification', press('r'))).toBeNull();
    // Delete is a button now, never a letter.
    expect(reviewKey('notification', press('d'))).toBeNull();
  });

  it('Incoming: x dismisses and a accepts, as its own action table says', () => {
    expect(reviewKey('incoming', press('x'))).toBe('dismiss');
    expect(reviewKey('incoming', press('a'))).toBe('accept');
    expect(reviewKey('incoming', press('r'))).toBeNull();
    // Incoming's own table (`triageActions`) names the same letters.
    for (const label of ['Reject', 'Mark read', 'Dismiss']) expect(triageSource).toContain(`{ label: '${label}', key: 'x'`);
    expect(triageSource).toContain("{ label: 'Accept', key: 'a'");
    expect(triageSource).not.toMatch(/key: 'r'/);
  });
});

describe('snoozeUntil', () => {
  const local = (y: number, m: number, d: number, h: number, min = 0) => new Date(y, m - 1, d, h, min);

  it('a snooze at 23:30 wakes at 09:00 the next morning', () => {
    expect(snoozeUntil(local(2026, 10, 24, 23, 30))).toEqual(local(2026, 10, 25, 9));
  });

  it('during the day it is tomorrow at 09:00', () => {
    expect(snoozeUntil(local(2026, 10, 24, 10, 15))).toEqual(local(2026, 10, 25, 9));
    expect(snoozeUntil(local(2026, 10, 24, 8, 59))).toEqual(local(2026, 10, 25, 9));
  });

  it('after midnight, the same night: this morning at 09:00', () => {
    expect(snoozeUntil(local(2026, 10, 25, 1, 0))).toEqual(local(2026, 10, 25, 9));
  });

  it('crosses a month end', () => {
    expect(snoozeUntil(local(2026, 10, 31, 23, 30))).toEqual(local(2026, 11, 1, 9));
  });
});
