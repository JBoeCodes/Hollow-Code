#!/usr/bin/env node
// The Nightly release builder (updates design 2026-11-14, section 5, row U2).
// One run: fetch origin/main, take the tip, and when it is new, build it in a
// clean worktree, run every check, then publish it to the Nightly channel
// with the channel manifest written last. Any failed check publishes nothing
// and leaves a notification naming the check and the commit.
//
//   node scripts/release/nightly.mjs            one run (the LaunchDaemon runs this every 300 s)
//   node scripts/release/nightly.mjs --retry    build the tip even if it already failed
//
// The steps, in order (each a named check in the log and the notification):
//   fetch      git fetch origin main into refs/remotes/origin/main
//   config     a notarized Developer ID build with a team, the updater key
//              and R2 settings, or nothing runs; then the live manifest is
//              read (already the tip: record it and stop)
//   checkout   a detached worktree at the tip (never the dev checkout), cleaned
//   install    `npm ci` (HOLLOW_NIGHTLY_INSTALL_COMMAND)
//   gate       a passing gate record for the tip's source fingerprint
//              (`<git common dir>/hollow-gate-records/<fp>.json`, as
//              ri27-tools/merge.sh reads it), else `npm run gate` there
//   build      scripts/release/build-app.mjs --ref <tip> --no-verify [--dmg]
//   verify     build-app.mjs --verify-only --no-selftest (codesign, team,
//              spctl and stapler when notarized, version, provenance)
//   selftest   build-app.mjs --selftest-only (the bundle's embed-selftest)
//   sign       Hollow.app.tar.gz and its minisign .sig (`tauri signer sign`)
//   archive    the unpacked archive passes codesign, spctl and stapler
//   notes      what's new since the previous Nightly (scripts/release/notes.mjs)
//   manifest   channels/nightly.json, checked with the Worker's own parser
//   upload     never over the version the live manifest serves, never lower;
//              publishOrder(): archive, .sig, DMG, the build's own copy of
//              the manifest (builds/<v>/manifest.json, which promotion to
//              Main copies, row U8), notes.json, channel manifest LAST
// Then `prune` keeps the newest Nightlies and what the channels point at.
//
// Every effect goes through `deps` (git, run, fetch, uploader, clock, pid
// checks), so scripts/release-nightly.test.mjs drives it with fakes and
// temp git repos. Configuration and its environment variables: loadConfig
// below and docs/UPDATES.md. Secrets (the updater key, R2 and App Store
// Connect credentials) come from the environment only, reach only the step
// that needs them, and are never printed or put in argv.
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, existsSync, linkSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir, hostname } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { appPath, resolveSigning } from './build-app.mjs';
import { collectNotes, notesText } from './notes.mjs';
import { createR2Uploader, missingR2Config } from './r2-uploader.mjs';
import { buildVersion } from '../version.mjs';
import { BUILD_RECORD, PLATFORM, buildKey, channelKey, CHANNELS, isVersion } from '../../infra/updates-worker/src/layout.js';
import { parseManifest } from '../../infra/updates-worker/src/manifest.js';
import { publishOrder } from '../../infra/updates-worker/src/publish-order.js';

export const CHANNEL = 'nightly';
export const DEFAULT_HOST = 'updates.example.com';
export const DEFAULT_KEEP_NIGHTLY = 20;
export const DEFAULT_KEEP_MAIN = 10;
export const DEFAULT_LOCK_STALE_MINUTES = 360;
/** Each step's time limit in minutes; a step that runs over is killed (SIGTERM) and fails. */
export const STEP_TIMEOUT_MINUTES = Object.freeze({ git: 10, fingerprint: 5, install: 30, gate: 120, build: 150, verify: 15, selftest: 10, tar: 15, sign: 5, archive: 15 });
export const SIGNING_VARS = ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PATH', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'];
export const R2_VARS = ['HOLLOW_R2_ACCESS_KEY_ID', 'HOLLOW_R2_SECRET_ACCESS_KEY', 'HOLLOW_R2_ENDPOINT', 'HOLLOW_R2_BUCKET'];
const NOTARY_VARS = ['APPLE_API_KEY', 'APPLE_API_ISSUER', 'APPLE_API_KEY_PATH', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID'];
const SHA = /^[0-9a-f]{40}$/;
const FINGERPRINT = /^[0-9a-f]{64}$/;

/**
 * Each store's `PRAGMA user_version` as the source declares it (design
 * section 9; U6 reads the manifest's `schema`): the constant each store
 * stamps (Sessions and the companion ledger since the U6 review, through
 * `hollow_databases::stamp_schema_version`). `constant: null` would mean a
 * store that never sets user_version (0); if that file ever starts writing
 * one, the manifest step fails until its constant is listed here. Read from
 * the tip's source, so no app has to run.
 */
export const SCHEMA_SOURCES = Object.freeze([
  { store: 'sessions', file: 'src-tauri/src/session_store.rs', constant: 'SCHEMA_VERSION' },
  { store: 'companion', file: 'src-tauri/src/companion_inbox.rs', constant: 'LEDGER_SCHEMA_VERSION' },
  { store: 'searchIndex', file: 'src-tauri/src/vault_search_index.rs', constant: 'SCHEMA_VERSION' },
  { store: 'embedIndex', file: 'src-tauri/src/vault_embed_index.rs', constant: 'SCHEMA_VERSION' },
  { store: 'projectIndex', file: 'src-tauri/src/project_text_index.rs', constant: 'SCHEMA_VERSION' },
]);

/** A failed step: `check` is its name in the log and the notification. */
export class StepError extends Error {
  constructor(check, message) {
    super(message);
    this.check = check;
  }
}

const set = (env, k) => typeof env[k] === 'string' && env[k].trim() !== '';
const words = (text) => text.trim().split(/\s+/).filter(Boolean);
/** Text safe to log: URL credentials, query strings and token-looking words removed. */
export function redact(text) {
  return String(text)
    .replace(/([a-z][a-z0-9+.-]*:\/\/)[^\s/@]+@/gi, '$1***@')
    .replace(/(https?:\/\/[^\s?#]+)\?[^\s]*/gi, '$1?***')
    .replace(/\b(gh[pousr]_[A-Za-z0-9]{10,}|github_pat_[A-Za-z0-9_]{10,})\b/g, '***');
}
const tail = (r) => redact((r.error || `${r.stderr ?? ''}\n${r.stdout ?? ''}`).trim().split('\n').slice(-3).join(' ').slice(0, 400));
const minutes = (m) => m * 60_000;

/** The environment without the named variables. */
export function envWithout(env, names) {
  const out = { ...env };
  for (const k of names) delete out[k];
  return out;
}

/** The builder's settings from the environment. Throws on a bad value. */
export function loadConfig(env = process.env, { scriptRoot = fileURLToPath(new URL('../..', import.meta.url)) } = {}) {
  const abs = (k, fallback) => {
    const v = set(env, k) ? env[k].trim() : fallback;
    if (!isAbsolute(v) || v.split('/').some(s => s.startsWith('~'))) throw new Error(`${k} must be an absolute path`);
    return resolve(v);
  };
  const stateDir = abs('HOLLOW_NIGHTLY_STATE_DIR', join(homedir(), 'Library', 'Application Support', 'hollow-nightly'));
  const repo = abs('HOLLOW_NIGHTLY_REPO', resolve(scriptRoot));
  const worktree = abs('HOLLOW_NIGHTLY_WORKTREE', join(stateDir, 'worktree'));
  if (worktree === repo || worktree.startsWith(`${repo}/`)) throw new Error('HOLLOW_NIGHTLY_WORKTREE must be outside HOLLOW_NIGHTLY_REPO');
  const host = set(env, 'HOLLOW_UPDATES_HOST') ? env.HOLLOW_UPDATES_HOST.trim() : DEFAULT_HOST;
  if (!/^[a-z0-9.-]+(:\d+)?$/i.test(host)) throw new Error('HOLLOW_UPDATES_HOST must be a host name');
  const count = (k, d) => {
    if (!set(env, k)) return d;
    const n = Number(env[k]);
    if (!Number.isSafeInteger(n) || n < 1) throw new Error(`${k} must be a whole number above zero`);
    return n;
  };
  return {
    repo,
    stateDir,
    worktree,
    outbox: abs('HOLLOW_NIGHTLY_OUTBOX', join(stateDir, 'outbox')),
    host,
    dmg: env.HOLLOW_NIGHTLY_DMG === '1',
    installCommand: words(set(env, 'HOLLOW_NIGHTLY_INSTALL_COMMAND') ? env.HOLLOW_NIGHTLY_INSTALL_COMMAND : 'npm ci'),
    gateCommand: words(set(env, 'HOLLOW_NIGHTLY_GATE_COMMAND') ? env.HOLLOW_NIGHTLY_GATE_COMMAND : 'npm run gate'),
    wrap: set(env, 'HOLLOW_NIGHTLY_WRAP') ? words(env.HOLLOW_NIGHTLY_WRAP) : [],
    gateRecordDirs: set(env, 'HOLLOW_NIGHTLY_GATE_RECORDS') ? env.HOLLOW_NIGHTLY_GATE_RECORDS.split(':').filter(Boolean).map(d => resolve(d)) : [],
    keepNightly: count('HOLLOW_NIGHTLY_KEEP', DEFAULT_KEEP_NIGHTLY),
    keepMain: count('HOLLOW_NIGHTLY_KEEP_MAIN', DEFAULT_KEEP_MAIN),
    lockStaleMs: count('HOLLOW_NIGHTLY_LOCK_STALE_MINUTES', DEFAULT_LOCK_STALE_MINUTES) * 60_000,
  };
}

// ---------------------------------------------------------------- state files

/** Write JSON atomically (temp file in the same folder, then rename). */
export function writeJsonAtomic(path, value) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const tmp = `${path}.${process.pid}.${Date.now()}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
  renameSync(tmp, path);
}

export function readJson(path, fallback) {
  try { return JSON.parse(readFileSync(path, 'utf8')); } catch { return fallback; }
}

export const statePath = (config) => join(config.stateDir, 'state.json');
export const lockPath = (config) => join(config.stateDir, 'nightly.lock');
export const mainHistoryPath = (config) => join(config.stateDir, 'main-history.json');

/** promote.mjs's lock: a publish or withdraw holds it, and so does pruning. */
export const promoteLockPath = (config) => join(config.stateDir, 'promote.lock');

/**
 * The Main stack `main-history.json` describes, oldest first: a publish
 * pushes its version, a withdraw pops the live one (its entry names the
 * version Main went back to, which must be the new top). Entries are
 * `{ version, commit, promotedAt, from, action }` (promote.mjs); a plain
 * version string is a publish. Throws when the list doesn't replay.
 */
export function replayMainStack(history) {
  if (!Array.isArray(history)) throw new Error('the Main history is not a list');
  const stack = [];
  for (const e of history) {
    const entry = typeof e === 'string' ? { version: e, action: 'publish' } : e;
    if (!entry || typeof entry !== 'object' || !isVersion(entry.version)) throw new Error('the Main history holds an entry that is not a Main version');
    if (entry.action !== 'withdraw') { stack.push({ version: entry.version, commit: entry.commit ?? null }); continue; }
    stack.pop();
    if (stack.length === 0 || stack.at(-1).version !== entry.version) {
      throw new Error(`the Main history's withdraw to ${entry.version} doesn't follow from the entries before it`);
    }
  }
  return stack;
}

/**
 * The Main versions for pruneBuilds, oldest first: the replayed stack, so a
 * withdrawn build is not "the previous Main". null when the file isn't a
 * history that replays.
 */
export function mainVersions(history) {
  try { return replayMainStack(history).map(e => e.version); } catch { return null; }
}

/** The process's start time as `ps` prints it, or null when unknown. */
export function realStartTimeOf(pid) {
  try {
    const out = execFileSync('/bin/ps', ['-o', 'lstart=', '-p', String(pid)], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], timeout: 5000 }).trim();
    return out || null;
  } catch { return null; }
}

function realIsAlive(pid) {
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
}

/**
 * Takes the run lock: `{ ok: true }`, or `{ ok: false, holder }` when another
 * run holds it. A lock is taken over only when its holder is provably gone:
 * same host and its pid is not alive or now belongs to a process started at
 * another time than the recorded `pidStart` (pid reuse, e.g. after a
 * reboot), or the file is unreadable and older
 * than `staleMs`. A live pid is never taken over, however old (it is logged
 * by the caller; stopping a hung builder is a person's call). The takeover
 * renames the lock aside, checks the renamed file is the one it judged stale
 * (else puts it back and backs off), then creates a new one exclusively.
 * `afterJudge` is a test hook run between judging and renaming.
 */
export function acquireLock(path, { pid = process.pid, now = Date.now(), isAlive = realIsAlive, startTimeOf = realStartTimeOf, staleMs, host = hostname(), afterJudge = () => {} }) {
  try { mkdirSync(dirname(path), { recursive: true, mode: 0o700 }); } catch (e) { return { ok: false, holder: null, error: e.message }; }
  const body = JSON.stringify({ pid, pidStart: startTimeOf(pid), startedAt: new Date(now).toISOString(), host });
  let createError = null;
  const create = () => {
    try { writeFileSync(path, body, { flag: 'wx', mode: 0o600 }); return true; } catch (e) {
      if (e.code !== 'EEXIST') createError = e.message;
      return false;
    }
  };
  if (create()) return { ok: true, tookOver: null };
  if (createError) return { ok: false, holder: null, error: createError };
  let raw = null;
  let mtime = now;
  try { raw = readFileSync(path, 'utf8'); mtime = statSync(path).mtimeMs; } catch { return { ok: false, holder: null }; }
  let holder = null;
  try { holder = JSON.parse(raw); } catch { /* unreadable */ }
  const validPid = Number.isSafeInteger(holder?.pid) && holder.pid > 0;
  const sameHost = validPid && (!holder.host || holder.host === host);
  // A pid reused after a reboot (or by another program) is not the holder:
  // its start time differs from the one recorded. Unknown start time: trust isAlive.
  const startNow = sameHost ? startTimeOf(holder.pid) : null;
  const reused = sameHost && typeof holder.pidStart === 'string' && startNow !== null && startNow !== holder.pidStart;
  const dead = sameHost && (!isAlive(holder.pid) || reused);
  const garbage = !validPid && now - mtime > staleMs;
  if (!dead && !garbage) return { ok: false, holder, old: validPid && now - Date.parse(holder.startedAt ?? '') > staleMs };
  afterJudge();
  const aside = `${path}.stale-${pid}-${now}`;
  try { renameSync(path, aside); } catch { return { ok: false, holder }; }
  let moved = null;
  try { moved = readFileSync(aside, 'utf8'); } catch { /* checked below */ }
  if (moved !== raw) {
    // Someone replaced the lock between our read and the rename: put theirs back.
    try { linkSync(aside, path); } catch (e) {
      return { ok: false, holder: readJson(path, null), error: `a newer lock appeared; the replaced one is left at ${aside} (${e.code ?? e.message})` };
    }
    rmSync(aside, { force: true });
    return { ok: false, holder: readJson(path, null) };
  }
  rmSync(aside, { force: true });
  if (create()) return { ok: true, tookOver: holder ?? { pid: '?', startedAt: '?' } };
  return { ok: false, holder: readJson(path, null), error: createError ?? undefined };
}

export function releaseLock(path, pid = process.pid) {
  const holder = readJson(path, null);
  if (holder?.pid === pid) rmSync(path, { force: true });
}

/** The notification file for a failed run (Hollow's NotificationInput shape). */
export function writeFailureNotice(config, { commit, version, check, message, buildId, at }) {
  const short = commit ? commit.slice(0, 8) : 'unknown commit';
  const notice = {
    title: `Nightly not published: ${check} failed`,
    body: `The ${check} check failed on ${short}${version ? ` (${version})` : ''}: ${message}`.slice(0, 1000),
    kind: 'error',
    source: 'release.nightly',
    notifyNative: true,
    commit: commit ?? null,
    version: version ?? null,
    check,
    buildId: buildId ?? null,
    at,
  };
  const path = join(config.outbox, `failure-${commit && SHA.test(commit) ? commit : 'unknown'}.json`);
  writeJsonAtomic(path, notice);
  return path;
}

// ---------------------------------------------------------------- helpers

/** The schema map for the manifest, read from the checkout's source. */
export function readSchema(root) {
  const schema = {};
  for (const { store, file, constant } of SCHEMA_SOURCES) {
    let text;
    try { text = readFileSync(join(root, file), 'utf8'); } catch { throw new Error(`${file} is missing; update SCHEMA_SOURCES in scripts/release/nightly.mjs`); }
    if (constant === null) {
      if (/pragma_update\([^)]*"user_version"|PRAGMA user_version\s*=/.test(text)) {
        throw new Error(`${file} now sets user_version; name its constant in SCHEMA_SOURCES`);
      }
      schema[store] = 0;
      continue;
    }
    const m = new RegExp(`const ${constant}:\\s*[iu](?:32|64)\\s*=\\s*(\\d+)\\s*;`).exec(text);
    if (!m) throw new Error(`${file} has no ${constant} constant; update SCHEMA_SOURCES in scripts/release/nightly.mjs`);
    schema[store] = Number(m[1]);
  }
  return schema;
}

/** Tauri's static updater manifest plus Hollow's fields (Worker README). */
export function buildManifest({ version, notes, pubDate, signature, host, commit, buildId, schema, channel = CHANNEL }) {
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: { [PLATFORM]: { signature: signature.trim(), url: `https://${host}/${buildKey(version, 'Hollow.app.tar.gz')}` } },
    channel,
    commit,
    buildId,
    schema,
  };
}

const rfc3339 = (ms) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z');

/** Whether a gate record in `dirs` vouches for `fingerprint`; its path or null. */
export function findGateRecord(dirs, fingerprint) {
  if (!FINGERPRINT.test(fingerprint)) return null;
  for (const dir of dirs) {
    const path = join(dir, `${fingerprint}.json`);
    const record = readJson(path, null);
    if (record && record.gate === 'passed' && record.sourceFingerprint === fingerprint && record.dirty === false) return path;
  }
  return null;
}

const versionNumber = (v) => Number(v.split('.')[2]);

/**
 * Retention (design section 13, default 9): delete build folders beyond the
 * newest `keepNightly` versions, never touching what a channel manifest
 * points at or its previous build, nor the newest `keepMain` Mains.
 * `mainHistory` is every Main version in publish order (oldest first, as
 * promotion appends it); null means missing or unreadable. The previous
 * Nightly is the next lower version (Nightly versions only rise); the
 * previous Main is the entry before the current one in `mainHistory`. Nothing
 * is deleted when a channel manifest can't be read or parsed, or when Main
 * has a manifest but its history is missing, unreadable or doesn't hold it.
 */
export async function pruneBuilds({ uploader, keepNightly = DEFAULT_KEEP_NIGHTLY, keepMain = DEFAULT_KEEP_MAIN, mainHistory = [], log = () => {} }) {
  const live = {};
  for (const channel of CHANNELS) {
    const text = await uploader.get(channelKey(channel));
    if (text === null) continue;
    let manifest;
    try { manifest = JSON.parse(text); } catch { throw new Error(`${channelKey(channel)} is not JSON; nothing pruned`); }
    if (!isVersion(manifest?.version)) throw new Error(`${channelKey(channel)} has no valid version; nothing pruned`);
    live[channel] = manifest.version;
  }
  const history = Array.isArray(mainHistory) && mainHistory.every(isVersion) ? mainHistory : null;
  if (live.main && !history) throw new Error('channels/main.json exists but the Main history is missing or unreadable; nothing pruned');
  if (live.main && !history.includes(live.main)) throw new Error(`the Main history does not hold ${live.main}, which channels/main.json points at; nothing pruned`);
  const keys = await uploader.list('builds/');
  const byVersion = new Map();
  for (const key of keys) {
    const m = /^builds\/([^/]+)\/[^/]+$/.exec(key);
    if (!m || !isVersion(m[1])) continue;
    if (!byVersion.has(m[1])) byVersion.set(m[1], []);
    byVersion.get(m[1]).push(key);
  }
  const versions = [...byVersion.keys()].sort((a, b) => versionNumber(b) - versionNumber(a));
  const keep = new Set(versions.slice(0, keepNightly));
  if (live.nightly) {
    keep.add(live.nightly);
    const below = versions.find(v => versionNumber(v) < versionNumber(live.nightly));
    if (below) keep.add(below);
  }
  if (history) {
    for (const v of history.slice(-keepMain)) keep.add(v);
    if (live.main) {
      const i = history.lastIndexOf(live.main);
      keep.add(live.main);
      if (i > 0) keep.add(history[i - 1]);
    }
  }
  const deleted = [];
  for (const v of versions) {
    if (keep.has(v)) continue;
    for (const key of byVersion.get(v)) await uploader.delete(key);
    deleted.push(v);
  }
  if (deleted.length) log(`pruned ${deleted.length} old build(s): ${deleted.join(', ')}`);
  return { deleted, kept: versions.filter(v => keep.has(v)) };
}

// ---------------------------------------------------------------- default effects

function realGit(cwd, args) {
  try {
    return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: minutes(STEP_TIMEOUT_MINUTES.git), killSignal: 'SIGTERM', env: envWithout(process.env, [...SIGNING_VARS, ...R2_VARS, ...NOTARY_VARS]) }).trim();
  } catch (e) {
    const why = e.code === 'ETIMEDOUT' || e.signal === 'SIGTERM' ? `timed out after ${STEP_TIMEOUT_MINUTES.git} minutes` : String(e.stderr || e.message).trim().split('\n').slice(-2).join(' ');
    throw new Error(`git ${args[0]} failed: ${redact(why)}`);
  }
}

const groupAlive = (pgid) => {
  try { process.kill(-pgid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
};
const sleep = (ms) => new Promise(r => setTimeout(r, ms));

/**
 * Runs one step in its own process group (`detached`), so a time limit stops
 * everything it started: npm, hollow-slot, tauri, cargo, vitest, notarytool.
 * On timeout the whole group gets SIGTERM, then SIGKILL after `graceMs`, and
 * the step returns only once no process of the group is left. Only that path
 * reports "timed out"; any other signal is reported as itself. Long steps
 * stream their output straight into the daemon's log (inherited stdio); short
 * ones whose output is parsed are captured.
 */
export function runGrouped(command, args, { cwd, env = process.env, capture = false, timeout, graceMs = 10_000 } = {}) {
  return new Promise((resolveRun) => {
    let child;
    try {
      child = spawn(command, args, { cwd, env, detached: true, stdio: capture ? ['ignore', 'pipe', 'pipe'] : ['ignore', 'inherit', 'inherit'] });
    } catch (e) {
      resolveRun({ status: null, stdout: '', stderr: '', error: e.message });
      return;
    }
    const out = { stdout: '', stderr: '' };
    const cap = 1024 * 1024;
    if (capture) {
      for (const name of ['stdout', 'stderr']) {
        child[name].setEncoding('utf8');
        child[name].on('data', (d) => { if (out[name].length < cap) out[name] += d; });
      }
    }
    let timedOut = false;
    let settled = false;
    const kill = (sig) => { try { process.kill(-child.pid, sig); } catch { /* group gone */ } };
    const timer = timeout ? setTimeout(() => { timedOut = true; kill('SIGTERM'); }, timeout) : null;
    const finish = (result) => { if (!settled) { settled = true; clearTimeout(timer); resolveRun(result); } };
    child.on('error', (e) => finish({ status: null, ...out, error: e.message }));
    child.on('exit', async (code, signal) => {
      if (timedOut) {
        // Wait for the whole group to go: SIGTERM first, SIGKILL after the grace.
        const until = Date.now() + graceMs;
        while (groupAlive(child.pid) && Date.now() < until) await sleep(25);
        if (groupAlive(child.pid)) kill('SIGKILL');
        while (groupAlive(child.pid)) await sleep(25);
        finish({ status: null, ...out, error: `timed out after ${Math.round(timeout / 1000)} s`, timedOut: true });
        return;
      }
      // Let captured output drain, but don't wait on a grandchild holding the pipe.
      if (capture) await Promise.race([new Promise(r => child.once('close', r)), sleep(1000)]);
      finish({ status: signal ? null : code, ...out, error: signal ? `killed by ${signal}` : undefined });
    });
  });
}

/** `git fetch origin main` into the tracking ref (the CLAUDE.md fetch rule: never prune it away). */
export async function defaultFetch(git, repo, run = runGrouped) {
  // Fetch talks to the network through ssh or a credential helper, so it runs
  // in its own group under a time limit; the other git calls are local.
  const r = await run('git', ['fetch', '--no-tags', '--no-prune', 'origin', '+refs/heads/main:refs/remotes/origin/main'],
    { cwd: repo, env: envWithout(process.env, [...SIGNING_VARS, ...R2_VARS, ...NOTARY_VARS]), capture: true, timeout: minutes(STEP_TIMEOUT_MINUTES.git) });
  if (r.status !== 0) throw new Error(`git fetch failed: ${tail(r)}`);
}

export function defaultDeps(env = process.env) {
  return {
    git: realGit,
    run: runGrouped,
    fetch: defaultFetch,
    uploader: null, // made from env on first use
    makeUploader: () => createR2Uploader({ env }),
    now: () => Date.now(),
    pid: process.pid,
    isAlive: realIsAlive,
    startTimeOf: realStartTimeOf,
    host: hostname(),
    execPath: process.execPath,
  };
}

/**
 * `tauri signer sign`'s argv. The key and any password come from the
 * environment only; with no password set, an empty `--password ''` (tauri
 * CLI 2.11 has no `--no-password`; without it the CLI would prompt).
 */
export function signerArgs(file, env = process.env) {
  return ['--no-install', 'tauri', 'signer', 'sign', ...(set(env, 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD') ? [] : ['--password', '']), file];
}

// ---------------------------------------------------------------- the run

/**
 * One builder run. Returns `{ status: 'published' | 'skipped' | 'failed',
 * reason?, check?, commit?, version? }`; never throws for a failed step.
 */
export async function runNightly({ config, env = process.env, deps, argv = [], log = (line) => console.log(line) }) {
  const say = (line) => log(`[nightly] ${line}`);
  const now = () => deps.now();
  const lock = lockPath(config);
  const got = acquireLock(lock, { pid: deps.pid, now: now(), isAlive: deps.isAlive, startTimeOf: deps.startTimeOf ?? realStartTimeOf, staleMs: config.lockStaleMs, host: deps.host });
  if (!got.ok && got.error) {
    say(`could not take the lock: ${got.error}; skipping this run`);
    return { status: 'skipped', reason: 'locked' };
  }
  if (!got.ok) {
    say(`another run holds the lock (pid ${got.holder?.pid ?? '?'} since ${got.holder?.startedAt ?? '?'})${got.old ? '; it is older than the stale limit but its pid is alive, so it is left alone (stop it by hand if it hung)' : ''}; nothing to do`);
    return { status: 'skipped', reason: 'locked' };
  }
  if (got.tookOver) say(`took over a stale lock (pid ${got.tookOver.pid ?? '?'} since ${got.tookOver.startedAt ?? '?'})`);
  try {
    return await runLocked({ config, env, deps, argv, say });
  } finally {
    releaseLock(lock, deps.pid);
  }
}

async function runLocked({ config, env, deps, argv, say }) {
  const state = readJson(statePath(config), {});
  const git = (cwd, args) => deps.git(cwd, args);
  // Credentials reach only the step that needs them.
  const quiet = envWithout(env, [...SIGNING_VARS, ...R2_VARS, ...NOTARY_VARS]);
  const buildEnv = envWithout(env, [...SIGNING_VARS, ...R2_VARS]);

  let tip;
  try {
    await deps.fetch(git, config.repo);
    tip = git(config.repo, ['rev-parse', '--verify', 'refs/remotes/origin/main^{commit}']);
    if (!SHA.test(tip)) throw new Error('origin/main is not a commit');
  } catch (e) {
    // No commit to name, and a network blip shouldn't page anyone: log only.
    say(`fetch failed: ${e.message}`);
    return { status: 'failed', check: 'fetch', reason: e.message };
  }
  if (state.lastPublished?.commit === tip) {
    say(`origin/main ${tip.slice(0, 8)} is already the published Nightly ${state.lastPublished.version}; nothing to do`);
    return { status: 'skipped', reason: 'no new commit', commit: tip };
  }
  // The publish needs a Developer ID with a team, notarization and the updater
  // key; refuse before anything runs (and before any GET) without them.
  const preflight = () => {
    let signing;
    try { signing = resolveSigning(env); } catch (e) { return e.message; }
    if (signing.adHoc || !signing.teamId) return `a Nightly needs a Developer ID identity with a team (HOLLOW_SIGNING_IDENTITY); "${signing.identity}" is ${signing.adHoc ? 'ad hoc' : 'self-signed'}`;
    if (!signing.notarize) return 'a Nightly must be notarized: set APPLE_API_KEY, APPLE_API_ISSUER and APPLE_API_KEY_PATH';
    if (!set(env, 'TAURI_SIGNING_PRIVATE_KEY') && !set(env, 'TAURI_SIGNING_PRIVATE_KEY_PATH')) return 'the updater key is not configured (TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH)';
    if (!deps.uploader) {
      const missing = missingR2Config(env);
      if (missing.length) return `the uploader is not configured (${missing.join(', ')})`;
      try { deps.uploader = deps.makeUploader(); } catch (e) { return e.message; }
    }
    return null;
  };
  const refused = preflight();
  // The live manifest: a run that crashed after writing it but before
  // state.json is caught up here instead of re-uploading the same version.
  let live = null;
  if (!refused) {
    let liveText;
    try { liveText = await deps.uploader.get(channelKey(CHANNEL)); } catch (e) {
      say(`could not read the live ${channelKey(CHANNEL)}: ${redact(e.message)}; trying again next run`);
      return { status: 'failed', check: 'fetch', reason: e.message, commit: tip };
    }
    if (liveText !== null) {
      try { live = JSON.parse(liveText); } catch { say(`the live ${channelKey(CHANNEL)} is not JSON; the Worker serves no Nightly until this run replaces it`); }
    }
    if (live?.commit === tip && isVersion(live.version)) {
      const history = [{ version: live.version, commit: tip, at: live.pub_date ?? null }, ...(state.history ?? []).filter(h => h.version !== live.version)].slice(0, 100);
      writeJsonAtomic(statePath(config), { ...state, lastPublished: { commit: tip, version: live.version, buildId: live.buildId ?? null, at: live.pub_date ?? null }, lastFailed: null, history });
      say(`the live Nightly ${live.version} is already ${tip.slice(0, 8)}; recorded it and nothing to do`);
      return { status: 'skipped', reason: 'already live', commit: tip, version: live.version };
    }
  }
  // A failed tip waits for the next merge (or --retry): no rebuild every 5 minutes.
  if (state.lastFailed?.commit === tip && !argv.includes('--retry')) {
    say(`origin/main ${tip.slice(0, 8)} already failed (${state.lastFailed.check}); waiting for the next merge (--retry to try again)`);
    return { status: 'skipped', reason: 'already failed', commit: tip };
  }

  const startedAt = deps.now();
  const buildId = `nightly-${rfc3339(startedAt).replace(/[-:]/g, '')}-${tip.slice(0, 8)}`;
  let version = null;
  let current = 'config';
  const step = (name) => { current = name; say(`step ${name}`); };
  const ensure = (ok, message) => { if (!ok) throw new StepError(current, message); };
  const node = deps.execPath ?? process.execPath;
  const runIn = (cmd, args, opts) => deps.run(cmd, args, { cwd: config.worktree, ...opts });
  const wrapped = (cmd, args) => (config.wrap.length ? [config.wrap[0], [...config.wrap.slice(1), cmd, ...args]] : [cmd, args]);
  const buildApp = (args, limit) => runIn(node, ['scripts/release/build-app.mjs', '--ref', tip, ...args], { env: buildEnv, timeout: minutes(limit) });
  const why = (r) => (r.error ? redact(r.error) : `exit ${r.status ?? 'none'}`);

  try {
    say(`building ${tip.slice(0, 8)} (${buildId})`);
    ensure(refused === null, refused);

    step('checkout');
    if (existsSync(config.worktree)) {
      let top = null;
      try { top = git(config.worktree, ['rev-parse', '--show-toplevel']); } catch { /* not a checkout */ }
      ensure(top !== null && realpathSync(top) === realpathSync(config.worktree), `${config.worktree} exists and is not a worktree; remove it by hand`);
      git(config.worktree, ['checkout', '--force', '--detach', tip]);
      git(config.worktree, ['clean', '-ffdx', '-e', '/node_modules']);
    } else {
      git(config.repo, ['worktree', 'prune']);
      mkdirSync(dirname(config.worktree), { recursive: true });
      git(config.repo, ['worktree', 'add', '--force', '--detach', config.worktree, tip]);
    }
    ensure(git(config.worktree, ['rev-parse', 'HEAD']) === tip, 'the worktree is not at the tip');
    version = buildVersion(config.worktree, tip, git);
    say(`version ${version}`);
    // Never move the channel backwards, and never overwrite the bytes it serves.
    if (live && isVersion(live.version)) {
      if (versionNumber(version) <= versionNumber(live.version)) throw new StepError('manifest', `the live Nightly is ${live.version}, not lower than ${version}; refusing to publish`);
    }

    step('gate');
    const printed = await runIn(node, ['scripts/source-provenance.mjs', '--print'], { env: quiet, capture: true, timeout: minutes(STEP_TIMEOUT_MINUTES.fingerprint) });
    ensure(printed.status === 0, `source-provenance --print failed: ${tail(printed)}`);
    let fingerprint = null;
    try { fingerprint = JSON.parse(printed.stdout).sourceFingerprint; } catch { /* checked below */ }
    ensure(FINGERPRINT.test(fingerprint ?? ''), 'source-provenance --print gave no fingerprint');
    const commonDir = resolve(config.worktree, git(config.worktree, ['rev-parse', '--git-common-dir']));
    const record = findGateRecord([join(commonDir, 'hollow-gate-records'), ...config.gateRecordDirs], fingerprint);

    step('install');
    const [ic, ...ia] = config.installCommand;
    const installed = await runIn(ic, ia, { env: quiet, timeout: minutes(STEP_TIMEOUT_MINUTES.install) });
    ensure(installed.status === 0, `${config.installCommand.join(' ')} failed (${why(installed)})`);

    current = 'gate';
    if (record) {
      say(`gate record for ${fingerprint.slice(0, 12)}: ${record}`);
    } else {
      say(`no gate record for ${fingerprint.slice(0, 12)}; running ${config.gateCommand.join(' ')}`);
      const [gc, ga] = wrapped(config.gateCommand[0], config.gateCommand.slice(1));
      const gated = await runIn(gc, ga, { env: quiet, timeout: minutes(STEP_TIMEOUT_MINUTES.gate) });
      ensure(gated.status === 0, `${config.gateCommand.join(' ')} failed (${why(gated)})`);
    }

    step('build');
    const [bc, ba] = wrapped(node, ['scripts/release/build-app.mjs', '--ref', tip, '--no-verify', ...(config.dmg ? ['--dmg'] : [])]);
    const built = await runIn(bc, ba, { env: buildEnv, timeout: minutes(STEP_TIMEOUT_MINUTES.build) });
    ensure(built.status === 0, `the release build failed (${why(built)})`);
    const app = appPath(config.worktree, buildEnv);
    ensure(existsSync(join(app, 'Contents', 'Info.plist')), `no app at ${app}`);

    step('verify');
    const verified = await buildApp(['--verify-only', '--no-selftest'], STEP_TIMEOUT_MINUTES.verify);
    ensure(verified.status === 0, `the app failed verification (${why(verified)}); the build log above says which check`);

    step('selftest');
    const tested = await buildApp(['--selftest-only'], STEP_TIMEOUT_MINUTES.selftest);
    ensure(tested.status === 0, `the embed self-test failed (${why(tested)})`);

    step('sign');
    const out = join(config.stateDir, 'out', version);
    rmSync(out, { recursive: true, force: true });
    mkdirSync(out, { recursive: true, mode: 0o700 });
    const files = { 'Hollow.app.tar.gz': join(out, 'Hollow.app.tar.gz') };
    // --no-mac-metadata and COPYFILE_DISABLE: no AppleDouble `._` entries,
    // which the updater would unpack as files and break the signature.
    const tarred = await deps.run('/usr/bin/tar', ['--no-mac-metadata', '-czf', files['Hollow.app.tar.gz'], '-C', dirname(app), 'Hollow.app'], { cwd: out, env: { ...quiet, COPYFILE_DISABLE: '1' }, capture: true, timeout: minutes(STEP_TIMEOUT_MINUTES.tar) });
    ensure(tarred.status === 0 && existsSync(files['Hollow.app.tar.gz']), `tar failed: ${tail(tarred)}`);
    const signerEnv = { ...quiet };
    for (const k of SIGNING_VARS) if (set(env, k)) signerEnv[k] = env[k];
    const signed = await runIn('npx', signerArgs(files['Hollow.app.tar.gz'], env), { env: signerEnv, capture: true, timeout: minutes(STEP_TIMEOUT_MINUTES.sign) });
    ensure(signed.status === 0, `tauri signer sign failed (${why(signed)})`);
    files['Hollow.app.tar.gz.sig'] = `${files['Hollow.app.tar.gz']}.sig`;
    let signature = '';
    try { signature = readFileSync(files['Hollow.app.tar.gz.sig'], 'utf8').trim(); } catch { /* checked below */ }
    ensure(signature !== '', 'the signer wrote no signature');
    if (config.dmg) {
      const dmgDir = join(dirname(dirname(app)), 'dmg');
      const dmg = (existsSync(dmgDir) ? readdirSync(dmgDir) : []).find(n => n.endsWith('.dmg') && n.includes(`_${version}_`));
      ensure(Boolean(dmg), `--dmg was asked for but ${dmgDir} has no dmg for ${version}`);
      files['Hollow.dmg'] = join(out, 'Hollow.dmg');
      copyFileSync(join(dmgDir, dmg), files['Hollow.dmg']);
    }

    // Check the bytes that ship: unpack the archive and assess that copy.
    step('archive');
    const unpacked = join(out, 'unpacked');
    mkdirSync(unpacked, { recursive: true, mode: 0o700 });
    const capture = (cmd, args) => deps.run(cmd, args, { cwd: out, env: quiet, capture: true, timeout: minutes(STEP_TIMEOUT_MINUTES.archive) });
    const untar = await capture('/usr/bin/tar', ['-xzf', files['Hollow.app.tar.gz'], '-C', unpacked]);
    const shipped = join(unpacked, 'Hollow.app');
    ensure(untar.status === 0 && existsSync(join(shipped, 'Contents', 'Info.plist')), `the archive does not unpack to Hollow.app: ${tail(untar)}`);
    for (const [label, cmd, args] of [
      ['codesign --verify --deep --strict', '/usr/bin/codesign', ['--verify', '--deep', '--strict', shipped]],
      ['spctl --assess --type execute', '/usr/sbin/spctl', ['--assess', '--type', 'execute', shipped]],
      ['stapler validate', '/usr/bin/xcrun', ['stapler', 'validate', shipped]],
    ]) {
      const r = await capture(cmd, args);
      ensure(r.status === 0, `the unpacked archive fails ${label}: ${tail(r)}`);
    }
    rmSync(unpacked, { recursive: true, force: true });

    step('notes');
    let notes;
    try {
      notes = collectNotes({ git: (args) => git(config.worktree, args), from: state.lastPublished?.commit ?? null, to: tip });
    } catch (e) { throw new StepError('notes', e.message); }
    const text = notesText(notes);
    files['notes.json'] = join(out, 'notes.json');
    writeJsonAtomic(files['notes.json'], { version, channel: CHANNEL, commit: tip, buildId, previous: state.lastPublished ? { version: state.lastPublished.version, commit: state.lastPublished.commit } : null, ...notes, text });

    step('manifest');
    let schema;
    try { schema = readSchema(config.worktree); } catch (e) { throw new StepError('manifest', e.message); }
    const manifest = buildManifest({ version, notes: text, pubDate: rfc3339(deps.now()), signature, host: config.host, commit: tip, buildId, schema });
    const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;
    ensure(parseManifest(manifestText, CHANNEL) !== null, "the manifest fails the Worker's own check (parseManifest)");
    files[channelKey(CHANNEL)] = join(out, `${CHANNEL}.json`);
    writeFileSync(files[channelKey(CHANNEL)], manifestText, { mode: 0o600 });
    // The same bytes, kept with the build so promotion can publish any kept Nightly.
    files[BUILD_RECORD] = files[channelKey(CHANNEL)];

    step('upload');
    // Re-read the live manifest: never overwrite the version it serves.
    let nowLive = null;
    try { nowLive = JSON.parse((await deps.uploader.get(channelKey(CHANNEL))) ?? 'null'); } catch (e) {
      if (!(e instanceof SyntaxError)) throw new StepError('upload', `reading ${channelKey(CHANNEL)}: ${e.message}`);
    }
    ensure(nowLive?.version !== version, `the live Nightly already points at ${version}; refusing to overwrite builds/${version}/`);
    ensure(!isVersion(nowLive?.version) || versionNumber(nowLive.version) < versionNumber(version), `the live Nightly ${nowLive?.version} is newer than ${version}; refusing to move the channel back`);
    const types = { 'Hollow.app.tar.gz': 'application/gzip', 'Hollow.app.tar.gz.sig': 'text/plain; charset=utf-8', 'Hollow.dmg': 'application/x-apple-diskimage', [BUILD_RECORD]: 'application/json', 'notes.json': 'application/json' };
    for (const { key, kind } of publishOrder({ version, channels: [CHANNEL] })) {
      const name = kind === 'manifest' ? key : key.split('/').pop();
      if (!files[name]) { say(`skip ${key} (not built)`); continue; }
      try {
        await deps.uploader.put(key, readFileSync(files[name]), { contentType: kind === 'manifest' ? 'application/json' : types[name] });
      } catch (e) { throw new StepError('upload', `${key}: ${e.message}`); }
      say(`uploaded ${key} (${statSync(files[name]).size} bytes)`);
    }
  } catch (e) {
    const check = e instanceof StepError ? e.check : current;
    const message = e.message ?? String(e);
    say(`FAILED at ${check} on ${tip.slice(0, 8)}${version ? ` (${version})` : ''}: ${message}; nothing was published`);
    const at = rfc3339(deps.now());
    try {
      const path = writeFailureNotice(config, { commit: tip, version, check, message, buildId, at });
      say(`notification: ${path}`);
    } catch (err) { say(`could not write the notification: ${err.message}`); }
    writeJsonAtomic(statePath(config), { ...state, lastFailed: { commit: tip, version, check, at, buildId } });
    return { status: 'failed', check, reason: message, commit: tip, version };
  }

  const at = rfc3339(deps.now());
  const history = [{ version, commit: tip, at }, ...(state.history ?? [])].slice(0, 100);
  writeJsonAtomic(statePath(config), { ...state, lastPublished: { commit: tip, version, buildId, at }, lastFailed: null, history });
  say(`published Nightly ${version} (${tip.slice(0, 8)})`);
  // Prune under promote.mjs's lock, so no publish or withdraw moves Main (or
  // its history) between reading them and deleting.
  const promoteLock = promoteLockPath(config);
  const got = acquireLock(promoteLock, { pid: deps.pid, now: deps.now(), isAlive: deps.isAlive, startTimeOf: deps.startTimeOf ?? realStartTimeOf, staleMs: config.lockStaleMs, host: deps.host });
  if (!got.ok) {
    say('prune skipped: a publish to Main or withdraw is running (the build is published)');
    return { status: 'published', commit: tip, version };
  }
  try {
    // Missing or unreadable is not "no Mains": pruneBuilds refuses while Main exists.
    const mainHistory = existsSync(mainHistoryPath(config)) ? mainVersions(readJson(mainHistoryPath(config), null)) : null;
    await pruneBuilds({ uploader: deps.uploader, keepNightly: config.keepNightly, keepMain: config.keepMain, mainHistory, log: say });
  } catch (e) {
    say(`prune skipped: ${e.message} (the build is published)`);
  } finally {
    releaseLock(promoteLock, deps.pid);
  }
  return { status: 'published', commit: tip, version };
}

// realpath: /tmp is a symlink on macOS, and node runs the real file.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const argv = process.argv.slice(2);
  const unknown = argv.filter(a => a !== '--retry');
  if (unknown.length) {
    console.error(`[nightly] unknown argument ${unknown[0]} (options: --retry)`);
    process.exit(2);
  }
  let config;
  try { config = loadConfig(process.env); } catch (e) {
    console.error(`[nightly] bad configuration: ${e.message}`);
    process.exit(2);
  }
  const result = await runNightly({ config, deps: defaultDeps(process.env), argv, log: (l) => console.log(`${new Date().toISOString()} ${l}`) });
  process.exit(result.status === 'failed' ? 1 : 0);
}
