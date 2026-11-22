#!/usr/bin/env node
/**
 * Build the Hollow Canvas Kit (Canvas C2) into the generated files every
 * artifact document inlines:
 *
 *   src/canvas/kit/index.ts (+ manifest, registry, components/) → generated/kit.min.js
 *   src/canvas/kit/kit.css                                     → generated/kit.min.css
 *
 * and the ask kit (Canvas C4 A1), the input components the host inlines only
 * into ask-canvases, after the display kit:
 *
 *   src/canvas/kit/ask/index.ts (+ model, field, components/)  → generated/kit-ask.min.js
 *   src/canvas/kit/ask/ask.css                                 → generated/kit-ask.min.css
 *
 * Deterministic: the same sources and esbuild version give the same bytes,
 * which matters because the safe artifact modes (Offline, Images, HTTPS) and
 * the app CSP allow the kit script by its sha256 (`sync-sandbox-csp.mjs`
 * computes it from generated/kit.min.js; artifacts.rs hashes the same file).
 * Runs as part of `npm run build`, before the hash sync. `--check` exits 1
 * when the committed files are stale instead of writing them;
 * `scripts/canvas-kit.test.mjs` does the same check plus the size budget.
 */
import { buildSync, transformSync } from 'esbuild';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
export const KIT_DIR = join(root, 'src', 'canvas', 'kit');
export const KIT_JS_PATH = join(KIT_DIR, 'generated', 'kit.min.js');
export const KIT_CSS_PATH = join(KIT_DIR, 'generated', 'kit.min.css');

/** kit CSS + JS, minified, must stay within this. Plan §3 set 40 KB; raised
 *  to 56 KB on 2026-10-07 for `hc-flow`, which replaces whiteboards (the
 *  kit is inlined by the host into each frame, never sent to an agent).
 *  72 KB on 2026-10-11 for four bounded SVG instruments and chart motion.
 *  84 KB for obstacle routing, transform-only flow motion and accessible pan/zoom.
 *  88 KB for checklist display; ask controls reuse matrix.
 *  The kit stays outside the app launch chunk. */
// 96 KiB for the requested whole-canvas static material, shared tile and
// solid reading plates. 97 KiB (2026-11-14) for opacity-only chart fills
// (ditherFill.ts, bar and sparkline fades), which measured 972 bytes over
// 96 KiB; the old stipple-overlay chart code it replaces is already netted
// out. 99 KiB (2026-11-14, hover cache, donut fill, legend looks, Quiet fades):
// the second raise, 2 KiB for those four gaps, with the cache itself netting
// the repaint cost out of every hover. No runtime dependency or launch-chunk import.
export const KIT_BUDGET_BYTES = 99 * 1024;

export const KIT_ASK_JS_PATH = join(KIT_DIR, 'generated', 'kit-ask.min.js');
export const KIT_ASK_CSS_PATH = join(KIT_DIR, 'generated', 'kit-ask.min.css');
/** The ask kit's own budget (CSS + JS, minified): 24 KB, the slice's
 *  ceiling, which is its measured size at C4 A1 (23,671 bytes) plus 905 bytes
 *  of margin. The display kit's 40 KB stays its own: an ask-canvas carries
 *  both. Raising it needs a written reason (docs/history/CANVAS_C4.md). */
// 25 KiB includes the static ask-field material; answer contracts stay intact.
export const KIT_ASK_BUDGET_BYTES = 25 * 1024;

/** WKWebView on the oldest supported macOS is Safari 16-class; Chromium runs
 *  the smoke suite. Pinned so the output doesn't drift with esbuild defaults. */
const TARGET = ['safari16', 'chrome110'];

function bundle(entry, cssFile) {
  const js = buildSync({
    entryPoints: [entry],
    bundle: true,
    minify: true,
    format: 'iife',
    target: TARGET,
    charset: 'ascii',
    legalComments: 'none',
    // Private members are written `_name` and shortened. This also keeps a
    // component's own state from shadowing an HTMLElement property (`dir`).
    mangleProps: /^_/,
    // ...but never a bridge field: `__hc` is the wire format the host reads.
    reserveProps: /^__/,
    write: false,
    logLevel: 'silent',
  }).outputFiles[0].text;
  const css = transformSync(readFileSync(cssFile, 'utf8'), {
    loader: 'css',
    minify: true,
    target: TARGET,
    charset: 'ascii',
    legalComments: 'none',
    logLevel: 'silent',
  }).code;
  return { js, css };
}

export function buildKit() {
  return bundle(join(KIT_DIR, 'index.ts'), join(KIT_DIR, 'kit.css'));
}

export function buildKitAsk() {
  return bundle(join(KIT_DIR, 'ask', 'index.ts'), join(KIT_DIR, 'ask', 'ask.css'));
}

export function kitBytes({ js, css }) {
  return Buffer.byteLength(js) + Buffer.byteLength(css);
}

/** Returns a list of problems with a built kit (empty when it is fine). */
export function checkKit(kit, budget = KIT_BUDGET_BYTES, name = 'kit') {
  const problems = [];
  const size = kitBytes(kit);
  if (size > budget) problems.push(`${name} is ${size} bytes minified, over the ${budget}-byte budget`);
  // Inlined into <script>/<style>: a closing tag would end the element early.
  if (/<\/script/i.test(kit.js)) problems.push(`${name}.min.js contains </script`);
  if (/<\/style/i.test(kit.css)) problems.push(`${name}.min.css contains </style`);
  // The kit loads nothing and evaluates nothing.
  for (const [what, re] of [
    ['eval', /\beval\s*\(/], ['Function constructor', /\bFunction\s*\(/], ['fetch', /\bfetch\s*\(/],
    ['XMLHttpRequest', /XMLHttpRequest/], ['WebSocket', /WebSocket/], ['EventSource', /EventSource/],
    ['sendBeacon', /sendBeacon/], ['importScripts', /importScripts/], ['dynamic import', /\bimport\s*\(/],
    ['window.open', /\.open\s*\(/], ['innerHTML', /innerHTML|outerHTML|insertAdjacentHTML/],
  ]) if (re.test(kit.js)) problems.push(`${name}.min.js uses ${what}`);
  if (/url\(|@import/i.test(kit.css)) problems.push(`${name}.min.css loads a resource (url() or @import)`);
  return problems;
}

function isMain() {
  return process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1];
}

if (isMain()) {
  const outputs = [
    { name: 'kit', built: buildKit(), budget: KIT_BUDGET_BYTES, js: KIT_JS_PATH, css: KIT_CSS_PATH },
    { name: 'kit-ask', built: buildKitAsk(), budget: KIT_ASK_BUDGET_BYTES, js: KIT_ASK_JS_PATH, css: KIT_ASK_CSS_PATH },
  ];
  const problems = outputs.flatMap(o => checkKit(o.built, o.budget, o.name));
  if (problems.length) {
    for (const p of problems) console.error(`build-canvas-kit: ${p}`);
    process.exit(1);
  }
  const read = (p) => { try { return readFileSync(p, 'utf8'); } catch { return null; } };
  const sizes = outputs.map(o => `${o.name} ${kitBytes(o.built)} bytes`).join(', ');
  const stale = outputs.filter(o => read(o.js) !== o.built.js || read(o.css) !== o.built.css);
  if (process.argv.includes('--check')) {
    if (stale.length) {
      console.error('build-canvas-kit: src/canvas/kit/generated is stale; run npm run kit:build');
      process.exit(1);
    }
    console.log(`build-canvas-kit: up to date (${sizes})`);
  } else if (stale.length) {
    for (const o of stale) {
      writeFileSync(o.js, o.built.js);
      writeFileSync(o.css, o.built.css);
    }
    console.log(`build-canvas-kit: wrote generated ${stale.map(o => o.name).join(' and ')} (${sizes})`);
  } else {
    console.log(`build-canvas-kit: up to date (${sizes})`);
  }
}
