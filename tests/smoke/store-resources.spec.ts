import { expect, test, type Page } from './fixtures/test';

/**
 * Shared-resource regressions: work the app used to do more than once, or
 * throw away and redo.
 *
 * Every assertion here reads `window.__mockApiCalls` — the per-command invoke
 * counter in `src/mockApi.ts`. "This ran once, not twice" and "this survived a
 * navigation" are invisible in the DOM: the second fetch returns the same
 * fixture, so the page looks identical either way. The counter is the only
 * witness.
 */

const APP_URL = process.env.HOLLOW_SMOKE_URL || '/?mock=1';

async function callCount(page: Page, command: string): Promise<number> {
  return page.evaluate(
    (cmd) => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.[cmd] ?? 0,
    command,
  );
}

/** Reach the Observatory. It is off the rail, so the Vault header owns the
 *  in-context entry (the palette is the other one). */
async function openGraph(page: Page, opts: { settle?: boolean } = {}) {
  await page.getByLabel('Main navigation')
    .getByRole('button', { name: /Vault/ }).click();
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Graph/ }).click();
  await expect(page.locator('.graph-page')).toBeVisible();
  // Wait for the force layout to park (the HUD's live dot goes off) so node
  // positions are comparable rather than mid-flight.
  if (opts.settle !== false) {
    await expect(page.locator('.graph-live-dot')).not.toHaveClass(/\bon\b/, { timeout: 15_000 });
  }
}

/** Every node's world position, keyed by vault path. */
async function nodePositions(page: Page): Promise<Record<string, [number, number]>> {
  return page.evaluate(() => {
    const out: Record<string, [number, number]> = {};
    for (const el of Array.from(document.querySelectorAll('.graph-node[data-id]'))) {
      const m = /translate\(([-\d.]+) ([-\d.]+)\)/.exec(el.getAttribute('transform') ?? '');
      if (m) out[el.getAttribute('data-id')!] = [Number(m[1]), Number(m[2])];
    }
    return out;
  });
}

test.describe('shared store resources', () => {
  test.beforeEach(async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 920 });
  });

});
