import { afterEach, expect, it, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { minuteClock } from './minuteClock';

afterEach(() => vi.useRealTimers());

it('mounted consumers share time, and the last unmount stops the clock until next use', () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-25T04:00:00Z'));
  const first = createRoot(dispose => ({ dispose, now: minuteClock() }));
  const second = createRoot(dispose => ({ dispose, now: minuteClock() }));
  expect(vi.getTimerCount()).toBe(1);
  const start = first.now();
  vi.advanceTimersByTime(300_000);
  expect(first.now()).toBe(start + 300_000);
  expect(second.now()).toBe(first.now());
  first.dispose();
  vi.advanceTimersByTime(30_000);
  expect(second.now()).toBe(start + 330_000);
  second.dispose();
  expect(vi.getTimerCount()).toBe(0);
  vi.advanceTimersByTime(120_000);
  const next = createRoot(dispose => ({ dispose, now: minuteClock() }));
  expect(next.now()).toBe(start + 450_000);
  next.dispose();
  expect(vi.getTimerCount()).toBe(0);
});
