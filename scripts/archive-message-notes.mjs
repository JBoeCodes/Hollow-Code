import { spawnSync } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import {
  existsSync, lstatSync, mkdirSync, readFileSync, readdirSync,
  realpathSync, renameSync, writeFileSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { dirname, extname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// Keep this allowlist literal. Calendar, recordings, notes, and tasks are never
// eligible, regardless of what a config file or a caller supplies.
export const MESSAGE_FOLDERS = Object.freeze(['emails', 'teams', 'slack']);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const recoveryRoot = join(homedir(), 'Library', 'Application Support', 'Hollow', 'removed-message-notes');

function inside(parent, child) {
  const rel = relative(parent, child);
  return rel === '' || (!rel.startsWith(`..${sep}`) && rel !== '..' && !isAbsolute(rel));
}

function pathExists(path) {
  try { lstatSync(path); return true; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

function rejectSymlinkAncestors(path) {
  let current = resolve(path);
  while (true) {
    if (pathExists(current) && lstatSync(current).isSymbolicLink()) {
      throw new Error('A source or recovery path contains a symbolic link; nothing was removed.');
    }
    const parent = dirname(current);
    if (parent === current) return;
    current = parent;
  }
}

function inventory(folder, prefix = '') {
  const rootStat = lstatSync(folder);
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) {
    throw new Error('A message directory changed or became a symbolic link.');
  }
  const result = [];
  for (const name of readdirSync(folder).sort()) {
    const path = join(folder, name);
    const rel = prefix ? `${prefix}/${name}` : name;
    const stat = lstatSync(path);
    if (stat.isSymbolicLink()) throw new Error('A message folder contains a symbolic link; nothing was removed.');
    if (stat.isDirectory()) {
      result.push({ path: rel, kind: 'directory', mode: stat.mode & 0o777 });
      result.push(...inventory(path, rel));
      continue;
    }
    if (!stat.isFile()) throw new Error('A message folder contains a special file; nothing was removed.');
    if (extname(name).toLowerCase() !== '.md' && rel !== '.sync-state.json') {
      throw new Error('A message folder contains an unexpected file type; inspect it before archiving.');
    }
    result.push({
      path: rel,
      kind: 'file',
      bytes: stat.size,
      mode: stat.mode & 0o777,
      sha256: createHash('sha256').update(readFileSync(path)).digest('hex'),
    });
  }
  return result;
}

function equalInventory(expected, actual) {
  return JSON.stringify(expected) === JSON.stringify(actual);
}

export function assertHollowStopped() {
  // Match installed bundles, development binaries, and headless CLI runs.
  // pgrep checks process names, so the archiver's own argv cannot match.
  // -a: pgrep leaves out its own ancestors, and a run from a Hollow turn descends from Hollow.
  const result = spawnSync('/usr/bin/pgrep', ['-a', '-ix', 'hollow'], { encoding: 'utf8' });
  if (result.error || (result.status !== 0 && result.status !== 1)) {
    throw new Error('Could not verify that Hollow is stopped; nothing was removed.');
  }
  if (result.status === 0) throw new Error('Hollow is running. Quit it before archiving message notes.');
}

export function resolveVaultFromConfig(configPath) {
  const configured = configPath ?? process.env.HOLLOW_CONFIG;
  const candidates = configured ? [resolve(configured)] : [
    join(homedir(), '.config', 'Hollow', 'config.json'),
    join(homedir(), 'Library', 'Application Support', 'Hollow', 'config.json'),
    join(repoRoot, 'config.json'),
  ];
  const path = candidates.find(candidate => existsSync(candidate));
  if (!path) throw new Error('Hollow config was not found. Supply --config /path/to/config.json.');
  let value;
  try { value = JSON.parse(readFileSync(path, 'utf8')); }
  catch { throw new Error('Could not read a valid Hollow config; its contents were not printed.'); }
  if (typeof value?.vaultPath !== 'string' || !value.vaultPath.trim() || !isAbsolute(value.vaultPath)) {
    throw new Error('The Hollow config must contain an absolute vaultPath.');
  }
  // No token, connector settings, or any other config field leaves this function.
  return resolve(value.vaultPath);
}

/**
 * Recoverable removal by same-filesystem directory rename. There is deliberately
 * no copy-and-delete fallback: EXDEV is a safe failure with rollback.
 * Dependency injection exists for isolated tests, never as extra CLI scope.
 */
export function archiveMessageNotes({ vaultPath, apply = false, archiveRoot = recoveryRoot, assertStopped = assertHollowStopped }) {
  rejectSymlinkAncestors(vaultPath);
  const vault = realpathSync(vaultPath);
  if (!lstatSync(vault).isDirectory()) throw new Error('vaultPath is not a directory.');
  const recovery = resolve(archiveRoot);
  rejectSymlinkAncestors(recovery);
  if (inside(vault, recovery)) throw new Error('The recovery directory must be outside the vault.');

  const folders = MESSAGE_FOLDERS.flatMap(name => {
    const source = join(vault, name);
    if (!pathExists(source)) return [];
    const stat = lstatSync(source);
    if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error(`The ${name} root must be an ordinary directory.`);
    return [{ name, mode: stat.mode & 0o777, entries: inventory(source) }];
  });
  const counts = folders.map(folder => ({
    folder: folder.name,
    notes: folder.entries.filter(entry => entry.kind === 'file' && extname(entry.path).toLowerCase() === '.md').length,
    files: folder.entries.filter(entry => entry.kind === 'file').length,
    directories: folder.entries.filter(entry => entry.kind === 'directory').length,
  }));
  const summary = { mode: apply ? 'apply' : 'dry-run', vault, folders: counts, recoveryRoot: recovery };
  if (!apply || folders.length === 0) return { ...summary, status: folders.length ? 'ready' : 'nothing-to-remove' };

  assertStopped();
  mkdirSync(recovery, { recursive: true, mode: 0o700 });
  rejectSymlinkAncestors(recovery);
  const archive = join(recovery, `${new Date().toISOString().replaceAll(':', '-')}-${randomUUID().slice(0, 8)}`);
  mkdirSync(archive, { mode: 0o700 });
  const manifestPath = join(archive, 'manifest.json');
  const checksumPath = join(archive, 'manifest.sha256');
  const manifest = {
    schemaVersion: 1,
    createdAt: new Date().toISOString(),
    sourceVault: vault,
    status: 'prepared',
    folders,
    recovery: 'Quit Hollow. Restore each named folder to sourceVault only if that destination is absent. Verify every SHA-256 before restoring. Do not merge or overwrite newly created folders.',
  };
  const writeManifest = () => {
    const temporary = join(archive, `.manifest-${randomUUID()}.tmp`);
    const contents = `${JSON.stringify(manifest, null, 2)}\n`;
    writeFileSync(temporary, contents, { flag: 'wx', mode: 0o600 });
    renameSync(temporary, manifestPath);
    const checksumTemporary = join(archive, `.checksum-${randomUUID()}.tmp`);
    writeFileSync(checksumTemporary, `${createHash('sha256').update(contents).digest('hex')}  manifest.json\n`, { flag: 'wx', mode: 0o600 });
    renameSync(checksumTemporary, checksumPath);
  };
  writeManifest();
  const moved = [];
  try {
    // Recheck after the full hash pass, then move each root as one atomic unit.
    for (const folder of folders) {
      assertStopped();
      const source = join(vault, folder.name);
      if (!equalInventory(folder.entries, inventory(source))) throw new Error('Message notes changed during preparation.');
      const destination = join(archive, folder.name);
      if (pathExists(destination)) throw new Error('A recovery destination already exists.');
      renameSync(source, destination);
      moved.push(folder.name);
      manifest.movedFolders = [...moved];
      writeManifest();
    }
    assertStopped();
    for (const folder of folders) {
      if (pathExists(join(vault, folder.name))) throw new Error('A removed message folder was recreated during archiving.');
      const destination = join(archive, folder.name);
      if ((lstatSync(destination).mode & 0o777) !== folder.mode || !equalInventory(folder.entries, inventory(destination))) {
        throw new Error('Archived file integrity verification failed.');
      }
    }
    manifest.status = 'complete';
    manifest.verifiedAt = new Date().toISOString();
    writeManifest();
    return { ...summary, status: 'complete', archive, manifest: manifestPath, checksum: checksumPath };
  } catch (error) {
    const unrestored = [];
    for (const name of [...moved].reverse()) {
      try {
        if (pathExists(join(vault, name))) throw new Error('source recreated');
        renameSync(join(archive, name), join(vault, name));
      } catch { unrestored.push(name); }
    }
    manifest.status = unrestored.length ? 'recovery-required' : 'rolled-back';
    manifest.unrestoredFolders = unrestored;
    try { writeManifest(); } catch { /* the original prepared manifest still describes every file */ }
    throw new Error(`${error.message} ${unrestored.length ? 'Some folders remain in recovery; do not overwrite existing vault folders.' : 'All moved folders were restored.'} Recovery record: ${manifestPath}`);
  }
}

function main() {
  const args = process.argv.slice(2);
  let apply = false;
  let config;
  for (let index = 0; index < args.length; index += 1) {
    if (args[index] === '--apply') apply = true;
    else if (args[index] === '--config' && args[index + 1] && !args[index + 1].startsWith('--')) config = args[++index];
    else throw new Error('Usage: node scripts/archive-message-notes.mjs [--config /path/to/config.json] [--apply]');
  }
  const result = archiveMessageNotes({ vaultPath: resolveVaultFromConfig(config), apply });
  // Summaries include only counts and recovery paths; never note bodies or credentials.
  console.log(JSON.stringify(result, null, 2));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); }
  catch (error) { console.error(`[archive-message-notes] ${error.message}`); process.exitCode = 1; }
}
