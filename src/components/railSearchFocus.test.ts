import { describe, expect, it } from 'vitest';
import { createSearchFocusReturn } from './railSearchFocus';

const el = (isConnected = true) => ({ isConnected, name: Math.random() });

describe('Search chats focus return', () => {
  it('returns to where Search was asked from', () => {
    const f = createSearchFocusReturn<ReturnType<typeof el>>();
    const icon = el();
    f.open(icon, false);
    expect(f.close(() => el())).toBe(icon);
  });

  it('keeps the first place when the shortcut repeats while searching', () => {
    const f = createSearchFocusReturn<ReturnType<typeof el>>();
    const icon = el();
    f.open(icon, false);
    f.open(el(), true);
    f.open(null, true);
    expect(f.close(() => el())).toBe(icon);
  });

  it('falls back to the top bar icon when the place left the page', () => {
    const f = createSearchFocusReturn<ReturnType<typeof el>>();
    const fallback = el();
    f.open(el(false), false);
    expect(f.close(() => fallback)).toBe(fallback);
  });

  it('falls back when Search was opened from nowhere, and forgets after closing', () => {
    const f = createSearchFocusReturn<ReturnType<typeof el>>();
    const fallback = el();
    f.open(null, false);
    expect(f.close(() => fallback)).toBe(fallback);
    const later = el();
    f.open(later, false);
    expect(f.close(() => fallback)).toBe(later);
    // Closed with nothing remembered: the fallback again, not a stale place.
    expect(f.close(() => fallback)).toBe(fallback);
  });

  it('forgets the remembered place when the sidebar hides (#1332 review)', () => {
    const f = createSearchFocusReturn<ReturnType<typeof el>>();
    const icon = el();
    const fallback = el();
    f.open(icon, false);
    f.forget();
    expect(f.close(() => fallback)).toBe(fallback);
  });
});
