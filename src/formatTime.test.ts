import { describe, expect, it } from 'vitest';
import { formatClock, formatDuration, formatRelative, formatSeconds } from './formatTime';

const minute = 60_000;
const hour = 60 * minute;
const day = 24 * hour;
// Local constructors keep clock expectations independent of the test host's zone.
const now = new Date(2026, 10, 15, 12).getTime();

describe('formatRelative', () => {
  it('uses compact past and future units at each boundary', () => {
    for (const offset of [0, 1, 59_999]) {
      expect(formatRelative(now - offset, now)).toBe('just now');
      expect(formatRelative(now + offset, now)).toBe('just now');
    }
    for (const [offset, label] of [
      [minute, '1m'], [59 * minute, '59m'], [hour, '1h'],
      [16 * hour, '16h'], [23 * hour, '23h'], [day, '1d'], [3 * day, '3d'],
    ] as const) {
      expect(formatRelative(now - offset, now)).toBe(`${label} ago`);
      expect(formatRelative(now + offset, now)).toBe(`in ${label}`);
    }
  });

  it('uses an absolute date after seven days and rejects missing timestamps', () => {
    expect(formatRelative(now - 7 * day, now)).toBe('7d ago');
    expect(formatRelative(new Date(2026, 10, 7, 12).getTime(), now)).toBe('Nov 7, 2026');
    expect(formatRelative(NaN, now)).toBe('');
    expect(formatRelative(now, NaN)).toBe('');
  });
});

describe('formatClock', () => {
  it('shows a local clock today, then weekday, month-day and clock', () => {
    expect(formatClock(new Date(2026, 10, 15, 9, 40).getTime(), now).replace(/\s/g, ' ')).toBe('9:40 AM');
    expect(formatClock(new Date(2026, 10, 13, 9, 40).getTime(), now).replace(/\s/g, ' ')).toBe('Fri 11-13 9:40 AM');
    expect(formatClock(new Date(2026, 10, 15, 0, 5).getTime(), now)).toBe('12:05 AM');
    expect(formatClock(new Date(2026, 10, 15, 13, 5).getTime(), now)).toBe('1:05 PM');
  });

  it('uses an absolute date after seven days, including the year', () => {
    expect(formatClock(now - 7 * day, now)).toBe('Sun 11-08 12:00 PM');
    expect(formatClock(new Date(2026, 10, 7, 9, 40).getTime(), now)).toBe('Nov 7, 2026');
    expect(formatClock(new Date(2025, 10, 13).getTime(), now)).toBe('Nov 13, 2025');
    expect(formatClock(NaN, now)).toBe('');
    expect(formatClock(now, NaN)).toBe('');
  });
});

describe('formatDuration', () => {
  it.each([
    [0, '0ms'], [-1, '0ms'], [12, '12ms'], [999, '999ms'],
    [1_000, '1s'], [2_345, '2.3s'], [59_999, '59.9s'],
    [60_000, '1m 0s'], [175_000, '2m 55s'], [3_599_999, '59m 59s'],
    [3_600_000, '1h 0m'], [3_840_000, '1h 4m'],
  ])('formats %s milliseconds as %s', (ms, label) => {
    expect(formatDuration(ms)).toBe(label);
  });

  it('does not render non-finite durations', () => {
    expect(formatDuration(NaN)).toBe('');
    expect(formatDuration(Infinity)).toBe('');
  });
});

describe('formatSeconds and far-future times', () => {
  it('reads whole seconds, with 0 as 0s', () => {
    expect(formatSeconds(0)).toBe('0s');
    expect(formatSeconds(175)).toBe('2m 55s');
  });
  it('uses a date beyond 7 days ahead too', () => {
    const now = new Date(2026, 10, 15, 12).getTime();
    expect(formatRelative(now + 12 * 86_400_000, now)).toMatch(/2026/);
  });
});
