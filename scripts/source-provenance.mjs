// Fingerprint build inputs, including uncommitted/untracked source, without embedding contents.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync, realpathSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export function fingerprintTree(dir) {
  const git = (...args) => execFileSync('git', args, {cwd: dir, encoding: 'utf8'}).trim();
  const inputs = [...new Set(git('ls-files', '-co', '--exclude-standard', '-z').split('\0'))]
    .filter(path => /^(src\/|src-tauri\/src\/|scripts\/|public\/|package(-lock)?\.json$|index\.html$|vite\.config\.ts$|tsconfig\.json$|rust-toolchain\.toml$|docs\/(WHATS_NEW\.md|HOLLOW_APP_COMMAND_MAP\.tsv)$|src-tauri\/(Cargo\.(toml|lock)|tauri(\.[a-z-]+)?\.conf\.json|build\.rs|capabilities\/|icons\/))/.test(path))
    .filter(path => existsSync(resolve(dir, path))).sort();
  const hash = createHash('sha256');
  for (const path of inputs) hash.update(path).update('\0').update(readFileSync(resolve(dir, path))).update('\0');
  return { sourceFingerprint: hash.digest('hex'), inputs, inputCount: inputs.length, git };
}

const isMain = () => {
  try {
    const self = fileURLToPath(import.meta.url);
    const arg = resolve(process.argv[1]);
    return arg === self || realpathSync(arg) === realpathSync(self);
  } catch {
    return false;
  }
};

if (process.argv[1] && isMain()) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const { sourceFingerprint, inputs, git } = fingerprintTree(root);
  if (process.argv[2] === '--print') {
    // The fingerprint of the tree as it is now, written nowhere (scripts/gate-record.mjs).
    console.log(JSON.stringify({ sourceFingerprint, commit: git('rev-parse','HEAD'), dirty: Boolean(git('status','--porcelain')), inputCount: inputs.length }));
  } else if (process.argv[2] === '--check') {
    const built = JSON.parse(readFileSync(process.argv[3], 'utf8'));
    if (built.sourceFingerprint !== sourceFingerprint) throw new Error('Built app does not match this working tree. Rebuild before installing.');
    console.log(`[provenance] verified ${sourceFingerprint}`);
  } else {
    // A build on the remote runner (smart routing 7, Settings → App builds):
    // the runner checks out a snapshot commit only it has, so its HEAD and
    // clean tree would be recorded. hollow-slot's HOLLOW_REMOTE_HEAD and
    // HOLLOW_REMOTE_DIRTY name this Mac's HEAD and whether it had changes (as
    // gate-run.mjs reads them), and the build command names this Mac's
    // checkout in HOLLOW_PROVENANCE_CHECKOUT.
    const remoteHead = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(process.env.HOLLOW_REMOTE_HEAD ?? '') ? process.env.HOLLOW_REMOTE_HEAD : null;
    // A release build (scripts/release/build-app.mjs) stamps 0.6.<n> without
    // touching package.json; this file is where the running app reads it.
    const stamped = process.env.HOLLOW_BUILD_VERSION;
    if (stamped && !/^\d+\.\d+\.\d+$/.test(stamped)) throw new Error(`HOLLOW_BUILD_VERSION ${JSON.stringify(stamped)} is not x.y.z`);
    writeFileSync(resolve(root,'src-tauri/build-provenance.json'), JSON.stringify({
      sourceFingerprint, commit: remoteHead ?? git('rev-parse','HEAD'), checkout: process.env.HOLLOW_PROVENANCE_CHECKOUT || root,
      generatedAt: new Date().toISOString(), inputCount: inputs.length,
      dirty: remoteHead ? process.env.HOLLOW_REMOTE_DIRTY === '1' : Boolean(git('status','--porcelain')), version: stamped || JSON.parse(readFileSync(resolve(root,'package.json'),'utf8')).version,
    },null,2)+'\n');
    console.log(`[provenance] ${inputs.length} inputs: ${sourceFingerprint}`);
  }
}
