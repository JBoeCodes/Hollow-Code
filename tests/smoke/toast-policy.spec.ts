import { openBackgroundTasks, openTasks } from './choose';
import { expect, test, type Page } from './fixtures/test';

/** Paperclip PR 22: quieter toasts. A notification about the workflow open
 * in Automations shows no toast; safety notices always do; a stopped run
 * reads neutrally. Commands go through the `?mock=1` IPC; the event bridge
 * below only adds Tauri event delivery, and the seam records the foreground
 * scope the page sends. */
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

async function installScopeSeam(page: Page) {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    if (!original.includes('export async function mockInvoke(')) throw new Error('Toast policy seam missing');
    await route.fulfill({ response, body: `${original}
{
  const baseToastMock = mockInvoke;
  window.__foregroundScopes = [];
  mockInvoke = async (command, args) => {
    if (command === 'set_foreground_notification_scope') window.__foregroundScopes.push(args?.scope ?? null);
    return baseToastMock(command, args);
  };
}
` });
  });
}

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

let seq = 0;
function notify(page: Page, actionUrl: string, source: string) {
  seq += 1;
  const payload = {
    id: `toast-${seq}`, title: `Notice ${seq} for ${actionUrl}`, body: '', kind: 'success', source,
    actionLabel: 'Open workflow', actionUrl, created: new Date().toISOString(), read: false,
  };
  return page.evaluate(body => {
    (window as unknown as { __emitTauriEvent: (name: string, payload: unknown) => void })
      .__emitTauriEvent('notification-event', body);
  }, payload).then(() => payload.title);
}

const scopes = (page: Page) =>
  page.evaluate(() => (window as unknown as { __foregroundScopes?: (string | null)[] }).__foregroundScopes ?? []);
const calls = (page: Page, command: string) =>
  page.evaluate(cmd => window.__mockApiCalls?.[cmd] ?? 0, command);

test('a workflow finishing while you watch it shows no toast', async ({ page }) => {
  await installEventBridge(page);
  await installScopeSeam(page);
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  await expect(page.locator('.workflow-output-actions')).toBeVisible();
  await expect.poll(() => scopes(page)).toContain('workflow:morning-brief');
  const toasts = page.locator('.notif-toast');

  // The workflow you are watching: no toast, but the unread count refetches.
  const unreadBefore = await calls(page, 'unread_notification_count');
  const watched = await notify(page, 'workflow:morning-brief', 'morning-brief');
  await expect.poll(() => calls(page, 'unread_notification_count')).toBeGreaterThan(unreadBefore);
  await expect(toasts.filter({ hasText: watched })).toHaveCount(0);

  // Ordinary completions remain in the bell; safety notices still toast.
  const other = await notify(page, 'workflow:close-my-day', 'close-my-day');
  await expect(toasts.filter({ hasText: other })).toHaveCount(0);
  const guard = await notify(page, 'workflow:morning-brief', 'run-guard');
  await expect(toasts.filter({ hasText: guard })).toHaveCount(1);
  // PER-36: a run whose only alerts are shared-doc notes is a bell item.
  const docs = await notify(page, 'settings:history', 'run-guard-docs');
  await expect(toasts.filter({ hasText: docs })).toHaveCount(0);
  const streak = await notify(page, 'workflow-failures:morning-brief', 'morning-brief');
  await expect(toasts.filter({ hasText: streak })).toHaveCount(1);

  // Leaving Automations clears the scope without reviving routine toasts.
  await openTasks(page);
  await expect.poll(async () => (await scopes(page)).at(-1)).toBeNull();
  const away = await notify(page, 'workflow:morning-brief', 'morning-brief');
  await expect(toasts.filter({ hasText: away })).toHaveCount(0);
});

test('a cancelled Run now is neutral', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockCancelledWorkflowRun', '1'));
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  await page.locator('.workflow-output-actions').getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => calls(page, 'run_workflow')).toBe(1);
  await expect(page.locator('.snackbar').filter({ hasText: 'Stopped.' })).toHaveCount(1);
  await expect(page.locator('.snackbar').filter({ hasText: 'run complete' })).toHaveCount(0);
  // The task log moved from the top bar's dock to Running (overnight 11-04
  // U1), which marks a stop neutrally: a dot that says Stopped, not a failure.
  const row = (await openBackgroundTasks(page)).locator('.activity-row').filter({ hasText: 'Run Morning Brief' });
  await expect(row).toHaveClass(/cancelled/);
  await expect(row.locator('.activity-icon')).toHaveText('·');
  await expect(row.locator('.activity-icon')).toHaveAttribute('aria-label', 'Stopped');
});

test('a Run now rejected with an expected cancellation is neutral too', async ({ page }) => {
  // The rejection path of trackTask: the run permit's "stopped while waiting
  // for a background agent slot" arrives as a thrown string, not a result.
  await page.addInitScript(() => localStorage.setItem('mockCancelledWorkflowRun', 'throw'));
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();
  await page.locator('.workflow-output-actions').getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => calls(page, 'run_workflow')).toBe(1);
  await expect(page.locator('.snackbar').filter({ hasText: 'Stopped.' })).toHaveCount(1);
  await expect(page.locator('.snackbar').filter({ hasText: 'run failed' })).toHaveCount(0);
  const row = (await openBackgroundTasks(page)).locator('.activity-row').filter({ hasText: 'Run Morning Brief' });
  await expect(row).toHaveClass(/cancelled/);
  await expect(row).not.toHaveClass(/error/);
  await expect(row.locator('.activity-icon')).toHaveText('·');
  await expect(row.locator('.activity-icon')).toHaveAttribute('aria-label', 'Stopped');
});
