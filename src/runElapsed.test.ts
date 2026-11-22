import { describe, expect, it } from 'vitest';
import { formatRunElapsed } from './runElapsed';

describe('formatRunElapsed', () => {
  it('counts seconds, minutes, then hours and minutes', () => {
    expect(formatRunElapsed(0)).toBe('0s');
    expect(formatRunElapsed(45_900)).toBe('45s');
    expect(formatRunElapsed(60_000)).toBe('1m');
    expect(formatRunElapsed(12 * 60_000 + 59_000)).toBe('12m');
    expect(formatRunElapsed(3_600_000)).toBe('1h 00m');
    expect(formatRunElapsed(3_600_000 + 5 * 60_000)).toBe('1h 05m');
  });
  it('never shows a negative or broken time', () => {
    expect(formatRunElapsed(-5)).toBe('0s');
    expect(formatRunElapsed(Number.NaN)).toBe('0s');
  });
});
