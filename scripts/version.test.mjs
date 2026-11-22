import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { STAMP_SERIES, TAURI_VERSION_REF, baselineNote, buildVersion, bumpVersion, checkVersions, readVersions, stampVersion } from './version.mjs';

const repo = fileURLToPath(new URL('..', import.meta.url));
const FILES = ['package.json', 'package-lock.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock'];

/** A scratch copy of the five real version-carrying files. */
function scratch() {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-version-'));
  mkdirSync(join(dir, 'src-tauri'));
  for (const f of FILES) copyFileSync(join(repo, f), join(dir, f));
  return dir;
}
const read = (dir, f) => readFileSync(join(dir, f), 'utf8');
const edit = (dir, f, from, to) => {
  const text = read(dir, f);
  assert.ok(from.test(text), `${f} has ${from}`);
  writeFileSync(join(dir, f), text.replace(from, to));
};
/** Lines that differ between two texts of equal line count. */
function changedLines(a, b) {
  const la = a.split('\n');
  const lb = b.split('\n');
  assert.equal(la.length, lb.length);
  return la.flatMap((line, i) => (line === lb[i] ? [] : [[line, lb[i]]]));
}

test('the repo has one version source and no drift', () => {
  assert.deepEqual(checkVersions(repo), []);
  const v = readVersions(repo);
  assert.equal(v['src-tauri/tauri.conf.json'], TAURI_VERSION_REF);
  assert.match(v['package.json'], /^\d+\.\d+\.\d+$/);
});

test('check flags a literal version in tauri.conf.json (the old second source)', () => {
  const dir = scratch();
  try {
    const current = readVersions(dir)['package.json'];
    edit(dir, 'src-tauri/tauri.conf.json', /"version": "[^"]*"/, `"version": "${current}"`);
    const problems = checkVersions(dir);
    assert.equal(problems.length, 1);
    assert.match(problems[0], /tauri\.conf\.json: version must be "\.\.\/package\.json"/);
  } finally { rmSync(dir, { recursive: true }); }
});

test('check flags each Cargo and npm-lock copy that drifts', () => {
  const dir = scratch();
  try {
    edit(dir, 'src-tauri/Cargo.toml', /^version = "[^"]+"$/m, 'version = "9.9.9"');
    edit(dir, 'src-tauri/Cargo.lock', /(name = "hollow"\nversion = ")[^"]+"/, '$19.9.8"');
    edit(dir, 'package-lock.json', /^ {2}"version": "[^"]+"/m, '  "version": "9.9.7"');
    const problems = checkVersions(dir).join('\n');
    assert.match(problems, /src-tauri\/Cargo\.toml: "9\.9\.9"/);
    assert.match(problems, /Cargo\.lock hollow package: "9\.9\.8"/);
    assert.match(problems, /package-lock\.json: "9\.9\.7"/);
    assert.doesNotMatch(problems, /root package/);
  } finally { rmSync(dir, { recursive: true }); }
});

test('bump rewrites exactly the version fields and leaves tauri.conf.json alone', () => {
  const dir = scratch();
  try {
    const before = Object.fromEntries(FILES.map(f => [f, read(dir, f)]));
    const current = readVersions(dir)['package.json'];
    const [maj, min, pat] = current.split('.').map(Number);
    const next = `${maj}.${min}.${pat + 1}`;
    const written = bumpVersion(dir, next);
    assert.deepEqual(written.sort(), ['package-lock.json', 'package.json', 'src-tauri/Cargo.lock', 'src-tauri/Cargo.toml']);
    assert.deepEqual(checkVersions(dir), []);
    assert.equal(readVersions(dir)['package.json'], next);
    assert.equal(read(dir, 'src-tauri/tauri.conf.json'), before['src-tauri/tauri.conf.json']);
    const counts = { 'package.json': 1, 'package-lock.json': 2, 'src-tauri/Cargo.toml': 1, 'src-tauri/Cargo.lock': 1 };
    for (const [f, n] of Object.entries(counts)) {
      const diff = changedLines(before[f], read(dir, f));
      assert.equal(diff.length, n, f);
      for (const [a, b] of diff) {
        assert.ok(a.includes(current) && b === a.replace(current, next), `${f}: ${a} -> ${b}`);
      }
    }
    // Same version again re-syncs and writes nothing.
    assert.deepEqual(bumpVersion(dir, next), []);
  } finally { rmSync(dir, { recursive: true }); }
});

test('bump points a literal tauri.conf.json version back at package.json', () => {
  const dir = scratch();
  try {
    edit(dir, 'src-tauri/tauri.conf.json', /"version": "[^"]*"/, '"version": "0.0.1"');
    bumpVersion(dir, readVersions(dir)['package.json']);
    assert.equal(JSON.parse(read(dir, 'src-tauri/tauri.conf.json')).version, TAURI_VERSION_REF);
    assert.deepEqual(checkVersions(dir), []);
  } finally { rmSync(dir, { recursive: true }); }
});

test('bump refuses a malformed or backwards version and writes nothing', () => {
  const dir = scratch();
  try {
    const before = Object.fromEntries(FILES.map(f => [f, read(dir, f)]));
    for (const bad of [undefined, '', '1.2', 'v1.2.3', '1.2.3-beta', '01.2.3']) {
      assert.throws(() => bumpVersion(dir, bad), /x\.y\.z/, String(bad));
    }
    assert.throws(() => bumpVersion(dir, '0.0.0'), /backwards/);
    for (const f of FILES) assert.equal(read(dir, f), before[f], f);
  } finally { rmSync(dir, { recursive: true }); }
});

test('bump checks every file before writing any: a broken Cargo copy leaves no half bump', () => {
  for (const [f, from, to, err] of [
    ['src-tauri/Cargo.toml', /^version = "[^"]+"$/m, 'version.workspace = true', /Cargo\.toml: no version in \[package\]/],
    ['src-tauri/Cargo.lock', /name = "hollow"\n/, 'name = "hollow-renamed"\n', /Cargo\.lock: no \[\[package\]\] entry for hollow/],
  ]) {
    const dir = scratch();
    try {
      edit(dir, f, from, to);
      const before = Object.fromEntries(FILES.map(g => [g, read(dir, g)]));
      const [maj, min, pat] = readVersions(dir)['package.json'].split('.').map(Number);
      assert.throws(() => bumpVersion(dir, `${maj}.${min}.${pat + 1}`), err, f);
      for (const g of FILES) assert.equal(read(dir, g), before[g], `${f} broken: ${g} must be untouched`);
    } finally { rmSync(dir, { recursive: true }); }
  }
});

test('committed versions never use the 0.6.<n> stamp series: check flags and bump refuses 0.6.x', () => {
  const dir = scratch();
  try {
    assert.throws(() => bumpVersion(dir, '0.6.0'), /stamp/);
    assert.throws(() => bumpVersion(dir, '0.6.1552'), /stamp/);
    assert.deepEqual(checkVersions(dir), []);
    edit(dir, 'package.json', /"version": "[^"]+"/, '"version": "0.6.3"');
    assert.match(checkVersions(dir).join('\n'), /package\.json: 0\.6\.3 collides with the 0\.6\.<n> release stamp/);
    // 0.7 and later are not refused, only noted: the baseline rule doesn't cover them.
    assert.equal(baselineNote('0.5.15'), null);
    assert.equal(baselineNote('0.6.2'), null);
    assert.match(baselineNote('0.7.0'), /does not cover it/);
    assert.match(baselineNote('1.0.0'), /does not cover it/);
  } finally { rmSync(dir, { recursive: true }); }
});

test('stampVersion is 0.6.<n> for a whole count above zero and refuses anything else', () => {
  assert.equal(STAMP_SERIES, '0.6');
  assert.equal(stampVersion('1552\n'), '0.6.1552');
  assert.equal(stampVersion(7), '0.6.7');
  for (const bad of ['0', '', '-3', '1.5', '12abc', 'fatal: bad revision', ' ', undefined]) {
    assert.throws(() => stampVersion(bad), /whole number/, String(bad));
  }
});

test('buildVersion counts the first-parent line, not every commit, in a repo with merges', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-stamp-'));
  const env = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't@example.invalid', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't@example.invalid' };
  const git = (...args) => execFileSync('git', args, { cwd: dir, env, encoding: 'utf8' }).trim();
  try {
    git('init', '-q', '-b', 'main');
    const commit = (name) => { writeFileSync(join(dir, name), name); git('add', name); git('commit', '-q', '-m', name); };
    commit('a'); commit('b');
    git('checkout', '-q', '-b', 'feature');
    commit('c'); commit('d'); commit('e');
    git('checkout', '-q', 'main');
    commit('f');
    git('merge', '-q', '--no-ff', '-m', 'merge feature', 'feature');
    // a, b, f, merge on main's own line; c, d, e only through the merge.
    assert.equal(git('rev-list', '--count', 'HEAD'), '7');
    assert.equal(buildVersion(dir), '0.6.4');
    assert.equal(buildVersion(dir, 'feature'), '0.6.5');
    assert.equal(buildVersion(dir, 'HEAD~1'), '0.6.3');
    assert.throws(() => buildVersion(dir, '--all'), /bad ref/);
    assert.throws(() => buildVersion(dir, 'no-such-branch'));
    // The stamp reads git only: the tree stays clean.
    assert.equal(git('status', '--porcelain'), '');
    // A shallow clone has too few commits to count: refused in words.
    const shallow = mkdtempSync(join(tmpdir(), 'hollow-stamp-shallow-'));
    try {
      execFileSync('git', ['clone', '-q', '--depth', '1', `file://${dir}`, shallow], { env, encoding: 'utf8' });
      assert.throws(() => buildVersion(shallow), /shallow clone/);
    } finally { rmSync(shallow, { recursive: true, force: true }); }
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
