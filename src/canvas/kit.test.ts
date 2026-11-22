import { describe, it, expect, vi } from 'vitest';
vi.mock('../api', () => ({ api: {} }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (id: string) => `artifact://localhost/${id}` }));
import KIT_JS from './kit/generated/kit.min.js?raw';
import KIT_CSS from './kit/generated/kit.min.css?raw';
import INDEX_TS from './kit/index.ts?raw';
import GALLERY from './kit/gallery.html?raw';
import { KIT_COMPONENTS, KIT_TONES, KIT_VERSION } from './kit/manifest';
import { CANVAS_KIT } from './kitAssets';
import {
  KIT_SCRIPT_HASH, NO_KIT, SANDBOX_RUNTIME_HASH, THEME_TOKENS,
  childCsp, loadCanvasKit, loadedCanvasKit, sandboxHead, type ArtifactMode,
} from '../components/htmlSandbox';

const NONCE = 'AAAAAAAAAAAAAAAAAAAAAAAA';
const scriptSrc = (mode: ArtifactMode) => childCsp(mode, NONCE).split('; ').find(d => d.startsWith('script-src '));

describe('Hollow Canvas Kit v1 (Canvas C2)', () => {
  it('is kit@1 and the built bundle carries that version', () => {
    expect(KIT_VERSION).toBe('kit@1');
    expect(CANVAS_KIT).toEqual({ version: KIT_VERSION, js: KIT_JS, css: KIT_CSS });
    expect(KIT_JS).toContain('"kit@1"');
  });

  it('registers every manifest component, and the bundle defines each one', () => {
    expect([...KIT_COMPONENTS]).toEqual([
      'hc-badge', 'hc-callout', 'hc-details', 'hc-progress', 'hc-stat', 'hc-stats',
      'hc-steps', 'hc-bars', 'hc-chart', 'hc-table', 'hc-compare', 'hc-tabs', 'hc-file', 'hc-code', 'hc-flow', 'hc-gauge', 'hc-sparkline', 'hc-timeline', 'hc-heatmap', 'hc-checklist',
    ]);
    const registered = [...INDEX_TS.matchAll(/^\s+'(hc-[a-z-]+)': Hc\w+,$/gm)].map(m => m[1]);
    expect(registered).toEqual([...KIT_COMPONENTS]);
    for (const name of KIT_COMPONENTS) expect(KIT_JS).toContain(`"${name}"`);
    expect(KIT_JS).toContain('customElements.define');
  });

  it('the script hash is the sha256 of the inlined bytes, distinct from the runtime', async () => {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(KIT_JS)));
    expect(KIT_SCRIPT_HASH).toBe(`'sha256-${btoa(String.fromCharCode(...digest))}'`);
    expect(KIT_SCRIPT_HASH).not.toBe(SANDBOX_RUNTIME_HASH);
  });

  it('runs by the document nonce in the script-free modes and by unsafe-inline (no nonce) in scripted ones', () => {
    for (const mode of ['offline', 'images', 'https'] as const) {
      expect(scriptSrc(mode), mode).toBe(`script-src 'nonce-${NONCE}'`);
    }
    expect(scriptSrc('canvas')).toBe("script-src 'unsafe-inline'");
    expect(scriptSrc('interactive')).toBe("script-src 'unsafe-inline' https:");
  });

  it('every artifact document inlines the kit: styles after the base, script after the runtime', () => {
    const head = sandboxHead('body{margin:0}', 'offline', CANVAS_KIT, NONCE);
    const base = head.indexOf('body{margin:0}');
    const css = head.indexOf(`<style>${KIT_CSS}</style>`);
    const runtime = head.indexOf(`<script nonce="${NONCE}">/**`);
    const js = head.indexOf(`<script nonce="${NONCE}">${KIT_JS}</script>`);
    expect(head.indexOf('Content-Security-Policy')).toBeLessThan(base);
    expect(base).toBeGreaterThan(-1);
    expect(css).toBeGreaterThan(base);
    expect(runtime).toBeGreaterThan(css);
    expect(js).toBeGreaterThan(runtime);
    // A document built without the kit carries no empty tags.
    const bare = sandboxHead('', 'offline', NO_KIT, NONCE);
    expect(bare.match(/<script nonce=/g)).toHaveLength(1);
    expect(bare.match(/<style>/g)).toHaveLength(1);
  });

  it('loads lazily, once, and is then available synchronously', async () => {
    const first = await loadCanvasKit();
    expect(first).toBe(CANVAS_KIT);
    expect(await loadCanvasKit()).toBe(first);
    expect(loadedCanvasKit()).toBe(CANVAS_KIT);
  });

  it('reads only tokens the host sends, each with a fallback', () => {
    const sent = new Set(THEME_TOKENS.map(([name]) => name));
    const reads = [...KIT_CSS.matchAll(/var\((--app-[a-z0-9-]+)(,?)/g)];
    expect(reads.length).toBeGreaterThan(10);
    for (const [, name, comma] of reads) {
      expect(sent.has(name), name).toBe(true);
      expect(comma, `${name} needs a fallback`).toBe(',');
    }
  });

  it('never conveys a tone by colour alone, respects reduced motion, and keeps a fallback', () => {
    for (const [tone, { icon, word }] of Object.entries(KIT_TONES)) {
      expect(icon.length, tone).toBeGreaterThan(0);
      expect(word.length, tone).toBeGreaterThan(0);
      if (tone !== 'neutral') expect(KIT_CSS).toContain(`hc-badge[tone=${tone} i]:before{content:`);
    }
    expect(KIT_CSS).toMatch(/@media \(prefers-reduced-motion: ?reduce\)\{[^}]*transition:none/);
    expect(KIT_CSS).toContain('.hc-details-toggle:focus-visible{outline:');
    for (const name of ['hc-stat', 'hc-progress', 'hc-callout', 'hc-details']) {
      expect(KIT_CSS, name).toContain(`${name}:not([data-hc-ready])`);
    }
  });
});

describe('kit gallery (Canvas C2 K3)', () => {
  // One source for the SmokeHarness gallery and its smoke spec.
  it('uses every kit@1 component, every tone, and one chart of each type', () => {
    const used = new Set([...GALLERY.matchAll(/<(hc-[a-z]+)/g)].map(m => m[1]));
    expect([...used].sort()).toEqual([...KIT_COMPONENTS].sort());
    const charts = [...GALLERY.matchAll(/<hc-chart\b[^>]*\btype="([a-z]+)"/g)].map(m => m[1]);
    expect(charts.sort()).toEqual(['area', 'bar', 'donut', 'line']);
    for (const t of Object.keys(KIT_TONES).filter(t => t !== 'neutral')) {
      expect(GALLERY, t).toContain(`<hc-badge tone="${t}">`);
      expect(GALLERY, t).toContain(`<hc-callout tone="${t}"`);
    }
  });

  it('is markup only: no script, no URL, nothing that loads', () => {
    expect(GALLERY).not.toMatch(/<script|\son[a-z]+=|https?:|src=|href=|url\(|@import/i);
  });
});
