#!/usr/bin/env node
// Every module has a line in a module map, and every line names a module
// that exists (overnight E5).
//
//   node scripts/check-module-map.mjs        (also run by npm run test:scripts)
//
// - Backend: each top-level `src-tauri/src/*.rs` must be named in the
//   "Backend module map" section of docs/reference/backend-module-map.md
//   (moved there from CLAUDE.md on 2026-10-13): a `name.rs` token; `*` and
//   `{a,b}` in a token match several, as in `self_install_*_tests.rs`. Every
//   name a token stands for must exist under src-tauri/src: a plain name
//   exactly, each name of a `{a,b}` list, and a `*` token must match at least
//   one file. (Prose paths starting `src-tauri/` aren't map entries.)
// - Frontend: each top-level `src/*.ts(x)` (not tests or `.d.ts`) must have a
//   row in docs/FRONTEND_MODULES.md, and each row must name a file that exists.
// - AGENTS.md (the manual entry point; CLAUDE.md only includes it) links both maps.
//
// Adding a module means adding its line in the same change; removing one
// means removing its line.
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export const BACKEND_MAP = 'docs/reference/backend-module-map.md';

/** The text of a markdown section: from `heading` to the next heading of the same or higher level. */
export function section(text, heading) {
  const start = text.indexOf(heading);
  if (start < 0) return '';
  const level = heading.match(/^#+/)[0].length;
  const rest = text.slice(start + heading.length);
  const next = rest.search(new RegExp(`\\n#{1,${level}} `));
  return next < 0 ? rest : rest.slice(0, next);
}

/** Backticked tokens ending in `ext` (`.rs`, `.ts`…). */
export function tokens(text, ext) {
  const out = [];
  for (const m of text.matchAll(/`([^`\s]+)`/g)) {
    if (m[1].endsWith(ext) || m[1].endsWith(`${ext})`)) out.push(m[1].replace(/\)$/, ''));
  }
  return out;
}

/** A token as a regex over file names: `*` any run, `{a,b}` either. */
export function tokenRegex(token) {
  const esc = s => s.replace(/[.+?^$()|[\]\\]/g, '\\$&');
  let src = '';
  for (let i = 0; i < token.length; i++) {
    const c = token[i];
    if (c === '*') src += '[^/]*';
    else if (c === '{') {
      const end = token.indexOf('}', i);
      src += `(?:${token.slice(i + 1, end).split(',').map(esc).join('|')})`;
      i = end;
    } else src += esc(c);
  }
  return new RegExp(`^${src}$`);
}

/** A token's `{a,b}` lists expanded into the names it stands for. */
export function expand(token) {
  const m = token.match(/\{([^}]*)\}/);
  if (!m) return [token];
  return m[1].split(',').flatMap(alt => expand(token.replace(m[0], alt)));
}

/** Problems for one map: `files` must each match a token, and every token
 *  must stand for files that exist in `all` (each brace name exactly; a `*`
 *  name matching at least one). */
export function checkMap({ name, files, mapTokens, all }) {
  const problems = [];
  const entries = mapTokens.filter(t => !t.startsWith('src-tauri/'));
  const regexes = entries.map(t => ({ t, re: tokenRegex(t) }));
  for (const f of files) {
    if (!regexes.some(({ re }) => re.test(f))) problems.push(`${f} has no line in ${name}`);
  }
  for (const t of entries) {
    for (const n of expand(t)) {
      const ok = n.includes('*') ? [...all].some(f => tokenRegex(n).test(f)) : all.has(n);
      if (!ok) problems.push(`${name} names ${n}, which doesn't exist`);
    }
  }
  return problems;
}

function walk(dir, base = dir, out = new Set()) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, base, out);
    else out.add(p.slice(base.length + 1));
  }
  return out;
}

export function checkModuleMaps(repo = root) {
  const entry = readFileSync(join(repo, 'AGENTS.md'), 'utf8');
  const backendDoc = readFileSync(join(repo, BACKEND_MAP), 'utf8');
  const rsDir = join(repo, 'src-tauri/src');
  const rsAll = walk(rsDir);
  const rsTop = [...rsAll].filter(f => !f.includes('/') && f.endsWith('.rs'));
  const backend = checkMap({
    name: `${BACKEND_MAP} "Backend module map"`,
    files: rsTop,
    mapTokens: tokens(section(backendDoc, '### Backend module map'), '.rs'),
    all: rsAll,
  });
  const frontendDoc = readFileSync(join(repo, 'docs/FRONTEND_MODULES.md'), 'utf8');
  const srcTop = readdirSync(join(repo, 'src'), { withFileTypes: true })
    .filter(e => e.isFile() && /\.(ts|tsx)$/.test(e.name) && !/\.test\.tsx?$/.test(e.name) && !e.name.endsWith('.d.ts'))
    .map(e => e.name);
  const rowTokens = [...frontendDoc.matchAll(/^\| `([^`]+\.tsx?)` \|/gm)].map(m => m[1]);
  const frontend = checkMap({
    name: 'docs/FRONTEND_MODULES.md',
    files: srcTop,
    mapTokens: rowTokens,
    all: new Set(srcTop),
  });
  const linked = [BACKEND_MAP, 'docs/FRONTEND_MODULES.md']
    .filter(doc => !entry.includes(doc))
    .map(doc => `AGENTS.md doesn't link ${doc}`);
  if (!section(backendDoc, '### Backend module map')) linked.push(`${BACKEND_MAP} has no "### Backend module map" section`);
  return [...backend, ...frontend, ...linked];
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const problems = checkModuleMaps();
  for (const p of problems) console.error(`module-map: ${p}`);
  if (problems.length) process.exit(1);
  console.log('module-map: ok');
}
