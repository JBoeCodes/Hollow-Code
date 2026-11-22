import { expect, test, type Page } from './fixtures/test';

async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) {
        const id = next++;
        callbacks.set(id, callback);
        return id;
      },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
          return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => {
      for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload });
    };
  });
}


for (const theme of ['light', 'dark'] as const) {
  test(`context ring is compact and keeps cache separate in ${theme}`, async ({ page }, info) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await installEventBridge(page);
    await page.goto('/?mock=1&demo=1');
    await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
    await page.evaluate(async selected => {
      const themePath = '/src/store/theme.ts';
      const colors = await import(/* @vite-ignore */ themePath);
      colors.setMatchSystem(false);
      colors.setTheme(selected);
      document.documentElement.dataset.theme = selected;
      (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', { type: 'usage', usage: { costUsd: 0.24, costStatus: 'estimated', billing: 'subscription', inputTokens: 7400, cachedInputTokens: 767200, outputTokens: 1100, model: 'opus[1m]', contextTokens: 389300, contextWindow: 1000000 } });
      localStorage.setItem('mockRunContextManifest', JSON.stringify({ runId: 'context-capture', manifest: { layers: [
        { name: 'request', bytes: 7200 }, { name: 'workspace', bytes: 400 }, { name: 'operatorProfile', bytes: 100 }, { name: 'policy', bytes: 2300 },
      ] } }));
    }, theme);
    await expect(page.locator('html')).toHaveAttribute('data-theme', theme);
    await page.locator('.usage-menu-trigger').click();
    const card = page.getByRole('dialog', { name: 'Usage', exact: true });
    await expect(card).toContainText('389.3k/1M');
    await expect(card.getByRole('heading', { name: 'Last turn', exact: true })).toHaveCount(1);
    await expect(card.locator('dt')).not.toContainText(['Cached input']);
    await card.locator('summary').filter({ hasText: "What's using it" }).click();
    await expect(card.locator('.usage-menu-split')).toHaveText('Conversation 72% · Project notes 4% · Your profile 1% · Instructions 23%');
    await expect(card).not.toContainText('WORKSPACE.md');
    await page.screenshot({ path: info.outputPath(`ring-${theme}.png`) });
    await card.locator('summary').filter({ hasText: /^Cache$/ }).click();
    await expect(card).toContainText('767.2k');
    await expect(card).toContainText('This total can exceed the context window.');
    await page.screenshot({ path: info.outputPath(`ring-cache-${theme}.png`) });
    await page.keyboard.press('Escape');
    await expect(card).toHaveCount(0);
    await expect(page.locator('.usage-menu-trigger')).toBeFocused();
    await page.locator('.usage-menu-trigger').click();
    await page.setViewportSize({ width: 720, height: 480 });
    await expect(card).toHaveCount(0);
  });
}

test('missing context measurements show unavailable without fabricated percentages', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockRunContextManifest', 'null'));
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await page.locator('.usage-menu-trigger').click();
  const card = page.getByRole('dialog', { name: 'Usage', exact: true });
  await card.locator('summary').filter({ hasText: "What's using it" }).click();
  await expect(card).toContainText('Breakdown unavailable');
  await expect(card.locator('.usage-menu-split')).toHaveCount(0);
});

test('the ring links to detailed tool records in Settings', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await page.locator('.usage-menu-trigger').click();
  const card = page.getByRole('dialog', { name: 'Usage', exact: true });
  await card.locator('summary').filter({ hasText: "What's using it" }).click();
  await card.getByRole('button', { name: 'Context details', exact: true }).click();
  await page.getByText('Conversation context details', { exact: true }).click();
  await expect(page.getByText('Turn 2 · Read · chat.rs', { exact: true })).toBeVisible();
});

test('the open Usage card closes when the pane changes width without a window resize (ENG-93)', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await page.locator('.usage-menu-trigger').click();
  const card = page.getByRole('dialog', { name: 'Usage', exact: true });
  await expect(card).toBeVisible();
  // A height-only change (the composer growing, a banner) keeps it open.
  const shrunk = await page.locator('.chat-main').first().evaluate(el => {
    const box = el as HTMLElement, before = box.getBoundingClientRect();
    box.style.maxHeight = `${Math.round(before.height - 40)}px`;
    const after = box.getBoundingClientRect();
    return after.height < before.height && after.width === before.width;
  });
  expect(shrunk).toBe(true);
  await page.waitForTimeout(150);
  await expect(card).toBeVisible();
  // A sidebar or workspace panel toggle resizes the pane like this; the window does not.
  await page.locator('.chat-main').first().evaluate(el => { (el as HTMLElement).style.maxWidth = '640px'; });
  await expect(card).toHaveCount(0);
});
