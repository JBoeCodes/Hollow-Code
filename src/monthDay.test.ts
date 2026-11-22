import { describe, expect, it } from 'vitest';
import { monthDay } from './monthDay';

describe('monthDay (ST2 follow-up: one formatter for per-row dates)', () => {
  it('says what toLocaleDateString with the same options says, an invalid date included', () => {
    for (const iso of ['2026-10-14T05:00:00Z', '2026-01-02T23:59:00Z', '2025-12-31T12:00:00Z', '2024-02-29T00:00:00Z']) {
      const d = new Date(iso);
      expect(monthDay(d)).toBe(d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }));
    }
    const bad = new Date('not a date');
    expect(monthDay(bad)).toBe(bad.toLocaleDateString(undefined, { month: 'short', day: 'numeric' }));
  });
});
