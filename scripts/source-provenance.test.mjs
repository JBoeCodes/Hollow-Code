// Source provenance fingerprinting: every compiled/tested build input — including
// rust-toolchain.toml, docs/WHATS_NEW.md, and docs/HOLLOW_APP_COMMAND_MAP.tsv —
// invalidates sourceFingerprint when edited, while unchecked docs do not.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { fingerprintTree } from './source-provenance.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const gitEnv = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1' };

function makeTempRepo() {
  const dir = mkdtempSync(join(tmpdir(), 'source-provenance-'));
  const git = (...args) => execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', ...args], {
    cwd: dir,
    encoding: 'utf8',
    env: gitEnv,
  });
  const write = (rel, content) => {
    mkdirSync(dirname(join(dir, rel)), { recursive: true });
    writeFileSync(join(dir, rel), content);
  };
  git('init', '-q', '-b', 'main');
  write('src/app.ts', 'export const a = 1;\n');
  write('package.json', '{"version":"0.5.15"}\n');
  write('rust-toolchain.toml', '[toolchain]\nchannel = "1.96.0"\n');
  write('docs/WHATS_NEW.md', '# What’s new\n');
  write('docs/HOLLOW_APP_COMMAND_MAP.tsv', 'command\ttier\n');
  write('docs/BUILD_LOG.md', '# Build log\n');
  write('README.md', '# Hollow\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'base');
  return { dir, write };
}

test('rust-toolchain.toml, docs/WHATS_NEW.md, and docs/HOLLOW_APP_COMMAND_MAP.tsv are hashed inputs', () => {
  const { dir, write } = makeTempRepo();
  try {
    const base = fingerprintTree(dir);
    assert.deepEqual(base.inputs, [
      'docs/HOLLOW_APP_COMMAND_MAP.tsv',
      'docs/WHATS_NEW.md',
      'package.json',
      'rust-toolchain.toml',
      'src/app.ts',
    ]);

    // Editing non-input docs leaves the fingerprint untouched.
    write('README.md', '# Hollow updated\n');
    write('docs/BUILD_LOG.md', '# Build log updated\n');
    assert.equal(fingerprintTree(dir).sourceFingerprint, base.sourceFingerprint);

    // Editing each of the three newly hashed inputs invalidates sourceFingerprint.
    for (const [rel, next, orig] of [
      ['rust-toolchain.toml', '[toolchain]\nchannel = "1.97.0"\n', '[toolchain]\nchannel = "1.96.0"\n'],
      ['docs/WHATS_NEW.md', '# What’s new\n\n### Feature\n', '# What’s new\n'],
      ['docs/HOLLOW_APP_COMMAND_MAP.tsv', 'command\ttier\nfoo\tread\n', 'command\ttier\n'],
    ]) {
      write(rel, next);
      assert.notEqual(fingerprintTree(dir).sourceFingerprint, base.sourceFingerprint, `editing ${rel} must change sourceFingerprint`);
      write(rel, orig);
      assert.equal(fingerprintTree(dir).sourceFingerprint, base.sourceFingerprint, `restoring ${rel} must restore sourceFingerprint`);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('install_ready.rs uses the same input filter pattern as source-provenance.mjs', () => {
  const js = readFileSync(join(root, 'scripts/source-provenance.mjs'), 'utf8');
  const rs = readFileSync(join(root, 'src-tauri/src/install_ready.rs'), 'utf8');
  const jsMatch = js.match(/\.filter\(path => \/(.+?)\/\.test\(path\)\)/);
  const rsMatch = rs.match(/Regex::new\(r"(.+?)"\)\.unwrap\(\)/);
  assert.ok(jsMatch, 'found input regex in source-provenance.mjs');
  assert.ok(rsMatch, 'found input regex in install_ready.rs');
  assert.equal(jsMatch[1].replace(/\\\//g, '/'), rsMatch[1], 'install_ready.rs::is_input must match source-provenance.mjs');
});

test('--print outputs a valid fingerprint for this repository', () => {
  const r = spawnSync(process.execPath, [join(root, 'scripts/source-provenance.mjs'), '--print'], { encoding: 'utf8' });
  assert.equal(r.status, 0, r.stderr);
  const out = JSON.parse(r.stdout);
  assert.match(out.sourceFingerprint, /^[0-9a-f]{64}$/);
  assert.ok(out.inputCount > 0);
});
