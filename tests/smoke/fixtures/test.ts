/**
 * The smoke suite's `test` (review F05/F07, 2026-10-13): Playwright's, plus an
 * automatic `pageErrors` fixture. An uncaught exception or unhandled promise
 * rejection in the page fails the test that caused it, unless the test (or
 * the short list below) allows it by pattern with a reason. A broad mock
 * suite that passes while the page throws is not evidence of anything.
 *
 * Specs import from here instead of '@playwright/test'
 * (`scripts/check-smoke-imports.mjs` refuses the old import in a spec).
 */
import { test as base } from '@playwright/test';

export * from '@playwright/test';

export interface PageErrors {
  /** Allow page errors matching `pattern` in this test; say why. */
  allow(pattern: RegExp, reason: string): void;
  /** Errors seen so far (allowed ones included). */
  readonly seen: readonly string[];
}

/** Errors every test may produce, each with its reason. Keep this short. */
const ALWAYS_ALLOWED: { pattern: RegExp; reason: string }[] = [
  // Chromium's own benign notice when a ResizeObserver callback resizes what it observes.
  { pattern: /ResizeObserver loop (limit exceeded|completed with undelivered notifications)/, reason: 'browser notice, not an app error' },
];

/** A spec's own `page.addInitScript` runs in EVERY frame, sandboxed canvas
 *  frames included, where touching storage throws. Those errors come from the
 *  harness (the throwing frame is the anonymous init script), not from Hollow,
 *  so they are dropped; the same message from app code (a URL in its stack)
 *  still fails the test. */
function isHarnessNoise(e: Error): boolean {
  if (!/The document is sandboxed and lacks the 'allow-same-origin' flag/.test(e.message)) return false;
  const frames = (e.stack ?? '').split('\n').slice(1).map(l => l.trim()).filter(Boolean);
  // The THROWING frame is the anonymous init script (later frames can be an
  // async parent, such as the bridge's postMessage that woke the frame).
  return frames.length > 0 && /^at (\S+ \()?<anonymous>/.test(frames[0]);
}

export const test = base.extend<{ pageErrors: PageErrors }>({
  pageErrors: [async ({ page }, use, testInfo) => {
    // Specs that fake `window.__TAURI_INTERNALS__` to deliver events lack the
    // event plugin's unlisten half, so a component unmounting threw. Supply it.
    await page.addInitScript(() => {
      const w = window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown };
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ ??= { unregisterListener() { /* the spec's fake has no listeners to drop */ } };
    });
    const seen: string[] = [];
    const allowed = ALWAYS_ALLOWED.map(a => a.pattern);
    const where = new Map<string, string>();
    page.on('pageerror', e => {
      if (isHarnessNoise(e)) return;
      const line = `${e.name}: ${e.message}`;
      seen.push(line);
      if (!where.has(line)) where.set(line, (e.stack ?? '').split('\n').slice(1, 3).map(l => l.trim()).join(' ← '));
    });
    await use({ allow: (pattern: RegExp) => { allowed.push(pattern); }, get seen() { return seen; } });
    const unexpected = [...new Set(seen.filter(m => !allowed.some(re => re.test(m))))];
    if (unexpected.length && testInfo.status === testInfo.expectedStatus) {
      const lines = unexpected.map(m => `${m} (×${seen.filter(x => x === m).length})${where.get(m) ? `\n      at ${where.get(m)}` : ''}`);
      throw new Error(`Unexpected page error${unexpected.length === 1 ? '' : 's'} (uncaught exception or unhandled rejection):\n  ${lines.join('\n  ')}`);
    }
  }, { auto: true }],
});
