// Agent effectiveness F8 (wave 3 · lane O, PR O1): the preparation
// baseline's report (scripts/agent-eval/prep-bench.mjs) over the bench's
// output and run manifests.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { STEPS, loadInputs, percentile, render, report, stageOf, summarize, totalOf } from './agent-eval/prep-bench.mjs';

const rust = (file) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');

/** A timeline with `ran` steps ({name: [start, duration]}), the rest null. */
function timeline(ran, totalMs = null) {
  return {
    origin: 'admission',
    totalMs,
    phases: STEPS.map((name) => ({ name, startMs: ran[name]?.[0] ?? null, durationMs: ran[name]?.[1] ?? null })),
  };
}

test('the step vocabulary is prep_timing.rs\'s, in its order', () => {
  const source = rust('prep_timing.rs');
  const names = [...source.matchAll(/Step::\w+ => "(\w+)",/g)].map((m) => m[1]);
  assert.deepEqual(names, STEPS);
});

test('nearest-rank percentiles', () => {
  const values = [5, 1, 4, 2, 3, 6, 7, 8, 9, 10];
  assert.equal(percentile(values, 50), 5);
  assert.equal(percentile(values, 95), 10);
  assert.equal(percentile([7], 95), 7);
  assert.equal(percentile([], 50), null);
  assert.equal(percentile([null, 3], 50), 3);
});

test('the total is totalMs, else the last preparation step\'s end (first outputs excluded)', () => {
  assert.equal(totalOf(timeline({ retrieval: [10, 5] }, 900)), 900);
  assert.equal(totalOf(timeline({ admission: [0, 40], connectors: [50, 4000], command: [4060, 2], firstStdout: [4070, 9000] })), 4062);
  assert.equal(totalOf(timeline({})), null);
});

test('steps that did not run are counted, not averaged as zero', () => {
  const s = summarize([
    timeline({ admission: [0, 10], connectorsJev: [20, 4000] }),
    timeline({ admission: [0, 30] }),
    timeline({ admission: [0, 20], connectorsJev: [20, 4002] }),
  ]);
  const step = (name) => s.steps.find((x) => x.name === name);
  assert.equal(s.samples, 3);
  assert.deepEqual(step('admission'), { name: 'admission', ran: 3, p50: 20, p95: 30, max: 30 });
  assert.deepEqual(step('connectorsJev'), { name: 'connectorsJev', ran: 2, p50: 4000, p95: 4002, max: 4002 });
  assert.equal(step('codexInventory').ran, 0);
  assert.equal(step('codexInventory').p50, null);
  assert.deepEqual(s.total, { p50: 4020, p95: 4022, max: 4022 });
});

test('bench files group by mode and manifests by kind; old manifests are skipped with a reason', () => {
  const dir = mkdtempSync(join(tmpdir(), 'prep-bench-'));
  try {
    const sample = (ms, outcome) => ({ timeline: timeline({ admission: [0, 10], connectors: [10, ms] }), jev: [{ use: 'connectors', outcome, items: 24 }], sizes: { pool: 24, kept: 8 } });
    writeFileSync(join(dir, 'timeout.json'), JSON.stringify({ kind: 'hollow-prep-bench', mode: 'timeout', samples: [sample(4003, 'timeout'), sample(4005, 'timeout')] }));
    writeFileSync(join(dir, 'run-a.json'), JSON.stringify({ schemaVersion: 1, runId: 'run-a', layers: [], preparation: { retrievalMs: 3, timeline: timeline({ admission: [0, 5], spawn: [40, 2] }, 42) } }));
    writeFileSync(join(dir, 'run-b.json'), JSON.stringify({ runId: 'run-b', manifest: { schemaVersion: 1, runId: 'run-b', layers: [], preparation: { timeline: timeline({ admission: [0, 7] }, 50) } } }));
    writeFileSync(join(dir, 'run-old.json'), JSON.stringify({ schemaVersion: 1, runId: 'run-old', layers: [], preparation: { retrievalMs: 3 } }));
    writeFileSync(join(dir, 'other.json'), JSON.stringify({ hello: 1 }));
    const { groups, skipped } = loadInputs([dir]);
    assert.deepEqual([...groups.keys()].sort(), ['manifests', 'timeout']);
    assert.equal(groups.get('timeout').timelines.length, 2);
    assert.equal(groups.get('manifests').timelines.length, 2);
    assert.deepEqual(skipped.map((s) => s.reason).sort(), ['a manifest from before the preparation timeline (wave 3)', 'neither a bench file nor a run manifest']);
    const r = report([dir]);
    assert.equal(r.groups.timeout.jev, 'connectors timeout×2');
    assert.deepEqual(r.groups.timeout.sizes, { pool: 24, kept: 8 });
    assert.deepEqual(r.groups.manifests.summary.total, { p50: 42, p95: 50, max: 50 });
    const text = render(r);
    assert.match(text, /### timeout \(2 samples\)/);
    assert.match(text, /\| connectors \| 2 \| 4003 \| 4005 \| 4005 \|/);
    assert.match(text, /\| \*\*total\*\* \| 2 \| 42 \| 50 \| 50 \|/);
    assert.match(text, /Skipped: /);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the bench writes what the report reads, and only to a loopback fake', () => {
  const bench = rust('prep_bench.rs');
  assert.match(bench, /"kind": "hollow-prep-bench"/);
  assert.match(bench, /"mode": mode/);
  assert.match(bench, /json!\(\{ "timeline": timeline, "jev": uses, "sizes": sizes, "deadline": deadline, "semantic": stats\.semantic, "codexInventory": manifest\.preparation\.codex_inventory \}\)/);
  for (const mode of ['off', 'score', 'timeout', 'malformed', 'score-shared', 'timeout-shared', 'semantic-slow', 'codex', 'codex-cached']) assert.ok(bench.includes(`"${mode}"`), mode);
  // O3: the Codex modes list through a stand-in CLI, never the installed one.
  assert.match(bench, /codex_inventory::fake::/);
  assert.match(bench, /agent::build_command_via\(AgentKind::Codex/);
  // O2: the bench prepares through the production stage, as a chat turn does.
  assert.match(bench, /prep_deadline::concurrently\(/);
  assert.match(bench, /directory_connections::prioritized_context\(/);
  assert.match(bench, /context_rank::select_connectors\(/);
  assert.match(bench, /loopback_jev_url/);
  assert.ok(!bench.includes('typesafe.ai'), 'never TypeSafe');
  assert.match(bench, /#\[ignore = /, 'run on purpose only');
});

test('O2: the stage is retrieval and connectors together, and the deadline line counts what was cut', () => {
  // At once: connectors 10+3001 beside retrieval 10+60.
  assert.equal(stageOf(timeline({ retrieval: [10, 60], connectors: [10, 3001] })), 3001);
  // Serially (O1): retrieval 10+4060, then connectors 4070+4002.
  assert.equal(stageOf(timeline({ retrieval: [10, 4060], connectors: [4070, 4002] })), 8062);
  assert.equal(stageOf(timeline({ admission: [0, 5] })), null);
  const dir = mkdtempSync(join(tmpdir(), 'prep-bench-'));
  try {
    const cut = { budgetMs: 3000, cut: ['retrievalJev', 'connectorsJev'] };
    const sample = (ms, deadline) => ({ timeline: timeline({ admission: [0, 10], retrieval: [12, 60], connectors: [12, ms] }), jev: [{ use: 'connectors', outcome: deadline.cut.length ? 'deadline' : 'circuit-open', items: 24 }], deadline });
    writeFileSync(join(dir, 'timeout-shared.json'), JSON.stringify({ kind: 'hollow-prep-bench', mode: 'timeout-shared', samples: [sample(3003, cut), sample(3005, cut), sample(2, { budgetMs: 3000, cut: [] })] }));
    writeFileSync(join(dir, 'run-a.json'), JSON.stringify({ schemaVersion: 1, runId: 'run-a', layers: [], jev: [{ use: 'connectors', outcome: 'cached', items: 24 }], preparation: { timeline: timeline({ retrieval: [5, 40], connectors: [5, 3] }, 90), deadline: { budgetMs: 3000, cut: [] } } }));
    const r = report([dir]);
    assert.deepEqual(r.groups['timeout-shared'].summary.stage, { ran: 3, p50: 3003, p95: 3005, max: 3005 });
    assert.equal(r.groups['timeout-shared'].deadline, 'budget 3000 ms; cut connectorsJev×2, retrievalJev×2');
    assert.equal(r.groups['timeout-shared'].jev, 'connectors deadline×2, connectors circuit-open×1');
    assert.equal(r.groups.manifests.deadline, 'budget 3000 ms; nothing cut');
    assert.equal(r.groups.manifests.jev, 'connectors cached×1');
    const text = render(r);
    assert.match(text, /Deadline: budget 3000 ms; cut connectorsJev×2, retrievalJev×2/);
    assert.match(text, /\| \*\*stage\*\* \(retrieval ∪ connectors\) \| 3 \| 3003 \| 3005 \| 3005 \|/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('O3: the Codex inventory line counts how each Codex run was served', () => {
  const dir = mkdtempSync(join(tmpdir(), 'prep-bench-'));
  try {
    const sample = (served) => ({ timeline: timeline({ admission: [0, 10], command: [20, served === 'listed' ? 47 : 0], codexInventory: [20, served === 'listed' ? 47 : 0] }), jev: [], codexInventory: served });
    writeFileSync(join(dir, 'codex-cached.json'), JSON.stringify({ kind: 'hollow-prep-bench', mode: 'codex-cached', samples: [sample('listed'), sample('cached'), sample('cached')] }));
    writeFileSync(join(dir, 'off.json'), JSON.stringify({ kind: 'hollow-prep-bench', mode: 'off', samples: [{ timeline: timeline({ admission: [0, 10] }), jev: [], codexInventory: null }] }));
    writeFileSync(join(dir, 'run-c.json'), JSON.stringify({ schemaVersion: 1, runId: 'run-c', layers: [], jev: [], preparation: { timeline: timeline({ codexInventory: [5, 0] }, 9), codexInventory: 'cached' } }));
    const r = report([dir]);
    assert.equal(r.groups['codex-cached'].codexInventory, 'listed×1, cached×2');
    assert.equal(r.groups.off.codexInventory, null, 'no Codex run, no line');
    assert.equal(r.groups.manifests.codexInventory, 'cached×1');
    const text = render(r);
    assert.match(text, /Codex inventory: listed×1, cached×2/);
    assert.match(text, /\| codexInventory \| 3 \| 0 \| 47 \| 47 \|/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
