import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Merged plan slice 1 (reviews A·S3 / B·F13): a remote image in a reply is
 * a click-to-load placeholder, and nothing is requested from its host until
 * the click — after which the bytes come through `fetch_remote_image`, never
 * from the page. The dev server applies no Tauri CSP, so without the
 * placeholder the browser here would request the image on render. Events
 * come through a Tauri event bridge (as in chat-account-failover.spec.ts);
 * commands go through the `?mock=1` IPC. */

const REMOTE_HOST = 'remote-image.test';

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

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

test('a remote image in a reply waits for a click and then loads through the backend', async ({ page }) => {
  const remoteRequests: string[] = [];
  page.on('request', req => {
    if (new URL(req.url()).hostname === REMOTE_HOST) remoteRequests.push(req.url());
  });
  // Never let a request (if one slipped out) reach DNS.
  await page.route(`**://${REMOTE_HOST}/**`, route => route.abort());

  await installEventBridge(page);
  await openChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Show me the chart');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  const tag = { clientTurnId: args.clientTurnId, sessionKey: 's-20260925-mock0001' };

  await emitChatEvent(page, { type: 'runStarted', runId: 'run-img-1', recall: [], ...tag });
  await emitChatEvent(page, {
    type: 'assistantText',
    text: `Here it is: ![usage chart](https://${REMOTE_HOST}/chart.png?q=SECRET)\n\n<img src="https://${REMOTE_HOST}/raw.png" alt="raw">`,
    ...tag,
  });

  const reply = page.locator('.chat-main .turn.assistant').last();
  const placeholders = reply.locator('button.md-remote-img');
  await expect(placeholders).toHaveCount(2);
  await expect(placeholders.first()).toContainText(`Image from ${REMOTE_HOST} — Load`);
  await expect(placeholders.first()).toContainText('usage chart');
  await expect(reply.locator(`img[src*="${REMOTE_HOST}"], source[srcset*="${REMOTE_HOST}"]`)).toHaveCount(0);
  // Give a would-be image load time to start.
  await page.waitForTimeout(300);
  expect(remoteRequests, 'nothing is requested from the image host before a click').toEqual([]);
  // App icons (connector favicons) may use the same command; this image hasn't.
  expect(await page.evaluate(() => localStorage.getItem('mockLastRemoteImage') ?? '')).not.toContain(REMOTE_HOST);

  await placeholders.first().click();
  await expect(reply.locator('img[alt="usage chart"]')).toHaveAttribute('src', /^data:image\/png;base64,/);
  await expect(placeholders).toHaveCount(1);
  expect(await page.evaluate(() => localStorage.getItem('mockLastRemoteImage'))).toBe(`https://${REMOTE_HOST}/chart.png?q=SECRET`);
  expect(remoteRequests, 'the click loads through the backend, not the page').toEqual([]);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
