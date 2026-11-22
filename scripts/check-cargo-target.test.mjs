// The CARGO_TARGET_DIR guard (overnight A4): the rule, its command line, and
// where it runs. The Rust twin (src-tauri/src/cargo_target_rule.rs) keeps the
// same cases.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { cargoTargetProblem } from './check-cargo-target.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const read = path => readFileSync(join(root, path), 'utf8');

test('a tilde segment or a relative path is refused; absolute and unset pass', () => {
  for (const bad of [
    '~/Library/Caches/hollow-app-a1-target',
    '~',
    '~dev/target',
    '/Users/dev/w/src-tauri/~/Library/Caches/t',
    'target-2',
    './target',
    '../target',
  ]) {
    assert.ok(cargoTargetProblem(bad), bad);
  }
  assert.match(cargoTargetProblem('~/x'), /didn't expand/);
  assert.match(cargoTargetProblem('target'), /relative/);
  for (const good of [
    undefined,
    '',
    '/Users/dev/Library/Caches/hollow-overnight-a',
    '/Users/dev/Library/Mobile Documents/com~apple~CloudDocs/t',
    '/tmp/a~b',
  ]) {
    assert.equal(cargoTargetProblem(good), null, String(good));
  }
});

test('the command exits 1 with the reason for a bad value and 0 otherwise', () => {
  const run = value => {
    const env = { ...process.env };
    if (value === undefined) delete env.CARGO_TARGET_DIR;
    else env.CARGO_TARGET_DIR = value;
    return spawnSync(process.execPath, [join(root, 'scripts', 'check-cargo-target.mjs')], { env, encoding: 'utf8' });
  };
  const bad = run('~/Library/Caches/x');
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /check-cargo-target: CARGO_TARGET_DIR is "~\/Library\/Caches\/x"/);
  assert.equal(run('/tmp/hollow-target').status, 0);
  assert.equal(run(undefined).status, 0);
});

test('the command still checks when called through a symlink', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-cargo-target-'));
  try {
    const link = join(dir, 'check.mjs');
    symlinkSync(join(root, 'scripts', 'check-cargo-target.mjs'), link);
    const out = spawnSync(process.execPath, [link], {
      env: { ...process.env, CARGO_TARGET_DIR: '~/x' },
      encoding: 'utf8',
    });
    assert.equal(out.status, 1);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('the installer reports a bad target like any other refusal', () => {
  const installer = read('scripts/install-macos-app.mjs');
  const check = installer.indexOf('cargoTargetProblem(process.env.CARGO_TARGET_DIR)');
  const tryAt = installer.lastIndexOf('try {', check);
  const report = installer.indexOf("writeReport(reportPath, { ok: false", check);
  assert.ok(check > 0 && tryAt > 0 && report > check, 'the check sits in the try whose catch writes the report');
  assert.ok(installer.slice(tryAt, check).split('\n').length < 6, 'right at the start of that try');
});

test('the Rust gate and the app bundle check the target first', () => {
  const scripts = JSON.parse(read('package.json')).scripts;
  assert.match(scripts['gate:rust'], /^node scripts\/check-cargo-target\.mjs && /);
  assert.match(scripts['tauri:bundle'], /^node scripts\/check-cargo-target\.mjs && /);
});

test('the installer refuses a bad target instead of resolving it', () => {
  const installer = read('scripts/install-macos-app.mjs');
  assert.doesNotMatch(installer, /resolve\(process\.env\.CARGO_TARGET_DIR\)/);
  assert.match(installer, /cargoTargetProblem\(process\.env\.CARGO_TARGET_DIR\)/);
});

test('build.rs refuses the same targets, and git ignores stray "~" folders', () => {
  const build = read('src-tauri/build.rs');
  assert.match(build, /#\[path = "src\/cargo_target_rule\.rs"\]/);
  assert.match(build, /cargo_target_rule::problem\(/);
  assert.match(build, /cargo_target_rule::out_dir_problem\(/);
  const ignore = read('.gitignore').split('\n');
  assert.ok(ignore.includes('/~/'));
  assert.ok(ignore.includes('/src-tauri/~/'));
});
