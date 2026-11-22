import { openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Tasks search, kept after the issue-list rewrite (11-10): the scopes, List /
 * Board layouts, saved filters, bulk edits, suggestions row and row menu are
 * gone. What stays: search by title, opening a local task from its row, and a
 * new local task keeping the search that it matches. Demo fixtures.
 */

async function openDemoTasks(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    // The mock's create doesn't add to the list; this one does, like the app.
    const create = api.createTask;
    let n = 0;
    api.createTask = async (input: { title: string }) => {
      await create(input);
      const made = { url: `vault://tasks/smoke-new-${++n}.md`, id: `smoke-new-${n}.md`, existing: false };
      const w = window as unknown as { __mockDemoTasks: Record<string, unknown>[] };
      const base = w.__mockDemoTasks[0];
      w.__mockDemoTasks = [...w.__mockDemoTasks, { ...base, url: made.url, title: input.title, vaultPath: made.url.replace('vault://', ''), priority: null, due: null, bucket: 'undated', status: 'Not started', labels: [], projectName: null, projectUrl: null, description: '', unresolvedBlockers: [], blockedBy: [], ready: true, subtasks: [] }];
      return made;
    };
  });
  await openTasks(page);
  await expect(page.locator('.task-collection .issue-row').first()).toBeVisible();
}
const search = (page: Page) => page.getByRole('textbox', { name: 'Find a task' });
const localRow = (page: Page, title: string) => page.locator('.issue-group[data-kind="local"] .issue-row.task-row').filter({ hasText: title });

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop task layouts'); });

test('an unmatched search shows no task in the list', async ({ page }) => {
  await openDemoTasks(page);
  await expect(page.locator('.task-collection .issue-row').first()).toBeVisible();
  await search(page).fill('zz no such task');
  await expect(page.locator('.task-collection .issue-row')).toHaveCount(0);
});

test('a list row opens its task', async ({ page }) => {
  await openDemoTasks(page);
  await localRow(page, 'Send the draft to Dana for review').locator('.issue-row-main').click();
  await expect(page.locator('.task-detail-title')).toHaveText('Send the draft to Dana for review');
});

test('a new local task that matches the search keeps it', async ({ page }) => {
  await openDemoTasks(page);
  await search(page).fill('Quarterly');
  await page.getByRole('button', { name: 'New local task', exact: true }).click();
  await page.getByRole('textbox', { name: 'Task name' }).fill('Quarterly numbers for Dana');
  await page.getByRole('button', { name: 'Create task', exact: true }).click();
  await expect(page.locator('.task-detail-title')).toHaveText('Quarterly numbers for Dana');
  await expect(search(page)).toHaveValue('Quarterly');
  await expect(localRow(page, 'Quarterly numbers for Dana')).toBeVisible();
});
