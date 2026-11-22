import { describe, expect, it } from 'vitest';
import { ROW_STEP, nextLimit, rowsDrawn } from './rowWindow';

describe('rowsDrawn', () => {
  it('draws a first step, or everything when the group is smaller', () => {
    expect(rowsDrawn(5000, null, -1)).toBe(ROW_STEP);
    expect(rowsDrawn(12, null, -1)).toBe(12);
    expect(rowsDrawn(0, null, -1)).toBe(0);
  });
  it('keeps the selected row drawn, whatever the limit', () => {
    expect(rowsDrawn(5000, null, 400)).toBe(401);
    expect(rowsDrawn(5000, 120, 10)).toBe(120);
    expect(rowsDrawn(30, null, 29)).toBe(30);
  });
  it('never draws more than the group holds', () => {
    expect(rowsDrawn(100, 500, -1)).toBe(100);
    expect(rowsDrawn(100, null, 900)).toBe(100);
  });
});

describe('nextLimit', () => {
  it('adds one step from what is drawn, up to the total', () => {
    expect(nextLimit(60, 5000)).toBe(120);
    expect(nextLimit(401, 5000)).toBe(461);
    expect(nextLimit(100, 130)).toBe(130);
  });
});
