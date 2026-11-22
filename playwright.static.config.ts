import { defineConfig, devices } from '@playwright/test';

// Browser checks against a static build made for the mock
// (`npm run build:mock`: a production build that still honours `?mock=1`;
// plain `npm run build` drops the mock, see src/mockMode.ts), then
// serve dist/ with any static file server, e.g.
// `python3 -m http.server 4810 --bind 127.0.0.1 --directory dist`).
// No dev server is started: use this when another checkout already holds the
// machine's one dev-server slot. Specs that rewrite dev-served modules
// (`page.route('**/src/…')`) need the dev-server config instead.
const PORT = Number(process.env.PW_STATIC_PORT ?? 4810);

export default defineConfig({
  testDir: './tests',
  testMatch: process.env.PW_STATIC_MATCH ? new RegExp(process.env.PW_STATIC_MATCH) : /(review|smoke)\/.*\.spec\.ts$/,
  timeout: 30_000,
  expect: { timeout: 5_000 },
  workers: 2,
  reporter: [['list']],
  use: { baseURL: `http://127.0.0.1:${PORT}`, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  // PW_STATIC_BROWSER=webkit runs the same specs in Playwright's WebKit: the
  // engine family of the native WKWebView, still not the native app.
  projects: [
    process.env.PW_STATIC_BROWSER === 'webkit'
      ? { name: 'webkit-desktop', use: { ...devices['Desktop Safari'], viewport: { width: 1440, height: 920 } } }
      : { name: 'chromium-desktop', use: { ...devices['Desktop Chrome'], viewport: { width: 1440, height: 920 } } },
    // PW_STATIC_NARROW=1 adds the phone-width project the dev-server config has.
    ...(process.env.PW_STATIC_NARROW ? [{ name: 'chromium-narrow', use: { ...devices['Desktop Chrome'], viewport: { width: 390, height: 844 } } }] : []),
  ],
});
