import { afterEach, describe, expect, it, vi } from 'vitest';

// The browser mock is dev-only (merged plan slice 6, hygiene): `?mock=1` must
// do nothing in a production build, and no shipped module may pull the
// fixtures in statically. `scripts/check-no-mock-bundle.mjs` checks the built
// bundle itself; this pins the two source-level halves of that guarantee.

async function loadMockMode(env: { DEV: boolean; MODE: string }, search: string) {
  vi.resetModules();
  vi.stubEnv('DEV', env.DEV);
  vi.stubEnv('MODE', env.MODE);
  vi.stubGlobal('window', { location: { search } });
  return import('./mockMode');
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe('browser mock mode', () => {
  it('is never on in a production build, whatever the URL says', async () => {
    const m = await loadMockMode({ DEV: false, MODE: 'production' }, '?mock=1');
    expect(m.MOCK_BUILD).toBe(false);
    expect(m.isBrowserMockMode()).toBe(false);
  });

  it('follows ?mock=1 on the dev server (the Playwright smoke)', async () => {
    const on = await loadMockMode({ DEV: true, MODE: 'development' }, '?mock=1');
    expect(on.isBrowserMockMode()).toBe(true);
    const off = await loadMockMode({ DEV: true, MODE: 'development' }, '');
    expect(off.isBrowserMockMode()).toBe(false);
  });

  it('follows ?mock=1 in a `--mode mock` build (static Playwright, perf budgets)', async () => {
    const m = await loadMockMode({ DEV: false, MODE: 'mock' }, '?mock=1&demo=large');
    expect(m.MOCK_BUILD).toBe(true);
    expect(m.isBrowserMockMode()).toBe(true);
  });
});

describe('mock fixtures stay out of the static import graph', () => {
  // Every source file under src/, as text (vite's glob; no node:fs here,
  // the frontend tsconfig has no Node types).
  const sources = import.meta.glob('./**/*.{ts,tsx}', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
  const isMockModule = (rel: string) => /(^|\/)mock[A-Z][^/]*\.tsx?$/.test(rel) && rel !== 'mockMode.ts';
  const isTest = (rel: string) => /\.test\.tsx?$/.test(rel);
  // `import … from './mockApi'` / `export … from '../mockDemo'`, not `import('./mockApi')`.
  const STATIC_MOCK_IMPORT = /^\s*(?:import|export)\s[^;]*?from\s+['"](?:\.{1,2}\/)+(?:[^'"]*\/)?mock(?!Mode['"])[A-Z][^'"]*['"]/m;

  it('no shipped module imports mockApi or its fixtures statically', () => {
    const files = Object.entries(sources).map(([key, text]) => ({ rel: key.replace(/^\.\//, ''), text }));
    expect(files.some(f => f.rel === 'api.ts')).toBe(true);
    const offenders = files
      .filter(({ rel }) => !isMockModule(rel) && !isTest(rel))
      .filter(({ text }) => STATIC_MOCK_IMPORT.test(text))
      .map(({ rel }) => rel);
    expect(offenders).toEqual([]);
  });

  it('the pattern catches the import api.ts used to have', () => {
    expect(STATIC_MOCK_IMPORT.test("import { isBrowserMockMode, mockInvoke } from './mockApi';")).toBe(true);
    expect(STATIC_MOCK_IMPORT.test("import { isBrowserMockMode } from '../../mockApi';")).toBe(true);
    expect(STATIC_MOCK_IMPORT.test("import { MOCK_BUILD, isBrowserMockMode } from './mockMode';")).toBe(false);
    expect(STATIC_MOCK_IMPORT.test("  ? <T>(c: string) => import('./mockApi').then(m => m.mockInvoke<T>(c))")).toBe(false);
  });
});
