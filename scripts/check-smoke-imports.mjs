#!/usr/bin/env node
// Review F05/F07 (2026-10-13): every smoke spec takes `test` from
// tests/smoke/fixtures/test.ts, whose automatic pageErrors fixture fails a test
// on an unexpected uncaught exception or unhandled rejection. A spec importing
// '@playwright/test' directly would silently opt out of that.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SMOKE = join(fileURLToPath(new URL('.', import.meta.url)), '..', 'tests', 'smoke');

export function specsBypassingFixture(dir = SMOKE, read = f => readFileSync(f, 'utf8'), list = d => readdirSync(d)) {
  return list(dir).filter(n => n.endsWith('.spec.ts')).filter(n => /from\s+['"]@playwright\/test['"]/.test(read(join(dir, n))));
}

// Overnight 2026-10-15 H1: a spec that saves a screenshot to a fixed path in
// the checkout (`.context/x.png` was tracked) dirties the tree, and a dirty
// tree makes gate-record skip its record. Screenshots go to
// `testInfo.outputPath(...)` (under test-results/, ignored) or `attach`; a
// path built from an environment folder (`${process.env.X}/…`) is the
// caller's choice and allowed.
// A heuristic over the source text: a string literal, a template not starting
// with `${process.env.`, or join/resolve of a literal, as a screenshot's path.
const FIXED_SCREENSHOT_PATH = /screenshot\(\s*\{[^;]*?\bpath:\s*(['"]|`(?!\$\{process\.env\.)|(?:join|resolve)\(\s*['"])/;

export function specsWritingCheckoutFiles(dir = SMOKE, read = f => readFileSync(f, 'utf8'), list = d => readdirSync(d)) {
  return list(dir).filter(n => n.endsWith('.spec.ts')).filter(n => FIXED_SCREENSHOT_PATH.test(read(join(dir, n))));
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const bad = specsBypassingFixture();
  if (bad.length) { console.error(`Import from './fixtures/test', not '@playwright/test':\n  ${bad.join('\n  ')}`); process.exit(1); }
  const writers = specsWritingCheckoutFiles();
  if (writers.length) { console.error(`Save screenshots with testInfo.outputPath(...), not a fixed path:\n  ${writers.join('\n  ')}`); process.exit(1); }
  console.log('smoke imports: every spec uses the page-error fixture and saves no screenshot into the checkout');
}
