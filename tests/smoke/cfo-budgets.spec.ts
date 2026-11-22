import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * The CFO's budgets (COO rebuild L9) in the mock harness: Settings → Usage →
 * Budget for each subagent (tokens with a k / M suffix, and minutes) and
 * Project budgets (add, list with usage, remove). The mock keeps the policy
 * in `mockCfoSettings`.
 */

async function openUsage(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'usage');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: /Usage/ }).first().click();
  }
  await page.getByRole('tab', { name: 'Budgets', exact: true }).click();
}

const saved = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockCfoSettings') ?? 'null'));

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaces', JSON.stringify([
    { id: 'budget-fixture', title: 'Budget fixture', status: 'active', path: 'workspaces/budget-fixture', cwd: '/fixture/workspaces/budget-fixture', modified: '2026-11-10T12:00:00Z' },
  ])));
});

test('A subagent budget saves tokens with a suffix and minutes, and blank clears the cap', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const tokens = page.getByLabel('Subagent budget, tokens', { exact: true });
  const minutes = page.getByLabel('Subagent budget, minutes', { exact: true });
  await tokens.fill('2M');
  await minutes.fill('90');
  await page.getByRole('button', { name: 'Save budget for each subagent', exact: true }).click();
  await expect.poll(async () => (await saved(page))?.subagentBudget).toEqual({ tokens: 2000000, minutes: 90 });
  await expect(tokens).toHaveValue('2M');
  await tokens.fill('lots');
  await page.getByRole('button', { name: 'Save budget for each subagent', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'tokens are a whole number' })).toBeVisible();
  expect((await saved(page)).subagentBudget).toEqual({ tokens: 2000000, minutes: 90 });
  await tokens.fill('');
  await minutes.fill('');
  await page.getByRole('button', { name: 'Save budget for each subagent', exact: true }).click();
  await expect.poll(async () => (await saved(page))?.subagentBudget).toEqual({});
});

test('A project budget is added, listed with its usage, and removed', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const add = page.getByRole('group', { name: 'Add a project budget' });
  await choose(add.getByLabel('Project to budget', { exact: true }), { label: 'Budget fixture' });
  await add.getByLabel('New budget, 5-hour tokens', { exact: true }).fill('2M');
  await add.getByLabel('New budget, 7-day tokens', { exact: true }).fill('20M');
  await add.getByRole('button', { name: 'Add project budget', exact: true }).click();
  await expect.poll(async () => (await saved(page))?.projectBudgets).toEqual([{ workspaceId: 'budget-fixture', fiveHourTokens: 2000000, weekTokens: 20000000 }]);
  const row = page.getByRole('group', { name: 'Budget for Budget fixture' });
  await expect(row).toContainText('1.2M of 2M in the last 5 hours');
  await expect(row).toContainText('4M of 20M in the last 7 days');
  await expect(row).not.toContainText('Reached');
  await row.getByRole('button', { name: 'Remove the budget for Budget fixture', exact: true }).click();
  await expect(row).toHaveCount(0);
  expect((await saved(page)).projectBudgets).toEqual([]);
});

test('A budget with no cap is refused', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const add = page.getByRole('group', { name: 'Add a project budget' });
  await choose(add.getByLabel('Project to budget', { exact: true }), { label: 'Budget fixture' });
  await add.getByRole('button', { name: 'Add project budget', exact: true }).click();
  await expect(page.getByRole('alert').filter({ hasText: 'give the 5-hour cap' })).toBeVisible();
  expect((await saved(page))?.projectBudgets ?? []).toEqual([]);
});

test('Settings search finds the budget rows', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  const search = page.getByRole('searchbox', { name: 'Find a setting' });
  const index = page.getByRole('navigation', { name: 'Settings sections' });
  for (const label of ['Budget for each subagent', 'Project budgets']) {
    await search.fill(label);
    await expect(index.getByRole('button', { name: label, exact: true }).and(page.locator('.settings-index-control'))).toHaveCount(1);
  }
});
