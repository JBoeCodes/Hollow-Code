import { expect, test, type Page } from './fixtures/test';
import { navigateWaiting, waitingBell, waitingPanel } from './waiting';

/** U3: the queue opens from the top bar's bell, then See all needs you (the
 *  rail's Review row went). */
function reviewRail(page: Page) {
  return waitingBell(page);
}

/**
 * P1-13: Review brings you back to the queue. A failed read offers Try again,
 * a workspace filter narrows the rows and its counts follow, and the cards'
 * key hints show only while the letter keys work. Against the demo mock.
 */

test.beforeEach(({}, info) => { test.skip(info.project.name !== 'chromium-desktop', 'list states, not layout: one size covers them'); });

test('a failed waiting read offers Try again, which reads again', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(reviewRail(page)).toBeVisible();
  await page.evaluate(async () => {
    // @ts-expect-error dev fixture
    const { api } = await import('/src/api.ts');
    const w = window as unknown as { __fail: boolean };
    w.__fail = true;
    const real = api.listNeedsYou;
    api.listNeedsYou = async (...args: unknown[]) => { if (w.__fail) throw 'database is locked'; return real(...args); };
  });
  await navigateWaiting(page);
  const panel = waitingPanel(page);
  const alert = panel.getByRole('alert').filter({ hasText: 'Couldn’t load what is waiting' });
  await expect(alert).toBeVisible();
  await page.evaluate(() => { (window as unknown as { __fail: boolean }).__fail = false; });
  await alert.getByRole('button', { name: 'Try again' }).click();
  await expect(alert).toHaveCount(0);
});

test('the workspace filter narrows the rows, and the counts follow', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(reviewRail(page)).toBeVisible();
  const names = await page.evaluate(async () => {
    // @ts-expect-error dev fixture
    const { api } = await import('/src/api.ts');
    // @ts-expect-error dev fixture
    const { workspaces } = await import('/src/store/workspaces.ts');
    const [a, b] = (workspaces() as { id: string; title: string }[]).filter(w => w.title).slice(0, 2);
    const item = (id: string, ws: string) => ({ id, kind: 'failure', sourceId: id, title: `Item ${id}`, detail: null, state: 'pending', at: new Date().toISOString(), workspaceId: ws, sessionKey: null, path: null, category: null, link: null });
    const items = [item('f:1', a.id), item('f:2', a.id), item('f:3', b.id)];
    api.listNeedsYou = async () => ({ items, total: items.length, nextCursor: null, unavailable: [] });
    return [a.title, b.title];
  });
  await navigateWaiting(page);
  const panel = waitingPanel(page);
  const rows = panel.getByRole('button', { name: /^Item f:/ });
  await expect(rows).toHaveCount(3);
  const select = panel.getByRole('combobox', { name: /Workspace/ });
  await expect(select.locator('option')).toHaveText(['All workspaces', `${names[0]} (2)`, `${names[1]} (1)`]);
  await select.selectOption({ label: `${names[1]} (1)` });
  await expect(rows).toHaveCount(1);
  await expect(panel.locator('[data-group="failures"] .rail-group-count')).toHaveText('1');
  await select.selectOption({ label: 'All workspaces' });
  await expect(rows).toHaveCount(3);
});

test('with shortcuts off, review cards show no key hints', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await navigateWaiting(page);
  const panel = waitingPanel(page);
  // A row that opens its card in place (it has aria-expanded): a learning's
  // row is a link to Library Learnings instead (#1238), with no card here.
  await panel.locator('[data-group="reviews"] [data-waiting-row][aria-expanded]').first().click();
  await expect(panel.locator('.review-key-hint').first()).toContainText('⇧R');
  await page.evaluate(async () => {
    // @ts-expect-error dev fixture
    const { setShortcutSettings, shortcutSettings } = await import('/src/shortcuts.ts');
    setShortcutSettings({ ...shortcutSettings(), enabled: false });
  });
  await expect(panel.locator('.review-key-hint')).toHaveCount(0);
});
