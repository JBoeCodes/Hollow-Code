import { test, expect } from './fixtures/test';

// Consolidation (lane B's J8 finding, 2026-10-13): an automation's run in
// Activity opens that automation, with its latest result and runs, not the
// Automations list. Clicks only.
test('Activity: an automation run’s Open lands on that automation', async ({ page }) => {
  await page.addInitScript(() => {
    const now = new Date(Date.now() - 2000).toISOString();
    localStorage.setItem('mockRunQueue', JSON.stringify([{
      id: 43, source: 'cron', sourceId: 'morning-brief', label: 'Morning brief', detail: null, reason: 'cron', status: 'done', priority: 0, position: null, attempt: 1,
      coalescedCount: 0, requestedAt: now, claimedAt: now, finishedAt: new Date().toISOString(), historyRunId: null, error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false,
    }]));
    localStorage.setItem('hollow.missionView', 'list');
  });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true }).click();
  await page.locator('.mc-done-row').filter({ hasText: 'Morning brief' }).getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Automations', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: /Morning brief/i }).first()).toBeVisible();
});
