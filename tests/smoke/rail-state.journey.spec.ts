import { test, expect, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';

async function seed(page: Page) {
  await page.addInitScript(() => {
    if (sessionStorage.getItem('railStateSeeded')) return;
    sessionStorage.setItem('railStateSeeded', '1');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.setItem('sidebarCollapsed', 'true');
    const at = new Date().toISOString();
    const workspaces = [
      { id: 'pinned', title: 'Pinned project', pinned: 0 },
      { id: 'unpinned', title: 'Other project' },
      { id: 'quick-chats', title: 'Quick chats', kind: 'quick' },
    ].map(w => ({ ...w, status: 'active', path: `workspaces/${w.id}`, cwd: `/Users/fixture/vault/workspaces/${w.id}`, modified: at }));
    const sessions = workspaces.map((w, i) => ({ key: `s-20261025-rail000${i}`, title: ['Waiting thread', 'Failed thread', 'Quick thread'][i], agent: i === 1 ? 'codex' : 'claude', cwd: w.cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none', harnessSessionId: null }));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    localStorage.setItem('mockNeedsYouExtra', JSON.stringify([{ id: 'question:rail', kind: 'question', sourceId: 'rail', title: 'Choose one', detail: null, state: 'pending', at, workspaceId: 'pinned', sessionKey: sessions[0].key, path: null, category: 'question', link: null }]));
    localStorage.setItem('mockRunQueue', JSON.stringify([{ id: 9143, source: 'chat', sourceId: sessions[1].key, label: 'Chat', detail: 'Failed thread', reason: '', status: 'failed', priority: 0, position: null, attempt: 1, coalescedCount: 0, requestedAt: at, claimedAt: at, finishedAt: at, historyRunId: 'rail-failed', error: 'Fixture failure', waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false }]));
  });
}

test('collapsed rail exposes pinned and unpinned states, and a pinned icon opens its project’s lead in one click', async ({ page }) => {
  await page.setViewportSize({ width: 760, height: 600 });
  await seed(page);
  await page.goto('/?mock=1');
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  const pinned = rail.getByRole('button', { name: /Pinned project, pinned, 1 chat waiting on you/ });
  await expect.soft(pinned).toBeVisible();
  await expect.soft(pinned.getByRole('img', { name: '1 chat waiting on you', exact: true })).toBeVisible();
  const other = rail.getByRole('button', { name: 'Projects, 1 failed: Failed thread', exact: true });
  await expect.soft(other.getByRole('img', { name: '1 failed: Failed thread', exact: true })).toBeVisible();
  // No Chats icon since L8 (11-10): chats with no project are the COO's.
  await expect(rail.getByRole('button', { name: /^Chats(,|$)/ })).toHaveCount(0);
  await pinned.click();
  await expect(page.locator('.work-header h1')).toHaveText('Pinned project');
  expect(await page.evaluate(async moduleUrl => (await import(moduleUrl)).chatAgentSessionKey(), '/src/store/runtime.ts')).toMatch(/^s-project-[a-f0-9]{48}$/);
});

test('collapsed rail has its own workspace list failure and working Retry', async ({ page }) => {
  await seed(page);
  await page.addInitScript(() => localStorage.setItem('mockWorkspaceListError', '1'));
  await page.goto('/?mock=1');
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  await expect(rail.getByRole('alert')).toContainText("Couldn't load workspaces");
  const retry = rail.getByRole('button', { name: 'Retry loading workspaces' });
  await expect(retry).toBeVisible();
  await page.evaluate(() => localStorage.removeItem('mockWorkspaceListError'));
  await retry.click();
  await expect(rail.getByRole('alert')).toHaveCount(0);
  await expect(rail.getByRole('button', { name: /^Pinned project, pinned/ })).toBeVisible();
});

test('the rail shows a loading skeleton until the first list resolves', async ({ page }) => {
  await seed(page);
  await page.addInitScript(() => {
    const w = window as any;
    w.__mockHoldOnce = { command: 'list_workspaces', release: new Promise(resolve => { w.releaseRailList = resolve; }) };
  });
  await page.goto('/?mock=1');
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  await expect(rail.getByRole('status', { name: 'Loading workspaces' })).toBeVisible();
  await page.evaluate(() => (window as any).releaseRailList());
  await expect(rail.getByRole('status', { name: 'Loading workspaces' })).toHaveCount(0);
  await expect(rail.getByRole('button', { name: /^Pinned project, pinned/ })).toBeVisible();
});

test('narrow workspace top bar keeps a named live status glyph', async ({ page }) => {
  await seed(page);
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /^Pinned project, pinned/ }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Pinned project');
  await page.setViewportSize({ width: 760, height: 480 });
  const status = page.locator('.top-bar .work-status');
  await expect(status).toBeVisible();
  await expect(status).toHaveAccessibleName('1 chat waiting on you');
  await expect(status.locator('.session-indicator')).toBeVisible();
  await expect(status.locator('.work-status-words')).toBeHidden();
});

for (const theme of ['light', 'dark']) test(`expanded narrow rail fixture metadata has AA contrast in ${theme}`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 480 });
  await seed(page);
  await page.addInitScript(theme => { localStorage.setItem('theme', theme); localStorage.removeItem('glass'); }, theme);
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /^Pinned project, pinned/ }).click();
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  // The open project's row and its waiting roll-up (the sidebar lists no
  // conversation rows since L8, 11-10).
  await expect(page.locator('.work-tree-head.current')).toBeVisible();
  await expect(page.locator('.work-tree-head.current .session-rollup')).toBeVisible();
  expect((await new AxeBuilder({ page }).include('.work-rail').withTags(['wcag2a', 'wcag2aa']).analyze()).violations).toEqual([]);
});
