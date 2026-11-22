import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { showUsageTab } from './usageTab';

/** "$0" or "$0.00" as a whole figure; not "$0.42" and not "<$0.01". */
const ZERO_DOLLARS = /\$0(?:\.0+)?(?![.\d])/;

async function openUsage(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'usage');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: /Usage/ }).click();
  }
  // Settings PR 8: Usage opens on Limits; the totals and tables are on All agents.
  await showUsageTab(page, 'All agents');
}

test('Settings → Usage separates missing run costs from known zero cap spend', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const panel = page.locator('.usage-panel');
  await expect(panel).toBeVisible();
  for (const title of ['By surface', 'By automation', 'By agent']) {
    const block = panel.locator('.usage-block').filter({ has: page.getByRole('heading', { name: title, exact: true }) });
    await expect(block.getByRole('heading', { name: title, exact: true })).toBeVisible();
    await expect(block.locator('.usage-table')).not.toContainText(ZERO_DOLLARS);
  }
  await expect(panel.getByRole('rowheader', { name: 'Morning brief' })).toBeVisible();
  await expect(panel.getByRole('rowheader', { name: 'Teammates' })).toBeVisible();
  await expect(panel).toContainText('≈$3.10 estimated');
  await expect(panel).toContainText('$0.42 reported');
  await expect(panel).toContainText('1 partial');
  await expect(panel).toContainText('2 with no usable usage figures');
  await expect(panel).toContainText("1 didn't start");
  await expect(panel).toContainText('1 in progress');
  const total = panel.locator('.usage-totals > .usage-total');
  await expect(total).toHaveCount(1);
  await expect(total).not.toContainText(ZERO_DOLLARS);
  const agents = panel.locator('.usage-block').filter({ has: page.getByRole('heading', { name: 'By agent', exact: true }) });
  const codex = agents.getByRole('row').filter({ has: page.getByRole('rowheader', { name: 'Codex', exact: true }) });
  await expect(codex.getByRole('cell').last()).toHaveText(/^\u2014$/);
  // Cap totals may be zero. The overnight row shows no spend since the night
  // queue went (COO rebuild S6, 11-10), so it says so instead of a $0.
  // Settings PR 8: the caps sit on Limits.
  await showUsageTab(page, 'Limits');
  const caps = panel.getByRole('list', { name: 'Caps Hollow keeps for you' });
  await expect(caps.locator('[data-cap="overnight"]')).toContainText('Spend per night isn’t shown here');
  await expect(caps.locator('[data-cap="overnight"]')).not.toContainText(ZERO_DOLLARS);
  // Preserve the whole-panel guard, excluding only the fixed cap list.
  await expect.poll(() => panel.evaluate((element: HTMLElement) => {
    const text = element.innerText;
    const capText = element.querySelector<HTMLElement>('.budgets-fixed')?.innerText;
    return capText ? text.replace(capText, '') : text;
  })).not.toMatch(ZERO_DOLLARS);
});

// Overnight 9 A13: the price table's visible home. A saved row asks once
// more, then lists as the person's own with Remove; built-in rows have none.
test('Settings → Advanced → Prices lists the table and saves a price after a confirm', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  // Prices is its own closed disclosure in Advanced since Settings PR 3.
  const prices = page.locator('.usage-prices');
  await prices.locator('summary').click();
  await expect(prices.getByRole('rowheader', { name: 'claude-opus-5-5 (built in)' })).toBeVisible();
  await expect(prices.getByRole('button', { name: /^Remove your price/ })).toHaveCount(0);

  await prices.getByRole('button', { name: 'Save price…' }).click();
  await expect(prices.getByRole('alert')).toContainText('Enter a model id');
  await prices.getByLabel('Model id', { exact: true }).fill('gpt-test-codex');
  await prices.getByLabel('Input price', { exact: true }).fill('2');
  await prices.getByLabel('Cached input price', { exact: true }).fill('0.2');
  await prices.getByLabel('Output price', { exact: true }).fill('10');
  await prices.getByLabel('Source of the price', { exact: true }).fill('test price');
  await prices.getByRole('button', { name: 'Save price…' }).click();
  const confirm = prices.getByRole('alertdialog', { name: 'Confirm the price' });
  await expect(confirm).toContainText('Every later cost of this model uses it.');
  await confirm.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(confirm).toHaveCount(0);
  await expect(prices.getByRole('rowheader', { name: 'gpt-test-codex', exact: true })).toBeVisible();
  await expect(prices.locator('tbody tr')).toHaveCount(3);

  await prices.getByRole('button', { name: 'Remove your price for gpt-test-codex' }).click();
  const removing = prices.getByRole('alertdialog', { name: 'Confirm the price' });
  await expect(removing).toContainText('Remove your price for gpt-test-codex?');
  await removing.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(prices.getByRole('rowheader', { name: 'gpt-test-codex', exact: true })).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).not.toContain('price_table');
});

test('switching the usage period refetches that window', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  await expect(page.locator('.usage-panel')).toBeVisible();
  const before = await page.evaluate(() => window.__mockApiCalls?.usage_summary ?? 0);
  expect(before).toBeGreaterThan(0);
  await page.getByRole('tablist', { name: 'Usage period' }).getByRole('tab', { name: 'This month' }).click();
  await expect
    .poll(() => page.evaluate(() => window.__mockApiCalls?.usage_summary ?? 0))
    .toBeGreaterThan(before);
  expect(await page.evaluate(() => localStorage.getItem('mockLastUsageWindow'))).toBe('month');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).not.toContain('usage_summary');
});
