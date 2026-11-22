#!/usr/bin/env node
// No new wall-clock sleep or clock read in a test (TS1, 2026-11-16).
//
//   node scripts/check-test-sleeps.mjs             (also run by npm run test:scripts)
//   node scripts/check-test-sleeps.mjs --update    lower scripts/test-sleeps-baseline.json to today's counts
//
// A test that sleeps a fixed time passes on an idle Mac and fails when two
// lanes gate at once, and one that reads the clock passes today and fails in
// the DST hour or at midnight. The smoke notes list such flakes by name. A
// test should wait for the state it needs (a drained queue, an emitted event,
// a gate only the test opens, `expect.poll`) and take its time from a fixed
// clock (`vi.useFakeTimers`, `vi.setSystemTime`, a passed-in `now`).
//
// A text check over the tests, counted per file and kind:
// - `sleep`: Playwright `waitForTimeout(`, the `new Promise(r => setTimeout(r, …))`
//   idiom, `await delay(…)`, and in Rust tests `thread::sleep(` and
//   `tokio::time::sleep(`. Frontend and script tests only (`*.test.ts`,
//   `*.test.tsx`, `*.test.mjs`, everything under `tests/`); Rust tests are
//   `*_tests.rs` files, files under `src-tauri/tests/`, and the lines inside
//   a `#[cfg(test)] mod … { … }` block of any other file (production code may
//   sleep).
// - `clock`: `Date.now()` and `new Date()` with no argument, in the same
//   frontend and script tests. (Rust clock reads are everywhere in fixtures;
//   `check-timing-asserts.mjs` already covers the Rust assertions that depend
//   on them.)
//
// A line that has to sleep or read the clock says why: `// sleep-ok: <reason>`
// on the line or the line directly above accepts it (never counted). The
// counts left when the check landed are in scripts/test-sleeps-baseline.json
// (path, kind, count): a count may only go down. A new file or kind with any
// offender fails, a count above its baseline fails, and so does a baseline
// above the count (run `--update` and commit the lower number), so the list
// ratchets toward zero and never grows.
//
// The idea is t3code's (ban sleeps and the wall clock in tests by lint, wait
// for a drained queue instead); ideas only, no code taken.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export const BASELINE = 'scripts/test-sleeps-baseline.json';
/** Its own test spells out every pattern it checks, as text. */
const SELF = ['scripts/check-test-sleeps.test.mjs'];

const TS_SLEEP = [
  /\bwaitForTimeout\s*\(/,
  /new\s+Promise\s*(?:<[^>]*>)?\s*\(\s*\(?\s*\w+\s*\)?\s*=>\s*setTimeout\s*\(/,
  /\bawait\s+delay\s*\(/,
];
const TS_CLOCK = [/\bDate\.now\s*\(\s*\)/, /\bnew\s+Date\s*\(\s*\)/];
const RUST_SLEEP = [/\b(?:std::)?thread::sleep\s*\(/, /\b(?:tokio::)?time::sleep\s*\(/];

const OK = /\/\/\s*sleep-ok:\s*\S/;

/** Whether the file is a frontend or script test (everything under `tests/` counts). */
export function isTsTest(path) {
  if (!/\.(?:[cm]?[jt]sx?)$/.test(path)) return false;
  if (path.startsWith('tests/')) return true;
  return /\.test\.[cm]?[jt]sx?$/.test(path);
}

/** Whether every line of the Rust file is test code, not only a `cfg(test)` block. */
export function isRustTestFile(path) {
  return path.endsWith('.rs') && (/_tests\.rs$/.test(path) || path.startsWith('src-tauri/tests/'));
}

/** The 0-based line numbers inside `#[cfg(test)] mod … { … }` blocks of a Rust file (the block ends at the `}` with the `mod` line's indent). */
export function rustTestLines(text) {
  const lines = text.split('\n');
  const inside = new Set();
  for (let i = 0; i < lines.length; i++) {
    if (!/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) continue;
    const head = lines[i + 1] ?? '';
    if (!/^\s*(pub(\([^)]*\))?\s+)?mod\s+\w+\s*\{\s*$/.test(head)) continue;
    const indent = head.match(/^\s*/)[0];
    let j = i + 2;
    while (j < lines.length && lines[j] !== `${indent}}`) j++;
    for (let k = i + 2; k < Math.min(j, lines.length); k++) inside.add(k);
    i = j;
  }
  return inside;
}

const isComment = line => /^\s*(?:\/\/|\*|\/\*)/.test(line);

/** `{ sleep, clock }` offender counts for one file's text. */
export function countOffenders(path, text) {
  const out = { sleep: 0, clock: 0 };
  const lines = text.split('\n');
  const accepted = i => OK.test(lines[i]) || (i > 0 && OK.test(lines[i - 1]));
  if (isTsTest(path)) {
    lines.forEach((line, i) => {
      if (isComment(line) || accepted(i)) return;
      if (TS_SLEEP.some(re => re.test(line))) out.sleep += 1;
      if (TS_CLOCK.some(re => re.test(line))) out.clock += 1;
    });
  } else if (path.endsWith('.rs')) {
    const inside = isRustTestFile(path) ? null : rustTestLines(text);
    if (inside && inside.size === 0) return out;
    lines.forEach((line, i) => {
      if (inside && !inside.has(i)) return;
      if (isComment(line) || accepted(i)) return;
      if (RUST_SLEEP.some(re => re.test(line))) out.sleep += 1;
    });
  }
  return out;
}

/** The files the check reads: the repo's tracked files that can hold a test. */
export function candidateFiles(repo = root) {
  return execFileSync('git', ['-C', repo, 'ls-files', '-z', '--', 'src', 'tests', 'scripts', 'src-tauri/src', 'src-tauri/tests'], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\0')
    .filter(f => f && !SELF.includes(f) && (isTsTest(f) || f.endsWith('.rs')));
}

/** Counts per file and kind, only where something was found. */
export function countAll(files, read) {
  const counts = {};
  for (const file of files) {
    const text = read(file);
    if (text == null) continue;
    const c = countOffenders(file, text);
    for (const kind of ['sleep', 'clock']) {
      if (c[kind] > 0) (counts[file] ??= {})[kind] = c[kind];
    }
  }
  return counts;
}

/**
 * What differs between `counts` and the `baseline`: `grew` (a count above its
 * baseline, or any count where none is allowed) and `shrank` (a baseline above
 * the count, to lower), each as `path kind: baseline → count`.
 */
export function compare(counts, baseline) {
  const grew = [];
  const shrank = [];
  const paths = new Set([...Object.keys(counts), ...Object.keys(baseline)]);
  for (const path of [...paths].sort()) {
    for (const kind of ['sleep', 'clock']) {
      const have = counts[path]?.[kind] ?? 0;
      const allowed = baseline[path]?.[kind] ?? 0;
      if (have > allowed) grew.push(`${path} ${kind}: ${allowed} → ${have}`);
      else if (have < allowed) shrank.push(`${path} ${kind}: ${allowed} → ${have}`);
    }
  }
  return { grew, shrank };
}

function main(argv) {
  const files = candidateFiles();
  const read = f => {
    try {
      return readFileSync(join(root, f), 'utf8');
    } catch {
      return null;
    }
  };
  const counts = countAll(files, read);
  const basePath = join(root, BASELINE);
  let baseline = {};
  try {
    baseline = JSON.parse(readFileSync(basePath, 'utf8')).files ?? {};
  } catch {
    // no baseline yet: every offender is new
  }
  const { grew, shrank } = compare(counts, baseline);
  if (argv.includes('--update')) {
    // The first run writes the baseline; after that it may only go down.
    if (grew.length && existsSync(basePath)) {
      console.error(`test-sleeps: --update only lowers the baseline; these grew:\n  ${grew.join('\n  ')}`);
      return 1;
    }
    writeFileSync(basePath, `${JSON.stringify({ version: 1, files: counts }, null, 1)}\n`);
    console.log(`test-sleeps: baseline written (${Object.keys(counts).length} files)`);
    return 0;
  }
  if (grew.length) {
    console.error(
      `test-sleeps: a test sleeps or reads the clock where it did not before. Wait for the state the test needs (a drained queue, an emitted event, expect.poll) and take time from a fixed clock; or, if it must, add \`// sleep-ok: <reason>\` on the line:\n  ${grew.join('\n  ')}`,
    );
  }
  if (shrank.length) {
    console.error(`test-sleeps: fewer than the baseline now; run \`node scripts/check-test-sleeps.mjs --update\` and commit the lower numbers:\n  ${shrank.join('\n  ')}`);
  }
  if (grew.length || shrank.length) return 1;
  const total = Object.values(counts).reduce((n, c) => n + (c.sleep ?? 0) + (c.clock ?? 0), 0);
  console.log(`test-sleeps: ok (${total} in the baseline, in ${Object.keys(counts).length} files)`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) process.exit(main(process.argv.slice(2)));
