import { openTasks } from './choose';
import { expect, test } from './fixtures/test';

/**
 * Profiling night 11-20: the Tasks list draws a group's rows in steps. The
 * scale fixture (5,000 tasks) used to draw 4,550 rows at once (a 283 ms
 * switch); a group now draws 60 and "Show more" adds 60 more.
 */
test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop task layouts'); });

test('a large Tasks list draws a step per group, and Show more adds the next', async ({ page }) => {
  await page.goto('/?mock=1&demo=large');
  await openTasks(page);
  await expect(page.locator('.issue-row').first()).toBeVisible();
  const more = page.locator('.issue-more-btn').first();
  await expect(more).toBeVisible();
  const groups = await page.locator('.issue-group').count();
  const drawn = await page.locator('.issue-row').count();
  expect(drawn).toBeLessThanOrEqual(groups * 60);
  const group = page.locator('.issue-group').filter({ has: page.locator('.issue-more-btn') }).first();
  const before = await group.locator('.issue-row').count();
  await more.click();
  // Scrolling the button near the view adds a step too, so at least one step more.
  await expect.poll(() => group.locator('.issue-row').count()).toBeGreaterThanOrEqual(before + 60);
});

test('j moves through the rows and keeps the one it opens in the page', async ({ page }) => {
  await page.goto('/?mock=1&demo=large');
  await openTasks(page);
  const first = page.locator('.issue-row').first().locator('.issue-row-main');
  await first.click();
  for (let i = 0; i < 70; i++) await page.keyboard.press('j');
  await expect(page.locator('.issue-row[data-selected]')).toHaveCount(1);
  await expect(page.locator('.issue-row[data-selected] .issue-row-main')).toBeFocused();
});
