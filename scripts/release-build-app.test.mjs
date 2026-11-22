// Tests for scripts/release/build-app.mjs (updates row U1). Deterministic: no
// network, no keychain, no real build. One test signs a tiny fixture bundle
// ad hoc with the real codesign in a temp folder.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  DEFAULT_TEAM_ID,
  LOCAL_IDENTITY,
  appPath,
  buildPlan,
  releaseBuild,
  resolveSigning,
  verifyApp,
} from './release/build-app.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const SECRETS = { APPLE_API_KEY: 'KEYID-SECRET-1', APPLE_API_ISSUER: 'ISSUER-SECRET-2', APPLE_API_KEY_PATH: '/secret/path/AuthKey.p8' };
const DEV_ID = 'Developer ID Application: Example Person (TEAM000000)';

function temp(t, prefix) {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), prefix));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/** A tiny app bundle whose executable answers `embed-selftest` like Hollow's. */
function fixtureApp(dir, { version = '0.6.7', provenanceVersion = version } = {}) {
  const app = join(dir, 'Hollow.app');
  mkdirSync(join(app, 'Contents', 'MacOS'), { recursive: true });
  mkdirSync(join(app, 'Contents', 'Resources'), { recursive: true });
  writeFileSync(join(app, 'Contents', 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>hollow</string>
<key>CFBundleIdentifier</key><string>com.hollow.app.fixture</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>${version}</string>
</dict></plist>
`);
  writeFileSync(join(app, 'Contents', 'MacOS', 'hollow'), '#!/bin/sh\necho "EMBED_SELFTEST_OK $(dirname "$0")/../Resources/models/bge-small-en-v1.5"\n', { mode: 0o755 });
  writeFileSync(join(app, 'Contents', 'Resources', 'build-provenance.json'), JSON.stringify({ version: provenanceVersion, sourceFingerprint: 'f'.repeat(64) }));
  return app;
}

/**
 * A fake runner: records every call and answers like the real tools would for
 * a good build. `over` replaces an answer by a key such as 'codesign -dv'.
 */
function fakeRunner({ count = '1552', team = 'TEAM000000', authority = DEV_ID, adHoc = false, version = '0.6.1552', over = {} } = {}) {
  const calls = [];
  const run = (cmd, args, opts = {}) => {
    calls.push({ cmd, args, env: opts.env, capture: opts.capture });
    const ok = (stdout = '', stderr = '') => ({ status: 0, stdout, stderr });
    const key = cmd === 'git' ? `git ${args[0]}`
      : cmd === 'npx' ? 'tauri build'
        : cmd === '/usr/bin/codesign' ? (args[0] === '-dv' ? 'codesign -dv' : 'codesign --verify')
          : cmd === '/usr/sbin/spctl' ? 'spctl'
            : cmd === '/usr/bin/xcrun' ? 'stapler'
              : cmd === '/usr/libexec/PlistBuddy' ? `plist ${args[1].replace('Print :', '')}`
                : cmd === process.execPath ? 'provenance'
                  : cmd.endsWith('/Contents/MacOS/hollow') ? 'embed' : cmd;
    if (over[key]) return over[key](args);
    switch (key) {
      case 'git rev-list': return ok(`${count}\n`);
      case 'git rev-parse': return ok('a'.repeat(40));
      case 'codesign -dv': return ok('', adHoc
        ? 'Signature=adhoc\nTeamIdentifier=not set\n'
        : `Authority=${authority}\nAuthority=Developer ID Certification Authority\nTeamIdentifier=${team}\n`);
      case 'plist CFBundleShortVersionString': return ok(`${version}\n`);
      case 'plist CFBundleExecutable': return ok('hollow\n');
      case 'embed': return ok(`EMBED_SELFTEST_OK ${cmd.replace(/hollow$/, '')}../Resources/models/bge\n`);
      default: return ok();
    }
  };
  return { run, calls, keys: () => calls.map(c => c.cmd) };
}

/** A fake target dir holding a built app with the given stamped provenance. */
function fakeTarget(t, version = '0.6.1552') {
  const target = temp(t, 'hollow-release-target-');
  const app = appPath(repo, { CARGO_TARGET_DIR: target });
  fixtureApp(join(app, '..'), { version });
  return { target, app };
}

test('identity: HOLLOW_SIGNING_IDENTITY, then APPLE_SIGNING_IDENTITY, then the local self-signed one', () => {
  assert.deepEqual(resolveSigning({}), { identity: LOCAL_IDENTITY, adHoc: false, teamId: null, notarize: false });
  assert.deepEqual(resolveSigning({ APPLE_SIGNING_IDENTITY: DEV_ID }), { identity: DEV_ID, adHoc: false, teamId: DEFAULT_TEAM_ID, notarize: false });
  assert.equal(resolveSigning({ HOLLOW_SIGNING_IDENTITY: 'Other', APPLE_SIGNING_IDENTITY: DEV_ID }).identity, 'Other');
  assert.equal(resolveSigning({ HOLLOW_SIGNING_IDENTITY: '  ', APPLE_SIGNING_IDENTITY: DEV_ID }).identity, DEV_ID);
  assert.deepEqual(resolveSigning({ HOLLOW_SIGNING_IDENTITY: '-' }), { identity: '-', adHoc: true, teamId: null, notarize: false });
});

test('team: TEAM000000 by default for a named identity, HOLLOW_TEAM_ID overrides, none for ad hoc', () => {
  assert.equal(DEFAULT_TEAM_ID, 'TEAM000000');
  assert.equal(resolveSigning({ HOLLOW_SIGNING_IDENTITY: DEV_ID, HOLLOW_TEAM_ID: 'ABCDE12345' }).teamId, 'ABCDE12345');
  assert.equal(resolveSigning({ HOLLOW_TEAM_ID: 'ABCDE12345' }).teamId, 'ABCDE12345');
  assert.throws(() => resolveSigning({ HOLLOW_SIGNING_IDENTITY: DEV_ID, HOLLOW_TEAM_ID: 'short' }), /10-character/);
  assert.throws(() => resolveSigning({ HOLLOW_SIGNING_IDENTITY: '-', HOLLOW_TEAM_ID: 'ABCDE12345' }), /ad hoc .* has no team/);
});

test('notarization: all three API variables, none, or a refusal for some; never ad hoc or self-signed', () => {
  assert.equal(resolveSigning({ HOLLOW_SIGNING_IDENTITY: DEV_ID, ...SECRETS }).notarize, true);
  assert.throws(() => resolveSigning({ HOLLOW_SIGNING_IDENTITY: DEV_ID, APPLE_API_KEY: 'x', APPLE_API_ISSUER: 'y' }), /APPLE_API_KEY_PATH not set/);
  assert.throws(() => resolveSigning({ HOLLOW_SIGNING_IDENTITY: '-', ...SECRETS }), /Developer ID .*ad hoc/);
  assert.throws(() => resolveSigning({ ...SECRETS }), /local self-signed/);
  try { resolveSigning({ HOLLOW_SIGNING_IDENTITY: '-', ...SECRETS }); } catch (e) {
    for (const v of Object.values(SECRETS)) assert.ok(!e.message.includes(v));
  }
});

test('the tauri command carries the stamp and identity; Apple ID variables never reach it', () => {
  const env = { PATH: '/usr/bin', HOLLOW_SIGNING_IDENTITY: DEV_ID, APPLE_SIGNING_IDENTITY: 'stale', APPLE_ID: 'me@example.invalid', APPLE_PASSWORD: 'pw', APPLE_TEAM_ID: 'T' };
  const signing = resolveSigning(env);
  const plan = buildPlan({ root: repo, env, version: '0.6.42', signing });
  assert.equal(plan.command, 'npx');
  assert.deepEqual(plan.args.slice(0, 6), ['--no-install', 'tauri', 'build', '--ci', '--bundles', 'app']);
  assert.deepEqual(JSON.parse(plan.args[7]), { version: '0.6.42', bundle: { macOS: { signingIdentity: DEV_ID } } });
  assert.equal(plan.env.APPLE_SIGNING_IDENTITY, DEV_ID);
  assert.equal(plan.env.HOLLOW_BUILD_VERSION, '0.6.42');
  for (const k of ['APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID', 'APPLE_API_KEY']) assert.ok(!(k in plan.env), k);
  assert.equal(buildPlan({ root: repo, env, version: '0.6.42', signing, dmg: true }).args[5], 'app,dmg');
  const notarizing = buildPlan({ root: repo, env: { ...env, ...SECRETS }, version: '0.6.42', signing: resolveSigning({ ...env, ...SECRETS }) });
  for (const [k, v] of Object.entries(SECRETS)) assert.equal(notarizing.env[k], v, `${k} passed through`);
});

test('without credentials: builds, verifies, skips spctl and stapler, says notarization is off', (t) => {
  const { target, app } = fakeTarget(t, '0.6.1552');
  const fake = fakeRunner();
  const lines = [];
  const out = releaseBuild({ root: repo, env: { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID }, run: fake.run, log: l => lines.push(l) });
  assert.equal(out.version, '0.6.1552');
  assert.equal(out.app, app);
  const tauri = fake.calls.find(c => c.cmd === 'npx');
  assert.equal(tauri.env.HOLLOW_BUILD_VERSION, '0.6.1552');
  assert.ok(!fake.keys().includes('/usr/sbin/spctl') && !fake.keys().includes('/usr/bin/xcrun'));
  assert.deepEqual(out.passed, ['codesign --verify --deep --strict', 'team TEAM000000', 'version 0.6.1552', 'source provenance', 'embed-selftest']);
  assert.ok(lines.some(l => /notarization: off/.test(l)));
});

test('with credentials: notarization path checks spctl and stapler, and no credential value is ever logged', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const fake = fakeRunner();
  const lines = [];
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID, ...SECRETS };
  const out = releaseBuild({ root: repo, env, run: fake.run, log: l => lines.push(l) });
  assert.ok(out.passed.includes('spctl --assess --type execute') && out.passed.includes('stapler validate'));
  assert.deepEqual(fake.calls.find(c => c.cmd === '/usr/bin/xcrun').args.slice(0, 2), ['stapler', 'validate']);
  assert.ok(lines.some(l => /notarization: on \(APPLE_API_KEY, APPLE_API_ISSUER, APPLE_API_KEY_PATH are set\)/.test(l)));
  const printed = lines.join('\n') + fake.calls.map(c => `${c.cmd} ${c.args.join(' ')}`).join('\n');
  for (const v of Object.values(SECRETS)) assert.ok(!printed.includes(v), `${v} leaked`);

  // A failed Gatekeeper check fails the build in words.
  const bad = fakeRunner({ over: { spctl: () => ({ status: 3, stdout: '', stderr: 'rejected' }) } });
  assert.throws(() => releaseBuild({ root: repo, env, run: bad.run, log() {} }), /Gatekeeper rejects the app .*rejected/);
});

test('each verify failure stops with its reason', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID };
  const fails = (opts, re) => assert.throws(() => releaseBuild({ root: repo, env, run: fakeRunner(opts).run, log() {} }), re);
  fails({ over: { 'tauri build': () => ({ status: 1 }) } }, /tauri build failed \(exit 1\)/);
  fails({ over: { 'codesign --verify': () => ({ status: 1, stderr: 'a sealed resource is missing or invalid' }) } }, /signature does not verify.*sealed resource/);
  fails({ team: 'ZZZZZZZZZZ' }, /team is ZZZZZZZZZZ, expected TEAM000000/);
  fails({ authority: 'Hollow Dev Signing' }, /signed by "Hollow Dev Signing", expected "Developer ID/);
  fails({ adHoc: true }, /signed ad hoc/);
  fails({ count: '1551' }, /the app says version 0\.6\.1552, expected 0\.6\.1551/);
  fails({ over: { provenance: () => ({ status: 1, stderr: 'Built app does not match this working tree.' }) } }, /not built from this tree/);
  fails({ over: { embed: () => ({ status: 0, stdout: 'EMBED_SELFTEST_OK /x/src-tauri/models/bge' }) } }, /did not load the bundled model/);
  fails({ over: { embed: () => ({ status: 1, stdout: '', stderr: 'boom' }) } }, /embed self-test failed: boom/);
  fails({ over: { 'git rev-list': () => ({ status: 128, stderr: 'fatal: bad revision' }) } }, /git rev-list .* failed/);
});

test('--ref must be the checked-out commit; unknown options are refused', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID };
  let n = 0;
  const differ = fakeRunner({ over: { 'git rev-parse': () => ({ status: 0, stdout: String(++n).repeat(40) }) } });
  assert.throws(() => releaseBuild({ root: repo, env, argv: ['--ref', 'origin/main'], run: differ.run, log() {} }), /check out the commit/);
  const same = fakeRunner();
  releaseBuild({ root: repo, env, argv: ['--ref', 'origin/main', '--verify-only'], run: same.run, log() {} });
  assert.deepEqual(same.calls.find(c => c.cmd === 'git' && c.args[0] === 'rev-list').args, ['rev-list', '--count', '--first-parent', 'origin/main', '--']);
  assert.ok(!same.keys().includes('npx'), '--verify-only builds nothing');
  assert.throws(() => releaseBuild({ root: repo, env, argv: ['--publish'], run: same.run, log() {} }), /unknown option --publish/);
  assert.throws(() => releaseBuild({ root: repo, env: { ...env, CARGO_TARGET_DIR: 'rel/target' }, run: same.run, log() {} }), /relative path/);
});

test('the nightly builder\'s split steps: --no-verify, --no-selftest, --selftest-only', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID };
  const build = fakeRunner();
  assert.deepEqual(releaseBuild({ root: repo, env, argv: ['--no-verify'], run: build.run, log() {} }).passed, []);
  assert.ok(build.keys().includes('npx') && !build.keys().includes('/usr/bin/codesign'), '--no-verify builds and checks nothing');
  const checks = fakeRunner();
  assert.deepEqual(releaseBuild({ root: repo, env, argv: ['--verify-only', '--no-selftest'], run: checks.run, log() {} }).passed,
    ['codesign --verify --deep --strict', 'team TEAM000000', 'version 0.6.1552', 'source provenance']);
  assert.ok(!checks.keys().includes('npx'));
  const self = fakeRunner();
  assert.deepEqual(releaseBuild({ root: repo, env, argv: ['--selftest-only'], run: self.run, log() {} }).passed, ['embed-selftest']);
  assert.ok(!self.keys().includes('npx') && !self.keys().includes('/usr/bin/codesign'));
  assert.throws(() => releaseBuild({ root: repo, env, argv: ['--no-verify', '--verify-only'], run: self.run, log() {} }), /cannot be combined/);
  assert.throws(() => releaseBuild({ root: repo, env, argv: ['--selftest-only', '--no-selftest'], run: self.run, log() {} }), /contradict/);
  assert.throws(() => releaseBuild({ root: repo, env, argv: ['--selftest-only', '--verify-only'], run: self.run, log() {} }), /drop --verify-only/);
});

test('a real ad hoc codesign of a fixture bundle passes the verify step; a changed file fails it', { skip: !existsSync('/usr/bin/codesign') && 'no codesign here' }, (t) => {
  const app = fixtureApp(temp(t, 'hollow-release-sign-'), { version: '0.6.7' });
  const sign = spawnSync('/usr/bin/codesign', ['-s', '-', '--force', app], { encoding: 'utf8' });
  assert.equal(sign.status, 0, sign.stderr);
  // Real codesign, PlistBuddy and executable; only the source fingerprint
  // check is answered by the fake (the fixture is not built from this tree).
  const real = (cmd, args, opts = {}) => {
    if (cmd === process.execPath) return { status: 0, stdout: '', stderr: '' };
    const r = spawnSync(cmd, args, { encoding: 'utf8', cwd: opts.cwd });
    return { status: r.error ? null : r.status, stdout: r.stdout ?? '', stderr: r.stderr ?? '', error: r.error?.message };
  };
  const expect = { identity: '-', adHoc: true, teamId: null, notarized: false, version: '0.6.7' };
  assert.deepEqual(verifyApp(app, expect, { run: real, root: repo }),
    ['codesign --verify --deep --strict', 'ad hoc signature', 'version 0.6.7', 'source provenance', 'embed-selftest']);
  assert.throws(() => verifyApp(app, { ...expect, adHoc: false, identity: DEV_ID, teamId: DEFAULT_TEAM_ID }, { run: real, root: repo }), /signed ad hoc/);
  assert.throws(() => verifyApp(app, { ...expect, version: '0.6.8' }, { run: real, root: repo }), /says version 0\.6\.7, expected 0\.6\.8/);
  writeFileSync(join(app, 'Contents', 'Resources', 'build-provenance.json'), '{"version":"0.6.7","changed":true}');
  assert.throws(() => verifyApp(app, expect, { run: real, root: repo }), /signature does not verify/);
});

test('a dirty tree is refused unless --allow-dirty, and describe says dirty', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID };
  const dirty = () => fakeRunner({ over: { 'git status': () => ({ status: 0, stdout: ' M src/App.tsx\n', stderr: '' }) } });
  const d1 = dirty();
  assert.throws(() => releaseBuild({ root: repo, env, run: d1.run, log() {} }), /uncommitted changes \(dirty\).*--allow-dirty/);
  assert.ok(!d1.keys().includes('npx'), 'nothing built');
  const lines = [];
  const d2 = dirty();
  releaseBuild({ root: repo, env, argv: ['--allow-dirty'], run: d2.run, log: l => lines.push(l) });
  assert.ok(d2.keys().includes('npx'));
  assert.ok(lines.some(l => /version 0\.6\.1552 \(dirty/.test(l)));
  const clean = [];
  releaseBuild({ root: repo, env, run: fakeRunner().run, log: l => clean.push(l) });
  assert.ok(!clean.some(l => /dirty/.test(l)));
});

test('positional arguments are refused; only --ref takes a value', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID };
  const go = (argv) => releaseBuild({ root: repo, env, argv, run: fakeRunner().run, log() {} });
  assert.throws(() => go(['main']), /unexpected argument "main"/);
  assert.throws(() => go(['--dmg', 'extra']), /unexpected argument "extra"/);
  assert.throws(() => go(['-x']), /unknown option -x/);
  assert.throws(() => go(['--ref']), /--ref needs a value/);
  go(['--ref', 'HEAD', '--dmg']);
});

test('capture runs (git, verify tools, the self-test) get no credentials; only tauri build gets the API key', (t) => {
  const { target } = fakeTarget(t, '0.6.1552');
  const fake = fakeRunner();
  const env = { CARGO_TARGET_DIR: target, HOLLOW_SIGNING_IDENTITY: DEV_ID, ...SECRETS, APPLE_ID: 'me@example.invalid', APPLE_PASSWORD: 'pw', APPLE_TEAM_ID: 'T' };
  releaseBuild({ root: repo, env, run: fake.run, log() {} });
  const captured = fake.calls.filter(c => c.capture);
  assert.ok(captured.some(c => c.cmd.endsWith('/Contents/MacOS/hollow')), 'self-test ran');
  for (const c of captured) {
    assert.ok(c.env, `${c.cmd} got an explicit env`);
    for (const k of [...Object.keys(SECRETS), 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']) assert.ok(!(k in c.env), `${c.cmd} got ${k}`);
  }
  assert.equal(fake.calls.find(c => c.cmd === 'npx').env.APPLE_API_KEY, SECRETS.APPLE_API_KEY);
});
