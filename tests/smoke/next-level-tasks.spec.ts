import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Next-level P09/P10 against the demo fixtures, as kept after the issue-list
 * rewrite (11-10): "Work on this" through an explicit plan link, and no
 * Agenda on Tasks. Bulk edits, the row action menu and board drag went with
 * the old page.
 */

async function openDemoTasks(page: Page, opts: { calendarOff?: boolean } = {}) {
  if (opts.calendarOff) await page.addInitScript(() => localStorage.setItem('mockCalendarOff', '1'));
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await expect(page.locator('.issue-row').first()).toBeVisible();
}

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop task layouts'); });

test('Tasks has no Agenda, so a task is not dragged onto a week day', async ({ page }) => {
  await openDemoTasks(page);
  await expect(page.getByRole('group', { name: 'Agenda range' })).toHaveCount(0);
  await expect(page.locator('.agenda-week-day')).toHaveCount(0);
  await expect(page.locator('.agenda-source')).toHaveCount(0);
});

test('Work on this opens the task’s own workspace with the task in the composer', async ({ page }) => {
  await openDemoTasks(page);
  await page.locator('.issue-row.task-row').filter({ hasText: 'Revise section 3 with the September close' }).locator('.issue-row-main').click();
  await page.locator('.task-detail').getByRole('button', { name: /Work on this/ }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Q3 regional operations review');
  await expect(page.getByLabel('Chat message')).toHaveValue(/^Work on the task “Revise section 3 with the September close”/);
});

test('calendar sign-in is not an Agenda control on Tasks', async ({ page }) => {
  await openDemoTasks(page);
  await expect(page.locator('.agenda-source')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Calendar event:/ })).toHaveCount(0);
  await expect(page.getByRole('form', { name: 'Plan a focus block' })).toHaveCount(0);
});
