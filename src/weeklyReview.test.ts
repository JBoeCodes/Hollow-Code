import { describe, expect, it } from 'vitest';
import { barScale, cardWeekOffset, isoWeek, factsDigest, money, quietWeek, reviewCardDue, shiftWeek, weekLabel } from './weeklyReview';
import type { WeeklyReview } from './types';

const empty: WeeklyReview = {
  week: '2026-W41', monday: '2026-10-05', sunday: '2026-10-11',
  stats: { accepted: 0, tasksDone: 0, prsMerged: 0, decisions: 0, spendUsd: null },
  shipped: [], inProgress: [], slipped: [], decisions: [], kept: [], made: [], overdue: [], nextWeek: [], numbers: [],
  spendByDay: [], spendByWorkspace: [], unpricedRuns: 0,
};

describe('weekly review (overnight 10-11 A7)', () => {
  it('steps ISO weeks across years', () => {
    expect(shiftWeek('2026-W41', 1)).toBe('2026-W42');
    expect(shiftWeek('2026-W41', -1)).toBe('2026-W40');
    expect(shiftWeek('2026-W53', 1)).toBe('2027-W01');
    expect(shiftWeek('2027-W01', -1)).toBe('2026-W53');
    expect(shiftWeek('2025-W01', -1)).toBe('2024-W52');
    expect(shiftWeek('nonsense', 1)).toBe('nonsense');
  });

  it('knows the ISO week of a local date', () => {
    expect(isoWeek(new Date(2026, 9, 11))).toBe('2026-W41');
    expect(isoWeek(new Date(2026, 9, 5))).toBe('2026-W41');
    expect(isoWeek(new Date(2026, 9, 12))).toBe('2026-W42');
    expect(isoWeek(new Date(2027, 0, 1))).toBe('2026-W53');
    expect(isoWeek(new Date(2025, 11, 29))).toBe('2026-W01');
  });

  it('labels a week', () => {
    expect(weekLabel(empty)).toMatch(/^Week 41 · 5–11 /);
    expect(weekLabel({ week: '2026-W40', monday: '2026-09-28', sunday: '2026-10-04' })).toMatch(/^Week 40 · 28 .+ – 4 /);
  });

  it("shows Home's card from Friday 15:00 to Monday noon, about last week on Monday", () => {
    expect(reviewCardDue(new Date(2026, 9, 9, 14, 59))).toBe(false);
    expect(reviewCardDue(new Date(2026, 9, 9, 15, 0))).toBe(true);
    expect(reviewCardDue(new Date(2026, 9, 11, 9))).toBe(true);
    expect(reviewCardDue(new Date(2026, 9, 12, 11, 59))).toBe(true);
    expect(reviewCardDue(new Date(2026, 9, 12, 12))).toBe(false);
    expect(reviewCardDue(new Date(2026, 9, 7, 10))).toBe(false);
    expect(cardWeekOffset(new Date(2026, 9, 12, 9))).toBe(-1);
    expect(cardWeekOffset(new Date(2026, 9, 10, 9))).toBe(0);
  });

  it('formats money and scales bars', () => {
    expect(money(null)).toBe('—');
    expect(money(3)).toBe('$3.00');
    expect(money(123.4)).toBe('$123');
    expect(barScale([0, 2, 4])).toEqual([0, 0.5, 1]);
    expect(barScale([0, 0])).toEqual([0, 0]);
  });

  it('gives the agent facts only, and spots a quiet week', () => {
    const r = { ...empty, slipped: [{ text: 'Board pack (due Wed 7 Oct)', path: null, sessionKey: null, day: '2026-10-07' }], stats: { ...empty.stats, spendUsd: 3 } };
    const text = factsDigest(r, ['Mon 9:00 Atlas review']);
    expect(text).toContain('Slipped (due this week and past: not done, or finished after the due day):\n- Board pack (due Wed 7 Oct)');
    expect(text).toContain('Shipped: none');
    expect(text).toContain('Next week on the calendar:\n- Mon 9:00 Atlas review');
    expect(text).toContain('spend $3.00');
    expect(quietWeek(empty)).toBe(true);
    expect(quietWeek(r)).toBe(false);
  });
});
