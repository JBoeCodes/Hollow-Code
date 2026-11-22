#!/usr/bin/env node
/**
 * Fails the build when a stylesheet reads a custom property that nothing
 * defines and supplies no fallback.
 *
 * Why this needs a check rather than a convention: `color: var(--nope)` is not
 * a CSS parse error. The declaration is computed as invalid-at-computed-value
 * time and dropped, so the property silently falls back to inherited or
 * initial. 93 declarations had rotted this way — mostly tokens renamed during
 * the Carved Light pass (`--radius-sm` after the scale became `--r-*`,
 * `--font-mono` after it became `--mono`) whose call sites were never updated.
 * Nothing in the pipeline noticed, because the CSS still built and still
 * looked plausible.
 *
 * Runtime-set properties are the legitimate exception: `--pct` on a meter fill
 * and `--i` on a stagger index are written from JS as inline styles, so they
 * never appear in a stylesheet's `:root`. Those are allowlisted below and must
 * stay a short, deliberate list — a name that lands here should genuinely be
 * assigned at runtime.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const STYLE_DIR = 'src/styles';

/**
 * Custom properties assigned from JS at runtime, never declared in CSS. Keep
 * this list minimal and exact: every name here is a hole in the check. It was
 * briefly padded with defensive guesses, two of which (`--breathe-base`,
 * `--breathe-min`) are in fact declared in CSS, and one of which (`--warn`)
 * was simply a misspelling of `--warning` that the allowlist would then have
 * hidden forever. Only names with no CSS declaration and a real runtime
 * assignment belong here.
 */
const RUNTIME_SET = new Set([
  '--pct',        // .meter-fill scaleX, 0..1
  '--i',          // list stagger index
  '--dx', '--dy', // reveal-origin offsets
  '--progress',   // day meter fill
  '--weeks',      // ActivityHeatmap column count
]);

const files = [
  ...readdirSync(STYLE_DIR).filter(f => f.endsWith('.css')).map(f => join(STYLE_DIR, f)),
  'src/styles.css',
];

const defined = new Set();
const sources = new Map();
for (const file of files) {
  let text;
  try { text = readFileSync(file, 'utf8'); } catch { continue; }
  sources.set(file, text);
  for (const m of text.matchAll(/^\s*(--[A-Za-z0-9-]+)\s*:/gm)) defined.add(m[1]);
}

// Inspect blocks, not an @layer substring: comments and a layered prefix
// must not hide rules that would outrank the shell's keyboard focus ring.
const declaredLayers = new Set((sources.get('src/styles.css')?.match(/@layer\s+([^;{]+);/)?.[1] ?? '').split(',').map(s => s.trim()));
const specialLayers = {
  tokens: ['src/styles/tokens.css'],
  base: ['src/styles/foundations.css'],
  glass: ['src/styles/glass.css'],
  a11y: ['src/styles/foundations.css', 'src/styles/usage-instruments.css', 'src/styles/chat-motion.css', 'src/styles/empty-art.css'],
};
const editorDescendants = new Set([
  '.cm-md-image img',
  '.cm-tooltip.cm-tooltip-autocomplete > ul',
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li',
  '.cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]',
]);
function unlayeredRules(file, text) {
  const stripped = text
    .replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '))
    .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, m => m.replace(/[^\n]/g, ' '));
  const stack = [];
  const out = [];
  let start = 0;
  for (const m of stripped.matchAll(/[{};]/g)) {
    const head = stripped.slice(start, m.index).trim();
    if (m[0] === '{') {
      const line = text.slice(0, start + stripped.slice(start, m.index).search(/\S/)).split('\n').length;
      if (head.startsWith('@layer')) {
        const name = head.slice(6).trim();
        if (!declaredLayers.has(name) || (!['components', 'shell'].includes(name) && !specialLayers[name]?.includes(file))) {
          out.push({ line, text: `Unexpected layer: ${head}` });
        }
      }
      const layered = stack.some(s => /^@layer\s/.test(s));
      const keyframe = stack.some(s => /^@(?:-webkit-)?keyframes\s/.test(s));
      // CodeMirror injects an unlayered theme. Only selectors scoped to its
      // editor classes may stay unlayered; app-owned controls belong in layers.
      const editor = file === 'src/styles/codemirror.css'
        && !head.includes('(')
        && head.split(',').every(s => editorDescendants.has(s.trim())
          || /^\.(?:cm-[\w-]+|vault-editor-host)\b/.test(s.trim().split(/[\s>+~]+/).at(-1)));
      if (head && !head.startsWith('@') && !layered && !keyframe && !editor) {
        out.push({ line, text: head.slice(0, 100) });
      }
      stack.push(head);
    } else if (m[0] === '}') stack.pop();
    start = m.index + 1;
  }
  return out;
}
const unlayered = [...sources].flatMap(([file, text]) => unlayeredRules(file, text).map(rule => ({ file, ...rule })));
if (unlayered.length) {
  console.error('\nUnlayered CSS rules override the shell focus contract:');
  for (const rule of unlayered) console.error(`  ${rule.file}:${rule.line}  ${rule.text}`);
  console.error('Put component rules inside @layer components; deliberate focus exceptions use @layer shell.');
  process.exit(1);
}

/**
 * A declaration outside any rule. esbuild only WARNS on these, so `npm run build`
 * still exits 0 and a grep for "built in" hides it — which is exactly how a
 * misplaced `align-self: start;` survived two commits. Braces are counted per
 * file, ignoring comments and strings, and any `prop: value;` seen at depth 0 is
 * an error.
 */
function strayDeclarations(text) {
  const stripped = text
    .replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '))
    .replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, m => m.replace(/[^\n]/g, ' '));
  const out = [];
  let depth = 0;
  stripped.split('\n').forEach((line, i) => {
    const before = depth;
    for (const ch of line) {
      if (ch === '{') depth++;
      else if (ch === '}') depth = Math.max(0, depth - 1);
    }
    // At depth 0 an at-rule or a selector is fine; a `prop: value;` is not.
    if (before === 0 && depth === 0 && /^\s*[-a-zA-Z]+\s*:[^;]*;\s*$/.test(line) && !/^\s*@/.test(line)) {
      out.push({ line: i + 1, text: line.trim() });
    }
  });
  return out;
}

/**
 * An `animation-name` with no matching `@keyframes` never runs — not even the
 * `both` fill — so the rule reads as deliberate motion while doing nothing.
 * `riseIn` on the Action Queue rows and `row-fade-in` on the activity rows both
 * shipped this way.
 *
 * This has to be a text check, not a CSSOM one: when an `animation` shorthand
 * contains a `var()`, the whole value is stored as a pending-substitution and
 * `rule.style.animationName` comes back EMPTY, so a stylesheet walk in the
 * browser silently sees nothing to check. Every animation in this codebase uses
 * `var(--t-*)` for its duration, so that failure mode would cover all of them.
 */
const KEYWORDS = new Set([
  'none', 'inherit', 'initial', 'unset', 'revert', 'infinite', 'normal', 'reverse',
  'alternate', 'forwards', 'backwards', 'both', 'running', 'paused', 'linear', 'ease',
  'ease-in', 'ease-out', 'ease-in-out', 'step-start', 'step-end',
]);

function danglingAnimations(text, defined) {
  const out = [];
  text.split('\n').forEach((line, i) => {
    const m = line.match(/^\s*animation(?:-name)?\s*:\s*([^;]+);/);
    if (!m) return;
    // Drop var() calls and timing functions, then take the bare identifiers.
    const cleaned = m[1].replace(/var\([^)]*\)/g, ' ').replace(/[a-z-]+\([^)]*\)/gi, ' ');
    for (const tok of cleaned.split(/[\s,]+/).filter(Boolean)) {
      if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(tok)) continue; // times, counts
      if (KEYWORDS.has(tok.toLowerCase())) continue;
      if (defined.has(tok)) continue;
      out.push({ line: i + 1, name: tok, text: line.trim() });
    }
  });
  return out;
}

const keyframes = new Set();
for (const text of sources.values()) {
  for (const m of text.matchAll(/@keyframes\s+([A-Za-z][A-Za-z0-9_-]*)/g)) keyframes.add(m[1]);
}
const dangling = [];
for (const [file, text] of sources) {
  for (const d of danglingAnimations(text, keyframes)) dangling.push({ file, ...d });
}
if (dangling.length) {
  console.error(`\n${dangling.length} animation(s) name a @keyframes that does not exist:\n`);
  for (const d of dangling) console.error(`  ${d.file}:${d.line}  ${d.name}  —  ${d.text.slice(0, 88)}`);
  console.error('\nAn unresolvable animation-name does not run at all. Delete it or define the keyframes.\n');
  process.exit(1);
}

/**
 * Every infinite ambient animation must opt into the pause switch by declaring
 * `animation-play-state: var(--ambient-play, …)` after its `animation:`
 * shorthand, so the hearth actually banks when the window loses focus.
 *
 * This exists because the previous mechanism was a hand-maintained list of nine
 * selectors in one file. It covered 11 of 50 infinite animations and had no way
 * to notice the other 39 — exactly the rot this check prevents. Live-work
 * indicators (spinners, streaming cursors, running pills, drop targets) must
 * keep moving while banked; they opt out by being listed here.
 */
const LIVE_WORK = new Set([
  'src/styles/foundations.css:spinner-turn',
  'src/styles/library-today.css:pulse',
  'src/styles/library-today.css:breathe',
  'src/styles/notifications-people.css:chat-attach-stripe',
  'src/styles/vault-inbox-chat.css:shimmer',
  'src/styles/workflows.css:workflow-running-pulse',
]);

/**
 * Keyframes that exist only to show work in progress. Any rule may use them
 * without the pause opt-in: a spinner or a busy pulse must keep moving while
 * the window is banked, and resting states never use them.
 */
const LIVE_WORK_KEYFRAMES = new Set(['spinner-turn', 'status-pulse']);

function ambientOptOuts(file, text) {
  const stripped = text.replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '));
  const lines = stripped.split('\n');
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    if (!/animation\s*:/.test(lines[i])) continue;
    // The shorthand can wrap; gather until the semicolon.
    let decl = lines[i];
    let end = i;
    while (!decl.includes(';') && end + 1 < lines.length && end - i < 4) decl += ' ' + lines[++end];
    if (!decl.includes('infinite')) continue;
    const name = (decl.match(/animation\s*:\s*([A-Za-z][A-Za-z0-9_-]*)/) || [])[1] || '?';
    // A busy/running/working state selector is live work, keyed by name+state.
    // Walk back over a wrapped selector list to find the state class.
    const selector = lines.slice(Math.max(0, i - 3), i + 1).join(' ');
    const busy = /\bbusy\b|\brunning\b|\bworking\b|\bsaving\b|\bpending\b|is-running/.test(selector);
    if (LIVE_WORK_KEYFRAMES.has(name)) continue;
    if (LIVE_WORK.has(`${file}:${name}`) || (busy && LIVE_WORK.has(`${file}:${name}-busy`))) continue;
    // Look for the opt-in on the declaration or the next 3 NON-BLANK lines —
    // stripped comments leave blank lines behind, so a fixed offset misses it.
    const after = [];
    for (let k = end + 1; k < lines.length && after.length < 3; k++) {
      if (lines[k].trim()) after.push(lines[k]);
    }
    const window = [decl, ...after].join(' ');
    if (window.includes('animation-play-state') && window.includes('--ambient-play')) continue;
    out.push({ line: i + 1, name, text: lines[i].trim() });
  }
  return out;
}

const unpaused = [];
for (const [file, text] of sources) {
  for (const u of ambientOptOuts(file, text)) unpaused.push({ file, ...u });
}
if (unpaused.length) {
  console.error(`\n${unpaused.length} infinite ambient animation(s) do not opt into the pause switch:\n`);
  for (const u of unpaused) console.error(`  ${u.file}:${u.line}  ${u.name}  —  ${u.text.slice(0, 80)}`);
  console.error('\nAdd `animation-play-state: var(--ambient-play, running);` AFTER the');
  console.error('`animation:` shorthand (the shorthand resets play-state, so order matters),');
  console.error('or add the animation to LIVE_WORK in scripts/check-css-tokens.mjs if it must');
  console.error('keep running while the window is banked.\n');
  process.exit(1);
}

/**
 * The token contract (frontier F2). Three things are errors anywhere outside
 * src/styles/tokens.css:
 *   - a raw hex colour — colours are tokens, so light, dark, glass and custom
 *     palettes all reach every surface;
 *   - a px font size (in `font-size` or a `font` shorthand) — sizes come from
 *     the --fs-* scale;
 *   - `!important`, except inside the `@layer a11y` block (the motion
 *     preference gates, which must beat CodeMirror's unlayered theme and inline
 *     styles). Precedence comes from cascade layers (styles.css), not force.
 * Comments and data: URLs are ignored. There is deliberately no allowlist.
 */
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, m => m.replace(/[^\n]/g, ' '));
}
function layerSpans(text, name) {
  const spans = [];
  const re = new RegExp(`@layer\\s+${name}\\s*\\{`, 'g');
  let m;
  while ((m = re.exec(text))) {
    let depth = 0, i = m.index + m[0].length - 1;
    for (; i < text.length; i++) { if (text[i] === '{') depth++; else if (text[i] === '}' && --depth === 0) break; }
    spans.push([m.index, i]);
  }
  return spans;
}
const contract = [];
for (const [file, raw] of sources) {
  if (file.endsWith('/tokens.css') || file === 'src/styles.css') continue;
  const text = stripComments(raw).replace(/url\((?:"[^"]*"|'[^']*'|[^)]*)\)/g, m => m.replace(/[^\n]/g, ' '));
  const a11y = layerSpans(text, 'a11y');
  const lineAt = idx => text.slice(0, idx).split('\n').length;
  for (const m of text.matchAll(/#[0-9a-fA-F]{3,8}\b/g)) contract.push({ file, line: lineAt(m.index), kind: 'raw hex colour', text: m[0] });
  for (const m of text.matchAll(/font-size\s*:\s*[^;{}]*?\d(?:\.\d+)?px/g)) contract.push({ file, line: lineAt(m.index), kind: 'px font size', text: m[0].trim() });
  for (const m of text.matchAll(/(?:^|[\s;{])font\s*:\s*([^;{}]*)/g)) {
    if (/(^|\s)\d+(?:\.\d+)?px(\/|\s)/.test(m[1])) contract.push({ file, line: lineAt(m.index), kind: 'px font size', text: `font: ${m[1].trim()}`.slice(0, 70) });
  }
  for (const m of text.matchAll(/!\s*important/g)) {
    if (a11y.some(([s, e]) => m.index > s && m.index < e)) continue;
    contract.push({ file, line: lineAt(m.index), kind: '!important', text: '!important' });
  }
}
if (contract.length) {
  console.error(`\n${contract.length} token-contract violation(s):\n`);
  for (const c of contract) console.error(`  ${c.file}:${c.line}  ${c.kind}  —  ${c.text}`);
  console.error('\nUse a token from src/styles/tokens.css (add one there if the value is new),');
  console.error('a --fs-* step for type, and a cascade layer instead of !important.\n');
  process.exit(1);
}

const problems = [];
for (const [file, text] of sources) {
  text.split('\n').forEach((line, i) => {
    // Only bare var(--x) with no comma: a fallback makes an undefined token safe.
    for (const m of line.matchAll(/var\(\s*(--[A-Za-z0-9-]+)\s*\)/g)) {
      const token = m[1];
      if (defined.has(token) || RUNTIME_SET.has(token)) continue;
      problems.push({ file, line: i + 1, token, text: line.trim() });
    }
  });
}

const strays = [];
for (const [file, text] of sources) {
  for (const s of strayDeclarations(text)) strays.push({ file, ...s });
}
if (strays.length) {
  console.error(`\n${strays.length} CSS declaration(s) sit outside any rule and do nothing:\n`);
  for (const s of strays) console.error(`  ${s.file}:${s.line}  ${s.text}`);
  console.error('\nesbuild only warns about these, so the build still passes. Fixed here instead.\n');
  process.exit(1);
}

if (problems.length === 0) {
  console.log(`css tokens ok — ${defined.size} defined, no undefined reads without a fallback`);
  process.exit(0);
}

console.error(`\n${problems.length} undefined CSS custom propert${problems.length === 1 ? 'y' : 'ies'} read without a fallback.`);
console.error('Each of these declarations is silently DROPPED by the browser.\n');
const byToken = new Map();
for (const p of problems) {
  if (!byToken.has(p.token)) byToken.set(p.token, []);
  byToken.get(p.token).push(p);
}
for (const [token, hits] of [...byToken].sort((a, b) => b[1].length - a[1].length)) {
  console.error(`  ${token}  (${hits.length} site${hits.length === 1 ? '' : 's'})`);
  for (const h of hits.slice(0, 3)) console.error(`      ${h.file}:${h.line}  ${h.text.slice(0, 88)}`);
  if (hits.length > 3) console.error(`      ... and ${hits.length - 3} more`);
}
console.error('\nFix by pointing at a real token, adding a fallback, or — if it is');
console.error('assigned from JS — adding it to RUNTIME_SET in scripts/check-css-tokens.mjs.\n');
process.exit(1);
