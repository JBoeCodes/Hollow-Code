import { describe, expect, it } from 'vitest';
import { backgroundClock, clockArc, clockTime } from './backgroundClock';
import type { AutomationWindows, WindowView } from './generated/ipc';
const window = (patch: Partial<WindowView> = {}): WindowView => ({
  name: 'tonight', label: 'Tonight', days: ['fri'], start: '18:00', end: '07:00', period: 'week', openNow: false, ...patch,
});
const data = (windows = [window()], timeZone = 'Asia/Tokyo'): AutomationWindows => ({ windows, timeZone, overnightCapUsd: 40 });
describe('background clock uses the configured zone and start weekday', () => {
  it('shows Saturday in Tokyo while the browser date is Friday, retaining Friday’s tail', () => {
    const model = backgroundClock(data(), new Date('2026-10-16T21:00:00Z'))!;
    expect(model.date).toBe('2026-10-17');
    expect(model.minute).toBe(360);
    expect(model.bands[0].spans).toEqual([{ start: 0, end: 420, fromYesterday: true }]);
  });
  it('shows the start portion on Friday and no tail on Sunday', () => {
    expect(backgroundClock(data(), new Date('2026-10-16T10:00:00Z'))!.bands[0].spans).toEqual([{ start: 1080, end: 1440, fromYesterday: false }]);
    expect(backgroundClock(data(), new Date('2026-10-18T01:00:00Z'))!.bands[0].spans).toEqual([]);
  });
  it('equal hours span 24 hours and both adjacent starts remain distinguishable', () => {
    const model = backgroundClock(data([window({ days: ['fri', 'sat'], start: '07:00', end: '07:00' })]), new Date('2026-10-16T21:00:00Z'))!;
    expect(model.bands[0].spans).toEqual([{ start: 0, end: 420, fromYesterday: true }, { start: 420, end: 1440, fromYesterday: false }]);
  });
  it('midnight end has no zero-length tail; midnight equal is a whole day', () => {
    const now = new Date('2026-10-16T21:00:00Z');
    expect(backgroundClock(data([window({ end: '00:00' })]), now)!.bands[0].spans).toEqual([]);
    expect(backgroundClock(data([window({ days: ['sat'], start: '00:00', end: '00:00' })]), now)!.bands[0].spans).toEqual([{ start: 0, end: 1440, fromYesterday: false }]);
  });
  it('keeps configured civil hours through a DST skip and repeated hour', () => {
    const value = data([window({ days: ['sun'], start: '01:00', end: '04:00' })], 'America/New_York');
    for (const instant of ['2026-03-08T07:30:00Z', '2026-11-01T05:30:00Z', '2026-11-01T06:30:00Z']) {
      expect(backgroundClock(value, new Date(instant))!.bands[0].spans).toEqual([{ start: 60, end: 240, fromYesterday: false }]);
    }
  });
  it('missing, invalid or empty data is unknown, never zero', () => {
    const now = new Date('2026-10-16T21:00:00Z');
    for (const value of [null, undefined, data([]), data([window()], ''), data([window()], 'wrong/zone'), data([window({ start: '25:00' })]), data([window({ days: [] })]), data([window(), window()])]) {
      expect(backgroundClock(value, now)).toBeNull();
    }
    expect(backgroundClock(data(), new Date('invalid'))).toBeNull();
    expect(backgroundClock(data([window()], "this Mac's zone"), now)).toBeNull();
  });
  it('an absent Tonight stays unknown and other windows route only to Automations', () => {
    const model = backgroundClock(data([window({ name: 'other' })]), new Date('2026-10-16T21:00:00Z'))!;
    expect(model.tonightKnown).toBe(false);
    expect(model.bands[0].target).toBe('automations');
  });
  it('formats day boundaries and draws two arcs for a full circle', () => {
    expect(clockTime(1440)).toBe('24:00');
    expect(clockArc(0, 1440, 50).match(/ A /g)).toHaveLength(2);
    expect(clockArc(0, 1440, 50)).not.toContain('NaN');
  });
});
