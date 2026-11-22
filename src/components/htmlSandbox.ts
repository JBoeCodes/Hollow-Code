/**
 * Shared security model for rendering agent-authored HTML inside a
 * HARD-SANDBOXED iframe. Used by both `HtmlArtifact` (chat ```html / ```svg
 * fences) and `HtmlDoc` (full-document workflow briefs). Single-sourcing the
 * load-bearing pieces here keeps the sandbox guarantees identical across both
 * surfaces — change the security model in one place, not two.
 *
 * The guarantees:
 *   - `SANDBOX` is `allow-scripts` WITHOUT `allow-same-origin`, `allow-popups`
 *     or `allow-forms` → the document runs in an opaque (null) origin and can
 *     open no window and submit no form. `allow-scripts` stays on even in safe
 *     preview mode so the host-owned runtime (allowed by the document's nonce)
 *     can measure height and
 *     bridge vault-link clicks; the CSP blocks artifact-authored scripts
 *     outside Canvas (scripts, no network) and Interactive (scripts and HTTPS).
 *     Which policy a document gets is its surface's default, raised only by
 *     the person's grant after a refused load (10-11 B1, canvasGrants.ts).
 *   - A strict child CSP is injected as a <meta> into the srcdoc AND served as
 *     the artifact:// response header, capping what the artifact may load. The
 *     null origin already blocks app endpoints, so this is defense-in-depth,
 *     not the primary control.
 *   - The parent app CSP must allow `frame-src 'self'` for the srcdoc to load
 *     (see tauri.conf.json).
 *
 * CSP INHERITANCE — the production foot-gun (June 2026): a srcdoc document
 * INHERITS the embedding page's CSP, and the app's production CSP has no
 * 'unsafe-inline' in script-src (deliberately — keep the parent strict). So
 * inline <script> inside the sandbox runs in `tauri dev` (no devCsp is set →
 * no CSP) but is silently BLOCKED in installed builds. And `script-src 'self'`
 * can't rescue an external script: the inherited policy is evaluated in the
 * CHILD document, whose sandboxed origin is opaque, so 'self' matches nothing
 * there. The host-side sandbox runtime (height ping, vault-link nav bridge,
 * error bridge) therefore lives in `./sandbox-doc.js`, is embedded VERBATIM
 * via a `?raw` import, and is allowed by its sha256 HASH in tauri.conf.json's
 * script-src — hash-sources match content, independent of origin.
 * `scripts/sync-sandbox-csp.mjs` (run by `npm run build`) keeps that hash in
 * lockstep with the file.
 *
 * RESOLVED (June 2026) — artifact inline JS in installed builds: documents
 * now load off the dedicated `artifact://` custom protocol by default
 * (`artifactDocUrl` below → `artifact_put` → `artifacts.rs` serves the doc
 * with ITS OWN CSP header, mirroring `childCsp(mode)`). A real-URL document
 * does not inherit the app CSP, so Interactive artifacts can run scripts in
 * installed builds when the user explicitly enables them. srcdoc (and the
 * sandbox-doc.js hash) remains as the non-Tauri fallback path.
 *
 * Canvas C1 (2026-10-04): the frontend names a MODE and `artifacts.rs` serves
 * that mode's policy, so this file no longer supplies the header. `childCsp`
 * still emits the matching `<meta>` (both are tested against
 * `artifactCsp.golden.json`). Canvas mode cannot work from srcdoc, which
 * inherits the app CSP, so outside Tauri a Canvas document renders Offline
 * (`srcdocMode`).
 *
 * App theme tokens are exposed one-way as `--app-*` CSS variables so artifacts
 * match Hollow without being able to mutate it, and are kept current over the
 * canvas bridge (`watchFrameTheme`, `src/canvasBridge.ts`). Canvas C2 widened
 * the set from 9 tokens to the full colour palette (`THEME_TOKENS`).
 *
 * Canvas C2 (kit@1): every artifact document also inlines the Hollow Canvas
 * Kit (`src/canvas/kit/`, built into `generated/` by
 * `scripts/build-canvas-kit.mjs`): its stylesheet, and its script after the
 * runtime. In Canvas and Interactive the kit script runs under
 * 'unsafe-inline' (no hash or nonce may be listed there). In Offline, Images
 * and HTTPS it runs by the document's nonce, like the runtime, so host-owned
 * components render in the default mode while artifact scripts stay blocked.
 * The kit's bytes load lazily (`loadCanvasKit`), off the App chunk.
 *
 * Canvas C4 A1: an ask-canvas (`<hc-canvas ask>`) also inlines the ask kit
 * (`generated/kit-ask.min.{js,css}`, the input components), its stylesheet
 * after the kit's and its script after the kit's, with a nonce slot of its
 * own in the script-free modes. Other documents never carry it.
 *
 * SRI repair (2026-10-05): the script-free modes used to allow the runtime and
 * kit by sha256. Under CSP3 a hash-source also allows an EXTERNAL script whose
 * `integrity` carries that hash, and browsers request it before the integrity
 * check fails, so agent HTML could leak data in a script URL. Those modes now
 * allow `'nonce-…'` only. The document is built as PARTS (`sandboxDocParts`)
 * with a nonce slot in the policy meta and on the runtime and kit tags, all
 * before the agent's body: `artifacts.rs` draws the served document's nonce
 * and joins the parts with it; the srcdoc fallback seals them with its own
 * (`sealDoc`). Nothing ever searches the text for a placeholder, so agent
 * content can neither carry nor receive a nonce. The two hashes remain only in
 * the app CSP (`tauri.conf.json`), which a srcdoc document inherits; the
 * nonce meta still blocks external scripts there (both policies apply).
 */

import SANDBOX_DOC_JS from './sandbox-doc.js?raw';
import { convertFileSrc } from '@tauri-apps/api/core';
import { api } from '../api';
import { isSafeThemeValue, themeMessage, type ColorScheme } from '../canvasBridge';
import { createEffect, createRoot } from 'solid-js';
import type { Look } from '../canvasLook';
import type { CanvasKitAssets } from '../canvas/kitAssets';
import { motionQuery, motionReduced } from '../motion';

/** Opaque origin, no popups: a fresh popup could recover WebRTC constructors. */
export const SANDBOX = 'allow-scripts';

export type ArtifactMode = 'offline' | 'canvas' | 'images' | 'https' | 'interactive';

export const DEFAULT_ARTIFACT_MODE: ArtifactMode = 'offline';

/** The five policies `artifacts.rs` serves (golden: `artifactCsp.golden.json`).
 *  Since 10-11 B1 none is a button: a document opens in its surface's default
 *  and a refused load asks the person (`canvas/canvasGrants.ts`). */
export const ARTIFACT_MODES: readonly ArtifactMode[] = ['offline', 'canvas', 'images', 'https', 'interactive'];

// The runtime's, the kit's and the ask kit's (C4) sha256, kept in sync with
// tauri.conf.json's script-src by scripts/sync-sandbox-csp.mjs. They are for the app CSP only,
// which a srcdoc fallback inherits: no artifact policy may list them (an
// external script carrying one as `integrity` would load; SRI repair).
export const SANDBOX_RUNTIME_HASH = "'sha256-WrspaX7PYrE0J+5diIJgEEywjQGwmtqRd3zkfcRzjlY='";
export const KIT_SCRIPT_HASH = "'sha256-0eTRAvSdeXh94UmhzFAlXCO9bvLT4CZT51UpZq9zXTI='";
export const KIT_ASK_SCRIPT_HASH = "'sha256-atkEmkPt3QP1zhMwseb0Es0KYM9F/GyK0uMW3Al/GM4='";

/** Whether a mode runs the artifact's own scripts. Links a scripted frame asks
 *  to open always need the host's confirmation (`openFrameLink`). */
export function modeRunsScripts(mode: ArtifactMode): boolean {
  return mode === 'canvas' || mode === 'interactive';
}

/** The mode a srcdoc fallback renders in. srcdoc inherits the app CSP, so a
 *  Canvas document's scripts could not run there anyway: say so by rendering
 *  Offline rather than pretending. */
export function srcdocMode(mode: ArtifactMode): ArtifactMode {
  return mode === 'canvas' ? 'offline' : mode;
}

/** Whether a mode's host scripts run by the document's nonce (the
 *  script-free modes). Canvas and Interactive run them under
 *  'unsafe-inline' and must list no nonce, which would disable it. */
export function modeUsesNonce(mode: ArtifactMode): boolean {
  return mode === 'offline' || mode === 'images' || mode === 'https';
}

/** A nonce is base64 of at least 128 bits; `artifacts.rs` and `newNonce`
 *  both make 24 characters. */
const NONCE_RE = /^[A-Za-z0-9+/]{22,}$/;

/** A fresh per-document nonce for the srcdoc fallback: 18 bytes (144 bits)
 *  from `crypto.getRandomValues`, standard base64 (the served document's comes
 *  from `artifacts.rs` instead). */
export function newNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(18));
  return btoa(String.fromCharCode(...bytes));
}

/** Where a document's nonce goes: a slot between two parts, never text. */
const SLOT = Symbol('nonce slot');
type Piece = string | typeof SLOT;

function toParts(pieces: Piece[]): string[] {
  const parts = [''];
  for (const piece of pieces) {
    if (piece === SLOT) parts.push('');
    else parts[parts.length - 1] += piece;
  }
  return parts;
}

function cspPieces(mode: ArtifactMode): Piece[] {
  const scriptSrc: Piece[] = mode === 'interactive'
    ? ["'unsafe-inline' https:"]
    : mode === 'canvas'
      ? ["'unsafe-inline'"]
      : ["'nonce-", SLOT, "'"];
  const styleSrc = mode === 'https' || mode === 'interactive'
    ? "'unsafe-inline' https:"
    : "'unsafe-inline'";
  const imgSrc = mode === 'offline' || mode === 'canvas'
    ? 'data: blob:'
    : 'data: blob: https:';
  const fontSrc = mode === 'https' || mode === 'interactive'
    ? 'data: https:'
    : 'data:';
  const connectSrc = mode === 'https' || mode === 'interactive'
    ? 'https:'
    : "'none'";
  const mediaSrc = mode === 'https' || mode === 'interactive'
    ? 'data: blob: https:'
    : 'data: blob:';
  const workerSrc = mode === 'interactive' ? 'blob:' : "'none'";

  const rest = [
    `style-src ${styleSrc}`,
    `img-src ${imgSrc}`,
    `font-src ${fontSrc}`,
    `connect-src ${connectSrc}`,
    `media-src ${mediaSrc}`,
    `worker-src ${workerSrc}`,
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
  ].join('; ');
  return ["default-src 'none'; script-src ", ...scriptSrc, `; ${rest}`];
}

/** Strict CSP injected as the child document's `<meta>`. The served header
 *  comes from `artifacts.rs`; both match `artifactCsp.golden.json`. Offline,
 *  Images and HTTPS allow only `nonce` (never a hash: SRI repair); Canvas and
 *  Interactive list no nonce, which would disable 'unsafe-inline'. */
export function childCsp(mode: ArtifactMode, nonce: string): string {
  if (!NONCE_RE.test(nonce)) throw new Error('childCsp: not a nonce');
  return toParts(cspPieces(mode)).join(nonce);
}

/**
 * Register a fully-scaffolded document with the backend and return the
 * `artifact://` URL to load it from (null outside Tauri — callers fall back
 * to srcdoc, where the same CSP meta still applies).
 *
 * WHY: a document served off the dedicated scheme gets ITS OWN selected CSP
 * from the response header (`artifacts.rs`), instead of inheriting the app CSP
 * the way srcdoc does. The iframe sandbox (opaque origin, postMessage-only) is
 * unchanged; this only moves WHERE the document's CSP comes from. `parts` is
 * `sandboxDocParts`' output: Rust draws the nonce and fills the slots.
 */
export async function artifactDocUrl(parts: string[], mode: ArtifactMode = DEFAULT_ARTIFACT_MODE): Promise<string | null> {
  try {
    const id = await api.artifactPut(parts, mode);
    return convertFileSrc(id, 'artifact');
  } catch {
    return null;
  }
}

/** The app tokens a frame sees: `[frame name, app token, fallback]`. The
 *  first nine are the C1 set (names kept for existing artifacts); C2 adds the
 *  full colour palette from `styles/tokens.css`, with the status roles under
 *  kit names (`--error` is `--app-danger`, `--accent-teal` is `--app-info`).
 *  At most `LIMITS.themeVars` (48) entries, each name within the bridge's
 *  `--app-*` rule (canvasBridge.test.ts pins both). */
export const THEME_TOKENS: ReadonlyArray<readonly [string, string, string]> = [
  ['--app-ink', '--ink', '#1a1a1a'],
  ['--app-canvas', '--canvas', '#ffffff'],
  ['--app-surface', '--surface-card', '#ffffff'],
  ['--app-primary', '--primary', '#b5251c'],
  ['--app-accent', '--accent-teal', '#5f6b7a'],
  ['--app-muted', '--muted', '#626262'],
  ['--app-hairline', '--hairline', '#d5d6d8'],
  ['--app-mono', '--mono', 'ui-monospace, SFMono-Regular, monospace'],
  ['--app-display', '--display', 'system-ui, sans-serif'],
  // Surfaces and text
  ['--app-surface-soft', '--surface-soft', '#f3f3f4'],
  ['--app-surface-strong', '--surface-cream-strong', '#e8e8e9'],
  ['--app-surface-raised', '--surface-raised', '#ffffff'],
  ['--app-hairline-soft', '--hairline-soft', '#e8e8e9'],
  ['--app-body', '--body', '#454545'],
  ['--app-body-strong', '--body-strong', '#393839'],
  ['--app-muted-soft', '--muted-soft', '#666666'],
  // Accent
  ['--app-primary-active', '--primary-active', '#dd3227'],
  ['--app-primary-disabled', '--primary-disabled', '#d6c7c5'],
  ['--app-on-primary', '--on-primary', '#ffffff'],
  ['--app-on-danger', '--on-error', '#ffffff'],
  ['--app-primary-soft', '--primary-soft', 'rgba(221, 50, 39, .06)'],
  ['--app-primary-soft-strong', '--primary-soft-strong', 'rgba(221, 50, 39, .12)'],
  // Status roles and their soft fills
  ['--app-info', '--accent-teal', '#5f6b7a'],
  ['--app-info-soft', '--teal-soft', 'rgba(95, 107, 122, 0.12)'],
  ['--app-success', '--success', '#3d7634'],
  ['--app-success-soft', '--success-soft', 'rgba(74, 143, 63, 0.14)'],
  ['--app-warning', '--warning', '#896213'],
  ['--app-warning-soft', '--amber-soft', 'rgba(184, 132, 26, 0.14)'],
  ['--app-danger', '--error', '#b73d3d'],
  ['--app-danger-soft', '--error-soft', 'rgba(183, 61, 61, 0.12)'],
  ['--app-risk', '--risk', '#855699'],
  ['--app-risk-soft', '--risk-soft', 'rgba(138, 90, 158, 0.14)'],
  // Syntax (for code in canvases)
  ['--app-syn-keyword', '--syn-keyword', '#b5251c'],
  ['--app-syn-string', '--syn-string', '#3b7556'],
  ['--app-syn-comment', '--syn-comment', '#666666'],
  ['--app-syn-number', '--syn-number', '#805320'],
  ['--app-syn-function', '--syn-function', '#3a64b0'],
  ['--app-syn-type', '--syn-type', '#2f6f8f'],
  ['--app-syn-builtin', '--syn-builtin', '#805320'],
  ['--app-syn-operator', '--syn-operator', '#626262'],
  ['--app-syn-punct', '--syn-punct', '#666666'],
  ['--app-syn-property', '--syn-property', '#3a64b0'],
  ['--app-syn-tag', '--syn-tag', '#b5251c'],
  ['--app-syn-attr', '--syn-attr', '#2f6f8f'],
  ['--app-syn-meta', '--syn-meta', '#805320'],
  ['--app-text-scale', '--text-scale', '1'],
];

const SAFE_THEME_VAR = /^--app-[a-z0-9-]{1,40}$/;

function pickSafeThemeValue(live: string, fallback: string): string | null {
  const trimmed = live.trim();
  if (isSafeThemeValue(trimmed)) return trimmed;
  return isSafeThemeValue(fallback) ? fallback : null;
}

/** The app theme tokens a frame sees as `--app-*`, read live from `<html>`
 *  (so a custom theme's inline tokens and the dark block both count). Every
 *  name and value is filtered through `isSafeThemeValue` so an unvalidated
 *  theme token cannot break out of `<style>` in `<head>`. */
export function themePairs(look?: Look | null): Array<[string, string]> {
  try {
    const cs = getComputedStyle(document.documentElement);
    if (look) {
      return look.pairs.flatMap(([name, value]): Array<[string, string]> => {
        if (!SAFE_THEME_VAR.test(name)) return [];
        const chosen = name === '--app-mono' || name === '--app-display' || name === '--app-text-scale'
          ? pickSafeThemeValue(cs.getPropertyValue(name.replace('--app-', '--')), value)
          : (isSafeThemeValue(value) ? value : null);
        return chosen ? [[name, chosen]] : [];
      });
    }
    return THEME_TOKENS.flatMap(([name, token, fallback]): Array<[string, string]> => {
      if (!SAFE_THEME_VAR.test(name)) return [];
      const chosen = pickSafeThemeValue(cs.getPropertyValue(token), fallback);
      return chosen ? [[name, chosen]] : [];
    });
  } catch {
    return (look?.pairs ?? []).filter(([name, value]) => SAFE_THEME_VAR.test(name) && isSafeThemeValue(value));
  }
}

/** The tokens as a `:root` declaration list, baked into the scaffold. */
export function themeVars(look?: Look | null): string {
  return themePairs(look).filter(([k, v]) => SAFE_THEME_VAR.test(k) && isSafeThemeValue(v)).map(([k, v]) => `${k}:${v}`).join(';');
}

export function colorScheme(): ColorScheme {
  return document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light';
}

/* Host → child theme (bridge v1). One observer on <html> watches every way the
 * theme changes (data-theme, data-glass, a custom theme's inline tokens) and
 * posts the current tokens to each mounted frame, so a switch restyles frames
 * in place instead of leaving them on the palette they were built with. */
const themedFrames = new Map<HTMLIFrameElement, (() => Look | null) | undefined>();
const sentThemes = new WeakMap<HTMLIFrameElement, string>();
let themeObserver: MutationObserver | null = null;
let lastTheme = '';
let themeQueued = false;
let stopMedia: Array<() => void> = [];

function frameMotion() {
  const reducedMotion = motionReduced();
  const restricted = typeof window !== 'undefined' && window.matchMedia?.('(prefers-contrast: more), (prefers-reduced-transparency: reduce), (forced-colors: active)').matches;
  return { __hc: 1, kind: 'motion', reducedMotion, textureLevel: document.documentElement.dataset.textureLevel === 'quiet' ? 'quiet' : 'full', textureAllowed: !reducedMotion && !restricted && document.documentElement.dataset.textureLevel !== 'off' && document.documentElement.dataset.textureActive !== 'false' };
}
function postTheme(frame: HTMLIFrameElement, force = false) {
  try {
    const look = themedFrames.get(frame)?.();
    const theme = themeMessage(look?.scheme ?? colorScheme(), themePairs(look)), motion = frameMotion();
    const key = JSON.stringify([theme, motion]);
    if (!force && sentThemes.get(frame) === key) return;
    sentThemes.set(frame, key);
    frame.contentWindow?.postMessage(theme, '*');
    frame.contentWindow?.postMessage(motion, '*');
  } catch { /* frame gone */ }
}

function onThemeMutation() {
  if (themeQueued) return;
  themeQueued = true;
  requestAnimationFrame(() => {
    themeQueued = false;
    const next = JSON.stringify([themeMessage(colorScheme(), themePairs()), frameMotion()]);
    if (next === lastTheme) return; // unrelated inline-style change
    lastTheme = next;
    themedFrames.forEach((_, frame) => postTheme(frame));
  });
}

/** Keep `frame` on the live theme until the returned stop is called: post on
 *  every load (a document built earlier may carry an older palette) and on
 *  every theme change. */
export function watchFrameTheme(frame: HTMLIFrameElement, look?: () => Look | null): () => void {
  const onLoad = () => postTheme(frame, true);
  frame.addEventListener('load', onLoad);
  themedFrames.set(frame, look);
  const dispose = createRoot(dispose => { createEffect(() => postTheme(frame)); return dispose; });
  if (!themeObserver && typeof MutationObserver !== 'undefined') {
    lastTheme = JSON.stringify([themeMessage(colorScheme(), themePairs()), frameMotion()]);
    themeObserver = new MutationObserver(onThemeMutation);
    themeObserver.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme', 'data-glass', 'data-motion', 'data-text-scale', 'data-texture-active', 'data-texture-level', 'style'] });
    const reduceQuery = motionQuery();
    reduceQuery?.addEventListener('change', onThemeMutation);
    stopMedia.push(() => reduceQuery?.removeEventListener('change', onThemeMutation));
    for (const query of ['(prefers-contrast: more)', '(prefers-reduced-transparency: reduce)', '(forced-colors: active)']) {
      const media = window.matchMedia?.(query);
      media?.addEventListener('change', onThemeMutation);
      stopMedia.push(() => media?.removeEventListener('change', onThemeMutation));
    }
  }
  return () => {
    dispose();
    frame.removeEventListener('load', onLoad);
    themedFrames.delete(frame);
    if (!themedFrames.size && themeObserver) {
      themeObserver.disconnect(); themeObserver = null;
      stopMedia.forEach(stop => stop()); stopMedia = [];
    }
  };
}

/* The Canvas Kit's bytes, loaded once in their own chunk. A failed load is
 * not cached: the next artifact tries again, and this one renders without
 * the kit (its light-DOM fallback). The ask kit (C4) loads the same way, in
 * a chunk of its own, and only for ask-canvases. */
export const NO_KIT: CanvasKitAssets = { version: '', js: '', css: '' };

function lazyKit(load: () => Promise<CanvasKitAssets>): [() => Promise<CanvasKitAssets>, () => CanvasKitAssets | null] {
  let pending: Promise<CanvasKitAssets> | null = null;
  let loaded: CanvasKitAssets | null = null;
  return [
    () => (pending ??= load().then(k => (loaded = k), () => {
      pending = null;
      return NO_KIT;
    })),
    () => loaded,
  ];
}

/** `loadCanvasKit()` resolves to the kit; `loadedCanvasKit()` is the kit if
 *  it has already loaded, so later frames build synchronously. */
export const [loadCanvasKit, loadedCanvasKit] = lazyKit(() => import('../canvas/kitAssets').then(m => m.CANVAS_KIT));
/** The same for the ask kit (Canvas C4 A1). */
export const [loadCanvasKitAsk, loadedCanvasKitAsk] = lazyKit(() => import('../canvas/kitAskAssets').then(m => m.CANVAS_KIT_ASK));

/**
 * The `<head>` inner HTML of a sandboxed document, as pieces with nonce
 * slots: charset, the child CSP, DNS prefetching off, `<base target="_blank">`
 * (a link never replaces the frame's own document; web and mail clicks are
 * posted by the nav bridge and the host opens them, `openFrameLink` in
 * store/browser.ts), a `<style>` that always carries the `:root` theme vars
 * plus the caller's presentational CSS, then the Canvas Kit stylesheet, the
 * sandbox runtime script (height ping + nav bridge + error bridge) and the kit
 * script, then (an ask-canvas only) the ask kit's stylesheet and script. The
 * runtime and kits are embedded byte-for-byte (`./sandbox-doc.js`,
 * `kit.min.js`, `kit-ask.min.js`); in the script-free modes they carry the
 * document's nonce, the only script source those modes allow. Don't add
 * other inline scripts here. The runtime comes first: it removes WebRTC and
 * captures the host window before any other script.
 */
function headPieces(styles: string, mode: ArtifactMode, kit: CanvasKitAssets, ask: CanvasKitAssets, look?: Look | null): Piece[] {
  const open: Piece[] = modeUsesNonce(mode) ? ['<script nonce="', SLOT, '">'] : ['<script>'];
  const css = (k: CanvasKitAssets) => (k.css ? `<style>${k.css}</style>\n` : '');
  const js = (k: CanvasKitAssets): Piece[] => (k.js ? ['\n', ...open, k.js, '<\/script>'] : []);
  return [
    '<meta charset="utf-8">\n<meta http-equiv="Content-Security-Policy" content="', ...cspPieces(mode), `">
<meta http-equiv="x-dns-prefetch-control" content="off">
<base target="_blank">
<style>:root{color-scheme:light dark;${themeVars(look)}}
${styles}</style>
${css(kit)}${css(ask)}`,
    ...open, SANDBOX_DOC_JS, '<\/script>',
    ...js(kit), ...js(ask),
  ];
}

/** `headPieces` sealed with `nonce`, for a document built by hand. `ask`:
 *  the ask kit, for an ask-canvas. */
export function sandboxHead(styles: string, mode: ArtifactMode, kit: CanvasKitAssets, nonce: string, ask: CanvasKitAssets = NO_KIT): string {
  if (!NONCE_RE.test(nonce)) throw new Error('sandboxHead: not a nonce');
  return toParts(headPieces(styles, mode, kit, ask)).join(nonce);
}

/**
 * A whole sandboxed document as PARTS: one nonce goes between each pair, and
 * every slot (policy meta, runtime tag, kit tag, ask kit tag) comes before
 * `body`, the agent's HTML, which is a plain string inside the last part.
 * Canvas and Interactive documents are one part (no slot). Hand the parts to
 * `artifactDocUrl` (Rust fills the slots) or `sealDoc` (srcdoc). `ask` is the
 * ask kit, for an ask-canvas only.
 */
export function sandboxDocParts(styles: string, mode: ArtifactMode, kit: CanvasKitAssets, body: string, ask: CanvasKitAssets = NO_KIT, look?: Look | null): string[] {
  return toParts(['<!doctype html><html><head>', ...headPieces(styles, mode, kit, ask, look), '</head><body>', body, '</body></html>']);
}

/** The srcdoc fallback: the parts joined with a fresh nonce of their own. */
export function sealDoc(parts: string[], nonce: string = newNonce()): string {
  if (!NONCE_RE.test(nonce)) throw new Error('sealDoc: not a nonce');
  return parts.join(nonce);
}
