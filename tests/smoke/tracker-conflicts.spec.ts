// The Tasks page's conflicts panel (design section 7: "Linear changed this;
// your edit was not applied", with Re-apply), against the browser mock's
// `conflicts` seed (`src/mockTracker.ts`). "Conflicts (n)" in the view bar
// shows while any issue has one and lists them; Re-apply is the issue
// detail's Use mine. Written in the 11-12 overnight (row T2) without
// running it; the lane's browser run covers it.
import { expect, test } from './fixtures/test';
import { openTasks, showTaskList } from './choose';

const views = (page: import('@playwright/test').Page) => page.getByRole('group', { name: 'Views' });

test('with no conflicts there is no Conflicts control', async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(views(page).getByRole('button', { name: 'All', exact: true })).toBeVisible();
  await expect(views(page).getByRole('button', { name: /^Conflicts/ })).toHaveCount(0);
});

test('Conflicts (n) lists the issues with a conflict, persists, and empties once each is settled', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockTracker', JSON.stringify({
    conflicts: { 'i-32': [{ field: 'priority', local: '1', remote: '3', at: '2026-11-11T09:00:00Z' }] },
  })));
  await page.reload();
  await openTasks(page);
  const control = views(page).getByRole('button', { name: 'Conflicts (1)' });
  await expect(control).toBeVisible();
  await expect(page.locator('.issue-row[data-issue-id="i-32"] .issue-flag[data-tone="warning"]')).toHaveAttribute('title', /re-apply/);
  await control.click();
  await expect(control).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.issue-row[data-issue-id]')).toHaveCount(1);
  await expect(page.locator('.issue-row[data-issue-id="i-32"]')).toBeVisible();
  // Local tasks aren't tracked, so they have no conflicts to list.
  await expect(page.locator('.issue-group[data-kind="local"]')).toHaveCount(0);
  // The view persists like the others.
  await page.reload();
  await openTasks(page);
  await expect(views(page).getByRole('button', { name: 'Conflicts (1)' })).toHaveAttribute('aria-pressed', 'true');
  // Re-apply lives in the issue detail.
  await page.locator('.issue-row[data-issue-id="i-32"] .issue-row-main').click();
  await page.locator('.issue-pane').getByRole('button', { name: 'Use mine' }).click();
  // Settled: the view stays chosen, says so and offers the way back.
  await expect(views(page).getByRole('button', { name: 'Conflicts (0)' })).toHaveAttribute('aria-pressed', 'true');
  await showTaskList(page);
  await expect(page.getByText('No conflicts')).toBeVisible();
  await page.getByRole('button', { name: 'Show all issues' }).click();
  await expect(views(page).getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(views(page).getByRole('button', { name: /^Conflicts/ })).toHaveCount(0);
});
