#!/usr/bin/env node
// Section-extracted handoff (LF4, 2026-11-17): build a lane brief from named
// sections of checked upstream artifacts instead of pasting whole files.
//
//   node scripts/lane-extract.mjs --from <path::Section A|Section B>... [--out brief.md] [--stats]
//
// Each `--from` names a file and the heading sections to pull (a heading whose
// text equals the name, case-insensitive; the section runs to the next heading of the same or
// a higher level). It REFUSES (exit 1, nothing written) when:
//   - the file is missing or empty,
//   - a named section is missing or has an empty body,
//   - the file contradicts itself on a verdict key: lines `Verdict:`, `Result:`,
//     `Status:`, `Merge sha:` or `Head sha:` (at line start, after any bullet)
//     with two different values anywhere in the file, so a stale verdict higher
//     up cannot hide behind a fresh extract.
// The brief tells the reader to open the full file only if an extract is missing
// or contradicts. `--stats` prints source bytes, brief bytes and the ratio.
// No model, no network. Method only (LoopForge's prompt builder); code is ours.
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const KEYS = ['verdict', 'result', 'status', 'merge sha', 'head sha'];

/** Sections by heading with the level, so a section can end at a sibling or parent. */
export function sections(text) {
  const lines = text.split('\n');
  const heads = [];
  let fence = false;
  lines.forEach((line, i) => {
    if (/^(```|~~~)/.test(line)) fence = !fence;
    const h = !fence && /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (h) heads.push({ level: h[1].length, name: h[2], at: i });
  });
  return heads.map((h, n) => {
    const end = heads.slice(n + 1).find(x => x.level <= h.level)?.at ?? lines.length;
    return { name: h.name, body: lines.slice(h.at + 1, end).join('\n').trim() };
  });
}

export function contradictions(text) {
  const seen = new Map();
  for (const line of text.split('\n')) {
    const m = /^\s*(?:[-*]\s+)?\**([A-Za-z ]+?)\**:\s*\**\s*(\S.*?)\s*$/.exec(line);
    if (!m || !KEYS.includes(m[1].toLowerCase())) continue;
    const key = m[1].toLowerCase();
    const val = m[2].toLowerCase().replace(/[*.`]/g, '');
    if (!seen.has(key)) seen.set(key, new Set());
    seen.get(key).add(val);
  }
  return [...seen].filter(([, v]) => v.size > 1).map(([k, v]) => `${k} has ${v.size} different values (${[...v].join(' vs ')})`);
}

/** The section named `name`: exact heading text (case-insensitive); of several with the same
 * name the first with a non-empty body wins, so an empty template heading cannot shadow a filled one. */
export function findSection(have, name) {
  const same = have.filter(s => s.name.toLowerCase() === name.toLowerCase());
  return same.find(s => s.body) ?? same[0];
}

/** `{ brief, sourceBytes, briefBytes, problems }`; `brief` is '' when refused. */
export function extract(specs) {
  const problems = [];
  const parts = [];
  let sourceBytes = 0;
  for (const spec of specs) {
    const at = spec.indexOf('::');
    const path = at < 0 ? spec : spec.slice(0, at);
    const names = at < 0 ? [] : spec.slice(at + 2).split('|').map(s => s.trim()).filter(Boolean);
    if (!names.length) {
      problems.push(`${path}: no sections named`);
      continue;
    }
    if (!existsSync(path)) {
      problems.push(`${path}: extract source is missing`);
      continue;
    }
    const text = readFileSync(path, 'utf8');
    sourceBytes += Buffer.byteLength(text);
    if (!text.trim()) {
      problems.push(`${path}: extract source is empty`);
      continue;
    }
    for (const c of contradictions(text)) problems.push(`${path}: contradicts itself: ${c}`);
    const have = sections(text);
    for (const name of names) {
      const hit = findSection(have, name);
      if (!hit) problems.push(`${path}: section "${name}" is missing`);
      else if (!hit.body) problems.push(`${path}: section "${name}" is empty`);
      else parts.push(`### ${hit.name} (from ${path})\n\n${hit.body}`);
    }
  }
  if (problems.length) return { brief: '', sourceBytes, briefBytes: 0, problems };
  const brief = `## Upstream extract\n\nChecked sections only. Open the full file only if an extract is missing or contradicts.\n\n${parts.join('\n\n')}\n`;
  return { brief, sourceBytes, briefBytes: Buffer.byteLength(brief), problems };
}

function main(argv) {
  const from = [];
  let out = '';
  let stats = false;
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--from') from.push(argv[++i]);
    else if (argv[i] === '--out') out = argv[++i];
    else if (argv[i] === '--stats') stats = true;
    else {
      console.error(`unknown argument ${argv[i]}`);
      return 2;
    }
  }
  if (!from.length || from.some(f => !f) || (argv.includes('--out') && !out)) {
    console.error('usage: lane-extract.mjs --from <path::Section|Section>... [--out brief.md] [--stats]');
    return 2;
  }
  const sources = from.map(f => resolve(f.split('::')[0]));
  if (out && sources.includes(resolve(out))) {
    console.error(`refusing --out ${out}: it is one of the source files`);
    return 2;
  }
  const r = extract(from);
  if (r.problems.length) {
    for (const p of r.problems) console.log(`REFUSE ${p}`);
    return 1;
  }
  if (out) writeFileSync(out, r.brief);
  else process.stdout.write(r.brief);
  if (stats) console.error(`source ${r.sourceBytes} B, brief ${r.briefBytes} B, ratio ${(r.briefBytes / r.sourceBytes).toFixed(2)}`);
  return 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));
