/**
 * Whether this page runs on the browser mock (`mockApi.ts`) instead of Tauri.
 *
 * `?mock=1` alone is not enough (review hygiene finding, merged plan slice 6):
 * the URL flag is honoured only by the dev server (`npm run dev`, which the
 * Playwright smoke starts) or by a build made for it (`npm run build:mock`,
 * i.e. `vite build --mode mock`, for `playwright.static.config.ts` and
 * `scripts/perf-mock-budgets.mjs`). In a production build `MOCK_BUILD` is the
 * literal `false`, so every `MOCK_BUILD && …` branch, and the dynamic
 * `import('./mockApi')` behind it, is dropped from the bundle;
 * `scripts/check-no-mock-bundle.mjs` fails `npm run build` if it ever is not.
 *
 * This module must stay free of fixtures: everything that only needs to ask
 * "am I mocked?" imports it, never `mockApi.ts`.
 */
export const MOCK_BUILD: boolean = import.meta.env.DEV || import.meta.env.MODE === 'mock';

export function isBrowserMockMode(): boolean {
  if (!MOCK_BUILD) return false;
  try {
    return new URLSearchParams(window.location.search).get('mock') === '1';
  } catch {
    return false;
  }
}
