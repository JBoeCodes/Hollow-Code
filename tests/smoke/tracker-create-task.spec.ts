// Create in Linear from the Tasks page (design section 7's feature map and
// section 12), against the browser mock (`src/mockTracker.ts`). Written in
// the 11-12 overnight (row T2) without running it; the lane's browser run
// covers it. tracker-linked-tasks keeps its one New issue case.
import { expect, test } from './fixtures/test';
import { choose, openTasks, showTaskList } from './choose';

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); await openTasks(page); });

test('New issue opens the form with every main field', async ({ page }) => {
  await page.getByRole('button', { name: 'New issue' }).click();
  const form = page.locator('.issue-new');
  await expect(form.getByRole('heading', { name: 'New issue' })).toBeVisible();
  await expect(form.getByRole('textbox', { name: 'Issue title' })).toBeVisible();
  await expect(form.getByRole('textbox', { name: 'Description' })).toBeVisible();
  for (const name of ['Status', 'Priority', 'Assignee', 'Project']) await expect(form.getByRole('combobox', { name })).toBeVisible();
  await expect(form.getByLabel('Due date')).toBeVisible();
  await form.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.locator('.issue-new')).toHaveCount(0);
});

test('a title is required: Create stays off, and Enter with no title creates nothing', async ({ page }) => {
  // Count once the mock's issues have loaded, not the empty first frame.
  await expect(page.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  const before = await page.locator('.issue-row[data-issue-id]').count();
  await page.getByRole('button', { name: 'New issue' }).click();
  const form = page.locator('.issue-new');
  const create = form.getByRole('button', { name: 'Create in Linear' });
  await expect(create).toBeDisabled();
  const title = form.getByRole('textbox', { name: 'Issue title' });
  await title.fill('   ');
  await expect(create).toBeDisabled();
  await title.press('Enter');
  // The form stays, and no issue was added.
  await expect(form).toBeVisible();
  await expect(page.locator('.issue-row[data-issue-id]')).toHaveCount(before);
});

test('the created issue lands in the list with its fields and opens', async ({ page }) => {
  await page.getByRole('button', { name: 'New issue' }).click();
  const form = page.locator('.issue-new');
  await form.getByRole('textbox', { name: 'Issue title' }).fill('Draft the claims intake form');
  await choose(form.getByRole('combobox', { name: 'Priority' }), '1');
  await choose(form.getByRole('combobox', { name: 'Project' }), 'p-3');
  await form.getByRole('button', { name: 'Create in Linear' }).click();
  // The pane switches to the new issue's detail.
  await expect(page.locator('.issue-new')).toHaveCount(0);
  await expect(page.locator('.issue-pane .issue-title-input')).toHaveValue('Draft the claims intake form');
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('AI-');
  // The list has it, with the project chosen, under Todo (the team default).
  // A narrow window shows the pane in the list's place: close it to look.
  const narrow = await page.locator('.task-collection').isHidden();
  await showTaskList(page);
  const row = page.locator('.issue-row', { hasText: 'Draft the claims intake form' });
  await expect(row).toBeVisible();
  await expect(row).toContainText('Claims intake');
  await expect(row.getByRole('img', { name: 'Urgent' })).toBeVisible();
  if (!narrow) {
    await expect(row).toHaveAttribute('data-selected', '');
    // Closing and clicking the row opens it again.
    await page.getByRole('button', { name: 'Close issue details' }).click();
  }
  await row.locator('.issue-row-main').click();
  await expect(page.locator('.issue-pane .issue-title-input')).toHaveValue('Draft the claims intake form');
});

test('c opens the same form from the list', async ({ page }) => {
  await page.locator('.tracker-tasks').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('c');
  await expect(page.locator('.issue-new')).toBeVisible();
  await expect(page.locator('.issue-new').getByRole('button', { name: 'Create in Linear' })).toBeDisabled();
});
