// Overnight 2026-10-17 OB15: the golden envelope's per-layer deltas
// (scripts/agent-eval/envelope-diff.mjs). Two tiny fixtures in the shape
// chat_g2_budget_tests::envelope_fixture_is_current writes, one with a
// guidance string 100 bytes longer: +100 B on exactly that layer, 0
// elsewhere.
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, test } from 'node:test';
import { diffEnvelopes, main, renderMarkdown, summarize } from './agent-eval/envelope-diff.mjs';

const dir = mkdtempSync(join(tmpdir(), 'envelope-diff-'));
after(() => rmSync(dir, { recursive: true, force: true }));

const LAYERS = { visualPrompt: 2_100, capabilityIndex: 3_000, policy: 5_844, workspace: 7_731, request: 120 };
const scenario = (name, agent, policy = LAYERS.policy) => {
  const layers = Object.entries({ ...LAYERS, policy }).map(([n, bytes]) => ({ name: n, bytes }));
  return { scenario: name, agent, entry: 'composer', session: 'resume', total: layers.reduce((s, l) => s + l.bytes, 0), layers };
};
const base = { scenarios: [scenario('checkInWake', 'claude'), scenario('packetChild', 'codex')] };
const head = { scenarios: [scenario('checkInWake', 'claude', LAYERS.policy + 100), scenario('packetChild', 'codex', LAYERS.policy + 100)] };

async function run(argv, opts = {}) {
  const out = [];
  const err = [];
  const log = console.log;
  const error = console.error;
  console.log = (s) => out.push(String(s));
  console.error = (s) => err.push(String(s));
  try {
    const code = await main(argv, { repo: dir, ...opts });
    return { code, out: out.join('\n'), err: err.join('\n') };
  } finally {
    console.log = log;
    console.error = error;
  }
}

test('a 100-byte guidance change is +100 B on exactly that layer and 0 elsewhere', () => {
  const diff = diffEnvelopes(base, head);
  assert.equal(diff.rows.length, 10);
  for (const r of diff.rows) assert.equal(r.delta, r.layer === 'policy' ? 100 : 0, `${r.scenario} ${r.layer}`);
  assert.deepEqual(diff.totals.map((t) => t.delta), [100, 100]);
  const md = renderMarkdown(diff);
  assert.match(md, /^\| checkInWake · claude \| policy \| 5,844 \| 5,944 \| \+100 \|$/m);
  assert.match(md, /^\| packetChild · codex \| policy \| 5,844 \| 5,944 \| \+100 \|$/m);
  assert.doesNotMatch(md, /workspace/, 'unchanged layers stay out of the table');
  assert.match(md, /2 of 2 scenarios changed\./);
  assert.match(renderMarkdown(diff, { all: true }), /\| workspace \| 7,731 \| 7,731 \| 0 \|/);
  const s = summarize(diff);
  assert.deepEqual([s.scenarios, s.scenariosChanged, s.layersChanged, s.bytesDelta], [2, 2, 2, 200]);
  assert.deepEqual(s.perLayer, { policy: 200 });
});

test('identical fixtures change nothing', () => {
  const diff = diffEnvelopes(base, base);
  assert.ok(diff.rows.every((r) => r.delta === 0));
  assert.equal(renderMarkdown(diff), 'No layer changed in 2 scenarios.');
});

test('a layer or scenario on one side only shows as new or gone', () => {
  const grown = structuredClone(base);
  grown.scenarios[0].layers.push({ name: 'taskState', bytes: 640 });
  grown.scenarios[0].total += 640;
  grown.scenarios.pop();
  const diff = diffEnvelopes(base, grown);
  const md = renderMarkdown(diff);
  assert.match(md, /\| checkInWake · claude \| taskState \| — \| 640 \| \+640 \|/);
  assert.match(md, /\| packetChild · codex \| policy \| 5,844 \| — \| -5,844 \|/);
  assert.match(md, /\| packetChild · codex \| 18,795 \| — \| -18,795 \|/);
});

test('turn-budget.json rows read their after sizes as layers', () => {
  const row = (workspace) => ({ agent: 'grok', surface: 'vaultChat', turn: 'resumed', before: {}, after: { systemPrompt: 0, prompt: 900 + workspace, configured: 400, workspace, total: 900 + workspace } });
  const diff = diffEnvelopes({ rows: [row(100)] }, { rows: [row(150)] });
  const changed = diff.rows.filter((r) => r.delta);
  assert.deepEqual(changed.map((r) => [r.scenario, r.layer, r.delta]), [
    ['vaultChat/resumed · grok', 'prompt', 50],
    ['vaultChat/resumed · grok', 'workspace', 50],
  ]);
});

test('the CLI prints the table and writes the counts', async () => {
  const a = join(dir, 'base.json');
  const b = join(dir, 'head.json');
  const json = join(dir, 'envelope-diff.json');
  writeFileSync(a, JSON.stringify(base));
  writeFileSync(b, JSON.stringify(head));
  const { code, out } = await run([a, b, '--json', json]);
  assert.equal(code, 0);
  assert.match(out, /\| checkInWake · claude \| policy \| 5,844 \| 5,944 \| \+100 \|/);
  const written = JSON.parse(readFileSync(json, 'utf8'));
  assert.equal(written.bytesDelta, 200);
  assert.equal(Object.keys(written)[0], 'scenarios', 'number leaves first');
});

/** A stand-in for git: `refs` name commits, `trees` hold each commit's files. */
function fakeGit(refs, trees) {
  const calls = [];
  const git = (args) => {
    calls.push(args);
    if (args[0] === 'rev-parse') {
      const sha = refs[args.at(-1).replace(/\^\{commit\}$/, '')];
      if (!sha) throw new Error('exit 1');
      return `${sha}\n`;
    }
    if (args[0] === 'show') {
      const [sha, path] = args[1].split(/:(.*)/s);
      const text = trees[sha]?.[path];
      if (text == null) throw new Error(`fatal: path '${path}' does not exist in '${sha}'`);
      return text;
    }
    throw new Error(`unexpected git ${args.join(' ')}`);
  };
  return { git, calls };
}

const SHA = 'a1b2c3d4e5'.repeat(4);
const REL = 'tests/fixtures/prompts/envelope.json';

test('with no files the working tree is diffed against the ref, whose short SHA heads the output', async () => {
  mkdirSync(join(dir, 'tests', 'fixtures', 'prompts'), { recursive: true });
  writeFileSync(join(dir, REL), JSON.stringify(head));
  const { git, calls } = fakeGit({ 'origin/main': SHA }, { [SHA]: { [REL]: JSON.stringify(base) } });
  const { code, out } = await run([], { git });
  assert.equal(code, 0);
  assert.match(out, /^Base: origin\/main at a1b2c3d4e5; head: the working tree's tests\/fixtures\/prompts\/envelope\.json\.$/m);
  assert.match(out, /\| checkInWake · claude \| policy \| 5,844 \| 5,944 \| \+100 \|/);
  assert.deepEqual(calls[0], ['rev-parse', '--verify', '--quiet', '--end-of-options', 'origin/main^{commit}']);
});

test('a commit without the fixture reads as empty: every layer is new', async () => {
  writeFileSync(join(dir, REL), JSON.stringify(head));
  const { git } = fakeGit({ 'o17/base': SHA }, { [SHA]: {} });
  const { code, out } = await run(['--ref', 'o17/base'], { git });
  assert.equal(code, 0);
  assert.match(out, /o17\/base has no tests\/fixtures\/prompts\/envelope\.json: every layer is new\./);
  assert.match(out, /\| checkInWake · claude \| policy \| — \| 5,944 \| \+5,944 \|/);
});

test('a ref that names no commit exits 3, never "every layer is new"', async () => {
  writeFileSync(join(dir, REL), JSON.stringify(head));
  const { git } = fakeGit({ 'origin/main': SHA }, { [SHA]: {} });
  const typo = await run(['--ref', 'orgin/main'], { git });
  assert.equal(typo.code, 3);
  assert.match(typo.err, /orgin\/main is not a commit here/);
  assert.doesNotMatch(typo.out, /every layer is new/);
  // Real git, in a folder that is no checkout: nothing resolves. The ceiling
  // stops git at the temp folder, so a TMPDIR inside a checkout can't lend
  // it that checkout's origin/main (overnight 9 A12).
  const ceiling = process.env.GIT_CEILING_DIRECTORIES;
  process.env.GIT_CEILING_DIRECTORIES = dirname(dir);
  const real = await run(['--ref', 'origin/main']).finally(() => {
    if (ceiling === undefined) delete process.env.GIT_CEILING_DIRECTORIES;
    else process.env.GIT_CEILING_DIRECTORIES = ceiling;
  });
  assert.equal(real.code, 3);
  assert.match(real.err, /origin\/main is not a commit here/);
  assert.equal((await run(['--ref', '-x'], { git })).code, 2, 'a ref starting with - is refused');
});

test('a missing file exits 3, a bad argument 2', async () => {
  const missing = await run([join(dir, 'absent.json'), join(dir, 'absent.json')]);
  assert.equal(missing.code, 3);
  assert.match(missing.err, /no fixture at/);
  assert.equal((await run(['only-one.json'])).code, 2);
  assert.equal((await run(['--nope'])).code, 2);
  assert.equal((await run(['--json'])).code, 2);
  assert.equal((await run(['--fixture', '../x.json'])).code, 2);
});
