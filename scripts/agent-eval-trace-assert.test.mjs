// NV5: trace assertions (scripts/agent-eval/trace-assert.mjs) over recorded
// streams in scripts/agent-eval/fixtures/trace-assert/. bad-round.jsonl is a
// stream-json capture that breaks every check once; its expected figures are
// worked out beside each assert. good-round.jsonl is clean; quiet-round.jsonl
// uses none of the things the checks look at.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { ALWAYS_LOAD, main as scorecardMain } from './agent-eval/scorecard.mjs';
import { FLAGS, MIN_ELIGIBLE, assertTrace, parseTraceArgs, readTrace, renderFlags, summarizeTraces, traceFiles } from './agent-eval/trace-assert.mjs';

const dir = join(fileURLToPath(new URL('.', import.meta.url)), 'agent-eval', 'fixtures', 'trace-assert');
const fixture = (n) => join(dir, `${n}.jsonl`);
const run = async (n, opts) => assertTrace(await readTrace(fixture(n)), opts);

test('a recorded stream is read in order, one call per tool use id, with each result', async () => {
  const calls = await readTrace(fixture('bad-round'));
  // 11 tool uses; call 4's assistant line is written twice and counts once.
  assert.equal(calls.length, 11);
  assert.deepEqual(calls.map((c) => c.tool), ['Read', 'list_actions', 'run_action', 'Read', 'Read', 'Read', 'Grep', 'Bash', 'Read', 'Bash', 'Bash']);
  assert.deepEqual(calls.map((c) => c.error), [false, false, false, false, false, false, false, true, false, true, true]);
  assert.equal(calls[7].errorClass, 'not-found');
  assert.equal(calls[9].errorClass, 'permission');
  // The tool names the checks match are the app's own always-loaded ones.
  for (const t of ['run_action', 'describe_action']) assert.ok(ALWAYS_LOAD.includes(t), t);
});

test('bad-round: skill never read and run_action with no describe_action are flagged', async () => {
  const r = await run('bad-round');
  const skill = r.checks['skill-never-read'];
  assert.equal(skill.status, 'fail');
  assert.deepEqual(skill.offenders, [{ skill: 'anatomy', reason: 'never' }]);
  const describe = r.checks['run-action-without-describe'];
  assert.equal(describe.status, 'fail');
  assert.deepEqual(describe.offenders, [{ action: 'notes.write_note', at: 2 }]);
});

test('bad-round: errors are recovered only when a later call succeeds within three calls', async () => {
  const e = (await run('bad-round')).checks['errors-recovered'];
  // Errors at 7, 9, 10. Call 8 (Read, ok) recovers 7. Nothing succeeds after 9 or 10.
  assert.equal(e.status, 'fail');
  assert.equal(e.errors, 3);
  assert.equal(e.recovered, 1);
  assert.deepEqual(e.unrecovered.map((u) => [u.at, u.tool, u.class]), [[9, 'Bash', 'permission'], [10, 'Bash', 'permission']]);
});

test('bad-round: the share of wasteful calls counts identical repeats of an unchanged read', async () => {
  const w = (await run('bad-round')).checks['wasteful-calls'];
  // Eligible (not Bash, run_action, a poll): Read x5, list_actions, Grep = 7.
  // Calls 4 and 5 repeat call 3 with nothing that changes state between: 2 of 7.
  assert.equal(w.eligible, 7);
  assert.deepEqual(w.repeats.map((r) => r.at), [4, 5]);
  assert.equal(w.share, 2 / 7);
  assert.equal(w.status, 'fail');
});

test('good-round passes every check; a Skill call, a SKILL.md read and a shell cat all load a skill', async () => {
  const r = await run('good-round', { exempt: ['sessions.wait'] });
  for (const [name, c] of Object.entries(r.checks)) assert.equal(c.status, 'pass', `${name}: ${JSON.stringify(c)}`);
  assert.deepEqual(r.checks['skill-never-read'].skills.sort(), ['anatomy', 'dithered', 'other']);
  // 14 calls; the two sessions.wait runs are polls, so they are not waste candidates.
  assert.equal(r.calls, 14);
  assert.equal(r.checks['wasteful-calls'].wasteful, 0);
  assert.equal(r.checks['errors-recovered'].recovered, 1);
});

test('an action id the agent may know is flagged unless it is exempt', async () => {
  const r = await run('good-round');
  assert.deepEqual(r.checks['run-action-without-describe'].offenders.map((o) => o.action), ['sessions.wait', 'sessions.wait']);
});

test('quiet-round: nothing applies, so every check is not applicable', async () => {
  const r = await run('quiet-round');
  for (const [name, c] of Object.entries(r.checks)) assert.equal(c.status, 'na', name);
  assert.equal(r.checks['wasteful-calls'].eligible, 1);
  assert.ok(1 < MIN_ELIGIBLE);
});

test('a skill the eval row expects but the run never loaded fails; a loaded one passes', async () => {
  assert.equal((await run('quiet-round', { skills: ['anatomy'] })).checks['skill-never-read'].status, 'fail');
  assert.equal((await run('good-round', { skills: ['anatomy'], exempt: ['sessions.wait'] })).checks['skill-never-read'].status, 'pass');
});

test('the summary share leaves not-applicable runs out instead of counting them as zero', async () => {
  const results = await traceFiles(['bad-round', 'good-round', 'quiet-round'].map(fixture), { exempt: ['sessions.wait'] });
  const s = summarizeTraces(results.map((r) => r.result));
  // skill: bad fails, good passes, quiet is n/a: 1 of 2, not 1 of 3.
  assert.deepEqual(s['skill-never-read'], { pass: 1, fail: 1, na: 1, applicable: 2, failShare: 0.5 });
  assert.deepEqual(s['run-action-without-describe'], { pass: 1, fail: 1, na: 1, applicable: 2, failShare: 0.5 });
  assert.deepEqual(s['errors-recovered'], { pass: 1, fail: 1, na: 1, applicable: 2, failShare: 0.5 });
  assert.deepEqual(s['wasteful-calls'], { pass: 1, fail: 1, na: 1, applicable: 2, failShare: 0.5 });
  // Only quiet-round: every check applied to no run, so there is no share at all.
  const none = summarizeTraces([results[2].result]);
  assert.equal(none['skill-never-read'].failShare, null);
  assert.equal(none['skill-never-read'].na, 1);
});

test('an eval round reports "skill never read" and "run_action called with no describe_action first"', async () => {
  const results = await traceFiles([fixture('bad-round')]);
  const out = renderFlags(results);
  assert.match(out, /bad-round\.jsonl: skill never read: anatomy/);
  assert.match(out, /bad-round\.jsonl: run_action called with no describe_action first: notes\.write_note/);
  assert.deepEqual(FLAGS, ['skill-never-read', 'run-action-without-describe']);
  assert.match(out, /skill-never-read: 1 fail, 0 pass, 0 not applicable \(fail share 100% of 1\)/);
  // No prompt or text from the stream reaches the report.
  assert.doesNotMatch(out, /MUST-NOT-LEAK/);
});

test('scorecard.mjs trace-assert prints the flags and writes the JSON, and an empty path fails', async () => {
  const tmp = mkdtempSync(join(tmpdir(), 'trace-assert-'));
  const lines = [];
  const log = console.log;
  console.log = (...a) => lines.push(a.join(' '));
  try {
    const json = join(tmp, 'out.json');
    assert.equal(await scorecardMain(['trace-assert', fixture('bad-round'), '--json', json]), 0);
    assert.match(lines.join('\n'), /skill never read: anatomy/);
    const written = readFileSync(json, 'utf8');
    assert.equal(JSON.parse(written).summary['run-action-without-describe'].fail, 1);
    assert.doesNotMatch(written, /MUST-NOT-LEAK/);
    const err = console.error;
    console.error = () => {};
    try {
      assert.equal(await scorecardMain(['trace-assert', join(tmp, 'missing.jsonl')]), 2);
      assert.equal(await scorecardMain(['trace-assert', '--nope']), 2);
    } finally {
      console.error = err;
    }
  } finally {
    console.log = log;
    rmSync(tmp, { recursive: true, force: true });
  }
  assert.deepEqual(parseTraceArgs(['a', '--skill', 'x', '--exempt', 'y', '--json', 'j']), { files: ['a'], skills: ['x'], exempt: ['y'], json: 'j' });
});

test('SKILL.md read after the skill files were used fails as after-use; a pending result still counts as a read', async () => {
  const tmp = mkdtempSync(join(tmpdir(), 'trace-assert-'));
  try {
    const use = (id, name, input) => JSON.stringify({ type: 'assistant', message: { id, content: [{ type: 'tool_use', id, name, input }] } });
    const ok = (id) => JSON.stringify({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: id, content: 'ok' }] } });
    const late = join(tmp, 'late.jsonl');
    writeFileSync(late, [use('a', 'Read', { file_path: '/s/.agents/skills/x/scripts/run.sh' }), ok('a'), use('b', 'Read', { file_path: '/s/.agents/skills/x/SKILL.md' }), ok('b')].join('\n'));
    const r = assertTrace(await readTrace(late));
    assert.deepEqual(r.checks['skill-never-read'].offenders, [{ skill: 'x', reason: 'after-use' }]);
    // The SKILL.md read has no result line (the stream ended): still a read.
    const pending = join(tmp, 'pending.jsonl');
    writeFileSync(pending, [use('c', 'Read', { file_path: '/s/.agents/skills/y/SKILL.md' }), use('d', 'Read', { file_path: '/s/.agents/skills/y/references/a.md' })].join('\n'));
    assert.equal(assertTrace(await readTrace(pending)).checks['skill-never-read'].status, 'pass');
    // A describe_action that errored taught the agent nothing.
    const bad = join(tmp, 'errdescribe.jsonl');
    const app = 'mcp__hollow_app__';
    writeFileSync(
      bad,
      [
        use('e', `${app}describe_action`, { id: 'a.b' }),
        JSON.stringify({ type: 'user', message: { content: [{ type: 'tool_result', tool_use_id: 'e', is_error: true, content: 'Unknown action' }] } }),
        use('f', `${app}run_action`, { id: 'a.b' }),
        ok('f'),
      ].join('\n'),
    );
    assert.deepEqual(assertTrace(await readTrace(bad)).checks['run-action-without-describe'].offenders, [{ action: 'a.b', at: 1 }]);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});

test('a line that is not JSON is skipped, and only tool lines are parsed', async () => {
  const tmp = mkdtempSync(join(tmpdir(), 'trace-assert-'));
  try {
    const f = join(tmp, 's.jsonl');
    writeFileSync(f, ['not json "tool_use"', JSON.stringify({ type: 'user', message: { content: 'prompt' } }), JSON.stringify({ type: 'assistant', message: { id: 'z', content: [{ type: 'tool_use', id: 'z', name: 'Read', input: { file_path: '/a' } }] } })].join('\n'));
    let parsed = 0;
    const calls = await readTrace(f, { parse: (l) => (parsed++, JSON.parse(l)) });
    assert.equal(calls.length, 1);
    // The bad line is tried, the prompt line has no tool part so it is never parsed.
    assert.equal(parsed, 2);
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
});
