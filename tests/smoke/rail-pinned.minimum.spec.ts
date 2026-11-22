import { test, expect, type Page } from './fixtures/test';
import { showWorkspaceList } from './choose';

// Pinned workspaces at the native window's minimum (720 × 480) and at Largest
// text (2026-10-21): pin, reorder and unpin by visible controls only, and the
// sidebar's Pinned group fits. Runs only in the chromium-minimum projects.
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z' });
const session = (key: string, title: string, ws: string) => {
  const at = new Date(Date.now() - 5 * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

async function seed(page: Page) {
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('pinSeeded')) return;
    sessionStorage.setItem('pinSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2']));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
  }, [[workspace('alpha-1', 'Alpha with a fairly long workspace name'), workspace('beta-2', 'Beta')],
      [session('s-20261021-aaaa0001', 'Draft the memo', 'alpha-1'), session('s-20261021-bbbb0001', 'Beta thread', 'beta-2')]] as const);
  await page.goto('/?mock=1');
}
async function homeMenu(page: Page, title: string, item: string) {
  await showWorkspaceList(page);
  const more = page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: `Actions for ${title}` });
  await more.scrollIntoViewIfNeeded();
  await more.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: item, exact: true }).click();
}

test('pin, reorder and unpin with visible controls; the Pinned group fits', async ({ page }) => {
  await seed(page);
  await homeMenu(page, 'Alpha with a fairly long workspace name', 'Pin');
  await homeMenu(page, 'Beta', 'Pin');
  await homeMenu(page, 'Alpha with a fairly long workspace name', 'Move down');
  await expect(page.locator('.snackbar-msg').filter({ hasText: 'Moved Alpha with a fairly long workspace name to 2 of 2 in Pinned' })).toBeVisible();
  // Show the sidebar if this width starts with it hidden.
  const show = page.getByRole('button', { name: 'Show sidebar' });
  if (await show.isVisible()) await show.click();
  const rail = page.getByRole('navigation', { name: 'Project navigation' });
  // Pinned projects lead the list (overnight 11-04 S1), each with its pin;
  // it is headed Recent since L1 (11-12). The Quick chats heading (11-14)
  // follows it once its list loads, so the first heading is the one asked.
  await expect(rail.locator('.work-rail-project-heading .work-rail-label').first()).toHaveText('Recent');
  await expect(rail.locator(':scope > .work-tree .work-workspace-title')).toHaveText(['Beta', 'Alpha with a fairly long workspace name']);
  await expect(rail.locator(':scope > .work-tree .work-tree-pin')).toHaveCount(2);
  const [scroll, client] = await rail.evaluate(el => [el.scrollWidth, el.clientWidth]);
  expect(scroll, 'the Pinned group fits the sidebar').toBeLessThanOrEqual(client + 1);
  // The row's pin (its last mark; the row has no + since L8, 11-10) is on
  // screen, and the row's right-click menu works with its Unpin (the pin is
  // a mark since 2026-11-06; the row has no ⋯ since 2026-11-07).
  const head = rail.locator('.work-tree[data-project-id="beta-2"] .work-tree-head');
  const pin = head.locator('.work-tree-pin');
  const box = (await pin.boundingBox())!;
  expect(box.x + box.width).toBeLessThanOrEqual(page.viewportSize()!.width + 0.5);
  await head.click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Unpin', exact: true }).click();
  await expect(rail.locator(':scope > .work-tree .work-workspace-title').first()).toHaveText('Alpha with a fairly long workspace name');
});
