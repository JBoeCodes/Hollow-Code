// Settings → Usage → Value (overnight A12), against the mock: work whose
// kind has no baseline reads "baseline not set", never 0 h; the editor has
// nothing filled in for it and says so; setting one shows when.
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

test('work with no baseline reads "baseline not set", and the editor says which are set', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const block = page.locator('.usage-block').filter({ hasText: 'Value this month' });
  await expect(block.locator('.usage-total')).toContainText('1 without a baseline');
  await expect(block.getByRole('row', { name: /HR/ })).toContainText('baseline not set');
  await expect(block.getByRole('row', { name: /Finance/ })).toContainText('~1.5 h');

  const admin = block.getByLabel('Minutes saved per accepted admin run');
  await expect(admin).toHaveValue('');
  await expect(admin).toHaveAttribute('placeholder', 'not set');
  await expect(block.getByRole('row', { name: /^admin/ })).toContainText('not set');
  await expect(block.getByRole('row', { name: /^data/ })).toContainText(/set \d{4}-\d{2}-\d{2}/);
  await admin.fill('30');
  await admin.press('Enter');
  await expect(block.getByRole('row', { name: /^admin/ })).toContainText(/set \d{4}-\d{2}-\d{2}/);
  expect(await page.evaluate(() => (window as unknown as { __mockApiMisses?: string[] }).__mockApiMisses ?? [])).toEqual([]);
});

// G4: hours saved stay hidden until a run is accepted (an accepted Result,
// or its tracker issue reached Done after the run started).
test('hours saved stay hidden while no run is accepted', async ({ page }) => {
  await page.addInitScript(() => { (window as unknown as { __mockValueNoneAccepted: boolean }).__mockValueNoneAccepted = true; });
  await page.goto('/?mock=1');
  await openUsage(page);
  const block = page.locator('.usage-block').filter({ hasText: 'Value this month' });
  await expect(block.locator('.usage-total')).toContainText('3 runs');
  await expect(block.locator('.usage-total')).not.toContainText(/saved|baseline/);
  await expect(block.getByRole('columnheader', { name: 'Saved (estimate)' })).toHaveCount(0);
  await expect(block.getByText('Hours saved stay hidden until a run is accepted.')).toBeVisible();
});

test('once runs are accepted, hours saved say how many and their basis', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const block = page.locator('.usage-block').filter({ hasText: 'Value this month' });
  await expect(block.locator('.usage-total')).toContainText('from 2 accepted runs (estimate from your baselines');
  await expect(block.getByRole('columnheader', { name: 'Saved (estimate)' })).toBeVisible();
});
