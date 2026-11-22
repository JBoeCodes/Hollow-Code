#!/usr/bin/env node
// Review F07 (2026-10-13): the required specs (the supported-minimum window
// cases, *.minimum.spec.ts, and the click-only journeys, *.journey.spec.ts)
// must execute, never skip. They run only in the projects that match them
// (playwright.config.ts), so they need no project gating, and a skip, fixme
// or only in one is refused here.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
export const REQUIRED = /\.(minimum|journey)\.spec\.ts$/;
const FORBIDDEN = /\btest(\.describe)?\.(skip|fixme|only)\s*\(/;

export function requiredSpecProblems(dir = join(ROOT, 'tests', 'smoke'), read = f => readFileSync(f, 'utf8'), list = d => readdirSync(d)) {
  const problems = [];
  const files = list(dir).filter(n => REQUIRED.test(n));
  if (!files.length) problems.push('no required specs found');
  for (const name of files) {
    const text = read(join(dir, name));
    const hit = FORBIDDEN.exec(text);
    if (hit) problems.push(`${name} uses ${hit[0].replace(/\s*\($/, '')}: a required spec may not skip`);
  }
  return problems;
}

export function configProblems(text = readFileSync(join(ROOT, 'playwright.config.ts'), 'utf8')) {
  const problems = [];
  for (const project of ['chromium-minimum', 'chromium-minimum-large-text']) {
    const at = text.indexOf(`name: '${project}'`);
    if (at < 0) { problems.push(`playwright.config.ts has no ${project} project`); continue; }
    const next = text.indexOf("name: '", at + 7);
    const block = text.slice(at, next < 0 ? undefined : next);
    if (!/testMatch: \[MINIMUM_ONLY, JOURNEYS\]/.test(block)) problems.push(`${project} must match the required specs`);
    if (!/viewport:\s*\{\s*width:\s*720,\s*height:\s*480\s*\}/.test(block)) problems.push(`${project} must use the native minimum window (720 × 480)`);
  }
  return problems;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const problems = [...requiredSpecProblems(), ...configProblems()];
  if (problems.length) { console.error(problems.join('\n')); process.exit(1); }
  console.log('required specs: present, never skipped, matched by the minimum projects');
}
