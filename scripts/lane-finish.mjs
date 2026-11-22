#!/usr/bin/env node
// Fresh-artifact finish rule (LF2, 2026-11-17): a lane may finish only with
// artifacts that are new or changed since the stage began.
//
//   node scripts/lane-finish.mjs --baseline <baseline.json> --require <spec>... [--now <iso>]
//   node scripts/lane-finish.mjs --check    <baseline.json> --require <spec>...
//
// `<spec>` is `path` or `path::Section A|Section B`. `--baseline` records each
// required file's sha-256, size and mtime at stage start (a missing file is
// recorded as absent). `--check` exits 1, naming the file and the reason, when a
// required artifact is:
//   - unchanged since the baseline (same content hash; touching it does not
//     count, and neither does an mtime change with the same bytes),
//   - missing or empty,
//   - lacking a named section (a heading whose text equals the name) or
//     holding one with an empty body,
//   - carrying an unfilled `<...>` placeholder or a TODO, TBD or FIXME marker
//     (fenced code and inline code are ignored).
// Freshness is a content hash, not a clock: a file rewritten with the same bytes
// is unchanged. A required path that is not in the baseline fails the check
// (paths are compared resolved, so relative and absolute spellings agree).
// Sections come from lane-extract's parser: exact heading text, a heading holding
// only subsections is not empty, an empty heading cannot shadow a filled twin.
// No model, no network; time is only the `--now` label stored in the baseline.
// Method only (LoopForge's stage baseline, stitch-skills' baton); code is ours.
import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { findSection, sections as sectionsOf } from './lane-extract.mjs';

export function parseSpec(spec) {
  const at = spec.indexOf('::');
  if (at < 0) return { path: spec, sections: [] };
  return { path: spec.slice(0, at), sections: spec.slice(at + 2).split('|').map(s => s.trim()).filter(Boolean) };
}

export function snapshot(path) {
  if (!existsSync(path)) return null;
  const bytes = readFileSync(path);
  return { sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length, mtimeMs: Math.round(statSync(path).mtimeMs) };
}

export function makeBaseline(paths, now = '') {
  return { at: now, files: Object.fromEntries(paths.map(p => [resolve(p), snapshot(p)])) };
}

const HTML = /^<\/?(a|br|b|i|u|em|strong|code|pre|p|div|span|ul|ol|li|details|summary|img|table|thead|tbody|tr|td|th|hr|h[1-6]|kbd|sub|sup)\b/i;

/** Placeholder markers in text, ignoring fenced and inline code. */
export function placeholders(text) {
  const bare = text.replace(/^(```|~~~)[\s\S]*?^\1.*$/gm, '').replace(/`[^`\n]*`/g, '');
  const found = [];
  for (const m of bare.matchAll(/<[^<>\n]{2,}>/g)) {
    if (HTML.test(m[0]) || /^<(https?|mailto|file):/i.test(m[0]) || /^<!--/.test(m[0])) continue;
    found.push(m[0]);
  }
  for (const m of bare.matchAll(/\b(TODO|TBD|FIXME)\b/g)) found.push(m[0]);
  return found;
}

export function checkFinish(baseline, specs) {
  const problems = [];
  for (const spec of specs) {
    const { path, sections } = parseSpec(spec);
    const now = snapshot(path);
    if (!now) {
      problems.push(`${path}: missing`);
      continue;
    }
    const text = readFileSync(path, 'utf8');
    if (!text.trim()) {
      problems.push(`${path}: empty`);
      continue;
    }
    const key = resolve(path);
    if (!baseline.files || !(key in baseline.files)) {
      problems.push(`${path}: not in the baseline (record it with --baseline before the stage starts)`);
      continue;
    }
    const before = baseline.files[key];
    if (before && before.sha256 === now.sha256) problems.push(`${path}: unchanged since the baseline${baseline.at ? ` (${baseline.at})` : ''}`);
    const have = sectionsOf(text);
    for (const name of sections) {
      const hit = findSection(have, name);
      if (!hit) problems.push(`${path}: missing section "${name}"`);
      else if (!hit.body.trim()) problems.push(`${path}: section "${name}" is empty`);
    }
    const open = placeholders(text);
    if (open.length) problems.push(`${path}: unfilled placeholder ${[...new Set(open)].slice(0, 3).join(', ')}`);
  }
  return problems;
}

function main(argv) {
  const o = { require: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--require') o.require.push(argv[++i]);
    else if (a === '--baseline' || a === '--check' || a === '--now') o[a.slice(2)] = argv[++i];
    else {
      console.error(`unknown argument ${a}`);
      return 2;
    }
  }
  if (!o.require.length || !!o.baseline === !!o.check || o.require.some(r => !r)) {
    console.error('usage: lane-finish.mjs (--baseline f | --check f) --require <path[::Section|Section]>... [--now iso]');
    return 2;
  }
  const specs = o.require;
  if (o.baseline) {
    writeFileSync(o.baseline, `${JSON.stringify(makeBaseline(specs.map(s => parseSpec(s).path), o.now), null, 2)}\n`);
    console.log(`baseline of ${specs.length} artifact(s) written to ${o.baseline}`);
    return 0;
  }
  const problems = checkFinish(JSON.parse(readFileSync(o.check, 'utf8')), specs);
  for (const p of problems) console.log(`BLOCK ${p}`);
  if (!problems.length) console.log(`ok: ${specs.length} artifact(s) fresh and complete`);
  return problems.length ? 1 : 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));
