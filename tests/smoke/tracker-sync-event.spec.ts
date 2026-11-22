// A tracker pass (`tracker-sync-updated`, emitted after every timer, Sync
// now or agent pass) refreshes the Tasks list, against the browser mock
// (`src/mockTracker.ts`, whose `conflicts` seed is read on every call, so a
// change between passes stands in for what the pass pulled). Written in the
// 11-12 overnight (row T2) without running it; the lane's browser run covers
// it.
import { expect, test } from './fixtures/test';
import { openTasks } from './choose';

const CONFLICT = { 'i-32': [{ field: 'title', local: 'Export the report', remote: 'Export the month-end report', at: '2026-11-11T09:00:00Z' }] };
const seedConflicts = (value: unknown) => {
  const current = JSON.parse(localStorage.getItem('mockTracker') ?? '{}');
  localStorage.setItem('mockTracker', JSON.stringify({ ...current, conflicts: value }));
};

test.beforeEach(async ({ page }) => {
  await page.goto('/?mock=1');
  await openTasks(page);
  await expect(page.locator('.issue-row[data-issue-id="i-32"]')).toBeVisible();
});

test('the list shows what a pass changed only once the pass event arrives', async ({ page }) => {
  const row = page.locator('.issue-row[data-issue-id="i-32"]');
  await expect(row).not.toHaveAttribute('data-conflict', '');
  // The mirror changes; the page doesn't read it until a pass says so.
  await page.evaluate(seedConflicts, CONFLICT);
  await expect(row.locator('.issue-flag', { hasText: 'Conflict' })).toHaveCount(0);
  await page.evaluate(() => window.__mockEmit('tracker-sync-updated', { ok: true, changed: 1 }));
  await expect(row).toHaveAttribute('data-conflict', '');
  await expect(row.locator('.issue-flag', { hasText: 'Conflict' })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Views' }).getByRole('button', { name: 'Conflicts (1)' })).toBeVisible();
  await expect(page.locator('.tracker-sync-line')).toContainText('Synced');
});

test('a later pass that settles the change clears it from the list', async ({ page }) => {
  await page.evaluate(seedConflicts, CONFLICT);
  await page.evaluate(() => window.__mockEmit('tracker-sync-updated', { ok: true, changed: 1 }));
  const conflicts = page.getByRole('group', { name: 'Views' }).getByRole('button', { name: /^Conflicts/ });
  await expect(conflicts).toBeVisible();
  await page.evaluate(seedConflicts, {});
  await page.evaluate(() => window.__mockEmit('tracker-sync-updated', { ok: true, changed: 1 }));
  await expect(conflicts).toHaveCount(0);
  await expect(page.locator('.issue-row[data-issue-id="i-32"]')).not.toHaveAttribute('data-conflict', '');
});
