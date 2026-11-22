import { expect, test, type Page } from './fixtures/test';
import { backgroundTasks, choose, runningList } from './choose';

/** The silent-run indicator (Paperclip PR 15b): a running automation's card
 * in Running (the top bar's dock row until overnight 11-04 U1) shows its
 * progress and quiet time from the backend's `activity` ticks, with a Stop;
 * the chat composer shows the turn's quiet
 * time. Commands go through the `?mock=1` IPC; events through a stand-in
 * for Tauri's event bridge (as in run-queue.spec.ts). No agent is involved. */

const MOCK_KEY = 's-20260925-mock0001';

const RUNNING_CRON = {
  id: 11,
  source: 'cron',
  sourceId: 'morning-brief',
  label: 'Morning brief',
  detail: null,
  reason: 'scheduled',
  status: 'running',
  priority: 3,
  position: null,
  attempt: 0,
  coalescedCount: 0,
  requestedAt: '2026-09-26T08:59:00.000Z',
  claimedAt: '2026-09-26T09:00:00.000Z',
  finishedAt: null,
  historyRunId: 'r1',
  error: null,
  waitingSince: null,
  waitingHolder: null,
  heldReason: null,
  cancelRequested: false,
};

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

function emit(page: Page, name: string, payload: Record<string, unknown>) {
  return page.evaluate(([event, body]) => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent(event as string, body);
  }, [name, payload] as const);
}

/** An ISO time `minutes` ago in the page's clock. */
function minutesAgo(page: Page, minutes: number) {
  return page.evaluate(m => new Date(Date.now() - m * 60_000).toISOString(), minutes);
}

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

function narrow(page: Page) {
  return (page.viewportSize()?.width ?? 1000) <= 860;
}

async function showChat(page: Page) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'advanced');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Advanced', exact: true }).click();
  }
  await page.locator('#settings-group-this-mac > summary').click(); // History & recovery: Advanced → This Mac
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await page.locator('.library-tabs').getByRole('button', { name: 'Conversations', exact: true }).click(); // Settings PR 4 cut the Conversation history button
  await expect(page.getByRole('textbox', { name: 'Chat message' })).toBeVisible();
}

test('a running automation shows its quiet time and a Stop on its own Running card', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(items => {
    localStorage.setItem('mockRunQueue', JSON.stringify(items));
  }, [RUNNING_CRON]);
  await page.goto('/?mock=1');
  await expect.poll(() => callCount(page, 'list_run_queue')).toBeGreaterThan(0);

  // A running automation alone (nothing queued, no task tracked) is a card on
  // Running's turn list, named as running. The top bar's dock showed it until
  // overnight 11-04 U1.
  const turns = await runningList(page);
  const row = turns.getByRole('article', { name: /^Morning brief,/ });
  await expect(row).toContainText('Running');

  // A run started before a reload: no `started` was seen, and its first
  // activity tick still attaches to its card by its history run id.
  await emit(page, 'workflow-event', {
    type: 'activity',
    slug: 'morning-brief',
    runId: 'r1',
    queueId: 11,
    lastEventAt: await minutesAgo(page, 7.5),
    progress: null,
  });
  await expect(row).toContainText('quiet 7m');
  // The activity rides on the run's card: no second entry for the run, in
  // the turn list or in Running's background task log.
  await expect(turns.getByRole('listitem').filter({ hasText: 'Morning brief' })).toHaveCount(1);
  await expect(backgroundTasks(page)).toHaveCount(0);

  // A later tick with a progress phase replaces it.
  await emit(page, 'workflow-event', {
    type: 'activity',
    slug: 'morning-brief',
    runId: 'r1',
    queueId: 11,
    lastEventAt: await minutesAgo(page, 8.5),
    progress: 'researching',
  });
  await expect(row).toContainText('Researching · quiet 8m');

  // Another run's activity changes nothing here.
  await emit(page, 'workflow-event', {
    type: 'activity',
    slug: 'close-my-day',
    runId: 'r-other',
    queueId: 12,
    lastEventAt: await minutesAgo(page, 30),
    progress: 'coordinating',
  });
  await expect(row).not.toContainText('Coordinating');

  // Stop goes through the queue's own cancel, after Running's confirm.
  expect(await callCount(page, 'cancel_queued_run')).toBe(0);
  await row.getByRole('button', { name: 'Stop', exact: true }).click();
  await page.getByRole('dialog', { name: 'Stop “Morning brief”?' }).getByRole('button', { name: 'Stop', exact: true }).click();
  await expect.poll(() => callCount(page, 'cancel_queued_run')).toBe(1);

  // Its `done` drops the activity: no quiet time is left on Running, whether
  // the card falls back to its queue status or leaves the list.
  await emit(page, 'workflow-event', {
    type: 'done',
    slug: 'morning-brief',
    runId: 'r1',
    queueId: 11,
    status: 'cancelled',
    output: '',
    durationSec: 1,
    error: 'cancelled by user',
    nextRunAt: null,
    receipt: null,
    evidenceError: null,
  });
  await expect(turns.getByText(/quiet \d+m/)).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

test('the chat composer shows how long the running turn has been quiet', async ({ page }) => {
  await installEventBridge(page);
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await showChat(page);
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await composer.fill('Summarize the launch notes');
  await composer.press('Meta+Enter');
  await expect.poll(() => callCount(page, 'chat_send')).toBe(1);
  const clientTurnId = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}').clientTurnId);

  await emit(page, 'chat-event', { type: 'runStarted', runId: 'run-chat-1', sessionKey: MOCK_KEY, recall: [], clientTurnId });
  const ember = page.locator('.chat-ember');
  await expect(ember).toBeVisible();
  await expect(ember).not.toContainText('quiet');

  // A tick for another run is ignored; the turn's own tick shows.
  await emit(page, 'chat-event', { type: 'activity', runId: 'run-other', sessionKey: MOCK_KEY, lastEventAt: await minutesAgo(page, 20), progress: null, clientTurnId });
  await expect(ember).not.toContainText('quiet');
  await emit(page, 'chat-event', { type: 'activity', runId: 'run-chat-1', sessionKey: MOCK_KEY, lastEventAt: await minutesAgo(page, 7.5), progress: 'usingTools', clientTurnId });
  await expect(ember).toContainText('quiet 7m');
  await expect(ember).toContainText('Using tools');

  await emit(page, 'chat-event', { type: 'done', sessionId: null, sessionKey: MOCK_KEY, clientTurnId });
  await expect(ember).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});

/** Paperclip PR 15c: a receipt's phase timings render as a Timings section
 * listing only the measured phases (the mock's workflow receipt has no first
 * output figure). */
test('a workflow receipt lists its measured phase timings', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('complementary', { name: 'Main navigation' })
    .getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  const receipt = page.locator('.execution-receipt').first();
  await expect(receipt).toBeVisible();
  await receipt.locator('summary.execution-receipt-summary').click();
  const timings = receipt.locator('.execution-receipt-timings');
  await expect(receipt.getByText('Timings', { exact: true })).toBeVisible();
  await expect(timings).toContainText('Queue wait · 1.2s');
  await expect(timings).toContainText('Preparation · 2.3s');
  await expect(timings).toContainText('Agent turn · 2m 5s');
  await expect(timings).toContainText('Repository capture · 140ms');
  await expect(timings).toContainText('History diff · 96ms');
  await expect(timings).not.toContainText('First output');
  expect(await page.evaluate(() => window.__mockApiMisses ?? [])).toEqual([]);
});
