import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { childrenAt, extraneous, installedAt, lockDrift, manifestDrift, otherPlatform } from './check-installed-lock.mjs';

const lock = {
  packages: {
    '': { name: 'hollow', dependencies: { vite: '^5.4.0' }, devDependencies: { '@scope/kit': '1.0.0' } },
    'node_modules/vite': { version: '5.4.21' },
    'node_modules/@scope/kit': { version: '1.0.0' },
    'node_modules/vite/node_modules/esbuild': { version: '0.21.0' },
    'node_modules/fsevents': { version: '2.3.3', optional: true, os: ['darwin'] },
    'node_modules/@types/trusted-types': { version: '2.0.7', optional: true },
    'node_modules/linked': { link: true },
  },
};
const all = {
  'node_modules/vite': '5.4.21',
  'node_modules/@scope/kit': '1.0.0',
  'node_modules/vite/node_modules/esbuild': '0.21.0',
  'node_modules/@types/trusted-types': '2.0.7',
};
const without = (...gone) => p => (gone.includes(p) ? null : (all[p] ?? null));

test('a node_modules matching the lock passes; another platform\'s optional build may be absent', () => {
  assert.deepEqual(lockDrift(lock, without(), 'linux', 'x64'), []);
  // On macOS, fsevents is for this machine: absent is drift.
  assert.match(lockDrift(lock, without(), 'darwin', 'arm64')[0], /fsevents is missing/);
});

test('a missing, upgraded or unversioned package is refused, optional or not', () => {
  const v = { ...all, 'node_modules/vite': '5.4.22', 'node_modules/@scope/kit': undefined };
  const problems = lockDrift(lock, p => (p in v ? v[p] : null), 'linux', 'x64');
  assert.ok(problems.some(p => /vite is 5\.4\.22, the lock says 5\.4\.21/.test(p)), problems.join('\n'));
  assert.ok(problems.some(p => /@scope\/kit is unversioned/.test(p)), problems.join('\n'));
  // An optional package with no os/cpu limit is installed by npm ci: absent is drift.
  assert.match(lockDrift(lock, without('node_modules/@types/trusted-types'), 'linux', 'x64')[0], /trusted-types is missing/);
});

test('os and cpu limits, negations included', () => {
  assert.equal(otherPlatform({ os: ['darwin'] }, 'linux', 'x64'), true);
  assert.equal(otherPlatform({ os: ['darwin'] }, 'darwin', 'arm64'), false);
  assert.equal(otherPlatform({ cpu: ['x64'] }, 'darwin', 'arm64'), true);
  assert.equal(otherPlatform({ os: ['!win32'] }, 'win32', 'x64'), true);
  assert.equal(otherPlatform({ os: ['!win32'] }, 'darwin', 'arm64'), false);
  assert.equal(otherPlatform({}, 'darwin', 'arm64'), false);
});

test('package.json out of sync with the lock is refused, as npm ci refuses it', () => {
  assert.deepEqual(manifestDrift({ dependencies: { vite: '^5.4.0' }, devDependencies: { '@scope/kit': '1.0.0' } }, lock), []);
  const drift = manifestDrift({ dependencies: { vite: '^5.5.0', 'left-pad': '1.0.0' }, devDependencies: { '@scope/kit': '1.0.0' } }, lock);
  assert.equal(drift.length, 2);
  assert.match(drift[0], /dependencies\.vite is \^5\.5\.0, the lock has \^5\.4\.0/);
  assert.match(drift[1], /dependencies\.left-pad is 1\.0\.0, the lock has absent/);
});

test('hand-installed packages are refused at the top and nested', () => {
  const kids = { '': ['vite', '@scope/kit', 'left-pad'], 'node_modules/vite': ['esbuild', 'stale'] };
  assert.deepEqual(extraneous(lock, b => kids[b] ?? []), [
    'node_modules/left-pad is installed but not in the lock',
    'node_modules/vite/node_modules/stale is installed but not in the lock',
  ]);
});

test('it reads versions and folders from disk, scopes and nesting included', () => {
  const root = mkdtempSync(join(tmpdir(), 'installed-lock-'));
  try {
    for (const [dir, version] of Object.entries(all)) {
      mkdirSync(join(root, dir), { recursive: true });
      writeFileSync(join(root, dir, 'package.json'), JSON.stringify({ version }));
    }
    mkdirSync(join(root, 'node_modules', '.bin'));
    mkdirSync(join(root, 'node_modules', 'bare'));
    writeFileSync(join(root, 'node_modules', 'bare', 'package.json'), '{}');
    assert.equal(installedAt(root)('node_modules/vite'), '5.4.21');
    assert.equal(installedAt(root)('node_modules/nope'), null);
    assert.equal(installedAt(root)('node_modules/bare'), undefined);
    assert.deepEqual(childrenAt(root)('').sort(), ['@scope/kit', '@types/trusted-types', 'bare', 'vite']);
    assert.deepEqual(childrenAt(root)('node_modules/vite'), ['esbuild']);
    assert.deepEqual(extraneous(lock, childrenAt(root)), ['node_modules/bare is installed but not in the lock']);
    assert.deepEqual(lockDrift(lock, installedAt(root), 'linux', 'x64'), []);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
