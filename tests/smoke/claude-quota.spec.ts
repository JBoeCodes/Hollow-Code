import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Settings → Usage "Claude plan" card (Paperclip PR 8c). The backend may read
 * the Keychain, so nothing may call `claude_quota` before the user opts in;
 * a null answer hides the card entirely.
 */

async function openUsage(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'usage');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: /Usage/ }).click();
  }
  await expect(page.locator('.usage-panel')).toBeVisible();
}

const quotaCalls = (page: Page) =>
  page.evaluate(() => window.__mockApiCalls?.claude_quota ?? 0);

test('the Claude plan card asks nothing before opt-in, then shows both windows', async ({ page }) => {
  await page.goto('/?mock=1');
  await openUsage(page);

  // Settings PR 8: the opt-in is the Claude row's "Read live from Claude" switch on Usage, Limits.
  const live = page.getByRole('switch', { name: 'Read live from Claude' });
  await expect(live).not.toBeChecked();
  await expect(page.locator('.claude-quota-card')).toHaveCount(0);
  expect(await quotaCalls(page)).toBe(0);

  await live.click();
  const card = page.locator('.claude-quota-card');
  await expect(card).toBeVisible();
  await expect(card.locator('.claude-quota-window')).toHaveCount(2);
  await expect(card.locator('[data-window="five_hour"] .claude-quota-percent')).toHaveText('101% used');
  await expect(card.locator('[data-window="five_hour"]')).toHaveAttribute('data-level', 'over');
  await expect(card.locator('[data-window="seven_day"] .claude-quota-percent')).toHaveText('30% used');
  await expect(page.locator('.claude-quota-optin')).toHaveCount(0);
  expect(await quotaCalls(page)).toBe(1);

  // The opt-in is remembered on this machine.
  expect(await page.evaluate(() => localStorage.getItem('claudeQuotaOptIn'))).toBe('1');

  // Refresh asks again and keeps the card.
  await card.getByRole('button', { name: 'Refresh' }).click();
  await expect.poll(() => quotaCalls(page)).toBe(2);
  await expect(card.locator('.claude-quota-window')).toHaveCount(2);

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a null answer hides the Claude plan card', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('claudeQuotaOptIn', '1');
    localStorage.setItem('mockClaudeQuota', 'none');
  });
  await page.goto('/?mock=1');
  await openUsage(page);

  await expect.poll(() => quotaCalls(page)).toBe(1);
  await expect(page.locator('.claude-quota-card')).toHaveCount(0);
  await expect(page.locator('.claude-quota-optin')).toHaveCount(0);
  await expect(page.locator('.usage-panel')).not.toContainText('Checking Claude plan usage');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
