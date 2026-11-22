import { describe, expect, it } from 'vitest';
import { RAIL_MAX_DASHES, currentPrompt, dashFor, railDashes, railTabStop } from './chatOutlineRail';

describe('railDashes', () => {
  it('gives every prompt a dash up to the cap', () => {
    expect(railDashes(0)).toEqual([]);
    expect(railDashes(3)).toEqual([0, 1, 2]);
    expect(railDashes(RAIL_MAX_DASHES)).toHaveLength(RAIL_MAX_DASHES);
  });
  it('samples a longer chat evenly, keeping the first and last prompt', () => {
    const d = railDashes(200);
    expect(d).toHaveLength(RAIL_MAX_DASHES);
    expect(d[0]).toBe(0);
    expect(d[d.length - 1]).toBe(199);
    for (let i = 1; i < d.length; i++) expect(d[i]).toBeGreaterThan(d[i - 1]);
    expect(railDashes(5, 3)).toEqual([0, 2, 4]);
  });
});

describe('dashFor', () => {
  it('marks the dash at or before the current prompt', () => {
    expect(dashFor([0, 2, 4], 0)).toBe(0);
    expect(dashFor([0, 2, 4], 3)).toBe(1);
    expect(dashFor([0, 2, 4], 9)).toBe(2);
    expect(dashFor([0, 2, 4], -1)).toBe(-1);
  });
});

describe('currentPrompt', () => {
  const prompts = [0, 4, 8, 12];
  it('picks the last prompt above the reading line', () => {
    expect(currentPrompt(prompts, [{ index: 0, top: -900 }, { index: 4, top: -100 }, { index: 8, top: 400 }], 200)).toBe(4);
  });
  it('with none above the line, takes the prompt before the render window', () => {
    expect(currentPrompt(prompts, [{ index: 8, top: 300 }, { index: 12, top: 900 }], 200)).toBe(4);
    expect(currentPrompt(prompts, [{ index: 0, top: 300 }], 200)).toBe(0);
  });
  it('is -1 with nothing rendered', () => {
    expect(currentPrompt(prompts, [], 200)).toBe(-1);
    expect(currentPrompt([], [{ index: 0, top: 0 }], 200)).toBe(-1);
  });
});

// #1322 re-review: ↑, ↓, Home and End moved focus but not the tab stop, so
// Tab could land on the current prompt's dash again.
describe('railTabStop', () => {
  it('follows keyboard focus along the strip, else the current dash', () => {
    expect(railTabStop(40, 39, null)).toBe(39);
    expect(railTabStop(40, 39, 0)).toBe(0);
    expect(railTabStop(40, -1, null)).toBe(0);
  });
  it('stays inside a strip that re-rendered shorter, and is -1 with no dashes', () => {
    expect(railTabStop(3, 39, 12)).toBe(2);
    expect(railTabStop(0, 0, null)).toBe(-1);
  });
});
