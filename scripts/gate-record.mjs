// Record a passing gate where removing its worktree can't lose it (overnight
// 10-12/13, S12). `npm run gate` ends with this step, so it runs only when
// every check passed. It writes `<git common dir>/hollow-gate-records/
// <sourceFingerprint>.json`: shared by every worktree of the checkout, never
// committed, and still there after the worktree that ran the gate is removed.
// Install readiness (src-tauri/src/install_ready.rs) finds origin/main's tree
// there by fingerprint, so a gated branch counts for the merge commit that
// carries exactly its files. On 10-12 `e410ec8f` was installed with only a
// tree match because the gated worktree had been deleted.
//
// `npm run gate` runs it twice: `--start` first (a snapshot of the tree, per
// checkout), then plain at the end, which records only when it runs inside
// `npm run gate` (npm sets `npm_lifecycle_event`), the start snapshot exists,
// and the tree at the start, the build's provenance and the tree at the end
// are one clean tree. The halves run on their own, or this script by hand,
// record nothing. It never fails the gate: anything it can't vouch for, or
// any error of its own, is reported and not recorded.
import { execFileSync } from 'node:child_process';
import { mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const RECORDS_DIR = 'hollow-gate-records';
/** The most records kept (the oldest go first). */
export const KEEP = 300;
const FINGERPRINT = /^[0-9a-f]{64}$/;

/** `{ record }` for a passing gate on a clean, unchanged tree, else `{ skip: reason }`. */
export function gateRecord({ inGate, start, provenance, current, dirty, now }) {
  if (!inGate) return { skip: 'only `npm run gate` records (not its halves, not this script by hand)' };
  if (!start?.sourceFingerprint) return { skip: "the gate's start wasn't recorded" };
  if (!provenance?.sourceFingerprint) return { skip: 'no src-tauri/build-provenance.json (the gate builds first)' };
  if (dirty || provenance.dirty) return { skip: 'the tree has uncommitted changes: commit, then run the gate' };
  if (!FINGERPRINT.test(provenance.sourceFingerprint)) return { skip: 'build-provenance.json has no valid fingerprint' };
  if (start.dirty) return { skip: 'the tree had uncommitted changes when the gate started' };
  if (start.sourceFingerprint !== current.sourceFingerprint || current.sourceFingerprint !== provenance.sourceFingerprint) {
    return { skip: 'files changed during the gate, so it tested another tree' };
  }
  if (start.commit !== current.commit || current.commit !== provenance.commit) return { skip: 'HEAD moved during the gate' };
  const { sourceFingerprint, commit, checkout, generatedAt, inputCount, version } = provenance;
  return {
    record: { sourceFingerprint, commit, checkout, generatedAt, inputCount, version, dirty: false, gate: 'passed', gateCommand: 'npm run gate', recordedAt: now },
  };
}

/** Write `record` as `<fingerprint>.json` in `dir` (atomically); keep the newest `keep`. */
export function writeRecord(dir, record, keep = KEEP) {
  mkdirSync(dir, { recursive: true });
  const path = join(dir, `${record.sourceFingerprint}.json`);
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(record, null, 2)}\n`);
  renameSync(tmp, path);
  const records = readdirSync(dir)
    .filter(name => FINGERPRINT.test(name.replace(/\.json$/, '')) && name.endsWith('.json'))
    .map(name => {
      try {
        return { name, at: statSync(join(dir, name)).mtimeMs };
      } catch {
        return null; // another gate pruned it
      }
    })
    .filter(Boolean)
    .sort((a, b) => b.at - a.at || a.name.localeCompare(b.name));
  for (const { name } of records.slice(keep)) {
    if (join(dir, name) !== path) rmSync(join(dir, name), { force: true });
  }
  return path;
}

/** Where `checkout`'s start snapshot is kept (one per checkout: two worktrees may gate at once). */
export function startPath(dir, checkout) {
  return join(dir, 'start', `${createHash('sha256').update(checkout).digest('hex').slice(0, 16)}.json`);
}

/** The line for a PR body or lane report. */
export function prLine(record) {
  return `Gate: \`${record.gateCommand ?? 'npm run gate'}\` passed on clean \`${record.commit.slice(0, 8)}\`; gate fingerprint \`${record.sourceFingerprint}\`.`;
}

const isMain = () => {
  try {
    return resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
};

if (process.argv[1] && isMain()) {
  try {
    const root = resolve(fileURLToPath(new URL('..', import.meta.url)));
    const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
    const dir = join(resolve(root, git('rev-parse', '--git-common-dir')), RECORDS_DIR);
    const snapshot = () => JSON.parse(execFileSync(process.execPath, [join(root, 'scripts/source-provenance.mjs'), '--print'], { cwd: root, encoding: 'utf8' }));
    const start = startPath(dir, root);
    if (process.argv[2] === '--start') {
      mkdirSync(dirname(start), { recursive: true });
      writeFileSync(start, `${JSON.stringify({ ...snapshot(), checkout: root, at: new Date().toISOString() })}\n`);
    } else {
      const read = path => {
        try {
          return JSON.parse(readFileSync(path, 'utf8'));
        } catch {
          return null;
        }
      };
      const current = snapshot();
      const inGate = process.env.npm_lifecycle_event === 'gate';
      const outcome = gateRecord({
        inGate,
        start: read(start),
        provenance: read(join(root, 'src-tauri/build-provenance.json')),
        current,
        dirty: current.dirty,
        now: new Date().toISOString(),
      });
      // One start, one record: the gate's end consumes its snapshot (a run
      // by hand, e.g. from the gate's own script tests, leaves it alone).
      if (inGate) rmSync(start, { force: true });
      if (outcome.skip) {
        console.log(`[gate-record] not recorded: ${outcome.skip}.`);
      } else {
        const path = writeRecord(dir, outcome.record);
        console.log(`[gate-record] recorded ${path}`);
        console.log(`[gate-record] ${prLine(outcome.record)}`);
      }
    }
  } catch (e) {
    console.log(`[gate-record] not recorded: ${e?.message ?? e}`);
  }
}
