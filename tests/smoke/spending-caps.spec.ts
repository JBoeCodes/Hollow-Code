import { expect, test, type Page } from './fixtures/test';
import { showUsageTab } from './usageTab';

/**
 * P1-07: the caps Hollow keeps for you have one list and editor, Settings →
 * Usage → Budgets; Automation links there; the Main agent header says the
 * day's spend in one line with a way to the cap. Against the mock.
 */

const STATUS = {
  spentTodayUsd: 12.4, askedTodayUsd: 30, wakeSkipped: null, dailyCapUsd: 100,
  checkIns: false, lastCheckIn: null, nextCheckIn: null, checkInsToday: 0,
  suggestions: { answered: 0, taken: 0, declinedForGood: 0, takenShare: null }, actOnOwn: true,
};
const SETTINGS = { checkIns: false, intervalMin: 30, startHour: 8, endHour: 18, weekdaysOnly: true, dailyCapUsd: 100, actOnOwn: true, swarmMax: 25, swarmModel: 'claude-haiku-5-5', swarmConcurrency: 6 };

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-desktop', 'list states and links, not layout: one size covers them');
  await page.addInitScript(([status, settings]) => {
    localStorage.setItem('mockMainAgentStatus', JSON.stringify(status));
    if (!localStorage.getItem('mockMainAgentSettings')) localStorage.setItem('mockMainAgentSettings', JSON.stringify(settings));
    localStorage.setItem('mockBudgets', '[]');
  }, [STATUS, SETTINGS] as const);
});

async function openSection(page: Page, name: RegExp) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name }).click();
}

const capRow = (page: Page, id: string) => page.locator(`[data-cap="${id}"]`);

test('with no budgets of your own, Budgets lists the caps Hollow keeps with their spend', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, /^Usage/);
  // Settings PR 8: the caps sit on Usage, Limits; the full list on the Budgets tab.
  await expect(capRow(page, 'main-agent')).toContainText('COO daily hard stop');
  await expect(capRow(page, 'main-agent')).toContainText('$12.40 of $100 today');
  await expect(capRow(page, 'overnight')).toContainText('Overnight cap');
  await expect(capRow(page, 'overnight')).toContainText(/Current limit \$\d/);
  await expect(page.locator('.usage-limits-budgets')).toContainText('No budgets of your own.');
  await showUsageTab(page, 'Budgets');
  const panel = page.locator('.budgets-panel');
  await expect(panel.locator('.budgets-empty')).toContainText('No budgets of your own.');
  await expect(capRow(page, 'main-agent')).toHaveCount(0);
});

test('changing the COO cap in Budgets saves the COO setting', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, /^Usage/);
  const row = capRow(page, 'main-agent');
  await row.getByRole('button', { name: 'Change limit' }).click();
  await row.getByLabel('COO daily hard stop (USD)').fill('lots');
  await row.getByRole('button', { name: 'Save' }).click();
  await expect(row.getByRole('alert')).toContainText('Type a dollar amount');
  await row.getByLabel('COO daily hard stop (USD)').fill('25');
  await row.getByRole('button', { name: 'Save' }).click();
  await expect(row).toContainText('Limit $25');
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('mockMainAgentSettings')!).dailyCapUsd);
  expect(saved).toBe(25);
});

test('Automation has no cap inputs: its links open the Budgets row', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, /^Background work/);
  await expect(page.getByRole('spinbutton', { name: 'Daily hard stop in dollars' })).toHaveCount(0);
  await expect(page.getByRole('textbox', { name: 'Overnight cap' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Set in Usage → Budgets' })).toHaveCount(1); // the Automations page keeps its own link (Settings PR 4)
  await page.locator('.settings-row', { hasText: 'Daily hard stop' }).getByRole('button', { name: 'Set in Usage → Budgets' }).click();
  await expect(capRow(page, 'main-agent')).toBeVisible();
  await expect(capRow(page, 'main-agent')).toHaveClass(/settings-found/);
});

test('the COO day is in Settings → Background work and its top bar menu links to the hard stop', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ }).click();
  await expect(page.locator('.work-conversation')).toBeVisible();
  await page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true }).click();
  await page.getByRole('menuitem', { name: /^(Check-in settings|Turn check-ins on in Settings)$/ }).click();
  await expect(page.getByText('Today $42.40 · $12.40 on its own of $100 · $30.00 you asked for')).toBeVisible();
  await page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ }).click();
  await page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Change hard stop', exact: true }).click();
  await expect(capRow(page, 'main-agent')).toBeVisible();
});
