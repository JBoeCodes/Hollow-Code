// Install the built, signed Hollow bundle, keeping the previous verified one
// for a one-step rollback (merged plan step 0).
//
//   npm run tauri:install                     install <target>/release/bundle/macos/Hollow.app
//   node scripts/install-macos-app.mjs --rollback   swap the installed app with the previous one
//   node scripts/install-macos-app.mjs --status     show both bundles' provenance
//
// Hollow's self-install helper adds `--report <file>` (write what happened, why
// it failed, and whether the new app, the old one, or neither can be vouched
// for, as one JSON line) and `--install-id <id>` (its receipt id, which names
// the journal), and afterwards runs `--confirm-running <file> --expect <commit>`:
// reopen the app, check the running process started after the swap, relaunch
// once if not, and add the outcome to the same file and the journal (review B·F02).
//
// HOLLOW_INSTALL_PROFILE=candidate does the same for the side-by-side test-bed
// build (`Hollow Candidate.app`, `com.hollow.app.candidate`, installed in
// ~/Hollow-candidate or $HOLLOW_TEST_BED, never /Applications). See docs/history/TEST_BED.md.
//
// On disk, beside the installed app:
//   .<App>.app.previous    the bundle installed before this one, verified when it was rotated out
//   .<App>.app.installing  a staged copy, verified before it is swapped in
//   .<App>.app.discard     the older previous, while the rotation is in flight
//   .<App>.app.rollback    the installed app, while a rollback swap is in flight
// None ends in `.app`, so LaunchServices and Spotlight treat them as plain folders.
//
// In the app's data folder (`~/Library/Application Support/<bundle id>/installer`,
// review B·F02, merged plan slice 6; a candidate bed moved by HOLLOW_TEST_BED
// uses `installer-<hash of the bed root>` instead, see candidateStateDir):
//   installer.lock         held by the one installer allowed at a time; a second refuses
//                          (a dead holder's is taken over through installer.lock.takeover-<inode>)
//   journal/<id>.json      one per install: prepared / promoted / verified / healthy,
//                          with the planned renames and how many are done, written
//                          durably before and after each step. Recovery reads it
//                          instead of guessing from the folders on disk.
import { spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import {
  closeSync,
  existsSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
  writeSync,
} from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { cargoTargetProblem } from './check-cargo-target.mjs';
import { buildVersion } from './version.mjs';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const plistBuddy = '/usr/libexec/PlistBuddy';
const launchServices =
  '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister';

const appData = (bundleId, home = homedir(), folder = 'installer') => join(home, 'Library', 'Application Support', bundleId, folder);

/**
 * The candidate's journal and lock folder, keyed by its bed: a bed that
 * `HOLLOW_TEST_BED` moves gets `installer-<hash of the bed root>`, so two
 * beds never share a lock or replay each other's journals. The default bed
 * keeps the plain `installer` folder it always had.
 */
export function candidateStateDir(bed, home = homedir()) {
  const root = resolve(bed);
  if (root === join(home, 'Hollow-candidate')) return appData('com.hollow.app.candidate', home);
  const key = createHash('sha256').update(root).digest('hex').slice(0, 12);
  return appData('com.hollow.app.candidate', home, `installer-${key}`);
}

const candidateBed = process.env.HOLLOW_TEST_BED || join(homedir(), 'Hollow-candidate');

export const PROFILES = {
  everyday: { appName: 'Hollow', bundleId: 'com.hollow.app', installDir: '/Applications', stateDir: appData('com.hollow.app') },
  candidate: {
    appName: 'Hollow Candidate',
    bundleId: 'com.hollow.app.candidate',
    installDir: candidateBed,
    stateDir: candidateStateDir(candidateBed),
  },
};

/** The journal and lock folder. A profile without one (tests) keeps them beside its apps. */
export function installStateDir(profile) {
  return profile.stateDir ?? join(profile.installDir, `.${profile.appName}.installer`);
}

export function layout(profile) {
  const at = (name) => join(profile.installDir, name);
  return {
    installed: at(`${profile.appName}.app`),
    staged: at(`.${profile.appName}.app.installing`),
    previous: at(`.${profile.appName}.app.previous`),
    discard: at(`.${profile.appName}.app.discard`),
    swap: at(`.${profile.appName}.app.rollback`),
  };
}

function invoke(command, args, { capture = false, allowFailure = false, env } = {}) {
  const result = spawnSync(command, args, {
    encoding: 'utf8',
    stdio: capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
    env: env ? { ...process.env, ...env } : undefined,
  });

  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    const detail = capture
      ? [result.stdout, result.stderr].filter(Boolean).join('\n').trim()
      : '';
    throw new Error(
      `${command} exited ${result.status}${detail ? `\n${detail}` : ''}`,
    );
  }

  return {
    status: result.status,
    stdout: capture ? result.stdout.trim() : '',
    stderr: capture ? result.stderr.trim() : '',
  };
}

const escapeRegex = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function plistValue(bundle, key) {
  return invoke(plistBuddy, ['-c', `Print :${key}`, join(bundle, 'Contents', 'Info.plist')], {
    capture: true,
  }).stdout;
}

/** The real macOS operations. Tests swap these for fakes. */
export const macos = {
  identity(bundle) {
    return {
      version: plistValue(bundle, 'CFBundleShortVersionString'),
      bundleId: plistValue(bundle, 'CFBundleIdentifier'),
      executable: plistValue(bundle, 'CFBundleExecutable'),
    };
  },
  /** codesign, then the embed self-test. Throws with the reason. */
  verify(bundle, executable) {
    invoke('/usr/bin/codesign', ['--verify', '--deep', '--strict', '--verbose=2', bundle]);
    const out = invoke(join(bundle, 'Contents', 'MacOS', executable), ['embed-selftest'], {
      capture: true,
    }).stdout;
    if (!out.startsWith('EMBED_SELFTEST_OK ')) throw new Error(`embed self-test failed: ${out}`);
    return out;
  },
  sameFile(a, b) {
    return invoke('/usr/bin/cmp', ['-s', a, b], { allowFailure: true }).status === 0;
  },
  copy(from, to) {
    invoke('/usr/bin/ditto', ['--rsrc', '--extattr', '--acl', from, to]);
  },
  /** pids running `<bundle>/Contents/MacOS/<exe>`; throws when unknown. */
  running(bundle, executable) {
    const pattern = `^${escapeRegex(join(bundle, 'Contents', 'MacOS', executable))}( |$)`;
    // -a: macOS pgrep leaves out its own ancestors, and an install started
    // from a Hollow turn (or by Hollow's self-install helper) descends from Hollow.
    const found = invoke('/usr/bin/pgrep', ['-a', '-f', pattern], { capture: true, allowFailure: true });
    if (found.status === 1) return [];
    if (found.status === 0) return found.stdout.split('\n').filter(Boolean);
    throw new Error(`could not determine whether ${bundle} is running`);
  },
  /**
   * The app processes (no subcommand such as `team-mcp`) running
   * `<bundle>/Contents/MacOS/<exe>`, with their start times (whole seconds).
   * `ps -ax` lists every process, ancestors included.
   */
  processes(bundle, executable) {
    const exe = join(bundle, 'Contents', 'MacOS', executable);
    const out = invoke('/bin/ps', ['-axww', '-o', 'pid=,lstart=,args='], { capture: true, env: { LC_ALL: 'C' } }).stdout;
    return appProcesses(out, exe);
  },
  open(bundle) {
    invoke('/usr/bin/open', ['-a', bundle]);
  },
  /** A normal TERM, never KILL. */
  quit(pids) {
    for (const pid of pids) {
      try {
        process.kill(Number(pid), 'SIGTERM');
      } catch (error) {
        if (error.code !== 'ESRCH') throw error;
      }
    }
  },
  checkProvenance(bundle) {
    invoke(process.execPath, [
      join(repoRoot, 'scripts', 'source-provenance.mjs'),
      '--check',
      join(bundle, 'Contents', 'Resources', 'build-provenance.json'),
    ]);
  },
  /** Unregister and delete the build bundle; fail on a duplicate indexed app. */
  cleanupBuild(bundle, profile) {
    if (existsSync(launchServices)) {
      const unregistered = invoke(launchServices, ['-u', bundle], { capture: true, allowFailure: true });
      if (unregistered.status !== 0) {
        console.warn(`[install-app] LaunchServices unregister returned ${unregistered.status}; deleting the bundle anyway.`);
      }
    }
    rmSync(bundle, { recursive: true, force: true });
    if (existsSync(bundle)) throw new Error(`temporary build bundle still exists: ${bundle}`);
    const indexed = invoke('/usr/bin/mdfind', [`kMDItemCFBundleIdentifier == '${profile.bundleId}'`], {
      capture: true,
    }).stdout;
    const duplicates = duplicateApps(indexed, profile);
    if (duplicates.length > 0) {
      throw new Error(`macOS still indexes duplicate ${profile.appName} apps:\n${duplicates.join('\n')}`);
    }
  },
};

/**
 * The indexed apps that are a real second copy: anything but the installed
 * app and the installer's own side folders. Spotlight keeps listing
 * `.previous` for a while after the rename that made it one, because it was
 * indexed while still named `<App>.app`; LaunchServices does not register it
 * (it does not end in `.app`), so it is not a duplicate. Found on the
 * candidate test bed, 2026-10-02: the second install reported failure after
 * it had succeeded.
 */
export function duplicateApps(mdfindOutput, profile) {
  const p = layout(profile);
  const ours = new Set([p.installed, p.previous, p.staged, p.discard, p.swap]);
  return mdfindOutput
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((path) => !ours.has(path));
}

/** `ps -o pid=,lstart=,args=` lines → the app processes of `exe`. */
export function appProcesses(psOutput, exe) {
  const found = [];
  for (const line of psOutput.split('\n')) {
    const m = line.match(/^\s*(\d+)\s+(\w{3}\s+\w{3}\s+\d+\s+\d\d:\d\d:\d\d\s+\d{4})\s+(.*)$/);
    if (!m) continue;
    const args = m[3].trim();
    if (args !== exe && !args.startsWith(`${exe} -`)) continue;
    const startedAt = Date.parse(m[2]);
    if (Number.isFinite(startedAt)) found.push({ pid: Number(m[1]), startedAt });
  }
  return found;
}

/** The full commit in the bundle's embedded provenance, or ''. */
export function commitOf(bundle) {
  try {
    return String(JSON.parse(readFileSync(join(bundle, 'Contents', 'Resources', 'build-provenance.json'), 'utf8')).commit ?? '');
  } catch {
    return '';
  }
}

function provenanceOf(bundle) {
  try {
    const p = JSON.parse(readFileSync(join(bundle, 'Contents', 'Resources', 'build-provenance.json'), 'utf8'));
    return `${String(p.commit).slice(0, 8)}${p.dirty ? ' (dirty)' : ''} fingerprint ${String(p.sourceFingerprint).slice(0, 12)}…`;
  } catch {
    return 'unknown provenance';
  }
}

/** Verified now? Returns the self-test line, or null with the reason logged. */
function verified(ops, bundle, profile, log) {
  try {
    const id = ops.identity(bundle);
    if (id.bundleId !== profile.bundleId) throw new Error(`unexpected bundle identifier ${id.bundleId}`);
    return ops.verify(bundle, id.executable);
  } catch (error) {
    log(`[install-app] ${bundle} did not verify: ${error.message.split('\n')[0]}`);
    return null;
  }
}

// ------------------------------------------------------------ journal & lock

/** Journal states after which nothing is in flight. */
const SETTLED = new Set(['verified', 'healthy', 'running-mismatch', 'failed', 'undone']);
const isoMs = (ms) => new Date(ms).toISOString();

/** Write `text` so a crash leaves the old file or the new one: temp file, fsync, rename. */
function writeDurably(path, text) {
  const tmp = `${path}.tmp`;
  const fd = openSync(tmp, 'w', 0o600);
  try {
    writeSync(fd, text);
    fsyncSync(fd);
  } finally {
    closeSync(fd);
  }
  renameSync(tmp, path);
  try {
    const dir = openSync(dirname(path), 'r');
    try {
      fsyncSync(dir);
    } finally {
      closeSync(dir);
    }
  } catch {
    // The rename is already atomic; a folder fsync is extra durability where supported.
  }
}

export const safeInstallId = (id) => String(id ?? '').replace(/[^A-Za-z0-9._-]/g, '_').slice(0, 80);

/** `20261002-150755-d7007a7b`: local time, then 4 random bytes. */
export function newInstallId(now = Date.now) {
  const d = new Date(now());
  const two = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}${two(d.getMonth() + 1)}${two(d.getDate())}-${two(d.getHours())}${two(d.getMinutes())}${two(d.getSeconds())}-${randomBytes(4).toString('hex')}`;
}

/**
 * One install's journal. `path` null keeps it in memory (direct `install()`
 * calls in tests); the steps and the rename plan are the same either way.
 */
export class Journal {
  constructor(path, record, now = Date.now) {
    this.path = path;
    this.record = record;
    this.now = now;
  }
  save() {
    if (this.path) writeDurably(this.path, `${JSON.stringify(this.record, null, 2)}\n`);
  }
  /** Save, but never let a journal write stop a rename that puts the apps back. */
  trySave() {
    try {
      this.save();
    } catch {
      // The next recovery re-checks the folders against the plan.
    }
  }
  step(name, detail = {}) {
    this.record.steps.push({ step: name, at: isoMs(this.now()), ...detail });
    this.record.state = name;
    this.save();
  }
  get state() {
    return this.record.state;
  }
  get steps() {
    return this.record.steps.map((s) => s.step);
  }
}

function journalRecord(profile, { id, source = '', commit = '', now = Date.now }) {
  return { id, app: profile.appName, bundleId: profile.bundleId, source, commit, pid: process.pid, startedAt: isoMs(now()), state: 'started', plan: null, movesDone: 0, swappedAt: null, steps: [] };
}

export function memoryJournal(profile, fields = {}) {
  return new Journal(null, journalRecord(profile, { id: fields.id ?? 'unjournaled', ...fields }), fields.now);
}

/** Start install `id`'s journal under `dir/journal/`, durably, before anything is staged. */
export function openJournal(dir, profile, { id, source, commit, now = Date.now }) {
  const folder = join(dir, 'journal');
  mkdirSync(folder, { recursive: true, mode: 0o700 });
  const path = join(folder, `${safeInstallId(id)}.json`);
  if (existsSync(path)) throw new Error(`install id ${id} already has a journal (${path})`);
  const journal = new Journal(path, journalRecord(profile, { id, source, commit, now }), now);
  journal.save();
  return journal;
}

/** Every readable journal in `dir`, oldest first. */
export function listJournals(dir, { log = console.log, now = Date.now } = {}) {
  const folder = join(dir, 'journal');
  if (!existsSync(folder)) return [];
  const found = [];
  for (const name of readdirSync(folder).filter((n) => n.endsWith('.json')).sort()) {
    const path = join(folder, name);
    try {
      found.push(new Journal(path, JSON.parse(readFileSync(path, 'utf8')), now));
    } catch (error) {
      log(`[install-app] could not read the install journal ${path}: ${String(error.message).split('\n')[0]}`);
    }
  }
  return found.sort((a, b) => String(a.record.startedAt).localeCompare(String(b.record.startedAt)));
}

export function unsettledJournals(dir, opts) {
  return listJournals(dir, opts).filter((j) => !SETTLED.has(j.state));
}

/** Keep the newest `keep` settled journals; never touch one still in flight. */
export function pruneJournals(dir, keep = 20) {
  const settled = listJournals(dir, { log() {} }).filter((j) => SETTLED.has(j.state));
  for (const j of settled.slice(0, Math.max(0, settled.length - keep))) rmSync(j.path, { force: true });
}

/** Append a step to the journal at `path` (the reopen check, after the install). */
export function recordStep(path, step, detail = {}, now = Date.now) {
  const journal = new Journal(path, JSON.parse(readFileSync(path, 'utf8')), now);
  journal.step(step, detail);
  return journal;
}

export function pidAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === 'EPERM';
  }
}

/** Block this thread for `ms` (the installer is synchronous throughout). */
const sleepSync = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

/** A takeover claim older than this was left by an installer that died mid-takeover. */
const ABANDONED_CLAIM_MS = 10_000;

/**
 * Create `path` holding `text`, or return false if it exists. The text goes
 * to a private temp file first and is hard-linked into place (link never
 * replaces an existing file), so the lock never exists empty: an O_EXCL
 * create showed a contender an empty lock it took for a dead one's.
 */
function createWithText(path, text) {
  const tmp = `${path}.${process.pid}.${randomBytes(4).toString('hex')}.tmp`;
  const fd = openSync(tmp, 'wx', 0o600);
  try {
    try {
      writeSync(fd, text);
      fsyncSync(fd);
    } finally {
      closeSync(fd);
    }
    linkSync(tmp, path);
    return true;
  } catch (error) {
    if (error.code === 'EEXIST') return false;
    throw error;
  } finally {
    rmSync(tmp, { force: true });
  }
}

/** The lock file's inode and text, read through one descriptor; null when it is gone. */
function readLock(path) {
  let fd;
  try {
    fd = openSync(path, 'r');
  } catch (error) {
    if (error.code === 'ENOENT') return null;
    throw error;
  }
  try {
    return { ino: fstatSync(fd).ino, text: readFileSync(fd, 'utf8') };
  } finally {
    closeSync(fd);
  }
}

/**
 * Take the one-installer-at-a-time lock (`dir/installer.lock`). A live holder
 * refuses with its pid and install id; a lock whose pid is gone is stale
 * (that install died; its journal says what it left) and is taken over.
 * Returns `{ path, release() }`.
 *
 * The takeover is atomic (wave-1 review): two installers that both find the
 * same dead lock must not both remove it, or the slower one deletes the
 * faster one's fresh lock and both run. So a contender first claims that
 * very file by hard-linking it to `installer.lock.takeover-<inode>`, a name
 * only one can create, and removes the lock only if the claim reached the
 * same file (inode and text) it judged dead. Nobody else removes a claimed
 * lock, so the one it removes is that one.
 */
export function acquireLock(dir, { profile, installId = '', what = 'install', pid = process.pid, alive = pidAlive, now = Date.now, log = console.log, sleep = sleepSync, abandonedClaimMs = ABANDONED_CLAIM_MS } = {}) {
  mkdirSync(dir, { recursive: true, mode: 0o700 });
  const path = join(dir, 'installer.lock');
  const mine = JSON.stringify({ pid, installId, what, startedAt: isoMs(now()) });
  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (createWithText(path, mine)) {
      return {
        path,
        release() {
          try {
            if (readFileSync(path, 'utf8') === mine) rmSync(path, { force: true });
          } catch {
            // Already gone.
          }
        },
      };
    }
    const seen = readLock(path);
    if (!seen) continue;
    let holder = {};
    try {
      holder = JSON.parse(seen.text);
    } catch {
      // Unreadable: the lock is always written whole, so this one is damaged.
    }
    if (Number.isInteger(holder.pid) && alive(holder.pid)) {
      const app = profile?.appName ?? 'Hollow';
      throw new Error(`another ${app} installer is running (pid ${holder.pid}, ${holder.what ?? 'install'}${holder.installId ? ` ${holder.installId}` : ''}), so this ${what} did not start; wait for it to finish, then try again`);
    }
    const claim = join(dir, `installer.lock.takeover-${seen.ino}`);
    try {
      linkSync(path, claim);
    } catch (error) {
      if (error.code === 'ENOENT') continue; // Gone already: someone finished taking it over.
      if (error.code !== 'EEXIST') throw error;
      // Another installer is taking this lock over. Wait for it, unless its
      // claim is old enough that it died doing so.
      try {
        // Wall-clock time, like the claim's ctime (link sets it); `now` may be a test clock.
        if (Date.now() - lstatSync(claim).ctimeMs >= abandonedClaimMs) rmSync(claim, { force: true });
      } catch {
        // The claim went away meanwhile.
      }
      sleep(25);
      continue;
    }
    try {
      const claimed = readLock(claim);
      // The lock changed between reading and claiming it: not ours to remove.
      if (!claimed || claimed.ino !== seen.ino || claimed.text !== seen.text) continue;
      log(`[install-app] taking over a stale installer lock (pid ${holder.pid ?? 'unknown'} is gone)`);
      rmSync(path, { force: true });
    } finally {
      rmSync(claim, { force: true });
    }
  }
  throw new Error(`could not take the installer lock ${path}: other installers kept taking it over`);
}

/** Rename `from` → `to` is done: only `to` exists (true for the first move not yet recorded). */
const moveDone = ([from, to]) => !existsSync(from) && existsSync(to);

/**
 * Undo the journal's recorded renames, newest first. A move whose undo
 * already happened (the crash came between the rename and the journal
 * write) is skipped; anything else stops with the paths.
 */
function undoMoves(journal) {
  const r = journal.record;
  const moves = r.plan?.moves ?? [];
  r.state = 'undoing';
  journal.trySave();
  for (let k = r.movesDone - 1; k >= 0; k -= 1) {
    const [from, to] = moves[k];
    if (existsSync(to) && !existsSync(from)) renameSync(to, from);
    else if (!(existsSync(from) && !existsSync(to))) {
      throw new Error(`could not undo ${from} → ${to}: ${existsSync(from) ? 'both exist' : 'neither exists'}; resolve by hand`);
    }
    r.movesDone = k;
    journal.trySave();
  }
}

/**
 * Finish or undo one interrupted install from its journal: nothing moved
 * yet, some of the planned renames done, or all done but not verified.
 * Returns the state it settles in (`verified` or `undone`).
 */
export function recoverJournal(profile, journal, { ops = macos, log = console.log } = {}) {
  const p = layout(profile);
  const r = journal.record;
  const tag = `[install-app] install ${r.id}`;
  if (!r.plan) {
    rmSync(p.staged, { recursive: true, force: true });
    journal.step('undone', { recovered: true, reason: 'stopped before anything was moved' });
    log(`${tag} stopped before anything was moved; removed its staged copy`);
    return 'undone';
  }
  if (r.state === 'prepared' || r.state === 'promoted') {
    const moves = r.plan.moves;
    if (r.movesDone < moves.length && moveDone(moves[r.movesDone])) {
      r.movesDone += 1;
      journal.save();
    }
    if (r.movesDone === moves.length) {
      if (verified(ops, p.installed, profile, log)) {
        rmSync(p.discard, { recursive: true, force: true });
        journal.step('verified', { recovered: true });
        log(`${tag} had swapped in its app before it stopped; the app verifies, so the install stands`);
        return 'verified';
      }
      log(`${tag} had swapped in its app before it stopped, and it does not verify: undoing the install`);
    } else {
      log(`${tag} stopped after ${r.movesDone} of ${moves.length} renames: undoing them`);
    }
  } else {
    log(`${tag} stopped while putting the apps back (${r.state}): finishing that`);
  }
  undoMoves(journal);
  rmSync(p.staged, { recursive: true, force: true });
  journal.step('undone', { recovered: true });
  return 'undone';
}

/**
 * Finish or undo whatever an interrupted install or rollback left. With
 * `stateDir`, an install that has a journal is recovered from it; the
 * folder-by-folder checks below are for installs from before the journal
 * and for a rollback swap. Never deletes a backup unless the installed app
 * verifies in its place.
 */
export function recover(profile, { ops = macos, log = console.log, stateDir = null } = {}) {
  const p = layout(profile);
  if (stateDir) {
    for (const journal of unsettledJournals(stateDir, { log })) recoverJournal(profile, journal, { ops, log });
  }
  rmSync(p.staged, { recursive: true, force: true });

  // An interrupted rollback swap.
  if (existsSync(p.swap)) {
    if (!existsSync(p.installed)) renameSync(p.swap, p.installed);
    else if (!existsSync(p.previous)) renameSync(p.swap, p.previous);
    else throw new Error(`${p.swap}, ${p.installed} and ${p.previous} all exist; resolve by hand`);
    log('[install-app] recovered an interrupted rollback');
  }

  // An interrupted rotation from an installer without a journal: discard is
  // the older verified backup.
  if (existsSync(p.discard)) {
    log(`[install-app] ${p.discard} is not in any install journal; recovering from the folders on disk`);
    if (!existsSync(p.installed) && existsSync(p.previous)) {
      // Moved out the old app, never moved the new one in.
      renameSync(p.previous, p.installed);
      renameSync(p.discard, p.previous);
      log('[install-app] restored the app an interrupted install had moved aside');
    } else if (existsSync(p.installed) && !existsSync(p.previous)) {
      renameSync(p.discard, p.previous);
    } else if (existsSync(p.installed) && existsSync(p.previous)) {
      // The new app was swapped in but never confirmed: confirm it now.
      if (verified(ops, p.installed, profile, log)) {
        rmSync(p.discard, { recursive: true, force: true });
        log('[install-app] confirmed the app an interrupted install had swapped in');
      } else {
        rmSync(p.installed, { recursive: true, force: true });
        renameSync(p.previous, p.installed);
        renameSync(p.discard, p.previous);
        log('[install-app] undid an interrupted install whose app did not verify');
      }
    } else {
      throw new Error(`${p.discard} exists without an installed or previous app; resolve by hand`);
    }
  }

  // Only the backup survived: put it back.
  if (!existsSync(p.installed) && existsSync(p.previous)) {
    renameSync(p.previous, p.installed);
    log('[install-app] restored the previous app (nothing was installed)');
  }
}

function runningPids(ops, bundles) {
  const pids = [];
  for (const bundle of bundles) {
    if (!existsSync(bundle)) continue;
    let executable;
    try {
      executable = ops.identity(bundle).executable;
    } catch {
      continue;
    }
    pids.push(...ops.running(bundle, executable));
  }
  return pids;
}

const sleepMs = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

/**
 * Wait up to `timeoutMs` for the app to quit, then refuse if it still runs.
 * Hollow's install helper starts the installer as Hollow quits, and a helper
 * written by an app older than #261 does not wait for the quit itself (its
 * pgrep cannot see Hollow, its ancestor). On 2026-10-02 that made the
 * 3476ad61 → b3ff868b install refuse a Hollow that quit a second later.
 */
export function waitUntilQuit(ops, profile, bundles, { timeoutMs = 60_000, pollMs = 500, now = Date.now, sleep = sleepMs, log = console.log } = {}) {
  const deadline = now() + timeoutMs;
  let pids = runningPids(ops, bundles);
  if (pids.length > 0) log(`[install-app] waiting up to ${Math.round(timeoutMs / 1000)} s for ${profile.appName} (pid ${pids.join(', ')}) to quit`);
  while (pids.length > 0 && now() < deadline) {
    sleep(pollMs);
    pids = runningPids(ops, bundles);
  }
  if (pids.length > 0) {
    throw new Error(`${profile.appName} is running (pid ${pids.join(', ')}); quit it before installing or rolling back`);
  }
}

/**
 * Throw, after `cleanup`, when any bundle of the profile is running now (by
 * bundle path; `ops.running` keeps `pgrep -a` so a Hollow that is the
 * installer's ancestor is still seen). Nothing has been moved yet.
 */
function refuseIfRunning(ops, profile, p, what, cleanup = () => {}) {
  let pids;
  try {
    pids = runningPids(ops, [p.installed, p.previous, p.discard, p.swap]);
  } catch (error) {
    cleanup();
    throw new Error(`could not check that ${profile.appName} is closed before the swap, so nothing was swapped: ${error.message}`);
  }
  if (pids.length > 0) {
    cleanup();
    throw new Error(`${profile.appName} was opened during the ${what} (pid ${pids.join(', ')}), so nothing was swapped; quit it and try again`);
  }
}

const iso = (ms) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z');

/**
 * After the swap: open the app and confirm the running process is the
 * installed one (started after the swap, from the installed bundle, whose
 * provenance is `expectCommit`). A process from before the swap is quit (TERM)
 * and the app reopened, once; if it still does not match, say why.
 * Returns `{ runningMatches, relaunched, runningPid, runningReason }`.
 */
export function confirmRunning(profile, { ops = macos, swappedAt, expectCommit = '', appearMs = 30_000, quitMs = 30_000, pollMs = 500, now = Date.now, sleep = sleepMs, log = console.log } = {}) {
  const p = layout(profile);
  let relaunched = false;
  const outcome = (runningMatches, runningReason, runningPid = null) => ({ runningMatches, relaunched, runningPid, runningReason });
  try {
    const installedCommit = commitOf(p.installed);
    if (expectCommit && !installedCommit.startsWith(expectCommit)) {
      return outcome(false, `${p.installed} is ${installedCommit.slice(0, 8) || 'of unknown provenance'}, not ${expectCommit.slice(0, 8)}, the commit this install was built from.`);
    }
    if (!Number.isFinite(swappedAt)) return outcome(false, 'The installer recorded no swap time, so the running app could not be checked.');
    const executable = ops.identity(p.installed).executable;
    const poll = (until, done) => {
      const deadline = now() + until;
      let seen = ops.processes(p.installed, executable);
      while (!done(seen) && now() < deadline) {
        sleep(pollMs);
        seen = ops.processes(p.installed, executable);
      }
      return seen;
    };
    for (;;) {
      ops.open(p.installed);
      const seen = poll(appearMs, (list) => list.length > 0);
      if (seen.length === 0) return outcome(false, `${profile.appName} did not start within ${Math.round(appearMs / 1000)} s of reopening.`);
      // `ps` start times are whole seconds and `swappedAt` is not, so compare
      // seconds: an app opened in the swap's own second is the installed one.
      // (The 3b40f87e install, 2026-10-03, called two fresh apps "from before
      // the swap" and quit one.) Nothing older can be running: refuseIfRunning
      // checked right before the renames.
      const swapSecond = Math.floor(swappedAt / 1000) * 1000;
      const stale = seen.filter((proc) => !(proc.startedAt >= swapSecond));
      if (stale.length === 0) {
        log(`[install-app] ${profile.appName} pid ${seen[0].pid} started after the swap: it runs the installed app${relaunched ? ' (after one relaunch)' : ''}`);
        return outcome(true, '', seen[0].pid);
      }
      const which = stale.map((proc) => `pid ${proc.pid}, started ${iso(proc.startedAt)}`).join('; ');
      if (relaunched) {
        return outcome(false, `${profile.appName} (${which}) is still the app from before the swap at ${iso(swappedAt)}, even after one relaunch; quit and reopen Hollow.`, stale[0].pid);
      }
      log(`[install-app] ${profile.appName} (${which}) started before the swap at ${iso(swappedAt)}; quitting it and reopening once`);
      const pids = stale.map((proc) => proc.pid);
      ops.quit(pids);
      const left = poll(quitMs, (list) => !list.some((proc) => pids.includes(proc.pid)));
      if (left.some((proc) => pids.includes(proc.pid))) {
        return outcome(false, `${profile.appName} (${which}) started before the swap at ${iso(swappedAt)} and did not quit within ${Math.round(quitMs / 1000)} s; it still runs the previous app.`, pids[0]);
      }
      relaunched = true;
    }
  } catch (error) {
    return outcome(false, `Could not confirm the running app: ${error.message.split('\n')[0]}`);
  }
}

/** Tag `error` with what is on disk after it: `installed` is 'yes', 'no' or 'unknown'. */
const tagged = (error, installed) => Object.assign(error instanceof Error ? error : new Error(String(error)), { installed });

/**
 * Install `bundle` as the profile's app. The current app becomes the
 * previous one only if it verifies now; otherwise the existing previous is
 * kept. On any failure the apps on disk are put back as they were.
 *
 * `journal` records each step durably: `prepared` (the staged copy verified,
 * with the renames planned), each rename as it happens, `promoted` (the new
 * app is in place, with `swappedAt`) and `verified`. A thrown error carries
 * `installed`: 'no' (nothing moved, or every rename was undone) or 'unknown'
 * (putting the apps back failed too).
 */
export function install(profile, bundle, { ops = macos, log = console.log, packageVersion, now = Date.now, journal = memoryJournal(profile, { source: bundle, now }) } = {}) {
  const p = layout(profile);
  let id;
  let moves;
  try {
    id = ops.identity(bundle);
    // `packageVersion` is one version or the list a build may carry
    // (package.json's, or the release stamp for this commit: acceptedVersions).
    const accepted = packageVersion ? [packageVersion].flat() : [];
    if (accepted.length && !accepted.includes(id.version)) {
      throw new Error(`bundle version ${id.version} does not match package version ${accepted.join(' or ')}`);
    }
    if (id.bundleId !== profile.bundleId) throw new Error(`unexpected bundle identifier: ${id.bundleId}`);
    const exe = (b) => join(b, 'Contents', 'MacOS', id.executable);

    ops.verify(bundle, id.executable);
    rmSync(p.staged, { recursive: true, force: true });
    ops.copy(bundle, p.staged);
    let stagedSelftest;
    try {
      if (!ops.sameFile(exe(bundle), exe(p.staged))) throw new Error('staged executable differs from the build');
      stagedSelftest = ops.verify(p.staged, id.executable);
    } catch (error) {
      rmSync(p.staged, { recursive: true, force: true });
      throw new Error(`staged copy did not verify: ${error.message}`);
    }

    // Rotate: the current app is kept as previous only when it verifies now.
    const keepCurrent = existsSync(p.installed) && verified(ops, p.installed, profile, log) !== null;
    if (existsSync(p.discard)) throw new Error(`${p.discard} is left from an earlier install; run the installer's recovery first`);
    // Re-check right before the swap, not only before the install: an app
    // opened while the build was staged and verified keeps running the old
    // binary after the swap (seen 2026-10-02 in the 30f3896e install).
    refuseIfRunning(ops, profile, p, 'install', () => rmSync(p.staged, { recursive: true, force: true }));
    moves = [];
    if (keepCurrent) {
      if (existsSync(p.previous)) moves.push([p.previous, p.discard]);
      moves.push([p.installed, p.previous]);
    } else if (existsSync(p.installed)) {
      // Not verifiable: never let it replace a verified previous.
      moves.push([p.installed, p.discard]);
    }
    moves.push([p.staged, p.installed]);
    journal.record.plan = { keepCurrent, moves };
    journal.record.movesDone = 0;
    journal.step('prepared', { keepCurrent, selftest: String(stagedSelftest ?? '') });
  } catch (error) {
    rmSync(p.staged, { recursive: true, force: true });
    try {
      journal.step('failed', { reason: reasonOf(error), moved: false });
    } catch {
      // Nothing was moved; the journal stays `started` and recovery only clears the staged copy.
    }
    throw tagged(error, 'no');
  }

  const exe = (b) => join(b, 'Contents', 'MacOS', id.executable);
  try {
    for (const [from, to] of moves) {
      renameSync(from, to);
      journal.record.movesDone += 1;
      journal.save();
    }
    const swappedAt = now();
    journal.record.swappedAt = swappedAt;
    journal.step('promoted', { swappedAt });

    if (!ops.sameFile(exe(bundle), exe(p.installed))) throw new Error('installed executable differs from the build');
    const selftest = ops.verify(p.installed, id.executable);
    rmSync(p.discard, { recursive: true, force: true });
    try {
      journal.step('verified', { selftest: String(selftest ?? '') });
    } catch (error) {
      // Installed and verified; the journal still says promoted, so the
      // next recovery verifies it again rather than undoing it.
      log(`[install-app] could not record the verified step: ${reasonOf(error)}`);
    }
    log(`[install-app] installed ${profile.appName} ${id.version} (${id.bundleId}): ${provenanceOf(p.installed)}`);
    log(`[install-app] ${selftest}`);
    log(
      existsSync(p.previous)
        ? `[install-app] kept the previous verified app for rollback: ${provenanceOf(p.previous)}`
        : '[install-app] no previous app to keep',
    );
    return { installed: p.installed, previous: existsSync(p.previous) ? p.previous : null, selftest, swappedAt };
  } catch (error) {
    try {
      undoMoves(journal);
      rmSync(p.staged, { recursive: true, force: true });
    } catch (rollbackError) {
      try {
        journal.step('restore-failed', { reason: reasonOf(rollbackError) });
      } catch {
        // The journal still names the plan; recovery retries the undo.
      }
      throw tagged(new AggregateError([error, rollbackError], `${profile.appName} install failed and rollback also failed`), 'unknown');
    }
    try {
      journal.step('failed', { reason: reasonOf(error), moved: true, undone: true });
    } catch {
      // The apps are back; the journal says `undoing` and recovery finds nothing left to undo.
    }
    throw tagged(error, 'no');
  }
}

/** Swap the installed app with the previous verified one. One step, reversible. */
export function rollback(profile, { ops = macos, log = console.log } = {}) {
  const p = layout(profile);
  if (!existsSync(p.previous)) throw new Error(`no previous ${profile.appName} to roll back to (${p.previous})`);
  if (!existsSync(p.installed)) throw new Error(`${p.installed} is missing; run the installer's recovery first`);
  if (!verified(ops, p.previous, profile, log)) throw new Error(`the previous app does not verify; not rolling back`);
  refuseIfRunning(ops, profile, p, 'rollback');
  const before = provenanceOf(p.installed);

  renameSync(p.installed, p.swap);
  try {
    renameSync(p.previous, p.installed);
    renameSync(p.swap, p.previous);
    const selftest = ops.verify(p.installed, ops.identity(p.installed).executable);
    log(`[install-app] rolled back: ${provenanceOf(p.installed)} is installed; ${before} is now the previous app`);
    log(`[install-app] ${selftest}`);
    return { installed: p.installed, previous: p.previous, selftest };
  } catch (error) {
    // Put them back the way they were.
    if (existsSync(p.swap)) {
      if (existsSync(p.installed)) renameSync(p.installed, p.previous);
      renameSync(p.swap, p.installed);
    } else {
      renameSync(p.installed, p.swap);
      renameSync(p.previous, p.installed);
      renameSync(p.swap, p.previous);
    }
    throw error;
  }
}

export function status(profile) {
  const p = layout(profile);
  return {
    installed: existsSync(p.installed) ? provenanceOf(p.installed) : null,
    previous: existsSync(p.previous) ? provenanceOf(p.previous) : null,
  };
}

/** One line per error, for the receipt. */
function reasonOf(error) {
  const first = (e) => String(e?.message ?? e).split('\n')[0];
  return error instanceof AggregateError ? `${first(error)}: ${error.errors.map(first).join('; ')}` : first(error);
}

/**
 * The versions a build of this checkout may carry: package.json's (a plain
 * `npm run tauri:bundle`), the release stamp `0.6.<first-parent count of
 * HEAD>` (scripts/release/build-app.mjs), or the stamp of the commit the
 * bundle's build-provenance.json records (a docs-only commit after the build
 * leaves the source fingerprint, so the provenance check, unchanged). Call it
 * only after that check passed. Any other version is refused.
 */
export function acceptedVersions(root, { git, bundle } = {}) {
  const accepted = [JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version];
  // A shallow clone has no stamp; a plain build still installs.
  try { accepted.push(buildVersion(root, 'HEAD', git)); } catch { /* shallow */ }
  let commit = null;
  try {
    commit = JSON.parse(readFileSync(join(bundle, 'Contents', 'Resources', 'build-provenance.json'), 'utf8')).commit;
  } catch { /* no bundle or no provenance: HEAD's stamp only */ }
  if (bundle && /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(commit ?? '')) {
    // A commit this checkout doesn't have can't be counted: HEAD's stamp only.
    try { accepted.push(buildVersion(root, commit, git)); } catch { /* unknown commit */ }
  }
  return [...new Set(accepted)];
}

/** Write `report` as one JSON line, atomically (the helper embeds it as is). */
export function writeReport(path, report) {
  if (!path) return;
  writeFileSync(`${path}.tmp`, `${JSON.stringify(report)}\n`);
  renameSync(`${path}.tmp`, path);
}

/**
 * Take the installer lock, wait for the app to quit, recover from any
 * unfinished journal, install under a new journal, and clean up the build
 * bundle. Returns `{ report, failure }`. The report says what happened and,
 * on failure, why (`reason`), and what is on disk now (`installed`: 'yes',
 * 'no', or 'unknown' when the installer cannot vouch for either app), with
 * the install id, its journal and the steps it reached. `ok` is true only
 * after the installed app verified in place.
 */
export function runInstall(profile, bundle, { ops = macos, log = console.log, packageVersion, waitMs = 60_000, clock = {}, installId = newInstallId(clock.now), stateDir = installStateDir(profile) } = {}) {
  const p = layout(profile);
  let lock;
  try {
    lock = acquireLock(stateDir, { profile, installId, what: 'install', log, now: clock.now });
  } catch (error) {
    // The build bundle may be what the running install is copying: leave it.
    return { report: { ok: false, reason: reasonOf(error), installed: 'no', installId, journal: null, steps: [] }, failure: error };
  }
  let failure;
  let installed;
  let journal = null;
  let outcome = 'no';
  try {
    // A running app is refused, and the build bundle is still cleaned up.
    waitUntilQuit(ops, profile, [p.installed, p.previous, p.discard, p.swap], { timeoutMs: waitMs, log, ...clock });
    try {
      recover(profile, { ops, log, stateDir });
    } catch (error) {
      // Recovery may have moved some folders before it stopped.
      throw tagged(error, 'unknown');
    }
    journal = openJournal(stateDir, profile, { id: installId, source: bundle, commit: commitOf(bundle), now: clock.now });
    installed = install(profile, bundle, { ops, log, packageVersion, now: clock.now, journal });
    outcome = 'yes';
  } catch (error) {
    failure = error;
    outcome = error?.installed ?? 'unknown';
  }
  try {
    try {
      ops.cleanupBuild(bundle, profile);
    } catch (cleanupError) {
      failure = failure
        ? new AggregateError([failure, cleanupError], `${profile.appName} install and temporary-bundle cleanup both failed`)
        : cleanupError;
    }
    try {
      pruneJournals(stateDir);
    } catch {
      // Old journals are only history.
    }
  } finally {
    lock.release();
  }
  const base = { installed: outcome, installId, journal: journal?.path ?? null, steps: journal?.steps ?? [] };
  if (failure) return { report: { ok: false, reason: reasonOf(failure), ...base }, failure };
  return { report: { ok: true, reason: '', ...base, installedCommit: commitOf(p.installed), swappedAt: installed.swappedAt } };
}

/**
 * The reopen check after a successful install, under the installer lock:
 * `confirmRunning`, then the journal's `healthy` (or `running-mismatch`)
 * step. Returns the report with the check (and the steps) added.
 */
export function confirmAndRecord(profile, report, { ops = macos, log = console.log, expectCommit = '', stateDir = installStateDir(profile), confirm = confirmRunning, ...opts } = {}) {
  let lock;
  let checked;
  try {
    lock = acquireLock(stateDir, { profile, installId: report.installId ?? '', what: 'reopen check', log });
    checked = confirm(profile, { ops, log, swappedAt: report.swappedAt, expectCommit, ...opts });
  } catch (error) {
    checked = { runningMatches: false, relaunched: false, runningPid: null, runningReason: `Could not confirm the running app: ${reasonOf(error)}` };
  } finally {
    lock?.release();
  }
  let steps = report.steps;
  if (report.journal) {
    try {
      const step = checked.runningMatches ? 'healthy' : 'running-mismatch';
      steps = recordStep(report.journal, step, checked.runningMatches ? { runningPid: checked.runningPid } : { reason: checked.runningReason }).steps;
    } catch (error) {
      log(`[install-app] could not record the reopen check in ${report.journal}: ${reasonOf(error)}`);
    }
  }
  return { ...report, ...checked, ...(steps ? { steps } : {}) };
}

const flag = (argv, name) => {
  const at = argv.indexOf(name);
  return at >= 0 ? (argv[at + 1] ?? '') : null;
};

function main(argv) {
  if (process.platform !== 'darwin') throw new Error('the Hollow install workflow is macOS-only');
  const name = process.env.HOLLOW_INSTALL_PROFILE || 'everyday';
  const profile = PROFILES[name];
  if (!profile) throw new Error(`unknown HOLLOW_INSTALL_PROFILE ${name}`);
  const p = layout(profile);

  if (argv.includes('--status')) {
    console.log(JSON.stringify(status(profile), null, 2));
    return;
  }

  const confirmFrom = flag(argv, '--confirm-running');
  if (confirmFrom !== null) {
    // After a successful install: add the running-app check to its report.
    let report = {};
    try {
      report = JSON.parse(readFileSync(confirmFrom, 'utf8'));
    } catch {
      // Checked below as a missing swap time.
    }
    const checked = confirmAndRecord(profile, report, { expectCommit: flag(argv, '--expect') ?? '' });
    writeReport(confirmFrom, checked);
    if (!checked.runningMatches) console.warn(`[install-app] ${checked.runningReason}`);
    return;
  }

  const onDisk = [p.installed, p.previous, p.discard, p.swap];
  const waitMs = 1000 * Number(process.env.HOLLOW_INSTALL_WAIT_SECS ?? 60);

  if (argv.includes('--rollback')) {
    const lock = acquireLock(installStateDir(profile), { profile, what: 'rollback' });
    try {
      // Never move a bundle that is in use.
      waitUntilQuit(macos, profile, onDisk, { timeoutMs: waitMs });
      recover(profile, { stateDir: installStateDir(profile) });
      rollback(profile);
    } finally {
      lock.release();
    }
    return;
  }

  const reportPath = flag(argv, '--report');
  // Hollow's helper passes its own id, so the receipt and the journal match.
  const installId = safeInstallId(flag(argv, '--install-id')) || newInstallId();
  const targetDir = process.env.CARGO_TARGET_DIR || join(repoRoot, 'src-tauri', 'target');
  const bundle = join(targetDir, 'release', 'bundle', 'macos', `${profile.appName}.app`);
  let packageVersion;
  try {
    // Refuse rather than resolve(): resolve('~/x') is <cwd>/~/x (overnight A4).
    const targetProblem = cargoTargetProblem(process.env.CARGO_TARGET_DIR);
    if (targetProblem) throw new Error(targetProblem);
    if (!existsSync(join(bundle, 'Contents', 'Info.plist'))) {
      throw new Error(`built app not found at ${bundle}; run the app bundle build first`);
    }
    // A matching version/commit is insufficient when worktrees contain uncommitted work.
    macos.checkProvenance(bundle);
    packageVersion = acceptedVersions(repoRoot, { bundle });
  } catch (error) {
    writeReport(reportPath, { ok: false, reason: reasonOf(error), installed: 'no', installId, journal: null, steps: [] });
    throw error;
  }
  const { report, failure } = runInstall(profile, bundle, { packageVersion, waitMs, installId });
  writeReport(reportPath, report);
  if (failure) throw failure;
  console.log(`[install-app] removed the temporary build bundle; ${p.installed} is the only indexed ${profile.appName}`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2));
}
