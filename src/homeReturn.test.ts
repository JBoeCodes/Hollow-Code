import { describe, expect, it } from 'vitest';
import { relativeWhen } from './homeReturn';

describe('relativeWhen', () => {
  it('says when in words', () => {
    const now = Date.parse('2026-10-01T12:00:00Z');
    expect(relativeWhen('2026-10-01T11:59:40Z', now)).toBe('just now');
    expect(relativeWhen('2026-10-01T11:48:00Z', now)).toBe('12m ago');
    expect(relativeWhen('2026-10-01T09:00:00Z', now)).toBe('3h ago');
    expect(relativeWhen('2026-09-30T11:00:00Z', now)).toBe('1d ago');
    expect(relativeWhen('2026-09-27T12:00:00Z', now)).toBe('4d ago');
    expect(relativeWhen('2026-10-02T11:00:00Z', now)).toBe('in 23h');
    expect(relativeWhen('not a date', now)).toBe('');
    expect(relativeWhen(null, now)).toBe('');
  });
});
