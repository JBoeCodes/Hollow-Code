import { expect, test } from './fixtures/test';
import { openGlobalSearch, reachSidebar, sidebarSearch } from './choose';

// P1-48: the palette answers its own ⌘K and ⌘J, offers the keymap as a
// button, says when conversation search failed, and runs vault search only
// in scopes that list it.
const palette = (page: import('@playwright/test').Page) => page.locator('[aria-label="Command palette"]').first();

test('⌘K inside the palette closes it and gives focus back; ⌘J switches it to Workspaces', async ({ page }) => {
  await page.goto('/?mock=1');
  // The sidebar's search entry, where the top bar's Search anything went (U1).
  const { target: opener } = await reachSidebar(page, sidebarSearch(page));
  await opener.focus();
  await page.keyboard.press('Meta+k');
  await expect(palette(page)).toBeVisible();
  await page.keyboard.press('Meta+k');
  await expect(palette(page)).toHaveCount(0);
  await expect(opener).toBeFocused();

  await page.keyboard.press('Meta+k');
  await expect(palette(page)).toBeVisible();
  await page.keyboard.press('Meta+j');
  await expect(palette(page)).toBeVisible();
  await expect(palette(page).getByRole('group', { name: 'Search in' }).getByRole('button', { name: 'Workspaces' })).toHaveAttribute('aria-pressed', 'true');
});

test('the footer offers Keyboard shortcuts as a button, not a key that can’t fire', async ({ page }) => {
  await page.goto('/?mock=1');
  await openGlobalSearch(page);
  const foot = palette(page).locator('.palette-foot');
  await expect(foot.locator('kbd', { hasText: '?' })).toHaveCount(0);
  await foot.getByRole('button', { name: 'Keyboard shortcuts' }).click();
  await expect(palette(page)).toHaveCount(0);
  await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeVisible();
});

test('a failed conversation search says so, and Projects makes no vault search', async ({ page }) => {
  await page.goto('/?mock=1');
  await openGlobalSearch(page);
  await page.evaluate(() => { (window as unknown as { __mockFailCommandOnce?: string }).__mockFailCommandOnce = 'search_sessions'; });
  await palette(page).getByRole('combobox').fill('zqxv');
  await expect(palette(page)).toContainText('Conversation search didn’t finish');

  await palette(page).getByRole('combobox').fill('');
  const before = await page.evaluate(() => window.__mockApiCalls?.vault_search ?? 0);
  await palette(page).getByRole('group', { name: 'Search in' }).getByRole('button', { name: 'Projects' }).click();
  await palette(page).getByRole('combobox').fill('garage lease');
  await expect(palette(page).locator('.palette-row').first()).toBeVisible();
  await page.waitForTimeout(400);
  expect(await page.evaluate(() => window.__mockApiCalls?.vault_search ?? 0)).toBe(before);
});
