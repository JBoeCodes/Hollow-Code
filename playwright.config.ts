import { defineConfig, devices } from '@playwright/test';

// The dev server port. Override with PW_PORT when more than one checkout is
// running the suite at once — several parallel worktrees each starting a dev
// server would otherwise collide on 1420, and because `reuseExistingServer` is
// true the loser silently ATTACHES to the winner's server and reports its
// results as its own. A private port plus reuseExistingServer:false is the only
// way to know whose code you just tested.
const PORT = Number(process.env.PW_PORT ?? 1420);
const OWN_PORT = PORT !== 1420;
/** Specs only the minimum-window projects run, and the click-only journeys. */
const MINIMUM_ONLY = /\.minimum\.spec\.ts$/;
const JOURNEYS = /\.journey\.spec\.ts$/;

export default defineConfig({
  testDir: './tests/smoke',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  // CI (.github/workflows/ci.yml `smoke`): a committed test.only fails the run
  // instead of quietly skipping the rest, and one retry absorbs a one-off
  // timing miss on a cold, 3-core runner (Vite compiles each module on first
  // request). A pass on retry is still reported as "flaky" in the log.
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [['list'], ['html', { open: 'never' }]],
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: {
    command: `npm run dev -- --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: `http://127.0.0.1:${PORT}/?mock=1`,
    reuseExistingServer: !OWN_PORT,
    timeout: 120_000,
  },
  projects: [
    {
      name: 'chromium-desktop',
      testIgnore: MINIMUM_ONLY,
      use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 920 } },
    },
    {
      name: 'chromium-narrow',
      testIgnore: [MINIMUM_ONLY, JOURNEYS],
      use: { ...devices['Desktop Chrome'], viewport: { width: 390, height: 844 } },
    },
    // Review F07 (2026-10-13): the native window's own minimum (tauri.conf.json
    // minWidth 720 / minHeight 480), and the same width at the Largest text
    // size. They run ONLY the required specs (*.minimum.spec.ts and the
    // click-only journeys), which may not skip: scripts/check-required-specs.mjs.
    {
      name: 'chromium-minimum',
      testMatch: [MINIMUM_ONLY, JOURNEYS],
      use: { ...devices['Desktop Chrome'], viewport: { width: 720, height: 480 } },
    },
    {
      name: 'chromium-minimum-large-text',
      testMatch: [MINIMUM_ONLY, JOURNEYS],
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 720, height: 480 },
        storageState: { cookies: [], origins: [{ origin: `http://127.0.0.1:${PORT}`, localStorage: [{ name: 'hollow.studio.textScale', value: '125' }] }] },
      },
    },
  ],
});
