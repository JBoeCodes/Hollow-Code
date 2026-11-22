import { expect, test } from './fixtures/test';
import { bellDialog, openBellHistory, waitingBell } from './waiting';
import { openProjectFiles, reachSidebar } from './choose';

test('a full notice shelf leaves navigation clear and scrolls to keyboard actions', async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.addInitScript(() => {
    localStorage.setItem('hollow.studio.textScale', '125');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['notices']));
    const at = new Date().toISOString(), cwd = '/Users/fixture/vault/workspaces/notices';
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'notices', title: 'Notice fixture', cwd, status: 'active', path: 'workspaces/notices', modified: at }]));
  });
  await page.goto('/?mock=1');
  const { target } = await reachSidebar(page, page.getByRole('button', { name: /^Notice fixture/ }));
  await target.click();
  // The Preview pane's tabs stand in for the workspace panel's section row
  // (COO rebuild L4).
  await openProjectFiles(page);
  await expect(page.getByRole('tablist', { name: 'Preview tabs' })).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts', s = await import(/* @vite-ignore */path);
    s.setWindowAway(true);
    s.flashWithUndo({ message: 'Install failed: the candidate could not be verified. The previous app is still installed.', kind: 'error', ttlMs: 0, actions: [{ label: 'Reveal log', run: () => {} }] });
    for (let i = 0; i < 5; i++) s.flashWithUndo({ message: `Saved notice ${i}`, ttlMs: 30000 });
  });
  const shelf = page.locator('.snackbar-stack');
  await expect(shelf.locator('.snackbar')).toHaveCount(6);
  expect(await shelf.evaluate(el => {
    const a = el.getBoundingClientRect();
    const controls = [...document.querySelectorAll('[aria-label="Preview tabs"], .top-bar button, .chat-send, .rail [aria-label="Settings"]')];
    if (!controls.some(control => control.matches('[aria-label="Preview tabs"]')) || !controls.some(control => control.matches('.top-bar button')) || !controls.some(control => control.matches('.rail [aria-label="Settings"]'))) throw new Error('Missing protected navigation control');
    return controls.some(control => {
      const b = control.getBoundingClientRect();
      return b.width > 0 && b.height > 0 && a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    });
  })).toBe(false);
  expect(await shelf.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
  const reveal = shelf.getByRole('button', { name: 'Reveal log', exact: true });
  await reveal.focus();
  await expect(reveal).toBeInViewport({ ratio: 1 });
  expect(await reveal.evaluate(el => {
    const button = el.getBoundingClientRect(), shelf = el.closest('.snackbar-stack')!.getBoundingClientRect();
    return button.top >= shelf.top && button.bottom <= shelf.bottom;
  })).toBe(true);
  await page.keyboard.press('Tab');
  await expect(shelf.getByRole('alert').getByRole('button', { name: 'Dismiss', exact: true })).toBeFocused();
});

test('a successful install with an unverified running app remains an alert', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSelfInstallResult', JSON.stringify({
    id: 'warning-fixture', ok: true, state: 'installed', branch: 'main', commit: 'abc1234', message: 'Running app is still old1234', log: '/Users/fixture/Library/Logs/Hollow/fixture.log',
  })));
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  const notice = page.locator('.snackbar-stack').getByRole('alert').filter({ hasText: 'Running app is still old1234' });
  await expect(notice).toBeVisible();
  await expect(notice.getByRole('button', { name: 'Reveal log', exact: true })).toBeVisible();
  await expect(notice.getByRole('button', { name: "What's new in this build", exact: true })).toHaveCount(0);
});

test('keyboard focus stops a notification countdown', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await page.clock.install();
  await page.evaluate(() => {
    (window as unknown as { __mockEmit: (name: string, payload: unknown) => void }).__mockEmit('notification-event', {
      id: 'held-toast', title: 'Fixture notification', body: 'Keep this readable', kind: 'success', source: 'run-guard', created: new Date().toISOString(), read: false,
    });
  });
  const toast = page.locator('.notif-toast', { hasText: 'Fixture notification' });
  await expect(toast).toBeVisible();
  await toast.getByRole('button', { name: 'Dismiss notification' }).focus();
  await page.clock.runFor(20000);
  await expect(toast).toBeVisible();
  // Focus leaves the toast for the bell (the rail's Review row went in U3).
  await waitingBell(page).focus();
  await page.clock.runFor(9000);
  await expect(toast).toHaveCount(0);
});

test('failed install is a sticky alert in the stack with a mocked Reveal log', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSelfInstallResult', JSON.stringify({
    id: 'notice-fixture', ok: false, state: 'failed', branch: 'main', commit: 'abc1234', message: 'Fixture failure', log: '/Users/fixture/Library/Logs/Hollow/fixture.log',
  })));
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  const notice = page.locator('.snackbar-stack').getByRole('alert').filter({ hasText: 'Install failed' });
  await expect(notice).toBeVisible();
  await page.clock.install();
  await page.clock.runFor(60000);
  await expect(notice).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/api.ts';
    const { api } = await import(/* @vite-ignore */path);
    api.chatPreviewOpen = async (path: string, reveal: boolean) => {
      document.body.dataset.revealLog = JSON.stringify({ path, reveal });
      return 'revealed';
    };
  });
  await notice.getByRole('button', { name: 'Reveal log', exact: true }).click();
  await expect(page.locator('body')).toHaveAttribute('data-reveal-log', JSON.stringify({ path: '/Users/fixture/Library/Logs/Hollow/fixture.log', reveal: true }));
  await expect(notice).toBeVisible();
  await notice.getByRole('button', { name: 'Dismiss', exact: true }).click();
  await expect(notice).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('mockSelfInstallAcked'))).toBe('1');
});

test('a failure waits while the window is away, then keeps its remaining seven seconds', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.clock.install();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts';
    const s = await import(/* @vite-ignore */path);
    s.setWindowAway(true);
    s.flashFooter('× Fixture merge failed', 1500);
  });
  await page.clock.runFor(20000);
  const bar = page.locator('.snackbar', { hasText: 'Fixture merge failed' });
  await expect(bar).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts';
    (await import(/* @vite-ignore */path)).setWindowAway(false);
  });
  await page.clock.runFor(6500);
  await expect(bar).toBeVisible();
  await page.clock.runFor(500);
  await expect(bar).toHaveCount(0);
});

test('evicted messages remain in the bell history with failure words and Copy', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/store/runtime.ts';
    const s = await import(/* @vite-ignore */path);
    s.flashFooter('× Oldest fixture failed');
    for (let i = 0; i < 5; i++) s.flashWithUndo({ message: `Saved fixture ${i}`, ttlMs: 30000 });
  });
  await expect(page.locator('.snackbar')).toHaveCount(5);
  // Bell → Notifications → App messages (U3b; it was the waiting view's Recent notifications).
  await openBellHistory(page);
  await bellDialog(page).locator('.notification-app-messages > summary').click();
  const history = page.getByRole('region', { name: 'App messages', exact: true });
  await expect(history).toContainText('Oldest fixture failed');
  const row = history.locator('li', { hasText: 'Oldest fixture failed' });
  await expect(row).toContainText('Failed');
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async () => { throw new Error('Clipboard denied'); } });
  });
  await row.getByRole('button', { name: 'Copy', exact: true }).click();
  await expect(row.getByRole('alert')).toHaveText('Could not copy. Select the message and copy it.');
  await page.evaluate(() => {
    Object.defineProperty(navigator.clipboard, 'writeText', { configurable: true, value: async (text: string) => { document.body.dataset.copiedMessage = text; } });
  });
  await row.getByRole('button', { name: 'Copy', exact: true }).click();
  await expect(row.getByRole('button', { name: 'Copied', exact: true })).toBeVisible();
  await expect(row.getByRole('alert')).toHaveCount(0);
  await expect(page.locator('body')).toHaveAttribute('data-copied-message', 'Oldest fixture failed');
});

test('stack containers do not repeat item announcements', async ({ page }) => {
  await page.goto('/?mock=1&demo=studio');
  await expect(page.locator('.studio-home')).toBeVisible();
  await expect(page.locator('.snackbar-stack[aria-live], .notif-toast-stack[aria-live]')).toHaveCount(0);
});
