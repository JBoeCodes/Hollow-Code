// NV4: with and without skill lift (scripts/agent-eval/skill-lift.mjs) on the
// humanizer suite, over answers the tests write from
// fixtures/skill-lift/humanizer-answers.json. No paid model is called: the
// live path runs against a fake spawn.
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { ARMS, MIN_CASES, armCommand, armScratch, gradeAnswer, liftReport, loadSuite, main, pairedLift, parseArgs, planRun, renderLift, runOne, validateSuite } from './agent-eval/skill-lift.mjs';

const dir = join(fileURLToPath(new URL('.', import.meta.url)), 'agent-eval', 'fixtures', 'skill-lift');
const suiteFile = join(dir, 'humanizer.json');
const suite = loadSuite(suiteFile);
const good = JSON.parse(readFileSync(join(dir, 'humanizer-answers.json'), 'utf8')).answers;
/** A failing answer: the case's own input (full of tells), or a wrong reply. */
const bad = (c) => (c.negative ? 'I am not sure.' : c.prompt.split('\n\n').slice(1).join('\n\n'));

/** Write arms: `passes(arm, case) → [bool per attempt]`. */
function writeArms(root, passes, cases = suite.cases) {
  for (const arm of ARMS) {
    mkdirSync(join(root, arm), { recursive: true });
    for (const c of cases) passes(arm, c).forEach((ok, i) => writeFileSync(join(root, arm, `${c.id}.${i + 1}.txt`), ok ? good[c.id] : bad(c)));
  }
}

test('the suite is valid, has negatives, and its one grader tells good from bad answers', () => {
  assert.equal(suite.skill, 'humanizer');
  assert.ok(suite.cases.length >= MIN_CASES, `${suite.cases.length}`);
  assert.equal(suite.cases.filter((c) => c.negative).length, 3);
  for (const c of suite.cases) {
    assert.ok(gradeAnswer(c, good[c.id]).pass, `${c.id} good: ${gradeAnswer(c, good[c.id]).failed}`);
    assert.ok(!gradeAnswer(c, bad(c)).pass, `${c.id} bad passes`);
  }
  assert.ok(!gradeAnswer(suite.cases[0], '   ').pass, 'an empty answer never passes');
  assert.throws(() => validateSuite({ schema: 1, skill: 'Bad Skill', cases: [{ id: 'a', prompt: '', negative: 'yes', rules: [] }, { id: 'a', prompt: 'p', rules: [{ id: 'r', kind: 'nope' }] }] }), (e) =>
    ['skill must be', 'prompt is required', 'negative is true or false', 'score.rules must list at least one rule', 'duplicate id', 'kind must be one of'].every((w) => e.message.includes(w)),
  );
});

test('lift is with minus without, paired over cases, with a bootstrap interval', () => {
  const root = mkdtempSync(join(tmpdir(), 'lift-'));
  try {
    // with: every attempt passes, except bio's second. without: a rewrite
    // passes only on its third attempt; negatives pass, except neg-json's third.
    writeArms(root, (arm, c) => {
      if (arm === 'with') return c.id === 'bio' ? [true, false, true] : [true, true, true];
      if (c.negative) return c.id === 'neg-json' ? [true, true, false] : [true, true, true];
      return [false, false, true];
    });
    const r = liftReport(suite, root);
    assert.equal(r.paired, 12);
    assert.equal(r.provisional, false);
    assert.deepEqual(r.arms.without.offered, { yes: 0, no: 0, unknown: 36 });
    // Per case diffs: 8 rewrites 1 - 1/3, bio 2/3 - 1/3, neg-json 1 - 2/3,
    // two negatives 0: mean (16/3 + 1/3 + 1/3) / 12 = 0.5.
    assert.equal(r.lift.lift, 0.5);
    // No resample can beat the largest per-case gap (2/3).
    assert.ok(r.lift.low > 0 && r.lift.low < 0.5 && r.lift.high > 0.5 && r.lift.high <= 2 / 3 + 1e-9, JSON.stringify(r.lift));
    assert.equal(r.liftPositive.lift, Math.round((17 / 27) * 1e4) / 1e4);
    assert.equal(r.liftNegative.lift, Math.round((1 / 9) * 1e4) / 1e4);
    assert.equal(r.arms.with.passRate, Math.round((35 / 36) * 1e4) / 1e4);
    assert.equal(r.arms.with.attempts, 36);
    // pass@3: every case solved by both; pass@1 tells them apart.
    assert.deepEqual([r.arms.with.passAtK.k, r.arms.with.passAtK.solved, r.arms.without.passAtK.solved], [3, 12, 12]);
    const k1 = liftReport(suite, root, { k: 1 });
    assert.deepEqual([k1.arms.with.passAtK.solved, k1.arms.without.passAtK.solved], [12, 3]);
    assert.ok(k1.arms.without.passAtK.interval.lo < 0.25 && k1.arms.without.passAtK.interval.hi > 0.25);
    // The interval is seeded: the same every run.
    assert.deepEqual(liftReport(suite, root).lift, r.lift);
    const text = renderLift(r);
    assert.match(text, /Skill lift: humanizer, 12\/12 cases answered by both arms\n/);
    assert.match(text, /Lift: \+50 pts \(95% /);
    assert.ok(!text.includes('PROVISIONAL'));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('an agent error is a fail; a run that never got going is left out and makes the report provisional', () => {
  const root = mkdtempSync(join(tmpdir(), 'lift-'));
  try {
    writeArms(root, () => [true, true, true]);
    // An agent error (turn limit) on a passing answer's slot: a fail.
    writeFileSync(join(root, 'with', 'event.2.txt'), '');
    writeFileSync(join(root, 'with', 'event.2.txt.fail'), 'the agent ended in error_max_turns\n');
    const r = liftReport(suite, root);
    assert.equal(r.arms.with.agentErrors, 1);
    assert.equal(r.perCase.find((c) => c.id === 'event').with, Math.round((2 / 3) * 1e4) / 1e4);
    assert.equal(r.provisional, false);
    // A spawn failure is left out, and named.
    writeFileSync(join(root, 'without', 'event.3.txt'), '');
    writeFileSync(join(root, 'without', 'event.3.txt.error'), 'spawn claude ENOENT\n');
    const left = liftReport(suite, root);
    assert.equal(left.arms.without.attempts, 35);
    assert.deepEqual(left.provisionalReasons, ['1 runs failed and were left out (with 0, without 1)']);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('an arm offered the wrong skills makes the report provisional', () => {
  const root = mkdtempSync(join(tmpdir(), 'lift-'));
  try {
    writeArms(root, () => [true]);
    for (const c of suite.cases) {
      writeFileSync(join(root, 'with', `${c.id}.1.txt.skills.json`), JSON.stringify(['humanizer']));
      writeFileSync(join(root, 'without', `${c.id}.1.txt.skills.json`), JSON.stringify([]));
    }
    const clean = liftReport(suite, root);
    assert.deepEqual([clean.arms.with.offered, clean.arms.without.offered], [{ yes: 12, no: 0, unknown: 0 }, { yes: 0, no: 12, unknown: 0 }]);
    assert.equal(clean.provisional, false);
    writeFileSync(join(root, 'without', 'bio.1.txt.skills.json'), JSON.stringify(['user:humanizer']));
    writeFileSync(join(root, 'with', 'bio.1.txt.skills.json'), JSON.stringify(['pdf']));
    assert.deepEqual(liftReport(suite, root).provisionalReasons, ['the without arm was offered humanizer in 1 attempts', 'the with arm lacked humanizer in 1 attempts']);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('under ten paired cases, or with no negative case, the report is provisional', () => {
  const root = mkdtempSync(join(tmpdir(), 'lift-'));
  try {
    const all = () => [true, false];
    writeArms(root, all, suite.cases.slice(0, 7).concat(suite.cases.filter((c) => c.negative).slice(0, 1)));
    const small = liftReport(suite, root);
    assert.equal(small.paired, 8);
    assert.equal(small.provisional, true);
    assert.deepEqual(small.provisionalReasons, [`8 paired cases (under ${MIN_CASES})`]);
    assert.equal(small.unpaired.length, 4);
    assert.match(renderLift(small), /\[PROVISIONAL: 8 paired cases \(under 10\)\]/);
    rmSync(root, { recursive: true, force: true });
    writeArms(root, all, suite.cases.filter((c) => !c.negative));
    const positiveOnly = liftReport(suite, root);
    assert.deepEqual(positiveOnly.provisionalReasons, [`9 paired cases (under ${MIN_CASES})`, 'no negative case answered by both arms']);
    assert.equal(positiveOnly.liftNegative, null);
    // A case only one arm answered is not paired.
    rmSync(join(root, 'without'), { recursive: true, force: true });
    assert.equal(liftReport(suite, root).paired, 0);
    assert.equal(liftReport(suite, root).lift, null);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('the paired bootstrap of identical arms is zero, and of a constant gap is that gap', () => {
  const att = (rates) => new Map(rates.map((r, i) => [`c${i}`, [{ pass: r }]]));
  const ids = ['c0', 'c1', 'c2', 'c3'];
  assert.deepEqual(pairedLift(att([true, false, true, false]), att([true, false, true, false]), ids), { cases: 4, lift: 0, low: 0, high: 0, resamples: 2000 });
  const gap = pairedLift(att([true, true, true, true]), att([false, false, false, false]), ids);
  assert.deepEqual([gap.lift, gap.low, gap.high], [1, 1, 1]);
  assert.equal(pairedLift(att([]), att([]), []), null);
});

/** A fake Claude printing `stdout` (a function of args) and closing. */
function fakeSpawn(stdout, calls, { code = 0, hang = false } = {}) {
  return (bin, args, opts) => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    child.kill = () => true;
    calls.push({ bin, args, opts });
    setImmediate(() => {
      child.stdout.write(typeof stdout === 'function' ? stdout(args) : stdout);
      if (!hang) setImmediate(() => child.emit('close', code));
    });
    return child;
  };
}

test('both arms run the same command; only the scratch folder differs', () => {
  const cmd = armCommand('P');
  assert.equal(cmd.args[1], 'P');
  assert.equal(cmd.args[cmd.args.indexOf('--setting-sources') + 1], 'project', 'no user or plugin skill in either arm');
  assert.ok(cmd.args.includes('--strict-mcp-config'));
  assert.equal(cmd.args.at(-1), 'Edit Write MultiEdit NotebookEdit Bash');
  assert.ok(!cmd.args.at(-1).includes('Skill'), 'both arms keep the Skill tool');
  const plan = planRun(suite, { out: '/o', repeats: 2 });
  assert.equal(plan.length, 2 * suite.cases.length * 2);
  // The arms take turns, case by case and attempt by attempt.
  assert.deepEqual(plan.slice(0, 4).map((r) => `${r.arm} ${r.id}.${r.attempt}`), ['with newsletter.1', 'without newsletter.1', 'with newsletter.2', 'without newsletter.2']);
  assert.ok(plan.every((r) => r.args.join('\0') === armCommand(suite.cases.find((c) => c.id === r.id).prompt).args.join('\0')));

  const lib = mkdtempSync(join(tmpdir(), 'lift-lib-'));
  try {
    const withDir = armScratch('with', 'humanizer', lib, { base: lib });
    const withoutDir = armScratch('without', 'humanizer', lib, { base: lib });
    assert.equal(readlinkSync(join(withDir, '.claude/skills/humanizer')), join(lib, 'humanizer'));
    assert.ok(!existsSync(join(withoutDir, '.claude')));
  } finally {
    rmSync(lib, { recursive: true, force: true });
  }
});

/** A stream-json reply: init (with `skills`) then a result. */
const stream = (result, { skills = ['humanizer'], subtype = 'success', isError = false } = {}) =>
  [JSON.stringify({ type: 'system', subtype: 'init', skills }), ...(result === undefined ? [] : [JSON.stringify({ type: 'result', subtype, is_error: isError, result })])].join('\n') + '\n';

test('a call writes the answer and its skills, or a fail or error note', async () => {
  const root = mkdtempSync(join(tmpdir(), 'lift-'));
  try {
    const run = { arm: 'with', id: 'newsletter', attempt: 1, answer: join(root, 'with', 'newsletter.1.txt'), ...armCommand('P') };
    assert.deepEqual(await runOne(run, root, { spawn: fakeSpawn(stream('Hello.'), []) }), { ok: true, kind: null, reason: null });
    assert.equal(readFileSync(run.answer, 'utf8'), 'Hello.');
    assert.deepEqual(JSON.parse(readFileSync(`${run.answer}.skills.json`, 'utf8')), ['humanizer']);
    // Older CLIs list skills as slash commands.
    await runOne(run, root, { spawn: fakeSpawn(JSON.stringify({ type: 'system', subtype: 'init', slash_commands: ['humanizer', 'review'] }) + '\n' + JSON.stringify({ type: 'result', subtype: 'success', result: 'x' }), []) });
    assert.deepEqual(JSON.parse(readFileSync(`${run.answer}.skills.json`, 'utf8')), ['humanizer', 'review']);
    for (const [stdout, opts, kind, reason] of [
      [stream('', { subtype: 'error_max_turns', isError: true }), {}, 'fail', 'the agent ended in error_max_turns'],
      [stream('x', { subtype: 'success', isError: true }), {}, 'fail', 'the agent ended in success'],
      [stream(undefined), { code: 1 }, 'error', 'exit 1'],
      ['oops', {}, 'error', 'no result in the stream'],
    ]) {
      assert.deepEqual(await runOne(run, root, { spawn: fakeSpawn(stdout, [], opts) }), { ok: false, kind, reason });
      assert.equal(readFileSync(`${run.answer}.${kind}`, 'utf8'), `${reason}\n`);
      assert.equal(readFileSync(run.answer, 'utf8'), '');
      assert.ok(!existsSync(`${run.answer}.${kind === 'fail' ? 'error' : 'fail'}`), 'notes from an earlier attempt are cleared');
    }
    const r = await runOne(run, root, { spawn: fakeSpawn('', [], { hang: true }), timeoutMs: 5, setTimer: (fn) => (setImmediate(fn), 1), clearTimer: () => {} });
    assert.deepEqual(r, { ok: false, kind: 'fail', reason: 'no answer within 5 ms' });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test('run without --live calls nothing; with --live it records both arms and reports the lift', async () => {
  const lib = mkdtempSync(join(tmpdir(), 'lift-lib-'));
  const out = mkdtempSync(join(tmpdir(), 'lift-out-'));
  try {
    mkdirSync(join(lib, 'humanizer'));
    writeFileSync(join(lib, 'humanizer', 'SKILL.md'), '---\nname: humanizer\n---\n');
    const never = () => assert.fail('a dry run must not spawn');
    const logs = [];
    const dry = await main(['run', suiteFile, '--out', out, '--library', lib], { spawn: never, log: (l) => logs.push(l) });
    assert.equal(dry.plan.length, 72);
    assert.match(logs[0], /dry run: 72 paid Claude calls planned/);
    await assert.rejects(main(['run', suiteFile, '--out', out, '--library', join(lib, 'none'), '--live'], { spawn: never, log: () => {} }), /has no skill humanizer/);

    // A stale answer from an earlier run is cleared, never graded.
    mkdirSync(join(out, 'with'), { recursive: true });
    writeFileSync(join(out, 'with', 'bio.9.txt'), 'stale');
    // A fake Claude: in the with arm's folder (the skill linked) it answers
    // well; otherwise it echoes a rewrite's input (its tells) and answers
    // negatives well.
    const calls = [];
    const spawn = (bin, args, opts) => {
      const withArm = existsSync(join(opts.cwd, '.claude', 'skills', 'humanizer'));
      const c = suite.cases.find((x) => x.prompt === args[1]);
      return fakeSpawn(stream(withArm || c.negative ? good[c.id] : bad(c), { skills: withArm ? ['humanizer'] : [] }), calls)(bin, args, opts);
    };
    const report = await main(['run', suiteFile, '--out', out, '--library', lib, '--repeats', '1', '--live'], { spawn, log: () => {} });
    assert.equal(calls.length, 24);
    assert.ok(calls.every((c) => !existsSync(c.opts.cwd)), 'scratch folders removed');
    assert.equal(report.paired, 12);
    assert.equal(report.lift.lift, 0.75);
    assert.ok(!existsSync(join(out, 'with', 'bio.9.txt')), 'stale answer cleared');
    assert.equal(report.provisional, false);
    assert.deepEqual([report.arms.with.offered.yes, report.arms.without.offered.no], [12, 12]);
    assert.equal(report.liftPositive.lift, 1);
    assert.equal(report.liftNegative.lift, 0);
    // score reads the same answers back.
    assert.deepEqual(await main(['score', suiteFile, out], { log: () => {} }), report);
  } finally {
    rmSync(lib, { recursive: true, force: true });
    rmSync(out, { recursive: true, force: true });
  }
});

test('arguments are checked', () => {
  assert.equal(parseArgs(['score', 's.json', 'd', '--k', '2']).k, 2);
  assert.equal(parseArgs(['run', 's.json', '--out', 'o']).repeats, 3);
  for (const [argv, re] of [
    [['score', 's.json'], /answers dir/],
    [['run', 's.json'], /--out/],
    [['run', 's.json', '--out', 'o', '--repeats', '0'], /at least 1/],
    [['score', 's.json', 'd', '--k'], /needs a value/],
    [['list', 's.json'], /usage/],
    [['score', 's.json', 'd', '--nope'], /unknown argument/],
  ]) assert.throws(() => parseArgs(argv), re, argv.join(' '));
});
