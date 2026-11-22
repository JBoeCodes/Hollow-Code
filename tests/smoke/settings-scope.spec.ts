/**
 * Settings say what they change (plan completion L4, 2026-10-13; review V05
 * "Settings scope", matrix X11/X12). Every section's header names what it
 * affects, when a change takes effect and how it saves, from the registry
 * (src/settingsSections.ts `scope`). Background work compares the four kinds of
 * background work (the explainer L2 wrote); Accounts and Connectors tell an
 * agent's sign-in, its connectors and Hollow's own sync apart, with a route
 * to each. Clicks only, against the demo mock.
 */
import { expect, test, type Page } from './fixtures/test';
import { SETTINGS_SECTION_GROUPS, saveNote } from '../../src/settingsSections';

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'At 390px Settings sections are a selector; the route ledger walks cover small windows');
});

async function settings(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  return page.getByRole('navigation', { name: 'Settings sections' });
}

test('every Settings section shows how it saves', async ({ page }) => {
  const nav = await settings(page);
  for (const section of SETTINGS_SECTION_GROUPS.flatMap(g => g.items)) {
    await nav.getByRole('button', { name: section.label, exact: true }).click();
    await expect(page.locator('.settings-section-scope')).toHaveCount(1);
    const line = page.locator('.settings-section-scope');
    await expect(line, section.id).toHaveAccessibleName(`What ${section.label} changes`);
    await expect(line, section.id).toHaveText(saveNote(section.scope));
    await expect(line.locator('details')).toHaveCount(0);
    await expect(page.getByText('About these settings', { exact: true })).toHaveCount(0);
  }
});

test('Settings → Background work no longer carries the comparison; it stays on Automations (Settings PR 6, C11)', async ({ page }) => {
  const nav = await settings(page);
  await nav.getByRole('button', { name: 'Background work', exact: true }).click();
  await expect(page.locator('#settings-background details.background-work')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Open Automations', exact: true })).toBeVisible();
});

test('Accounts and Connectors no longer open with the kinds-of-access explainer (Settings PR 6, C11)', async ({ page }) => {
  const nav = await settings(page);
  await nav.getByRole('button', { name: 'Accounts', exact: true }).click();
  await expect(page.locator('#settings-accounts details.settings-access-kinds')).toHaveCount(0);
  await nav.getByRole('button', { name: 'Connectors', exact: true }).click();
  await expect(page.locator('#settings-connectors details.settings-access-kinds')).toHaveCount(0);
  await expect(page.getByRole('table', { name: 'Service health by capability' })).toHaveCount(0);
});
