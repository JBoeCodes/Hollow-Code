// P0-04: a source Hollow couldn't read is unknown, never zero, in what Today
// and Weekly review show and in the facts the model writes from.
import { describe, expect, it } from 'vitest';
import { calendarCoverageOf, countsLine, factsDigest, todayCoverage, type TodayFacts } from './todayBrief';
import { factsDigest as weeklyDigest } from './weeklyReview';
import type { WeeklyReview } from './types';

const SIGN_IN = 'Microsoft 365 needs signing in again (AADSTS50173: the grant has expired)';
const view = (over: object = {}) => ({ connected: true, fetchedAt: '', events: [], complete: true, error: null, source: 'graph', ...over });
const ok = todayCoverage({ calendar: view(), calendarError: null, commitmentsError: null, overnightError: null, waitingFailed: false, waitingUnavailable: [] });
const facts = (coverage = ok): TodayFacts => ({
  date: '2026-10-22', meetings: [], overdue: [], dueToday: [], commitments: [], waiting: { total: 0, titles: [] }, overnight: [], coverage,
});
const zeroLines = (text: string) => text.split('\n').filter(l => /\(0\):/.test(l));

describe('unknown is never zero (P0-04)', () => {
  it('reads each calendar state as coverage', () => {
    expect(calendarCoverageOf(view(), null)).toEqual({ state: 'ok', reason: null });
    expect(calendarCoverageOf(view({ connected: false }), null).state).toBe('not-connected');
    expect(calendarCoverageOf(view({ complete: false, error: SIGN_IN }), null)).toEqual({ state: 'unavailable', reason: 'Microsoft 365 needs you to sign in again' });
    expect(calendarCoverageOf(view({ error: 'Graph 503' }), null)).toEqual({ state: 'unavailable', reason: 'Graph 503' });
    expect(calendarCoverageOf(view({ complete: false, partial: 'stopped after 5 pages' }), null)).toEqual({ state: 'partial', reason: 'stopped after 5 pages' });
    expect(calendarCoverageOf(null, 'network down')).toEqual({ state: 'unavailable', reason: 'network down' });
    expect(calendarCoverageOf(undefined, null).state).toBe('unavailable');
    // Still loading is neither a failure nor an empty calendar.
    expect(calendarCoverageOf(undefined, null, true).state).toBe('loading');
    expect(calendarCoverageOf(undefined, 'rejected', true).state).toBe('unavailable');
  });

  it("never calls a day clear when a source wasn't read", () => {
    const f = facts(todayCoverage({ ...inputs(), calendar: view({ complete: false, error: SIGN_IN }) }));
    expect(countsLine(f)).toBe('calendar unknown.');
    expect(countsLine(facts())).toBe('A clear day: nothing scheduled or due.');
    expect(countsLine(facts(todayCoverage({ ...inputs(), calendar: undefined, calendarLoading: true })))).toBe("Reading today's facts…");
    expect(countsLine(facts(todayCoverage({ ...inputs(), calendar: view({ connected: false }) })))).toBe('A clear day: nothing scheduled or due.');
    expect(factsDigest(facts(todayCoverage({ ...inputs(), calendar: undefined, calendarLoading: true })))).toContain('Meetings today: unknown (still being read)');
  });

  it('counts every source when all were read', () => {
    const text = factsDigest(facts());
    expect(text).toContain('Meetings today (0):');
    expect(text).toContain('Commitments due (0):');
    expect(text).toContain('Waiting on me (0):');
    expect(text).toContain('Last night (0):');
  });

  it('tells the model the calendar is unknown when its read failed or the sign-in expired', () => {
    for (const calendar of [
      todayCoverage({ ...inputs(), calendar: null, calendarError: 'agenda_events rejected' }),
      todayCoverage({ ...inputs(), calendar: view({ complete: false, error: SIGN_IN }) }),
    ]) {
      const text = factsDigest(facts(calendar));
      expect(text).not.toMatch(/Meetings today \(/);
      expect(text).toMatch(/Meetings today: unknown \(.+\)/);
    }
    expect(factsDigest(facts(todayCoverage({ ...inputs(), calendar: view({ complete: false, error: SIGN_IN }) })))).toContain('Meetings today: unknown (Microsoft 365 needs you to sign in again)');
  });

  it('never prints (0) for commitments, last night or waiting it could not read', () => {
    const text = factsDigest(facts(todayCoverage({ ...inputs(), commitmentsError: 'ledger unreadable', overnightError: 'summary failed', waitingFailed: true })));
    expect(zeroLines(text).filter(l => !l.startsWith('Meetings') && !l.startsWith('Overdue') && !l.startsWith('Tasks'))).toEqual([]);
    expect(text).toContain('Commitments due: unknown (ledger unreadable)');
    expect(text).toContain('Last night: unknown (summary failed)');
    expect(text).toMatch(/Waiting on me: unknown \(.+\)/);
  });

  it('says a partial read is partial', () => {
    const text = factsDigest(facts(todayCoverage({ ...inputs(), calendar: view({ complete: false, partial: 'stopped after 5 pages' }), waitingUnavailable: ['approvals'] })));
    expect(text).toContain('Meetings today (at least 0; partial read: stopped after 5 pages):');
    expect(text).toContain('Waiting on me (at least 0; partial read: couldn\'t read approvals):');
  });

  it('tells the weekly take next week’s calendar is unknown, not empty', () => {
    const r: WeeklyReview = {
      week: '2026-W43', monday: '2026-10-19', sunday: '2026-10-25',
      stats: { accepted: 0, tasksDone: 0, prsMerged: 0, decisions: 0, spendUsd: null },
      shipped: [], inProgress: [], slipped: [], decisions: [], kept: [], made: [], overdue: [], nextWeek: [], numbers: [],
      spendByDay: [], spendByWorkspace: [], unpricedRuns: 0,
    };
    expect(weeklyDigest(r, [])).toContain('Next week on the calendar: none');
    const unknown = weeklyDigest(r, [], calendarCoverageOf(view({ complete: false, error: SIGN_IN }), null));
    expect(unknown).toContain('Next week on the calendar: unknown (Microsoft 365 needs you to sign in again)');
    expect(unknown).not.toContain('Next week on the calendar: none');
  });
});

function inputs() {
  return { calendar: view() as ReturnType<typeof view> | undefined, calendarError: null as string | null, commitmentsError: null, overnightError: null, waitingFailed: false, waitingUnavailable: [] as string[] };
}
