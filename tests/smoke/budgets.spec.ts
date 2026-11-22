import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { openReviews, waitingPanel } from './waiting';

/**
 * Usage budgets (Paperclip PR 8a) in the mock harness: Settings → Usage lists
 * and saves budgets, the pause card (Automations → Budget pauses opens it in
 * Waiting on you) is gated by "above observed",
 * "Keep paused" rejects, and a budget-held automation says so.
 */

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

async function openUsage(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'usage');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: /Usage/ }).click();
  }
  await page.getByRole('tab', { name: 'Budgets', exact: true }).click();
  await expect(page.locator('.budgets-panel')).toBeVisible();
}

async function openPauses(page: Page) {
  return openReviews(page, 'budgets');
}

const misses = (page: Page) => page.evaluate(() => window.__mockApiMisses ?? []);

test('Settings → Usage lists budgets, offers a suggestion after a week, and saves one', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);
  const panel = page.locator('.budgets-panel');
  const row = panel.locator('.budgets-row').first();
  await expect(row).toContainText('the “morning-brief” automation');
  await expect(row.locator('.budgets-chip')).toHaveText('Paused');
  await expect(row).toContainText('1.2k tokens of 1k tokens');
  await expect(panel).toContainText('Ordinary chat only warns');
  await expect(panel).toContainText('Hard stop applies to automations:');
  await expect(panel).not.toContainText('company');

  // Edit shows "Suggest a limit" (9 days of fixture history).
  await row.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(panel.getByRole('button', { name: 'Suggest a limit' })).toBeVisible();
  await panel.getByRole('button', { name: 'Cancel', exact: true }).click();

  // A spending budget on Codex can never trip: its option is disabled.
  await panel.getByRole('button', { name: 'Add budget' }).click();
  const form = panel.locator('.budgets-form');
  await choose(form.getByLabel('Applies to'), 'agent:codex');
  // Playwright 1.61's toBeDisabled reads an <option> as enabled even with the
  // attribute set, so assert the DOM property.
  await form.getByLabel('Measure').click();
  await expect(page.locator('.hs-pop [role=option][data-value="costUsd"]')).toHaveAttribute('aria-disabled', 'true');
  await page.keyboard.press('Escape');
  await choose(form.getByLabel('Applies to'), 'global');
  await form.getByLabel('Limit', { exact: true }).fill('50000');
  await form.getByRole('button', { name: 'Save budget' }).click();
  await expect(panel.locator('.budgets-row')).toHaveCount(2);
  await expect(panel.locator('.budgets-row').filter({ hasText: 'all usage' })).toBeVisible();

  // Resume… is gated by "above this window's usage".
  await row.getByRole('button', { name: 'Resume…' }).click();
  const input = row.getByLabel('New limit');
  await input.fill('1200');
  await expect(row.getByRole('button', { name: 'Raise and resume' })).toBeDisabled();
  await input.fill('2000');
  await row.getByRole('button', { name: 'Raise and resume' }).click();
  await expect(row.locator('.budgets-chip')).not.toHaveText('Paused');

  expect(await misses(page)).toEqual([]);
});

test('a hard-stop budget over its limit with no pause card yet still offers Resume…', async ({ page }) => {
  // Usage from chat or ingest crossed the limit before any claim opened a
  // pause: automations are held, so Settings must offer the raise.
  await page.addInitScript(() => localStorage.setItem('mockBudgetPaused', '[]'));
  await page.goto('/?mock=1');
  await openUsage(page);
  const row = page.locator('.budgets-row').first();
  await expect(row.locator('.budgets-chip')).toHaveText('Over limit');
  await expect(row.getByRole('button', { name: 'Resume…' })).toBeVisible();
  expect(await misses(page)).toEqual([]);
});

test('without a week of data there is no suggestion', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockLedgerDays', '3'));
  await page.goto('/?mock=1');
  await openUsage(page);
  const row = page.locator('.budgets-row').first();
  await row.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Suggest a limit' })).toHaveCount(0);
});

test('the pause card raises only above observed usage, and Keep paused rejects', async ({ page }) => {
  await page.goto('/?mock=1');
  const panel = await openPauses(page);
  const card = panel.locator('.review-inbox-card').filter({ hasText: 'Budget pause' });
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('the “morning-brief” automation');
  await expect(card).toContainText('1.2k tokens of 1k tokens');
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toHaveCount(0);
  await expect(card.getByRole('button', { name: 'Tomorrow', exact: true })).toHaveCount(0);

  const input = card.getByLabel('New limit');
  await expect(input).toHaveValue('1800'); // the suggestion
  await input.fill('1200');
  await expect(card.getByRole('button', { name: 'Raise and resume' })).toBeDisabled();
  await input.fill('1500');
  await card.getByRole('button', { name: 'Raise and resume' }).click();
  await expect(card).toHaveCount(0);
  await expect(panel).toContainText('No budget pauses are waiting.');
  expect(await page.evaluate(() => window.__mockApiCalls?.resolve_review_item ?? 0)).toBe(1);
  expect(await misses(page)).toEqual([]);
});

test('Keep paused rejects the pause card', async ({ page }) => {
  await page.goto('/?mock=1');
  const panel = await openPauses(page);
  const card = panel.locator('.review-inbox-card').filter({ hasText: 'Budget pause' });
  await card.getByRole('button', { name: 'Keep paused' }).click();
  await expect(card).toHaveCount(0);
  expect(await page.evaluate(() => {
    const items = JSON.parse(localStorage.getItem('mockReviewItems') ?? '[]') as Array<{ id: string; status: string }>;
    return items.find(item => item.id === 'review-budget-pause-1')?.status;
  })).toBe('rejected');
});

test('a budget-held automation shows the chip and a paused next run', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockBudgetHeld', JSON.stringify(['morning-brief'])));
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  const row = page.locator('.workflow-row').filter({ has: page.locator('.workflow-budget-chip') });
  await expect(row.first()).toBeVisible();
  await expect(row.first()).toContainText('Budget paused');
  expect(await misses(page)).toEqual([]);
});

test('a run a budget stopped reads as a budget stop, not a failure (Paperclip PR 8b)', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockBudgetStopped', JSON.stringify(['morning-brief'])));
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  const row = page.locator('.workflow-row').filter({ hasText: 'Morning Brief' });
  await expect(row.locator('.status-conduit')).toHaveAttribute('data-state', 'warn');
  await row.click();
  const notice = page.locator('.workflow-budget-stop');
  await expect(notice).toContainText('Stopped at a budget limit');
  await expect(notice).toContainText('Budget limit reached:');
  await expect(page.locator('.workflow-debugger')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Retry now' })).toHaveCount(0);
  await notice.getByRole('button', { name: 'Review budget pauses' }).click();
  await expect(waitingPanel(page)).toHaveAttribute('data-filter', 'budgets');
  await expect(waitingPanel(page).locator('.review-inbox-card').filter({ hasText: 'Budget pause' })).toHaveCount(1);
  expect(await misses(page)).toEqual([]);
});
