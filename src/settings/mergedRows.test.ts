import { describe, expect, it } from 'vitest';
import { parseWhole } from './mergedRows';

describe('merged row parsing', () => {
  it('takes whole numbers inside the range, ends included', () => {
    expect(parseWhole('5', 5, 720)).toBe(5);
    expect(parseWhole(' 720 ', 5, 720)).toBe(720);
    expect(parseWhole('4', 5, 720)).toBeNull();
    expect(parseWhole('721', 5, 720)).toBeNull();
  });
  it('refuses text, decimals, signs and an empty box', () => {
    for (const bad of ['', ' ', 'ten', '1.5', '-3', '+3', '1e2', '0x10']) expect(parseWhole(bad, 0, 1000)).toBeNull();
  });
});
