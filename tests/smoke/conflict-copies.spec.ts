// Conflict copies (overnight A10), against src/mockConflictCopies.ts: the
// Waiting on you item opens the dialog; the diff shows; Keep asks first and
// moves only the chosen pair; the palette opens the same dialog.
import { expect, test, type Page } from './fixtures/test';
import { openWaiting } from './waiting';

const resolved = (page: Page) => page.evaluate(() => (window as unknown as { __mockResolved?: { copy: string; keepCopy: boolean }[] }).__mockResolved ?? []);
const misses = (page: Page) => page.evaluate(() => (window as unknown as { __mockApiMisses?: string[] }).__mockApiMisses ?? []);

test('the Waiting on you item opens the dialog; Keep the copy asks first and resolves one pair', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify([
      { id: 'copies:vault', kind: 'copies', sourceId: 'vault', title: '2 conflict copies in the vault', detail: 'notes/Ideas (1).md (same as the original); notes/Plan 2.md', state: 'pending', at: null, workspaceId: null, sessionKey: null, path: null, category: null, link: null },
    ]));
  });
  await page.goto('/?mock=1');
  const panel = await openWaiting(page);
  const group = panel.getByRole('region', { name: 'Conflict copies' });
  await group.getByRole('button', { name: /2 conflict copies in the vault/ }).click();
  const dialog = page.getByRole('dialog', { name: 'Conflict copies' });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('.rbk-row.is-active .rbk-row-title')).toHaveText('notes/Ideas (1).md');
  await expect(dialog.locator('.rbk-row').first()).toContainText('same contents');

  await dialog.getByRole('button', { name: /Plan 2\.md/ }).click();
  await expect(dialog).toContainText('Write the release note');
  // Cancel moves nothing.
  await dialog.getByRole('button', { name: 'Keep the copy' }).click();
  await page.locator('.confirm-backdrop').getByRole('button', { name: 'Cancel' }).click();
  expect(await resolved(page)).toEqual([]);
  await dialog.getByRole('button', { name: 'Keep the copy' }).click();
  const confirm = page.locator('.confirm-backdrop');
  await expect(confirm.locator('.confirm-title')).toHaveText('Keep the copy?');
  await confirm.getByRole('button', { name: 'Keep the copy', exact: true }).click();
  await expect(dialog.locator('.rbk-row')).toHaveCount(1);
  // Said in the dialog: its backdrop covers the footer (overnight S5).
  await expect(dialog.getByRole('status')).toContainText("the other is in the vault's trash");
  expect(await resolved(page)).toEqual([{ copy: 'notes/Plan 2.md', keepCopy: true }]);
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  expect(await misses(page)).toEqual([]);
});

test('the palette opens the dialog, and Keep the original resolves the selected pair', async ({ page }) => {
  await page.goto('/?mock=1');
  // The shell is up: the top bar's global group (its New menu went in U1).
  await expect(page.getByRole('group', { name: 'Everywhere' })).toBeVisible();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Resolve conflict copies');
  await page.keyboard.press('Enter');
  const dialog = page.getByRole('dialog', { name: 'Conflict copies' });
  await expect(dialog.locator('.rbk-row')).toHaveCount(2);
  await dialog.getByRole('button', { name: 'Keep the original' }).click();
  await page.locator('.confirm-backdrop').getByRole('button', { name: 'Keep the original', exact: true }).click();
  await expect(dialog.locator('.rbk-row')).toHaveCount(1);
  expect(await resolved(page)).toEqual([{ copy: 'notes/Ideas (1).md', keepCopy: false }]);
  expect(await misses(page)).toEqual([]);
});
