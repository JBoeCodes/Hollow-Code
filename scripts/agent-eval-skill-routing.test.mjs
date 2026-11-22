// BI1: skill routing eval (scripts/agent-eval/skill-routing.mjs) over the
// recorded streams in scripts/agent-eval/fixtures/skill-routing/streams/.
// Every agent call here is a fixture or a fake spawn: no paid model is called.
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readlinkSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { AGENTS, CLAUDE_DISALLOWED, choiceOf, commandFor, endOf, libraryNames, loadCases, main, makeScratch, parseArgs, planRun, renderRouting, resolveSkill, runOne, scoreDir, streamChoice, unknownSkills, validateCases } from './agent-eval/skill-routing.mjs';

const dir = join(fileURLToPath(new URL('.', import.meta.url)), 'agent-eval', 'fixtures', 'skill-routing');
const testCases = join(dir, 'test-cases.json');
const streams = join(dir, 'streams');

test('the shipped case set is valid and has negatives and confusable pairs', () => {
  const cases = loadCases(join(dir, 'cases.json'));
  assert.ok(cases.length >= 20, `${cases.length}`);
  assert.ok(cases.filter((c) => c.expect === null).length >= 3, 'none-expected negatives');
  assert.ok(cases.filter((c) => c.confusable.length).length >= 10, 'confusable near misses');
});

test('a bad case set is refused with every problem named', () => {
  assert.throws(() => validateCases({ version: 2, cases: [] }), /version must be 1[\s\S]*cases is empty/);
  const bad = {
    version: 1,
    cases: [
      { id: 'Bad Id', prompt: 'x', expect: 'a' },
      { id: 'ok', prompt: '', expect: 'a' },
      { id: 'ok', prompt: 'x', expect: 'a' },
      { id: 'self', prompt: 'x', expect: 'a', confusable: ['a'] },
      { id: 'empty', prompt: 'x', expect: '' },
    ],
  };
  assert.throws(() => validateCases(bad), (err) => {
    for (const want of ['id must match', 'prompt is required', 'duplicate id', 'confusable names the expected skill', 'expect is a skill name or null']) assert.ok(err.message.includes(want), `${want}: ${err.message}`);
    return true;
  });
});

test('the first skill a Claude stream reaches for is its choice, by any of the four reads', () => {
  const use = (name, input) => ({ type: 'assistant', message: { content: [{ type: 'tool_use', id: 't', name, input }] } });
  assert.deepEqual(choiceOf('claude', use('Skill', { skill: 'alpha' })), { skill: 'alpha', via: 'Skill' });
  assert.deepEqual(choiceOf('claude', use('Read', { file_path: '/h/.agents/skills/alpha/SKILL.md' })), { skill: 'alpha', via: 'Read' });
  assert.deepEqual(choiceOf('claude', use('mcp__hollow_vault__read_skill', { name: 'beta' })), { skill: 'beta', via: 'read_skill' });
  assert.deepEqual(choiceOf('claude', use('mcp__hollow_app__run_action', { id: 'skills.read_skill', args: { path: '/v/.agents/skills/beta/SKILL.md' } })), { skill: 'beta', via: 'run_action' });
  assert.deepEqual(choiceOf('claude', use('Bash', { command: 'cat ~/.agents/skills/gamma/SKILL.md' })), { skill: 'gamma', via: 'Bash' });
  // Not a choice: another file, a skill's other file, a listing, a result line.
  for (const [name, input] of [
    ['Read', { file_path: '/h/notes/SKILL.md.bak' }],
    ['Read', { file_path: '/h/.agents/skills/alpha/references/x.md' }],
    ['Bash', { command: 'ls ~/.agents/skills/*/SKILL.md' }],
    ['mcp__hollow_app__run_action', { id: 'skills.list_skills' }],
  ]) assert.equal(choiceOf('claude', use(name, input)), null, `${name} ${JSON.stringify(input)}`);
  assert.equal(choiceOf('claude', { type: 'user', message: { content: [] } }), null);
  assert.throws(() => choiceOf('grok', {}), /unknown agent/);
});

test('a Codex stream chooses by a shell read of SKILL.md or an MCP read', () => {
  const item = (type, extra) => ({ type: 'item.started', item: { id: 'i', type, ...extra } });
  assert.deepEqual(choiceOf('codex', item('command_execution', { command: '/bin/zsh -lc "sed -n 1,200p /h/.agents/skills/beta/SKILL.md"' })), { skill: 'beta', via: 'shell' });
  assert.deepEqual(choiceOf('codex', item('mcp_tool_call', { tool: 'read_skill', arguments: '{"name":"alpha"}' })), { skill: 'alpha', via: 'read_skill' });
  assert.deepEqual(choiceOf('codex', item('mcp_tool_call', { tool: 'read_skill', arguments: { name: 'alpha' } })), { skill: 'alpha', via: 'read_skill' });
  assert.equal(choiceOf('codex', item('mcp_tool_call', { tool: 'read_skill', arguments: '{bad' })), null);
  assert.equal(choiceOf('codex', item('command_execution', { command: 'ls /h/.agents/skills' })), null);
  assert.equal(choiceOf('codex', { type: 'turn.completed' }), null);
});

test('recorded streams score each agent apart, naming each confusable pair that failed', async () => {
  const cases = loadCases(testCases);
  assert.equal((await streamChoice('claude', join(streams, 'claude', 'a.jsonl'))).choice.skill, 'alpha', 'first choice wins over a later read');
  assert.deepEqual(await streamChoice('claude', join(streams, 'claude', 'c.jsonl')), { choice: null }, 'finished with no skill');
  assert.deepEqual(await streamChoice('claude', join(streams, 'claude', 'f.jsonl')), { failed: 'the agent ended in an error' });
  assert.deepEqual(await streamChoice('claude', join(streams, 'claude', 'g.jsonl')), { failed: 'the stream has no end' });
  assert.deepEqual(await streamChoice('codex', join(streams, 'codex', 'f.jsonl')), { failed: 'empty stream' });
  assert.deepEqual(await streamChoice('codex', join(streams, 'codex', 'g.jsonl')), { failed: 'the agent ended in an error' });
  const report = await scoreDir(cases, streams);
  const [claude, codex] = report.agents;
  assert.deepEqual(report.agents.map((a) => a.agent), AGENTS);
  // Claude: a right; b wrong (gamma is no near miss); c no skill; d fired its
  // near miss with none expected; e confused (beta expected, its near miss
  // alpha read); f ended in an error and g has no end: failed, left out.
  assert.deepEqual(claude.rows.map((r) => r.outcome), ['correct', 'wrong', 'missed', 'falseFire', 'confused', 'error', 'error']);
  assert.deepEqual({ ran: claude.ran, errors: claude.errors, correct: claude.correct, accuracy: claude.accuracy }, { ran: 5, errors: 2, correct: 1, accuracy: 0.2 });
  assert.deepEqual(claude.confusedPairs, [{ expect: '(none)', chose: 'beta', cases: ['d'] }, { expect: 'beta', chose: 'alpha', cases: ['e'] }]);
  assert.ok(claude.interval.lo < 0.2 && claude.interval.hi > 0.2, JSON.stringify(claude.interval));
  // Codex: a confused (beta for alpha); b, d, e right; c has no stream: not
  // run, never a miss; f empty and g turn.failed: failed, never a miss.
  assert.deepEqual(codex.rows.map((r) => r.outcome), ['confused', 'correct', 'notRun', 'correct', 'correct', 'error', 'error']);
  assert.deepEqual({ ran: codex.ran, notRun: codex.notRun, errors: codex.errors, correct: codex.correct, accuracy: codex.accuracy }, { ran: 4, notRun: 1, errors: 2, correct: 3, accuracy: 0.75 });
  assert.deepEqual(codex.confusedPairs, [{ expect: 'alpha', chose: 'beta', cases: ['a'] }]);
  assert.deepEqual(codex.perSkill, { alpha: { cases: 1, correct: 0 }, beta: { cases: 2, correct: 2 }, '(none)': { cases: 1, correct: 1 } });
  const text = renderRouting(report);
  assert.match(text, /claude: 1\/5 routed right, 20%.*2 failed runs \(left out\)/);
  assert.match(text, /confusable pair failed: \(none\) -> beta \(d\)/);
  assert.match(text, /confusable pair failed: beta -> alpha \(e\)/);
  assert.match(text, /codex: 3\/4 routed right, 75%.*1 not run/);
  // Privacy: no prompt text in the report.
  assert.ok(!JSON.stringify(report).includes('Prompt for'), 'no prompt text');
});

test('a stream end is done or failed the way each CLI reports it', () => {
  assert.equal(endOf('claude', { type: 'result', subtype: 'success', is_error: false }), 'done');
  assert.equal(endOf('claude', { type: 'result', subtype: 'error_max_turns', is_error: false }), 'done', 'no skill within the turns is an answer');
  assert.equal(endOf('claude', { type: 'result', subtype: 'success', is_error: true }), 'failed');
  assert.equal(endOf('claude', { type: 'result', subtype: 'error_during_execution' }), 'failed');
  assert.equal(endOf('claude', { type: 'assistant' }), null);
  assert.equal(endOf('codex', { type: 'turn.completed' }), 'done');
  assert.equal(endOf('codex', { type: 'turn.failed' }), 'failed');
  assert.equal(endOf('codex', { type: 'error', message: 'x' }), 'failed');
  assert.equal(endOf('codex', { type: 'item.completed' }), null);
});

test('a choice is named as the library names it', () => {
  const lib = mkdtempSync(join(tmpdir(), 'sr-lib-'));
  try {
    mkdirSync(join(lib, 'alpha-folder'));
    writeFileSync(join(lib, 'alpha-folder', 'SKILL.md'), '---\nname: "alpha"\ndescription: x\n---\nBody.\n');
    mkdirSync(join(lib, 'beta'));
    writeFileSync(join(lib, 'beta', 'SKILL.md'), 'No frontmatter.\n');
    mkdirSync(join(lib, 'not-a-skill'));
    const names = libraryNames(lib);
    assert.deepEqual([...names], [['alpha-folder', 'alpha'], ['beta', 'beta']]);
    assert.equal(resolveSkill('alpha-folder', names), 'alpha');
    assert.equal(resolveSkill('plugin:beta', names), 'beta');
    assert.equal(resolveSkill(null, names), null);
    assert.deepEqual(unknownSkills([{ expect: 'alpha', confusable: ['gamma'] }, { expect: null, confusable: ['beta'] }], names), ['gamma']);
    assert.equal(libraryNames(join(lib, 'missing')).size, 0);
  } finally {
    rmSync(lib, { recursive: true, force: true });
  }
});

/** A fake child process that prints `lines` and records a kill. */
function fakeSpawn(linesFor, calls, { code = 0, hang = false } = {}) {
  return (bin, args, opts) => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    child.killed = false;
    child.kill = () => {
      child.killed = true;
      if (!child.stdout.writableEnded) child.stdout.end();
      return true;
    };
    calls.push({ bin, args, opts, child });
    setImmediate(() => {
      for (const line of linesFor(bin, args)) if (!child.killed) child.stdout.write(`${line}\n`);
      if (!child.killed && !hang) child.stdout.end();
    });
    child.stdout.on('end', () => setImmediate(() => child.emit('close', child.killed ? null : code)));
    return child;
  };
}

test('a live run is planned per agent and case, edits disallowed, and stops at the choice', async () => {
  const cases = loadCases(testCases);
  const plan = planRun(cases, { out: '/o', limit: 2 });
  assert.deepEqual(plan.map((r) => `${r.agent}/${r.id}`), ['claude/a', 'claude/b', 'codex/a', 'codex/b']);
  const claude = commandFor('claude', 'P');
  assert.deepEqual(claude.args.slice(0, 2), ['-p', 'P']);
  assert.equal(claude.args[claude.args.indexOf('--disallowedTools') + 1], CLAUDE_DISALLOWED.join(' '));
  for (const t of ['Edit', 'Write', 'Bash']) assert.ok(CLAUDE_DISALLOWED.includes(t), t);
  assert.ok(claude.args.includes('--strict-mcp-config'), 'no MCP server');
  assert.equal(claude.args[claude.args.indexOf('--permission-mode') + 1], 'default');
  const codex = commandFor('codex', 'P');
  assert.equal(codex.args[codex.args.indexOf('--sandbox') + 1], 'read-only');
  assert.ok(codex.args.includes('--ignore-user-config'), 'no MCP server or memory from the user config');
  assert.equal(codex.args.at(-1), 'P');

  const out = mkdtempSync(join(tmpdir(), 'sr-out-'));
  try {
    const calls = [];
    const lines = readFileSync(join(streams, 'claude', 'a.jsonl'), 'utf8').trim().split('\n');
    const spawn = fakeSpawn(() => lines, calls);
    const run = { agent: 'claude', id: 'a', stream: join(out, 'claude', 'a.jsonl'), ...commandFor('claude', 'P') };
    const r = await runOne(run, out, { spawn });
    assert.deepEqual(r, { choice: { skill: 'alpha', via: 'Skill' }, error: null });
    assert.ok(calls[0].child.killed, 'stopped once the choice was seen');
    assert.equal(calls[0].opts.cwd, out);
    // The stream holds the lines up to the choice, not the later read.
    const kept = readFileSync(run.stream, 'utf8');
    assert.ok(kept.includes('"Skill"') && !kept.includes('beta/SKILL.md'), kept);

    const failing = (bin, args, opts) => {
      const child = new EventEmitter();
      child.stdout = new PassThrough();
      child.kill = () => true;
      setImmediate(() => child.emit('error', new Error('spawn claude ENOENT')));
      return child;
    };
    assert.deepEqual(await runOne({ ...run, stream: join(out, 'x.jsonl') }, out, { spawn: failing }), { choice: null, error: 'spawn claude ENOENT' });
    assert.equal(readFileSync(join(out, 'x.jsonl.error'), 'utf8'), 'spawn claude ENOENT\n');

    // A run that never chooses is stopped at the timeout (a fake timer that
    // fires at once) and leaves a note; a non-zero exit leaves one too.
    const hung = [];
    const timer = (fn) => (setImmediate(fn), 1);
    const slow = { ...run, stream: join(out, 'slow.jsonl') };
    const r2 = await runOne(slow, out, { spawn: fakeSpawn(() => ['{"type":"system"}'], hung, { hang: true }), timeoutMs: 5, setTimer: timer, clearTimer: () => {} });
    assert.deepEqual(r2, { choice: null, error: 'no choice within 5 ms' });
    assert.ok(hung[0].child.killed);
    const bad = { ...run, stream: join(out, 'claude', 'bad.jsonl') };
    const r3 = await runOne(bad, out, { spawn: fakeSpawn(() => ['{"type":"result","subtype":"success","is_error":false}'], [], { code: 1 }) });
    assert.deepEqual(r3, { choice: null, error: 'exit 1' });
    // The note makes a finished-looking stream count as failed.
    const failed = await scoreDir([{ id: 'bad', prompt: 'p', expect: null, confusable: [] }], out, { agents: ['claude'] });
    assert.deepEqual(failed.agents[0].rows.map((x) => [x.outcome, x.reason]), [['error', 'exit 1']]);
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
});

test('run without --live spawns nothing; with --live it scores what it recorded', async () => {
  const lib = mkdtempSync(join(tmpdir(), 'sr-lib-'));
  const out = mkdtempSync(join(tmpdir(), 'sr-out-'));
  try {
    for (const s of ['alpha', 'beta']) {
      mkdirSync(join(lib, s));
      writeFileSync(join(lib, s, 'SKILL.md'), `---\nname: ${s}\n---\n`);
    }
    const never = () => assert.fail('a dry run must not spawn');
    const logs = [];
    const dry = await main(['run', testCases, '--out', out, '--library', lib], { spawn: never, log: (l) => logs.push(l) });
    assert.equal(dry.plan.length, 14);
    assert.match(logs[0], /dry run: 14 paid agent calls planned/);
    assert.ok(!existsSync(join(out, 'claude')), 'nothing recorded');

    // --live with a fake agent: each case's own fixture stream, or none.
    const calls = [];
    const spawn = fakeSpawn((bin, args) => {
      const agent = bin === 'claude' ? 'claude' : 'codex';
      const prompt = agent === 'claude' ? args[1] : args.at(-1);
      const id = loadCases(testCases).find((c) => c.prompt === prompt).id;
      const file = join(streams, agent, `${id}.jsonl`);
      return existsSync(file) ? readFileSync(file, 'utf8').trim().split('\n') : [];
    }, calls);
    const json = join(out, 'report.json');
    const report = await main(['run', testCases, '--out', out, '--library', lib, '--agent', 'codex', '--live', '--json', json], { spawn, log: () => {} });
    assert.equal(calls.length, 7);
    // The scratch folder links the library as Claude's and Codex's skills, and is gone after.
    const cwd = calls[0].opts.cwd;
    assert.ok(!existsSync(cwd), 'scratch removed');
    assert.deepEqual(report.agents.map((a) => a.agent), ['codex']);
    // Case c has no fixture: its live run printed nothing, a failed run (as
    // f's empty stream and g's turn.failed), never a miss.
    assert.deepEqual(report.agents[0].rows.map((r) => r.outcome), ['confused', 'correct', 'error', 'correct', 'correct', 'error', 'error']);
    assert.deepEqual(JSON.parse(readFileSync(json, 'utf8')).agents[0].correct, 3);

    // A case naming a skill the library lacks is refused before any call.
    const other = mkdtempSync(join(tmpdir(), 'sr-lib-'));
    await assert.rejects(main(['run', testCases, '--out', out, '--library', other, '--live'], { spawn: never, log: () => {} }), /no skill named: alpha, beta/);
    rmSync(other, { recursive: true, force: true });
  } finally {
    rmSync(lib, { recursive: true, force: true });
    rmSync(out, { recursive: true, force: true });
  }
});

test('the scratch folder links each library skill and nothing else', () => {
  const lib = mkdtempSync(join(tmpdir(), 'sr-lib-'));
  const base = mkdtempSync(join(tmpdir(), 'sr-base-'));
  try {
    mkdirSync(join(lib, 'alpha'));
    const scratch = makeScratch(lib, new Map([['alpha', 'alpha']]), { base, shared: '/elsewhere' });
    assert.equal(readlinkSync(join(scratch, '.claude/skills/alpha')), join(lib, 'alpha'));
    assert.equal(readlinkSync(join(scratch, '.agents/skills/alpha')), join(lib, 'alpha'));
    const shared = makeScratch(lib, new Map([['alpha', 'alpha']]), { base, shared: lib });
    assert.ok(!existsSync(join(shared, '.agents')), 'the shared library is read by Codex itself');
  } finally {
    rmSync(lib, { recursive: true, force: true });
    rmSync(base, { recursive: true, force: true });
  }
});

test('arguments are checked', () => {
  assert.deepEqual(parseArgs(['score', 'c.json', 'd']).agents, AGENTS);
  assert.equal(parseArgs(['run', 'c.json', '--out', 'o', '--limit', '3']).limit, 3);
  for (const [argv, re] of [
    [['score', 'c.json'], /streams dir/],
    [['run', 'c.json'], /--out/],
    [['run', 'c.json', '--out', 'o', '--agent', 'grok'], /--agent must be/],
    [['run', 'c.json', '--out', 'o', '--limit', '0'], /--limit/],
    [['run', 'c.json', '--out'], /needs a value/],
    [['list', 'c.json'], /usage/],
    [['score', 'c.json', 'd', '--nope'], /unknown argument/],
  ]) assert.throws(() => parseArgs(argv), re, argv.join(' '));
});
