// Fixed zone with DST (the same one agenda.test uses): the next-runs helper
// must follow the backend's `cron` crate + `chrono::Local` across both
// transitions. 2026 ends DST on Nov 1; 2027 starts it on Mar 14.
(globalThis as unknown as { process: { env: Record<string, string> } }).process.env.TZ = 'America/New_York';

import { describe, expect, it } from 'vitest';
import { firstDstSkip, formatRun, isValidSchedule, missedRunSentences, nextRuns } from './workflowSchedule';

const iso = (runs: { at: Date }[]) => runs.map(r => r.at.toISOString());
const at = (s: string) => new Date(s).getTime();
const flat = (s: string) => s.replace(/\s/g, ' ');

describe('nextRuns', () => {
  it('lists the next three daily runs across the autumn change with each zone name', () => {
    const r = nextRuns('0 0 7 * * * *', at('2026-10-30T12:00:00-04:00'))!;
    expect(iso(r.runs)).toEqual(['2026-10-31T11:00:00.000Z', '2026-11-01T12:00:00.000Z', '2026-11-02T12:00:00.000Z']);
    expect(r.runs.map(x => x.zoneName)).toEqual(['EDT', 'EST', 'EST']);
    expect(r.timeZone).toBe('America/New_York');
    expect(r.skipped).toEqual([]);
  });

  it('skips a time the spring change removes, as the cron crate does', () => {
    const r = nextRuns('0 30 2 * * * *', at('2027-03-12T12:00:00-05:00'))!;
    expect(iso(r.runs)).toEqual(['2027-03-13T07:30:00.000Z', '2027-03-15T06:30:00.000Z', '2027-03-16T06:30:00.000Z']);
    expect(r.skipped).toEqual([{ wall: '2027-03-14 02:30', reason: 'gap' }]);
  });

  it('skips a time the autumn change repeats (chrono `.single()` is None for both)', () => {
    const r = nextRuns('0 30 1 * * * *', at('2026-10-30T12:00:00-04:00'))!;
    expect(iso(r.runs)).toEqual(['2026-10-31T05:30:00.000Z', '2026-11-02T06:30:00.000Z', '2026-11-03T06:30:00.000Z']);
    expect(r.skipped).toEqual([{ wall: '2026-11-01 01:30', reason: 'repeat' }]);
  });

  it('keeps hourly runs on the hour through the spring gap', () => {
    const r = nextRuns('0 0 * * * * *', at('2027-03-14T00:30:00-05:00'))!;
    expect(iso(r.runs)).toEqual(['2027-03-14T06:00:00.000Z', '2027-03-14T07:00:00.000Z', '2027-03-14T08:00:00.000Z']);
    expect(r.runs.map(x => x.zoneName)).toEqual(['EST', 'EDT', 'EDT']);
    expect(r.skipped).toEqual([{ wall: '2027-03-14 02:00', reason: 'gap' }]);
  });

  it('is strictly after `from` and honours weekdays and names', () => {
    expect(iso(nextRuns('0 0 7 * * * *', at('2026-10-05T07:00:00-04:00'))!.runs)[0]).toBe('2026-10-06T11:00:00.000Z');
    // Friday 10:00 → Mon, Tue, Wed at 9.
    expect(iso(nextRuns('0 0 9 * * Mon-Fri *', at('2026-10-02T10:00:00-04:00'))!.runs))
      .toEqual(['2026-10-05T13:00:00.000Z', '2026-10-06T13:00:00.000Z', '2026-10-07T13:00:00.000Z']);
    // Day of month AND day of week (Friday the 13th), across years.
    expect(iso(nextRuns('0 0 9 13 * Fri *', at('2026-10-01T00:00:00-04:00'))!.runs))
      .toEqual(['2026-11-13T14:00:00.000Z', '2027-08-13T13:00:00.000Z', '2028-10-13T13:00:00.000Z']);
    // 6-field form, steps and shorthands.
    expect(iso(nextRuns('0 */20 9 * * *', at('2026-10-05T09:05:00-04:00'))!.runs))
      .toEqual(['2026-10-05T13:20:00.000Z', '2026-10-05T13:40:00.000Z', '2026-10-06T13:00:00.000Z']);
    expect(iso(nextRuns('@daily', at('2026-10-05T09:05:00-04:00'))!.runs)[0]).toBe('2026-10-06T04:00:00.000Z');
  });

  it('returns null for manual and for schedules the backend rejects', () => {
    expect(nextRuns('manual')).toBeNull();
    expect(nextRuns('')).toBeNull();
    for (const bad of ['not-cron', '0 0 25 * * * *', '0 0 9 * * 0 *', '0 0 9 * *', '0 0 9 * * Mon/2 *', '0 0 9 * * Mon-5 *']) {
      expect(nextRuns(bad), bad).toBeNull();
      expect(isValidSchedule(bad), bad).toBe(false);
    }
    for (const good of ['manual', '0 0 7 * * * *', '0 0 9 * * Mon-Fri', '@hourly', '0 0 9 ? * Sun *', '0 0 9 1-7 Jan-Mar * 2027']) {
      expect(isValidSchedule(good), good).toBe(true);
    }
  });

  it('stops at a pinned year', () => {
    const r = nextRuns('0 0 9 1 1 * 2027', at('2026-10-01T00:00:00-04:00'))!;
    expect(iso(r.runs)).toEqual(['2027-01-01T14:00:00.000Z']);
  });

  it('formats a run with its zone name', () => {
    const r = nextRuns('0 0 7 * * * *', at('2026-10-05T08:00:00-04:00'))!;
    expect(flat(formatRun(r.runs[0], 'en-US'))).toBe('Tue, Oct 6 · 7:00 AM EDT');
  });
});

describe('firstDstSkip', () => {
  it('finds the next skipped or repeated time and ignores schedules clear of the change', () => {
    expect(firstDstSkip('0 30 2 * * * *', at('2027-03-01T00:00:00-05:00'))).toEqual({ wall: '2027-03-14 02:30', reason: 'gap' });
    expect(firstDstSkip('0 30 1 * * Sun *', at('2026-10-01T00:00:00-04:00'))).toEqual({ wall: '2026-11-01 01:30', reason: 'repeat' });
    expect(firstDstSkip('0 0 7 * * * *', at('2026-10-01T00:00:00-04:00'))).toBeNull();
    expect(firstDstSkip('manual')).toBeNull();
  });
});

describe('missedRunSentences', () => {
  it('describes manual, triggered and scheduled automations as the backend runs them', () => {
    expect(missedRunSentences({ schedule: 'manual', misfireGraceMin: null })[0]).toMatch(/only when you start it/);
    const triggered = missedRunSentences({ schedule: 'manual', misfireGraceMin: null, triggers: ['task-overdue'] });
    expect(triggered[0]).toContain('task-overdue');
    expect(triggered[1]).toMatch(/joins that run/);

    const daily = missedRunSentences({ schedule: '0 0 7 * * * *', misfireGraceMin: null, timeZone: 'America/New_York' });
    expect(daily[0]).toBe('Times are in America/New_York, this Mac’s time zone.');
    expect(daily[1]).toContain('within 2 hours of the time');
    expect(daily[1]).toContain('A missed time runs at most once');
    expect(missedRunSentences({ schedule: '0 0 7 * * * *', misfireGraceMin: 45 })[1]).toContain('within 45 minutes');
    expect(missedRunSentences({ schedule: '0 0 7 * * * *', misfireGraceMin: 90 })[1]).toContain('within 1 h 30 min');

    const quiet = missedRunSentences({ schedule: '0 0 7 * * * *', misfireGraceMin: null, skipIfQuiet: true });
    expect(quiet.some(s => /nothing it watches changed/.test(s))).toBe(true);
    expect(missedRunSentences({ schedule: 'bogus', misfireGraceMin: null })[0]).toMatch(/can’t read this schedule/);
  });

  it('names a daylight-saving skip in plain words', () => {
    const gap = missedRunSentences({
      schedule: '0 30 2 * * * *', misfireGraceMin: null, locale: 'en-US',
      dstSkip: { wall: '2027-03-14 02:30', reason: 'gap' },
    }).map(flat);
    expect(gap).toContain('Mar 14, 2027, 2:30 AM doesn’t exist that day (clocks move forward), so that run is skipped.');
    const repeat = missedRunSentences({
      schedule: '0 30 1 * * * *', misfireGraceMin: null, locale: 'en-US',
      dstSkip: { wall: '2026-11-01 01:30', reason: 'repeat' },
    }).map(flat);
    expect(repeat).toContain('Nov 1, 2026, 1:30 AM happens twice that day (clocks move back), so that run is skipped.');
  });
});
