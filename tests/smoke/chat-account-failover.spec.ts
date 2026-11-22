import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** R1, chat quota failover: when a turn's Claude account runs out, the
 * backend resumes the same conversation on another account and emits
 * `accountSwitched`. The transcript shows one quiet row, "Switched to Tech
 * (Admin hit its limit)", between the two halves of the reply; Settings →
 * Accounts holds the toggle. Events come through a Tauri event bridge (as in
 * turn-footer.spec.ts); commands go through the `?mock=1` IPC. */

/** The key the mocked `chat_send` accepts a new conversation's turn under. */
const MOCK_KEY = 's-20260925-mock0001';

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

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

function emitChatEvent(page: Page, payload: Record<string, unknown>) {
  return page.evaluate(event => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent('chat-event', event);
  }, payload);
}

function narrow(page: Page) {
  return (page.viewportSize()?.width ?? 1000) <= 860;
}

async function openSettingsSection(page: Page, id: string, label: RegExp) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: label }).click();
  }
}

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await openSettingsSection(page, 'advanced', /^Advanced/);
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

test('a turn that fails over shows one quiet notice between its halves', async ({ page }, info) => {
  await installEventBridge(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Finish the plan');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);
  const tag = { clientTurnId, sessionKey: MOCK_KEY };

  await emitChatEvent(page, { type: 'runStarted', runId: 'run-r1-1', recall: [], ...tag });
  await emitChatEvent(page, { type: 'assistantDelta', text: 'Reading the plan first.', ...tag });
  await emitChatEvent(page, { type: 'accountSwitched', from: 'Admin', to: 'Tech', ...tag });
  await emitChatEvent(page, { type: 'assistantText', text: 'The plan is done.', ...tag });

  const notice = page.locator('.chat-main .turn.system.turn-notice');
  await expect(notice).toHaveCount(1);
  await expect(notice).toHaveText('Switched to Tech (Admin hit its limit)');
  await expect(notice).toHaveAttribute('data-notice', 'accountSwitch');
  // Quiet: no "system" kicker, unlike every other system row.
  await expect(notice.locator('.chat-system-kicker')).toHaveCount(0);
  // The dead account's half stops streaming; the resumed half is its own bubble.
  const assistants = page.locator('.chat-main .turn.assistant');
  await expect(assistants).toHaveCount(2);
  await expect(assistants.first()).not.toHaveClass(/partial/);
  await expect(assistants.nth(1)).toContainText('The plan is done.');

  await emitChatEvent(page, { type: 'done', sessionId: 'id-fo-1', ...tag });
  await expect(notice).toHaveText('Switched to Tech (Admin hit its limit)');
  await page.screenshot({ path: info.outputPath('account-failover-notice.png') });
  await info.attach('account-failover-notice', { body: await page.locator('.chat-main').screenshot(), contentType: 'image/png' });
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
