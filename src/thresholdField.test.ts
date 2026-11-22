import { expect, it } from 'vitest';
import { thresholdRows } from './thresholdField';

it('produces deterministic bounded original glyph art with an open doorway', () => {
  const rows = thresholdRows('folded');
  expect(rows).toEqual(thresholdRows('folded'));
  expect(rows.length).toBeLessThanOrEqual(36);
  expect(rows.every(row => row.length <= 80 && /^[ .:+*oO#@]+$/.test(row))).toBe(true);
  expect(rows.join('').trim().length).toBeGreaterThan(100);
  expect(rows[18].slice(36,44).trim()).toBe('');
});

it('shares a bounded field for orbital and quiet variants without random state', () => {
  const full = thresholdRows('orbital');
  const quiet = thresholdRows('orbital', true);
  expect(full).not.toEqual(thresholdRows('folded'));
  expect(quiet.join('').replaceAll(' ', '').length).toBeLessThan(full.join('').replaceAll(' ', '').length);
  expect(full[18].slice(36,44).trim()).toBe('');
});
