import assert from 'node:assert/strict';
import { cpSync, existsSync, linkSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  acceptedVersions,
  acquireLock,
  appProcesses,
  candidateStateDir,
  confirmAndRecord,
  confirmRunning,
  duplicateApps,
  install,
  installStateDir,
  layout,
  openJournal,
  recover,
  rollback,
  runInstall,
  waitUntilQuit,
} from './install-macos-app.mjs';

// Fake bundles: Contents/marker.json says who the bundle is and whether it
// verifies; Contents/MacOS/hollow carries its build tag.
function bed(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-install-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const profile = { appName: 'Hollow', bundleId: 'com.hollow.app', installDir: join(root, 'Applications') };
  mkdirSync(profile.installDir);
  const make = (path, tag, { bad = false, failWhenInstalled = false, bundleId = profile.bundleId } = {}) => {
    mkdirSync(join(path, 'Contents', 'MacOS'), { recursive: true });
    writeFileSync(join(path, 'Contents', 'MacOS', 'hollow'), tag);
    writeFileSync(join(path, 'Contents', 'marker.json'), JSON.stringify({ tag, bad, failWhenInstalled, bundleId }));
    return path;
  };
  const build = (tag, opts) => make(join(root, 'builds', tag, 'Hollow.app'), tag, opts);
  const marker = (path) => JSON.parse(readFileSync(join(path, 'Contents', 'marker.json'), 'utf8'));
  const tagOf = (path) => (existsSync(path) ? marker(path).tag : null);
  const p = layout(profile);
  const ops = {
    identity: (b) => ({ version: '0.5.3', bundleId: marker(b).bundleId, executable: 'hollow' }),
    verify(b) {
      if (marker(b).bad || (marker(b).failWhenInstalled && b === p.installed)) throw new Error(`${marker(b).tag} does not verify`);
      return `EMBED_SELFTEST_OK ${marker(b).tag}`;
    },
    sameFile: (a, b) => readFileSync(a, 'utf8') === readFileSync(b, 'utf8'),
    copy: (from, to) => cpSync(from, to, { recursive: true }),
    running: () => [],
  };
  const state = () => ({
    installed: tagOf(p.installed),
    previous: tagOf(p.previous),
    leftovers: [p.staged, p.discard, p.swap].filter(existsSync),
  });
  const quiet = { ops, log() {} };
  return { profile, p, make, build, ops, state, quiet };
}

test('installs keep exactly the last two verified bundles', (t) => {
  const { profile, build, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  assert.deepEqual(state(), { installed: 'a', previous: null, leftovers: [] });
  install(profile, build('b'), quiet);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  install(profile, build('c'), quiet);
  assert.deepEqual(state(), { installed: 'c', previous: 'b', leftovers: [] });
});

// OC11: macOS charges a helper to the app path it started from, so what an
// update does to paths is the fact the privacy note in
// docs/reference/build-and-ship.md rests on. The installed path never changes;
// the bytes it held move to a hidden folder that is not an app.
test('an update keeps the installed path and moves the old bytes to the hidden previous path', (t) => {
  const { profile, p, build, quiet } = bed(t);
  install(profile, build('a'), quiet);
  const installedBefore = p.installed;
  install(profile, build('b'), quiet);
  assert.equal(p.installed, installedBefore);
  assert.equal(JSON.parse(readFileSync(join(p.installed, 'Contents', 'marker.json'), 'utf8')).tag, 'b');
  assert.equal(JSON.parse(readFileSync(join(p.previous, 'Contents', 'marker.json'), 'utf8')).tag, 'a');
  // A process still running from 'a' now holds files under p.previous, the
  // path `lsof +D` is pointed at; and it is not a second app.
  assert.ok(p.previous.split('/').pop().startsWith('.'), p.previous);
  assert.ok(!p.previous.endsWith('.app'), p.previous);
});

test('an installed app that no longer verifies never replaces the verified previous', (t) => {
  const { profile, p, make, build, state, quiet } = bed(t);
  make(p.previous, 'good-old');
  make(p.installed, 'broken', { bad: true });
  install(profile, build('new'), quiet);
  assert.deepEqual(state(), { installed: 'new', previous: 'good-old', leftovers: [] });
});

test('a build that does not verify changes nothing on disk', (t) => {
  const { profile, build, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  assert.throws(() => install(profile, build('bad', { bad: true }), quiet), /does not verify/);
  assert.throws(() => install(profile, build('other', { bundleId: 'com.example' }), quiet), /unexpected bundle identifier/);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('a release-stamped build installs; a version that is neither package.json nor the stamp does not', (t) => {
  const { profile, build, state, quiet } = bed(t);
  // The fake bundles say 0.5.3.
  install(profile, build('a'), { ...quiet, packageVersion: '0.5.3' });
  install(profile, build('b'), { ...quiet, packageVersion: ['0.5.2', '0.5.3'] });
  assert.throws(() => install(profile, build('c'), { ...quiet, packageVersion: ['0.5.15', '0.6.1552'] }),
    /bundle version 0\.5\.3 does not match package version 0\.5\.15 or 0\.6\.1552/);
  assert.throws(() => install(profile, build('d'), { ...quiet, packageVersion: '0.5.4' }), /does not match package version 0\.5\.4$/);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('acceptedVersions: package.json, the stamp of HEAD, and the stamp of the commit the bundle records', (t) => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-accepted-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  writeFileSync(join(root, 'package.json'), JSON.stringify({ version: '0.5.15' }));
  const built = 'b'.repeat(40);
  const counts = { HEAD: '1553', [built]: '1552' };
  const calls = [];
  const git = (cwd, args) => {
    calls.push(args.join(' '));
    if (args[0] === 'rev-parse') return 'false';
    if (!(args[3] in counts)) throw new Error('fatal: bad revision');
    return `${counts[args[3]]}\n`;
  };
  assert.deepEqual(acceptedVersions(root, { git }), ['0.5.15', '0.6.1553']);
  // Built at 1552, then a docs-only commit moved HEAD to 1553.
  const bundle = join(root, 'Hollow.app');
  mkdirSync(join(bundle, 'Contents', 'Resources'), { recursive: true });
  const provenance = (commit) => writeFileSync(join(bundle, 'Contents', 'Resources', 'build-provenance.json'), JSON.stringify({ commit }));
  provenance(built);
  assert.deepEqual(acceptedVersions(root, { git, bundle }), ['0.5.15', '0.6.1553', '0.6.1552']);
  assert.ok(calls.includes(`rev-list --count --first-parent ${built} --`));
  // A commit this checkout lacks, or a malformed one, adds nothing.
  provenance('c'.repeat(40));
  assert.deepEqual(acceptedVersions(root, { git, bundle }), ['0.5.15', '0.6.1553']);
  provenance('--all');
  assert.deepEqual(acceptedVersions(root, { git, bundle }), ['0.5.15', '0.6.1553']);
  // Any other 0.6.N is still refused by install().
  const { profile, build, quiet, state } = bed(t);
  const as = (version) => ({ ...quiet, packageVersion: ['0.5.15', '0.6.1553', '0.6.1552'], ops: { ...quiet.ops, identity: (b) => ({ ...quiet.ops.identity(b), version }) } });
  assert.throws(() => install(profile, build('x'), as('0.6.1551')), /bundle version 0\.6\.1551 does not match/);
  install(profile, build('y'), as('0.6.1552'));
  assert.equal(state().installed, 'y');
  // A shallow clone has no stamp; package.json's version still installs.
  const shallow = (cwd, args) => (args[0] === 'rev-parse' ? 'true' : '9\n');
  assert.deepEqual(acceptedVersions(root, { git: shallow, bundle }), ['0.5.15']);
});

test('a failure after the swap puts both apps back', (t) => {
  const { profile, build, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  // `c` verifies as a build and staged copy, then fails its final check in place.
  assert.throws(() => install(profile, build('c', { failWhenInstalled: true }), quiet), /does not verify/);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('rollback is one step and reversible', (t) => {
  const { profile, build, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  rollback(profile, quiet);
  assert.deepEqual(state(), { installed: 'a', previous: 'b', leftovers: [] });
  rollback(profile, quiet);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('rollback refuses a previous app that does not verify, or none', (t) => {
  const { profile, p, make, build, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  assert.throws(() => rollback(profile, quiet), /no previous/);
  make(p.previous, 'rotten', { bad: true });
  assert.throws(() => rollback(profile, quiet), /does not verify/);
  assert.deepEqual(state(), { installed: 'a', previous: 'rotten', leftovers: [] });
});

test('recovery keeps the backup in the steady state (it used to delete it)', (t) => {
  const { profile, p, make, state, quiet } = bed(t);
  make(p.installed, 'b');
  make(p.previous, 'a');
  make(p.staged, 'half-copied');
  recover(profile, quiet);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('recovery finishes or undoes each interrupted rotation step', async (t) => {
  // Stopped after previous → discard.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'b');
    make(p.discard, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  }
  // Stopped after installed → previous.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.previous, 'b');
    make(p.discard, 'a');
    make(p.staged, 'c');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  }
  // Stopped after staged → installed, before its final check: verify it.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'c');
    make(p.previous, 'b');
    make(p.discard, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'c', previous: 'b', leftovers: [] });
  }
  // Same, but the new app does not verify: undo the install.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'c', { bad: true });
    make(p.previous, 'b');
    make(p.discard, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  }
  // Only a backup survived.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.previous, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'a', previous: null, leftovers: [] });
  }
});

test('recovery finishes an interrupted rollback swap', (t) => {
  {
    const { profile, p, make, state, quiet } = bed(t);
    // Stopped after installed → swap.
    make(p.swap, 'b');
    make(p.previous, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  }
  {
    const { profile, p, make, state, quiet } = bed(t);
    // Stopped after previous → installed.
    make(p.swap, 'b');
    make(p.installed, 'a');
    recover(profile, quiet);
    assert.deepEqual(state(), { installed: 'a', previous: 'b', leftovers: [] });
  }
});

test('the candidate profile never touches /Applications', async () => {
  const { PROFILES } = await import('./install-macos-app.mjs');
  assert.equal(PROFILES.everyday.installDir, '/Applications');
  assert.notEqual(PROFILES.candidate.installDir, '/Applications');
  assert.notEqual(PROFILES.candidate.bundleId, PROFILES.everyday.bundleId);
  assert.notEqual(layout(PROFILES.candidate).installed, layout(PROFILES.everyday).installed);
});

test('backups never end in .app, so macOS never registers a second app', (t) => {
  const { p } = bed(t);
  for (const path of [p.previous, p.staged, p.discard, p.swap]) assert.ok(!path.endsWith('.app'), path);
});

test('the kept backup is not a duplicate app, even while Spotlight still lists it', (t) => {
  const { profile, p } = bed(t);
  // What mdfind returned on the test bed right after the second install.
  assert.deepEqual(duplicateApps(`${p.installed}\n${p.previous}\n`, profile), []);
  assert.deepEqual(duplicateApps(`${p.installed}\n${p.swap}\n${p.discard}\n${p.staged}`, profile), []);
  // A real second copy still fails the install.
  assert.deepEqual(duplicateApps(`${p.installed}\n/Users/x/Downloads/Hollow.app\n`, profile), ['/Users/x/Downloads/Hollow.app']);
});

test('the installer waits for an app that is quitting, and refuses one that stays up', (t) => {
  const { profile, p, make, ops } = bed(t);
  make(p.installed, 'a');
  // A fake clock: each sleep advances it.
  const clock = () => { let ms = 0; return { now: () => ms, sleep: (d) => { ms += d; } }; };
  const quiet = () => {};

  // Still running for the first three polls (about 1.5 s), then gone: proceeds.
  let polls = 0;
  const quitting = { ...ops, running: () => (polls++ < 3 ? ['4034'] : []) };
  const c1 = clock();
  waitUntilQuit(quitting, profile, [p.installed], { timeoutMs: 60_000, pollMs: 500, ...c1, log: quiet });
  assert.equal(c1.now(), 1500);

  // Never quits: refused once the wait runs out, with the pid.
  const stuck = { ...ops, running: () => ['4034'] };
  const c2 = clock();
  assert.throws(() => waitUntilQuit(stuck, profile, [p.installed], { timeoutMs: 5_000, pollMs: 500, ...c2, log: quiet }), /running \(pid 4034\)/);
  assert.ok(c2.now() >= 5_000);

  // Not running at all: no wait.
  const c3 = clock();
  waitUntilQuit({ ...ops, running: () => [] }, profile, [p.installed], { ...c3, log: quiet });
  assert.equal(c3.now(), 0);
});

// Review B·F02: a Hollow opened two seconds into the 30f3896e install, after
// the installer's wait but before the swap, kept running the old binary.
test('an app opened during the install is refused right before the swap, and nothing moves', (t) => {
  const { profile, p, build, ops, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  // Nothing runs while the installer waits; Hollow opens while the current
  // app is being verified for the rotation.
  let opened = false;
  let cleaned = 0;
  const racing = {
    ...ops,
    verify(b, exe) {
      if (b === p.installed) opened = true;
      return ops.verify(b, exe);
    },
    running: (b) => (opened && b === p.installed ? ['4242'] : []),
    cleanupBuild: () => { cleaned += 1; },
  };
  const { report, failure } = runInstall(profile, build('c'), { ops: racing, log() {} });
  assert.ok(failure);
  assert.equal(report.ok, false);
  assert.match(report.reason, /opened during the install \(pid 4242\), so nothing was swapped/);
  assert.equal(report.installed, 'no', 'nothing moved, so the receipt can say the old app is still there');
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  assert.equal(cleaned, 1, 'the build bundle is still cleaned up');
});

test('rollback re-checks the app is closed right before its swap', (t) => {
  const { profile, p, build, ops, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  const running = (b) => (b === p.installed ? ['77'] : []);
  assert.throws(() => rollback(profile, { ops: { ...ops, running }, log() {} }), /opened during the rollback \(pid 77\), so nothing was swapped/);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('a successful install reports the installed commit and when it was swapped', (t) => {
  const { profile, p, build, ops } = bed(t);
  const bundle = build('a');
  mkdirSync(join(bundle, 'Contents', 'Resources'));
  writeFileSync(join(bundle, 'Contents', 'Resources', 'build-provenance.json'), JSON.stringify({ commit: '30f3896ecba42505' }));
  const { report, failure } = runInstall(profile, bundle, { ops: { ...ops, cleanupBuild() {} }, log() {}, clock: { now: () => 1_000_000 }, installId: 'i-ok' });
  assert.equal(failure, undefined);
  assert.deepEqual(report, {
    ok: true,
    reason: '',
    installed: 'yes',
    installId: 'i-ok',
    journal: join(installStateDir(profile), 'journal', 'i-ok.json'),
    steps: ['prepared', 'promoted', 'verified'],
    installedCommit: '30f3896ecba42505',
    swappedAt: 1_000_000,
  });
  assert.ok(existsSync(p.installed));
});

test('the app-process probe keeps the app and drops its subcommands', () => {
  const exe = '/Applications/Hollow.app/Contents/MacOS/hollow';
  const ps = [
    `  4034 Fri Oct  2 15:07:58 2026     ${exe}`,
    `  4040 Fri Oct  2 15:08:01 2026     ${exe} team-mcp`,
    `  4050 Fri Oct  2 15:09:12 2026     ${exe} -NSDocumentRevisionsDebugMode YES`,
    `  4060 Fri Oct  2 15:09:12 2026     /Applications/Hollow Candidate.app/Contents/MacOS/hollow`,
    '     1 Mon Sep 21 21:34:43 2026     /sbin/launchd',
  ].join('\n');
  assert.deepEqual(appProcesses(ps, exe), [
    { pid: 4034, startedAt: new Date('Fri Oct 2 15:07:58 2026').getTime() },
    { pid: 4050, startedAt: new Date('Fri Oct 2 15:09:12 2026').getTime() },
  ]);
});

// The running-app check after reopening. `procs` decides what ps shows.
function reopenBed(t, procs) {
  const { profile, p, make, ops } = bed(t);
  make(p.installed, 'new');
  mkdirSync(join(p.installed, 'Contents', 'Resources'));
  writeFileSync(join(p.installed, 'Contents', 'Resources', 'build-provenance.json'), JSON.stringify({ commit: '30f3896ecba42505c464' }));
  let ms = 0;
  const calls = { open: 0, quit: [] };
  const world = { opens: () => calls.open, quits: () => calls.quit };
  const fake = {
    ...ops,
    open: () => { calls.open += 1; },
    quit: (pids) => { calls.quit.push(pids); },
    processes: () => procs(world),
  };
  const run = (opts = {}) => confirmRunning(profile, { ops: fake, swappedAt: 10_000, expectCommit: '30f3896ecba4', now: () => ms, sleep: (d) => { ms += d; }, log() {}, ...opts });
  return { run, calls };
}

test('after the swap, a process started after it is the installed app', (t) => {
  let polls = 0;
  const { run, calls } = reopenBed(t, () => (polls++ < 2 ? [] : [{ pid: 200, startedAt: 13_000 }]));
  assert.deepEqual(run(), { runningMatches: true, relaunched: false, runningPid: 200, runningReason: '' });
  assert.equal(calls.open, 1);
  assert.deepEqual(calls.quit, []);
});

test('an app opened in the same second as the swap is the installed app (ps has whole seconds)', (t) => {
  // The 3b40f87e install: swapped at :13.296, the app opened at :13 by ps.
  const { run, calls } = reopenBed(t, () => [{ pid: 33145, startedAt: 13_000 }]);
  assert.deepEqual(run({ swappedAt: 13_296 }), { runningMatches: true, relaunched: false, runningPid: 33145, runningReason: '' });
  assert.deepEqual(calls.quit, []);
});

test('an app from before the swap is quit and reopened once', (t) => {
  // pid 100 started 2 s before the swap and exits when told; the reopen starts pid 300.
  const { run, calls } = reopenBed(t, (w) => (w.quits().length === 0 ? [{ pid: 100, startedAt: 8_000 }] : w.opens() < 2 ? [] : [{ pid: 300, startedAt: 40_000 }]));
  assert.deepEqual(run(), { runningMatches: true, relaunched: true, runningPid: 300, runningReason: '' });
  assert.equal(calls.open, 2);
  assert.deepEqual(calls.quit, [[100]]);
});

test('an app that is still from before the swap is reported, never ok and silent', (t) => {
  // Does not quit.
  {
    const { run, calls } = reopenBed(t, () => [{ pid: 100, startedAt: 8_000 }]);
    const out = run({ quitMs: 5_000 });
    assert.equal(out.runningMatches, false);
    assert.equal(out.relaunched, false);
    assert.match(out.runningReason, /pid 100, started 1970-01-01T00:00:08Z\) started before the swap at 1970-01-01T00:00:10Z and did not quit within 5 s/);
    assert.deepEqual(calls.quit, [[100]]);
  }
  // Quits, but what is running after the relaunch is still from before the swap.
  {
    const { run } = reopenBed(t, (w) => (w.quits().length === 0 ? [{ pid: 100, startedAt: 8_000 }] : w.opens() < 2 ? [] : [{ pid: 101, startedAt: 9_000 }]));
    const out = run();
    assert.equal(out.runningMatches, false);
    assert.equal(out.relaunched, true);
    assert.match(out.runningReason, /pid 101.*even after one relaunch/);
  }
  // Never starts.
  {
    const { run } = reopenBed(t, () => []);
    const out = run({ appearMs: 3_000 });
    assert.equal(out.runningMatches, false);
    assert.match(out.runningReason, /did not start within 3 s/);
  }
  // The installed app is not the receipt's commit.
  {
    const { run, calls } = reopenBed(t, () => [{ pid: 200, startedAt: 13_000 }]);
    const out = run({ expectCommit: 'd5c2ab14' });
    assert.equal(out.runningMatches, false);
    assert.match(out.runningReason, /is 30f3896e, not d5c2ab14, the commit this install was built from/);
    assert.equal(calls.open, 0);
  }
});

// ---------------------------------------------------------------- journal and lock
// Merged plan slice 6, review B·F02: each install journals its steps, one
// installer runs at a time, and recovery reads the journal.

const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));

test('each install journals prepared, promoted and verified durably, per install id', (t) => {
  const { profile, build, ops, state } = bed(t);
  const quietOps = { ops: { ...ops, cleanupBuild() {} }, log() {} };
  const first = runInstall(profile, build('a'), { ...quietOps, installId: 'one' });
  const second = runInstall(profile, build('b'), { ...quietOps, installId: 'two' });
  assert.equal(first.failure, undefined);
  assert.equal(second.failure, undefined);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
  const journal = readJson(second.report.journal);
  assert.equal(second.report.journal, join(installStateDir(profile), 'journal', 'two.json'));
  assert.equal(journal.id, 'two');
  assert.equal(journal.state, 'verified');
  assert.deepEqual(journal.steps.map((s) => s.step), ['prepared', 'promoted', 'verified']);
  assert.equal(journal.movesDone, journal.plan.moves.length);
  assert.equal(readJson(first.report.journal).id, 'one', 'one journal per install');
  assert.ok(!existsSync(join(installStateDir(profile), 'installer.lock')), 'the lock is released');
});

test('a second installer refuses while one holds the lock, and leaves the build alone', (t) => {
  const { profile, build, ops, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  const held = acquireLock(installStateDir(profile), { profile, installId: 'first', log() {} });
  let cleaned = 0;
  const { report, failure } = runInstall(profile, build('b'), { ops: { ...ops, cleanupBuild: () => { cleaned += 1; } }, log() {}, installId: 'second' });
  assert.ok(failure);
  assert.match(report.reason, new RegExp(`another Hollow installer is running \\(pid ${process.pid}, install first\\)`));
  assert.equal(report.installed, 'no');
  assert.equal(cleaned, 0, 'the build bundle may be what the first install is copying');
  assert.deepEqual(state(), { installed: 'a', previous: null, leftovers: [] });
  held.release();
  const after = runInstall(profile, build('b'), { ops: { ...ops, cleanupBuild() {} }, log() {}, installId: 'third' });
  assert.equal(after.failure, undefined);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('a lock left by an installer that died is taken over', (t) => {
  const { profile } = bed(t);
  const dir = installStateDir(profile);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'installer.lock'), JSON.stringify({ pid: 4321, installId: 'dead', what: 'install' }));
  const lines = [];
  const lock = acquireLock(dir, { profile, installId: 'next', alive: () => false, log: (l) => lines.push(l) });
  assert.match(lines.join('\n'), /stale installer lock \(pid 4321 is gone\)/);
  assert.equal(readJson(lock.path).installId, 'next');
  lock.release();
  assert.ok(!existsSync(lock.path));
});

test('two installers taking over one dead lock: the slower never deletes the faster one\'s lock', (t) => {
  const { profile } = bed(t);
  const dir = installStateDir(profile);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'installer.lock'), JSON.stringify({ pid: 4321, installId: 'dead', what: 'install' }));
  let first = null;
  // B reads the dead lock; while it decides, A takes the same lock over.
  const aliveForB = (pid) => {
    if (pid === 4321) {
      first ??= acquireLock(dir, { profile, installId: 'a', pid: 1111, alive: () => false, log() {} });
      return false;
    }
    return pid === 1111;
  };
  assert.throws(
    () => acquireLock(dir, { profile, installId: 'b', pid: 2222, alive: aliveForB, log() {} }),
    /another Hollow installer is running \(pid 1111, install a\)/,
  );
  assert.ok(first, 'A took the lock over');
  assert.equal(readJson(first.path).installId, 'a', 'A still holds the lock it took');
  assert.deepEqual(readdirSync(dir).sort(), ['installer.lock'], 'no claim or temp file is left');
  first.release();
  assert.ok(!existsSync(first.path));
});

test('a takeover claim left by an installer that died mid-takeover is cleared', (t) => {
  const { profile } = bed(t);
  const dir = installStateDir(profile);
  mkdirSync(dir, { recursive: true });
  const path = join(dir, 'installer.lock');
  writeFileSync(path, JSON.stringify({ pid: 4321, installId: 'dead', what: 'install' }));
  linkSync(path, join(dir, `installer.lock.takeover-${statSync(path).ino}`));
  const lock = acquireLock(dir, { profile, installId: 'next', alive: () => false, abandonedClaimMs: 0, sleep() {}, log() {} });
  assert.equal(readJson(lock.path).installId, 'next');
  assert.deepEqual(readdirSync(dir).sort(), ['installer.lock']);
  lock.release();
});

test('a candidate bed moved by HOLLOW_TEST_BED keeps its own lock and journals', () => {
  const home = '/Users/someone';
  const base = `${home}/Library/Application Support/com.hollow.app.candidate`;
  assert.equal(candidateStateDir(`${home}/Hollow-candidate`, home), `${base}/installer`, 'the default bed keeps its folder');
  const a = candidateStateDir('/tmp/bed-a', home);
  const b = candidateStateDir('/tmp/bed-b', home);
  assert.match(a, new RegExp(`^${base.replace(/[.]/g, '\\.')}/installer-[0-9a-f]{12}$`));
  assert.notEqual(a, b, 'two beds never share a journal folder');
  assert.equal(candidateStateDir('/tmp/bed-a/', home), a, 'a trailing slash does not make a second bed');
});

/** An interrupted install's journal, as a crash would leave it. */
function crashedJournal(profile, id, { plan, movesDone, state }) {
  const journal = openJournal(installStateDir(profile), profile, { id, source: '/builds/new/Hollow.app' });
  Object.assign(journal.record, { plan, movesDone, state, steps: [{ step: 'prepared', at: 'then' }] });
  journal.save();
  return journal.path;
}

test('recovery reads the journal: an unverifiable current app never becomes the previous one', (t) => {
  // A broken installed app (not kept) and a verified previous. The install
  // moved the broken app to discard and died before moving the new one in.
  const { profile, p, make, state, quiet } = bed(t);
  make(p.previous, 'good-old');
  make(p.discard, 'broken', { bad: true });
  make(p.staged, 'new');
  const path = crashedJournal(profile, 'stopped-mid-rotation', {
    plan: { keepCurrent: false, moves: [[p.installed, p.discard], [p.staged, p.installed]] },
    movesDone: 1,
    state: 'prepared',
  });
  recover(profile, { ...quiet, stateDir: installStateDir(profile) });
  // Guessing from the folders put good-old in place and broken in .previous.
  assert.deepEqual(state(), { installed: 'broken', previous: 'good-old', leftovers: [] });
  assert.equal(readJson(path).state, 'undone');
});

test('recovery reads the journal: a swapped-in app is verified, or undone into the right slots', (t) => {
  const plan = (p) => ({ keepCurrent: false, moves: [[p.installed, p.discard], [p.staged, p.installed]] });
  // The last rename landed but the journal write after it did not: the new app verifies, so it stands.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'new');
    make(p.previous, 'good-old');
    make(p.discard, 'broken', { bad: true });
    const path = crashedJournal(profile, 'swapped', { plan: plan(p), movesDone: 1, state: 'prepared' });
    recover(profile, { ...quiet, stateDir: installStateDir(profile) });
    assert.deepEqual(state(), { installed: 'new', previous: 'good-old', leftovers: [] });
    assert.equal(readJson(path).state, 'verified');
  }
  // Promoted, and the new app does not verify: each folder goes back where it was.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'new', { bad: true });
    make(p.previous, 'good-old');
    make(p.discard, 'broken', { bad: true });
    const path = crashedJournal(profile, 'promoted-bad', { plan: plan(p), movesDone: 2, state: 'promoted' });
    recover(profile, { ...quiet, stateDir: installStateDir(profile) });
    // Guessing from the folders swapped good-old in and kept broken as the backup.
    assert.deepEqual(state(), { installed: 'broken', previous: 'good-old', leftovers: [] });
    assert.equal(readJson(path).state, 'undone');
  }
  // Stopped before anything moved: only the staged copy goes.
  {
    const { profile, p, make, state, quiet } = bed(t);
    make(p.installed, 'b');
    make(p.previous, 'a');
    make(p.staged, 'half');
    const journal = openJournal(installStateDir(profile), profile, { id: 'started-only' });
    recover(profile, { ...quiet, stateDir: installStateDir(profile) });
    assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
    assert.equal(readJson(journal.path).state, 'undone');
  }
});

test('a failure after the swap is journaled and reported as not installed', (t) => {
  const { profile, build, ops, state, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  const { report, failure } = runInstall(profile, build('c', { failWhenInstalled: true }), { ops: { ...ops, cleanupBuild() {} }, log() {}, installId: 'bad-c' });
  assert.ok(failure);
  assert.equal(report.installed, 'no');
  assert.deepEqual(report.steps, ['prepared', 'promoted', 'failed']);
  assert.deepEqual(state(), { installed: 'b', previous: 'a', leftovers: [] });
});

test('when putting the apps back fails too, the report says unknown, never ok', (t) => {
  const { profile, p, build, ops, quiet } = bed(t);
  install(profile, build('a'), quiet);
  install(profile, build('b'), quiet);
  // Something removes the kept app while the new one fails its check in place.
  const interfering = {
    ...ops,
    cleanupBuild() {},
    verify(b, exe) {
      if (b === p.installed && readFileSync(join(b, 'Contents', 'MacOS', 'hollow'), 'utf8') === 'c') {
        rmSync(p.previous, { recursive: true, force: true });
        throw new Error('c does not verify');
      }
      return ops.verify(b, exe);
    },
  };
  const { report, failure } = runInstall(profile, build('c'), { ops: interfering, log() {}, installId: 'lost' });
  assert.ok(failure);
  assert.equal(report.ok, false);
  assert.equal(report.installed, 'unknown');
  assert.match(report.reason, /rollback also failed/);
  assert.equal(readJson(report.journal).state, 'restore-failed', 'the next recovery retries from the journal');
});

test('the reopen check adds healthy (or running-mismatch) to the journal', (t) => {
  const { profile, build, ops } = bed(t);
  const quietOps = { ops: { ...ops, cleanupBuild() {} }, log() {} };
  const { report } = runInstall(profile, build('a'), { ...quietOps, installId: 'reopen' });
  const ok = confirmAndRecord(profile, report, { log() {}, confirm: () => ({ runningMatches: true, relaunched: false, runningPid: 5, runningReason: '' }) });
  assert.equal(ok.runningMatches, true);
  assert.deepEqual(ok.steps, ['prepared', 'promoted', 'verified', 'healthy']);
  assert.equal(readJson(report.journal).state, 'healthy');

  const second = runInstall(profile, build('b'), { ...quietOps, installId: 'reopen-stale' });
  const stale = confirmAndRecord(profile, second.report, { log() {}, confirm: () => ({ runningMatches: false, relaunched: true, runningPid: 9, runningReason: 'still the old app' }) });
  assert.deepEqual(stale.steps.slice(-1), ['running-mismatch']);
  assert.equal(readJson(second.report.journal).steps.at(-1).reason, 'still the old app');

  // Another installer holds the lock: the check does not run, and says why.
  const held = acquireLock(installStateDir(profile), { profile, installId: 'other', log() {} });
  let ran = false;
  const blocked = confirmAndRecord(profile, second.report, { log() {}, confirm: () => { ran = true; return {}; } });
  held.release();
  assert.equal(ran, false);
  assert.equal(blocked.runningMatches, false);
  assert.match(blocked.runningReason, /another Hollow installer is running/);
});
