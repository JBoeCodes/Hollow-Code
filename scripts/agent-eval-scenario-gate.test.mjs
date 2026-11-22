// ND3 + ND4 + BG4: the eval gate by scenario fingerprint
// (scripts/agent-eval/scenario-gate.mjs). The real test: every scenario in the
// repo fails when the agent does nothing, and no ledger entry is stale. The
// rest pin the rules on synthetic scenarios. No model is called.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  REQUIRED_PASSES, admit, canonical, controlledHealthCheck, fingerprintOf, gateReport, loadLedger, loadScenarios, main, negativeControl, renderGate,
} from './agent-eval/scenario-gate.mjs';

const lift = (rules, over = {}) => ({ key: 'lift/s/c', kind: 'lift', id: 'c', content: { skill: 's', case: { id: 'c', prompt: 'p', negative: false, rules, ...over } } });
const routing = (expect, confusable = []) => ({ key: 'routing/r', kind: 'routing', id: 'r', content: { id: 'r', prompt: 'p', expect, confusable } });
const empty = { schema: 1, required: 5, scenarios: {} };
const five = [true, true, true, true, true];

test('every scenario in the repo fails when the agent does nothing, and the ledger has no stale entry', () => {
  const scenarios = loadScenarios();
  assert.ok(scenarios.length >= 20, `found ${scenarios.length} scenarios`);
  assert.deepEqual([...new Set(scenarios.map((s) => s.kind))].sort(), ['lift', 'pilot', 'routing']);
  const r = gateReport(scenarios, loadLedger());
  assert.equal(r.ok, true, r.problems.join('\n'));
});

test('fingerprints ignore key order and change with any content edit', () => {
  assert.equal(canonical({ b: 1, a: [{ d: 1, c: 2 }] }), canonical({ a: [{ c: 2, d: 1 }], b: 1 }));
  const a = fingerprintOf({ x: { y: 'one' } });
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(a, fingerprintOf({ x: { y: 'one' } }));
  assert.notEqual(a, fingerprintOf({ x: { y: 'one ' } }));
});

test('a rule-graded scenario with only absent rules, or a rule an empty answer meets, has no control', () => {
  assert.equal(negativeControl(lift([{ id: 'a', kind: 'absent', pattern: 'x' }])).ok, false);
  assert.equal(negativeControl(lift([{ id: 'a', kind: 'contains', pattern: '.*' }])).ok, false);
  assert.equal(negativeControl(lift([{ id: 'a', kind: 'contains', pattern: 'Done' }])).ok, false, 'a no-op answer that says Done must not pass');
  const ok = negativeControl(lift([{ id: 'a', kind: 'contains', any: ['Thursday'] }, { id: 'b', kind: 'absent', pattern: 'x' }]));
  assert.equal(ok.ok, true);
});

test('a routing case that expects a skill must score missed; one that expects none must score falseFire', () => {
  assert.equal(negativeControl(routing('humanizer', ['slideshow'])).ok, true);
  assert.equal(negativeControl(routing(null, ['slideshow'])).ok, true);
  assert.match(negativeControl(routing(null, [])).detail, /falseFire/);
  assert.equal(negativeControl(routing(null, [])).ok, true, 'a none case with no near miss still detects a false fire');
});

test('admission: no entry is pending, a match at 5 of 5 is admitted, an edit makes it stale and red', () => {
  const s = lift([{ id: 'a', kind: 'contains', any: ['Thursday'] }]);
  let r = gateReport([s], empty);
  assert.deepEqual([r.ok, r.pending, r.admitted], [true, 1, 0]);
  const ledger = admit(empty, s, five, { now: new Date('2026-11-20T00:00:00Z') });
  assert.equal(ledger.scenarios['lift/s/c'].admitted, '2026-11-20');
  r = gateReport([s], ledger);
  assert.deepEqual([r.ok, r.admitted], [true, 1]);
  const edited = lift([{ id: 'a', kind: 'contains', any: ['Friday'] }]);
  r = gateReport([edited], ledger);
  assert.equal(r.ok, false);
  assert.equal(r.rows[0].state, 'stale');
  assert.match(renderGate(r), /RED/);
  assert.match(r.problems[0], /content changed since admission/);
});

test('admission needs exactly 5 of 5 and a passing negative control; orphans and short entries are red', () => {
  const s = lift([{ id: 'a', kind: 'contains', any: ['Thursday'] }]);
  assert.throws(() => admit(empty, s, [true, true, true, true]), /exactly 5/);
  assert.throws(() => admit(empty, s, [true, true, true, true, false]), /4 of 5/);
  assert.throws(() => admit(empty, s, five.map((pass) => ({ pass: !pass }))), /0 of 5/);
  assert.equal(admit(empty, s, five.map((pass) => ({ pass }))).scenarios['lift/s/c'].of, REQUIRED_PASSES);
  const vacuous = lift([{ id: 'a', kind: 'absent', pattern: 'x' }]);
  assert.throws(() => admit(empty, vacuous, five), /negative control failed/);
  const weak = { ...empty, scenarios: { 'lift/s/c': { fingerprint: fingerprintOf(s.content), passes: 4, of: 5 } } };
  assert.match(gateReport([s], weak).problems[0], /not 5 of 5/);
  const orphan = { ...empty, scenarios: { 'lift/gone/x': { fingerprint: 'a', passes: 5, of: 5 } } };
  assert.match(gateReport([s], orphan).problems[0], /no longer exists/);
});

test('a health check counts only when it says yes to live and no to dead', async () => {
  const live = 'up', dead = 'down';
  assert.equal((await controlledHealthCheck((t) => t === live, { live, dead })).ok, true);
  const vacuous = await controlledHealthCheck(() => true, { live, dead });
  assert.deepEqual([vacuous.ok, /vacuous/.test(vacuous.detail)], [false, true]);
  assert.equal((await controlledHealthCheck(() => false, { live, dead })).ok, false);
  assert.equal((await controlledHealthCheck((t) => { if (t === dead) throw new Error('refused'); return true; }, { live, dead })).ok, true, 'a throw is a no');
});

test('status prints the verdict and exits 0 on the repo as it is', () => {
  const lines = [];
  assert.equal(main(['status'], { log: (l) => lines.push(l) }), 0);
  assert.match(lines[0], /Eval gate: green/);
  assert.equal(main(['admit', 'pilot/nope'], { log: () => {} }), 2);
});
