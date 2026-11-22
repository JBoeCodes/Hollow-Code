// NV6: judge qualification (scripts/agent-eval/judge-qualify.mjs) on the
// pilot suite's judged items. Every judge here is the free rule judge or a
// fake spawn: no paid model is called.
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import { test } from 'node:test';
import { KAPPA_MIN, LABEL_MIN, MIN_HELD_OUT, SELF_MIN, agreement, callJudge, cohenKappa, grade, itemAgreement, judgeCommand, judgePrompt, loadItems, main, parseArgs, parseVerdict, qualify, replyText, scoreVerdicts, splitKey, splitOf, validateItems } from './agent-eval/judge-qualify.mjs';
import { loadSuite } from './agent-eval/score.mjs';

const items = loadItems();

test('the pilot suite gives every reference answer as pass and every failing answer as fail', () => {
  const tasks = loadSuite().tasks;
  const fails = tasks.reduce((s, t) => s + t.judged.fails.length, 0);
  assert.equal(items.length, tasks.length + fails);
  assert.equal(items.filter((i) => i.label === 'pass').length, tasks.length);
  assert.equal(items.filter((i) => i.label === 'fail').length, fails);
  assert.equal(new Set(items.map((i) => i.id)).size, items.length);
  // The split is fixed by the task, so a task's reference and failing
  // answers are never on two sides, and enough items are held out.
  for (const t of tasks) assert.equal(new Set(items.filter((i) => i.task.id === t.id).map((i) => splitOf(splitKey(i)))).size, 1, t.id);
  const heldOut = items.filter((i) => splitOf(splitKey(i)) === 'held-out');
  assert.ok(heldOut.length >= MIN_HELD_OUT, `${heldOut.length} held out`);
  assert.ok(heldOut.some((i) => i.label === 'pass') && heldOut.some((i) => i.label === 'fail'), 'both labels held out');
});

test('the judge sees the task, evidence and answer, never the label or the reference', () => {
  const fail = items.find((i) => i.id === 'simple-outcome/fail-1');
  const prompt = judgePrompt(fail);
  assert.ok(prompt.includes(fail.prompt) && prompt.includes(fail.answer), prompt);
  assert.ok(prompt.includes(fail.evidence[0].quote), 'evidence quoted');
  assert.ok(!/\blabel\b|\bexpected\b/i.test(prompt), prompt);
  assert.ok(!prompt.includes(fail.because), 'not the reason it fails');
  assert.match(prompt, /VERDICT: PASS or VERDICT: FAIL/);
});

test('a verdict is the reply\'s last VERDICT line', () => {
  assert.equal(parseVerdict('Looks right.\nVERDICT: PASS'), 'pass');
  assert.equal(parseVerdict('verdict: fail'), 'fail');
  assert.equal(parseVerdict('**VERDICT: FAIL**'), 'fail');
  assert.equal(parseVerdict('VERDICT: PASS or VERDICT: FAIL?\nVERDICT: FAIL'), 'fail');
  for (const t of ['', 'PASS', 'VERDICT: maybe', null]) assert.equal(parseVerdict(t), null, String(t));
});

test('agreement: unanimity, majority, pairs and kappa', () => {
  assert.deepEqual(itemAgreement(['pass', 'pass', 'pass']), { unanimous: true, majority: 'pass', pairs: 3, agreeing: 3 });
  assert.deepEqual(itemAgreement(['pass', 'fail', 'pass']), { unanimous: false, majority: 'pass', pairs: 3, agreeing: 1 });
  assert.deepEqual(itemAgreement(['pass', null, 'fail']), { unanimous: false, majority: 'undecided', pairs: 3, agreeing: 0 });
  assert.deepEqual(itemAgreement([null, null, null]), { unanimous: false, majority: 'undecided', pairs: 3, agreeing: 0 });
  assert.equal(cohenKappa(['pass', 'fail', 'pass', 'fail'], ['pass', 'fail', 'pass', 'fail']), 1);
  assert.equal(cohenKappa(['pass', 'pass', 'fail', 'fail'], ['pass', 'fail', 'pass', 'fail']), 0);
  assert.equal(cohenKappa([], []), null);
  assert.equal(cohenKappa(['pass', 'pass'], ['pass', 'pass']), 1);
});

/** Records for `ids`, three repeats each, from `verdictOf(id, repeat)`. */
const records = (ids, verdictOf) => ids.flatMap((id) => [0, 1, 2].map((repeat) => ({ item: id, repeat, verdict: verdictOf(id, repeat) })));

test('a judge that flips on 2 of 20 repeated items is flagged before its scores are used', () => {
  const labelled = Array.from({ length: 20 }, (_, i) => ({ id: `x/${i}`, label: i % 2 ? 'fail' : 'pass', evidence: [], prompt: 'p', answer: 'a' }));
  // Every item is held out here (agreement over all rows), so qualify sees 20.
  const rows = records(labelled.map((i) => i.id), (id, r) => {
    const label = labelled.find((i) => i.id === id).label;
    const flip = (id === 'x/3' || id === 'x/8') && r === 1;
    return flip ? (label === 'pass' ? 'fail' : 'pass') : label;
  });
  const byId = new Map();
  for (const r of rows) (byId.get(r.item) ?? byId.set(r.item, []).get(r.item))[r.repeat] = r.verdict;
  const graded = labelled.map((i) => ({ id: i.id, label: i.label, ...itemAgreement(byId.get(i.id)) }));
  const a = agreement(graded);
  assert.equal(a.self.share, 0.9);
  assert.deepEqual(a.self.flipped, ['x/3', 'x/8']);
  assert.equal(a.label.share, 1, 'the majority still matches every label');
  const q = qualify(a);
  assert.equal(q.status, 'not qualified');
  assert.match(q.reasons[0], new RegExp(`self-agreement 0.9 under ${SELF_MIN} \\(2 items flipped\\)`));
  // A label miss on top is named too; a small clean set is only provisional.
  assert.match(qualify({ ...a, label: { ...a.label, share: 0.8 } }).reasons.join('; '), new RegExp(`label agreement 0.8 under ${LABEL_MIN}`));
  // A judge that always says fail is not qualified however many fail items
  // there are: the pass label and kappa refuse it.
  const lopsided = [...Array.from({ length: 4 }, (_, i) => ({ id: `p/${i}`, label: 'pass' })), ...Array.from({ length: 40 }, (_, i) => ({ id: `f/${i}`, label: 'fail' }))];
  const always = agreement(lopsided.map((i) => ({ ...i, ...itemAgreement(['fail', 'fail', 'fail']) })));
  assert.ok(always.label.share > LABEL_MIN, `${always.label.share}`);
  const refused = qualify(always);
  assert.equal(refused.status, 'not qualified');
  assert.match(refused.reasons.join('; '), new RegExp(`pass agreement 0 under ${LABEL_MIN}; kappa 0 under ${KAPPA_MIN}`));
  assert.match(qualify(agreement(lopsided.slice(4).map((i) => ({ ...i, ...itemAgreement(['fail', 'fail', 'fail']) })))).reasons.join('; '), /no held-out item labelled pass/);
  // An undecided item is a miss in kappa too.
  const undecided = agreement([{ id: 'a', label: 'pass', ...itemAgreement([null, null, null]) }, { id: 'b', label: 'fail', ...itemAgreement(['fail', 'fail', 'fail']) }]);
  assert.equal(undecided.label.kappa, 0);
  const small = agreement(graded.slice(0, 5).map((g) => ({ ...g, ...itemAgreement([g.label, g.label, g.label]) })));
  assert.equal(qualify(small).status, 'provisional');
  assert.equal(qualify(agreement([])).status, 'not qualified');
});

test('the rule judge qualifies on the existing judged fixtures (the free baseline)', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'jq-'));
  try {
    const json = join(dir, 'report.json');
    const logs = [];
    const report = await main(['run', '--judge', 'rules', '--out', dir, '--json', json], { log: (l) => logs.push(l) });
    assert.equal(report.graded, items.length);
    assert.equal(report.all.self.share, 1);
    assert.equal(report.all.label.share, 1);
    assert.equal(report.all.label.kappa, 1);
    assert.equal(report.heldOut.items + report.dev.items, items.length);
    assert.equal(report.qualification.status, 'qualified');
    assert.match(logs[0], /Qualification \(held-out\): qualified/);
    // score re-reads the verdicts it wrote and gives the same report.
    const again = await main(['score', join(dir, 'verdicts.jsonl')], { log: () => {} });
    assert.deepEqual({ ...again, judge: 'rules' }, JSON.parse(readFileSync(json, 'utf8')));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('incomplete and unknown verdicts are reported, never scored', () => {
  const some = items.slice(0, 3);
  const recs = [...records([some[0].id], () => 'pass'), { item: some[0].id, repeat: 1, verdict: 'fail' }, { item: some[1].id, repeat: 0, verdict: 'pass' }, { item: some[1].id, repeat: 7, verdict: 'pass' }, { item: 'nope/1', repeat: 0, verdict: 'pass' }];
  const r = scoreVerdicts(some, recs, { judge: 'x' });
  assert.equal(r.graded, 1);
  assert.deepEqual(r.incomplete, [some[1].id, some[2].id]);
  assert.deepEqual(r.unknown, ['nope/1', `${some[1].id}#7`]);
  assert.deepEqual(r.duplicate, [`${some[0].id}#1`]);
  // A trimmed or doctored verdict file never qualifies.
  assert.equal(r.qualification.status, 'not qualified');
  assert.match(r.qualification.reasons.join('; '), /2 items without all 3 verdicts; 2 verdicts for unknown items or repeats; 1 duplicate verdicts/);
});

/** A fake judge process printing `stdout` and closing with `code`. */
function fakeSpawn(stdout, calls, { code = 0, hang = false } = {}) {
  return (bin, args, opts) => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    child.killed = false;
    child.kill = () => {
      child.killed = true;
      setImmediate(() => child.emit('close', null));
      return true;
    };
    calls.push({ bin, args, opts, child });
    setImmediate(() => {
      child.stdout.write(typeof stdout === 'function' ? stdout(args) : stdout);
      if (!hang) setImmediate(() => child.emit('close', code));
    });
    return child;
  };
}

test('a model judge runs with no tools and no MCP server, and only with --live', async () => {
  const claude = judgeCommand('claude', 'P');
  assert.equal(claude.args[claude.args.indexOf('--tools') + 1], '');
  assert.ok(claude.args.includes('--strict-mcp-config'));
  const codex = judgeCommand('codex', 'P');
  assert.equal(codex.args[codex.args.indexOf('--sandbox') + 1], 'read-only');
  assert.ok(codex.args.includes('--ignore-user-config'));
  assert.throws(() => judgeCommand('rules', 'P'), /not a model judge/);

  const never = () => assert.fail('no call without --live');
  await assert.rejects(grade(items, { judge: 'claude', spawn: never }), /makes 78 paid calls; add --live/);
  await assert.rejects(main(['run', '--judge', 'codex', '--out', '/tmp/never'], { spawn: never, log: () => {} }), /add --live/);
});

test('model judge replies are read from each CLI\'s output', async () => {
  assert.equal(replyText('claude', JSON.stringify({ type: 'result', result: 'VERDICT: PASS', is_error: false })), 'VERDICT: PASS');
  assert.equal(replyText('claude', JSON.stringify({ type: 'result', result: 'x', is_error: true })), null);
  assert.equal(replyText('claude', 'not json'), null);
  const events = ['{"type":"thread.started"}', '{"type":"item.completed","item":{"type":"agent_message","text":"VERDICT: FAIL"}}', '{"type":"turn.completed"}'].join('\n');
  assert.equal(replyText('codex', events), 'VERDICT: FAIL');

  const item = items[0];
  const calls = [];
  assert.deepEqual(await callJudge('claude', item, { spawn: fakeSpawn(JSON.stringify({ result: 'Fine.\nVERDICT: PASS' }), calls) }), { verdict: 'pass', error: null });
  assert.equal(calls[0].args[1], judgePrompt(item));
  // Each call runs in an empty folder of its own, removed after.
  assert.ok(calls[0].opts.cwd.startsWith(tmpdir()) && !existsSync(calls[0].opts.cwd), calls[0].opts.cwd);
  assert.deepEqual(await callJudge('codex', item, { spawn: fakeSpawn(events, []) }), { verdict: 'fail', error: null });
  assert.deepEqual(await callJudge('claude', item, { spawn: fakeSpawn('{"result":"unsure"}', []) }), { verdict: null, error: 'no verdict in the reply' });
  assert.deepEqual(await callJudge('claude', item, { spawn: fakeSpawn('', [], { code: 2 }) }), { verdict: null, error: 'exit 2' });
  const hung = [];
  const r = await callJudge('claude', item, { spawn: fakeSpawn('', hung, { hang: true }), timeoutMs: 5, setTimer: (fn) => (setImmediate(fn), 1), clearTimer: () => {} });
  assert.deepEqual(r, { verdict: null, error: 'no answer within 5 ms' });
  assert.ok(hung[0].child.killed);
});

test('a live model judge grades every item three times and is scored like any other', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'jq-'));
  try {
    // A fake judge that answers each item's label, except one flip.
    let n = 0;
    const spawn = fakeSpawn((args) => {
      const prompt = args[1];
      const it = items.find((i) => prompt === judgePrompt(i));
      n += 1;
      const verdict = n === 2 ? (it.label === 'pass' ? 'FAIL' : 'PASS') : it.label.toUpperCase();
      return JSON.stringify({ result: `VERDICT: ${verdict}` });
    }, []);
    const report = await main(['run', '--judge', 'claude', '--out', dir, '--live'], { spawn, log: () => {} });
    assert.equal(n, items.length * 3);
    assert.equal(report.graded, items.length);
    assert.deepEqual(report.all.self.flipped, [items[0].id]);
    assert.equal(report.all.label.share, 1);
    assert.equal(readFileSync(join(dir, 'verdicts.jsonl'), 'utf8').trim().split('\n').length, items.length * 3);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('hand-labelled items join the pilot items', () => {
  const dir = mkdtempSync(join(tmpdir(), 'jq-'));
  try {
    const file = join(dir, 'items.json');
    writeFileSync(file, JSON.stringify({ schema: 1, items: [{ id: 'trace/1', prompt: 'Do x.', answer: 'Did x.', label: 'pass', evidence: [{ path: 'a.md', quote: 'x' }] }] }));
    const all = loadItems([file]);
    assert.equal(all.length, items.length + 1);
    assert.equal(all.at(-1).task, null);
    writeFileSync(file, JSON.stringify({ schema: 1, items: [{ id: 'simple-outcome/reference', prompt: 'p', answer: 'a', label: 'pass' }] }));
    assert.throws(() => loadItems([file]), /duplicate id simple-outcome\/reference/);
    assert.throws(() => validateItems({ schema: 2, items: [{ id: 'Bad', prompt: '', label: 'maybe' }] }), (e) => ['schema must be 1', 'id must match', 'prompt is required', 'answer is required', 'label must be pass or fail'].every((w) => e.message.includes(w)));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('arguments are checked', () => {
  assert.equal(parseArgs(['run', '--judge', 'rules', '--out', 'o']).repeats, 3);
  assert.equal(parseArgs(['score', 'v.jsonl', '--items', 'a.json']).items[0], 'a.json');
  for (const [argv, re] of [
    [['run', '--out', 'o'], /--judge/],
    [['run', '--judge', 'gpt', '--out', 'o'], /--judge/],
    [['run', '--judge', 'rules'], /--out/],
    [['run', '--judge', 'rules', '--out', 'o', '--repeats', '1'], /at least 2/],
    [['score'], /verdicts/],
    [['list'], /usage/],
    [['run', '--judge', 'rules', '--out', 'o', '--nope'], /unknown argument/],
  ]) assert.throws(() => parseArgs(argv), re, argv.join(' '));
});
