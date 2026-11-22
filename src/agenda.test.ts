// Fixed zone with DST: the layout must hold across both transitions.
(globalThis as unknown as { process: { env: Record<string, string> } }).process.env.TZ = 'America/New_York';
import { describe, expect, it } from 'vitest';
import { allDayCovers, calendarCoverage, freeSlots, freshness, instantAt, layoutDay, localIso, shiftDay, type AgendaEvent } from './agenda';

const ev = (over: Partial<AgendaEvent>): AgendaEvent => ({ id: 'e', title: 'Meeting', start: '2026-10-01T10:00:00-04:00', end: '2026-10-01T11:00:00-04:00', allDay: false, cancelled: false, ...over });

describe('agenda layout (next-level P10)', () => {
  it('lays out an ordinary day with overlaps in columns', () => {
    const d = layoutDay('2026-10-01', [ev({ id: 'a' }), ev({ id: 'b', start: '2026-10-01T10:30:00-04:00', end: '2026-10-01T11:30:00-04:00' }), ev({ id: 'c', start: '2026-10-01T13:00:00-04:00', end: '2026-10-01T14:00:00-04:00' })], [], []);
    expect(d.minutes).toBe(1440);
    const a = d.timed.find(p => p.id === 'a')!, b = d.timed.find(p => p.id === 'b')!, c = d.timed.find(p => p.id === 'c')!;
    expect([a.startMin, a.endMin]).toEqual([600, 660]);
    expect([a.cols, b.cols, a.col, b.col]).toEqual([2, 2, 0, 1]);
    expect([c.col, c.cols]).toEqual([0, 1]);
  });
  it('has 23 hours on the spring-forward day and 25 on the fall-back day', () => {
    expect(layoutDay('2026-03-08', [], [], []).minutes).toBe(1380);
    const fall = layoutDay('2026-11-01', [ev({ id: 'early', start: '2026-11-01T01:30:00-04:00', end: '2026-11-01T02:00:00-04:00' }), ev({ id: 'late', start: '2026-11-01T01:30:00-05:00', end: '2026-11-01T02:00:00-05:00' })], [], []);
    expect(fall.minutes).toBe(1500);
    expect(fall.hours.length).toBe(25);
    // The repeated 1:30 AM lands twice, an hour apart.
    expect(fall.timed.find(p => p.id === 'early')!.startMin).toBe(90);
    expect(fall.timed.find(p => p.id === 'late')!.startMin).toBe(150);
    // A local 9 AM task is at elapsed 10 h on the long day.
    const t = layoutDay('2026-11-01', [], [], [{ url: 't', title: 'T', due: '2026-11-01', dueTime: '09:00', durationMin: 60, status: 'todo' }]);
    expect(t.timed[0].startMin).toBe(600);
    expect(localIso(instantAt('2026-11-01', 600))).toBe('2026-11-01T09:00:00-05:00');
  });
  it('shows all-day events on every covered date, end-exclusive', () => {
    const offsite = ev({ id: 'off', allDay: true, start: '2026-10-05T00:00:00-04:00', end: '2026-10-07T00:00:00-04:00' });
    expect(['2026-10-04', '2026-10-05', '2026-10-06', '2026-10-07'].map(k => allDayCovers(offsite, k))).toEqual([false, true, true, false]);
    expect(layoutDay('2026-10-06', [offsite], [], []).allDay.map(e => e.id)).toEqual(['off']);
  });
  it('keeps cancelled events visible and out of busy time; recurrence exceptions use their own times', () => {
    const weekly = ev({ id: 'standup-1', title: 'Standup', start: '2026-10-01T09:00:00-04:00', end: '2026-10-01T09:15:00-04:00' });
    const moved = ev({ id: 'standup-2-exception', title: 'Standup (moved)', start: '2026-10-01T15:00:00-04:00', end: '2026-10-01T15:15:00-04:00' });
    const cancelled = ev({ id: 'x', title: 'Vendor call', cancelled: true, start: '2026-10-01T11:00:00-04:00', end: '2026-10-01T12:00:00-04:00' });
    const d = layoutDay('2026-10-01', [weekly, moved, cancelled], [], []);
    expect(d.timed.find(p => p.id === 'x')!.cancelled).toBe(true);
    expect(d.timed.find(p => p.id === 'standup-2-exception')!.startMin).toBe(900);
    const free = freeSlots(d, 60);
    expect(free.some(s => s.startMin <= 660 && s.endMin >= 720)).toBe(true);
  });
  it('separates due tasks with and without a time, and skips done ones', () => {
    const d = layoutDay('2026-10-01', [], [{ id: 'fb', title: 'Focus', start: '2026-10-01T08:00:00-04:00', end: '2026-10-01T09:00:00-04:00' }], [
      { url: 'a', title: 'Timed', due: '2026-10-01', dueTime: '14:30', durationMin: 30, status: 'todo' },
      { url: 'b', title: 'Undated time', due: '2026-10-01', dueTime: null, durationMin: null, status: 'todo' },
      { url: 'c', title: 'Done', due: '2026-10-01', dueTime: null, durationMin: null, status: 'Delivered' },
    ]);
    expect(d.timed.map(p => p.kind).sort()).toEqual(['focus', 'task']);
    expect(d.dueNoTime.map(t => t.url)).toEqual(['b']);
  });
  it('shifts days and says how fresh external data is', () => {
    expect(shiftDay('2026-10-31', 1)).toBe('2026-11-01');
    expect(shiftDay('2026-03-08', -1)).toBe('2026-03-07');
    expect(freshness('2026-10-01T11:57:00Z', Date.parse('2026-10-01T12:00:00Z'))).toBe('fetched 3 min ago');
    expect(freshness(null)).toBe('not fetched yet');
  });
  it('shows a partial calendar read as partial, never as fresh (merged plan slice 5)', () => {
    const now = Date.parse('2026-10-01T12:00:00Z');
    const base = { fetchedAt: '2026-10-01T12:00:00Z', source: 'Microsoft 365 · read only' };
    expect(calendarCoverage({ ...base, complete: true, partial: null }, now)).toEqual({ label: 'Microsoft 365 · read only · fetched just now', tone: 'ok' });
    // Older backends send no `complete`: unchanged.
    expect(calendarCoverage(base, now).tone).toBe('ok');
    const cut = calendarCoverage({ ...base, complete: false, partial: 'page 2 failed: Microsoft Graph 503' }, now);
    expect(cut.tone).toBe('warn');
    expect(cut.label).toBe('Microsoft 365 · read only · partial, fetched just now');
    expect(cut.hint).toContain('page 2 failed');
  });
});
