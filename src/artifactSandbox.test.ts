import { describe, it, expect, vi } from 'vitest';
import GOLDEN from './components/artifactCsp.golden.json?raw';
import RUNTIME_JS from './components/sandbox-doc.js?raw';
vi.mock('./api', () => ({ api: {} }));
vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (id: string) => `artifact://localhost/${id}` }));
import TOKENS_CSS from './styles/tokens.css?raw';
import {
  ARTIFACT_MODES, DEFAULT_ARTIFACT_MODE, KIT_ASK_SCRIPT_HASH, KIT_SCRIPT_HASH, NO_KIT, SANDBOX, SANDBOX_RUNTIME_HASH, THEME_TOKENS,
  childCsp, modeRunsScripts, modeUsesNonce, newNonce, sandboxDocParts, sandboxHead, sealDoc, srcdocMode, themePairs,
  type ArtifactMode, watchFrameTheme,
} from './components/htmlSandbox';
import { LIMITS, isSafeThemeValue, themeMessage } from './canvasBridge';

const golden = JSON.parse(GOLDEN) as Record<string, string>;
const MODES = Object.keys(golden).filter(k => !k.startsWith('_')) as ArtifactMode[];
const directive = (csp: string, name: string) => csp.split('; ').find(d => d.startsWith(`${name} `));
/** A well-formed nonce for tests that only need one. */
const NONCE = 'AAAAAAAAAAAAAAAAAAAAAAAA';

it('frames carry the live app text scale even with a named look', () => {
  expect(THEME_TOKENS).toContainEqual(['--app-text-scale', '--text-scale', '1']);
  vi.stubGlobal('document', { documentElement: { dataset: { textScale: '125' } } });
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (name: string) => name === '--text-scale' ? '1.25' : '' }));
  try {
    expect(new Map(themePairs()).get('--app-text-scale')).toBe('1.25');
    const look = { scheme: 'light' as const, pairs: [['--app-text-scale', '1'], ['--app-canvas', '#ffffff']] as Array<[string, string]>, adjusted: [] };
    expect(new Map(themePairs(look)).get('--app-text-scale')).toBe('1.25');
  } finally { vi.unstubAllGlobals(); }
});

it('watched frames receive their own look on load; other frames keep the app palette', () => {
  const callbacks: Array<() => void> = [], messages: unknown[][] = [[], []];
  vi.stubGlobal('document', { documentElement: { dataset: { theme: 'dark' } } });
  vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '#123456' }));
  const frames = messages.map((sent, i) => ({
    contentWindow: { postMessage: (message: unknown) => sent.push(message) },
    addEventListener: (_: string, cb: () => void) => { callbacks[i] = cb; },
    removeEventListener: vi.fn(),
  }) as unknown as HTMLIFrameElement);
  const look = { scheme: 'light' as const, pairs: [['--app-canvas', '#ffffff']] as Array<[string, string]>, adjusted: [] };
  const stops = [watchFrameTheme(frames[0], () => look), watchFrameTheme(frames[1])];
  callbacks.forEach(cb => cb());
  expect(messages[0]).toContainEqual(themeMessage('light', look.pairs));
  expect(messages[1]).toContainEqual(themeMessage('dark', themePairs()));
  stops.forEach(stop => stop());
  vi.unstubAllGlobals();
});

describe('artifact sandbox modes (Canvas C1)', () => {
  it('forwards the shared host texture veto without reducing functional motion', () => {
    const sent: unknown[] = [];
    const dataset: Record<string,string> = { theme: 'light', motion: 'full', textureActive: 'false' };
    let load = () => {};
    const frame = { contentWindow: { postMessage: (v: unknown) => sent.push(v) }, addEventListener: (_: string, cb: () => void) => { load = cb; }, removeEventListener() {} } as unknown as HTMLIFrameElement;
    vi.stubGlobal('document', { documentElement: { dataset } });
    vi.stubGlobal('window', { matchMedia: () => ({ matches: false }) });
    vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '' }));
    const stop = watchFrameTheme(frame);
    try {
      load();
      expect(sent.at(-1)).toEqual({ __hc: 1, kind: 'motion', reducedMotion: false, textureLevel: 'full', textureAllowed: false });
      dataset.textureActive = 'true'; load();
      expect(sent.at(-1)).toEqual({ __hc: 1, kind: 'motion', reducedMotion: false, textureLevel: 'full', textureAllowed: true });
      dataset.textureLevel = 'quiet'; load();
      expect(sent.at(-1)).toEqual({ __hc: 1, kind: 'motion', reducedMotion: false, textureLevel: 'quiet', textureAllowed: true });
      delete dataset.textureActive;
      dataset.textureLevel = 'off'; load();
      expect(sent.at(-1)).toEqual({ __hc: 1, kind: 'motion', reducedMotion: false, textureLevel: 'full', textureAllowed: false });
    } finally { stop(); vi.unstubAllGlobals(); }
  });

  it('frame motion follows system changes and removes its media listener', () => {
    const sent: unknown[] = [], listeners = new Set<() => void>();
    const media = { matches: true, addEventListener: (_: string, cb: () => void) => listeners.add(cb), removeEventListener: (_: string, cb: () => void) => listeners.delete(cb) };
    let loaded = () => {};
    const frame = { contentWindow: { postMessage: (v: unknown) => sent.push(v) }, addEventListener: (_: string, cb: () => void) => { loaded = cb; }, removeEventListener: vi.fn() } as unknown as HTMLIFrameElement;
    vi.stubGlobal('document', { documentElement: { dataset: { theme: 'light', motion: 'full' } } });
    vi.stubGlobal('window', { matchMedia: (q: string) => q.includes('motion') ? media : { matches: false, addEventListener() {}, removeEventListener() {} } });
    vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: () => '' }));
    vi.stubGlobal('MutationObserver', class { observe() {} disconnect() {} });
    vi.stubGlobal('requestAnimationFrame', (cb: () => void) => { cb(); return 0; });
    const stop = watchFrameTheme(frame);
    try {
      loaded();
      expect(sent).toContainEqual({ __hc: 1, kind: 'motion', reducedMotion: true, textureLevel: 'full', textureAllowed: false });
      expect(listeners.size).toBe(1);
      media.matches = false; listeners.forEach(cb => cb());
      expect(sent.at(-1)).toEqual({ __hc: 1, kind: 'motion', reducedMotion: false, textureLevel: 'full', textureAllowed: true });
    } finally { stop(); vi.unstubAllGlobals(); }
    expect(listeners.size).toBe(0);
  });

  it('every mode emits the policy artifacts.rs serves', () => {
    expect(MODES.sort()).toEqual([...ARTIFACT_MODES].sort());
    for (const mode of MODES) {
      expect(childCsp(mode, NONCE), mode).toBe(golden[mode].replace('NONCE', NONCE));
    }
  });

  it('the runtime hash is the sha256 of the bytes the frame inlines', async () => {
    const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(RUNTIME_JS)));
    expect(SANDBOX_RUNTIME_HASH).toBe(`'sha256-${btoa(String.fromCharCode(...digest))}'`);
  });

  it('canvas runs scripts with no network, and lists no hash that would disable them', () => {
    const csp = childCsp('canvas', NONCE);
    expect(directive(csp, 'script-src')).toBe("script-src 'unsafe-inline'");
    expect(csp).not.toMatch(/sha256-|nonce-|https?:|\*|unsafe-eval/);
    for (const d of ['connect-src', 'worker-src', 'frame-src', 'object-src', 'form-action']) {
      expect(directive(csp, d)).toBe(`${d} 'none'`);
    }
  });

  it('the frame gets no popups, forms or same-origin, and Offline stays the default', () => {
    expect(SANDBOX.split(/\s+/)).toEqual(['allow-scripts']);
    expect(DEFAULT_ARTIFACT_MODE).toBe('offline');
  });

  it('only canvas and interactive run artifact scripts', () => {
    expect(MODES.filter(modeRunsScripts).sort()).toEqual(['canvas', 'interactive']);
  });

  it('outside Tauri a canvas document falls back to Offline', () => {
    expect(srcdocMode('canvas')).toBe('offline');
    for (const mode of MODES.filter(m => m !== 'canvas')) expect(srcdocMode(mode)).toBe(mode);
  });

  it('every document turns DNS prefetching off', () => {
    const head = sandboxHead('', 'canvas', NO_KIT, NONCE);
    expect(head).toContain('<meta http-equiv="x-dns-prefetch-control" content="off">');
    // The policy meta comes first, before anything the document could load.
    expect(head.indexOf('Content-Security-Policy')).toBeLessThan(head.indexOf('<style>'));
  });
});

// SRI repair (2026-10-05): under CSP3 a hash-source also allows an EXTERNAL
// script whose integrity attribute carries that hash, and browsers fetch it
// before the integrity check fails. The runtime and kit hashes are public, so
// no artifact policy may list one: the host scripts run by a per-document
// nonce that the agent's HTML can't know.
describe('artifact script policy: a nonce, never a hash (SRI repair)', () => {
  const BODY = '<p>agent</p>';
  const KIT = { version: 'kit@1', js: '/* kit */', css: '.k{}' };

  it('no mode lists a hash-source an external script could match', () => {
    for (const mode of MODES) {
      const csp = childCsp(mode, NONCE);
      expect(csp, mode).not.toMatch(/'sha(256|384|512)-/);
      expect(csp, mode).not.toContain(SANDBOX_RUNTIME_HASH);
      expect(csp, mode).not.toContain(KIT_SCRIPT_HASH);
      expect(csp, mode).not.toContain(KIT_ASK_SCRIPT_HASH);
    }
  });

  it('Offline, Images and HTTPS allow only the nonce; Canvas and Interactive list none', () => {
    for (const mode of MODES) {
      const src = directive(childCsp(mode, NONCE), 'script-src');
      if (modeUsesNonce(mode)) expect(src, mode).toBe(`script-src 'nonce-${NONCE}'`);
      else expect(src, mode).not.toMatch(/nonce-|sha256-/);
    }
    expect(MODES.filter(modeUsesNonce).sort()).toEqual(['https', 'images', 'offline']);
  });

  it('a fresh nonce is at least 128 bits of base64 and never repeats', () => {
    const seen = new Set<string>();
    for (let i = 0; i < 64; i++) {
      const n = newNonce();
      expect(n).toMatch(/^[A-Za-z0-9+/]{22,}$/);
      seen.add(n);
    }
    expect(seen.size).toBe(64);
    expect(() => childCsp('offline', "x' https:")).toThrow();
    expect(() => childCsp('offline', '')).toThrow();
  });

  it('the host leaves nonce slots in the policy meta and on the runtime and kit tags, all before the body', () => {
    for (const mode of MODES) {
      const parts = sandboxDocParts('body{}', mode, KIT, BODY);
      const doc = parts.join(NONCE);
      if (!modeUsesNonce(mode)) {
        expect(parts, mode).toHaveLength(1);
        expect(doc, mode).not.toContain('nonce');
        continue;
      }
      // meta, runtime, kit: three slots, four parts; no kit script, two slots.
      expect(parts, mode).toHaveLength(4);
      expect(sandboxDocParts('', mode, { ...KIT, js: '' }, BODY)).toHaveLength(3);
      expect(doc).toContain(`script-src 'nonce-${NONCE}'`);
      expect(doc).toContain(`<script nonce="${NONCE}">/**`);
      expect(doc).toContain(`<script nonce="${NONCE}">/* kit */</script>`);
      expect(parts.at(-1)!.endsWith(`<body>${BODY}</body></html>`)).toBe(true);
      expect(parts.slice(0, -1).join('')).not.toContain(BODY);
      // An ask-canvas (C4 A1) adds the ask kit's tag: four slots, five parts,
      // still all before the body (artifacts.rs accepts two to four).
      const ask = sandboxDocParts('', mode, KIT, BODY, { version: 'kit@1', js: '/* ask */', css: '.a{}' });
      expect(ask, mode).toHaveLength(5);
      expect(ask.join(NONCE)).toContain(`<script nonce="${NONCE}">/* kit */</script>\n<script nonce="${NONCE}">/* ask */</script>`);
      expect(ask.slice(0, -1).join('')).not.toContain(BODY);
    }
  });

  it('agent text is never a slot: placeholder-looking markup stays as written', () => {
    const agent = `NONCE __NONCE__ {nonce} \${nonce} <script nonce="">x</script><script nonce="NONCE" src="https://x.invalid/a.js"></script>`;
    const parts = sandboxDocParts('', 'offline', KIT, agent);
    expect(parts.at(-1)!.endsWith(`<body>${agent}</body></html>`)).toBe(true);
    const doc = sealDoc(parts);
    const nonce = doc.match(/script-src 'nonce-([^']+)'/)![1];
    expect(doc.split(nonce)).toHaveLength(4);
    expect(doc.endsWith(`<body>${agent}</body></html>`)).toBe(true);
    // Every seal draws its own nonce.
    expect(sealDoc(parts).match(/script-src 'nonce-([^']+)'/)![1]).not.toBe(nonce);
  });
});

describe('theme tokens a frame sees (Canvas C2)', () => {
  const names = THEME_TOKENS.map(([name]) => name);

  it('fit the bridge: at most LIMITS.themeVars unique --app-* names with safe fallbacks', () => {
    expect(names.length).toBeLessThanOrEqual(LIMITS.themeVars);
    expect(new Set(names).size).toBe(names.length);
    for (const [name, , fallback] of THEME_TOKENS) expect(isSafeThemeValue(fallback), name).toBe(true);
    const sent = themeMessage('light', THEME_TOKENS.map(([name, , fallback]) => [name, fallback] as [string, string]));
    expect(Object.keys(sent.vars)).toEqual(names);
  });

  it('pass the frame runtime\'s own name and count checks', () => {
    const rule = RUNTIME_JS.match(/var THEME_VAR = \/(.+)\/;/);
    expect(rule).not.toBeNull();
    const runtimeName = new RegExp(rule![1]);
    for (const name of names) expect(runtimeName.test(name), name).toBe(true);
    expect(RUNTIME_JS).toContain(`names.length > ${LIMITS.themeVars}`);
  });

  it('keep the C1 names and cover the full colour set in tokens.css', () => {
    expect(names.slice(0, 9)).toEqual([
      '--app-ink', '--app-canvas', '--app-surface', '--app-primary', '--app-accent',
      '--app-muted', '--app-hairline', '--app-mono', '--app-display',
    ]);
    const dark = TOKENS_CSS.slice(TOKENS_CSS.indexOf(':root[data-theme="dark"] {'));
    const block = dark.slice(0, dark.indexOf('\n}'));
    const colours = [...block.matchAll(/^\s*(--[a-z0-9-]+)\s*:/gm)].map(m => m[1])
      // Elevation and numeric host-only opacity are not frame colors;
      // accent-amber is the existing alias of warning.
      .filter(t => !t.startsWith('--elev-') && t !== '--ambient-rail-opacity' && t !== '--accent-amber');
    expect(colours.length).toBeGreaterThan(30);
    const sources = new Set(THEME_TOKENS.map(([, token]) => token));
    for (const token of colours) expect(sources.has(token), token).toBe(true);
    for (const role of ['success', 'warning', 'danger', 'info']) {
      expect(names).toContain(`--app-${role}`);
      expect(names).toContain(`--app-${role}-soft`);
    }
  });

  it('read live values from <html>, so custom and dark palettes flow through', () => {
    const live: Record<string, string> = { '--ink': ' #eeeeee ', '--success': '#00aa00', '--error-soft': 'rgba(1, 2, 3, 0.5)' };
    vi.stubGlobal('document', { documentElement: {} });
    vi.stubGlobal('getComputedStyle', () => ({ getPropertyValue: (n: string) => live[n] ?? '' }));
    try {
      const pairs = new Map(themePairs());
      expect(pairs.get('--app-ink')).toBe('#eeeeee');
      expect(pairs.get('--app-success')).toBe('#00aa00');
      expect(pairs.get('--app-danger-soft')).toBe('rgba(1, 2, 3, 0.5)');
      expect(pairs.get('--app-info')).toBe('#5f6b7a'); // fallback when the app has none
      expect(pairs.size).toBe(THEME_TOKENS.length);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
