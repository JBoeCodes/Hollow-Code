import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * P0-11 (SET2-01): opening Settings → Personalization (then Knowledge) used to start a metered
 * profile read every few days even with Keep learning off, while the screen
 * said "Only when you ask". With it off, opening the page reads nothing; with
 * it on, the first visit still reads on its own and says which agent reads.
 * The mock store starts never read; `mockProfileBuilderLearn` = 'off' starts
 * it with Keep learning off.
 */

const scans = (page: Page) => page.evaluate(() => window.__mockApiCalls?.profile_builder_scan ?? 0);

async function openAboutYou(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'personalization');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Personalization', exact: true }).click();
  }
}

test('with Keep learning off, opening About you reads nothing', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockProfileBuilderLearn', 'off'));
  await page.goto('/?mock=1');
  await openAboutYou(page);
  const meta = page.locator('.about-you-meta');
  await expect(meta).toContainText('Only when you ask');
  // The panel asks for a read right after its first load, so once the meta
  // line shows, a read would already have been asked for.
  await expect(meta).toContainText('Hasn’t looked yet');
  expect(await scans(page)).toBe(0);
  await expect(page.getByRole('button', { name: 'Learn about me' })).toBeEnabled();
});

test('with Keep learning on, the first visit reads and names the agent reading', async ({ page }) => {
  // Hold the read so the line shown while it runs can be checked.
  await page.addInitScript(() => {
    const w = window as unknown as { __releaseScan: () => void };
    window.__mockHoldOnce = { command: 'profile_builder_scan', release: new Promise<void>(r => { w.__releaseScan = () => r(); }) };
  });
  await page.goto('/?mock=1');
  await openAboutYou(page);
  const meta = page.locator('.about-you-meta');
  await expect(meta).toContainText('Reading now with Claude');
  await expect.poll(() => scans(page)).toBe(1);
  await page.evaluate(() => (window as unknown as { __releaseScan: () => void }).__releaseScan());
  await expect(meta).toContainText('Last looked just now');
  expect(await scans(page)).toBe(1);
});
