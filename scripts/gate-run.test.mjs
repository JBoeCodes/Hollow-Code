// Gate telemetry (overnight 10-17, OF11): `npm run gate` runs its chain
// through scripts/gate-run.mjs, which tees the output and records every run,
// failed ones too, in `<git common dir>/hollow-gate-runs/`.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { GateLog, KEEP_LOGS, lineSplitter, prune, RUNS_DIR, runFileName, runOrigin, tmpdirProblem } from './gate-run.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

const alive = pid => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e) {
    return e.code === 'EPERM';
  }
};
async function until(cond, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (cond()) return true;
    await delay(25);
  }
  return cond();
}
const readPid = file => (existsSync(file) ? Number(readFileSync(file, 'utf8').trim()) || null : null);
const killQuietly = pid => {
  try {
    if (pid) process.kill(pid, 'SIGKILL');
  } catch {
    // gone
  }
};

/** gate-run.mjs on `chain` as a live child (common dir `common`); `exited` resolves to {code, signal}, or null after `ms`. */
function gateRunLive(chain, common, env = {}) {
  const child = spawn(process.execPath, [join(root, 'scripts/gate-run.mjs'), chain], {
    cwd: root,
    env: { ...process.env, npm_lifecycle_event: 'gate', HOLLOW_GATE_COMMON_DIR: common, ...env },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const out = { stdout: '', stderr: '' };
  child.stdout.on('data', c => (out.stdout += c));
  child.stderr.on('data', c => (out.stderr += c));
  const done = new Promise(res => child.on('exit', (code, signal) => res({ code, signal })));
  const exited = ms => Promise.race([done, delay(ms).then(() => null)]);
  return { child, out, exited };
}

const theRun = common => {
  const dir = join(common, RUNS_DIR);
  const names = readdirSync(dir).filter(n => n.endsWith('.json'));
  assert.equal(names.length, 1, `one run record, got ${names}`);
  return { run: JSON.parse(readFileSync(join(dir, names[0]), 'utf8')), log: join(dir, 'logs', names[0].replace(/\.json$/, '.log')) };
};

/** Run gate-run.mjs on `chain` with its common dir in a scratch dir; the run record it wrote. */
function gateRun(chain, prepare = () => {}) {
  const common = mkdtempSync(join(tmpdir(), 'gate-run-'));
  prepare(common);
  const env = { ...process.env, npm_lifecycle_event: 'gate', HOLLOW_GATE_COMMON_DIR: common };
  const r = spawnSync(process.execPath, [join(root, 'scripts/gate-run.mjs'), chain], { cwd: root, encoding: 'utf8', env });
  const dir = join(common, RUNS_DIR);
  const names = readdirSync(dir).filter(n => n.endsWith('.json'));
  assert.equal(names.length, 1, `one run record, got ${names}`);
  const run = JSON.parse(readFileSync(join(dir, names[0]), 'utf8'));
  rmSync(common, { recursive: true, force: true });
  return { r, run, name: names[0] };
}

const header = name => `echo '> hollow@0.0.0 ${name}'`;

test('a TMPDIR the gate cannot use is named, a short neutral one is kept (overnight 9 A12)', () => {
  assert.equal(tmpdirProblem(undefined, root), null, 'unset: /tmp');
  assert.equal(tmpdirProblem('/tmp', root), null);
  assert.equal(tmpdirProblem('/tmp/o19a.Ab12Cd', root), null, 'the lanes\' own 16-character folders');
  assert.match(tmpdirProblem('/var/folders/xy/abcdefgh/T/', root), /longer than 16/);
  assert.equal(tmpdirProblem(join(root, 'tmp'), root), 'inside the checkout');
  assert.equal(tmpdirProblem(root, root), 'inside the checkout');
  assert.match(tmpdirProblem('/tmp/hollow-run-1/x', root), /Hollow run/);
});

test('the chain gets a fresh short TMPDIR for a long or in-checkout one, recorded and removed at exit (A12)', () => {
  for (const [given, replaced] of [[join(root, 'node_modules', '.gate-tmp'), true], ['/tmp', false]]) {
    const common = mkdtempSync(join(tmpdir(), 'gate-run-'));
    const env = { ...process.env, npm_lifecycle_event: 'gate', HOLLOW_GATE_COMMON_DIR: common, TMPDIR: given };
    const r = spawnSync(process.execPath, [join(root, 'scripts/gate-run.mjs'), 'echo "T=$TMPDIR"; test -d "$TMPDIR"'], { cwd: root, encoding: 'utf8', env });
    const dir = join(common, RUNS_DIR);
    const run = JSON.parse(readFileSync(join(dir, readdirSync(dir).find(n => n.endsWith('.json'))), 'utf8'));
    rmSync(common, { recursive: true, force: true });
    const used = r.stdout.match(/^T=(.*)$/m)?.[1];
    assert.equal(r.status, 0, r.stderr);
    if (replaced) {
      assert.match(used, /^\/tmp\/hg\.[A-Za-z0-9]{6}$/);
      assert.deepEqual(run.tmpdir, { used, replaced: given, why: 'inside the checkout' });
      assert.match(r.stderr, /this gate uses \/tmp\/hg\./);
      assert.equal(existsSync(used), false, 'removed when the wrapper exits');
    } else {
      assert.equal(used, '/tmp');
      assert.equal(run.tmpdir, null);
    }
  }
});

test('a gate failing at step 3 is recorded with the step, the failing test and the slow test, and keeps its exit code', () => {
  const chain = [
    header('one'),
    'echo "event=$npm_lifecycle_event"',
    header('two'),
    header('three'),
    "printf '%s\\n' 'test chat::slow_one has been running for over 60 seconds' '---- usage_ledger::tests::breaks stdout ----' 'failures:' '    usage_ledger::tests::breaks' 'test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.50s'",
    'echo oops >&2',
    'exit 7',
    header('four'),
  ].join(' && ');
  const { r, run, name } = gateRun(chain);
  assert.equal(r.status, 7, r.stderr);
  // Tee: the output still streams, so callers that grep the log still match.
  assert.match(r.stdout, /event=gate/, 'npm_lifecycle_event reaches the steps, so gate-record still records');
  assert.match(r.stdout, /^test result: FAILED\. 1 passed; 1 failed/m);
  assert.match(r.stderr, /oops/);
  assert.doesNotMatch(r.stdout, /hollow@0\.0\.0 four/);
  assert.equal(run.exitCode, 7);
  assert.equal(run.ok, false);
  assert.equal(run.failedStep, 'three');
  assert.deepEqual(run.failingTests, ['usage_ledger::tests::breaks']);
  assert.deepEqual(run.slowTests, ['chat::slow_one']);
  assert.equal(run.durations.libtest, 0.5);
  assert.deepEqual(run.steps.map(s => s.name), ['start', 'one', 'two', 'three']);
  assert.deepEqual(run.steps.map(s => s.ok), [true, true, true, false]);
  assert.match(run.fingerprint, /^[0-9a-f]{64}$/);
  assert.match(run.commit, /^[0-9a-f]{40}$/);
  assert.ok(name.startsWith(runFileName(new Date(run.start), run.fingerprint).slice(0, 10)));
  assert.ok(name.endsWith(`-${run.fingerprint.slice(0, 8)}.json`));
  assert.ok(run.end >= run.start);
  assert.equal(run.load.start.length, 3);
  assert.equal(run.load.end.length, 3);
  assert.equal(run.otherStartSnapshot, false);
  assert.equal(run.log, undefined, 'the record holds what was parsed, never the log text');
});

test('a passing gate exits 0, sums every libtest "finished in", and sees another checkout gating', () => {
  const chain = [
    header('gate:rust'),
    "printf '%s\\n' '    Finished `dev` profile [unoptimized] target(s) in 35.86s' '    Finished `test` profile [unoptimized] target(s) in 1m 4.70s'",
    "echo 'test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 1.25s'",
    "echo 'test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 100.5s'",
    "echo '[gate-record] Gate: `npm run gate` passed on clean `abcdef12`; gate fingerprint `" + 'b'.repeat(64) + "`.'",
  ].join(' && ');
  let lane = null;
  const { r, run } = gateRun(chain, common => {
    const starts = join(common, 'hollow-gate-records', 'start');
    mkdirSync(starts, { recursive: true });
    lane = join(common, 'other-lane');
    mkdirSync(lane);
    const now = Date.now();
    writeFileSync(join(starts, '0123456789abcdef.json'), JSON.stringify({ checkout: lane, at: new Date(now - 5 * 60_000).toISOString() }));
    // A failed gate never removes its snapshot: an old one, or one from a removed worktree, is no gate running now.
    writeFileSync(join(starts, '1123456789abcdef.json'), JSON.stringify({ checkout: lane, at: new Date(now - 2 * 3600_000).toISOString() }));
    writeFileSync(join(starts, '2123456789abcdef.json'), JSON.stringify({ checkout: join(common, 'removed-lane'), at: new Date(now).toISOString() }));
  });
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /\[gate-record\] Gate: `npm run gate` passed on clean `abcdef12`/);
  assert.equal(run.ok, true);
  assert.equal(run.failedStep, null);
  assert.equal(run.durations.libtest, 101.75);
  assert.equal(run.durations.libtestRuns, 2);
  assert.deepEqual(run.durations.cargo, { dev: 35.86, test: 64.7 });
  assert.equal(run.gateRecord.recorded, true);
  assert.equal(run.gateRecord.fingerprint, 'b'.repeat(64));
  assert.deepEqual(run.steps.map(s => s.name), ['start', 'cargo fmt', 'cargo clippy', 'cargo test (build)', 'cargo test', 'gate-record']);
  assert.equal(run.otherStartSnapshot, true);
  assert.deepEqual(run.otherStarts.map(s => s.checkout), [lane], 'only a snapshot from the last hour whose checkout still exists');
});

test('SIGTERM to the wrapper reaches the running step: the wrapper exits 143 within 3 s and the step is gone', async () => {
  const common = mkdtempSync(join(tmpdir(), 'gate-run-term-'));
  const pidFile = join(common, 'step.pid');
  // The step is its own process under /bin/sh, as vitest or cargo is.
  const chain = `${header('one')} && sh -c 'echo $$ > "$0"; exec sleep 30' "${pidFile}"`;
  const { child, out, exited } = gateRunLive(chain, common);
  let pid = null;
  try {
    assert.ok(await until(() => (pid = readPid(pidFile)), 15_000), `the step started: ${out.stderr}`);
    const t0 = Date.now();
    child.kill('SIGTERM');
    const res = await exited(5000);
    assert.ok(res, 'the wrapper exited');
    assert.ok(Date.now() - t0 < 3000, `exited in ${Date.now() - t0} ms`);
    assert.equal(res.code, 143, out.stderr);
    assert.ok(await until(() => !alive(pid), 1000), 'the sleep step was signalled too');
    const { run } = theRun(common);
    assert.equal(run.exitCode, 143);
    assert.equal(run.failedStep, 'one');
  } finally {
    killQuietly(pid);
    child.kill('SIGKILL');
    rmSync(common, { recursive: true, force: true });
  }
});

test('a leftover process holding stdout does not hold the record: the exit code is taken at exit, close is waited for briefly, and the leftover is signalled', async () => {
  const common = mkdtempSync(join(tmpdir(), 'gate-run-close-'));
  const pidFile = join(common, 'leftover.pid');
  // As a Rust test's `/bin/sleep 60` child does: it inherits the step's stdout and outlives it.
  const chain = `${header('one')} && { sleep 30 & echo $! > "${pidFile}"; } && echo done`;
  const { child, out, exited } = gateRunLive(chain, common, { HOLLOW_GATE_CLOSE_WAIT_MS: '300' });
  try {
    const res = await exited(4000);
    assert.ok(res, `the wrapper exited without waiting for the leftover: ${out.stderr}`);
    assert.equal(res.code, 0, out.stderr);
    assert.match(out.stdout, /^done$/m);
    const { run } = theRun(common);
    assert.equal(run.exitCode, 0);
    assert.ok(run.end);
    const pid = readPid(pidFile);
    assert.ok(pid);
    assert.ok(await until(() => !alive(pid), 1000), 'the leftover in the chain process group was signalled');
  } finally {
    killQuietly(readPid(pidFile));
    child.kill('SIGKILL');
    rmSync(common, { recursive: true, force: true });
  }
});

test('a caller that stops reading (| head -1) gets no EPIPE crash: the chain runs on, the log and record complete, the exit code is the chain\'s', async () => {
  const common = mkdtempSync(join(tmpdir(), 'gate-run-epipe-'));
  const chain = `echo first && sleep 0.3 && seq 1 5000 && echo last-line && exit 3`;
  const { child, out, exited } = gateRunLive(chain, common);
  child.stdout.once('data', () => child.stdout.destroy());
  try {
    const res = await exited(15_000);
    assert.ok(res, 'the wrapper exited');
    assert.equal(res.code, 3, out.stderr);
    assert.doesNotMatch(out.stderr, /EPIPE/);
    const { run, log } = theRun(common);
    assert.equal(run.exitCode, 3);
    assert.ok(run.end, 'the record was finished');
    assert.match(readFileSync(log, 'utf8'), /^last-line$/m, 'the log was still teed after the caller left');
  } finally {
    child.kill('SIGKILL');
    rmSync(common, { recursive: true, force: true });
  }
});

test('a refused CARGO_TARGET_DIR or node_modules is its own failed step, not cargo fmt', () => {
  const rust = new GateLog(0);
  rust.line('> hollow@0.5.15 gate:rust', 1000);
  rust.line('check-cargo-target: CARGO_TARGET_DIR is "target": a relative path lands wherever cargo runs.', 1100);
  assert.equal(rust.summary(2000, 1).failedStep, 'check-cargo-target');
  const frontend = new GateLog(0);
  frontend.line('> hollow@0.5.15 gate:frontend', 1000);
  frontend.line('check-installed-lock: node_modules differs from package-lock.json (CI\'s npm ci would install something else):', 1100);
  assert.equal(frontend.summary(2000, 1).failedStep, 'check-installed-lock');
  // A silent check that passed: fmt still starts at the banner.
  const ok = new GateLog(0);
  ok.line('> hollow@0.5.15 gate:rust', 1000);
  ok.line('Diff in /w/src-tauri/src/lib.rs:12:', 1500);
  assert.equal(ok.summary(2000, 1).failedStep, 'cargo fmt');
});

test('a multibyte character split across chunks still parses', () => {
  const log = new GateLog(0);
  const lines = lineSplitter(log);
  const bytes = Buffer.from('ℹ duration_ms 6212.9\n', 'utf8');
  lines.push(bytes.subarray(0, 1));
  lines.push(bytes.subarray(1));
  lines.end();
  assert.equal(log.summary(1000, 0).durations.nodeTest, 6.21);
});

test('the log parser reads each tool line it times', () => {
  const log = new GateLog(0);
  const lines = [
    '> hollow@0.5.15 gate:frontend',
    '> hollow@0.5.15 test',
    '\x1b[2m   Duration \x1b[22m 11.72s (transform 3.46s, setup 0ms)',
    ' FAIL  src/palette.test.ts > palette > names an action',
    ' FAIL  src/palette.test.ts > palette > names an action',
    '> hollow@0.5.15 test:scripts',
    'ℹ duration_ms 6212.900375',
    '> hollow@0.5.15 build',
    '✓ built in 4.28s',
    '[gate-record] not recorded: the tree has uncommitted changes: commit, then run the gate.',
  ];
  lines.forEach((line, i) => log.line(line, (i + 1) * 1000));
  const s = log.summary(20_000, 1);
  assert.equal(s.durations.vitest, 11.72);
  assert.equal(s.durations.nodeTest, 6.21);
  assert.equal(s.durations.viteBuild, 4.28);
  assert.deepEqual(s.failingTests, ['src/palette.test.ts > palette > names an action']);
  assert.equal(s.gateRecord.recorded, false);
  assert.match(s.gateRecord.skip, /uncommitted/);
  assert.equal(s.failedStep, 'build', 'the frontend half ends in build here, so build failed');
  assert.deepEqual(s.steps.find(x => x.name === 'test'), { name: 'test', at: 2, seconds: 4, ok: true });
});

test('only the newest runs and logs are kept', () => {
  // Both lanes share the logs: 20 is about ten runs each.
  assert.ok(KEEP_LOGS >= 20, `KEEP_LOGS is ${KEEP_LOGS}`);
  const dir = mkdtempSync(join(tmpdir(), 'gate-run-prune-'));
  for (let i = 0; i < 5; i++) writeFileSync(join(dir, `2026-10-17T0${i}-00-00-000Z-abcdef12.log`), 'x');
  writeFileSync(join(dir, 'keep-me.txt'), 'x');
  prune(dir, '.log', 2);
  assert.deepEqual(readdirSync(dir).sort(), ['2026-10-17T03-00-00-000Z-abcdef12.log', '2026-10-17T04-00-00-000Z-abcdef12.log', 'keep-me.txt']);
  rmSync(dir, { recursive: true, force: true });
});

test('npm run gate is the same chain, run through gate-run', () => {
  const gate = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).scripts.gate;
  const m = gate.match(/^node scripts\/gate-run\.mjs '([^']+)'$/);
  assert.ok(m, gate);
  assert.equal(m[1], 'node scripts/gate-record.mjs --start && npm run gate:frontend && npm run gate:rust && node scripts/gate-record.mjs');
});

test('a run names its machine, and one hollow-slot --remote started records the caller HEAD; the chain never sees those variables', () => {
  const snap = { commit: 'a'.repeat(40), dirty: false };
  assert.deepEqual(runOrigin(snap, {}, 'dev-mac.local'), { commit: snap.commit, dirty: false, host: 'dev-mac', remote: false });
  const head = 'b'.repeat(40);
  assert.deepEqual(runOrigin(snap, { HOLLOW_REMOTE_HEAD: head, HOLLOW_REMOTE_DIRTY: '1' }, 'build-mac'), { commit: head, dirty: true, host: 'build-mac', remote: true });
  assert.deepEqual(runOrigin(snap, { HOLLOW_REMOTE_HEAD: head, HOLLOW_REMOTE_DIRTY: '0' }, 'build-mac'), { commit: head, dirty: false, host: 'build-mac', remote: true });
  assert.equal(runOrigin(snap, { HOLLOW_REMOTE_HEAD: 'not a sha' }, 'x').commit, snap.commit, 'a bad value is ignored');
  assert.equal(runOrigin(snap, {}, '').host, null);

  const common = mkdtempSync(join(tmpdir(), 'gate-run-'));
  try {
    const env = { ...process.env, npm_lifecycle_event: 'gate', HOLLOW_GATE_COMMON_DIR: common, HOLLOW_REMOTE_HEAD: head, HOLLOW_REMOTE_DIRTY: '1' };
    const r = spawnSync(process.execPath, [join(root, 'scripts/gate-run.mjs'), 'echo "head=${HOLLOW_REMOTE_HEAD:-none} dirty=${HOLLOW_REMOTE_DIRTY:-none}"'], { cwd: root, encoding: 'utf8', env });
    assert.equal(r.status, 0, r.stderr);
    assert.match(r.stdout, /^head=none dirty=none$/m);
    const dir = join(common, RUNS_DIR);
    const [name] = readdirSync(dir).filter(n => n.endsWith('.json'));
    const run = JSON.parse(readFileSync(join(dir, name), 'utf8'));
    assert.equal(run.commit, head);
    assert.equal(run.dirty, true);
    assert.equal(run.remote, true);
    assert.equal(typeof run.host, 'string');
  } finally {
    rmSync(common, { recursive: true, force: true });
  }
});
