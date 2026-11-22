import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Cua CU7: Settings → Connectors → Built into Hollow → Computer control →
 * Set up. The sheet holds the Basic verbs, Sandbox Mac and This Mac's screen
 * switches, Stop all and the Doctor's checks. Mock only (`computer_status`
 * and `computer_stop_all` in src/mockApi.ts).
 */

const narrow = (page: Page) => (page.viewportSize()?.width ?? 1000) <= 860;
const sheet = (page: Page) => page.locator('#settings-sheet-computer');

async function openSheet(page: Page) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.locator('.settings-index').getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('button', { name: 'Set up computer control' }).click();
  await expect(sheet(page)).toBeVisible();
}

test('the sheet has a switch for each target, and the defaults are the design\'s', async ({ page }) => {
  await openSheet(page);
  const basic = sheet(page).getByRole('switch', { name: 'Computer control basic verbs' });
  const sandbox = sheet(page).getByRole('switch', { name: 'Sandbox Mac' });
  const thisMac = sheet(page).getByRole('switch', { name: 'This Mac’s screen' });
  await expect(basic).toHaveAttribute('aria-checked', 'true');
  await expect(sandbox).toHaveAttribute('aria-checked', 'true');
  await expect(thisMac).toHaveAttribute('aria-checked', 'false');
  // Each flips and the choice is remembered.
  await thisMac.click();
  await expect(thisMac).toHaveAttribute('aria-checked', 'true');
  await sandbox.click();
  await expect(sandbox).toHaveAttribute('aria-checked', 'false');
  expect(await page.evaluate(() => [localStorage.getItem('computerThisMacEnabled'), localStorage.getItem('computerSandboxEnabled')])).toEqual(['true', 'false']);
  await basic.click();
  await expect(basic).toHaveAttribute('aria-checked', 'false');
});

test('the apps list starts blocked and one click allows an app, and another blocks it again', async ({ page }) => {
  await openSheet(page);
  const list = sheet(page).getByRole('list', { name: 'Apps agents can’t use on this Mac' });
  await expect(list.getByRole('listitem')).toHaveCount(18);
  await expect(list.locator('li[data-allowed="true"]')).toHaveCount(0);
  await list.getByRole('button', { name: 'Allow Terminal on this Mac' }).click();
  await expect(list.locator('li[data-allowed="true"]')).toContainText('Terminal');
  await list.getByRole('button', { name: 'Block Terminal on this Mac' }).click();
  await expect(list.locator('li[data-allowed="true"]')).toHaveCount(0);
});

test('Stop all answers, the Doctor lists its checks with a fix, and Esc returns focus', async ({ page }) => {
  await openSheet(page);
  await sheet(page).getByRole('button', { name: 'Stop all computer control' }).click();
  await expect(sheet(page).getByRole('status')).toContainText('Nothing was running');
  const checks = sheet(page).getByRole('list', { name: 'Computer control checks' });
  await expect(checks.getByRole('listitem')).toHaveCount(5);
  await expect(checks.locator('li[data-state="fail"]')).toContainText('Not set up');
  await expect(checks.locator('li[data-state="unknown"]')).toContainText('Not checked');
  await expect(sheet(page).getByRole('button', { name: /Copy the fix for Cua Driver/ })).toBeVisible();
  const results = await new AxeBuilder({ page }).include('#settings-sheet-computer').analyze();
  expect(results.violations).toEqual([]);
  await page.keyboard.press('Escape');
  await expect(sheet(page)).toBeHidden();
  await expect(page.getByRole('button', { name: 'Set up computer control' })).toBeFocused();
});
