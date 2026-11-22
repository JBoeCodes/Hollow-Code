// Agent effectiveness E1: the Jev study scaffold (scripts/agent-eval/jev-study.mjs,
// scripts/agent-eval/jev-fixtures/). Four conditions, one judged fixture
// format and one scorer each. The scorers read recorded outputs only: nothing
// here calls Jev, a model or the network (the study itself runs later, by
// the orchestrator, under Q3).
import assert from 'node:assert/strict';
import { cpSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { HEADLINE, JEV_CONDITIONS, loadJevFixture, main, renderStudy, scoreJev, scoreStudy, validateJevFixture, validateJevOutputs } from './agent-eval/jev-study.mjs';

const DIR = fileURLToPath(new URL('./agent-eval/jev-fixtures/', import.meta.url));
const fixture = (condition) => loadJevFixture(`${DIR}${condition}.json`);
const outputs = (condition, arm, results) => ({ schema: 1, condition, arm, results });

test('three conditions, one per Jev use, each with a judged example fixture', () => {
  assert.deepEqual(Object.keys(JEV_CONDITIONS).sort(), ['connector-relevance', 'passage-rerank', 'proposal-dedup']);
  assert.deepEqual(readdirSync(DIR).filter((f) => f.endsWith('.json')).sort(), Object.keys(JEV_CONDITIONS).map((c) => `${c}.json`).sort());
  for (const condition of Object.keys(JEV_CONDITIONS)) {
    const f = fixture(condition);
    assert.equal(f.condition, condition);
    assert.ok(f.items.length >= 2, `${condition}: at least two judged items`);
    assert.ok(JEV_CONDITIONS[condition].arms.includes('jev') && JEV_CONDITIONS[condition].arms.length >= 2, `${condition}: Jev against a local arm`);
  }
  // The connector fixture keeps the cardinality case: a named connector past Jev's 24.
  const big = fixture('connector-relevance').items.find((i) => i.candidates.length > 24);
  assert.ok(big && big.named.some((id) => big.candidates.findIndex((c) => c.id === id) >= 24));
});

test('fixture validation refuses labels that point nowhere', () => {
  const ok = JSON.parse(readFileSync(`${DIR}connector-relevance.json`, 'utf8'));
  const item = ok.items[0];
  const bad = [
    [{ ...ok, condition: 'model-routing' }, /condition/],
    [{ ...ok, items: [] }, /items/],
    [{ ...ok, items: [{ ...item, relevant: ['no-such'] }] }, /relevant/],
    [{ ...ok, items: [{ ...item, named: [item.candidates.find((c) => !item.relevant.includes(c.id)).id] }] }, /named/],
    [{ ...ok, items: [item, item] }, /twice/],
  ];
  for (const [json, why] of bad) assert.throws(() => validateJevFixture(json), why, String(why));
  const dedup = JSON.parse(readFileSync(`${DIR}proposal-dedup.json`, 'utf8'));
  assert.throws(() => validateJevFixture({ ...dedup, items: [{ ...dedup.items[0], duplicateOf: 'missing' }] }), /duplicateOf/);
});

test('outputs must answer every item of their fixture, for one of its arms', () => {
  const f = fixture('passage-rerank');
  const all = f.items.map((i) => ({ id: i.id, kept: i.relevant, outcome: 'ok', latencyMs: 10, costUsd: null }));
  assert.doesNotThrow(() => validateJevOutputs(f, outputs('passage-rerank', 'jev', all)));
  assert.throws(() => validateJevOutputs(f, outputs('passage-rerank', 'oracle', all)), /arm/);
  assert.throws(() => validateJevOutputs(f, outputs('connector-relevance', 'jev', all)), /condition/);
  assert.throws(() => validateJevOutputs(f, outputs('passage-rerank', 'jev', all.slice(1))), /no result for/);
  assert.throws(() => validateJevOutputs(f, outputs('passage-rerank', 'jev', [...all, { ...all[0], id: 'extra' }])), /not in the fixture/);
  assert.throws(() => validateJevOutputs(f, outputs('passage-rerank', 'jev', all.map((r) => ({ ...r, outcome: 'great' })))), /outcome/);
});

test('connector relevance: recall, precision, and the two failures that disqualify Jev (a named connector dropped, a wrong-length vector)', () => {
  const f = fixture('connector-relevance');
  const perfect = f.items.map((i) => ({ id: i.id, decisions: i.candidates.map((c) => ({ id: c.id, attach: i.relevant.includes(c.id) })), outcome: 'ok', latencyMs: 300, costUsd: null }));
  const good = scoreJev(f, outputs('connector-relevance', 'jev', perfect));
  assert.equal(good.status, 'scored');
  assert.deepEqual([good.metrics.recall, good.metrics.precision, good.metrics.namedDropped, good.metrics.cardinalityFailures], [1, 1, 0, 0]);
  assert.equal(good.gate.pass, true);
  assert.equal(good.cost.totalUsd, null, 'an unknown Jev cost stays null, never 0');
  // A 24-long vector for a 26-candidate request: the F5 defect.
  const big = f.items.find((i) => i.candidates.length > 24);
  const truncated = perfect.map((r) => (r.id === big.id ? { ...r, decisions: r.decisions.slice(0, 24) } : r));
  const bad = scoreJev(f, outputs('connector-relevance', 'jev', truncated));
  assert.equal(bad.metrics.cardinalityFailures, 1);
  assert.ok(bad.metrics.namedDropped >= 1, 'the named connector past 24 is lost');
  assert.equal(bad.gate.pass, false);
  assert.match(bad.gate.reasons.join(' '), /named/);
});

test('passage rerank: judged evidence kept, stale passages kept, and evidence dropped', () => {
  const f = fixture('passage-rerank');
  const run = (pick) => scoreJev(f, outputs('passage-rerank', 'local', f.items.map((i) => ({ id: i.id, kept: pick(i), outcome: 'ok', latencyMs: 5, costUsd: 0 }))));
  const all = run((i) => i.passages.map((p) => p.id));
  assert.equal(all.metrics.recall, 1);
  assert.ok(all.metrics.staleKept >= 1, 'keeping everything keeps the stale passage too');
  const none = run(() => []);
  assert.equal(none.metrics.recall, 0);
  assert.equal(none.metrics.droppedRelevant, f.items.reduce((n, i) => n + i.relevant.length, 0));
  assert.equal(none.cost.totalUsd, 0, 'a known zero (the local arm) is zero');
  assert.throws(() => run((i) => [i.passages[0].id, i.passages[0].id]), /twice/);
});

test('proposal dedup: a novel proposal dropped is the costly error; flags are review work', () => {
  const f = fixture('proposal-dedup');
  const verdicts = (fn) => scoreJev(f, outputs('proposal-dedup', 'jev', f.items.map((i) => ({ id: i.id, ...fn(i), outcome: 'ok', latencyMs: 200, costUsd: null }))));
  const right = verdicts((i) => (i.duplicateOf ? { verdict: 'duplicate', of: i.duplicateOf } : { verdict: 'new', of: null }));
  assert.deepEqual([right.metrics.falseOmissions, right.metrics.missedDuplicates, right.metrics.wrongTarget, right.metrics.flagged], [0, 0, 0, 0]);
  const dropAll = verdicts((i) => ({ verdict: 'duplicate', of: i.existing[0].id }));
  assert.equal(dropAll.metrics.falseOmissions, f.items.filter((i) => !i.duplicateOf).length);
  assert.equal(dropAll.gate.pass, false);
  const flagAll = verdicts((i) => ({ verdict: 'possible', of: i.existing[0].id }));
  assert.equal(flagAll.metrics.flagged, f.items.length);
});

test('failures are counted by outcome, and latency is summarised', () => {
  const f = fixture('passage-rerank');
  const out = f.items.map((i, n) => ({ id: i.id, kept: i.relevant, outcome: n ? 'timeout' : 'ok', latencyMs: n ? 4000 : 300, costUsd: null }));
  const s = scoreJev(f, outputs('passage-rerank', 'jev', out));
  assert.equal(s.outcomes.ok, 1);
  assert.equal(s.outcomes.timeout, f.items.length - 1);
  assert.equal(s.latency.p95Ms, 4000);
});

test('the scaffold reads files only: no Jev, model or network call', () => {
  const src = readFileSync(new URL('./agent-eval/jev-study.mjs', import.meta.url), 'utf8');
  assert.doesNotMatch(src, /node:(http|https|net|child_process)|fetch\(|typesafe\.ai|XMLHttpRequest/);
});

test('follow-up: each answer is checked for its shape, so a malformed one is never scored', () => {
  const dedup = fixture('proposal-dedup');
  const novel = dedup.items.find((i) => !i.duplicateOf);
  const twin = dedup.items.find((i) => i.duplicateOf);
  const verdicts = (change) => outputs('proposal-dedup', 'jev', dedup.items.map((i) => ({
    id: i.id, outcome: 'ok', latencyMs: 1, costUsd: null,
    ...(i.duplicateOf ? { verdict: 'duplicate', of: i.duplicateOf } : { verdict: 'new', of: null }),
    ...change(i),
  })));
  assert.doesNotThrow(() => validateJevOutputs(dedup, verdicts(() => ({}))));
  // An unknown or missing verdict used to count as correct for a novel proposal.
  assert.throws(() => validateJevOutputs(dedup, verdicts((i) => (i === novel ? { verdict: 'maybe' } : {}))), /verdict/);
  assert.throws(() => validateJevOutputs(dedup, verdicts((i) => (i === novel ? { verdict: undefined } : {}))), /verdict/);
  assert.throws(() => validateJevOutputs(dedup, verdicts((i) => (i === twin ? { of: 'nowhere' } : {}))), /of/);

  const conn = fixture('connector-relevance');
  const decided = (attach) => outputs('connector-relevance', 'jev', conn.items.map((i) => ({ id: i.id, outcome: 'ok', latencyMs: 1, costUsd: null, decisions: i.candidates.map((c) => ({ id: c.id, attach })) })));
  assert.doesNotThrow(() => validateJevOutputs(conn, decided(false)));
  assert.throws(() => validateJevOutputs(conn, decided('yes')), /attach/);

  const rerank = fixture('passage-rerank');
  assert.throws(() => validateJevOutputs(rerank, outputs('passage-rerank', 'jev', rerank.items.map((i) => ({ id: i.id, outcome: 'ok', latencyMs: 1, costUsd: null, kept: 'all' })))), /kept/);
});

test('#386 review: a result whose Jev call timed out, failed or was off carries the fallback answer; a missing one is a failure, not a rejected file', () => {
  const conn = fixture('connector-relevance');
  const answer = (i) => ({ decisions: i.candidates.map((c) => ({ id: c.id, attach: i.relevant.includes(c.id) })) });
  for (const outcome of ['timeout', 'error', 'off', 'malformed']) {
    const results = conn.items.map((i, n) => ({ id: i.id, outcome: n ? 'ok' : outcome, latencyMs: 1, costUsd: null, ...(n ? answer(i) : {}) }));
    const out = outputs('connector-relevance', 'jev', results);
    assert.doesNotThrow(() => validateJevOutputs(conn, out), outcome);
    const s = scoreJev(conn, out);
    assert.equal(s.metrics.missingAnswers, 1, outcome);
    assert.equal(s.gate.pass, false);
    assert.match(s.gate.reasons.join(' '), /1 result\(s\) without an answer/);
  }
  // An `ok` result without its answer is still a malformed file.
  const okMissing = conn.items.map((i, n) => ({ id: i.id, outcome: 'ok', latencyMs: 1, costUsd: null, ...(n ? answer(i) : {}) }));
  assert.throws(() => validateJevOutputs(conn, outputs('connector-relevance', 'jev', okMissing)), /decisions/);
  // A fallback answer that is there is scored like any other.
  const fallback = conn.items.map((i) => ({ id: i.id, outcome: 'timeout', latencyMs: 4000, costUsd: null, ...answer(i) }));
  const s = scoreJev(conn, outputs('connector-relevance', 'jev', fallback));
  assert.equal(s.metrics.missingAnswers, 0);
  assert.equal(s.metrics.recall, 1);
  for (const condition of ['passage-rerank', 'proposal-dedup']) {
    const f = fixture(condition);
    const res = f.items.map((i) => ({ id: i.id, outcome: 'timeout', latencyMs: 4000, costUsd: null }));
    const scored = scoreJev(f, outputs(condition, 'jev', res));
    assert.equal(scored.metrics.missingAnswers, f.items.length, condition);
    assert.equal(scored.gate.pass, false, condition);
  }
});

// ── E3: the runner's study folders, scored ──────────────────────────────
// scripts/agent-eval/fixtures/jev-study/<mode>/ is what `hollow jev-study`
// writes against the loopback fake (score, timeout and malformed mode);
// jev_study::tests::the_recorded_study_folders_are_what_the_runner_writes
// keeps them in step with the Rust runner.

const STUDY = (mode) => fileURLToPath(new URL(`./agent-eval/fixtures/jev-study/${mode}/`, import.meta.url));
const studyOf = (mode) => scoreStudy(STUDY(mode), { fixturesDir: DIR });

test('E3: a score-mode study compares every use, local against Jev, on the same judged items with intervals', () => {
  const r = studyOf('score');
  assert.equal(r.endpoint, 'loopback');
  assert.equal(r.costUsd, null, 'TypeSafe reports no cost: null, never 0');
  assert.deepEqual(r.conditions.map((c) => [c.condition, c.status]), Object.keys(JEV_CONDITIONS).map((c) => [c, 'compared']));
  const calls = r.conditions.reduce((n, c) => n + c.calls, 0);
  assert.equal(calls, r.calls, 'the per-item request counts add up to the run\'s');
  for (const c of r.conditions) {
    assert.equal(c.arms.local.outcomes.off, c.items, `${c.condition}: the local condition never asks Jev`);
    assert.equal(c.arms.local.cost.totalUsd, 0, 'no request is a known 0');
    assert.ok(c.arms.jev.outcomes.ok + c.arms.jev.outcomes['not-sent'] === c.items);
    const head = c.difference[HEADLINE[c.condition]];
    assert.ok(head && head.low <= head.difference && head.difference <= head.high, `${c.condition}: the estimate sits in its interval`);
    assert.ok(Number.isFinite(c.latency.p95MsAdded));
    assert.equal(typeof c.keep.candidate, 'boolean');
  }
  // The fake attaches every even-numbered connector it is sent: precision
  // falls, and the interval says so.
  const conn = r.conditions.find((c) => c.condition === 'connector-relevance');
  assert.ok(conn.arms.jev.metrics.precision < conn.arms.local.metrics.precision);
  assert.match(conn.finding, /jev worse/);
  assert.equal(conn.arms.jev.metrics.namedDropped, 0, 'a named connector is never sent, so never dropped');
  assert.equal(conn.arms.jev.metrics.cardinalityFailures, 0, 'one decision per candidate, past 24 too');
  // The fake drops the best shortlisted record's proposal: novel ones too.
  const dedup = r.conditions.find((c) => c.condition === 'proposal-dedup');
  assert.ok(dedup.arms.jev.metrics.falseOmissions > 0);
  assert.equal(dedup.arms.jev.gate.pass, false);
  assert.equal(dedup.keep.candidate, false);
  assert.match(renderStudy(r), /\| connector-relevance \| 4 \|/);
});

test('E3: timeout and malformed studies keep production\'s local decision, so Jev adds only latency and failures', () => {
  for (const mode of ['timeout', 'malformed']) {
    const r = studyOf(mode);
    for (const c of r.conditions) {
      assert.equal(c.status, 'compared', `${mode} ${c.condition}`);
      const head = c.difference[HEADLINE[c.condition]];
      assert.equal(head.difference, 0, `${mode} ${c.condition}: the fallback is the local decision`);
      assert.match(c.finding, /^no difference/);
      assert.equal(c.arms.jev.metrics.missingAnswers, 0, 'every fallback result carries its answer');
      assert.ok(c.arms.jev.outcomes[mode] >= 1, `${mode} ${c.condition}: ${JSON.stringify(c.arms.jev.outcomes)}`);
      assert.equal(c.arms.jev.outcomes.ok, 0);
    }
    if (mode === 'timeout') {
      assert.ok(r.conditions.every((c) => c.latency.p50MsAdded >= 3900), 'a timeout costs the 4 s bound');
      assert.equal(r.calls, Object.keys(JEV_CONDITIONS).length, 'one request per use, never retried');
    }
  }
});

test('E3: a stopped run\'s incomplete arm and a changed fixture are reported, never scored', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'jev-study-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  cpSync(STUDY('score'), root, { recursive: true });
  const run = JSON.parse(readFileSync(join(root, 'run.json'), 'utf8'));
  run.stopped = { reason: '--max-calls 2 reached', condition: 'passage-rerank', before: 'cadence-now' };
  writeFileSync(join(root, 'run.json'), JSON.stringify(run));
  const jev = JSON.parse(readFileSync(join(root, 'passage-rerank.jev.json'), 'utf8'));
  writeFileSync(join(root, 'passage-rerank.jev.json'), JSON.stringify({ ...jev, complete: false, results: jev.results.slice(0, 1) }));
  const r = scoreStudy(root, { fixturesDir: DIR });
  const pass = r.conditions.find((c) => c.condition === 'passage-rerank');
  assert.equal(pass.status, 'incomplete');
  assert.match(pass.reason, /--max-calls 2 reached/);
  assert.equal(r.conditions.find((c) => c.condition === 'connector-relevance').status, 'compared');
  assert.throws(() => validateJevOutputs(fixture('passage-rerank'), { ...jev, complete: false }), /stopped before every item/);
  run.conditions[0].fixtureSha256 = '0'.repeat(64);
  writeFileSync(join(root, 'run.json'), JSON.stringify(run));
  assert.equal(scoreStudy(root, { fixturesDir: DIR }).conditions[0].status, 'fixture-changed');
});

test('E3 follow-up: an interrupted study (state running, an arm not yet written) is reported, never scored or thrown on', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'jev-study-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  cpSync(STUDY('score'), root, { recursive: true });
  const run = JSON.parse(readFileSync(join(root, 'run.json'), 'utf8'));
  writeFileSync(join(root, 'run.json'), JSON.stringify({ ...run, state: 'running', finishedAt: undefined }));
  rmSync(join(root, 'passage-rerank.jev.json'));
  const jev = JSON.parse(readFileSync(join(root, 'proposal-dedup.jev.json'), 'utf8'));
  writeFileSync(join(root, 'proposal-dedup.jev.json'), JSON.stringify({ ...jev, complete: false, results: jev.results.slice(0, 2) }));
  const r = scoreStudy(root, { fixturesDir: DIR });
  assert.equal(r.state, 'running');
  for (const condition of ['proposal-dedup', 'passage-rerank']) {
    const c = r.conditions.find((x) => x.condition === condition);
    assert.equal(c.status, 'incomplete', condition);
    assert.match(c.reason, /did not finish .*--resume/, condition);
  }
  assert.equal(r.conditions.find((c) => c.condition === 'connector-relevance').status, 'compared');
});

test('E3: the score command writes score.json and score.md', (t) => {
  const out = mkdtempSync(join(tmpdir(), 'jev-score-'));
  t.after(() => rmSync(out, { recursive: true, force: true }));
  const log = console.log;
  console.log = () => {};
  try { main(['score', STUDY('malformed'), '--fixtures', DIR, '--out', out]); } finally { console.log = log; }
  assert.equal(JSON.parse(readFileSync(join(out, 'score.json'), 'utf8')).conditions.length, Object.keys(JEV_CONDITIONS).length);
  assert.match(readFileSync(join(out, 'score.md'), 'utf8'), /# Jev study: loopback endpoint/);
  assert.throws(() => main(['score']), /usage/);
  assert.throws(() => main(['score', STUDY('malformed'), '--typesafe']), /unknown flag/);
});

test('E3: the judged fixtures are big enough to measure: 20 to 40 items per use, with the cases the plan names', () => {
  for (const condition of Object.keys(JEV_CONDITIONS)) {
    const f = fixture(condition);
    assert.ok(f.items.length >= 20 && f.items.length <= 40, `${condition}: ${f.items.length} items`);
    assert.ok(f.items.every((i) => i.kind), `${condition}: every item says which case it tests`);
  }
  const conn = fixture('connector-relevance');
  const kinds = (k) => conn.items.filter((i) => i.kind === k);
  assert.ok(kinds('ambiguous').length >= 4, 'genuinely ambiguous requests');
  assert.ok(kinds('explicit').length >= 4 && kinds('explicit').every((i) => i.named.length), 'explicit names');
  const big = conn.items.filter((i) => i.candidates.length > 24);
  assert.ok(big.length >= 3, 'more than 24 candidates');
  assert.ok(big.some((i) => i.candidates.length === 50), 'the library limit');
  assert.ok(big.some((i) => !i.named.length && i.candidates.findIndex((c) => i.relevant.includes(c.id)) >= 24), 'a relevant, unnamed connector past 24');
  assert.ok(fixture('passage-rerank').items.every((i) => i.passages.length <= 24), 'the reranker sees at most 24');
  const dedup = fixture('proposal-dedup');
  assert.ok(dedup.items.filter((i) => i.duplicateOf == null).length >= 8, 'enough novel proposals to measure false omissions');
});

test('a study recorded before whiteboards went (it lists whiteboard-layout) still scores the uses that remain', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'jev-study-old-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  cpSync(STUDY('score'), root, { recursive: true });
  const run = JSON.parse(readFileSync(join(root, 'run.json'), 'utf8'));
  run.conditions.push({ condition: 'whiteboard-layout', arms: { grid: 'whiteboard-layout.grid.json', jev: 'whiteboard-layout.jev.json' }, complete: true, items: 4, fixtureSha256: 'x'.repeat(64) });
  writeFileSync(join(root, 'run.json'), JSON.stringify(run));
  const r = scoreStudy(root, { fixturesDir: DIR });
  const retired = r.conditions.find((c) => c.condition === 'whiteboard-layout');
  assert.equal(retired.status, 'retired');
  assert.match(retired.reason, /whiteboards/);
  for (const c of r.conditions.filter((x) => x.condition !== 'whiteboard-layout')) assert.equal(c.status, 'compared', c.condition);
  assert.match(renderStudy(r), /whiteboard-layout/);
});
