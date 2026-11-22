// Tasks' views, filters, grouping and order (overhaul B2, board S3b),
// against the browser mock (`src/mockTracker.ts`: the viewer is Alex
// Rivera, u-1). Written in lane 3 of the COO rebuild without running it;
// the one full browser run on final main runs it.
import { expect, test } from './fixtures/test';
import { choose, openTasks } from './choose';

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); await openTasks(page); });

const views = (page: import('@playwright/test').Page) => page.getByRole('group', { name: 'Views' });
const list = (page: import('@playwright/test').Page) => page.getByLabel('Tasks', { exact: true });

test('My issues keeps the viewer’s issues and the local tasks', async ({ page }) => {
  await views(page).getByRole('button', { name: 'My issues' }).click();
  await expect(views(page).getByRole('button', { name: 'My issues' })).toHaveAttribute('aria-pressed', 'true');
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  // ENG-33 is Sam Lee's, ENG-34 nobody's.
  await expect(list(page).locator('.issue-row[data-issue-id="i-33"]')).toHaveCount(0);
  await expect(list(page).locator('.issue-row[data-issue-id="i-34"]')).toHaveCount(0);
  await expect(page.locator('.issue-group[data-kind="local"]')).toBeVisible();
});

test('My issues says so when the key’s owner isn’t read yet', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockTracker', JSON.stringify({ viewer: false })));
  await page.reload();
  await openTasks(page);
  await views(page).getByRole('button', { name: 'My issues' }).click();
  await expect(page.getByRole('note').filter({ hasText: 'My issues can’t tell which issues are yours' })).toBeVisible();
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(0);
});

test('Active and Backlog keep their state kinds', async ({ page }) => {
  await views(page).getByRole('button', { name: 'Active' }).click();
  await expect(list(page).locator('.issue-group-name')).toContainText(['In Progress', 'In Review', 'Todo']);
  await expect(list(page).locator('.issue-row[data-issue-id="i-34"]')).toHaveCount(0);
  await views(page).getByRole('button', { name: 'Backlog' }).click();
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(1);
  await expect(list(page).locator('.issue-row[data-issue-id="i-34"]')).toBeVisible();
  await expect(page.locator('.issue-group[data-kind="local"]')).toHaveCount(0);
});

test('By project and By assignee group the open issues', async ({ page }) => {
  await views(page).getByRole('button', { name: 'By project' }).click();
  await expect(list(page).locator('.issue-group[data-by="project"] .issue-group-name')).toContainText(['Receipts rebuild', 'Warehouse enrichment']);
  await expect(list(page).locator('.issue-row[data-issue-id="i-35"]')).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Group by' })).toContainText('Project');
  await views(page).getByRole('button', { name: 'By assignee' }).click();
  const heads = list(page).locator('.issue-group[data-by="assignee"] .issue-group-name');
  // Yours first, nobody's last.
  await expect(heads.first()).toHaveText('Alex Rivera');
  await expect(heads.last()).toHaveText('No assignee');
});

test('a filter chip narrows the list, and its × removes it', async ({ page }) => {
  const filters = page.getByRole('group', { name: 'Filters' });
  await filters.getByRole('button', { name: 'Add a filter' }).click();
  await page.getByRole('menuitem', { name: 'Priority' }).click();
  await choose(filters.getByRole('combobox', { name: 'Priority filter' }), '1');
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(1);
  await expect(list(page).locator('.issue-row[data-issue-id="i-33"]')).toBeVisible();
  // A filter only an issue has hides the local tasks.
  await expect(page.locator('.issue-group[data-kind="local"]')).toHaveCount(0);
  await filters.getByRole('button', { name: 'Remove the Priority filter' }).click();
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await expect(page.locator('.issue-group[data-kind="local"]')).toBeVisible();
});

test('label, project, cycle and due filters each keep their matches', async ({ page }) => {
  const filters = page.getByRole('group', { name: 'Filters' });
  const add = async (name: string) => { await filters.getByRole('button', { name: 'Add a filter' }).click(); await page.getByRole('menuitem', { name }).click(); };
  await add('Label');
  await choose(filters.getByRole('combobox', { name: 'Label filter' }), 'Accounting');
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(2);
  await add('Cycle');
  await choose(filters.getByRole('combobox', { name: 'Cycle filter' }), 'Cycle 4');
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(1);
  await filters.getByRole('button', { name: 'Clear filters' }).click();
  await add('Due date');
  await choose(filters.getByRole('combobox', { name: 'Due date filter' }), 'overdue');
  await expect(list(page).locator('.issue-row[data-issue-id="i-33"]')).toBeVisible();
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toHaveCount(0);
  await filters.getByRole('button', { name: 'Clear filters' }).click();
  await add('Project');
  await choose(filters.getByRole('combobox', { name: 'Project filter' }), 'p-2');
  await expect(list(page).locator('.issue-row[data-issue-id="i-34"]')).toBeVisible();
  await expect(list(page).locator('.issue-row[data-issue-id="i-31"]')).toHaveCount(0);
});

test('Group and Order change the list', async ({ page }) => {
  await choose(page.getByRole('combobox', { name: 'Group by' }), 'priority');
  await expect(list(page).locator('.issue-group[data-by="priority"] .issue-group-name').first()).toHaveText('Urgent');
  await choose(page.getByRole('combobox', { name: 'Order by' }), 'due');
  await choose(page.getByRole('combobox', { name: 'Group by' }), 'status');
  // Todo: ENG-36 (due in 5 days) before ENG-32 (due in 9).
  const todo = list(page).locator('.issue-group').filter({ has: page.locator('.issue-group-name', { hasText: /^Todo$/ }) });
  await expect(todo.locator('.issue-row').first()).toHaveAttribute('data-issue-id', 'i-36');
});

test('the view persists in this window', async ({ page }) => {
  await views(page).getByRole('button', { name: 'Backlog' }).click();
  await page.reload();
  await openTasks(page);
  await expect(views(page).getByRole('button', { name: 'Backlog' })).toHaveAttribute('aria-pressed', 'true');
  await expect(list(page).locator('.issue-row[data-issue-id]')).toHaveCount(1);
});
