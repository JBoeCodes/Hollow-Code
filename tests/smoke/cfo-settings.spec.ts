import { expect, test, type Page } from './fixtures/test';
import { choose, optionLabels } from './choose';
import { showUsageTab } from './usageTab';

/**
 * The CFO's Settings rows (COO rebuild L7, docs/CFO.md) in the mock harness:
 * Models → Effort (Automatic or Fixed) and Models the CFO may use (real
 * models as checkboxes in a sheet, no free text), Usage → Reserve for your own prompts,
 * each account's room, and Extra usage for unattended work. The mock keeps
 * the policy in `mockCfoSettings`.
 */

async function openSection(page: Page, section: 'models' | 'usage') {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), section);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: section === 'models' ? 'Models' : /Usage/, exact: section === 'models' }).first().click();
  }
}

const saved = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockCfoSettings') ?? 'null'));

test('Effort is Automatic by default and Fixed shows each agent\'s saved level', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, 'models');
  const effort = page.getByLabel('Effort', { exact: true });
  await expect(effort).toHaveAttribute('data-value', 'automatic');
  expect(await optionLabels(effort)).toEqual(['Automatic', 'Fixed']);
  await expect(page.getByLabel('Claude effort', { exact: true })).toHaveCount(0);
  await choose(effort, 'fixed');
  await expect(page.getByLabel('Claude effort', { exact: true })).toBeVisible();
  expect((await saved(page)).effortMode).toBe('fixed');
  await choose(effort, 'automatic');
  await expect(page.getByLabel('Claude effort', { exact: true })).toHaveCount(0);
  expect((await saved(page)).effortMode).toBe('automatic');
});

test('Models the CFO may use lists real models as checkboxes, with no text box, and empty means all', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, 'models');
  const row = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: 'Models the CFO may use' }) });
  await expect(row).toContainText('Any installed agent, on its saved model');
  // Settings S5b: the checkboxes open in a sheet from the row's Choose.
  const opener = row.getByRole('button', { name: 'Choose the models the CFO may use', exact: true });
  await opener.click();
  const sheet = page.getByRole('dialog', { name: 'Models the CFO may use' });
  const group = sheet.getByRole('group', { name: 'Models the CFO may use' });
  await expect(group).toBeVisible();
  await expect(group.locator('input:not([type="checkbox"]), textarea')).toHaveCount(0);
  // The mock's Claude catalog: Opus 5.5, Sonnet 5.5, ...
  const claude = group.locator('fieldset').filter({ hasText: 'Claude' }).first();
  await claude.getByLabel('Sonnet 5.5', { exact: true }).check();
  await expect.poll(async () => (await saved(page))?.allow).toEqual([{ agent: 'claude', model: 'sonnet' }]);
  await claude.getByLabel('Any model of this agent', { exact: true }).check();
  await expect.poll(async () => (await saved(page))?.allow).toEqual([{ agent: 'claude' }]);
  await claude.getByLabel('Any model of this agent', { exact: true }).uncheck();
  await expect.poll(async () => (await saved(page))?.allow).toEqual([]);
  await sheet.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(sheet).toBeHidden();
  await expect(opener).toBeFocused();
  await expect(row).toContainText('Any installed agent, on its saved model');
});

test('Usage shows the reserve, each account\'s room and extra usage for unattended work', async ({ page }) => {
  await page.addInitScript(() => {
    // The Agents panel's fields: a Usage save must keep them (a patch).
    localStorage.setItem('mockCfoSettings', JSON.stringify({ allow: [{ agent: 'codex' }], effortMode: 'fixed', reservePercent: 20 }));
    localStorage.setItem('mockCfoAccounts', JSON.stringify([
      { agent: 'claude', account: 'a', label: 'Personal', fiveHourPercent: 40, weeklyPercent: 10, resetsAt: new Date(Date.now() + 3600_000).toISOString() },
      { agent: 'claude', account: 'b', label: 'Work', fiveHourPercent: 85, weeklyPercent: 20, resetsAt: new Date(Date.now() + 2 * 3600_000).toISOString() },
    ]));
  });
  await page.goto('/?mock=1');
  await openSection(page, 'usage');
  // The CFO rows are on Usage's Budgets tab (Settings PR 8).
  await showUsageTab(page, 'Budgets');
  const reserve = page.getByLabel('Reserve for your own prompts', { exact: true });
  await expect(reserve).toHaveAttribute('data-value', '20');
  expect(await optionLabels(reserve)).toEqual(['0%', '10%', '20%', '30%', '40%', '50%']);
  // Whole labels: hasText is a substring, and "Extra usage for unattended work" contains "work".
  const personal = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: /^Personal$/ }) });
  const work = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: /^Work$/ }) });
  await expect(personal).toContainText('Room for unattended work');
  await expect(work).toContainText('Past the reserve until');
  // A smaller reserve gives the fuller account room again.
  await choose(reserve, '10');
  await expect.poll(async () => (await saved(page))?.reservePercent).toBe(10);
  expect((await saved(page)).allow).toEqual([{ agent: 'codex' }]);
  expect((await saved(page)).effortMode).toBe('fixed');
  await expect(work).toContainText('Room for unattended work');

  const extra = page.locator('.settings-row').filter({ has: page.locator('.settings-row-k', { hasText: 'Extra usage for unattended work' }) });
  await expect(extra).toContainText('Off');
  await extra.getByRole('button', { name: 'Approve for 4 h', exact: true }).click();
  await expect(extra).toContainText('Approved until');
  expect((await saved(page)).extraUsageUntil).toBeTruthy();
  await extra.getByRole('button', { name: 'Withdraw', exact: true }).click();
  await expect(extra).toContainText('Off');
  expect((await saved(page)).extraUsageUntil).toBeUndefined();
});

test('Settings search finds the CFO rows', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  const search = page.getByRole('searchbox', { name: 'Find a setting' });
  const index = page.getByRole('navigation', { name: 'Settings sections' });
  for (const label of ['Effort', 'Models the CFO may use', 'Reserve for your own prompts', 'Extra usage for unattended work']) {
    await search.fill(label);
    await expect(index.getByRole('button', { name: label, exact: true }).and(page.locator('.settings-index-control'))).toHaveCount(1);
  }
});
