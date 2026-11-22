import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Connector capability and repair (next-level P16): one vocabulary, what
 * was tested, as whom and when, and a repair chosen by state. `?mock=1`
 * stamps claude.ai checks with `mockClaudeAiCheckLogin` (default: the
 * library's own login, you@example.com in Example Team). */
async function openConnectors(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.locator('.settings-index').getByRole('button', { name: 'Connectors', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your connectors' })).toBeVisible();
}

test('a claude.ai check names its login and time, and another login does not count', async ({ page }) => {
  await page.goto('/?mock=1');
  await openConnectors(page);
  await page.getByRole('button', { name: 'Add a connector…', exact: true }).click();
  await page.locator('.cx-gallery').getByRole('button', { name: 'Add Superhuman Mail', exact: true }).click();
  const row = page.locator('.cx-row').filter({ hasText: 'mcp.mail.superhuman.com' });
  await expect(row.locator('.cx-pill')).toHaveText('Available');
  await expect(row).toContainText('checked just now as you@example.com in Example Team');
  await expect(row).toContainText('Claude · chats');
  await expect(row).toContainText('claude.ai sign-in');

  // The same check, run under another claude.ai login, certifies nothing here.
  await page.evaluate(() => localStorage.setItem('mockClaudeAiCheckLogin', 'someone@else.com'));
  await page.locator('.cx-block-head').getByRole('button', { name: 'Check again' }).first().click();
  await expect(row.locator('.cx-pill')).toHaveText('Configured, not checked');
  await expect(row).toContainText('Last checked as someone@else.com, not you@example.com in Example Team');

  // The row's own repair re-checks; the result is kept with its time and login.
  await page.evaluate(() => localStorage.removeItem('mockClaudeAiCheckLogin'));
  await row.locator('.cx-row-main').getByRole('button', { name: 'Check again' }).click();
  await expect(row.locator('.cx-pill')).toHaveText('Available');
  const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('hollow.connectorChecks.v1') ?? '{}'));
  expect(stored.statuses.superhuman_mail).toMatchObject({ state: 'connected', login: 'you@example.com in Example Team' });
});

test('a calendar-only Microsoft 365 sign-in is Permission missing, with Grant file access', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => { localStorage.setItem('mockBuiltins', '1'); localStorage.setItem('mockM365', 'you@example.com'); localStorage.setItem('mockM365CalendarOnly', '1'); });
  await page.reload();
  await openConnectors(page);
  const m365 = page.locator('.cx-row').filter({ hasText: 'Microsoft 365' });
  await expect(m365.locator('.cx-pill')).toHaveText('Permission missing');
  await expect(m365).toContainText('Files.Read.All and Sites.Read.All');
  await m365.getByRole('button', { name: 'Grant file access' }).click();
  await expect(m365.locator('.cx-pill')).toHaveText('Available');
  await expect(m365).toContainText('as you@example.com');
});
