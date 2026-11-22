#!/usr/bin/env node
// Every class a stylesheet in src/ styles is named somewhere the app can put
// it on an element (night ST3, 2026-10-14).
//
//   node scripts/check-orphan-css.mjs        (also run by npm run test:scripts)
//
// A class counts as used when its exact name appears as a token in a source
// file the app ships or serves: src/ (not tests), src-tauri/src, index.html,
// public/. Docs and specs don't count: a spec that only checks a class is
// absent doesn't keep its rules alive.
//
// A class is also used when the app builds it from a prefix: `kind-${x}`,
// `'diff-' + t`, or `l${level}` for a class like `l3` (digits only after a
// prefix with no dash). A prefix keeps every class
// that starts with it, so a builder is a blunt tool: one `is-${status}`
// keeps all `is-*` rules. Values of `id=`, `for=`, `aria-*=`, `data-*=`,
// `href=`, `key=` and `name=` aren't classes and don't count as builders, nor
// do mock data and tests. Rust's `format!("x-{}")` isn't read as a builder
// either: the backend writes no class names, and its format strings are file
// names and ids. Classes only a library adds go in LIBRARY_CLASSES.
//
// Not checked: CodeMirror's own `cm-*` classes in src/styles/codemirror.css
// (`.cm-scroller`, `.cm-tooltip`…). That sheet's other classes are checked,
// including the app's own editor decorations (`cm-md-*`, `cm-ai-*`,
// `cm-wikilink`, `cm-seg`, `cm-cwd`). src/canvas/kit/generated/ is a build
// output of kit.css.
//
// Removing the last reference to a class means removing its rules in the same
// change (or, for a class only a library adds, listing it in LIBRARY_CLASSES).
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

/** Build output, not a source stylesheet. */
export const SKIP_CSS = ['src/canvas/kit/generated/'];

/** Classes a library's own DOM carries, by the sheet that styles them. */
export const LIBRARY_SHEETS = { 'src/styles/codemirror.css': /^cm-(?!md-|ai-|wikilink|seg|cwd)/ };

/** Classes a library puts on elements we style from our own sheets. */
export const LIBRARY_CLASSES = new Set([]);

const SOURCE_EXT = /\.(tsx?|jsx?|mjs|html|rs|svg)$/;
/** Prompt files the backend compiles in (`include_str!`). */
const BACKEND_PROMPT = /src-tauri\/src\/.*\.md$/;

function walk(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name === 'node_modules' || e.name === 'target' || e.name.startsWith('.')) continue;
    const p = join(dir, e.name);
    if (e.isDirectory()) walk(p, out);
    else out.push(p);
  }
  return out;
}

/** Class names in a stylesheet's selectors (not in declarations, comments or strings). */
export function selectorClasses(css) {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '').replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""');
  const out = new Set();
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === '{') {
      const prelude = text.slice(start, i).trim();
      if (!prelude.startsWith('@')) {
        // Attribute selectors and :is()/:not() arguments may hold quoted text or classes;
        // drop [..] bodies, keep the rest.
        const sel = prelude.replace(/\[[^\]]*\]/g, '');
        for (const m of sel.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)) out.add(m[1]);
      }
      start = i + 1;
    } else if (ch === '}' || ch === ';') start = i + 1;
  }
  return out;
}

/** Every identifier-like token in source text, for exact class lookups. */
export function sourceTokens(text, into = new Set()) {
  for (const m of text.matchAll(/[A-Za-z_][\w-]*/g)) into.add(m[0]);
  return into;
}

/** Attribute values that hold an id, a link or a key, never a class. */
const NOT_A_CLASS = /\b(?:id|for|href|key|name|aria-[\w-]+|data-[\w-]+)\s*=\s*\{?\s*[`'"]$/;

/** Prefixes the code builds class names from: `kind-${`, `'diff-' +`, `l${`. */
export function dynamicPrefixes(text, into = new Set()) {
  for (const m of text.matchAll(/(?<![\w-])([A-Za-z_][\w-]*)(?:\$\{|['"`]\s*\+)/g)) {
    if (!NOT_A_CLASS.test(text.slice(Math.max(0, m.index - 40), m.index))) into.add(m[1]);
  }
  return into;
}

/** Whether `cls` could come from one of the built prefixes. */
export function isBuilt(cls, prefixes) {
  for (const p of prefixes) {
    if (!cls.startsWith(p) || cls === p) continue;
    const rest = cls.slice(p.length);
    if (p.endsWith('-')) return true;
    if (/^\d+$/.test(rest)) return true;
  }
  return false;
}

/** Orphans: [{ cls, files }] for classes no shipped source names or builds. */
export function findOrphans({ css, tokens, prefixes, library = LIBRARY_CLASSES, sheets = LIBRARY_SHEETS }) {
  const where = new Map();
  for (const [file, text] of css) {
    for (const cls of selectorClasses(text)) {
      if (sheets[file]?.test(cls)) continue;
      if (!where.has(cls)) where.set(cls, []);
      where.get(cls).push(file);
    }
  }
  const out = [];
  for (const [cls, files] of where) {
    if (tokens.has(cls) || library.has(cls) || isBuilt(cls, prefixes)) continue;
    out.push({ cls, files });
  }
  return out.sort((a, b) => a.cls.localeCompare(b.cls));
}

export function checkRepo(dir = root) {
  const rel = f => relative(dir, f).split('\\').join('/');
  const css = new Map();
  for (const f of walk(join(dir, 'src'))) {
    const r = rel(f);
    if (!r.endsWith('.css') || SKIP_CSS.some(s => r === s || r.startsWith(s))) continue;
    css.set(r, readFileSync(f, 'utf8'));
  }
  const sources = [...walk(join(dir, 'src')), ...walk(join(dir, 'src-tauri/src')), ...walk(join(dir, 'public'))]
    .filter(f => (SOURCE_EXT.test(f) || BACKEND_PROMPT.test(rel(f))) && !/\.(test|spec)\.[tj]sx?$/.test(f));
  sources.push(join(dir, 'index.html'));
  const tokens = new Set();
  const prefixes = new Set();
  for (const f of sources) {
    if (!existsSync(f)) continue;
    const text = readFileSync(f, 'utf8');
    sourceTokens(text, tokens);
    if (!/^src\/mock/.test(rel(f))) dynamicPrefixes(text, prefixes);
  }
  return findOrphans({ css, tokens, prefixes });
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const orphans = checkRepo();
  if (orphans.length) {
    console.error(`${orphans.length} CSS class(es) nothing in the app uses:`);
    for (const o of orphans) console.error(`  .${o.cls}  (${o.files.join(', ')})`);
    console.error('Remove their rules, or name the class where the app sets it.');
    process.exit(1);
  }
  console.log('check-orphan-css: every styled class is used');
}
