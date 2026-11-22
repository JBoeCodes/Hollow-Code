import { openTasks } from './choose';
import { test, expect } from './fixtures/test';

// The Tasks pane's reserved task sections (Paperclip PR 11). The PLAN.md
// "Create tasks from plan" preview this file also covered went with the
// workspace panel's Plan section (COO rebuild L4, 2026-11-10).

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); });

test('the Tasks pane shows Done when and Activity read-only', async ({ page }) => {
  await openTasks(page);
  await expect(page.getByRole('heading', { name: 'Tasks', exact: true })).toBeVisible();
  await page.locator('.issue-group[data-kind="local"] .issue-row.task-row').filter({ hasText: 'Finish artifact safety defaults' }).locator('.issue-row-main').click();
  const detail = page.getByRole('complementary', { name: 'Task details' });
  const doneWhen = detail.locator('details.task-detail-reserved', { hasText: 'Done when' });
  await expect(doneWhen).toHaveCount(1);
  await doneWhen.locator('summary').click();
  await expect(doneWhen).toContainText('HTML previews open offline and without scripts by default.');
  await expect(detail.locator('details.task-detail-reserved > summary', { hasText: /^Log$/ })).toHaveCount(1);
  // Neither section is part of the editable notes.
  await expect(detail.locator('.task-detail-notes')).not.toContainText('Opened from the security review');
});
