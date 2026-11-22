import { test, expect } from './fixtures/test';

async function usageHarness(page: import('@playwright/test').Page, deferred = false, limits = false) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    await route.fulfill({ response, body: `${await response.text()}
const originalProviderUsage = mockInvoke;
mockInvoke = async (command, args) => {
  if (command === 'provider_accounts') {
    const value = [{ id: 'codex', label: 'Codex', installed: true, signedIn: true,
      email: 'fixture@example.test', plan: 'Fixture plan', note: null,
      signIn: 'browser', canSignOut: true,
      limits: ${limits} ? { observedAt: new Date().toISOString(), windows: [
        { id: 'primary', label: '5h limit', usedPercent: 23.5, resetsAt: null },
        { id: 'secondary', label: '7d limit', usedPercent: 101, resetsAt: null }
      ] } : null }];
    if (${deferred}) return new Promise(resolve => { window.__resolveProviderUsage = () => resolve(value); });
    return value;
  }
  return originalProviderUsage(command, args);
};` });
  });
  await page.goto('/?mock=1&demo=1');
  // Mount the real lazy component with the real store; no provider is called.
  await page.evaluate(async () => {
    const { render } = await import('/node_modules/.vite/deps/solid-js_web.js');
    const store = await import('/src/store.ts');
    const { ChatUsageMenu } = await import('/src/components/ChatUsageMenu.tsx');
    store.setChatAgentPersistent('codex');
    const host = document.createElement('div');
    host.className = 'chat-composer';
    host.id = 'provider-usage-fixture';
    host.style.cssText = 'position:fixed;bottom:30px;left:24px;width:380px;z-index:9999';
    document.body.append(host);
    render(() => ChatUsageMenu(), host);
  });
  await page.locator('#provider-usage-fixture .usage-menu-trigger').click();
}

test('fresh Codex context waits for a report and missing limits stay unknown', async ({ page }) => {
  await usageHarness(page);
  const pop = page.locator('#provider-usage-fixture').getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText('Shown after the next turn finishes.');
  await expect(pop).not.toContainText("doesn't report how full");
  await expect(pop).toContainText("Codex didn't report limits");
});

test('switching away drops pending Codex account and plan reads', async ({ page }) => {
  await usageHarness(page, true);
  await page.waitForFunction(() => typeof (window as any).__resolveProviderUsage === 'function');
  await page.evaluate(async () => {
    const store = await import('/src/store.ts');
    store.setChatAgentPersistent('antigravity');
    (window as any).__resolveProviderUsage();
  });
  const pop = page.locator('#provider-usage-fixture').getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText("Antigravity doesn't report");
  await expect(pop.locator('dt').filter({ hasText: /^Account$|^Plan$/ })).toHaveCount(0);
  await expect(pop).not.toContainText('fixture@example.test');
});

test('Codex plan meters share the used convention in chat and Settings', async ({ page }) => {
  await usageHarness(page, false, true);
  const pop = page.locator('#provider-usage-fixture').getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText('24% used');
  await expect(pop).toContainText('101% used');
  await expect(pop.getByRole('meter', { name: '5h limit' })).toHaveAttribute('aria-valuenow', '23.5');
  await expect(pop.getByRole('meter', { name: '7d limit' })).toHaveAttribute('aria-valuenow', '100');
  await page.evaluate(async () => {
    const { render } = await import('/node_modules/.vite/deps/solid-js_web.js');
    const { ProviderQuotaCard } = await import('/src/settings/ProviderQuotaCard.tsx');
    const host = document.createElement('div');
    host.id = 'provider-plan-fixture';
    document.body.append(host);
    render(() => ProviderQuotaCard(), host);
  });
  const card = page.locator('#provider-plan-fixture');
  await expect(card).toContainText('24% used');
  await expect(card).toContainText('101% used');
});

test('closing and reopening clears a saved provider while the next read waits', async ({ page }) => {
  await usageHarness(page, true);
  await page.waitForFunction(() => typeof (window as any).__resolveProviderUsage === 'function');
  await page.evaluate(() => (window as any).__resolveProviderUsage());
  const pop = page.locator('#provider-usage-fixture').getByRole('dialog', { name: 'Usage' });
  await expect(pop).toContainText('fixture@example.test');
  await page.keyboard.press('Escape');
  await expect(pop).toHaveCount(0);
  await page.locator('#provider-usage-fixture .usage-menu-trigger').click();
  await expect(pop).not.toContainText('fixture@example.test');
});

test('opting into Claude limits preserves an open context breakdown', async ({ page }) => {
  await usageHarness(page);
  await page.evaluate(async () => {
    const { api } = await import('/src/api.ts');
    const store = await import('/src/store.ts');
    api.claudeAccounts = async () => ({ nextAccount: null, accounts: [] });
    store.setChatAgentPersistent('claude');
    store.setChatAgentSessionKeyPersistent('s-20261024-fixture1');
  });
  const pop = page.locator('#provider-usage-fixture').getByRole('dialog', { name: 'Usage' });
  // Switching the chat's session closes an open card (7b25db7f1), so reopen it for the new chat.
  await expect(pop).toHaveCount(0);
  await page.locator('#provider-usage-fixture .usage-menu-trigger').click();
  const details = pop.locator('details').filter({ has: page.locator('summary', { hasText: "What's using it" }) });
  await details.locator('summary').click();
  await expect(details).not.toContainText('Reading…');
  const before = await details.textContent();
  await pop.getByRole('button', { name: 'Show plan usage', exact: true }).click();
  await expect(details).toHaveAttribute('open', '');
  await expect(details).toHaveText(before!);
});
