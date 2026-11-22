import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * P1-15 (polish plan), as kept after the issue-list rewrite (11-10): rows
 * keep j and k once one is open and focus follows the selection. The row
 * menu, the ⋯ buttons, Board columns and bulk selection are gone. Agenda,
 * Not this week and the Commitments screen were cut (overnight 11-08 S3).
 * Against the demo fixtures.
 */

async function openDemoTasks(page: Page, url = '/?mock=1&demo=1') {
  await page.goto(url);
  await openTasks(page);
  await expect(page.locator('.issue-row').first()).toBeVisible();
}
const titles = (page: Page) => page.locator('.task-collection .issue-row-main');

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop task layouts'); });

test('j and k work from a row title button, and focus follows the selection', async ({ page }) => {
  await openDemoTasks(page);
  await titles(page).nth(0).click();
  await expect(titles(page).nth(0)).toBeFocused();
  await page.keyboard.press('j');
  await expect(titles(page).nth(1)).toBeFocused();
  await expect(titles(page).nth(1)).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('k');
  await expect(titles(page).nth(0)).toBeFocused();
  await expect(titles(page).nth(0)).toHaveAttribute('aria-current', 'true');
});

test('Tasks has no Agenda week and no Not this week group', async ({ page }) => {
  await openDemoTasks(page);
  await expect(page.getByRole('group', { name: 'Agenda range' })).toHaveCount(0);
  await expect(page.locator('.task-backlog')).toHaveCount(0);
  await expect(page.locator('.agenda-week')).toHaveCount(0);
});

test('Tasks has no Commitments ledger', async ({ page }) => {
  await openDemoTasks(page, '/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Tasks and commitments' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Commitments', exact: true })).toHaveCount(0);
  await expect(page.locator('.cl-row')).toHaveCount(0);
});
