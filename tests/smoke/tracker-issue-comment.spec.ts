// Writing a comment from the issue detail (overhaul B5, board Fallback 6),
// against the browser mock (`src/mockTracker.ts`). Written in lane 3 of the
// COO rebuild without running it; the one full browser run on final main
// runs it. No live tracker call: the mock plays the adapter.
import { expect, test } from './fixtures/test';
import { openTasks, showTaskList } from './choose';

test.beforeEach(async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.locator('.issue-row[data-issue-id="i-32"] .issue-row-main').click();
});

test('a comment posts and joins the thread', async ({ page }) => {
  const box = page.getByRole('form', { name: 'Write a comment' });
  await expect(box.getByRole('button', { name: 'Comment' })).toBeDisabled();
  await box.getByRole('textbox', { name: 'Comment' }).fill('Export runs on the 1st.');
  await box.getByRole('button', { name: 'Comment' }).click();
  await expect(page.getByRole('region', { name: 'Activity', exact: true })).toContainText('Export runs on the 1st.');
  await expect(box.getByRole('textbox', { name: 'Comment' })).toHaveValue('');
});

test('a draft survives a move to another issue and back', async ({ page }) => {
  await page.getByRole('form', { name: 'Write a comment' }).getByRole('textbox', { name: 'Comment' }).fill('Half a thought');
  await showTaskList(page);
  await page.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await showTaskList(page);
  await page.locator('.issue-row[data-issue-id="i-32"] .issue-row-main').click();
  await expect(page.getByRole('form', { name: 'Write a comment' }).getByRole('textbox', { name: 'Comment' })).toHaveValue('Half a thought');
});

test('an overlong comment is held back with its count', async ({ page }) => {
  const box = page.getByRole('form', { name: 'Write a comment' });
  await box.getByRole('textbox', { name: 'Comment' }).fill('x'.repeat(2001));
  await expect(box).toContainText('Too long: 2,001 of 2,000 bytes');
  await expect(box.getByRole('button', { name: 'Comment' })).toBeDisabled();
});
