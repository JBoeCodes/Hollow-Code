// ST2: perf-switch's summary (median, p90 and the sample size it was taken from).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { summary } from './perf-switch.mjs';

test('summary reports n, p50, p90 and max, skipping missing samples', () => {
  assert.deepEqual(summary([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]), { n: 10, p50: 50, p90: 90, max: 100 });
  assert.deepEqual(summary([12.34, null, 56.78]), { n: 2, p50: 12.3, p90: 56.8, max: 56.8 });
  assert.deepEqual(summary([]), { n: 0, p50: null, p90: null, max: null });
});
