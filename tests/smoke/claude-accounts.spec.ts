import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';
import { showSettingsPart } from './settingsPart';

/**
 * Settings → Accounts → Claude: only the accounts added there (never the
 * Claude Code login) and the add form. Routing, the meters and In rotation are on
 * Settings → Usage → Limits (Settings PR 8). Tokens go in and never come back over IPC.
 */

const summaries = (page: Page) => page.locator('.usage-limits .claude-acct-routing > .claude-acct-summary');
// The first paragraph describes routing; the second describes quota failover.
const routingSummary = (page: Page) => summaries(page).first();

async function openSettingsSection(page: Page, id: string, name: string) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name, exact: true }).click();
  }
}

async function openAccounts(page: Page) {
  await openSettingsSection(page, 'accounts', 'Accounts');
  await showSettingsPart(page, 'accounts'); // a section of its own since Settings PR 3
  await expect(page.locator('.claude-accts:not(.provider-accts)')).toBeVisible();
  await expect(page.locator('.claude-accts:not(.provider-accts)')).toContainText('Limits and Spread usage are in Usage.');
}

async function openLimits(page: Page) {
  await openSettingsSection(page, 'usage', 'Usage');
  await expect(page.locator('.usage-limits')).toBeVisible();
  await expect(summaries(page)).toHaveCount(2);
}

test('accounts show usage, switch routing and add a token account', async ({ page }, info) => {
  await page.goto('/?mock=1');
  await openLimits(page);

  const meters = page.locator('.usage-limits .claude-acct:not([data-kind="login"])');
  await expect(meters).toHaveCount(2);
  await expect(meters.first().locator('.claude-acct-name')).toHaveText('Main plan');
  await expect(page.locator('.usage-limits .claude-acct[data-kind="login"]')).toHaveCount(0);
  await expect(meters.first().locator('.claude-acct-status')).toHaveText('Near its limit');
  await expect(meters.nth(1).locator('.claude-acct-next')).toHaveText('Next run');
  await expect(meters.first().locator('.usage-mark[data-mark="move"]')).toHaveCount(2);
  await expect(routingSummary(page)).toContainText('Spreading across 2 accounts');
  await info.attach('claude-accounts', { body: await page.screenshot({ fullPage: false }), contentType: 'image/png' });

  // One account: the picker pins the first added account.
  await page.getByRole('button', { name: 'One account' }).click();
  await expect(routingSummary(page)).toHaveText('Every Claude run uses Main plan.');

  // Add, in Accounts: an API key is flagged before it is sent.
  await openAccounts(page);
  const rows = page.locator('.claude-accts:not(.provider-accts) .claude-acct');
  await expect(rows).toHaveCount(2);
  await page.getByRole('button', { name: 'Add a Claude account…' }).click();
  await page.getByPlaceholder('e.g. Second Max plan').fill('Third plan');
  await page.getByPlaceholder('sk-ant-oat01-…').fill('sk-ant-api03-nope');
  await expect(page.locator('.claude-acct-add .claude-acct-hint')).toContainText('API key');
  await expect(page.getByRole('button', { name: 'Add account' })).toBeDisabled();
  await page.getByPlaceholder('sk-ant-oat01-…').fill(`sk-ant-oat01-${'x'.repeat(30)}\n  ${'y'.repeat(30)}`);
  await page.getByRole('button', { name: 'Add account' }).click();
  await expect(rows).toHaveCount(3);
  await expect(rows.nth(2).locator('.claude-acct-status')).toHaveText('Not measured yet');

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

// 2026-10-21: a claude.ai connector set to Always pins every chat to
// Claude Code's login, so Accounts must say so and route to Connectors.
test('an Always claude.ai connector shows the login row, true header and a route to Connectors', async ({ page }, info) => {
  await page.addInitScript(() => localStorage.setItem('mockConnectors', JSON.stringify([{
    id: 'superhuman_mail', name: 'Superhuman Mail', description: '', transport: 'claudeAi', command: '', args: [],
    url: 'https://mcp.mail.superhuman.com/mcp', serverId: 'mcpsrv_mock', iconUrl: '', envKeys: [], headerKeys: [],
    agents: ['claude'], chat: true, automations: false, mode: 'always', autoApprove: false, enabled: true,
    origin: 'claudeai', updatedAt: '2026-10-21T00:00:00Z',
  }])));
  await page.goto('/?mock=1');
  await openLimits(page);
  await expect(routingSummary(page)).toHaveText(
    "Chats run on Claude Code's login (you@example.com in Example Team), not on your added accounts, because Superhuman Mail is set to Always. Runs without a claude.ai connector use Second Max plan next.",
  );
  const login = page.locator('.usage-limits .claude-acct[data-kind="login"]');
  await expect(login.locator('.claude-acct-name')).toHaveText("Claude Code's login");
  await expect(login.locator('.claude-acct-next')).toHaveText('Chats run here');
  await expect(login).toContainText('you@example.com in Example Team');
  await expect(page.locator('.usage-limits .claude-acct', { hasText: 'Second Max plan' }).locator('.claude-acct-next')).toHaveText('Next run without a claude.ai connector');
  await expect(page.locator('[data-kind="step-aside"]')).toContainText('comes along only when a message needs it');
  await info.attach('claude-accounts-login-pin', { body: await page.screenshot({ fullPage: false }), contentType: 'image/png' });

  await login.getByRole('button', { name: 'Open Connectors' }).click();
  const row = page.locator('.cx-row', { hasText: 'Superhuman Mail' });
  await expect(row.locator('[data-kind="login-override"]')).toContainText('Always turns account spreading off');
  await row.getByRole('button', { name: 'Superhuman Mail settings' }).click();
  // While Always is picked the row's warning says it once; before picking it,
  // the Attach choice does.
  await expect(row.locator('[data-kind="always-spread"]')).toHaveCount(0);
  await row.getByRole('radio', { name: 'Only when relevant' }).click();
  await expect(row.locator('[data-kind="login-override"]')).toHaveCount(0);
  await expect(row.locator('[data-kind="always-spread"]')).toContainText('Always turns account spreading off for Claude');

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('every provider signs in from the same Accounts page', async ({ page }, info) => {
  await page.goto('/?mock=1');
  await openAccounts(page);
  const codex = page.locator('[data-provider="codex"]');
  await expect(codex).toContainText('ChatGPT (Codex)');
  await expect(codex).toContainText('Signed in');
  await expect(codex).toContainText('ChatGPT Pro');
  await expect(codex).toContainText('me@example.com');
  await expect(page.locator('[data-provider="grok"]')).toContainText('me@x.com');
  await expect(page.locator('[data-provider="antigravity"]')).toContainText('open Terminal, run agy');
  await expect(page.locator('[data-provider="antigravity"]').getByRole('button', { name: 'Check again' })).toBeVisible();
  await expect(page.locator('[data-provider="jev"]')).toContainText('No key');
  await expect(page.locator('[data-provider="gemini"]')).toHaveCount(0);
  await page.locator('[data-provider="grok"]').scrollIntoViewIfNeeded();
  await info.attach('accounts-page', { body: await page.locator('.provider-accts').screenshot(), contentType: 'image/png' });

  // Sign-in runs the provider's own login.
  await codex.getByRole('button', { name: 'Switch account' }).click();
  await expect.poll(() => page.evaluate(() => (window as any).__mockApiCalls?.provider_sign_in ?? 0)).toBe(1);

  // Sign-out asks first.
  await codex.getByRole('button', { name: 'Sign out' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Sign out', exact: true }).click();
  await expect(codex).toContainText('Signed out');

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
