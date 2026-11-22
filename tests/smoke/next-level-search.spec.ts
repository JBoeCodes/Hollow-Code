import { expect, test } from './fixtures/test';
import { openGlobalSearch } from './choose';

/**
 * Next-level P07 in the browser mock (src/mockDirectories.ts generates 2,804
 * files for the Launch project): the complete inventory, coverage counts, a
 * file after item 2,500 found by name and opened at its line, and palette
 * scopes. The Rust inventory itself is covered by cargo tests.
 */
test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop Library layout'); });

test('a project file past item 2,500 is listed, found by name and opened at its line', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Connected projects' }).click();
  const files = page.getByRole('region', { name: 'Connected project files' });
  await expect(files).toContainText(/2,804 files/);
  await expect(files).toContainText(/too large to read/);
  await files.getByLabel('Find connected file').fill('garage 14 lease');
  const hit = files.getByRole('button', { name: /garage-14-lease-renewal-terms/ }).first();
  await expect(hit).toBeVisible();
  await hit.click();
  await expect(page.getByRole('region', { name: 'Connected file reader' })).toContainText(/garage-14-lease-renewal-terms\.md · read only/);
});

test('palette scopes narrow results; Project files open in Library', async ({ page }) => {
  await page.goto('/?mock=1');
  await openGlobalSearch(page);
  const palette = page.locator('[aria-label="Command palette"]').first();
  await expect(palette).toBeVisible();
  await page.keyboard.type('garage 14');
  await expect(palette.locator('.palette-row').filter({ hasText: /garage-14-lease/ }).first()).toBeVisible();
  await palette.getByRole('group', { name: 'Search in' }).getByRole('button', { name: 'Notes' }).click();
  await expect(palette.locator('.palette-row').filter({ hasText: /garage-14-lease/ })).toHaveCount(0);
  await palette.getByRole('group', { name: 'Search in' }).getByRole('button', { name: 'Projects' }).click();
  const row = palette.locator('.palette-row').filter({ hasText: /garage-14-lease/ }).first();
  await expect(row).toBeVisible();
  await row.click();
  await expect(page.getByRole('region', { name: 'Connected file reader' })).toContainText(/garage-14-lease-renewal-terms/);
});
