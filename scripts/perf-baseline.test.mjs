// Overnight 10-11/12 P1: `npm run perf`'s pure parts.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseArgs, rustTimings } from './perf-baseline.mjs';
import { interactiveAt, pct } from './perf-launch-chat.mjs';

test('perf options: everything by default, halves can be skipped, never the shared dev port', () => {
  assert.deepEqual(parseArgs([]), { build: true, rust: true, browser: true, out: null, port: 1459 });
  assert.deepEqual(parseArgs(['--skip-rust', '--skip-build', '--out', 'p.json', '--port', '1470']), { build: false, rust: false, browser: true, out: 'p.json', port: 1470 });
  assert.throws(() => parseArgs(['--port', '1420']), /other than 1420/);
  assert.throws(() => parseArgs(['--fast']), /unknown argument/);
});

test('the Rust report lines become a timing map', () => {
  const text = 'running 1 test\nperf_hot_paths workspace_sessions             15.8 ms\nperf_hot_paths list_workspaces                 7.1 ms\nnoise\n';
  assert.deepEqual(rustTimings(text), { workspace_sessions: 15.8, list_workspaces: 7.1 });
  // OF10's report adds thread CPU time; the map keeps wall time, and names
  // with spaces or parentheses parse whole.
  const now = 'perf_hot_paths read_agent_conversation           1.8 ms wall      1.6 ms cpu\nperf_hot_paths vault_search(operating plan)       30.2 ms wall     29.0 ms cpu\n';
  assert.deepEqual(rustTimings(now), { read_agent_conversation: 1.8, 'vault_search(operating plan)': 30.2 });
});

test('percentiles are nearest-rank, and interactive waits out long tasks until a quiet second', () => {
  assert.equal(pct([5, 1, 3, 2, 4], 50), 3);
  assert.equal(pct([1, 2, 3, 4, 100], 95), 100);
  assert.equal(pct([], 50), null);
  assert.equal(interactiveAt([], 400), 400, 'no long task: the shell is interactive');
  assert.equal(interactiveAt([{ start: 450, duration: 100 }, { start: 900, duration: 60 }], 400), 960);
  assert.equal(interactiveAt([{ start: 450, duration: 100 }, { start: 2000, duration: 60 }], 400), 550, 'a task after a quiet second is not launch');
});

test('a port something already listens on is not free (a stale server would be measured)', async () => {
  const { createServer } = await import('node:net');
  const { portFree } = await import('./perf-baseline.mjs');
  const s = createServer();
  await new Promise(r => s.listen(0, '127.0.0.1', r));
  const { port } = s.address();
  assert.equal(await portFree(port), false);
  await new Promise(r => s.close(r));
  assert.equal(await portFree(port), true);
});
