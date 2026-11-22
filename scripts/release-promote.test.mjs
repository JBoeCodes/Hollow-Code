// Tests for scripts/release/promote.mjs (updates row U8): Publish to Main and
// Withdraw. Deterministic: an in-memory bucket, a temp state folder, a fake
// git and a fixed clock. No network, no R2, no real builder.
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { parseManifest } from '../infra/updates-worker/src/manifest.js';
import { buildManifest, loadConfig, mainHistoryPath, mainVersions } from './release/nightly.mjs';
import { main, mainStack, parseArgs, promoteLockPath, readHistory } from './release/promote.mjs';

const NOW = Date.parse('2026-11-15T12:00:00Z');
const commitOf = (n) => n.toString(16).padStart(40, 'c');
const sigOf = (v) => `SIG-${v}`;

/** A bucket in memory; records every write in order. */
function fakeBucket() {
  const objects = new Map();
  const writes = [];
  return {
    objects,
    writes,
    async put(key, bytes) { writes.push(key); objects.set(key, Buffer.from(bytes).toString('utf8')); },
    async get(key) { return objects.has(key) ? objects.get(key) : null; },
    async list(prefix) { return [...objects.keys()].filter(k => k.startsWith(prefix)); },
    async delete(key) { writes.push(`delete ${key}`); objects.delete(key); },
  };
}

/** A Nightly as nightly.mjs publishes it: files, the record, notes. */
function addBuild(bucket, n, { record = true, sig = null, days = 0 } = {}) {
  const v = `0.6.${n}`;
  const m = buildManifest({ version: v, notes: `Nightly notes for ${v}`, pubDate: new Date(NOW - days * 86_400_000).toISOString().replace(/\.\d{3}Z$/, 'Z'), signature: sigOf(v), host: 'updates.example.test', commit: commitOf(n), buildId: `nightly-${n}`, schema: { sessions: 0 } });
  const text = `${JSON.stringify(m, null, 2)}\n`;
  bucket.objects.set(`builds/${v}/Hollow.app.tar.gz`, 'TAR');
  bucket.objects.set(`builds/${v}/Hollow.app.tar.gz.sig`, `${sig ?? sigOf(v)}\n`);
  bucket.objects.set(`builds/${v}/notes.json`, '{}');
  if (record) bucket.objects.set(`builds/${v}/manifest.json`, text);
  return text;
}

function setup(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-promote-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const stateDir = join(root, 'state');
  mkdirSync(stateDir, { recursive: true });
  const config = loadConfig({ HOLLOW_NIGHTLY_REPO: join(root, 'repo'), HOLLOW_NIGHTLY_STATE_DIR: stateDir, HOLLOW_UPDATES_HOST: 'updates.example.test' });
  const bucket = fakeBucket();
  return { root, config, bucket, history: mainHistoryPath(config) };
}

/** A git that answers collectNotes with `count` merged PRs between any two commits. */
function fakeGit(count = 2) {
  return (args) => {
    if (args[0] === 'merge-base') return '';
    if (args[0] === 'show') return '';
    if (args[0] === 'log') {
      return Array.from({ length: count }, (_, i) => `${commitOf(900 + i)}\x1fMerge pull request #${100 + i} from me/b\x1f2026-11-14T00:00:00Z\x1fChange ${i}\x1e`).join('');
    }
    throw new Error(`unexpected git ${args.join(' ')}`);
  };
}

const run = (s, argv, extra = {}) => main({ argv, config: s.config, uploader: s.bucket, git: fakeGit(), now: () => NOW, pid: 4242, host: 'test-host', log() {}, ...extra });

test('promote copies the build record to channels/main.json, verifies first, writes it last and appends the history', async (t) => {
  const s = setup(t);
  addBuild(s.bucket, 10);
  const { code, result } = await run(s, ['promote', '0.6.10', '--expect-commit', commitOf(10)]);
  assert.equal(code, 0, JSON.stringify(result));
  assert.deepEqual(s.bucket.writes, ['channels/main.json'], 'one write: the channel manifest, no build file');
  const served = parseManifest(s.bucket.objects.get('channels/main.json'), 'main');
  assert.ok(served, 'the Worker would serve it');
  assert.equal(served.version, '0.6.10');
  assert.equal(served.platforms['darwin-aarch64'].signature, sigOf('0.6.10'), 'the same signed bytes');
  assert.equal(served.platforms['darwin-aarch64'].url, 'https://updates.example.test/builds/0.6.10/Hollow.app.tar.gz');
  assert.match(served.notes, /#100 Change 0/, "Main's notes are what's new since the previous Main");
  const history = JSON.parse(readFileSync(s.history, 'utf8'));
  assert.deepEqual(history, [{ version: '0.6.10', commit: commitOf(10), promotedAt: '2026-11-15T12:00:00Z', from: null, action: 'publish', by: 'test-host' }]);
  assert.deepEqual(mainVersions(history), ['0.6.10'], 'nightly.mjs prunes by it');
  assert.ok(!existsSync(promoteLockPath(s.config)), 'the lock is released');
});

test('promote refuses before writing anything: missing files, a bad signature, no record, the wrong commit', async (t) => {
  const s = setup(t);
  addBuild(s.bucket, 11, { sig: 'SOMETHING-ELSE' });
  addBuild(s.bucket, 12, { record: false });
  addBuild(s.bucket, 13);
  s.bucket.objects.delete('builds/0.6.13/notes.json');
  addBuild(s.bucket, 14);
  const cases = [
    [['promote', '0.6.11'], 'signature'],
    [['promote', '0.6.12'], 'no-record'],
    [['promote', '0.6.13'], 'missing-file'],
    [['promote', '0.6.14', '--expect-commit', commitOf(99)], 'commit'],
    [['promote', '0.6.99'], 'no-record'],
    [['promote', '0.6.1/../x'], 'usage'],
  ];
  for (const [argv, why] of cases) {
    const { code, result } = await run(s, argv);
    assert.equal(code, 1, argv.join(' '));
    assert.equal(result.ok, false);
    assert.equal(result.code, why, `${argv.join(' ')}: ${result.error}`);
  }
  assert.deepEqual(s.bucket.writes, [], 'nothing was written');
  assert.ok(!existsSync(s.history), 'no history entry');
  const usage = await run(s, ['promote']);
  assert.equal(usage.code, 2);
});

test('promote refuses the live Main and an older one without --force-older', async (t) => {
  const s = setup(t);
  for (const n of [10, 20]) addBuild(s.bucket, n);
  assert.equal((await run(s, ['promote', '0.6.20'])).code, 0);
  const same = await run(s, ['promote', '0.6.20']);
  assert.equal(same.result.code, 'unchanged');
  const older = await run(s, ['promote', '0.6.10']);
  assert.equal(older.result.code, 'older');
  assert.match(older.result.error, /Withdraw/);
  assert.deepEqual(s.bucket.writes, ['channels/main.json']);
  const forced = await run(s, ['promote', '0.6.10', '--force-older']);
  assert.equal(forced.code, 0, JSON.stringify(forced.result));
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.10');
  assert.deepEqual(readHistory(s.history).map(e => [e.version, e.from]), [['0.6.20', null], ['0.6.10', '0.6.20']]);
});

test('withdraw points Main back at the previous Main, twice in a row, and refuses with none', async (t) => {
  const s = setup(t);
  for (const n of [10, 20, 30]) addBuild(s.bucket, n);
  assert.equal((await run(s, ['withdraw'])).result.code, 'no-main');
  for (const v of ['0.6.10', '0.6.20', '0.6.30']) assert.equal((await run(s, ['promote', v])).code, 0);
  const stale = await run(s, ['withdraw', '--expect-version', '0.6.20']);
  assert.equal(stale.result.code, 'moved', 'the caller looked before Main moved');
  const first = await run(s, ['withdraw', '--expect-version', '0.6.30']);
  assert.equal(first.code, 0, JSON.stringify(first.result));
  assert.deepEqual([first.result.version, first.result.from], ['0.6.20', '0.6.30']);
  assert.ok(parseManifest(s.bucket.objects.get('channels/main.json'), 'main'));
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.20');
  const second = await run(s, ['withdraw']);
  assert.equal(second.result.version, '0.6.10');
  const none = await run(s, ['withdraw']);
  assert.equal(none.result.code, 'no-previous');
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.10', 'Main is never emptied');
  const history = readHistory(s.history);
  assert.deepEqual(history.map(e => `${e.action} ${e.version}`), ['publish 0.6.10', 'publish 0.6.20', 'publish 0.6.30', 'withdraw 0.6.20', 'withdraw 0.6.10']);
  assert.deepEqual(mainStack(history).map(e => e.version), ['0.6.10']);
});

test('withdraw refuses when main.json is no kept build the history can account for, or the earlier build was pruned', async (t) => {
  const s = setup(t);
  for (const n of [10, 20]) addBuild(s.bucket, n);
  for (const v of ['0.6.10', '0.6.20']) assert.equal((await run(s, ['promote', v])).code, 0);
  // Someone wrote main.json by hand: a build with no kept record.
  addBuild(s.bucket, 15, { record: false });
  s.bucket.objects.set('channels/main.json', s.bucket.objects.get('builds/0.6.10/manifest.json').replace('"nightly"', '"main"').replaceAll('0.6.10', '0.6.15'));
  const mismatch = await run(s, ['withdraw']);
  assert.equal(mismatch.result.code, 'history');
  assert.equal((await run(s, ['promote', '0.6.20'])).result.code, 'history', 'promote refuses it too');
  // Back in step, but the earlier build's record is gone.
  s.bucket.objects.set('channels/main.json', s.bucket.objects.get('builds/0.6.20/manifest.json').replace('"nightly"', '"main"'));
  s.bucket.objects.delete('builds/0.6.10/manifest.json');
  const pruned = await run(s, ['withdraw']);
  assert.equal(pruned.result.code, 'no-record');
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.20');
});

test('a crash between main.json and the history is repaired: promote of the live version adds the entry, withdraw adds it first', async (t) => {
  const s = setup(t);
  for (const n of [10, 20, 30]) addBuild(s.bucket, n);
  for (const v of ['0.6.10', '0.6.20']) assert.equal((await run(s, ['promote', v])).code, 0);
  const saved = readFileSync(s.history, 'utf8');
  assert.equal((await run(s, ['promote', '0.6.30'])).code, 0);
  writeFileSync(s.history, saved); // the crash: main.json says 0.6.30, the history ends at 0.6.20
  const writes = s.bucket.writes.length;
  const again = await run(s, ['promote', '0.6.30', '--expect-commit', commitOf(30), '--expect-main', '0.6.30']);
  assert.equal(again.code, 0, JSON.stringify(again.result));
  assert.deepEqual([again.result.action, again.result.version, again.result.from, again.result.repaired], ['publish', '0.6.30', '0.6.20', true]);
  assert.equal(s.bucket.writes.length, writes, 'nothing written to the bucket');
  assert.deepEqual(mainStack(readHistory(s.history)).map(e => e.version), ['0.6.10', '0.6.20', '0.6.30']);
  assert.equal((await run(s, ['promote', '0.6.30'])).result.code, 'unchanged', 'once in step, the live version is unchanged again');

  // The same crash, then a withdraw: the publish entry goes in first, then the withdraw.
  writeFileSync(s.history, saved);
  const back = await run(s, ['withdraw', '--expect-version', '0.6.30']);
  assert.equal(back.code, 0, JSON.stringify(back.result));
  assert.deepEqual([back.result.version, back.result.from, back.result.repaired], ['0.6.20', '0.6.30', true]);
  assert.deepEqual(readHistory(s.history).map(e => `${e.action} ${e.version}`), ['publish 0.6.10', 'publish 0.6.20', 'publish 0.6.30', 'withdraw 0.6.20']);

  // A withdraw that crashed after main.json: main serves 0.6.10, the history still ends at 0.6.20.
  const beforeWithdraw = readFileSync(s.history, 'utf8');
  assert.equal((await run(s, ['withdraw'])).code, 0);
  writeFileSync(s.history, beforeWithdraw);
  const status = await run(s, ['status']);
  assert.equal(status.result.main.version, '0.6.10');
  assert.equal(status.result.canWithdraw, false, 'status replays the missing withdraw: nothing before 0.6.10');
  const repaired = await run(s, ['promote', '0.6.10']);
  assert.equal(repaired.code, 0, JSON.stringify(repaired.result));
  assert.deepEqual([repaired.result.action, repaired.result.from], ['withdraw', '0.6.20']);
  assert.deepEqual(mainStack(readHistory(s.history)).map(e => e.version), ['0.6.10']);
});

test('a Main seeded by hand with an empty history becomes the stack base; Withdraw to it still needs its record', async (t) => {
  const s = setup(t);
  for (const n of [10, 20]) addBuild(s.bucket, n);
  // main.json written by hand for a build whose record is gone (pruned), no history yet.
  s.bucket.objects.set('channels/main.json', s.bucket.objects.get('builds/0.6.10/manifest.json').replace('"nightly"', '"main"'));
  s.bucket.objects.delete('builds/0.6.10/manifest.json');
  const status = await run(s, ['status']);
  assert.equal(status.result.main.version, '0.6.10');
  assert.equal(status.result.canWithdraw, false);
  const published = await run(s, ['promote', '0.6.20']);
  assert.equal(published.code, 0, JSON.stringify(published.result));
  assert.deepEqual(readHistory(s.history).map(e => `${e.action} ${e.version}${e.seeded ? ' seeded' : ''}`), ['publish 0.6.10 seeded', 'publish 0.6.20']);
  // Withdraw would go back to the seeded Main, whose record is gone: it refuses and Main stays.
  const back = await run(s, ['withdraw']);
  assert.equal(back.result.code, 'no-record');
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.20');
});

test('a transient read error on the live build record refuses instead of seeding Main as the stack base', async (t) => {
  const s = setup(t);
  for (const n of [10, 20]) addBuild(s.bucket, n);
  // Main serves 0.6.10 with no history: a missing record would seed it, an unreadable one must not.
  s.bucket.objects.set('channels/main.json', s.bucket.objects.get('builds/0.6.10/manifest.json').replace('"nightly"', '"main"'));
  const realGet = s.bucket.get;
  s.bucket.get = async (key) => {
    if (key === 'builds/0.6.10/manifest.json') throw new Error('GET builds/0.6.10/manifest.json failed: 503');
    return realGet(key);
  };
  const refused = await run(s, ['promote', '0.6.20']);
  assert.notEqual(refused.code, 0);
  assert.equal(refused.result.code, 'read-error');
  assert.match(refused.result.error, /couldn't be read/);
  assert.equal(existsSync(s.history), false, 'no history entry was written');
  assert.equal(JSON.parse(s.bucket.objects.get('channels/main.json')).version, '0.6.10');
  // The same call once the read works seeds and publishes as before.
  s.bucket.get = realGet;
  const ok = await run(s, ['promote', '0.6.20']);
  assert.equal(ok.code, 0, JSON.stringify(ok.result));
});

test('promote refuses when Main moved since the caller looked (--expect-main)', async (t) => {
  const s = setup(t);
  for (const n of [10, 20]) addBuild(s.bucket, n);
  assert.equal((await run(s, ['promote', '0.6.10', '--expect-main', '0.6.9'])).result.code, 'moved', 'Main was empty');
  assert.equal((await run(s, ['promote', '0.6.10', '--expect-main', 'none'])).code, 0);
  const moved = await run(s, ['promote', '0.6.20', '--expect-main', 'none']);
  assert.equal(moved.result.code, 'moved');
  assert.match(moved.result.error, /Main is 0\.6\.10 now, not empty/);
  assert.equal((await run(s, ['promote', '0.6.20', '--expect-main', '0.6.10'])).code, 0);
  assert.throws(() => parseArgs(['promote', '0.6.20', '--expect-main', 'latest']), /--expect-main/);
});

test('each release script runs from a symlinked path (/tmp is one on macOS)', (t) => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-entry-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const scripts = dirname(fileURLToPath(import.meta.url));
  const link = join(root, 'scripts');
  symlinkSync(scripts, link);
  const env = { PATH: process.env.PATH, HOME: root, TMPDIR: root };
  const node = (file, args) => spawnSync(process.execPath, [join(link, 'release', file), ...args], { env, encoding: 'utf8', timeout: 30_000 });
  const promote = node('promote.mjs', ['status']);
  assert.equal(promote.status, 2, promote.stderr);
  assert.match(promote.stdout, /^HOLLOW_PROMOTE_RESULT \{"ok":false,"code":"config"/m, 'the R2 settings are missing, and it says so');
  const nightly = node('nightly.mjs', ['--bogus']);
  assert.equal(nightly.status, 2);
  assert.match(nightly.stderr, /unknown argument --bogus/);
  const build = node('build-app.mjs', ['--bogus']);
  assert.equal(build.status, 1);
  assert.match(build.stderr, /unknown option --bogus/);
});

test('a broken history file stops a publish; another publish holding the lock makes it busy', async (t) => {
  const s = setup(t);
  addBuild(s.bucket, 10);
  writeFileSync(s.history, '{"not":"a list"}');
  assert.equal((await run(s, ['promote', '0.6.10'])).result.code, 'history');
  writeFileSync(s.history, JSON.stringify(['0.6.9']));
  assert.deepEqual(readHistory(s.history).map(e => e.version), ['0.6.9'], 'the version-string shape still reads');
  writeFileSync(promoteLockPath(s.config), JSON.stringify({ pid: 1, startedAt: '2026-11-15T11:59:00Z', host: 'test-host' }));
  const busy = await run(s, ['promote', '0.6.10'], { pid: 4243 });
  assert.equal(busy.result.code, 'busy');
  assert.deepEqual(s.bucket.writes, []);
});

test('status proposes the newest eligible build above Main, lists kept builds and says when to nudge', async (t) => {
  const s = setup(t);
  addBuild(s.bucket, 10, { days: 6 });
  addBuild(s.bucket, 20, { days: 4 });
  addBuild(s.bucket, 25, { days: 2 });
  addBuild(s.bucket, 30, { days: 0 });
  addBuild(s.bucket, 5, { record: false });
  s.bucket.objects.set('channels/nightly.json', s.bucket.objects.get('builds/0.6.30/manifest.json'));
  assert.equal((await run(s, ['promote', '0.6.10'])).code, 0);
  const { code, result } = await run(s, ['status', '--eligible', '0.6.25,0.6.20,0.6.10,0.6.99']);
  assert.equal(code, 0, JSON.stringify(result));
  assert.deepEqual(result.builds.map(b => b.version), ['0.6.30', '0.6.25', '0.6.20', '0.6.10'], 'newest first; a build with no record is not offered');
  assert.equal(result.builds[0].commit, commitOf(30));
  assert.equal(result.main.version, '0.6.10');
  assert.equal(result.nightly.version, '0.6.30');
  assert.equal(result.proposed, '0.6.25', 'the newest eligible above Main, not the newest Nightly');
  assert.ok(result.notes.pullRequests.length > 0);
  assert.equal(result.behindSince, '2026-11-11T12:00:00Z', 'the oldest build Main lacks');
  assert.equal(result.daysAhead, 4);
  assert.equal(result.nudge, true, '3 days ahead');
  assert.equal(result.canWithdraw, false, 'only one Main so far');
  const picked = await run(s, ['status', '--pick', '0.6.30']);
  assert.equal(picked.result.proposed, '0.6.30');
  const nothing = await run(s, ['status']);
  assert.equal(nothing.result.proposed, null);
  assert.match(nothing.result.proposedWhy, /pick one/);
  const listed = await run(s, ['list-nightlies']);
  assert.equal(listed.result.builds.length, 4);
  assert.deepEqual(s.bucket.writes, ['channels/main.json'], 'status and list write nothing');
});

test('status nudges at 20 changes ahead even the same day, and not when Main has the newest', async (t) => {
  const s = setup(t);
  addBuild(s.bucket, 10);
  addBuild(s.bucket, 11);
  s.bucket.objects.set('channels/nightly.json', s.bucket.objects.get('builds/0.6.11/manifest.json'));
  assert.equal((await run(s, ['promote', '0.6.10'])).code, 0);
  const few = await run(s, ['status'], { git: fakeGit(19) });
  assert.equal(few.result.changesAhead, 19);
  assert.equal(few.result.nudge, false);
  const many = await run(s, ['status'], { git: fakeGit(20) });
  assert.equal(many.result.nudge, true);
  assert.equal((await run(s, ['promote', '0.6.11'])).code, 0);
  const level = await run(s, ['status', '--eligible', '0.6.11']);
  assert.equal(level.result.nudge, false);
  assert.equal(level.result.proposed, null);
  assert.match(level.result.proposedWhy, /newest kept build/);
  assert.equal(level.result.canWithdraw, true);
  assert.equal(level.result.previousMain, '0.6.10');
});

test('arguments are checked', () => {
  assert.throws(() => parseArgs(['publish', '0.6.1']), /commands/);
  assert.throws(() => parseArgs(['promote', '0.6.1', '--expect-commit', 'abc']), /40-character/);
  assert.throws(() => parseArgs(['withdraw', '--expect-version', 'x']), /version/);
  assert.throws(() => parseArgs(['status', '--pick']), /needs a value/);
  assert.deepEqual(parseArgs(['status', '--eligible', '0.6.2, 0.6.1']).eligible, ['0.6.2', '0.6.1']);
});
