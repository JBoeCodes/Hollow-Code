#!/usr/bin/env node
// No new tight wall-clock upper bound in a Rust test (S23, 2026-10-17).
//
//   node scripts/check-timing-asserts.mjs        (also run by npm run test:scripts)
//
// `assert!(started.elapsed() < Duration::from_millis(550))` passes on an idle
// Mac and fails when two lanes gate at once: the bound measures the machine's
// load as much as the code. A pure-timer test (tokio sleeps and timeouts only,
// measured with `tokio::time::Instant`) could use the paused clock
// (`#[tokio::test(start_paused = true)]`; tokio's `test-util` isn't a dev
// feature yet, add it with the first such test); a test around a real
// process, socket, file or thread, or code that reads `std::time::Instant`,
// should assert the cause (the timeout flag, the result variant, a gate only
// the test opens) and keep only a wide "no hang" bound, ≥ 10× the expected
// time.
//
// A text check over src-tauri/src, every folder deep. A line is an offender
// when, inside an `assert!(…)` (on the line, or the macro opened up to four
// lines above), it puts a small upper bound on an elapsed time:
// - an elapsed time (`elapsed()`, or a name bound by `let name = ….elapsed()…;`
//   in that file) against `< Duration::from_millis(N)` with N < 2000 or
//   `< Duration::from_secs(N)` with N < 5 (also `<=`, the reversed
//   `Duration::from_millis(N) > …`, and the end of a range
//   `(… .. Duration::from_millis(N)).contains(&…)`);
// - `.as_millis() < N` / `.as_secs() < N` with the same limits, and
//   `.as_secs_f64() < X` (or `f32`) with X < 5;
// - a bare number bounding a millisecond operand, a `.ms` or `…_ms` field
//   (`mark.unwrap().ms < 700`), or a name bound from `.elapsed()` in that file
//   (`let ms = t.elapsed().as_millis(); assert!(ms < 500)`): N < 2000, or
//   X < 5 when that `let` reads `.as_secs…()`;
// - the end of an integer or float range holding such an operand:
//   `(150..1200).contains(&mark.ms)` (B < 2000 for milliseconds, < 5 for
//   seconds).
// Comment lines are skipped.
//
// A bound that has to be tight says why: `// timing: <reason>` on the line or
// on the line directly above accepts it. The offenders left when the check
// landed are in scripts/timing-asserts-allowlist.json, keyed by path and the
// trimmed line text (not line numbers, so edits elsewhere don't break it). The
// list may only shrink: a new offender fails, and so does an entry no line
// matches any more (delete it).
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export const SRC = 'src-tauri/src';
export const ALLOWLIST = 'scripts/timing-asserts-allowlist.json';
/** Upper bounds below these are "small". */
export const SMALL_MILLIS = 2000;
export const SMALL_SECS = 5;

const NUM = '(\\d[\\d_]*)';
// An integer or a float (`1.5`), for seconds.
const FLOAT = '(\\d[\\d_]*(?:\\.\\d+)?)';
// Not followed by more of a number (so `1_500` and `5.0` are read whole).
const END = '(?![\\d_.])';
const DURATION = `(?:std::time::|time::)?Duration::from_(millis|secs)\\(\\s*${NUM}\\s*\\)`;
// Each captures (unit, number). Counted only with an elapsed operand on the
// line (or ending the line above).
const BOUNDS = [
  // x < Duration::from_millis(N), x <= …
  new RegExp(`(?<![<=>-])<=?\\s*${DURATION}`, 'g'),
  // Duration::from_millis(N) > x, … >= x
  new RegExp(`${DURATION}\\s*>`, 'g'),
  // x.as_millis() < N, x.as_secs() <= N
  new RegExp(`\\.as_(millis|secs)\\(\\)\\s*<=?\\s*${NUM}${END}`, 'g'),
  // (Duration::from_millis(A)..Duration::from_millis(N)).contains(&x)
  new RegExp(`\\.\\.=?\\s*${DURATION}\\s*\\)\\s*\\.contains\\(`, 'g'),
];
// Each carries its own operand, so no elapsed() is needed elsewhere on the
// line; each captures (number), its unit fixed.
const OWN_BOUNDS = [
  // x.as_secs_f64() < 1.5, … as_secs_f32() <= X
  [new RegExp(`\\.as_secs_f(?:32|64)\\(\\)\\s*<=?\\s*${FLOAT}${END}`, 'g'), 'secs'],
  // mark.unwrap().ms < 700, map.took_ms <= 1_500
  [new RegExp(`(?:\\.ms|\\w_ms)\\s*<=?\\s*${NUM}${END}`, 'g'), 'millis'],
];
// (150..1200).contains(&…), (0.0..=2.5).contains(&…): captures (end); the
// operand follows the match.
const RANGE = new RegExp(`\\(\\s*${FLOAT}\\s*\\.\\.=?\\s*${FLOAT}\\s*\\)\\s*\\.contains\\(\\s*&`, 'g');
const ASSERT = /\b(?:debug_)?assert!\s*\(/;
const TIMING = /\/\/\s*timing:\s*\S/;
// `let took = started.elapsed();`, `let waited: Duration = t.elapsed();`,
// `let ms = t.elapsed().as_millis();`: (name, right-hand side).
const ELAPSED_LET = /\blet\s+(?:mut\s+)?([A-Za-z_]\w*)\s*(?::[^=;\n]*)?=([^;\n]*\.elapsed\(\)[^;\n]*)/g;

function small(unit, raw) {
  const n = Number(raw.replaceAll('_', ''));
  return unit === 'millis' ? n < SMALL_MILLIS : n < SMALL_SECS;
}

function hasSmallBound(line) {
  return BOUNDS.some(re => [...line.matchAll(re)].some(m => small(m[1], m[2])));
}

/** The argument of the `.contains(` call `rest` starts inside. */
function containsArg(rest) {
  let depth = 1;
  for (let k = 0; k < rest.length; k++) {
    if (rest[k] === '(') depth++;
    else if (rest[k] === ')' && --depth === 0) return rest.slice(0, k);
  }
  return rest;
}

function count(text, ch) {
  return text.split(ch).length - 1;
}

/** Whether line `i` sits inside an `assert!(…)`: on it, or opened (and not
 *  yet closed) up to four lines above, with no statement ending between. */
function inAssert(lines, i) {
  if (ASSERT.test(lines[i])) return true;
  for (let j = i - 1; j >= Math.max(0, i - 4); j--) {
    if (ASSERT.test(lines[j])) {
      const span = lines.slice(j, i).join('\n');
      return count(span, '(') > count(span, ')');
    }
    if (/[;{}]\s*$/.test(lines[j].replace(/\/\/.*$/, ''))) return false;
  }
  return false;
}

/** The offenders in one file's text: `{ path, line (1-based), text (trimmed) }`. */
export function scanSource(path, text) {
  const lets = [...text.matchAll(ELAPSED_LET)];
  const names = [...new Set(lets.map(m => m[1]))];
  const alt = names.join('|');
  const named = names.length ? new RegExp(`\\b(?:${alt})\\b`) : null;
  const endsNamed = names.length ? new RegExp(`(?:^|[^\\w])(?:${alt})$`) : null;
  const operand = (line, prev) =>
    line.includes('elapsed()') ||
    Boolean(named?.test(line)) ||
    prev.endsWith('elapsed()') ||
    Boolean(endsNamed?.test(prev));

  // Bare-number bounds on those names: seconds when any `let` of the name
  // reads `.as_secs…()`, milliseconds otherwise (a `Duration` name can't be
  // compared with a bare number, so that only compiles via `.as_millis()`).
  const secsNames = [...new Set(lets.filter(m => /\.as_secs/.test(m[2])).map(m => m[1]))];
  const msNames = names.filter(n => !secsNames.includes(n));
  const word = list => (list.length ? new RegExp(`(?<![\\w.])(?:${list.join('|')})\\b`) : null);
  const msWord = word(msNames);
  const secsWord = word(secsNames);
  const nameBounds = [];
  if (msNames.length) {
    nameBounds.push([new RegExp(`(?<![\\w.])(?:${msNames.join('|')})\\s*<=?\\s*${NUM}${END}`, 'g'), 'millis']);
  }
  if (secsNames.length) {
    nameBounds.push([new RegExp(`(?<![\\w.])(?:${secsNames.join('|')})\\s*<=?\\s*${FLOAT}${END}`, 'g'), 'secs']);
  }
  // The unit of a range's operand, or null when it isn't an elapsed time.
  const unitOf = expr => {
    if (/\.as_secs/.test(expr) || Boolean(secsWord?.test(expr))) return 'secs';
    if (/\.ms\b|\w_ms\b|\.as_millis\(\)/.test(expr) || Boolean(msWord?.test(expr))) return 'millis';
    return null;
  };
  const ownBound = line =>
    [...OWN_BOUNDS, ...nameBounds].some(([re, unit]) => [...line.matchAll(re)].some(m => small(unit, m[1]))) ||
    [...line.matchAll(RANGE)].some(m => {
      const unit = unitOf(containsArg(line.slice(m.index + m[0].length)));
      return unit !== null && small(unit, m[2]);
    });

  const lines = text.split('\n');
  const out = [];
  lines.forEach((line, i) => {
    const trimmed = line.trim();
    if (trimmed.startsWith('//')) return;
    const prev = (lines[i - 1] ?? '').trim();
    if (!((hasSmallBound(line) && operand(line, prev)) || ownBound(line))) return;
    if (!inAssert(lines, i)) return;
    if (TIMING.test(line) || TIMING.test(prev)) return;
    out.push({ path, line: i + 1, text: trimmed });
  });
  return out;
}

/** `files` ({ path, text }) against `allowlist` ({ path, text }): the
 *  offenders no entry covers, and the entries no offender matches. An entry
 *  covers one line; the same line twice in a file needs two entries. */
export function check(files, allowlist) {
  const key = e => `${e.path}\n${e.text.trim()}`;
  const left = new Map();
  for (const e of allowlist) left.set(key(e), (left.get(key(e)) ?? 0) + 1);
  const newOffenders = [];
  for (const o of files.flatMap(f => scanSource(f.path, f.text))) {
    const n = left.get(key(o)) ?? 0;
    if (n > 0) left.set(key(o), n - 1);
    else newOffenders.push(o);
  }
  const stale = [];
  for (const e of allowlist) {
    const n = left.get(key(e)) ?? 0;
    if (n > 0) {
      stale.push({ path: e.path, text: e.text.trim() });
      left.set(key(e), n - 1);
    }
  }
  return { newOffenders, stale };
}

function rustFiles(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return rustFiles(path);
    return entry.name.endsWith('.rs') ? [path] : [];
  });
}

/** Every `.rs` under src-tauri/src as `{ path (repo-relative, /), text }`. */
export function repoFiles(repo = root) {
  return rustFiles(join(repo, SRC))
    .sort()
    .map(p => ({ path: relative(repo, p).split(sep).join('/'), text: readFileSync(p, 'utf8') }));
}

/** The allowlist's entries (`allowed` in the JSON). */
export function loadAllowlist(repo = root) {
  return JSON.parse(readFileSync(join(repo, ALLOWLIST), 'utf8')).allowed;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const allowlist = loadAllowlist();
  const { newOffenders, stale } = check(repoFiles(), allowlist);
  for (const o of newOffenders) console.error(`timing-asserts: ${o.path}:${o.line}: ${o.text}`);
  if (newOffenders.length) {
    console.error(
      'timing-asserts: a tight wall-clock upper bound flakes under load. Use the paused clock for a pure-timer test, ' +
        'or assert the cause and widen the bound (≥ 10× the expected time); a bound that must stay tight says why ' +
        'with `// timing: <reason>` on its line or the line above.',
    );
  }
  for (const e of stale) console.error(`timing-asserts: stale entry, remove it from ${ALLOWLIST}: ${e.path}: ${e.text}`);
  if (newOffenders.length || stale.length) process.exit(1);
  console.log(`timing-asserts: ok (${allowlist.length} allowlisted)`);
}
