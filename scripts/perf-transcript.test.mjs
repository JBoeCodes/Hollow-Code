// Profiling night 11-20: the transcript budget's pure parts.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { TRANSCRIPT_BUDGET, overBudget } from './perf-transcript.mjs';

const ok = { maxTurnsInDom: 200, domNodesAtTop: 4726, frameP95Ms: 34.4, frameMaxMs: 45, heapOverHomeMB: 21.2, openMs: 150 };

test('a run inside every ceiling is within budget', () => {
  assert.deepEqual(overBudget(ok), []);
});

test('each ceiling is its own failure, named with its numbers', () => {
  assert.deepEqual(overBudget({ ...ok, maxTurnsInDom: 1000 }), ['turns in DOM 1000 > 240']);
  assert.deepEqual(overBudget({ ...ok, domNodesAtTop: 21132 }), ['DOM nodes 21132 > 9000']);
  assert.deepEqual(overBudget({ ...ok, frameMaxMs: 64.6 }), ['worst frame 64.6 > 60 ms']);
  assert.deepEqual(overBudget({ ...ok, heapOverHomeMB: 59.3 }), ['heap over Home 59.3 > 30 MB']);
  assert.equal(overBudget({ ...ok, openMs: 201, frameP95Ms: 41 }).length, 2);
});

test('the budget is a window of whole page loads: bigger than one 80-turn window, far under the 1,000-turn thread', () => {
  assert.ok(TRANSCRIPT_BUDGET.maxTurnsInDom > 80 && TRANSCRIPT_BUDGET.maxTurnsInDom < 1000);
  for (const [k, v] of Object.entries(TRANSCRIPT_BUDGET)) assert.ok(Number.isFinite(v) && v > 0, k);
});
