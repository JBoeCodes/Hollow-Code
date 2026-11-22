import { expect, test, type Page } from './fixtures/test';
import { bellBadge, bellDialog, bellHistoryRows, bellViews, openBellHistory, railButton, waitingBell } from './waiting';

/** U3: the popover opens to Needs you while something waits, so every
 *  history check first chooses its Notifications view, explicitly. */
async function showHistory(page: Page) {
  await openBellHistory(page);
  return bellDialog(page);
}
/** Only the history list (U3b: the one notification history). */
const historyRows = (page: Page) => bellHistoryRows(page);

test('bell opens without navigating, closes with Escape and restores focus', async ({ page }) => {
  await page.goto('/?mock=1');
  const bell = waitingBell(page);
  const badge = bellBadge(page);
  // A quiet dot since the top-bar polish (#1492); the exact count is in the bell's description and its dialog.
  await expect(badge).toHaveText('•');
  await expect(bell).toHaveAttribute('aria-description', /^Needs you, \d+ waiting/);
  expect(await badge.evaluate(el => parseFloat(getComputedStyle(el).fontSize))).toBeGreaterThanOrEqual(11);
  const title = await page.locator('.crumb-file').textContent();
  await bell.click();
  const center = bellDialog(page);
  await expect(center).toBeVisible();
  await expect(bellViews(page).getByRole('button', { name: /^Needs you/ })).toBeVisible();
  await showHistory(page);
  await expect(center.getByRole('button', { name: 'Refresh', exact: true })).toBeVisible();
  await expect(page.locator('.crumb-file')).toHaveText(title!);
  // A day label, not a row: the row itself opens its notification.
  await center.getByRole('region', { name: 'Notification history' }).locator('.notif-day-sep').first().click();
  await page.keyboard.press('Escape');
  await expect(center).toHaveCount(0);
  await expect(bell).toBeFocused();
});

test('bell keeps failed history separate from empty and offers retry', async ({ page }) => {
  await page.route('**/src/mockApi.ts*', async route => {
    const response = await route.fetch();
    const original = await response.text();
    await route.fulfill({ response, body: `${original}\n{
      const baseBellMock = mockInvoke;
      window.__bellRetry = false;
      mockInvoke = async (command, args) => {
        if (command === 'list_notifications' && !window.__bellRetry) throw new Error('Fixture history unavailable');
        return baseBellMock(command, args);
      };
    }` });
  });
  await page.goto('/?mock=1');
  await waitingBell(page).click();
  const center = await showHistory(page);
  await expect(center.getByRole('button', { name: 'Retry notifications' })).toBeVisible();
  await expect(center.getByText('Nothing since you last looked.', { exact: true })).toHaveCount(0);
  // The bell says so too: its history is unavailable, never zero unread.
  await expect(waitingBell(page)).toHaveAttribute('aria-description', /\. Notification history unavailable$/);
  await expect(bellBadge(page)).toHaveText('!');
  await page.evaluate(() => { (window as unknown as { __bellRetry: boolean }).__bellRetry = true; });
  await center.getByRole('button', { name: 'Retry notifications' }).click();
  await expect(center.getByRole('button', { name: 'Retry notifications' })).toHaveCount(0);
  await expect(historyRows(page).first()).toBeVisible();
  await expect(waitingBell(page)).toHaveAttribute('aria-description', /\. \d+ unread notifications$/);
});

test('bell owns its keys above Running', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio&missions=1');
  await expect(page.locator('.studio-home')).toBeVisible();
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true }).click();
  const surface = page.getByRole('region', { name: 'Running', exact: true });
  await expect(surface).toBeVisible();
  {
    const entry = page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true });
    await expect(entry).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(entry).toBeFocused();
    await page.keyboard.press('j');
    await expect(surface.locator('.mc-track-segment:focus')).toHaveCount(1);
    await entry.focus();
    await page.keyboard.press('k');
    await expect(surface.locator('.mc-track-segment:focus')).toHaveCount(1);
  }
  const history = await openBellHistory(page);
  const rows = history.locator('.notif-row-body');
  await expect(rows.nth(1)).toBeVisible();
  await history.getByRole('button', { name: 'All', exact: true }).focus();
  await page.keyboard.press('j');
  await expect(rows.first()).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(rows.nth(1)).toBeFocused();
  const details = history.locator('.notif-row').nth(1).locator('details');
  await details.locator('summary').focus();
  await page.keyboard.press('Enter');
  await expect(details).toHaveAttribute('open', '');
  const messages = history.locator('.notification-app-messages');
  await messages.locator('summary').focus();
  await page.keyboard.press('Enter');
  await expect(messages).toHaveAttribute('open', '');
  await expect(surface).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(bellDialog(page)).toHaveCount(0);
  await expect(waitingBell(page)).toBeFocused();
});

test('opening a failure toast leaves its explicit acknowledgment in the bell', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await page.evaluate(() => {
    (window as unknown as { __mockEmit: (name: string, payload: unknown) => void }).__mockEmit('notification-event', {
      id: 'mock-notif-2', title: 'Close my day failed', body: 'Failure details', kind: 'error', source: 'close-my-day',
      created: new Date().toISOString(), read: false, actionUrl: 'workflow:close-my-day', actionLabel: 'Open routine',
    });
  });
  const toast = page.locator('.notif-toast', { hasText: 'Close my day failed' });
  await expect(toast).toBeVisible();
  const reads = await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0);
  await toast.locator('.notif-toast-body').click();
  await expect(toast).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Automations', level: 1 })).toBeVisible();
  const history = await openBellHistory(page);
  const failure = history.locator('[data-notif-id="mock-notif-2"]');
  await expect(failure.getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(reads);
  await failure.getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(reads + 1);
});

test('listed history actions preserve the unread failure and its individual action', async ({ page }) => {
  await page.goto('/?mock=1');
  const bell = waitingBell(page);
  await bell.click();
  const center = await showHistory(page);
  await center.getByRole('button', { name: 'Mark listed read' }).click();
  await expect(center.getByRole('button', { name: 'Mark listed read' })).toHaveCount(0);
  const failure = historyRows(page).filter({ hasText: 'Close my day failed' });
  await expect(failure.getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
  await center.getByRole('button', { name: 'Clear listed' }).click();
  const confirm = page.getByRole('dialog').filter({ hasText: /Clear \d+ notifications?\?/ });
  await confirm.getByRole('button', { name: 'Clear', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.delete_notifications ?? 0)).toBe(1);
  await bell.click();
  await showHistory(page);
  await expect(historyRows(page)).toHaveCount(1);
  await expect(failure.getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
});

test('the bell history keeps saved filters, folded repeats and moved targets (U3b)', async ({ page }) => {
  await page.addInitScript(() => {
    const now = Date.now();
    const at = (minutes: number) => new Date(now - minutes * 60_000).toISOString();
    localStorage.setItem('mockNotificationsExtra', JSON.stringify([
      { id: 'repeat-1', title: 'Plan sweep finished', body: '', kind: 'info', source: 'plan-sweep', actionLabel: null, actionUrl: null, created: at(1), read: true },
      { id: 'repeat-2', title: 'Plan sweep finished', body: '', kind: 'info', source: 'plan-sweep', actionLabel: null, actionUrl: null, created: at(2), read: true },
      { id: 'repeat-3', title: 'Plan sweep finished', body: '', kind: 'info', source: 'plan-sweep', actionLabel: null, actionUrl: null, created: at(3), read: true },
      { id: 'moved-target', title: 'Summary written', body: '', kind: 'info', source: 'nudge', actionLabel: 'Open', actionUrl: 'vault:notes/u3b-gone-fixture.md', created: at(5), read: false },
    ]));
  });
  await page.goto('/?mock=1');
  // The index is loaded by Library, not by opening the bell.
  await railButton(page, 'Library').click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.vault_notes_list ?? 0)).toBeGreaterThan(0);
  const history = await openBellHistory(page);
  // Repeats fold into one row with a count, per day.
  await expect(history.locator('.notif-row').filter({ hasText: 'Plan sweep finished' })).toHaveCount(1);
  await expect(history.locator('.notif-row').filter({ hasText: 'Plan sweep finished' })).toContainText('3 times');
  // The filter is the saved notifFilter preference: it survives a reload.
  const filters = history.getByRole('group', { name: 'Filter notifications' });
  await filters.getByRole('button', { name: /^Nudges/ }).click();
  await expect(filters.getByRole('button', { name: /^Nudges/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(history.locator('.notif-row').filter({ hasText: 'Plan sweep finished' })).toHaveCount(0);
  const moved = history.locator('.notif-row').filter({ hasText: 'Summary written' });
  // Once the note index has loaded, a target that is gone says so, and the
  // row only acknowledges it.
  await expect(moved).toContainText('Target moved');
  await moved.locator('.notif-row-body').click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(1);
  await expect(bellDialog(page)).toBeVisible();
  await page.reload();
  const again = await openBellHistory(page);
  await expect(again.getByRole('group', { name: 'Filter notifications' }).getByRole('button', { name: /^Nudges/ })).toHaveAttribute('aria-pressed', 'true');
  // Per-row Delete removes just that row.
  await again.locator('.notif-row').filter({ hasText: 'Summary written' }).getByRole('button', { name: 'Delete notification' }).click();
  await expect(again.locator('.notif-row').filter({ hasText: 'Summary written' })).toHaveCount(0);
});

test('full notification details and list keys survive acknowledgment in the bell', async ({ page }) => {
  const title = 'A full notification title '.repeat(8);
  const body = 'A full report paragraph. '.repeat(30) + '\nFinal evidence stays available.';
  await page.addInitScript(({ title, body }) => {
    localStorage.setItem('mockNotificationsExtra', JSON.stringify([
      { id: 'long-detail', title, body, kind: 'info', source: 'daily-report', actionLabel: null, actionUrl: null, created: new Date().toISOString(), read: false },
    ]));
  }, { title, body });
  await page.goto('/?mock=1');
  const history = await openBellHistory(page);
  const row = history.locator('[data-notif-id="long-detail"]');
  const details = row.locator('details');
  await details.locator('summary').click();
  await expect(details.locator('strong')).toHaveText(title);
  await expect(details.locator('p')).toHaveText(body);
  await expect(details.locator('time')).toHaveAttribute('datetime', /T/);
  // x remains the deliberate Mark read action; stable ids keep Details open.
  await page.keyboard.press('x');
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(1);
  await expect(details).toHaveAttribute('open', '');
  await expect(row.getByRole('button', { name: 'Mark read', exact: true })).toHaveCount(0);
  await history.getByRole('button', { name: 'All', exact: true }).focus();
  await page.keyboard.press('j');
  await expect(history.locator('.notif-row-body').first()).toBeFocused();
  await page.keyboard.press('ArrowDown');
  await expect(history.locator('.notif-row-body').nth(1)).toBeFocused();
  await page.keyboard.press('k');
  await expect(history.locator('.notif-row-body').first()).toBeFocused();
});

test('opening an unread failure from history keeps its own Mark read', async ({ page }) => {
  await page.goto('/?mock=1');
  const history = await openBellHistory(page);
  await history.locator('.notif-row').filter({ hasText: 'Close my day failed' }).locator('.notif-row-body').click();
  await expect(bellDialog(page)).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.mark_notification_read ?? 0)).toBe(0);
  const again = await openBellHistory(page);
  await expect(again.locator('.notif-row').filter({ hasText: 'Close my day failed' }).getByRole('button', { name: 'Mark read', exact: true })).toBeVisible();
});

test('folded row actions acknowledge and delete every repeated notification', async ({ page }) => {
  await page.addInitScript(() => {
    const rows = ['read', 'delete', 'keys', 'open'].flatMap((group, index) => [1, 2, 3].map(number => ({
      id: `${group}-${number}`, title: `Repeated ${group} notice`, body: '', kind: 'info', source: 'daily-report', actionLabel: null, actionUrl: null,
      created: new Date(Date.now() - (index * 10 + number) * 60000).toISOString(), read: false,
    })));
    localStorage.setItem('mockNotificationsExtra', JSON.stringify(rows));
  });
  await page.goto('/?mock=1');
  const history = await openBellHistory(page);
  const repeated = (group: string) => history.locator('.notif-row').filter({ hasText: `Repeated ${group} notice` });
  for (const group of ['read', 'delete', 'keys', 'open']) {
    await expect(repeated(group)).toHaveCount(1);
    await expect(repeated(group)).toContainText('3 times');
  }
  await repeated('read').getByRole('button', { name: 'Mark read', exact: true }).click();
  await expect(repeated('read')).toHaveClass(/\bread\b/);
  await expect(repeated('read')).toHaveCount(1);
  await repeated('delete').getByRole('button', { name: 'Delete notifications', exact: true }).click();
  await expect(repeated('delete')).toHaveCount(0);
  await repeated('keys').locator('.notif-row-body').focus();
  await page.keyboard.press('x');
  await expect(repeated('keys')).toHaveClass(/\bread\b/);
  await expect(repeated('keys')).toHaveCount(1);
  await repeated('open').locator('.notif-row-body').click();
  await expect(repeated('open')).toHaveClass(/\bread\b/);
  await expect(repeated('open')).toHaveCount(1);
});
