import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/**
 * Cua CU8: a chat that has used the desktop tools gets a composer chip with
 * the person's controls, and a Computer section on the side-panel card with
 * the latest frame, the recent actions and the same controls. Mock only:
 * `computer_chat_state`, `computer_chat_frame` and `computer_chat_control` in
 * src/mockApi.ts (a mock desktop with two actions).
 */

/** Tauri event delivery on top of the `?mock=1` IPC (as chat-video.spec.ts). */
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

const emit = (page: Page, payload: Record<string, unknown>) => page.evaluate(event => {
  (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void }).__emitTauriEvent('chat-event', event);
}, payload);

async function openWorkspace(page: Page) {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Use the desktop.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Desktop');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Desktop');
}

async function sendTurn(page: Page, prompt: string, events: Record<string, unknown>[], runId = 'run-desktop') {
  const composer = page.getByLabel('Chat message');
  const sent = await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0);
  await composer.fill(prompt);
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(sent);
  await emit(page, { type: 'runStarted', asked: prompt, runId, recall: [] });
  for (const e of events) await emit(page, e);
  await emit(page, { type: 'done', sessionId: null });
}

test('no chip until the chat has used the desktop, then the chip and the controls', async ({ page }) => {
  await installEventBridge(page);
  await openWorkspace(page);
  await expect(page.getByText('Computer control: Desktop')).toHaveCount(0);
  await sendTurn(page, 'Open Notes', [{ type: 'toolUse', id: 'tu-1', name: 'mcp__hollow_computer__click', input: { x: 1, y: 2 } }]);
  const trigger = page.getByRole('button', { name: /Computer control: Desktop/ }).or(page.locator('summary[aria-label^="Computer control"]'));
  await expect(trigger.first()).toBeVisible();
  await trigger.first().click();
  // The chip's controls are ActionMenu items (role menuitem), not buttons.
  await page.getByRole('menuitem', { name: 'Pause', exact: true }).click();
  await expect(page.locator('summary[aria-label="Computer control: Paused"], button[aria-label="Computer control: Paused"]').first()).toBeVisible();
  await page.locator('summary[aria-label="Computer control: Paused"], button[aria-label="Computer control: Paused"]').first().click();
  await page.getByRole('menuitem', { name: 'Take over', exact: true }).click();
  await expect(page.locator('[aria-label="Computer control: In your hands"]').first()).toBeVisible();
  await page.locator('[aria-label="Computer control: In your hands"]').first().click();
  await page.getByRole('menuitem', { name: 'Hand back to the agent' }).click();
  await expect(page.locator('[aria-label="Computer control: Desktop"]').first()).toBeVisible();
});

test('the side panel Computer section shows the frame, the actions with their tier and the controls', async ({ page }) => {
  await installEventBridge(page);
  await openWorkspace(page);
  await sendTurn(page, 'Open Notes', [{ type: 'toolUse', id: 'tu-1', name: 'mcp__hollow_computer__click', input: { x: 1, y: 2 } }]);
  const card = page.locator('#side-panel-card');
  await card.getByRole('button', { name: /Computer/ }).click();
  await expect(card.getByRole('img', { name: 'What the agent last saw on the desktop' })).toBeVisible();
  const actions = card.getByRole('list', { name: 'Recent desktop actions' });
  await expect(actions.getByRole('listitem').first()).toContainText('click: act tier, ran');
  await card.getByRole('button', { name: 'Pause', exact: true }).click();
  await expect(card.getByRole('status')).toContainText('Paused');
  await card.getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(actions.getByRole('listitem')).toHaveCount(1);
  const results = await new AxeBuilder({ page }).include('#side-panel-card').analyze();
  expect(results.violations).toEqual([]);
});
