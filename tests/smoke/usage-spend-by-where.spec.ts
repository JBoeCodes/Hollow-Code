import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { showUsageTab } from './usageTab';

async function openUsage(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'usage');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: /Usage/ }).click();
  }
  await showUsageTab(page, 'All agents');
}

test('Settings, Usage shows spend by project, lead, subagent and account', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const panel = page.locator('.usage-panel');
  await expect(panel).toBeVisible();
  const rows: Record<string, string> = {
    // Settings PR 8: By project is By workspace; By lead's costliest ten are Costliest conversations.
    'By workspace': 'Hollow',
    'Costliest conversations': 'COO',
    'By subagent': 'Review the ledger',
    'By account': 'Claude Code login',
  };
  for (const [title, row] of Object.entries(rows)) {
    const block = panel.locator('.usage-block').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
    await expect(block.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(block.getByRole('rowheader', { name: row, exact: true })).toBeVisible();
  }
});
