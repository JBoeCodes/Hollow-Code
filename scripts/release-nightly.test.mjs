import { fileURLToPath } from 'node:url';
// Tests for scripts/release/nightly.mjs (updates row U2). Deterministic: a
// temp bare "origin" and a builder clone driven by real git, a fake runner
// for every build tool (no npm, tauri, codesign or network), a fake bucket
// for the uploader and a fixed clock.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { parseManifest } from '../infra/updates-worker/src/manifest.js';
import { appPath } from './release/build-app.mjs';
import {
  SCHEMA_SOURCES,
  acquireLock,
  defaultDeps,
  loadConfig,
  redact,
  signerArgs,
  lockPath,
  mainHistoryPath,
  mainVersions,
  promoteLockPath,
  pruneBuilds,
  readSchema,
  runGrouped,
  runNightly,
  statePath,
} from './release/nightly.mjs';

const GIT_ENV = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_AUTHOR_NAME: 'T', GIT_AUTHOR_EMAIL: 't@example.test', GIT_COMMITTER_NAME: 'T', GIT_COMMITTER_EMAIL: 't@example.test' };
const g = (cwd, ...args) => execFileSync('git', args, { cwd, env: GIT_ENV, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const put = (root, path, text) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), text); };

const SECRETS = {
  TAURI_SIGNING_PRIVATE_KEY: 'UPDATER-KEY-SECRET-1',
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'UPDATER-PASSWORD-SECRET-2',
  HOLLOW_R2_ACCESS_KEY_ID: 'R2-ID-SECRET-3',
  HOLLOW_R2_SECRET_ACCESS_KEY: 'R2-KEY-SECRET-4',
  APPLE_API_KEY: 'APPLE-KEY-SECRET-5',
  APPLE_API_ISSUER: 'APPLE-ISSUER-SECRET-6',
  APPLE_API_KEY_PATH: '/secret/AuthKey-SECRET-7.p8',
};
const DEV_ID = 'Developer ID Application: Example Person (TEAM000000)';
const FP = 'a'.repeat(64);

/** origin (bare), a seed clone that pushes to it, and the builder's clone. */
function setup(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-nightly-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const origin = join(root, 'origin.git');
  const seed = join(root, 'seed');
  g(root, 'init', '-q', '--bare', '-b', 'main', origin);
  g(root, 'init', '-q', '-b', 'main', seed);
  g(seed, 'remote', 'add', 'origin', origin);
  put(seed, 'docs/BUILD_LOG.md', '# Build log\n');
  put(seed, 'docs/WHATS_NEW.md', "# What's new\n\n### Old thing\n\n- **What:** old.\n");
  put(seed, 'src-tauri/src/session_store.rs', 'pub(crate) const SCHEMA_VERSION: i64 = 0;\n');
  put(seed, 'src-tauri/src/companion_inbox.rs', 'pub(crate) const LEDGER_SCHEMA_VERSION: i64 = 0;\n');
  put(seed, 'src-tauri/src/vault_search_index.rs', 'const SCHEMA_VERSION: i64 = 2;\n');
  put(seed, 'src-tauri/src/vault_embed_index.rs', 'const SCHEMA_VERSION: i64 = 3;\n');
  put(seed, 'src-tauri/src/project_text_index.rs', 'const SCHEMA_VERSION: i64 = 1;\n');
  g(seed, 'add', '.');
  g(seed, 'commit', '-q', '-m', 'Start');
  mergePr(seed, 12, 'Add the frobnicator', 'frob');
  g(seed, 'push', '-q', 'origin', 'main');
  const builder = join(root, 'builder');
  g(root, 'clone', '-q', origin, builder);
  const stateDir = join(root, 'state');
  const env = { ...SECRETS, HOLLOW_SIGNING_IDENTITY: DEV_ID, CARGO_TARGET_DIR: join(root, 'target'), PATH: process.env.PATH };
  const config = loadConfig({ HOLLOW_NIGHTLY_REPO: builder, HOLLOW_NIGHTLY_STATE_DIR: stateDir, HOLLOW_UPDATES_HOST: 'updates.example.test' });
  return { root, origin, seed, builder, stateDir, env, config };
}

function mergePr(seed, number, title, file) {
  g(seed, 'checkout', '-q', '-b', `b${number}`);
  put(seed, `${file}.txt`, title);
  g(seed, 'add', '.');
  g(seed, 'commit', '-q', '-m', `${title} work`);
  g(seed, 'checkout', '-q', 'main');
  g(seed, 'merge', '--no-ff', '-q', `b${number}`, '-m', `Merge pull request #${number} from me/b${number}`, '-m', title);
}

function gateRecord(builder, fp = FP) {
  const dir = join(builder, '.git', 'hollow-gate-records');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${fp}.json`), JSON.stringify({ sourceFingerprint: fp, gate: 'passed', dirty: false }));
}

/** A bucket in memory; `failOn` makes one put throw. */
function fakeBucket({ failOn = null, events = [] } = {}) {
  const objects = new Map();
  return {
    objects,
    puts: [],
    async put(key, bytes) {
      events.push(`put ${key}`);
      if (key === failOn) throw new Error('bucket said no');
      this.puts.push(key);
      objects.set(key, Buffer.from(bytes));
    },
    async get(key) { return objects.has(key) ? objects.get(key).toString('utf8') : null; },
    async list(prefix) { return [...objects.keys()].filter(k => k.startsWith(prefix)); },
    async delete(key) { events.push(`delete ${key}`); objects.delete(key); },
  };
}

/**
 * A runner that answers like the real tools on a good run. `status` maps a
 * step name (install, gate, build, verify, selftest, tar, sign) to an exit code.
 */
function fakeRun({ env, status = {}, events = [] }) {
  const calls = [];
  const run = (cmd, args, opts = {}) => {
    calls.push({ cmd, args, env: opts.env, cwd: opts.cwd, timeout: opts.timeout });
    const a = args.join(' ');
    const name = cmd === 'npm' && args[0] === 'ci' ? 'install'
      : cmd === 'npm' && a === 'run gate' ? 'gate'
        : a.includes('source-provenance.mjs --print') ? 'fingerprint'
          : a.includes('build-app.mjs') && a.includes('--no-verify') ? 'build'
            : a.includes('build-app.mjs') && a.includes('--selftest-only') ? 'selftest'
              : a.includes('build-app.mjs') && a.includes('--verify-only') ? 'verify'
                : cmd === '/usr/bin/tar' ? (args.includes('-xzf') ? 'untar' : 'tar')
                  : cmd === 'npx' && a.includes('signer sign') ? 'sign'
                    : cmd === '/usr/bin/codesign' ? 'codesign'
                      : cmd === '/usr/sbin/spctl' ? 'spctl'
                        : cmd === '/usr/bin/xcrun' ? 'stapler' : `unknown ${cmd} ${a}`;
    events.push(name);
    const code = status[name] ?? 0;
    if (code !== 0) return { status: code, stdout: '', stderr: `${name} broke` };
    if (name === 'fingerprint') return { status: 0, stdout: JSON.stringify({ sourceFingerprint: FP, commit: 'x', dirty: false }), stderr: '' };
    if (name === 'build') {
      const app = appPath(opts.cwd, env);
      mkdirSync(join(app, 'Contents'), { recursive: true });
      writeFileSync(join(app, 'Contents', 'Info.plist'), '<plist/>');
    }
    if (name === 'tar') writeFileSync(args[args.indexOf('-czf') + 1], 'TARBYTES');
    if (name === 'sign') writeFileSync(`${args[args.length - 1]}.sig`, 'U0lHTkFUVVJF\n');
    if (name === 'untar') {
      const app = join(args[args.indexOf('-C') + 1], 'Hollow.app', 'Contents');
      mkdirSync(app, { recursive: true });
      writeFileSync(join(app, 'Info.plist'), '<plist/>');
    }
    return { status: 0, stdout: '', stderr: '' };
  };
  return { run, calls };
}

function deps({ env, bucket, runner, pid = 4242, isAlive = () => false }) {
  let clock = Date.parse('2026-11-15T12:00:00Z');
  return { ...defaultDeps(env), run: runner.run, uploader: bucket, now: () => (clock += 1000), pid, isAlive, startTimeOf: () => null, host: 'test-host', execPath: '/usr/local/bin/node' };
}

async function go(s, { status = {}, failOn = null, argv = [], bucket, logs = [] } = {}) {
  const events = [];
  const b = bucket ?? fakeBucket({ failOn, events });
  const runner = fakeRun({ env: s.env, status, events });
  const result = await runNightly({ config: s.config, env: s.env, deps: deps({ env: s.env, bucket: b, runner }), argv, log: (l) => logs.push(l) });
  return { result, bucket: b, runner, events, logs };
}

const outbox = (s) => (existsSync(s.config.outbox) ? readdirSync(s.config.outbox) : []);

test('a good run publishes only after every check passes, manifest last', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const { result, bucket, events, runner } = await go(s);
  assert.equal(result.status, 'published', JSON.stringify(result));
  const tip = g(s.seed, 'rev-parse', 'HEAD');
  const count = g(s.seed, 'rev-list', '--count', '--first-parent', 'HEAD');
  assert.equal(result.version, `0.6.${count}`, 'the stamp is the tip\'s first-parent count');
  assert.equal(result.commit, tip);

  const steps = events.filter(e => !e.startsWith('put') && e !== 'fingerprint');
  assert.deepEqual(steps, ['install', 'build', 'verify', 'selftest', 'tar', 'sign', 'untar', 'codesign', 'spctl', 'stapler'], 'the gate record stood in for npm run gate; the unpacked archive is assessed');
  const firstPut = events.findIndex(e => e.startsWith('put'));
  assert.ok(firstPut > events.indexOf('stapler'), 'nothing is uploaded before every check passed');
  assert.ok(runner.calls.every(c => c.timeout > 0), 'every step has a time limit');
  const spctl = runner.calls.find(c => c.cmd === '/usr/sbin/spctl');
  assert.match(spctl.args.at(-1), /\/out\/0\.6\.\d+\/unpacked\/Hollow\.app$/, 'spctl assesses the unpacked archive, not the built bundle');
  const v = result.version;
  assert.deepEqual(bucket.puts, [`builds/${v}/Hollow.app.tar.gz`, `builds/${v}/Hollow.app.tar.gz.sig`, `builds/${v}/manifest.json`, `builds/${v}/notes.json`, 'channels/nightly.json']);
  assert.equal(await bucket.get(`builds/${v}/manifest.json`), await bucket.get('channels/nightly.json'), 'the build keeps the exact manifest it was published with (row U8)');

  const manifest = parseManifest(await bucket.get('channels/nightly.json'), 'nightly');
  assert.ok(manifest, 'the Worker would serve it');
  assert.equal(manifest.platforms['darwin-aarch64'].url, `https://updates.example.test/builds/${v}/Hollow.app.tar.gz`);
  assert.equal(manifest.platforms['darwin-aarch64'].signature, 'U0lHTkFUVVJF');
  assert.equal(manifest.commit, tip);
  assert.deepEqual(manifest.schema, { sessions: 0, companion: 0, searchIndex: 2, embedIndex: 3, projectIndex: 1 });
  assert.match(manifest.pub_date, /^2026-11-15T12:00:\d\dZ$/);
  assert.match(manifest.notes, /#12 Add the frobnicator/);
  const notes = JSON.parse(await bucket.get(`builds/${v}/notes.json`));
  assert.equal(notes.commit, tip);
  assert.equal(notes.previous, null);

  const state = JSON.parse(readFileSync(statePath(s.config), 'utf8'));
  assert.equal(state.lastPublished.commit, tip);
  assert.deepEqual(outbox(s), [], 'no notification on success');
  assert.ok(!existsSync(lockPath(s.config)), 'the lock is released');
  assert.ok(runner.calls.every(c => c.cwd === s.config.worktree || c.cwd.startsWith(join(s.stateDir, 'out'))), 'every tool runs in the release worktree or its output folder');
  assert.equal(g(s.config.worktree, 'rev-parse', 'HEAD'), tip);
});

test('no gate record: npm run gate runs first, and a failing gate publishes nothing and notifies', async (t) => {
  const s = setup(t);
  const ok = await go(s);
  assert.equal(ok.result.status, 'published');
  assert.ok(ok.events.indexOf('gate') > ok.events.indexOf('install') && ok.events.indexOf('gate') < ok.events.indexOf('build'));

  mergePr(s.seed, 13, 'Second change', 'two');
  g(s.seed, 'push', '-q', 'origin', 'main');
  const bad = await go(s, { status: { gate: 1 } });
  assert.equal(bad.result.status, 'failed');
  assert.equal(bad.result.check, 'gate');
  assert.deepEqual(bad.bucket.puts, []);
  assert.ok(!bad.events.includes('build'));
  const tip = g(s.seed, 'rev-parse', 'HEAD');
  const notice = JSON.parse(readFileSync(join(s.config.outbox, `failure-${tip}.json`), 'utf8'));
  assert.equal(notice.check, 'gate');
  assert.equal(notice.kind, 'error');
  assert.match(notice.body, new RegExp(`The gate check failed on ${tip.slice(0, 8)} \\(0\\.6\\.\\d+\\)`));
});

for (const [failing, check] of [['build', 'build'], ['verify', 'verify'], ['selftest', 'selftest'], ['sign', 'sign'], ['spctl', 'archive'], ['stapler', 'archive']]) {
  test(`a failing ${failing} step publishes nothing and notifies (${check})`, async (t) => {
    const s = setup(t);
    gateRecord(s.builder);
    const { result, bucket, logs } = await go(s, { status: { [failing]: 2 } });
    assert.equal(result.status, 'failed');
    assert.equal(result.check, check);
    assert.deepEqual(bucket.puts, []);
    const tip = g(s.seed, 'rev-parse', 'HEAD');
    assert.deepEqual(outbox(s), [`failure-${tip}.json`]);
    assert.equal(JSON.parse(readFileSync(join(s.config.outbox, `failure-${tip}.json`), 'utf8')).check, check);
    assert.ok(logs.some(l => l.includes(`FAILED at ${check} on ${tip.slice(0, 8)}`)), 'the failure is in the log');
    assert.equal(JSON.parse(readFileSync(statePath(s.config), 'utf8')).lastFailed.check, check);
  });
}

test('an upload failing midway never writes the manifest', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const v = `0.6.${g(s.seed, 'rev-list', '--count', '--first-parent', 'HEAD')}`;
  const { result, bucket } = await go(s, { failOn: `builds/${v}/notes.json` });
  assert.equal(result.status, 'failed');
  assert.equal(result.check, 'upload');
  assert.deepEqual(bucket.puts, [`builds/${v}/Hollow.app.tar.gz`, `builds/${v}/Hollow.app.tar.gz.sig`, `builds/${v}/manifest.json`]);
  assert.ok(!bucket.objects.has('channels/nightly.json'), 'no manifest');
  assert.equal(outbox(s).length, 1);
});

test('no new commit, or a tip that already failed, exits quietly; --retry tries again', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  assert.equal((await go(s)).result.status, 'published');
  const again = await go(s);
  assert.deepEqual([again.result.status, again.result.reason], ['skipped', 'no new commit']);
  assert.deepEqual(again.events, []);

  mergePr(s.seed, 14, 'Third', 'three');
  g(s.seed, 'push', '-q', 'origin', 'main');
  assert.equal((await go(s, { status: { build: 1 } })).result.check, 'build');
  const quiet = await go(s);
  assert.deepEqual([quiet.result.status, quiet.result.reason], ['skipped', 'already failed']);
  assert.deepEqual(quiet.events, []);
  assert.equal((await go(s, { argv: ['--retry'] })).result.status, 'published');
});

test('a second runner is refused while the first holds the lock; a stale lock is taken over', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const lock = lockPath(s.config);
  mkdirSync(dirname(lock), { recursive: true });
  writeFileSync(lock, JSON.stringify({ pid: 999, startedAt: '2026-11-15T11:59:00Z', host: 'test-host' }));
  const events = [];
  const runner = fakeRun({ env: s.env, events });
  const busy = await runNightly({ config: s.config, env: s.env, deps: deps({ env: s.env, bucket: fakeBucket(), runner, isAlive: (pid) => pid === 999 }), log() {} });
  assert.deepEqual([busy.status, busy.reason], ['skipped', 'locked']);
  assert.deepEqual(events, []);
  assert.equal(JSON.parse(readFileSync(lock, 'utf8')).pid, 999, 'the holder keeps its lock');

  // Dead pid: taken over.
  const logs = [];
  const dead = await go(s, { logs });
  assert.equal(dead.result.status, 'published');
  assert.ok(logs.some(l => l.includes('took over a stale lock (pid 999')));

  // Alive but older than the stale limit: still refused (a live pid is never taken over).
  const now = Date.parse('2026-11-15T12:00:00Z');
  writeFileSync(lock, JSON.stringify({ pid: 999, startedAt: '2026-11-15T01:00:00Z', host: 'test-host' }));
  const old = acquireLock(lock, { pid: 1, now, isAlive: () => true, staleMs: 6 * 3600_000, host: 'test-host', startTimeOf: () => null });
  assert.deepEqual([old.ok, old.old], [false, true]);
  // Another host's lock: its pid can't be checked here, so it is refused.
  writeFileSync(lock, JSON.stringify({ pid: 999, startedAt: '2026-11-15T01:00:00Z', host: 'other-mac' }));
  assert.equal(acquireLock(lock, { pid: 1, now, isAlive: () => false, staleMs: 6 * 3600_000, host: 'test-host' }).ok, false);
  // An unreadable lock is taken only once it is older than the stale limit.
  writeFileSync(lock, '{half');
  assert.equal(acquireLock(lock, { pid: 1, now: Date.now(), isAlive: () => false, staleMs: 6 * 3600_000, host: 'test-host' }).ok, false);
  assert.equal(acquireLock(lock, { pid: 1, now: Date.now() + 7 * 3600_000, isAlive: () => false, staleMs: 6 * 3600_000, host: 'test-host' }).ok, true);
});

test('a lock replaced between judging it stale and renaming it is put back and left alone', (t) => {
  const s = setup(t);
  const lock = lockPath(s.config);
  mkdirSync(dirname(lock), { recursive: true });
  writeFileSync(lock, JSON.stringify({ pid: 999, startedAt: '2026-11-15T11:59:00Z', host: 'test-host' }));
  const fresh = JSON.stringify({ pid: 777, startedAt: '2026-11-15T12:00:00Z', host: 'test-host', startTimeOf: () => null });
  const got = acquireLock(lock, { pid: 1, now: Date.parse('2026-11-15T12:00:01Z'), isAlive: (pid) => pid === 777, staleMs: 3600_000, host: 'test-host', startTimeOf: () => null, afterJudge: () => writeFileSync(lock, fresh) });
  assert.equal(got.ok, false);
  assert.equal(readFileSync(lock, 'utf8'), fresh, 'the other runner keeps its lock');
  assert.deepEqual(readdirSync(dirname(lock)).filter(n => n.includes('stale')), [], 'nothing left aside');
});

test('the second Nightly\'s notes cover only what merged since the first', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const first = await go(s);
  g(s.seed, 'checkout', '-q', '-b', 'b20');
  put(s.seed, 'docs/WHATS_NEW.md', "# What's new\n\n### Shiny thing\n\n- **What:** shines.\n\n### Old thing\n\n- **What:** old.\n");
  g(s.seed, 'commit', '-q', '-am', 'shiny');
  g(s.seed, 'checkout', '-q', 'main');
  g(s.seed, 'merge', '--no-ff', '-q', 'b20', '-m', 'Merge #20: Make it shine');
  g(s.seed, 'push', '-q', 'origin', 'main');
  const second = await go(s);
  assert.equal(second.result.status, 'published');
  const notes = JSON.parse(await second.bucket.get(`builds/${second.result.version}/notes.json`));
  assert.deepEqual(notes.pullRequests.map(p => p.number), [20]);
  assert.deepEqual(notes.whatsNew.map(e => e.title), ['Shiny thing']);
  assert.deepEqual(notes.previous, { version: first.result.version, commit: first.result.commit });
});

test('prune keeps the newest Nightlies, what each channel points at and the one before it, and recent Mains', async () => {
  const bucket = fakeBucket();
  for (let n = 1; n <= 30; n++) for (const f of ['Hollow.app.tar.gz', 'notes.json']) bucket.objects.set(`builds/0.6.${n}/${f}`, Buffer.from('x'));
  bucket.objects.set('channels/nightly.json', Buffer.from(JSON.stringify({ version: '0.6.30' })));
  bucket.objects.set('channels/main.json', Buffer.from(JSON.stringify({ version: '0.6.5' })));
  bucket.objects.set('people.json', Buffer.from('{}'));
  // Main history is in publish order (oldest first); the previous Main is 0.6.2, not 0.6.4.
  const { deleted, kept } = await pruneBuilds({ uploader: bucket, keepNightly: 20, keepMain: 1, mainHistory: ['0.6.1', '0.6.2', '0.6.5'] });
  assert.deepEqual(kept.sort((a, b) => Number(a.split('.')[2]) - Number(b.split('.')[2])),
    ['0.6.2', '0.6.5', ...Array.from({ length: 20 }, (_, i) => `0.6.${i + 11}`)]);
  assert.ok(deleted.includes('0.6.1') && deleted.includes('0.6.4') && deleted.includes('0.6.10'));
  assert.ok(bucket.objects.has('people.json') && bucket.objects.has('channels/main.json'));
  assert.ok(!bucket.objects.has('builds/0.6.3/notes.json'));

  const broken = fakeBucket();
  broken.objects.set('builds/0.6.1/notes.json', Buffer.from('x'));
  broken.objects.set('channels/main.json', Buffer.from('not json'));
  await assert.rejects(pruneBuilds({ uploader: broken, keepNightly: 0 }), /nothing pruned/);
  assert.ok(broken.objects.has('builds/0.6.1/notes.json'));

  // Main exists, but its history is missing, unreadable or lacks it: nothing deleted.
  const mainOnly = fakeBucket();
  mainOnly.objects.set('builds/0.6.1/notes.json', Buffer.from('x'));
  mainOnly.objects.set('channels/main.json', Buffer.from(JSON.stringify({ version: '0.6.1' })));
  await assert.rejects(pruneBuilds({ uploader: mainOnly, keepNightly: 0, mainHistory: null }), /missing or unreadable; nothing pruned/);
  await assert.rejects(pruneBuilds({ uploader: mainOnly, keepNightly: 0, mainHistory: { not: 'a list' } }), /missing or unreadable/);
  await assert.rejects(pruneBuilds({ uploader: mainOnly, keepNightly: 0, mainHistory: ['0.6.9'] }), /does not hold 0\.6\.1/);
  assert.ok(mainOnly.objects.has('builds/0.6.1/notes.json'));
});

test('the builder reads Main history for prune from its state folder', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  for (let i = 0; i < 5; i++) { put(s.seed, `d${i}.txt`, 'x'); g(s.seed, 'add', '.'); g(s.seed, 'commit', '-q', '-m', `direct ${i}`); }
  g(s.seed, 'push', '-q', 'origin', 'main');
  mkdirSync(s.stateDir, { recursive: true });
  writeFileSync(mainHistoryPath(s.config), JSON.stringify(['0.6.1']));
  const bucket = fakeBucket();
  for (const n of [1, 2, 6]) bucket.objects.set(`builds/0.6.${n}/notes.json`, Buffer.from('x'));
  s.config.keepNightly = 1;
  const { result } = await go(s, { bucket });
  assert.deepEqual([result.status, result.version], ['published', '0.6.7']);
  assert.ok(bucket.objects.has('builds/0.6.1/notes.json'), 'a recent Main is kept');
  assert.ok(bucket.objects.has('builds/0.6.6/notes.json'), 'the Nightly before the current one is kept');
  assert.ok(!bucket.objects.has('builds/0.6.2/notes.json'), 'an old Nightly goes');
  assert.ok(bucket.objects.has('builds/0.6.7/Hollow.app.tar.gz'));
});

test('Main history entries written by promotion read as versions for prune (row U8)', () => {
  assert.deepEqual(mainVersions(['0.6.1', { version: '0.6.4', promotedAt: '2026-11-15T12:00:00Z', from: '0.6.1', action: 'publish' }]), ['0.6.1', '0.6.4']);
  assert.equal(mainVersions({ not: 'a list' }), null);
  assert.equal(mainVersions([{ version: 'nope' }]), null);
  assert.equal(mainVersions(null), null);
});

test('prune takes the previous Main from the replayed history, not a withdrawn build (row U8)', async () => {
  const pub = (v) => ({ version: v, action: 'publish' });
  // 0.6.30 was published and withdrawn: Main is 0.6.20 and the Main before it 0.6.10.
  const history = [pub('0.6.10'), pub('0.6.20'), pub('0.6.30'), { version: '0.6.20', action: 'withdraw' }];
  assert.deepEqual(mainVersions(history), ['0.6.10', '0.6.20']);
  assert.equal(mainVersions([pub('0.6.10'), { version: '0.6.9', action: 'withdraw' }]), null, 'a withdraw that does not replay');
  const bucket = fakeBucket();
  for (const n of [10, 20, 30]) bucket.objects.set(`builds/0.6.${n}/notes.json`, Buffer.from('x'));
  bucket.objects.set('channels/main.json', Buffer.from(JSON.stringify({ version: '0.6.20' })));
  const { deleted, kept } = await pruneBuilds({ uploader: bucket, keepNightly: 0, keepMain: 1, mainHistory: mainVersions(history) });
  assert.deepEqual(deleted, ['0.6.30'], 'the withdrawn build is not "the previous Main"');
  assert.deepEqual(kept, ['0.6.20', '0.6.10']);
});

test('the builder does not prune while a publish to Main holds the promote lock (row U8)', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  for (let i = 0; i < 5; i++) { put(s.seed, `d${i}.txt`, 'x'); g(s.seed, 'add', '.'); g(s.seed, 'commit', '-q', '-m', `direct ${i}`); }
  g(s.seed, 'push', '-q', 'origin', 'main');
  mkdirSync(s.stateDir, { recursive: true });
  writeFileSync(mainHistoryPath(s.config), JSON.stringify(['0.6.1']));
  writeFileSync(promoteLockPath(s.config), JSON.stringify({ pid: 999, startedAt: '2026-11-15T11:59:00Z', host: 'test-host' }));
  const bucket = fakeBucket();
  for (const n of [1, 2, 6]) bucket.objects.set(`builds/0.6.${n}/notes.json`, Buffer.from('x'));
  s.config.keepNightly = 1;
  const logs = [];
  const runner = fakeRun({ env: s.env, events: [] });
  const result = await runNightly({ config: s.config, env: s.env, deps: deps({ env: s.env, bucket, runner, isAlive: (pid) => pid === 999 }), log: (l) => logs.push(l) });
  assert.equal(result.status, 'published');
  assert.ok(bucket.objects.has('builds/0.6.2/notes.json'), 'nothing pruned while the lock is held');
  assert.ok(logs.some(l => /prune skipped: a publish to Main or withdraw is running/.test(l)), logs.join('\n'));
  assert.ok(existsSync(promoteLockPath(s.config)), "the publisher's lock is left alone");
});

test('secrets never reach the log or any argv, and each reaches only its own step', async (t) => {
  const s = setup(t);
  const logs = [];
  const events = [];
  const runner = fakeRun({ env: s.env, events });
  const result = await runNightly({ config: s.config, env: s.env, deps: deps({ env: s.env, bucket: fakeBucket(), runner }), log: (l) => logs.push(l) });
  assert.equal(result.status, 'published');
  const values = Object.values(SECRETS);
  const text = [...logs, ...runner.calls.map(c => `${c.cmd} ${c.args.join(' ')}`)].join('\n');
  for (const v of values) assert.ok(!text.includes(v), `${v} leaked into a log line or argv`);
  for (const call of runner.calls) {
    const has = (k) => call.env && k in call.env;
    const step = call.cmd === 'npx' ? 'sign' : call.args.join(' ').includes('build-app.mjs') ? 'build' : 'other';
    assert.equal(has('TAURI_SIGNING_PRIVATE_KEY'), step === 'sign', `updater key in ${call.cmd} ${call.args.join(' ')}`);
    assert.ok(!has('HOLLOW_R2_SECRET_ACCESS_KEY'), 'R2 credentials never reach a tool');
    assert.equal(has('APPLE_API_KEY'), step === 'build', 'the notary key only reaches build-app');
  }
  const outText = readdirSync(join(s.stateDir, 'out'), { recursive: true }).map(f => join(s.stateDir, 'out', f)).filter(f => f.endsWith('.json')).map(f => readFileSync(f, 'utf8')).join('');
  for (const v of values) assert.ok(!outText.includes(v));
});

test('a missing updater key or uploader setting fails before any build, and notifies', async (t) => {
  const s = setup(t);
  const env = { ...s.env };
  delete env.TAURI_SIGNING_PRIVATE_KEY;
  delete env.TAURI_SIGNING_PRIVATE_KEY_PATH;
  const events = [];
  const runner = fakeRun({ env, events });
  const result = await runNightly({ config: s.config, env, deps: deps({ env, bucket: fakeBucket(), runner }), log() {} });
  assert.deepEqual([result.status, result.check], ['failed', 'config']);
  assert.deepEqual(events, []);
  assert.equal(outbox(s).length, 1);

  const s2 = setup(t);
  const env2 = { ...s2.env };
  delete env2.HOLLOW_R2_ENDPOINT;
  const d = deps({ env: env2, bucket: null, runner: fakeRun({ env: env2 }) });
  const r2 = await runNightly({ config: s2.config, env: env2, deps: d, log() {} });
  assert.equal(r2.check, 'config');
  assert.match(r2.reason, /HOLLOW_R2_ENDPOINT/);
});

test('schema versions come from the source constants and a missing constant is caught', (t) => {
  const s = setup(t);
  assert.deepEqual(readSchema(s.seed), { sessions: 0, companion: 0, searchIndex: 2, embedIndex: 3, projectIndex: 1 });
  put(s.seed, 'src-tauri/src/companion_inbox.rs', 'pub(crate) const LEDGER_SCHEMA_VERSION: i64 = 2;\n');
  assert.equal(readSchema(s.seed).companion, 2);
  put(s.seed, 'src-tauri/src/session_store.rs', 'conn.pragma_update(None, "user_version", 4);\n');
  assert.throws(() => readSchema(s.seed), /has no SCHEMA_VERSION constant/);
  // The real tree matches the table.
  const repo = fileURLToPath(new URL('..', import.meta.url));
  const real = readSchema(repo);
  assert.deepEqual(Object.keys(real), SCHEMA_SOURCES.map(x => x.store));
});

test('loadConfig refuses a worktree inside the repo and relative paths', () => {
  assert.throws(() => loadConfig({ HOLLOW_NIGHTLY_REPO: '/r', HOLLOW_NIGHTLY_WORKTREE: '/r/wt', HOLLOW_NIGHTLY_STATE_DIR: '/s' }), /outside/);
  assert.throws(() => loadConfig({ HOLLOW_NIGHTLY_STATE_DIR: 'rel' }), /absolute/);
  const c = loadConfig({ HOLLOW_NIGHTLY_REPO: '/r', HOLLOW_NIGHTLY_STATE_DIR: '/s', HOLLOW_NIGHTLY_WRAP: 'hollow-slot --here' });
  assert.deepEqual([c.worktree, c.outbox, c.host, c.keepNightly, c.keepMain, c.wrap], ['/s/worktree', '/s/outbox', 'updates.example.com', 20, 10, ['hollow-slot', '--here']]);
});

test('the publish path refuses an ad hoc, self-signed or un-notarized build before anything runs', async (t) => {
  for (const [change, re] of [
    [{ HOLLOW_SIGNING_IDENTITY: '-', APPLE_API_KEY: '', APPLE_API_ISSUER: '', APPLE_API_KEY_PATH: '' }, /ad hoc/],
    [{ HOLLOW_SIGNING_IDENTITY: '' , APPLE_API_KEY: '', APPLE_API_ISSUER: '', APPLE_API_KEY_PATH: '' }, /self-signed/],
    [{ APPLE_API_KEY: '', APPLE_API_ISSUER: '', APPLE_API_KEY_PATH: '' }, /must be notarized/],
  ]) {
    const s = setup(t);
    const env = { ...s.env, ...change };
    const events = [];
    const bucket = fakeBucket({ events });
    const r = await runNightly({ config: s.config, env, deps: deps({ env, bucket, runner: fakeRun({ env, events }) }), log() {} });
    assert.deepEqual([r.status, r.check], ['failed', 'config']);
    assert.match(r.reason, re);
    assert.deepEqual(events, [], 'no tool ran and nothing was uploaded');
  }
});

test('a crash between the manifest upload and the state write is caught up, not rebuilt', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const bucket = fakeBucket();
  const first = await go(s, { bucket });
  assert.equal(first.result.status, 'published');
  // As if the run died right after PUT channels/nightly.json.
  writeFileSync(statePath(s.config), JSON.stringify({}));
  const again = await go(s, { bucket });
  assert.deepEqual([again.result.status, again.result.reason, again.result.version], ['skipped', 'already live', first.result.version]);
  assert.deepEqual(again.events, []);
  assert.equal(JSON.parse(readFileSync(statePath(s.config), 'utf8')).lastPublished.commit, first.result.commit);
});

test('the channel never moves back and the served version is never overwritten', async (t) => {
  const s = setup(t);
  gateRecord(s.builder);
  const v = `0.6.${g(s.seed, 'rev-list', '--count', '--first-parent', 'HEAD')}`;
  const newer = fakeBucket();
  newer.objects.set('channels/nightly.json', Buffer.from(JSON.stringify({ version: '0.6.999', commit: 'b'.repeat(40) })));
  const back = await go(s, { bucket: newer });
  assert.deepEqual([back.result.status, back.result.check], ['failed', 'manifest']);
  assert.deepEqual(newer.puts, []);

  // The live manifest changes to this version while the build runs: no overwrite.
  const s2 = setup(t);
  gateRecord(s2.builder);
  const racing = fakeBucket();
  let reads = 0;
  racing.get = async (key) => (key === 'channels/nightly.json' && ++reads > 1 ? JSON.stringify({ version: v, commit: 'c'.repeat(40) }) : null);
  const raced = await go(s2, { bucket: racing });
  assert.deepEqual([raced.result.status, raced.result.check], ['failed', 'upload']);
  assert.match(raced.result.reason, /refusing to overwrite/);
  assert.deepEqual(racing.puts, []);
});

test('tauri signer sign gets the key from the environment and an empty --password when none is set', () => {
  assert.deepEqual(signerArgs('/o/Hollow.app.tar.gz', { TAURI_SIGNING_PRIVATE_KEY_PATH: '/k' }), ['--no-install', 'tauri', 'signer', 'sign', '--password', '', '/o/Hollow.app.tar.gz']);
  assert.deepEqual(signerArgs('/o/Hollow.app.tar.gz', { TAURI_SIGNING_PRIVATE_KEY_PATH: '/k', TAURI_SIGNING_PRIVATE_KEY_PASSWORD: 'pw' }), ['--no-install', 'tauri', 'signer', 'sign', '/o/Hollow.app.tar.gz']);
});

const TAURI_BIN = fileURLToPath(new URL('../node_modules/.bin/tauri', import.meta.url));
test('a real tauri signer sign with a throwaway key and those arguments writes a .sig', { skip: !existsSync(TAURI_BIN) && 'no tauri CLI here' }, (t) => {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-signer-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const key = join(dir, 'updater.key');
  execFileSync(TAURI_BIN, ['signer', 'generate', '--ci', '-p', '', '-w', key], { stdio: 'ignore', env: { PATH: process.env.PATH, CI: 'true' } });
  const file = join(dir, 'Hollow.app.tar.gz');
  writeFileSync(file, 'TARBYTES');
  const env = { PATH: process.env.PATH, TAURI_SIGNING_PRIVATE_KEY_PATH: key };
  const args = signerArgs(file, env).slice(2); // drop `--no-install tauri`: run the binary itself
  execFileSync(TAURI_BIN, args, { stdio: ['ignore', 'ignore', 'ignore'], env, timeout: 60_000 });
  assert.ok(readFileSync(`${file}.sig`, 'utf8').trim().length > 40);
});

test('logged git errors lose URL credentials, query strings and tokens', () => {
  assert.equal(redact('fatal: unable to access https://x-access-token:ghs_abcdefghijkl1234@github.com/o/r.git/: 403'),
    'fatal: unable to access https://***@github.com/o/r.git/: 403');
  assert.equal(redact('see https://h.example/p?token=abc and ghp_ABCDEFGHIJKLmnop'), 'see https://h.example/p?*** and ***');
});

test('a lock whose pid now belongs to a process started at another time is taken over (pid reuse)', (t) => {
  const s = setup(t);
  const lock = lockPath(s.config);
  mkdirSync(dirname(lock), { recursive: true });
  const holder = { pid: 999, pidStart: 'Sat Nov 15 11:00:00 2026', startedAt: '2026-11-15T11:00:00Z', host: 'test-host' };
  writeFileSync(lock, JSON.stringify(holder));
  const now = Date.parse('2026-11-15T12:00:00Z');
  const opts = { pid: 1, now, isAlive: () => true, staleMs: 6 * 3600_000, host: 'test-host' };
  assert.equal(acquireLock(lock, { ...opts, startTimeOf: (pid) => (pid === 999 ? holder.pidStart : 'x') }).ok, false, 'same start time: the holder is alive');
  assert.equal(acquireLock(lock, { ...opts, startTimeOf: () => null }).ok, false, 'unknown start time: trust isAlive');
  const taken = acquireLock(lock, { ...opts, startTimeOf: (pid) => (pid === 999 ? 'Sun Nov 16 08:00:00 2026' : 'Sun Nov 16 09:00:00 2026') });
  assert.equal(taken.ok, true, 'another start time: a reused pid, so the holder is gone');
  assert.equal(JSON.parse(readFileSync(lock, 'utf8')).pidStart, 'Sun Nov 16 09:00:00 2026', 'the new lock records its own start time');
});

test('a lock file that cannot be created is a skipped run, not a crash', async (t) => {
  const s = setup(t);
  // The state folder's parent is a file, so the lock can't be created (ENOTDIR).
  const file = join(s.root, 'a-file');
  writeFileSync(file, 'x');
  const config = { ...s.config, stateDir: join(file, 'state') };
  const logs = [];
  const r = await runNightly({ config, env: s.env, deps: deps({ env: s.env, bucket: fakeBucket(), runner: fakeRun({ env: s.env }) }), log: (l) => logs.push(l) });
  assert.deepEqual([r.status, r.reason], ['skipped', 'locked']);
  assert.ok(logs.some(l => l.includes('could not take the lock')));
});

test('a timed-out step kills its whole process group, grandchildren included, and says timed out', async () => {
  const r = await runGrouped('/bin/sh', ['-c', 'sleep 300 & echo $!; wait'], { capture: true, timeout: 1000, graceMs: 2000 });
  assert.equal(r.timedOut, true);
  assert.equal(r.status, null);
  assert.match(r.error, /^timed out after 1 s$/);
  const grandchild = Number(r.stdout.trim());
  assert.ok(Number.isSafeInteger(grandchild) && grandchild > 1, `got a pid: ${r.stdout}`);
  assert.throws(() => process.kill(grandchild, 0), { code: 'ESRCH' }, 'the background sleep is gone');

  const ok = await runGrouped('/bin/sh', ['-c', 'echo hi; exit 3'], { capture: true, timeout: 60_000 });
  assert.deepEqual([ok.status, ok.stdout, ok.error, ok.timedOut], [3, 'hi\n', undefined, undefined], 'a plain failure is not a timeout');
  const killed = await runGrouped('/bin/sh', ['-c', 'kill -TERM $$'], { capture: true, timeout: 60_000 });
  assert.deepEqual([killed.status, killed.error], [null, 'killed by SIGTERM']);
});
