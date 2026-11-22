import { test, expect, type Page } from './fixtures/test';

async function openAutomation(page: Page, status: string) {
  await page.addInitScript(status => localStorage.setItem('mockRunQueue', JSON.stringify([{
    id: 11, source: 'cron', sourceId: 'morning-brief', label: 'Morning brief',
    detail: null, reason: 'scheduled', status, priority: 3, position: null,
    attempt: 0, coalescedCount: 0, requestedAt: '2026-10-22T03:00:00Z',
    claimedAt: status === 'queued' ? null : '2026-10-22T03:01:00Z', finishedAt: null, historyRunId: 'r1',
    error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
  }])), status);
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await expect(page.locator('.workflow-review')).toBeVisible();
}

for (const status of ['running', 'claimed']) test(`an automation already ${status} has a Stop run on its review`, async ({ page }) => {
  await openAutomation(page, status);
  // Below 1100 px an open review takes the list's place; "All automations" is the way back.
  const back = page.getByRole('button', { name: '← All automations', exact: true });
  const listHidden = await back.isVisible();
  if (listHidden) await back.click();
  const row = page.locator('.workflow-row').filter({ has: page.getByRole('button', { name: 'Open Morning Brief', exact: true }) });
  await expect(row.locator('.workflow-row-run')).toBeDisabled();
  await expect(page.locator('.workflow-running-pill')).toHaveText('1 running');
  if (listHidden) await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  const review = page.locator('.workflow-review');
  await expect(review.locator('.workflow-cover')).toHaveAttribute('data-state', 'busy');
  await expect(review.getByRole('button', { name: 'Run now', exact: true })).toHaveCount(0);
  await review.getByRole('button', { name: 'Stop run', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.cancel_workflow ?? 0)).toBe(1);
  // A stop request is not proof the process ended. Only queue/event state clears it.
  await expect(review.getByRole('button', { name: 'Run now', exact: true })).toHaveCount(0);
  await page.evaluate(async () => {
    const path = '/src/store/runQueue.ts'; const store = await import(/* @vite-ignore */ path);
    store.setRunQueue(store.runQueue().map((r: { status: string }) => ({ ...r, status: 'cancelled' })));
  });
  await expect(review.getByRole('button', { name: 'Run now', exact: true })).toBeVisible();
  await expect(page.locator('.workflow-running-pill')).toHaveCount(0);
});

test('the running editor keeps every action inside the minimum Largest view', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 600 });
  await page.addInitScript(() => localStorage.setItem('hollow.studio.textScale', '125'));
  await openAutomation(page, 'running');
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const editor = page.locator('.workflow-detail-head');
  await expect.poll(() => editor.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
  const stop = editor.getByRole('button', { name: 'Stop run', exact: true });
  await stop.scrollIntoViewIfNeeded();
  const bounds = await stop.boundingBox();
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(720);
  await stop.click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.cancel_workflow ?? 0)).toBe(1);
});

test('the editor can stop a run that began before it opened', async ({ page }) => {
  await openAutomation(page, 'running');
  await page.getByRole('button', { name: 'Automation options' }).click();
  await page.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  const editor = page.locator('.workflow-detail-head');
  await editor.getByRole('button', { name: 'Stop run', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.cancel_workflow ?? 0)).toBe(1);
});

test('a merely queued automation is not shown as running', async ({ page }) => {
  await openAutomation(page, 'queued');
  await expect(page.locator('.workflow-review').getByRole('button', { name: 'Stop run', exact: true })).toHaveCount(0);
  await expect(page.locator('.workflow-review').getByRole('button', { name: 'Run now', exact: true })).toBeVisible();
});

test('a failed stop request says the outcome could not be confirmed', async ({ page }) => {
  await openAutomation(page, 'running');
  await page.evaluate(async () => {
    const path = '/src/api.ts'; const { api } = await import(/* @vite-ignore */ path);
    api.cancelWorkflow = async () => { throw new Error('Queue owner unavailable'); };
  });
  await page.locator('.workflow-review').getByRole('button', { name: 'Stop run', exact: true }).click();
  await expect(page.getByText(/Stop request could not be confirmed: Queue owner unavailable/)).toBeVisible();
  await expect(page.locator('.workflow-review').getByRole('button', { name: 'Stop run', exact: true })).toBeEnabled();
});

test('Last night and Tonight are not Automations views', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Automations', exact: true }).click();
  await expect(page.getByRole('navigation', { name: 'Automations views' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Last night', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Tonight', exact: true })).toHaveCount(0);
});
