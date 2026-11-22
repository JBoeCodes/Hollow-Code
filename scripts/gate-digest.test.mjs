// GD1: the digest of gate, build and test output. A failing gate and a
// passing one each print under 60 lines with the log path, the digest names
// every failing test, and a secret in what it quotes is masked.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { Digest, MAX_LINES, digestText, mask } from './gate-digest.mjs';
import { RUNS_DIR } from './gate-run.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Thousands of passing lines, the way a gate prints them, with noise a real error must not be confused with. */
const noise = n => Array.from({ length: n }, (_, i) => (i % 50 === 0 ? '[chat limits] Error: render failed' : ` ✓ src/part${i}.test.ts (4 tests) ${i % 9}ms`)).join('\n');

const PASSING = [
  '> hollow@0.5.15 gate:frontend',
  noise(3000),
  ' Test Files  412 passed (412)',
  '      Tests  6790 passed (6790)',
  'ℹ tests 1020',
  'ℹ pass 1020',
  'ℹ fail 0',
  '> hollow@0.5.15 gate:rust',
  '   Compiling hollow v0.5.15 (/x/src-tauri)',
  '    Finished `dev` profile [unoptimized] target(s) in 1m 02s',
  '    Finished `test` profile [unoptimized] target(s) in 2m 10s',
  'test result: ok. 6804 passed; 0 failed; 58 ignored; 0 measured; 0 filtered out; finished in 120.5s',
  '[gate-record] Gate: passed, gate fingerprint `' + 'a'.repeat(64) + '`',
].join('\n');

const FAILING = [
  '> hollow@0.5.15 gate:frontend',
  noise(2000),
  ' FAIL  src/skillCatalog.test.ts > lintBadge > labels the score',
  'AssertionError: expected "Lint 80" to be "Lint 85"',
  ' - Expected',
  ' + Received',
  ' ❯ src/skillCatalog.test.ts:41:9',
  ' FAIL  src/capabilityDoctor.test.ts > context > flags drift',
  ' Test Files  2 failed | 410 passed (412)',
  '      Tests  2 failed | 6788 passed (6790)',
  '> hollow@0.5.15 gate:rust',
  'Diff in /Users/runner/hollow-remote/work/x/src-tauri/src/context_layers.rs:1377:',
  'Diff in /Users/runner/hollow-remote/work/x/src-tauri/src/skill_lint.rs:12:',
].join('\n');

const lines = (text, over = {}) => digestText(text, { exitCode: 0, seconds: 267, logPath: '/runs/logs/a.log', host: 'build-mac', ...over });

test('a passing gate prints a short digest with its counts and the log path', async () => {
  const out = await lines(PASSING);
  assert.ok(out.length < MAX_LINES, `${out.length} lines`);
  assert.match(out[0], /^gate digest: PASSED in 267 s on build-mac$/);
  const text = out.join('\n');
  assert.match(text, /counts: vitest 6790 passed \(6790\) \(files: 412 passed \(412\)\)/);
  assert.match(text, /counts: node:test 1020 passed, 0 failed of 1020/);
  assert.match(text, /counts: cargo 6804 passed, 0 failed, 58 ignored/);
  assert.match(text, /full log: build-mac:\/runs\/logs\/a\.log/);
  assert.doesNotMatch(text, /first error/, 'a passing gate quotes no error, whatever noise it printed');
});

test('a failing gate names the failed step, every failing test and the first real error', async () => {
  const out = await lines(FAILING, { exitCode: 1 });
  const text = out.join('\n');
  assert.ok(out.length < MAX_LINES, `${out.length} lines`);
  assert.match(out[0], /^gate digest: FAILED \(exit 1\) in 267 s on build-mac$/);
  assert.match(text, /failed step: gate:rust|failed step: cargo fmt/);
  assert.match(text, /failing tests \(2\):/);
  assert.match(text, /src\/skillCatalog\.test\.ts > lintBadge > labels the score/);
  assert.match(text, /src\/capabilityDoctor\.test\.ts > context > flags drift/);
  assert.match(text, /first error:\n {2} FAIL {2}src\/skillCatalog\.test\.ts/, 'the bracketed noise before it is not the first error');
  assert.match(text, /AssertionError: expected "Lint 80" to be "Lint 85"/);
  assert.match(text, /cargo fmt differs in 2 files: src-tauri\/src\/context_layers\.rs, src-tauri\/src\/skill_lint\.rs/);
  assert.match(text, /full log: build-mac:\/runs\/logs\/a\.log$/);
});

test('every failing test is named, past the old list sizes, and the log path still ends the digest', async () => {
  const names = Array.from({ length: 25 }, (_, i) => ` FAIL  src/many${i}.test.ts > case ${i}`);
  const out = await lines(['> hollow@0.5.15 gate:frontend', ...names].join('\n'), { exitCode: 1 });
  for (let i = 0; i < 25; i++) assert.ok(out.some(l => l.includes(`src/many${i}.test.ts > case ${i}`)), `case ${i} named`);
  assert.ok(out.length < MAX_LINES);
  assert.match(out[out.length - 1], /^full log: /);
  // Far past the cap: the budget holds and the log path survives.
  const flood = Array.from({ length: 200 }, (_, i) => ` FAIL  src/flood${i}.test.ts > c`);
  const capped = await lines(flood.join('\n'), { exitCode: 1 });
  assert.ok(capped.length <= MAX_LINES, `${capped.length} lines`);
  assert.match(capped[capped.length - 1], /^full log: /);
  assert.ok(capped.some(l => /and 170 more/.test(l)));
});

test('rust failures: test names, panic context and counts, and cargo errors win over earlier noise', async () => {
  const text = [
    'running 3 tests',
    'test a::ok ... ok',
    'test b::broken ... FAILED',
    'test c::also_broken ... FAILED',
    'failures:',
    '---- b::broken stdout ----',
    "thread 'b::broken' panicked at src/b.rs:10:5:",
    'assertion `left == right` failed',
    '  left: 1',
    ' right: 2',
    '---- c::also_broken stdout ----',
    "thread 'c::also_broken' panicked at src/c.rs:3:1:",
    'test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s',
  ].join('\n');
  const out = (await lines(text, { exitCode: 101 })).join('\n');
  assert.match(out, /failing tests \(2\):\n {2}b::broken\n {2}c::also_broken/);
  assert.match(out, /counts: cargo 1 passed, 2 failed, 0 ignored/);
  assert.match(out, /first error:\n {2}---- b::broken stdout ----\n {2}thread 'b::broken' panicked at/);
  const compile = (await lines('   Compiling x\nerror[E0425]: cannot find value `y` in this scope\n --> src/a.rs:3:5\nerror: could not compile `x` (lib)\n', { exitCode: 101 })).join('\n');
  assert.match(compile, /counts: rustc\/clippy 1 error/);
  assert.match(compile, /first error:\n(?: {2}.*\n)* {2}error\[E0425\]: cannot find value/);
});

test('node:test failures are named and a run with no recognised error shows the last output', async () => {
  const out = (await lines('✔ fine (1ms)\n✖ planted name check (3ms)\nℹ tests 2\nℹ pass 1\nℹ fail 1\n', { exitCode: 1 })).join('\n');
  assert.match(out, /failing tests \(1\):\n {2}planted name check/);
  assert.match(out, /counts: node:test 1 passed, 1 failed of 2/);
  const quiet = (await lines('doing a thing\nthe thing stopped\n', { exitCode: 3 })).join('\n');
  assert.match(quiet, /last output:\n {2}doing a thing\n {2}the thing stopped/);
});

test('secrets in quoted output are masked and no value survives', async () => {
  const token = `ghp_${'a1B2c3D4e5'.repeat(4)}`;
  const text = `error: request failed with token ${token}\nerror: API_KEY=supersecretvalue99 rejected\nBearer ${'x'.repeat(30)}\n`;
  const out = (await lines(text, { exitCode: 1 })).join('\n');
  assert.doesNotMatch(out, /ghp_a1B2|supersecretvalue99|xxxxxxxxxxxxxxxxxxxxxxxx/);
  assert.match(out, /\[masked/);
  assert.equal(mask('plain text stays'), 'plain text stays');
  assert.match(mask('-----BEGIN RSA PRIVATE KEY-----'), /masked private key/);
});

test('a Digest fed line by line gives the same answer as digestText', async () => {
  const { GateLog } = await import('./gate-run.mjs');
  // sleep-ok: the clock only stamps step starts in the log; no assertion reads it
  const gate = new GateLog(Date.now());
  const digest = new Digest(gate);
  for (const l of FAILING.split('\n')) {
    gate.line(l, 0);
    digest.line(l);
  }
  const info = { exitCode: 1, seconds: 267, logPath: '/runs/logs/a.log', host: 'build-mac' };
  assert.deepEqual(digest.lines(info).slice(0, 1), (await digestText(FAILING, info)).slice(0, 1));
});

function gateRun(chain, extra = []) {
  const common = mkdtempSync(join(tmpdir(), 'gate-digest-'));
  const env = { ...process.env, npm_lifecycle_event: 'gate', HOLLOW_GATE_COMMON_DIR: common };
  const r = spawnSync(process.execPath, [join(root, 'scripts/gate-run.mjs'), chain, ...extra], { cwd: root, encoding: 'utf8', env });
  return { r, common };
}

test('gate-run --digest prints only the digest, keeps the full log and keeps the exit code', () => {
  const chain = `echo '> hollow@0.0.0 gate:frontend'; for i in $(seq 1 300); do echo "passing line $i"; done; echo ' FAIL  src/x.test.ts > it breaks'; echo ' Test Files  1 failed (1)'; exit 1`;
  const { r, common } = gateRun(chain, ['--digest']);
  try {
    assert.equal(r.status, 1);
    const lines = r.stdout.split('\n').filter(Boolean);
    assert.ok(lines.length < MAX_LINES, `${lines.length} lines`);
    assert.match(r.stdout, /^gate digest: FAILED \(exit 1\)/);
    assert.match(r.stdout, /failing tests \(1\):\n {2}src\/x\.test\.ts > it breaks/);
    assert.doesNotMatch(r.stdout, /passing line 150/);
    const log = r.stdout.match(/full log: (?:[^:\n]+:)?(\S+\.log)$/m)?.[1];
    assert.ok(log && existsSync(log), `the digest names a log that exists: ${log}`);
    assert.match(readFileSync(log, 'utf8'), /passing line 150/);
    assert.equal(readdirSync(join(common, RUNS_DIR)).filter(n => n.endsWith('.json')).length, 1, 'the run is still recorded');
  } finally {
    rmSync(common, { recursive: true, force: true });
  }
});

test('gate-run without --digest, or with --raw, still prints every line', () => {
  for (const extra of [[], ['--raw']]) {
    const { r, common } = gateRun(`for i in 1 2 3; do echo "line $i"; done`, extra);
    try {
      assert.equal(r.status, 0);
      assert.match(r.stdout, /line 1\nline 2\nline 3/);
      assert.doesNotMatch(r.stdout, /gate digest/);
    } finally {
      rmSync(common, { recursive: true, force: true });
    }
  }
});
