import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Useful and Not useful left the reply (overnight 11-08 S5), and the vote
 * wrappers and their mock went in the 11-10 sweep, so nothing in the window
 * can record a vote. No agent, provider or vault is involved. */

/** Tauri event delivery for `listen()`, as in chat-workspace.spec.ts:
 * commands still go through the mock IPC. */
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

async function showChat(page: Page) {
  await openSettingsSection(page, 'advanced', /^Advanced/);
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

async function openChat(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
}

async function openConversation(page: Page, _source: 'hollow', title: string) {
  const search = page.getByRole('textbox', { name: 'Search conversations' });
  if (!await search.isVisible()) {
    await page.getByRole('button', { name: 'Show conversations', exact: true }).click();
  }
  await expect(search).toBeVisible();
  // One list since run 15 P4 removed Claude history and its source switch.
  await page.locator('.chat-aside').getByRole('button', { name: `Open ${title}`, exact: true }).click();
}

function assistantTurns(page: Page) {
  return page.locator('.chat-main .turn.assistant');
}

// Useful and Not useful came back on every finished reply's action row
// (overnight 11-12, C2, #1307; `replyActions.canVote`).
test('a finished reply has Copy, Useful and Not useful', async ({ page }) => {
  await openChat(page);
  await openConversation(page, 'hollow', 'Wire the session store');
  const answer = assistantTurns(page).filter({ hasText: 'The shared conversation record is ready.' });
  await expect(answer).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Not useful', exact: true })).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Copy reply', exact: true })).toBeVisible();
});

test('a live multi-bubble run grows one footer and takes a vote once it ends', async ({ page }) => {
  await installEventBridge(page);
  await openChat(page);
  await emitChatEvent(page, { type: 'runStarted', runId: 'run-live-1', sessionKey: 's-20260926-feed0001', recall: [] });
  await emitChatEvent(page, { type: 'assistantText', text: 'First part of the answer.' });
  await emitChatEvent(page, { type: 'turnBoundary' });
  await emitChatEvent(page, { type: 'assistantText', text: 'Second part of the answer.' });
  await emitChatEvent(page, {
    type: 'evidenceReceipt',
    receipt: {
      id: 'receipt-live-1', runId: 'run-live-1', source: 'chat', sourceId: null, startedAt: new Date().toISOString(),
      finishedAt: new Date().toISOString(), status: 'success', verificationState: 'noChanges', files: [], checks: [],
      failures: [], historyRunId: 'run-live-1', sessionKey: 's-20260926-feed0001',
    },
  });
  await expect(assistantTurns(page)).toHaveCount(2);
  await expect(page.locator('.chat-main .turn-footer:not(.live)')).toHaveCount(0);
  // No vote while the run is still in flight.
  await expect(page.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(0);

  await emitChatEvent(page, { type: 'done', sessionId: null, sessionKey: 's-20260926-feed0001' });
  // One footer for the whole reply, on its last bubble, with one vote pair.
  await expect(page.locator('.chat-main .turn-footer')).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(1);
  await expect(page.getByRole('button', { name: 'Not useful', exact: true })).toHaveCount(1);
});

test('an artifact summary takes a vote like any finished reply', async ({ page }) => {
  await openChat(page);
  await page.evaluate(async moduleUrl => {
    const artifacts = await import(moduleUrl);
    await artifacts.continueArtifactSession('s-20260811-fixture1', 'projects/Hollow 3.0.md', true);
  }, '/src/store/artifactSessions.ts');
  await showChat(page);
  const answer = assistantTurns(page).filter({ hasText: 'Updated the project outcome.' });
  await expect(answer).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Useful', exact: true })).toHaveCount(1);
  await expect(answer.getByRole('button', { name: 'Not useful', exact: true })).toHaveCount(1);
});

test('Usage keeps Budgets and has no Votes list', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await openSettingsSection(page, 'usage', /^Usage/);
  await expect(page.getByRole('heading', { name: 'Budgets', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Votes', exact: true })).toHaveCount(0);
  await expect(page.locator('.feedback-row')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Clear feedback', exact: true })).toHaveCount(0);
});
