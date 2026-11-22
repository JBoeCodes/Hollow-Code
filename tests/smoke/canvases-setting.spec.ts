import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Canvas C3 wave 2 C4: Settings → Chat → Canvases (Off / When it helps /
 * Most replies), a chat's own choice in the model menu that is sent with its
 * turns and travels with the conversation, and the receipt's canvas count
 * and bytes in the reply footer. Commands go through the `?mock=1` IPC
 * (`get_agent_settings` / `set_agent_settings` keep `mockAgentSettings`).
 */

const narrow = (page: Page) => (page.viewportSize()?.width ?? 1000) <= 860;

async function openSettingsSection(page: Page, id: string, label: RegExp) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  else await page.locator('.settings-index').getByRole('button', { name: label }).click();
}

async function openChat(page: Page) {
  await openSettingsSection(page, 'advanced', /^Advanced/);
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

const saved = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockAgentSettings') ?? '{}'));
const canvasesGroup = (page: Page) => page.getByRole('group', { name: 'Canvases', exact: true });

/** Tauri event delivery for `listen()`, as in turn-footer.spec.ts. */
async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
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
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
  }, payload);
}

test('Settings → Chat → Canvases saves at once and survives a reload', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSettingsSection(page, 'chat', /^Chat/); // Canvases sits in Chat
  const group = canvasesGroup(page);
  await expect(group.getByRole('button')).toHaveText(['Off', 'When it helps', 'Most replies']);
  await expect(group.getByRole('button', { name: 'When it helps' })).toHaveAttribute('aria-pressed', 'true');

  await group.getByRole('button', { name: 'Most replies' }).click();
  await expect.poll(async () => (await saved(page)).canvases).toBe('most');
  await expect(group.getByRole('button', { name: 'Most replies' })).toHaveAttribute('aria-pressed', 'true');

  await page.reload();
  await openSettingsSection(page, 'chat', /^Chat/); // Canvases sits in Chat
  await expect(canvasesGroup(page).getByRole('button', { name: 'Most replies' })).toHaveAttribute('aria-pressed', 'true');
  // The default is never stored.
  await canvasesGroup(page).getByRole('button', { name: 'When it helps' }).click();
  await expect.poll(async () => 'canvases' in (await saved(page))).toBe(false);

  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('a chat chooses its own Canvases in the model menu; it is sent with the turn and travels with the chat', async ({ page }) => {
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockAgentSettings', JSON.stringify({ defaultAgent: null, models: {}, canvases: 'off' })));
  await page.reload();
  await openChat(page);

  const trigger = page.getByRole('button', { name: /^Model:/ });
  await trigger.click();
  const row = page.getByRole('button', { name: /^Canvases/ });
  await expect(row).toContainText('Off · setting');
  await row.click();
  const list = page.getByRole('listbox', { name: 'Canvases in this chat' });
  await expect(list.getByRole('option')).toHaveText([/Use the setting \(Off\)/, /^Off/, /When it helps/, /Most replies/]);
  await expect(list.getByRole('option', { selected: true })).toHaveText(/Use the setting/);
  await list.getByRole('option', { name: /Most replies/ }).click();
  await expect(row).toContainText('Most replies');
  await page.keyboard.press('Escape');

  // The choice rides with the turn; no setting was saved.
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Where does the launch stand?');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').canvases)).toBe('most');
  expect((await saved(page)).canvases).toBe('off');
  const key = await page.evaluate(() => localStorage.getItem('chatAgentSessionKey'));
  expect(key).toBeTruthy();
  expect(await page.evaluate(k => localStorage.getItem(`hollow.canvases.${k}`), key)).toBe('most');

  // Another conversation follows the setting; coming back brings the choice back.
  const runtime = '/src/store/runtime.ts';
  const choiceAfter = (next: string | null) => page.evaluate(async ([url, k]) => {
    const store = await import(url!);
    store.setChatAgentSessionKeyPersistent(k);
    return store.chatCanvases();
  }, [runtime, next] as const);
  expect(await choiceAfter('s-20261005-other001')).toBeNull();
  expect(await choiceAfter(key)).toBe('most');

  // Finish the turn, then a new chat follows the setting again.
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-canvas-1', sessionKey: key, recall: [], clientTurnId });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: key, clientTurnId });
  await expect(page.getByRole('button', { name: 'Send', exact: true })).toBeVisible();
  await composer.fill('/new');
  await composer.press('Meta+Enter');
  await trigger.click();
  await expect(page.getByRole('button', { name: /^Canvases/ })).toContainText('Off · setting');
  await page.keyboard.press('Escape');

  // A reload of that conversation keeps its own choice.
  await page.evaluate(k => localStorage.setItem('chatAgentSessionKey', k!), key);
  await page.reload();
  expect(await page.evaluate(async url => (await import(url)).chatCanvases(), runtime)).toBe('most');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('the reply footer and receipt show the canvases a turn drew', async ({ page }) => {
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Draw the status');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId)).toBeTruthy();
  const { clientTurnId, key } = await page.evaluate(() => ({
    clientTurnId: JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId,
    key: localStorage.getItem('chatAgentSessionKey'),
  }));
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-canvas-2', sessionKey: key, recall: [], clientTurnId });
  await emitChatEvent(page, { type: 'assistantText', text: 'The launch is on track: 41 of 45 checks pass.', clientTurnId });
  const now = new Date().toISOString();
  await emitChatEvent(page, {
    type: 'evidenceReceipt', clientTurnId,
    receipt: {
      id: 'receipt-canvas-2', runId: 'run-canvas-2', source: 'chat', sourceId: null, startedAt: now, finishedAt: now,
      status: 'success', verificationState: 'noChanges', files: [], checks: [], failures: [], historyRunId: 'run-canvas-2',
      sessionKey: key, canvases: { count: 2, bytes: 14_540 },
    },
  });
  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: key, clientTurnId });
  const footer = page.locator('.chat-main .turn-footer').last();
  await expect(footer.locator('.turn-footer-time')).toContainText('2 canvases · 14 KB');
  await footer.getByRole('button', { name: 'More actions' }).click();
  await page.locator('.ctx-item').filter({ hasText: 'Show run record' }).click();
  const receipt = page.locator('.chat-main .execution-receipt');
  await expect(receipt).toHaveCount(1);
  if (!(await receipt.evaluate(el => (el as HTMLDetailsElement).open))) await receipt.locator('summary').click();
  await expect(receipt.locator('.execution-receipt-canvases')).toHaveText(/Canvases · 2 canvases · 14 KB/);
});
