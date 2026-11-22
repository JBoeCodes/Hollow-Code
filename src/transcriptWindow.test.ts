import { describe, expect, it } from 'vitest';
import { AUTO_STEP, LOAD_STEP, TURN_CAP, laterEnd, trimEnd, trimStart, windowFor } from './transcriptWindow';
import { TURN_WINDOW } from './conversationTools';

describe('the window keeps to its cap', () => {
  it('is three steps and a little, so the latest window plus one load always fits', () => {
    expect(LOAD_STEP).toBe(TURN_WINDOW);
    expect(TURN_CAP).toBeGreaterThanOrEqual(2 * LOAD_STEP);
    expect(AUTO_STEP).toBeLessThan(LOAD_STEP);
  });

  it('cuts the end after the start moved up, keeping what is on screen', () => {
    // 1,000 turns, start at 600 after two loads (400 drawn), person reading turn 610.
    expect(trimEnd({ from: 600, to: 1000 }, 1000, 640, false)).toBe(800);
    // On screen reaches past the cap: those turns stay.
    expect(trimEnd({ from: 600, to: 1000 }, 1000, 790, false)).toBe(830);
    // Never beyond the thread.
    expect(trimEnd({ from: 820, to: 1000 }, 1000, 990, false)).toBe(null);
  });

  it('never cuts at the end, and cuts nothing within the cap', () => {
    expect(trimEnd({ from: 600, to: 1000 }, 1000, 640, true)).toBe(null);
    expect(trimEnd({ from: 800, to: 1000 }, 1000, 800, false)).toBe(null);
    expect(trimEnd({ from: 0, to: 80 }, 80, null, false)).toBe(null);
  });

  it('cuts the start after the end moved down, oldest first, keeping what is on screen', () => {
    expect(trimStart({ from: 0, to: 280 }, 220)).toBe(80);
    expect(trimStart({ from: 0, to: 280 }, 40)).toBe(null);
    expect(trimStart({ from: 0, to: 280 }, 100)).toBe(60);
    expect(trimStart({ from: 0, to: 280 }, null)).toBe(null);
    expect(trimStart({ from: 100, to: 250 }, 120)).toBe(null);
  });

  it('moves the end down one step, to the thread end at most', () => {
    expect(laterEnd(240, 1000)).toBe(320);
    expect(laterEnd(960, 1000)).toBe(1000);
    expect(laterEnd(240, 1000, AUTO_STEP)).toBe(260);
  });
});

describe('windowFor recenters a jump', () => {
  const total = 1000;
  it('keeps the window when the turn is inside it', () => {
    expect(windowFor({ from: 920, to: 1000 }, total, 950)).toEqual({ from: 920, to: 1000 });
  });
  it('loads only around a turn above it, not every turn between', () => {
    const w = windowFor({ from: 920, to: 1000 }, total, 10);
    expect(w.from).toBe(6);
    expect(w.to - w.from).toBeLessThanOrEqual(TURN_CAP);
    expect(w.to).toBeGreaterThan(10);
  });
  it('moves the window down to a turn below it', () => {
    const w = windowFor({ from: 0, to: 80 }, total, 600);
    expect(w.to).toBeGreaterThanOrEqual(601);
    expect(w.to - w.from).toBeLessThanOrEqual(TURN_CAP);
    expect(w.from).toBeLessThanOrEqual(596);
  });
  it('includes the first turn a fold needs', () => {
    const w = windowFor({ from: 920, to: 1000 }, total, 500, 490);
    expect(w.from).toBe(486);
    expect(w.to).toBeGreaterThan(500);
  });
});
