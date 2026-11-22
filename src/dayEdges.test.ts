// P0-03: today is a calendar day, not 24 hours from midnight. Fixed zone with
// DST (the gate's TZ): 2026-11-01 is 25 hours long in New York.
(globalThis as unknown as { process: { env: Record<string, string> } }).process.env.TZ = 'America/New_York';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { dayLongDate, todaysMeetings } from './todayBrief';
import { weekRangeLabel, type AgendaEvent } from './agenda';
import { nextWeekByDay } from './weeklyReview';
import { localDateKey } from './utils';

const timed = (id: string, start: Date, end: Date): AgendaEvent => ({ id, title: id, start: start.toISOString(), end: end.toISOString(), allDay: false, cancelled: false });
const allDay = (id: string, from: string, to: string): AgendaEvent => ({ id, title: id, start: `${from}T00:00:00Z`, end: `${to}T00:00:00Z`, allDay: true, cancelled: false });

afterEach(() => { vi.useRealTimers(); });

describe('Today across DST and midnight (P0-03)', () => {
  it('keeps a 23:30 meeting on the 25-hour Sunday 2026-11-01', () => {
    const late = timed('late', new Date(2026, 10, 1, 23, 30), new Date(2026, 10, 2, 0, 15));
    expect(todaysMeetings([late], new Date(2026, 10, 1, 20)).map(m => m.id)).toEqual(['late']);
  });

  it('shows a meeting that crosses midnight on both days', () => {
    const night = timed('night', new Date(2026, 10, 1, 23, 30), new Date(2026, 10, 2, 0, 30));
    expect(todaysMeetings([night], new Date(2026, 10, 1, 9)).map(m => m.id)).toEqual(['night']);
    expect(todaysMeetings([night], new Date(2026, 10, 2, 9)).map(m => m.id)).toEqual(['night']);
    // And never on the day after it ended.
    expect(todaysMeetings([night], new Date(2026, 10, 3, 9))).toEqual([]);
  });

  it("dates Today's header from the facts' day, which the minute tick moves at midnight", () => {
    expect(dayLongDate(localDateKey(new Date(2026, 10, 1, 23, 59)))).toBe('Sunday, November 1');
    expect(dayLongDate(localDateKey(new Date(2026, 10, 2, 0, 1)))).toBe('Monday, November 2');
  });

  it('labels the Agenda week with its range', () => {
    // ICU spaces vary by version (thin, narrow no-break): compare plain spaces.
    const flat = (s: string) => s.replace(/\s/g, ' ');
    expect(flat(weekRangeLabel('2026-10-22'))).toBe('Oct 22 – 28');
    expect(flat(weekRangeLabel('2026-10-29'))).toBe('Oct 29 – Nov 4');
    expect(flat(weekRangeLabel('2026-12-29'))).toBe('Dec 29, 2026 – Jan 4, 2027');
  });
});

describe('Weekly review next week (P0-03)', () => {
  it('lists a multi-day all-day event under each day of next week it covers', () => {
    // Fri 2026-10-23 to Wed 2026-10-28 inclusive (Graph's end is the day after).
    const offsite = allDay('Offsite', '2026-10-23', '2026-10-29');
    const standup = timed('Standup', new Date(2026, 9, 27, 9), new Date(2026, 9, 27, 9, 15));
    // A timed event from Sunday evening into Monday is on Monday too.
    const trip = timed('Trip', new Date(2026, 9, 25, 18), new Date(2026, 9, 26, 12));
    const days = nextWeekByDay([offsite, standup, trip], '2026-10-26');
    const flat = (s: string) => s.replace(/\s/g, ' ');
    expect(days.map(([day, items]) => [day, items.map(i => flat(`${i.time} ${i.title}`))])).toEqual([
      ['Mon, Oct 26', ['all day Offsite', 'until 12:00 PM Trip']],
      ['Tue, Oct 27', ['all day Offsite', '9:00 AM Standup']],
      ['Wed, Oct 28', ['all day Offsite']],
    ]);
  });

  it('a timed event over several days shows its start time on its first day only (RF-876)', () => {
    // Mon 09:00 to Wed 17:00: the model must not read it as three 9 AM meetings.
    const conference = timed('Conference', new Date(2026, 9, 26, 9), new Date(2026, 9, 28, 17));
    const flat = (s: string) => s.replace(/\s/g, ' ');
    expect(nextWeekByDay([conference], '2026-10-26').map(([day, items]) => [day, items.map(i => flat(`${i.time} ${i.title}`))])).toEqual([
      ['Mon, Oct 26', ['9:00 AM Conference']],
      ['Tue, Oct 27', ['continues Conference']],
      ['Wed, Oct 28', ['until 5:00 PM Conference']],
    ]);
    // Ending at the next midnight ends that day, said as midnight (not "12:00 AM",
    // which reads as the day's start); starting at midnight is the day's start.
    const late = timed('Late', new Date(2026, 9, 25, 18), new Date(2026, 9, 27, 0));
    const early = timed('Early', new Date(2026, 9, 27, 0), new Date(2026, 9, 27, 1));
    expect(nextWeekByDay([late, early], '2026-10-26').map(([day, items]) => [day, items.map(i => flat(`${i.time} ${i.title}`))])).toEqual([
      ['Mon, Oct 26', ['until midnight Late']],
      ['Tue, Oct 27', ['12:00 AM Early']],
    ]);
  });

  it('skips cancelled events and days outside the week', () => {
    const gone = { ...timed('Gone', new Date(2026, 9, 26, 9), new Date(2026, 9, 26, 10)), cancelled: true };
    const later = timed('Later', new Date(2026, 10, 2, 9), new Date(2026, 10, 2, 10));
    expect(nextWeekByDay([gone, later], '2026-10-26')).toEqual([]);
  });
});
