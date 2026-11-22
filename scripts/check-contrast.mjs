#!/usr/bin/env node
// Overnight F5: WCAG AA contrast for the design tokens, in both themes.
//
// Reads the light `:root` block and the `:root[data-theme="dark"]` block of
// src/styles/tokens.css, resolves `var(--x)` aliases (a dark block inherits
// what it doesn't set), and checks every text token the app paints as
// `color:` against every surface it sits on: 4.5:1 for text (AA, normal
// size). Fails with each pair under its target. Custom (kitty) palettes and
// Liquid Glass are derived at runtime and are not checked here.
//
//   node scripts/check-contrast.mjs            # check, exit 1 on a failure
//   node scripts/check-contrast.mjs --report   # every pair and its ratio
import { readFileSync, realpathSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
export const TOKENS = join(ROOT, 'src/styles/tokens.css');

/** WCAG AA for normal-size text. */
export const TEXT_AA = 4.5;

/** The surfaces text sits on. */
export const SURFACES = ['--canvas', '--surface-soft', '--surface-card', '--surface-raised'];

/** Tokens painted as text (`color: var(--x)` across src/styles), each with
 *  the surfaces it must read on. */
export const TEXT = [
  '--ink', '--body-strong', '--body', '--muted', '--muted-soft',
  '--primary', '--accent-teal',
  '--success', '--warning', '--error', '--risk',
].map(token => ({ token, on: SURFACES, min: TEXT_AA }));

/** Pairs that aren't text-on-surface: the label on a filled primary or
 *  danger button, and code (syntax colours on the code block's surface). */
export const EXTRA = [
  { token: '--on-primary', on: ['--primary', '--primary-active'], min: TEXT_AA },
  { token: '--on-error', on: ['--error'], min: TEXT_AA },
  ...['--syn-keyword', '--syn-string', '--syn-comment', '--syn-number', '--syn-function', '--syn-type',
    '--syn-builtin', '--syn-operator', '--syn-punct', '--syn-property', '--syn-tag', '--syn-attr', '--syn-meta',
  ].map(token => ({ token, on: ['--surface-soft', '--surface-card'], min: TEXT_AA })),
];

/** The declarations of the first block whose selector is exactly `selector`
 *  (comments stripped), as a name → raw value map. */
export function block(css, selector) {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const at = text.indexOf(`${selector} {`);
  if (at < 0) throw new Error(`no ${selector} block`);
  let depth = 0;
  let end = at;
  for (let i = text.indexOf('{', at); i < text.length; i++) {
    if (text[i] === '{') depth++;
    else if (text[i] === '}' && --depth === 0) { end = i; break; }
  }
  const body = text.slice(text.indexOf('{', at) + 1, end);
  const vars = new Map();
  for (const m of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) vars.set(m[1], m[2].trim());
  return vars;
}

/** `#rgb`/`#rrggbb`/`rgb(a)()` as [r, g, b] (0–255); null for anything else. */
export function parseColor(value) {
  const v = value.trim().toLowerCase();
  let m = /^#([0-9a-f]{3})$/.exec(v);
  if (m) return [...m[1]].map(c => parseInt(c + c, 16));
  m = /^#([0-9a-f]{6})$/.exec(v);
  if (m) return [0, 2, 4].map(i => parseInt(m[1].slice(i, i + 2), 16));
  m = /^rgba?\(\s*(\d+)\s*,\s*(\d+)\s*,\s*(\d+)\s*(?:,\s*([\d.]+)\s*)?\)$/.exec(v);
  if (m && (m[4] === undefined || Number(m[4]) === 1)) return [m[1], m[2], m[3]].map(Number);
  return null;
}

/** A token's colour in `vars` (following `var(--x)` aliases), or null. */
export function resolve(vars, token, seen = new Set()) {
  const raw = vars.get(token);
  if (raw === undefined || seen.has(token)) return null;
  seen.add(token);
  const alias = /^var\((--[\w-]+)\)$/.exec(raw);
  return alias ? resolve(vars, alias[1], seen) : parseColor(raw);
}

/** Pairs known to fail, each with why it isn't fixed yet. They print as
 *  warnings, not failures; the list may only shrink (its test pins it). */
export const KNOWN = new Map([
  // Empty since overnight F5b (--on-error). Add a pair only with its reason.
]);

/** WCAG relative luminance. */
export function luminance([r, g, b]) {
  const lin = c => { const s = c / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

/** WCAG contrast ratio of two colours (1–21). */
export function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The two themes' token maps: light, and dark over light. */
export function themes(css) {
  const light = block(css, ':root');
  const dark = new Map([...light, ...block(css, ':root[data-theme="dark"]')]);
  return { light, dark };
}

/** Every checked pair in both themes, with its ratio; a pair whose colour
 *  isn't a plain opaque colour is reported as unreadable (a failure). */
export function check(css, pairs = [...TEXT, ...EXTRA]) {
  const rows = [];
  for (const [theme, vars] of Object.entries(themes(css))) {
    for (const { token, on, min } of pairs) {
      for (const bg of on) {
        const fg = resolve(vars, token);
        const back = resolve(vars, bg);
        const ratio = fg && back ? contrast(fg, back) : null;
        const ok = ratio !== null && ratio >= min;
        const known = !ok ? KNOWN.get(`${theme} ${token} on ${bg}`) ?? null : null;
        rows.push({ theme, token, bg, min, ratio, ok, known });
      }
    }
  }
  return rows;
}

function main() {
  const rows = check(readFileSync(TOKENS, 'utf8'));
  const report = process.argv.includes('--report');
  const failed = rows.filter(r => !r.ok && !r.known);
  for (const r of report ? rows : rows.filter(r => !r.ok)) {
    const ratio = r.ratio === null ? 'unreadable' : `${r.ratio.toFixed(2)}:1`;
    const mark = r.ok ? 'ok  ' : r.known ? 'KNOWN' : 'FAIL';
    console.log(`${mark} ${r.theme.padEnd(5)} ${r.token} on ${r.bg}: ${ratio} (needs ${r.min}:1)${r.known ? ` — ${r.known}` : ''}`);
  }
  if (failed.length) {
    console.error(`contrast: ${failed.length} of ${rows.length} pairs under WCAG AA in src/styles/tokens.css`);
    process.exit(1);
  }
  const known = rows.filter(r => r.known).length;
  console.log(`contrast: ok (${rows.length} pairs, light and dark${known ? `; ${known} known failure${known === 1 ? '' : 's'}` : ''})`);
}

// Compare real paths: a symlinked argv[1] must not skip the check silently.
const isMain = () => {
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
};

if (isMain()) main();
