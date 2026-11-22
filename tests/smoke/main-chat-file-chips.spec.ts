import { expect, test, type Page } from './fixtures/test';

/**
 * 2026-11-11 (Alex: the Main agent's replies still showed file paths as
 * plain inline code on the installed #1280 build): a reply in the Main chat
 * (no project) turns every backticked or bare file path into a chip too.
 */

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

test('a Main chat reply shows its file paths as chips', async ({ page }) => {
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();
  // Main is the COO: its page has no header band since 11-11 (no
  // `.main-space`, no COO heading); the top bar's COO button opens it.
  await page.locator('.top-bar').getByRole('button', { name: /^(COO|Back from the COO) ·/ }).click();
  await expect(page.locator('.top-bar').getByRole('button', { name: 'Actions for COO', exact: true })).toBeVisible();
  const composer = page.locator('.work-conversation').getByLabel('Chat message');
  await composer.fill('Where is it?');
  await composer.press('Meta+Enter');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBeGreaterThan(0);
  await emit(page, { type: 'runStarted', asked: 'Where is it?', runId: 'run-main-chips', recall: [] });
  const text = [
    'The record is in `outputs/install-verification-f69b5e3c9-2026-11-11.md`.',
    '',
    'Hand checks: `workspaces/local-f9ebfd0d1c780db6a000fcf2/outputs/coo-rebuild-2026-11-10/REPORT.md`,',
    'also hollow `outputs/coo-rebuild-2026-11-10/REPORT.md` and [the plan](workspaces/main/PLAN.md).',
  ].join('\n');
  // Streamed like a live turn: deltas cut mid-path, a pause between them.
  for (let at = 0; at < text.length; at += 23) {
    await emit(page, { type: 'assistantDelta', text: text.slice(at, at + 23) });
    await page.waitForTimeout(40);
  }
  await emit(page, { type: 'done', sessionId: null });
  const reply = page.locator('.chat-msg').last();
  await expect(reply.locator('.file-chip')).toHaveCount(4);
  await expect(reply.locator('code.file-ref:not(.file-chip)')).toHaveCount(0);
});
