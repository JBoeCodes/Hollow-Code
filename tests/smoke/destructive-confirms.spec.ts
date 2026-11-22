import { test, expect, type Page } from './fixtures/test';
import { showSettingsPart } from './settingsPart';

// P0-13: one-click destructive or costed actions ask first, and Cancel
// sends nothing to the backend.
test.skip(({ viewport }) => (viewport?.width ?? 1000) <= 860, 'the desktop rail and Settings index');

const calls = (page: Page, name: string) => page.evaluate(n => window.__mockApiCalls?.[n] ?? 0, name);

async function openSettings(page: Page, section: RegExp) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: section }).click();
}

test('Clear all asks first, Cancel clears nothing, and an empty bell turns it off', async ({ page }) => {
  await openSettings(page, /^General/);
  const row = page.locator('.settings-row').filter({ hasText: 'Clear history' });
  const clear = row.getByRole('button', { name: 'Clear all', exact: true });
  await expect(clear).toBeEnabled();
  await clear.click();
  const confirm = page.getByRole('dialog', { name: 'Clear every notification?' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'clear_notifications')).toBe(0);
  await clear.click();
  await confirm.getByRole('button', { name: 'Clear all', exact: true }).click();
  await expect.poll(() => calls(page, 'clear_notifications')).toBe(1);
  await expect(clear).toBeDisabled();
});

test('Reset all to defaults asks first, and Cancel keeps the custom key', async ({ page }) => {
  await openSettings(page, /^Keyboard shortcuts/); // a section of its own since Settings PR 3
  await showSettingsPart(page, 'shortcuts');
  const task = page.locator('[data-shortcut="new-task"]');
  await task.getByRole('button', { name: 'Change the key for New task' }).click();
  await page.keyboard.press('Meta+Shift+y');
  await expect(task.locator('kbd')).toHaveText('⇧⌘Y');
  const before = await calls(page, 'set_shortcut_settings');
  await page.getByRole('button', { name: 'Reset all to defaults' }).click();
  const confirm = page.getByRole('dialog', { name: 'Reset every shortcut to its default?' });
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'set_shortcut_settings')).toBe(before);
  await expect(task.locator('kbd')).toHaveText('⇧⌘Y');
  await page.getByRole('button', { name: 'Reset all to defaults' }).click();
  await confirm.getByRole('button', { name: 'Reset all', exact: true }).click();
  await expect(task).toContainText('None');
});

test('Remove key asks first, and Cancel keeps the Jev key', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockJev', '{"enabled":true,"keyPresent":true}'));
  await openSettings(page, /^Accounts/); // a section of its own since Settings PR 3
  await showSettingsPart(page, 'accounts');
  const remove = page.locator('#settings-accounts').getByRole('button', { name: 'Remove key', exact: true });
  await remove.click();
  const confirm = page.getByRole('dialog', { name: 'Remove the Jev key?' });
  await expect(confirm).toContainText('Jev switches off');
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'remove_jev_key')).toBe(0);
  await expect(remove).toBeVisible();
  await remove.click();
  await confirm.getByRole('button', { name: 'Remove key', exact: true }).click();
  await expect.poll(() => calls(page, 'remove_jev_key')).toBe(1);
});

// Stop on a subagent asks first too: subagents-tab.spec.ts (COO rebuild L3).

test('an automation row’s Run now is named, asks first naming where it runs, and Cancel starts nothing', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
  const bolt = page.getByRole('button', { name: 'Run Morning Brief now', exact: true });
  await bolt.click();
  const confirm = page.getByRole('dialog', { name: 'Run “Morning Brief” now?' });
  await expect(confirm).toContainText('works in the vault');
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'run_workflow')).toBe(0);
  // The row menu's Run now asks the same way.
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Run now', exact: true }).click();
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => calls(page, 'run_workflow')).toBe(1);
});
