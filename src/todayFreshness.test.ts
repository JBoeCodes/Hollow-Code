import { afterEach, describe, expect, it, vi } from 'vitest';
import { autoWriteStep, briefFingerprint, shouldAutoWriteBrief, type TodayFacts } from './todayBrief';
import { dateKey } from './agenda';
// P1-12: a brief left overnight no longer writes at 00:01
// from a near-empty night while the window is away; it writes when the
// person is back. A brief remembers the facts it was written from, so a
// change since then is named instead of shown as current.
const facts = (over: Partial<TodayFacts> = {}) => ({
  date: '2026-10-24', meetings: [], overdue: [], dueToday: [], commitments: [],
  waiting: { total: 0, titles: [] }, overnight: [],
  coverage: { calendar: { state: 'ok', reason: null }, commitments: { state: 'ok', reason: null }, waiting: { state: 'ok', reason: null }, overnight: { state: 'ok', reason: null } },
  ...over,
}) as unknown as TodayFacts;

afterEach(() => { vi.useRealTimers(); });

describe('Today’s brief is honest about freshness (P1-12)', () => {
  it('a fake clock crossing midnight with the window away writes nothing; back, it writes', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 23, 23, 59));
    const written = dateKey(new Date());
    vi.setSystemTime(new Date(2026, 9, 24, 0, 1));
    const day = dateKey(new Date());
    expect(day).not.toBe(written);
    const base = { open: true, ready: true, cached: true, briefDate: written, day, writing: false, error: false };
    expect(shouldAutoWriteBrief({ ...base, away: true })).toBe(false);
    expect(shouldAutoWriteBrief({ ...base, away: false })).toBe(true);
    expect(shouldAutoWriteBrief({ ...base, away: false, briefDate: day }), 'once a day').toBe(false);
  });
  it('a write held while away reads the facts again before it writes (review B1)', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 24, 0, 1));
    const base = { open: true, ready: true, cached: true, briefDate: '2026-10-23', day: dateKey(new Date()), writing: false, error: false };
    let held = false;
    const steps: string[] = [];
    for (const away of [true, true, false]) {
      const step = autoWriteStep({ ...base, away }, held);
      steps.push(step);
      if (step === 'hold') held = true; else if (step !== 'none') held = false;
    }
    // Away across midnight: held, never written; back: a re-read, then a write.
    expect(steps).toEqual(['hold', 'hold', 'refresh']);
    expect(autoWriteStep({ ...base, away: false }, held)).toBe('write');
    // Nothing due (today's brief exists): nothing held, nothing written.
    expect(autoWriteStep({ ...base, away: true, briefDate: base.day }, false)).toBe('none');
  });
  it('the fingerprint follows the facts, not the clock passing a meeting', () => {
    const meeting = { time: '09:00', title: 'Standup', organizer: null, past: false };
    const a = briefFingerprint(facts({ meetings: [meeting] } as never));
    expect(briefFingerprint(facts({ meetings: [{ ...meeting, past: true }] } as never))).toBe(a);
    expect(briefFingerprint(facts({ meetings: [meeting], overnight: ['Lane A merged #917'] } as never))).not.toBe(a);
  });
});
