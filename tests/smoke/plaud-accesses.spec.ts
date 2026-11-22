import { expect, test, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

/**
 * P1-20 in the browser mock: the two Plaud accesses. Recordings' sign-in
 * button lands on the Recordings in Hollow sign-in itself, the Settings row
 * shows both statuses in one block, and Disconnect asks first.
 */
async function openConnectors(page: Page) {
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
}
const plaudRow = (page: Page) => page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: /^Recordings in Hollow$/ }) });

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop Settings layout'); });

test('signed out, Recordings’ button lands on the Recordings sign-in’s Connect', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockPlaudOff', '1'));
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Recordings');
  await page.getByRole('button', { name: 'Sign in to Plaud for Recordings' }).click();
  await expect(plaudRow(page).getByRole('button', { name: 'Connect', exact: true })).toBeFocused();
  await expect(plaudRow(page)).toContainText('not connected');
});

test("the Plaud row shows Hollow's own sign-in, and Disconnect asks first", async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await openConnectors(page);
  const row = plaudRow(page);
  // The agents' connector state is on its own row under Your connectors, once (Settings PR 7).
  await expect(row).not.toContainText('Plaud tools for agents');
  await expect(row.locator('.settings-row-v').first()).toHaveText(/^connected ·/);
  await row.getByRole('button', { name: 'Disconnect', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Recordings');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(row.getByRole('button', { name: 'Disconnect', exact: true })).toBeVisible();
  await row.getByRole('button', { name: 'Disconnect', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Sign out' }).click();
  await expect(row).toContainText('not connected');
  await expect(row.getByRole('button', { name: 'Connect', exact: true })).toBeVisible();
});
