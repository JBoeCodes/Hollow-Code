import { fileURLToPath } from 'node:url';
import { defineConfig, devices } from '@playwright/test';

// Run 15 S7: the frontend responsiveness probe (`frontend.perf.spec.ts`), kept
// out of the smoke suite (its own testDir). It measures a production-mode
// build that keeps the mock (`vite build --mode mock`, as scripts/perf-*.mjs
// do), not the dev server: Vite's dev mode serves hundreds of unbundled
// modules and Solid's dev build, which would swamp what a change saves. The
// build goes to node_modules/.cache (ignored), never dist/, so it can't
// collide with a gate's build in the same checkout. One server, one worker:
//   PW_PORT=<free port> npx playwright test --config=tests/perf/playwright.perf.config.ts
// Frontend only (mock IPC, Chromium, no Rust, no WKWebView): compare builds
// with it, not with the app's own numbers.
const PORT = Number(process.env.PW_PORT ?? 1459);
const OUT = 'node_modules/.cache/hollow-perf-dist';

export default defineConfig({
  testDir: '.',
  testMatch: /\.perf\.spec\.ts$/,
  // Its own results folder: Playwright clears its outputDir at every run, and
  // the smoke suite's test-results/ may hold evidence someone still needs.
  outputDir: '../../node_modules/.cache/hollow-perf-results',
  timeout: 900_000,
  workers: 1,
  reporter: [['list']],
  use: { baseURL: `http://127.0.0.1:${PORT}` },
  webServer: {
    // Sourcemaps only name the files in PROFILE=1 output; nothing loads them.
    command: `npx vite build --mode mock --outDir ${OUT} --emptyOutDir --sourcemap --logLevel warn && npx vite preview --mode mock --outDir ${OUT} --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}/?mock=1`,
    // The checkout's root (a webServer runs in the config's folder by default).
    cwd: fileURLToPath(new URL('../..', import.meta.url)),
    reuseExistingServer: false,
    timeout: 300_000,
  },
  projects: [{ name: 'chromium-desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 900 } } }],
});
