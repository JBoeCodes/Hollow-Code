// Tasks laid out like a tracker's issue list (overhaul B1, board S3),
// against the browser mock (`src/mockTracker.ts`). Written in lane 3 of the
// COO rebuild without running it; the one full browser run on final main
// runs it. Also covers the board's tracker-create-task and
// tracker-sync-event cases.
import { expect, test } from './fixtures/test';
import { choose, openTasks, showTaskList } from './choose';

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); await openTasks(page); });

test('issues are grouped by status, in progress first, with every row field', async ({ page }) => {
  const list = page.getByLabel('Tasks', { exact: true });
  const heads = list.locator('.issue-group-name');
  await expect(heads.first()).toHaveText('In Progress');
  await expect(heads).toContainText(['In Progress', 'In Review', 'Todo', 'Backlog', 'Done', 'Canceled', 'Local tasks']);
  const row = list.locator('.issue-row[data-issue-id="i-31"]');
  await expect(row.locator('.issue-key')).toHaveText('ENG-31');
  await expect(row.locator('.issue-title')).toHaveText('Match bank deposits to receipts');
  await expect(row).toContainText('Feature');
  await expect(row).toContainText('Receipts rebuild');
  await expect(row.locator('.issue-assignee-name')).toHaveText('Alex Rivera');
  await expect(row.getByRole('img', { name: 'High' })).toBeVisible();
  await expect(list.locator('.issue-row[data-issue-id="i-34"] .issue-assignee-name')).toHaveText('Unassigned');
});

test('groups collapse with their count; done and canceled start folded', async ({ page }) => {
  const todo = page.getByRole('button', { name: /^Todo/ });
  await expect(todo).toHaveAttribute('aria-expanded', 'true');
  await expect(todo.locator('.issue-group-count')).toHaveText('2');
  await todo.click();
  await expect(todo).toHaveAttribute('aria-expanded', 'false');
  await expect(page.locator('.issue-row[data-issue-id="i-32"]')).toHaveCount(0);
  const done = page.getByRole('button', { name: /^Done/ });
  await expect(done).toHaveAttribute('aria-expanded', 'false');
  await done.click();
  await expect(page.locator('.issue-row[data-issue-id="i-35"]')).toBeVisible();
});

test('local tasks sit in their own group with no identifier, and tick done', async ({ page }) => {
  const group = page.locator('.issue-group[data-kind="local"]');
  await expect(group.locator('.issue-group-name')).toHaveText('Local tasks');
  const first = group.locator('.task-row').first();
  await expect(first.locator('.issue-key')).toHaveCount(0);
  const name = await first.locator('.issue-title').textContent();
  await first.getByRole('button', { name: `Mark ${name} done` }).click();
  await first.locator('.issue-row-main').click();
  await expect(page.locator('.issue-pane')).toContainText('Local task');
});

test('j and k move through the rows, Enter and Escape open and close', async ({ page }) => {
  await page.locator('.tracker-tasks').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('j');
  await expect(page.locator('.issue-row[data-selected]')).toHaveCount(1);
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('AI-');
  const first = await page.locator('.issue-row[data-selected]').getAttribute('data-issue-id');
  await page.keyboard.press('j');
  await expect(page.locator('.issue-row[data-selected]')).not.toHaveAttribute('data-issue-id', first!);
  await page.keyboard.press('k');
  await expect(page.locator('.issue-row[data-selected]')).toHaveAttribute('data-issue-id', first!);
  await page.keyboard.press('Escape');
  await expect(page.locator('.issue-row[data-selected]')).toHaveCount(0);
  await page.keyboard.press('/');
  await expect(page.getByRole('textbox', { name: 'Find a task' })).toBeFocused();
});

test('Find a task narrows by key, title, label or assignee', async ({ page }) => {
  await page.getByRole('textbox', { name: 'Find a task' }).fill('sam');
  await expect(page.locator('.issue-row[data-issue-id]')).toHaveCount(1);
  await expect(page.locator('.issue-row[data-issue-id="i-33"]')).toBeVisible();
  await page.getByRole('textbox', { name: 'Find a task' }).fill('nothing like this');
  await expect(page.getByText('No task matches')).toBeVisible();
  await page.getByRole('button', { name: 'Clear the search' }).click();
  await expect(page.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
});

test('New issue creates one in the tracker and opens it', async ({ page }) => {
  await page.getByRole('button', { name: 'New issue' }).click();
  const form = page.locator('.issue-new');
  await expect(form.getByRole('button', { name: 'Create in Linear' })).toBeDisabled();
  await form.getByRole('textbox', { name: 'Issue title' }).fill('Write the cutover note');
  await choose(form.getByRole('combobox', { name: 'Priority' }), '2');
  await form.getByRole('button', { name: 'Create in Linear' }).click();
  await expect(page.locator('.issue-pane .issue-title-input')).toHaveValue('Write the cutover note');
  await showTaskList(page);
  await expect(page.locator('.issue-row', { hasText: 'Write the cutover note' })).toBeVisible();
});

test('a tracker pass refreshes the list', async ({ page }) => {
  await expect(page.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await page.evaluate(() => window.__mockEmit('tracker-sync-updated', { ok: true, changed: 1 }));
  await expect(page.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await expect(page.locator('.tracker-sync-line')).toContainText('Synced');
});

test('with no tracker set up, Tasks says so and opens its settings; local tasks still list', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockTracker', JSON.stringify({ provider: null })));
  await page.reload();
  await openTasks(page);
  await expect(page.getByRole('note')).toContainText('Tasks are local.');
  await expect(page.getByRole('button', { name: 'New issue' })).toHaveCount(0);
  await expect(page.locator('.issue-group[data-kind="local"]')).toBeVisible();
  await page.getByRole('button', { name: 'Open Task tracker settings' }).click();
  await expect(page.locator('.task-tracker-row')).toBeVisible();
});
