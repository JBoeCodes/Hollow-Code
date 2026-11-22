#!/usr/bin/env node
// One version source for Hollow (merged plan slice 6, item 5).
//
// `package.json` holds the release version. `src-tauri/tauri.conf.json` does not
// repeat it: its `"version"` is the path `"../package.json"`, which Tauri 2
// reads at build time (tauri-utils `PackageVersion`, resolved relative to the
// config file). Cargo cannot read package.json, so `src-tauri/Cargo.toml` and
// the `hollow` entry in `src-tauri/Cargo.lock` (behind `env!("CARGO_PKG_VERSION")`)
// carry a copy that this script keeps in sync, together with package-lock.json.
//
//   node scripts/version.mjs check          exit 1 on any drift (plan:check runs it in CI)
//   node scripts/version.mjs bump 0.5.4     rewrite every copy, then commit the result
//
//   node scripts/version.mjs stamp [ref]    print the built version, 0.6.<n> (below)
//
// Release builds carry a stamped version (updates design 2026-11-14, section 2):
// `0.6.<n>`, n = `git rev-list --count --first-parent <ref>` (default HEAD),
// applied at build time only (scripts/release/build-app.mjs), so the tree
// never gets dirty. Release builds are made from main's tip, where n is
// unique; a branch build counts its own first-parent history, so two lanes'
// branch builds can share an n. The committed copies stay on the 0.5.x
// baseline: `check` and `bump` refuse 0.6.x, which would collide with a stamp.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const TAURI_VERSION_REF = '../package.json';
const SEMVER = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
/** Committed versions stay below this; built apps carry `<STAMP_SERIES>.<n>`. */
export const STAMP_SERIES = '0.6';
const STAMP_FLOOR = `${STAMP_SERIES}.0`;
const isStampSeries = (v) => typeof v === 'string' && v.startsWith(`${STAMP_SERIES}.`);
/** A note (not a failure) for a committed version past the stamp series. */
export function baselineNote(version) {
  return SEMVER.test(version ?? '') && !isStampSeries(version) && compare(version, STAMP_FLOOR) > 0
    ? `note: package.json ${version} is above the ${STAMP_SERIES}.<n> release stamps; the baseline rule (committed 0.5.x, built ${STAMP_SERIES}.<n>) does not cover it, so change STAMP_SERIES with it`
    : null;
}
const CARGO_LOCK_ENTRY = /(\[\[package\]\]\nname = "hollow"\nversion = ")([^"]+)(")/;

const FILES = {
  pkg: 'package.json',
  lock: 'package-lock.json',
  tauri: 'src-tauri/tauri.conf.json',
  cargo: 'src-tauri/Cargo.toml',
  cargoLock: 'src-tauri/Cargo.lock',
};

/** The `version = "…"` line inside Cargo.toml's `[package]` table, as a match on the whole file. */
function cargoPackageVersion(toml) {
  const start = toml.search(/^\[package\]\s*$/m);
  if (start < 0) return null;
  const rest = toml.slice(start);
  const next = rest.slice(1).search(/^\[/m);
  const table = next < 0 ? rest : rest.slice(0, next + 1);
  const m = table.match(/^(version = ")([^"]+)(")$/m);
  return m ? { value: m[2], index: start + m.index, length: m[0].length, prefix: m[1], suffix: m[3] } : null;
}

/** Every place a version lives, keyed by the label `check` reports. */
export function readVersions(root) {
  const read = f => readFileSync(join(root, f), 'utf8');
  const pkg = JSON.parse(read(FILES.pkg));
  const lock = JSON.parse(read(FILES.lock));
  const tauri = JSON.parse(read(FILES.tauri));
  return {
    'package.json': pkg.version,
    'package-lock.json': lock.version,
    'package-lock.json root package': lock.packages?.['']?.version,
    'src-tauri/tauri.conf.json': tauri.version,
    'src-tauri/Cargo.toml': cargoPackageVersion(read(FILES.cargo))?.value,
    'src-tauri/Cargo.lock hollow package': read(FILES.cargoLock).match(CARGO_LOCK_ENTRY)?.[2],
  };
}

/** Human-readable problems; empty when every copy follows package.json. */
export function checkVersions(root) {
  const v = readVersions(root);
  const source = v['package.json'];
  const problems = [];
  if (typeof source !== 'string' || !SEMVER.test(source)) {
    problems.push(`package.json: version ${JSON.stringify(source)} is not x.y.z`);
  } else if (isStampSeries(source)) {
    problems.push(`package.json: ${source} collides with the ${STAMP_SERIES}.<n> release stamp; committed versions stay on the 0.5.x baseline (node scripts/version.mjs stamp)`);
  }
  if (v['src-tauri/tauri.conf.json'] !== TAURI_VERSION_REF) {
    problems.push(`src-tauri/tauri.conf.json: version must be ${JSON.stringify(TAURI_VERSION_REF)} (found ${JSON.stringify(v['src-tauri/tauri.conf.json'])}); package.json is the one source`);
  }
  for (const [label, value] of Object.entries(v)) {
    if (label === 'package.json' || label === 'src-tauri/tauri.conf.json') continue;
    if (value !== source) problems.push(`${label}: ${JSON.stringify(value)} != package.json ${JSON.stringify(source)}`);
  }
  return problems;
}

function compare(a, b) {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] - pb[i];
  return 0;
}

/**
 * Sets every copy to `next` and returns the files written. Refuses anything
 * that is not x.y.z, or lower than the current package.json version (the same
 * version re-syncs the copies). Never touches tauri.conf.json beyond pointing
 * it at package.json.
 *
 * Every file is read and checked before any is written, so a Cargo.toml or
 * Cargo.lock it can't update fails the bump with nothing changed rather than
 * leaving package.json bumped and the Cargo copies behind (wave-1 review).
 */
export function bumpVersion(root, next) {
  if (!SEMVER.test(next ?? '')) throw new Error(`version must be x.y.z, got ${JSON.stringify(next)}`);
  const path = f => join(root, f);
  const read = f => readFileSync(path(f), 'utf8');
  const current = JSON.parse(read(FILES.pkg)).version;
  if (isStampSeries(next)) {
    throw new Error(`refusing ${next}: ${STAMP_SERIES}.<n> is the release-build stamp; committed versions stay on the 0.5.x baseline`);
  }
  if (SEMVER.test(current ?? '') && compare(next, current) < 0) {
    throw new Error(`refusing to go backwards: ${next} < ${current}`);
  }
  const planned = [];
  const plan = (f, before, after) => { if (after !== before) planned.push([f, after]); };

  // npm writes both as 2-space JSON with a trailing newline, so a round trip
  // changes only the version fields.
  for (const f of [FILES.pkg, FILES.lock]) {
    const text = read(f);
    const json = JSON.parse(text);
    json.version = next;
    if (f === FILES.lock && json.packages?.['']) json.packages[''].version = next;
    plan(f, text, JSON.stringify(json, null, 2) + '\n');
  }

  const tauriText = read(FILES.tauri);
  const tauri = JSON.parse(tauriText);
  if (tauri.version !== TAURI_VERSION_REF) {
    const re = /^(\s*"version":\s*)"[^"]*"/m;
    if (!re.test(tauriText)) throw new Error(`${FILES.tauri}: no "version" key to point at package.json`);
    plan(FILES.tauri, tauriText, tauriText.replace(re, `$1${JSON.stringify(TAURI_VERSION_REF)}`));
  }

  const cargoText = read(FILES.cargo);
  const cv = cargoPackageVersion(cargoText);
  if (!cv) throw new Error(`${FILES.cargo}: no version in [package]`);
  plan(FILES.cargo, cargoText,
    cargoText.slice(0, cv.index) + cv.prefix + next + cv.suffix + cargoText.slice(cv.index + cv.length));

  const lockText = read(FILES.cargoLock);
  if (!CARGO_LOCK_ENTRY.test(lockText)) throw new Error(`${FILES.cargoLock}: no [[package]] entry for hollow`);
  plan(FILES.cargoLock, lockText, lockText.replace(CARGO_LOCK_ENTRY, `$1${next}$3`));

  for (const [f, text] of planned) writeFileSync(path(f), text);
  return planned.map(([f]) => f);
}

/** `0.6.<count>`; the count must be a whole number above zero. */
export function stampVersion(count) {
  const n = Number(count);
  if (!Number.isSafeInteger(n) || n < 1 || String(n) !== String(count).trim()) {
    throw new Error(`first-parent count must be a whole number above zero, got ${JSON.stringify(count)}`);
  }
  return `${STAMP_SERIES}.${n}`;
}

const realGit = (root, args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();

/**
 * The version a build of `ref` (default HEAD) carries: `0.6.<n>` with
 * n = `git rev-list --count --first-parent <ref>`. Only main's own line
 * counts, so merging a long branch adds one, and every machine gets the same
 * number for the same commit. `git` is injectable for tests.
 */
export function buildVersion(root, ref = 'HEAD', git = realGit) {
  if (typeof ref !== 'string' || !ref || ref.startsWith('-')) throw new Error(`bad ref ${JSON.stringify(ref)}`);
  // A shallow clone counts only the commits it has, so its number is too low.
  if (String(git(root, ['rev-parse', '--is-shallow-repository'])).trim() === 'true') {
    throw new Error('this checkout is a shallow clone, so its commit count is wrong; fetch the full history (git fetch --unshallow) before stamping a version');
  }
  return stampVersion(git(root, ['rev-list', '--count', '--first-parent', ref, '--']));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL('..', import.meta.url));
  const [cmd, arg] = process.argv.slice(2);
  try {
    if (cmd === 'check') {
      const problems = checkVersions(root);
      if (problems.length) {
        console.error('version: drift from package.json');
        for (const p of problems) console.error(`  ${p}`);
        console.error('Fix with: node scripts/version.mjs bump <x.y.z>');
        process.exit(1);
      }
      console.log(`version: ${readVersions(root)['package.json']} (package.json; tauri.conf.json, Cargo.toml, Cargo.lock and package-lock.json agree)`);
      const note = baselineNote(readVersions(root)['package.json']);
      if (note) console.error(`version: ${note}`);
    } else if (cmd === 'bump') {
      const written = bumpVersion(root, arg);
      const problems = checkVersions(root);
      if (problems.length) throw new Error(`still drifting after bump:\n  ${problems.join('\n  ')}`);
      console.log(`version: ${arg}${written.length ? `; wrote ${written.join(', ')}` : '; nothing to change'}`);
      console.log('Commit this in its own PR: the installer refuses a dirty tree. Built apps carry 0.6.<n> regardless (stamp).');
    } else if (cmd === 'stamp') {
      console.log(buildVersion(root, arg ?? 'HEAD'));
    } else {
      console.error('usage: node scripts/version.mjs check | bump <x.y.z> | stamp [ref]');
      process.exit(2);
    }
  } catch (e) {
    console.error(`version: ${e.message}`);
    process.exit(1);
  }
}
