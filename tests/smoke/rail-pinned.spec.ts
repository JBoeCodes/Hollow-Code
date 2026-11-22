import { showWorkspaceList } from './choose';
import { test, expect, type Page } from './fixtures/test';

// Pinned workspaces (2026-10-21): pinned projects lead the sidebar's Projects
// (overnight 11-04 S1; a Pinned group of their own until then), like
// pinned chats in the ChatGPT Mac app. Pin / Unpin / Move up / Move down in
// the workspace menus (a sidebar row's is its right-click; its pin is a mark
// since 2026-11-06 and it has no ⋯ since 11-07), drag to reorder, the
// collapsed sidebar's icons, and Home in the same order. The pin is one state
// (each brief's `pinned`; here the mock's workspace list).
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string, extra: Record<string, unknown> = {}) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z', ...extra });
const session = (key: string, title: string, ws: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

async function seed(page: Page, opts: { legacyPins?: string[]; workspaces?: unknown[] } = {}) {
  await page.addInitScript(([workspaces, sessions, legacy]) => {
    if (sessionStorage.getItem('pinSeeded')) return;
    sessionStorage.setItem('pinSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2']));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    if (legacy) localStorage.setItem('hollow.studio.pins', JSON.stringify(legacy));
  }, [
    opts.workspaces ?? [
      workspace('alpha-1', 'Alpha'), workspace('beta-2', 'Beta'), workspace('gamma-3', 'Gamma'),
      // A stale pin in an archived brief never shows.
      workspace('old-4', 'Old', { status: 'archived', pinned: 1 }),
    ],
    [session('s-20261021-aaaa0001', 'Draft the launch memo', 'alpha-1', 5), session('s-20261021-bbbb0001', 'Beta thread', 'beta-2', 60)],
    opts.legacyPins ?? null,
  ] as const);
  await page.goto('/?mock=1');
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
/** A project's one sidebar row and its marks (L8, 11-10: no conversations under it). */
const tree = (page: Page, title: string) => rail(page).locator('.work-tree')
  .filter({ has: page.locator('.work-workspace-title', { hasText: new RegExp(`^${title}$`) }) });
/** The Projects section's headers, top to bottom; a pinned one (its pin
 *  mark) reads `pinned:<title>`. Pinned projects lead Projects since
 *  overnight 11-04 S1, instead of a Pinned group of their own. */
const railOrder = (page: Page) => rail(page).locator(':scope > .work-tree')
  .evaluateAll(els => els.map(el => `${el.querySelector('.work-tree-pin') ? 'pinned:' : ''}${el.querySelector('.work-workspace-title')?.textContent ?? ''}`));
async function menu(page: Page, title: string, item: string) {
  await tree(page, title).locator('.work-tree-head').click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: item, exact: true }).click();
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('Pin, Move and Unpin from the menus; each pinned workspace is listed once', async ({ page }) => {
  await seed(page);
  await expect(tree(page, 'Alpha')).toBeVisible();
  // Nothing pinned: no Pinned group (the archived pin doesn't count).
  await expect.poll(() => railOrder(page)).toEqual(['Alpha', 'Beta']);

  await menu(page, 'Beta', 'Pin');
  await expect(page.locator('.snackbar-msg').filter({ hasText: 'Pinned Beta' })).toBeVisible();
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Beta', 'Alpha']);
  await expect(tree(page, 'Beta')).toHaveCount(1);

  // Gamma has no conversation, so it isn't in the rail until pinned (from Home's menu).
  await showWorkspaceList(page);
  await page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: 'Actions for Gamma' }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Pin', exact: true }).click();
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Beta', 'pinned:Gamma', 'Alpha']);

  // Keyboard reorder: Move up / Move down, disabled at the ends, announced.
  await tree(page, 'Beta').locator('.work-tree-head').click({ button: 'right' });
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Move up', exact: true })).toBeDisabled();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Move down', exact: true }).click();
  await expect(page.locator('.snackbar-msg').filter({ hasText: 'Moved Beta to 2 of 2 in Pinned' })).toBeVisible();
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Gamma', 'pinned:Beta', 'Alpha']);

  // Home's recent list no longer follows pin rank; rail order remains durable.
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();

  // Survives a reload (the pin is in the workspace list, not this window).
  await page.reload();
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Gamma', 'pinned:Beta', 'Alpha']);

  // A pinned row's pin is a quiet mark and the row has no ⋯ (11-07); its
  // right-click menu holds Unpin. Beta goes back to Workspaces.
  await tree(page, 'Beta').locator('.work-tree-head').hover();
  await expect(tree(page, 'Beta').getByRole('img', { name: 'Pinned', exact: true })).toBeVisible();
  await expect(tree(page, 'Beta').getByRole('button', { name: 'Workspace actions for Beta' })).toHaveCount(0);
  await menu(page, 'Beta', 'Unpin');
  await expect(page.locator('.snackbar-msg').filter({ hasText: 'Unpinned Beta' })).toBeVisible();
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Gamma', 'Alpha', 'Beta']);

  // Unpin from the menu: nothing pinned, the group goes.
  await menu(page, 'Gamma', 'Unpin');
  await expect.poll(() => railOrder(page)).toEqual(['Alpha', 'Beta']);
});

test('drag reorders Pinned; an archived workspace offers no Pin', async ({ page }) => {
  await seed(page, { workspaces: [
    workspace('alpha-1', 'Alpha', { pinned: 1 }), workspace('beta-2', 'Beta', { pinned: 2 }), workspace('gamma-3', 'Gamma', { pinned: 3 }),
    workspace('old-4', 'Old', { status: 'archived' }),
  ] });
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Alpha', 'pinned:Beta', 'pinned:Gamma']);
  const head = (title: string) => tree(page, title).locator('.work-tree-head');
  const box = await head('Alpha').boundingBox();
  await tree(page, 'Gamma').locator('.work-workspace-row').dragTo(head('Alpha'), { targetPosition: { x: 20, y: 4 } });
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Gamma', 'pinned:Alpha', 'pinned:Beta']);
  expect(box).not.toBeNull();
  await tree(page, 'Gamma').locator('.work-workspace-row').dragTo(head('Beta'), { targetPosition: { x: 20, y: box!.height - 4 } });
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Alpha', 'pinned:Beta', 'pinned:Gamma']);

  // Archived: Home's menu has no Pin, and its pin button says why.
  await showWorkspaceList(page);
  await page.getByRole('checkbox', { name: 'Include archived' }).check();
  await page.getByRole('button', { name: 'Actions for Old' }).click();
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Pin', exact: true })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Pin Old' })).toHaveCount(0);
});

test('a drag counts pins hidden from the sidebar, as the backend does', async ({ page }) => {
  await page.addInitScript(() => { if (!sessionStorage.getItem('pinSeeded')) localStorage.setItem('hollow.railHidden', JSON.stringify(['hidden-9'])); });
  await seed(page, { workspaces: [
    workspace('alpha-1', 'Alpha', { pinned: 1 }), workspace('hidden-9', 'Hidden', { pinned: 2 }),
    workspace('beta-2', 'Beta', { pinned: 3 }), workspace('gamma-3', 'Gamma', { pinned: 4 }),
  ] });
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Alpha', 'pinned:Beta', 'pinned:Gamma']);
  const head = tree(page, 'Gamma').locator('.work-tree-head');
  const box = await head.boundingBox();
  await tree(page, 'Alpha').locator('.work-workspace-row').dragTo(head, { targetPosition: { x: 20, y: box!.height - 4 } });
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Beta', 'pinned:Gamma', 'pinned:Alpha']);
});

test('the collapsed sidebar shows pinned workspaces on top, and old window pins move into the briefs', async ({ page }) => {
  await seed(page, { legacyPins: ['gamma-3', 'gone', 'old-4', 'alpha-1'] });
  // The window's old list is adopted in order (gone and archived skipped), then dropped.
  await expect.poll(() => railOrder(page)).toEqual(['pinned:Gamma', 'pinned:Alpha', 'Beta']);
  await expect.poll(() => page.evaluate(() => localStorage.getItem('hollow.studio.pins'))).toBeNull();
  await page.getByRole('button', { name: 'Hide sidebar' }).click();
  await expect(page.locator('.work-rail')).toHaveClass(/collapsed/);
  const mini = page.getByRole('group', { name: 'Pinned workspaces' });
  await expect(mini.getByRole('button')).toHaveCount(2);
  await expect(mini.getByRole('button', { name: 'Gamma, pinned' })).toBeVisible();
  // No Chats icon below them since L8 (11-10); a pinned icon opens the project's lead.
  await expect(page.locator('.work-rail-quick-mini')).toHaveCount(0);
  await mini.getByRole('button', { name: 'Alpha, pinned' }).click();
  await expect(mini.getByRole('button', { name: 'Alpha, pinned' })).toHaveClass(/selected/);
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  expect(await page.evaluate(async moduleUrl => (await import(moduleUrl)).chatAgentSessionKey(), '/src/store/runtime.ts')).toMatch(/^s-project-[a-f0-9]{48}$/);
});
