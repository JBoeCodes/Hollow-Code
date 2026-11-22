// The Tasks page's Projects tab (overhaul B4, board S3c), against the
// browser mock (`src/mockTracker.ts`: Receipts rebuild has issues in mixed
// states, Warehouse enrichment two, Claims intake none). Written in lane 3
// of the COO rebuild without running it; the one full browser run on final
// main runs it.
import { expect, test } from './fixtures/test';
import { openTasks } from './choose';

test.beforeEach(async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await page.getByRole('group', { name: 'Views' }).getByRole('button', { name: 'Projects' }).click();
});

test('the Projects tab lists the team’s projects with status, lead, target, progress and issues', async ({ page }) => {
  const list = page.getByLabel('Projects', { exact: true });
  const rows = list.locator('.project-row');
  // Started first, then planned, then backlog.
  await expect(rows.locator('.project-name')).toContainText(['Receipts rebuild', 'Warehouse enrichment', 'Claims intake']);
  const receipts = list.locator('.project-row[data-project-id="p-1"]');
  await expect(receipts).toContainText('In Progress');
  await expect(receipts).toContainText('Alex Rivera');
  // ENG-31, 32, 33, 35 and 36 are Receipts rebuild's: ENG-35 done (1 of 5).
  await expect(receipts.locator('.project-progress')).toContainText('20%');
  await expect(receipts.locator('.project-issue-count')).toHaveText('4 open of 5');
  const empty = list.locator('.project-row[data-project-id="p-3"]');
  await expect(empty).toContainText('No issues');
  await expect(empty).toContainText('No lead');
  // The issue filters and Display don't apply here.
  await expect(page.getByRole('group', { name: 'Filters' })).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Group by' })).toHaveCount(0);
});

test('opening a project shows its detail with its issues, and an issue opens from there', async ({ page }) => {
  await page.locator('.project-row[data-project-id="p-1"] .issue-row-main').click();
  const detail = page.locator('.project-detail');
  await expect(detail.getByRole('heading', { name: 'Receipts rebuild' })).toBeVisible();
  const issues = detail.getByRole('region', { name: 'Issues' }).or(detail.locator('.project-issues'));
  await expect(issues.locator('.issue-row[data-issue-id="i-31"]')).toBeVisible();
  await expect(issues.locator('.issue-row[data-issue-id="i-34"]')).toHaveCount(0);
  await issues.locator('.issue-row[data-issue-id="i-31"] .issue-row-main').click();
  await expect(page.locator('.issue-pane .issue-detail-crumb')).toContainText('ENG-31');
});

test('a project with no issues says so', async ({ page }) => {
  await page.locator('.project-row[data-project-id="p-3"] .issue-row-main').click();
  await expect(page.locator('.project-detail')).toContainText('No issues in this project yet.');
});

test('Find a task narrows the projects, and j and k move through them', async ({ page }) => {
  await page.getByRole('textbox', { name: 'Find a task' }).fill('claims');
  await expect(page.locator('.project-row')).toHaveCount(1);
  await page.getByRole('textbox', { name: 'Find a task' }).fill('');
  await page.locator('.tracker-tasks').click({ position: { x: 5, y: 5 } });
  await page.keyboard.press('j');
  await expect(page.locator('.project-row[data-selected]')).toHaveAttribute('data-project-id', 'p-1');
  await page.keyboard.press('j');
  await expect(page.locator('.project-row[data-selected]')).toHaveAttribute('data-project-id', 'p-2');
});
