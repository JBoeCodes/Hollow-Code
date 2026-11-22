import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { archiveMessageNotes, resolveVaultFromConfig } from './archive-message-notes.mjs';

function fixture(t) {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-message-archive-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const vaultPath = join(root, 'vault');
  const archiveRoot = join(root, 'recovery');
  for (const folder of ['emails', 'teams', 'slack', 'calendar']) {
    mkdirSync(join(vaultPath, folder), { recursive: true, mode: 0o700 });
    writeFileSync(join(vaultPath, folder, 'sample.md'), `Sample ${folder}\n`, { mode: 0o600 });
  }
  writeFileSync(join(vaultPath, 'emails', '.sync-state.json'), '{}\n', { mode: 0o600 });
  return { root, vaultPath, archiveRoot, assertStopped() {} };
}

test('defaults to a count-only dry run with no vault or recovery writes', t => {
  const options = fixture(t);
  const result = archiveMessageNotes(options);
  assert.equal(result.mode, 'dry-run');
  assert.equal(result.status, 'ready');
  assert.deepEqual(result.folders.map(row => [row.folder, row.notes, row.files]), [['emails', 1, 2], ['teams', 1, 1], ['slack', 1, 1]]);
  assert.ok(existsSync(join(options.vaultPath, 'emails', 'sample.md')));
  assert.equal(existsSync(options.archiveRoot), false);
});

test('archives only message roots and verifies private manifest hashes while Calendar remains', t => {
  const options = fixture(t);
  const result = archiveMessageNotes({ ...options, apply: true });
  assert.equal(result.status, 'complete');
  for (const name of ['emails', 'teams', 'slack']) assert.equal(existsSync(join(options.vaultPath, name)), false);
  assert.equal(readFileSync(join(options.vaultPath, 'calendar', 'sample.md'), 'utf8'), 'Sample calendar\n');
  const manifest = JSON.parse(readFileSync(result.manifest, 'utf8'));
  assert.equal(manifest.status, 'complete');
  assert.equal(readFileSync(result.checksum, 'utf8').split(' ')[0], createHash('sha256').update(readFileSync(result.manifest)).digest('hex'));
  assert.equal(statSync(result.manifest).mode & 0o777, 0o600);
  assert.equal(statSync(result.archive).mode & 0o777, 0o700);
  assert.ok(existsSync(join(result.archive, 'emails', '.sync-state.json')));
  for (const folder of manifest.folders) {
    for (const file of folder.entries.filter(entry => entry.kind === 'file')) {
      const bytes = readFileSync(join(result.archive, folder.name, file.path));
      assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256);
    }
  }
});

test('refuses a running Hollow before creating recovery data', t => {
  const options = fixture(t);
  assert.throws(() => archiveMessageNotes({ ...options, apply: true, assertStopped() { throw new Error('Hollow is running'); } }), /Hollow is running/);
  assert.equal(existsSync(options.archiveRoot), false);
  assert.ok(existsSync(join(options.vaultPath, 'emails')));
});

test('rolls all moved directories back if a later process check fails', t => {
  const options = fixture(t);
  let checks = 0;
  assert.throws(() => archiveMessageNotes({ ...options, apply: true, assertStopped() { if (++checks === 3) throw new Error('Hollow started'); } }), /All moved folders were restored/);
  for (const name of ['emails', 'teams', 'slack', 'calendar']) assert.ok(existsSync(join(options.vaultPath, name, 'sample.md')));
  const archive = join(options.archiveRoot, readdirSync(options.archiveRoot)[0]);
  assert.equal(JSON.parse(readFileSync(join(archive, 'manifest.json'), 'utf8')).status, 'rolled-back');
});

test('refuses changed notes between inventory and apply', t => {
  const options = fixture(t);
  let checks = 0;
  assert.throws(() => archiveMessageNotes({ ...options, apply: true, assertStopped() { if (++checks === 2) writeFileSync(join(options.vaultPath, 'emails', 'sample.md'), 'New edit\n'); } }), /changed during preparation/);
  assert.equal(readFileSync(join(options.vaultPath, 'emails', 'sample.md'), 'utf8'), 'New edit\n');
});

test('refuses unknown files and dangling symlinks without removing anything', t => {
  const options = fixture(t);
  const extra = join(options.vaultPath, 'emails', 'unrelated.txt');
  writeFileSync(extra, 'Keep me');
  assert.throws(() => archiveMessageNotes({ ...options, apply: true }), /unexpected file type/);
  rmSync(extra);
  symlinkSync(join(options.root, 'missing'), join(options.vaultPath, 'teams', 'dangling.md'));
  assert.throws(() => archiveMessageNotes({ ...options, apply: true }), /symbolic link/);
  assert.equal(existsSync(options.archiveRoot), false);
});

test('refuses recovery inside the vault', t => {
  const options = fixture(t);
  assert.throws(() => archiveMessageNotes({ ...options, apply: true, archiveRoot: join(options.vaultPath, '.recovery') }), /outside the vault/);
});

test('reads only an absolute vaultPath from config and never exposes secret text on parse failure', t => {
  const options = fixture(t);
  const config = join(options.root, 'config.json');
  writeFileSync(config, JSON.stringify({ vaultPath: options.vaultPath, slackToken: 'private-example' }));
  assert.equal(resolveVaultFromConfig(config), options.vaultPath);
  writeFileSync(config, '{"private-example"');
  assert.throws(() => resolveVaultFromConfig(config), error => !error.message.includes('private-example') && /valid Hollow config/.test(error.message));
});
