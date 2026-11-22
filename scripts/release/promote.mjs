#!/usr/bin/env node
// Publish to Main and Withdraw (updates design 2026-11-14, sections 9 and 10,
// row U8). Runs on the release builder's machine (the Mac mini, as
// `runner`), where the R2 write credential lives, started by Hollow over
// its runner connection (src-tauri/src/update_client/publish.rs) or by hand:
//
//   node scripts/release/promote.mjs status [--eligible v,v] [--pick v]
//   node scripts/release/promote.mjs list-nightlies
//   node scripts/release/promote.mjs promote <0.6.n> [--expect-commit <sha>] [--expect-main <0.6.n|none>] [--force-older]
//   node scripts/release/promote.mjs withdraw [--expect-version <0.6.n>]
//
// No rebuild: a promotion copies the build's stored manifest record
// (`builds/<v>/manifest.json`, written by nightly.mjs) to `channels/main.json`
// with `channel: "main"`, so Main serves the same signed bytes Nightly did.
// Before that one write it checks every file the Worker needs is in the
// bucket and that the stored `.sig` is the record's signature; the channel
// manifest is the only write, and the last. Then it appends to
// `main-history.json` in the builder's state folder (oldest first; nightly.mjs
// prunes by it): `{ version, commit, promotedAt, from, action }`.
//
// Rules (decided 2026-11-15, row U8):
// - promote refuses the version Main already serves, a version lower than
//   the live Main unless `--force-older` (Withdraw is the way back; Hollow
//   never passes it), and, with `--expect-main`, a Main that moved since the
//   caller looked.
// - withdraw points Main at the Main before the live one, replaying the
//   history (a publish pushes, a withdraw pops). It always moves to that
//   older build, so it needs no `--force-older`. It refuses when there is no
//   earlier Main (an empty Main channel would offer teammates nothing) and,
//   with `--expect-version`, when Main moved since the caller looked.
// - A crash between the `channels/main.json` write and the history append
//   is repaired by the next promote or withdraw (`reconcile`): promote of
//   the version Main already serves adds the missing entry and nothing
//   else. A main.json that is no kept build's record is refused.
// - Main's `notes` say what is new since the previous Main (notes.mjs from
//   the previous Main's commit, in the builder's clone); when that can't be
//   worked out, the build's own notes stay.
//
// Output: progress on stderr; the result as ONE stdout line,
// `HOLLOW_PROMOTE_RESULT <json>`, which Hollow parses. Exit 0 when done, 1
// when refused or failed (the result says why), 2 for bad usage or config.
// Secrets come from the environment only (the builder's env file), are never
// printed and never in argv.
import { execFileSync } from 'node:child_process';
import { existsSync, realpathSync } from 'node:fs';
import { hostname } from 'node:os';
import { fileURLToPath } from 'node:url';
import { collectNotes, notesText } from './notes.mjs';
import { createR2Uploader, missingR2Config } from './r2-uploader.mjs';
import { acquireLock, envWithout, loadConfig, mainHistoryPath, promoteLockPath, readJson, redact, releaseLock, replayMainStack, writeJsonAtomic, R2_VARS, SIGNING_VARS } from './nightly.mjs';
import { BUILD_RECORD, PLATFORM, buildKey, buildRecordKey, channelKey, isVersion } from '../../infra/updates-worker/src/layout.js';
import { parseManifest } from '../../infra/updates-worker/src/manifest.js';
import { manifestPrerequisites } from '../../infra/updates-worker/src/publish-order.js';

export const RESULT_PREFIX = 'HOLLOW_PROMOTE_RESULT ';
/** Nudge thresholds (design section 10): Nightly 3 days or 20 changes ahead. */
export const AHEAD_DAYS = 3;
export const AHEAD_CHANGES = 20;
const SHA = /^[0-9a-f]{40}$/;

export { promoteLockPath };
const versionNumber = (v) => Number(v.split('.')[2]);

/** A refusal: nothing was written. `code` is a stable word for the caller. */
export class Refused extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

// ---------------------------------------------------------------- history

/**
 * `main-history.json` as a list of entries, oldest first. Missing is []; a
 * file that exists but isn't a list of entries (or of version strings, the
 * shape before U8) throws, so nothing is written over a history a person
 * should look at.
 */
export function readHistory(path) {
  if (!existsSync(path)) return [];
  const raw = readJson(path, undefined);
  if (!Array.isArray(raw)) throw new Refused('history', `${path} is not a list; fix it by hand before publishing`);
  return raw.map((e) => {
    if (typeof e === 'string' && isVersion(e)) return { version: e, commit: null, promotedAt: null, from: null, action: 'publish' };
    if (e && typeof e === 'object' && isVersion(e.version) && (e.action === 'publish' || e.action === 'withdraw')) return e;
    throw new Refused('history', `${path} holds an entry that isn't a Main publish or withdraw; fix it by hand`);
  });
}

/**
 * The Main stack the history describes: a publish pushes its version, a
 * withdraw pops the live one (its entry names the version Main went back
 * to, which must be the new top). Returns the stack, oldest first.
 */
export function mainStack(history) {
  try { return replayMainStack(history); } catch (e) { throw new Refused('history', `${e.message}; fix it by hand`); }
}

/**
 * The history entry a crash left out: promote.mjs writes `channels/main.json`
 * first and the history after it, so a run stopped between the two leaves
 * Main serving a version the history doesn't end on. Returns `{ repairs,
 * stack }`: the missing entry (none when the two agree) and the stack with
 * it. A live Main one below the history's top is a withdraw that wasn't
 * recorded; any other is a publish, accepted only when the live manifest is
 * a kept build's record (same commit). A live Main with an empty history
 * (seeded by hand, or its record pruned) is recorded as the base of the stack
 * (`seeded: true`); a record that can't be read at all (a transient error)
 * refuses instead of seeding. Anything else is refused: a person changed main.json by
 * hand and should look.
 */
export async function reconcile(uploader, history, live, { at, host }) {
  const stack = mainStack(history);
  const top = stack.at(-1) ?? null;
  if (!live || top?.version === live.version) return { repairs: [], stack };
  let entry;
  if (stack.length >= 2 && stack.at(-2).version === live.version) {
    entry = { version: live.version, commit: live.commit, promotedAt: at, from: top.version, action: 'withdraw', by: host, repaired: true };
  } else {
    let record = null;
    try {
      record = await readRecord(uploader, live.version);
    } catch (e) {
      // A record that is missing or isn't a valid manifest is an answer
      // (refused below, or seeded on an empty history). A read that failed
      // is not: with an empty history it would seed Main as the stack's base
      // over a record that was only unreachable, so it refuses and the run
      // can be retried.
      if (!(e instanceof Refused && (e.code === 'no-record' || e.code === 'bad-record'))) {
        throw new Refused('read-error', `the build record of ${live.version} couldn't be read (${e.message}); nothing was changed, try again`);
      }
    }
    if (!record && stack.length === 0) {
      // A Main seeded by hand before any publish went through here (or whose
      // build record was pruned) has no history to disagree with: it becomes
      // the base of the stack, so the next publish and Withdraw work. Nothing
      // below it exists to go back to, so Withdraw still refuses on it.
      entry = { version: live.version, commit: live.commit, promotedAt: at, from: null, action: 'publish', by: host, repaired: true, seeded: true };
    } else if (!record || !live.commit || record.commit !== live.commit) {
      throw new Refused('history', `the Main history's latest is ${top?.version ?? 'nothing'}, but channels/main.json serves ${live.version}, which isn't a kept build's record; fix it by hand`);
    } else {
      entry = { version: live.version, commit: live.commit, promotedAt: at, from: top?.version ?? null, action: 'publish', by: host, repaired: true };
    }
  }
  return { repairs: [entry], stack: mainStack([...history, entry]) };
}

// ---------------------------------------------------------------- bucket reads

async function readJsonKey(uploader, key) {
  const text = await uploader.get(key);
  if (text === null) return { text: null, data: null };
  try { return { text, data: JSON.parse(text) }; } catch { return { text, data: null }; }
}

/** The live channel manifest's summary, or null when the channel is empty or unreadable. */
async function liveSummary(uploader, channel) {
  const { data } = await readJsonKey(uploader, channelKey(channel));
  if (!data || !isVersion(data.version)) return null;
  return { version: data.version, commit: SHA.test(data.commit ?? '') ? data.commit : null, pubDate: typeof data.pub_date === 'string' ? data.pub_date : null };
}

/**
 * A build's stored record, checked as the Nightly manifest it was published
 * as. Throws Refused when missing (published before U8, or pruned) or bad.
 */
export async function readRecord(uploader, version) {
  const text = await uploader.get(buildRecordKey(version));
  if (text === null) throw new Refused('no-record', `builds/${version}/${BUILD_RECORD} is missing: ${version} was published before builds kept their manifest, or it was pruned, so it can't be promoted`);
  const record = parseManifest(text, 'nightly');
  if (!record || record.version !== version) throw new Refused('bad-record', `builds/${version}/${BUILD_RECORD} isn't a valid Nightly manifest for ${version}`);
  return record;
}

/**
 * Every file the Worker needs for `version` is in the bucket and the stored
 * signature is the record's. Throws Refused naming the first problem.
 */
export async function verifyBuildFiles(uploader, version, record) {
  const keys = new Set(await uploader.list(`builds/${version}/`));
  for (const key of manifestPrerequisites(version)) {
    if (!keys.has(key)) throw new Refused('missing-file', `${key} is not in the bucket; nothing was published`);
  }
  const sig = await uploader.get(buildKey(version, 'Hollow.app.tar.gz.sig'));
  if (sig === null || sig.trim() !== record.platforms[PLATFORM].signature.trim()) {
    throw new Refused('signature', `builds/${version}/Hollow.app.tar.gz.sig doesn't match the build's manifest; nothing was published`);
  }
}

/** The kept builds that have a record: `{ version, commit, pubDate }`, newest first. */
export async function listNightlies(uploader) {
  const keys = await uploader.list('builds/');
  const versions = [...new Set(keys.map(k => /^builds\/([^/]+)\/manifest\.json$/.exec(k)?.[1]).filter(isVersion))];
  versions.sort((a, b) => versionNumber(b) - versionNumber(a));
  const out = [];
  for (const version of versions) {
    const { data } = await readJsonKey(uploader, buildRecordKey(version));
    if (!data || data.version !== version) continue;
    out.push({ version, commit: SHA.test(data.commit ?? '') ? data.commit : null, pubDate: typeof data.pub_date === 'string' ? data.pub_date : null });
  }
  return out;
}

// ---------------------------------------------------------------- notes

/** What `to` brings since `from` (null: no earlier Main), or null when git can't say. */
export function notesBetween(git, from, to) {
  if (!git || !SHA.test(to ?? '')) return null;
  try { return collectNotes({ git, from: from ?? null, to }); } catch { return null; }
}

/** Main's manifest for a build: its record with `channel: "main"` and notes since the previous Main. */
export function mainManifestFor(record, notes) {
  const manifest = { ...record, channel: 'main' };
  if (notes) manifest.notes = notesText(notes);
  return manifest;
}

// ---------------------------------------------------------------- commands

/**
 * status: the live channels, the kept builds, the history, the proposed
 * build and what it brings since the live Main. `eligible`: versions the
 * caller's Mac ran long enough (newest wins); `pick`: the person's choice.
 */
export async function status({ uploader, history, git, eligible = [], pick = null, now = Date.now() }) {
  const [nightly, main] = [await liveSummary(uploader, 'nightly'), await liveSummary(uploader, 'main')];
  const builds = await listNightlies(uploader);
  const known = new Set(builds.map(b => b.version));
  const above = (v) => !main || versionNumber(v) > versionNumber(main.version);
  let proposed = null;
  let proposedWhy = null;
  if (pick !== null) {
    if (!isVersion(pick) || !known.has(pick)) proposedWhy = `${pick} isn't a kept build with a manifest record`;
    else proposed = pick;
  } else {
    const ok = eligible.filter(v => isVersion(v) && known.has(v) && above(v)).sort((a, b) => versionNumber(b) - versionNumber(a));
    if (ok.length) proposed = ok[0];
    else if (!builds.some(b => above(b.version))) proposedWhy = 'Main already has the newest kept build';
    else proposedWhy = 'No newer build has run on this Mac for an hour without a go back; pick one';
  }
  const build = proposed ? builds.find(b => b.version === proposed) : null;
  const notes = build ? notesBetween(git, main?.commit ?? null, build.commit) : null;
  // How far the live Nightly is ahead of Main (the nudge).
  const newer = builds.filter(b => above(b.version));
  const ahead = nightly && main && nightly.version !== main.version ? notesBetween(git, main.commit, nightly.commit) : null;
  const dates = newer.map(b => Date.parse(b.pubDate ?? '')).filter(Number.isFinite);
  const behindSince = dates.length ? new Date(Math.min(...dates)).toISOString().replace(/\.\d{3}Z$/, 'Z') : null;
  const changesAhead = ahead ? ahead.pullRequests.length + ahead.commits.length : null;
  const daysAhead = behindSince ? Math.floor((now - Date.parse(behindSince)) / 86_400_000) : null;
  // With the entry a crash left out, as the next publish or withdraw adds it.
  let stack = null;
  try { ({ stack } = await reconcile(uploader, history, main, { at: null, host: null })); } catch { /* reported by canWithdraw */ }
  const previousMain = stack && stack.length >= 2 ? stack.at(-2).version : null;
  return {
    ok: true,
    nightly,
    main,
    builds,
    history,
    proposed,
    proposedWhy,
    notes,
    changesAhead,
    behindSince,
    daysAhead,
    nudge: newer.length > 0 && ((changesAhead !== null && changesAhead >= AHEAD_CHANGES) || (daysAhead !== null && daysAhead >= AHEAD_DAYS)),
    previousMain,
    canWithdraw: Boolean(main && stack && stack.at(-1)?.version === main.version && previousMain),
  };
}

/** Main must be what the caller saw: `expectMain` is a version or 'none'. */
function checkExpectedMain(live, expectMain) {
  if (expectMain === null) return;
  const now = live?.version ?? 'none';
  if (now !== expectMain) throw new Refused('moved', `Main is ${now === 'none' ? 'empty' : now} now, not ${expectMain === 'none' ? 'empty' : expectMain}; look again`);
}

/** promote <version>: see the header. Returns the result; throws Refused. */
export async function promote({ uploader, historyPath, git, version, expectCommit = null, expectMain = null, forceOlder = false, now = Date.now(), host = hostname() }) {
  if (!isVersion(version)) throw new Refused('usage', `${version} is not a version (0.6.<n>)`);
  const history = readHistory(historyPath);
  const live = await liveSummary(uploader, 'main');
  checkExpectedMain(live, expectMain);
  const at = new Date(now).toISOString().replace(/\.\d{3}Z$/, 'Z');
  const { repairs } = await reconcile(uploader, history, live, { at, host });
  if (live?.version === version) {
    if (!repairs.length) throw new Refused('unchanged', `Main already serves ${version}`);
    // A crash left Main serving it without its history entry: add the entry only.
    if (expectCommit !== null && live.commit !== expectCommit) {
      throw new Refused('commit', `${version} is commit ${String(live.commit).slice(0, 12)}, not the approved ${String(expectCommit).slice(0, 12)}; nothing was written`);
    }
    const historyError = appendHistory(historyPath, [...history, ...repairs]);
    const [entry] = repairs;
    return { ok: true, action: entry.action, version, commit: live.commit, from: entry.from, at, repaired: true, historyError };
  }
  if (live && versionNumber(version) < versionNumber(live.version) && !forceOlder) {
    throw new Refused('older', `${version} is older than the live Main ${live.version}; use Withdraw to go back, or --force-older`);
  }
  const record = await readRecord(uploader, version);
  if (expectCommit !== null && record.commit !== expectCommit) {
    throw new Refused('commit', `${version} is commit ${String(record.commit).slice(0, 12)}, not the approved ${String(expectCommit).slice(0, 12)}; nothing was published`);
  }
  await verifyBuildFiles(uploader, version, record);
  const manifest = mainManifestFor(record, notesBetween(git, live?.commit ?? null, record.commit));
  const text = `${JSON.stringify(manifest, null, 2)}\n`;
  if (!parseManifest(text, 'main')) throw new Refused('bad-record', "Main's manifest fails the Worker's own check");
  await uploader.put(channelKey('main'), Buffer.from(text), { contentType: 'application/json' });
  const entry = { version, commit: record.commit ?? null, promotedAt: at, from: live?.version ?? null, action: 'publish', by: host };
  const historyError = appendHistory(historyPath, [...history, ...repairs, entry]);
  return { ok: true, action: 'publish', version, commit: record.commit ?? null, from: live?.version ?? null, at, repaired: repairs.length > 0, historyError };
}

/** withdraw: see the header. Returns the result; throws Refused. */
export async function withdraw({ uploader, historyPath, git, expectVersion = null, now = Date.now(), host = hostname() }) {
  const history = readHistory(historyPath);
  const live = await liveSummary(uploader, 'main');
  if (!live) throw new Refused('no-main', 'Main has no build to withdraw');
  if (expectVersion !== null && live.version !== expectVersion) throw new Refused('moved', `Main is ${live.version} now, not ${expectVersion}; look again`);
  const at = new Date(now).toISOString().replace(/\.\d{3}Z$/, 'Z');
  // A crash's missing entry is added first, with this withdraw's own.
  const { repairs, stack } = await reconcile(uploader, history, live, { at, host });
  if (stack.length < 2) throw new Refused('no-previous', `${live.version} is the only Main published; there is no earlier Main to go back to`);
  const back = stack.at(-2);
  const record = await readRecord(uploader, back.version);
  await verifyBuildFiles(uploader, back.version, record);
  const before = stack.length >= 3 ? stack.at(-3).commit : null;
  const manifest = mainManifestFor(record, notesBetween(git, before, record.commit));
  const text = `${JSON.stringify(manifest, null, 2)}\n`;
  if (!parseManifest(text, 'main')) throw new Refused('bad-record', "Main's manifest fails the Worker's own check");
  await uploader.put(channelKey('main'), Buffer.from(text), { contentType: 'application/json' });
  const entry = { version: back.version, commit: record.commit ?? null, promotedAt: at, from: live.version, action: 'withdraw', by: host };
  const historyError = appendHistory(historyPath, [...history, ...repairs, entry]);
  return { ok: true, action: 'withdraw', version: back.version, commit: record.commit ?? null, from: live.version, at, repaired: repairs.length > 0, historyError };
}

/** Check the list still replays, then save it; the error text when it couldn't (the manifest is already live). */
function appendHistory(path, entries) {
  try { mainStack(entries); writeJsonAtomic(path, entries); return null; } catch (e) {
    return `channels/main.json was written but ${path} wasn't (${e.message}); the next publish or withdraw adds the entry`;
  }
}

// ---------------------------------------------------------------- command line

export function parseArgs(argv) {
  const [command, ...rest] = argv;
  const out = { command, version: null, eligible: [], pick: null, expectCommit: null, expectVersion: null, expectMain: null, forceOlder: false };
  if (!['status', 'list-nightlies', 'promote', 'withdraw'].includes(command)) throw new Refused('usage', 'commands: status, list-nightlies, promote <version>, withdraw');
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    const value = () => {
      const v = rest[++i];
      if (v === undefined) throw new Refused('usage', `${a} needs a value`);
      return v;
    };
    if (a === '--eligible') out.eligible = value().split(',').map(s => s.trim()).filter(Boolean);
    else if (a === '--pick') out.pick = value();
    else if (a === '--expect-commit') out.expectCommit = value();
    else if (a === '--expect-version') out.expectVersion = value();
    else if (a === '--expect-main' && command === 'promote') out.expectMain = value();
    else if (a === '--force-older') out.forceOlder = true;
    else if (!a.startsWith('--') && command === 'promote' && out.version === null) out.version = a;
    else throw new Refused('usage', `unknown argument ${a}`);
  }
  if (command === 'promote' && out.version === null) throw new Refused('usage', 'promote needs a version');
  if (out.expectCommit !== null && !SHA.test(out.expectCommit)) throw new Refused('usage', '--expect-commit needs a 40-character commit');
  if (out.expectVersion !== null && !isVersion(out.expectVersion)) throw new Refused('usage', '--expect-version needs a version');
  if (out.expectMain !== null && out.expectMain !== 'none' && !isVersion(out.expectMain)) throw new Refused('usage', "--expect-main needs a version or 'none'");
  return out;
}

/** One run: parse, lock (writes only), act. Returns `{ code, result }`; never throws. */
export async function main({ argv, config, uploader, git, now = () => Date.now(), pid = process.pid, host = hostname(), log = (l) => process.stderr.write(`${l}\n`) }) {
  let args;
  try { args = parseArgs(argv); } catch (e) { return { code: 2, result: { ok: false, code: e.code ?? 'usage', error: e.message } }; }
  const historyPath = mainHistoryPath(config);
  const writes = args.command === 'promote' || args.command === 'withdraw';
  const lock = promoteLockPath(config);
  if (writes) {
    const got = acquireLock(lock, { pid, now: now(), staleMs: config.lockStaleMs, host });
    if (!got.ok) return { code: 1, result: { ok: false, code: 'busy', error: 'Another publish or withdraw is running on the builder; try again in a minute' } };
  }
  try {
    let result;
    if (args.command === 'list-nightlies') result = { ok: true, builds: await listNightlies(uploader) };
    else if (args.command === 'status') {
      let history = [];
      try { history = readHistory(historyPath); } catch (e) { log(`[promote] ${e.message}`); }
      result = await status({ uploader, history, git, eligible: args.eligible, pick: args.pick, now: now() });
    } else if (args.command === 'promote') {
      result = await promote({ uploader, historyPath, git, version: args.version, expectCommit: args.expectCommit, expectMain: args.expectMain, forceOlder: args.forceOlder, now: now(), host });
      log(`[promote] Main now serves ${result.version}${result.from ? ` (was ${result.from})` : ''}`);
    } else {
      result = await withdraw({ uploader, historyPath, git, expectVersion: args.expectVersion, now: now(), host });
      log(`[promote] withdrew ${result.from}; Main serves ${result.version} again`);
    }
    return { code: 0, result };
  } catch (e) {
    if (e instanceof Refused) return { code: 1, result: { ok: false, code: e.code, error: e.message } };
    return { code: 1, result: { ok: false, code: 'failed', error: redact(e.message ?? String(e)) } };
  } finally {
    if (writes) releaseLock(lock, pid);
  }
}

// realpath: /tmp is a symlink on macOS, and node runs the real file.
if (process.argv[1] && realpathSync(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const emit = (code, result) => {
    process.stdout.write(`${RESULT_PREFIX}${JSON.stringify(result)}\n`);
    process.exit(code);
  };
  let config;
  try { config = loadConfig(process.env); } catch (e) { emit(2, { ok: false, code: 'config', error: `bad configuration: ${e.message}` }); }
  const missing = missingR2Config(process.env);
  if (missing.length) emit(2, { ok: false, code: 'config', error: `the builder's R2 settings are missing (${missing.join(', ')})` });
  let uploader;
  try { uploader = createR2Uploader({ env: process.env }); } catch (e) { emit(2, { ok: false, code: 'config', error: e.message }); }
  const quiet = envWithout(process.env, [...SIGNING_VARS, ...R2_VARS]);
  const git = (args) => execFileSync('git', args, { cwd: config.repo, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 60_000, env: quiet });
  const { code, result } = await main({ argv: process.argv.slice(2), config, uploader, git });
  emit(code, result);
}
