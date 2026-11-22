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

test('recall lives in the side panel card and feedback drafts a correction', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  const composer = page.getByLabel('Chat message', { exact: true });
  await expect(composer).toBeVisible();
  await page.evaluate(async () => {
    const runtimePath = '/src/store/chatRuntime.ts';
    const runtime = await import(/* @vite-ignore */ runtimePath);
    runtime.setActiveChatRecall([{
      subject: 'Status updates', statement: 'Keep status updates direct.',
      path: 'learnings/status.md', targetPath: '*', score: 8,
    }]);
  });
  const card = page.locator('#side-panel-card');
  await card.getByRole('button', { name: 'Recall 1', exact: true }).click();
  await card.getByRole('button', { name: 'Status updates', exact: true }).click();
  await expect(card).toContainText('Keep status updates direct.');
  await expect(card.getByRole('button', { name: 'Not relevant', exact: true })).toBeVisible();
  await expect(card.getByRole('button', { name: 'Outdated', exact: true })).toBeVisible();
  await card.getByRole('button', { name: 'Not relevant', exact: true }).click();
  await expect(card).toBeVisible();
  await expect(composer).toHaveValue(/Teach Hollow: the accepted learning/);
  const conversation = page.locator('.chat-main');
  await expect(conversation.locator('.execution-receipt').getByText('Recall used', { exact: true })).toHaveCount(0);
  await expect(conversation.getByText('Recall used', { exact: true })).toHaveCount(0);
});

test('after the turn ends the card still lists the newest reply’s recall, and the reply shows none', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await installEventBridge(page);
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await expect(page.getByLabel('Chat message', { exact: true })).toBeVisible();
  await page.evaluate(async () => {
    const runtimePath = '/src/store/chatRuntime.ts';
    const runtime = await import(/* @vite-ignore */ runtimePath);
    runtime.setActiveChatRecall([]);
    runtime.setChatTurns([
      { role: 'user', text: 'Update the team', tools: [] },
      { role: 'assistant', text: 'Sent.', tools: [], receipt: {
        status: 'unverified', ordering: 'known', files: [], checks: [], failures: [], eventCount: 0,
        recall: [{ subject: 'Status updates', statement: 'Keep status updates direct.', path: 'learnings/status.md', targetPath: '*', score: 8 }],
      } },
    ]);
  });
  const card = page.locator('#side-panel-card');
  await card.getByRole('button', { name: 'Recall 1', exact: true }).click();
  await card.getByRole('button', { name: 'Status updates', exact: true }).click();
  await expect(card).toContainText('Keep status updates direct.');
  await expect(page.locator('.chat-main').getByText('Recall used', { exact: true })).toHaveCount(0);
});
