import { expect, test, type Locator, type Page } from './fixtures/test';
import { backgroundTasks, openRunning, runningList } from './choose';

/** The durable automation queue's UI (Paperclip PR 7): Running's queued count
 * and turn list (the top bar's dock chip and its Manage until overnight 11-04
 * U1), the Automations windows and run queue list, cancel, and the event-driven refetch. Commands
 * go through the `?mock=1` IPC; `localStorage.mockRunQueue` seeds the rows. */

const QUEUED_CRON = {
  id: 7,
  source: 'cron',
  sourceId: 'morning-brief',
  label: 'Morning brief',
  detail: null,
  reason: 'scheduled',
  status: 'queued',
  priority: 3,
  position: 1,
  attempt: 0,
  coalescedCount: 0,
  requestedAt: '2026-07-15T16:59:00.000Z',
  claimedAt: null,
  finishedAt: null,
  historyRunId: null,
  error: null,
  waitingSince: '2026-07-15T16:59:30.000Z',
  waitingHolder: 'a chat turn',
  heldReason: null,
  cancelRequested: false,
};

/** Stand-in for Tauri's event bridge (as in chat-workspace.spec.ts), so
 *  `listen()` registers for real and a test can deliver `run-queue-event` to
 *  the app's own listener. Commands still go through the `?mock=1` IPC. */
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

function callCount(page: Page, command: string) {
  return page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);
}

const QUEUED_MANUAL = {
  ...QUEUED_CRON,
  id: 9,
  source: 'manual',
  sourceId: 'close-my-day',
  label: 'Close my day',
  reason: 'run now',
  priority: 1,
  position: 1,
  waitingSince: null,
  waitingHolder: null,
};

test('queued automations show in Running, and Automations can skip or cancel them', async ({ page }) => {
  await installEventBridge(page);
  await page.addInitScript(items => {
    localStorage.setItem('mockRunQueue', JSON.stringify(items));
  }, [QUEUED_MANUAL, { ...QUEUED_CRON, position: 2 }]);
  await page.goto('/?mock=1');

  // Live work's own home, Running (run 15 A2; the top bar's dock chip and
  // its Manage led here until overnight 11-04 U1), counts and lists both
  // queued runs, each with why it waits. The queue's full list, with Skip
  // and Cancel, stays in Automations (the windows and run queue fold).
  const queuedCount = (running: Locator) => running.locator('.mc-stats span', { hasText: 'queued' });
  const running = await openRunning(page);
  await expect(queuedCount(running)).toHaveText('2 queued');
  const turns = await runningList(page);
  await expect(turns).toContainText('Morning brief');
  await expect(turns).toContainText('Waiting for a chat turn');
  await expect(turns).toContainText('Close my day');

  // The queue's full list moved to Automations' windows and run queue fold (Settings PR 4).
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.getByText('Automation windows and the run queue', { exact: true }).first().click();
  const panel = page.locator('.run-queue-panel');
  await expect(panel).toBeVisible();
  await expect(panel).toContainText('Morning brief');
  await expect(panel).toContainText('Waiting for a chat turn');
  await expect(panel).toContainText('Scheduled');

  // Skipping a queued cron occurrence only flags it: the claim still records
  // the skipped occurrence, so the row stays until then, with no action left.
  expect(await callCount(page, 'cancel_queued_run')).toBe(0);
  const cronRow = panel.locator('.run-queue-row', { hasText: 'Morning brief' });
  await cronRow.getByRole('button', { name: 'Skip', exact: true }).click();
  await expect.poll(() => callCount(page, 'cancel_queued_run')).toBe(1);
  await expect(cronRow).toContainText('Skipping this occurrence');
  await expect(cronRow.getByRole('button')).toHaveCount(0);

  // Cancelling a queued Run now ends it; it moves to the finished list.
  const manualRow = panel.locator('.run-queue-row', { hasText: 'Close my day' }).first();
  await manualRow.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect.poll(() => callCount(page, 'cancel_queued_run')).toBe(2);
  await expect(panel.locator('.run-queue-recent')).toContainText('Close my day');
  // Running counts what is still queued: the skipped occurrence until its claim.
  await expect(queuedCount(await openRunning(page))).toHaveText('1 queued');

  // Any run-queue-event schedules one (debounced) refetch of the list.
  const listed = await callCount(page, 'list_run_queue');
  await page.evaluate(item => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent('run-queue-event', { kind: 'enqueued', item });
  }, { ...QUEUED_CRON, id: 8 });
  await expect.poll(() => callCount(page, 'list_run_queue')).toBeGreaterThan(listed);
});

test('an empty queue shows nothing queued, and Running adds no background task list', async ({ page }) => {
  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();
  await expect.poll(() => callCount(page, 'list_run_queue')).toBeGreaterThan(0);
  // No queue chip or dock anywhere in the top bar (overnight 11-04 U1).
  await expect(page.locator('.top-bar .activity-dock, .top-bar .activity-dock-queued')).toHaveCount(0);
  const running = await openRunning(page);
  await expect(running.locator('.mc-stats span', { hasText: 'queued' })).toHaveText('0 queued');
  await expect(backgroundTasks(page)).toHaveCount(0);
});
