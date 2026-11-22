import { expect, test } from './fixtures/test';
import { SCENARIOS } from '../../src/mockScenarios';

/**
 * Review F05/F06 (2026-10-13): every documented browser fixture mounts every
 * Settings section (Companion devices included) and opens the Main agent
 * without a page error. The automatic pageErrors fixture fails the test on any
 * uncaught exception or unhandled rejection.
 */
const FIXTURES = [
  { name: 'empty', url: '/?mock=1' },
  { name: 'populated demo', url: '/?mock=1&demo=1' },
  { name: 'large demo', url: '/?mock=1&demo=large' },
  // The named whole-app scenarios (src/mockScenarios.ts, docs/FIXTURES.md).
  ...Object.keys(SCENARIOS).map(s => ({ name: `scenario ${s}`, url: `/?mock=1&demo=${s === 'library-scale' ? 'large' : '1'}&scenario=${s}` })),
];
// 15 since P1-22 (D4): feedback, companions, learning, context and vaultsearch live inside others.
// 14 since run 15 P4: Other apps' conversations (`imports`) went with From other apps.
// 7 since S2: the other seven render whole as parts of these (all of a
// section's parts mount with it).
// 12 since Settings PR 3 (the old seven split by what a person looks for).
const SECTIONS = ['general', 'chat', 'models', 'accounts', 'personalization', 'connectors', 'sources', 'background', 'devices', 'usage', 'shortcuts', 'advanced'];

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'one width is enough for mount parity; narrow journeys live in the journey specs'); });

for (const f of FIXTURES) {
  test(`${f.name}: every Settings section mounts and unmounts without a page error`, async ({ page }) => {
    test.setTimeout(90_000);
    await page.goto(f.url);
    await page.getByRole('button', { name: /Settings/ }).first().click();
    const index = page.getByRole('navigation', { name: 'Settings sections' });
    const buttons = index.locator('.settings-index-item');
    await expect(buttons.first()).toBeVisible();
    const count = await buttons.count();
    expect(count, 'every section is listed').toBeGreaterThanOrEqual(SECTIONS.length);
    for (let i = 0; i < count; i++) {
      await buttons.nth(i).click();
      await expect(buttons.nth(i)).toHaveAttribute('aria-current', 'page');
    }
    // Leave Settings: every section's subscriptions are released.
    await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
    await page.waitForTimeout(200);
  });

  test(`${f.name}: the COO opens from the top bar`, async ({ page }) => {
    await page.goto(f.url);
    await page.locator('.top-bar').getByRole('button', { name: /^COO ·/ }).click();
    await expect(page.locator('.top-bar-crumbs')).toContainText('COO');
    await expect(page.getByLabel('Chat message')).toBeVisible();
  });
}
