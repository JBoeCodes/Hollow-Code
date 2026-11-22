import { test, expect, type Page } from './fixtures/test';

// Conductor-style rail rows (2026-09-28): a build workspace's row shows its
// branch tinted by PR state, and a project's row rolls up its running chats
// (asks 10-11 A1, A2). Since L8 (11-10) the sidebar lists one row per
// project and no conversation rows, so a chat's own indicator and `+a -d`
// line counts are no longer there.
const vaultCwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const tree = '/Users/fixture/Hollow/Worktrees/hollow/rail';
const session = (key: string, title: string, cwd: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

async function seed(page: Page, pr: string | null) {
  await page.addInitScript(([workspaces, sessions, pr]) => {
    if (sessionStorage.getItem('gitRailSeeded')) return;
    sessionStorage.setItem('gitRailSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['build-1', 'plain-2']));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    localStorage.setItem('mockSessionChanges', JSON.stringify({ 's-20260928-git00001': [1234, 5], 's-20260928-git00003': [55, 5] }));
    localStorage.setItem('mockRailPr', JSON.stringify(pr ? { 'build-1': pr } : {}));
    localStorage.setItem('mockRunQueue', JSON.stringify([{ id: 9, source: 'chat', sourceId: 's-20260928-git00004', label: 'Chat', detail: 'Keep going', reason: '', status: 'running', priority: 0, position: null, attempt: 1, coalescedCount: 0, requestedAt: new Date().toISOString(), claimedAt: new Date().toISOString(), finishedAt: null, historyRunId: 'run-9', error: null, waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false }]));
  }, [
    [
      { id: 'build-1', title: 'Rail polish', status: 'active', path: 'workspaces/build-1', cwd: tree, modified: '2026-09-28T12:00:00Z', build: { repo: '/Users/fixture/Projects/hollow', worktree: tree, branch: 'hollow/rail', base: 'origin/main', exists: true } },
      { id: 'plain-2', title: 'Weekly report', status: 'active', path: 'workspaces/plain-2', cwd: vaultCwd('plain-2'), modified: '2026-09-28T12:00:00Z' },
    ],
    [
      session('s-20260928-git00001', 'Tighten the rail', tree, 5),
      session('s-20260928-git00002', 'Look at spacing', tree, 60),
      session('s-20260928-git00003', 'Draft the one-pager', vaultCwd('plain-2'), 30),
      session('s-20260928-git00004', 'Keep going', vaultCwd('plain-2'), 1),
    ],
    pr,
  ] as const);
  await page.goto('/?mock=1');
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

// Since U2 (#1500, 11-16) a project whose chats only run in the background
// sits in the counted Working fold, closed by default; open it to reach the row.
async function openWorking(page: Page) {
  const working = page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Working projects (1)', exact: true });
  await expect(working).toHaveAttribute('aria-expanded', 'false');
  await working.click();
  await expect(working).toHaveAttribute('aria-expanded', 'true');
}

test('a project row shows its git state, and a running chat in it as a roll-up', async ({ page }) => {
  await seed(page, 'merged');
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  await expect(rail.locator('.work-tree-head .work-workspace-git')).toHaveAttribute('title', /PR #12 merged · hollow\/rail/);
  // No conversation rows, so nothing of a chat's own (provider mark, line counts).
  await expect(rail.locator('.work-session-row, .work-session-diff')).toHaveCount(0);
  await expect(rail).not.toContainText('Tighten the rail');
  // The running chat shows on its project's row, readable without motion or colour.
  await openWorking(page);
  const head = rail.locator('.work-tree-head', { hasText: 'Weekly report' });
  await expect(head.locator('.session-rollup[data-state="running"]')).toBeVisible();
  await expect(head.getByRole('button', { name: /^Weekly report, 1 chat running/ })).toBeVisible();
  await expect(rail.locator('.work-tree-head', { hasText: 'Rail polish' }).locator('.session-rollup')).toHaveCount(0);
  await page.locator('.work-rail').screenshot({ path: test.info().outputPath('rail-git-status.png') });
});

test('an open PR and a branch without one have their own glyphs', async ({ page }) => {
  await seed(page, 'open');
  await expect(page.locator('.work-tree-head .work-workspace-git.open')).toHaveCount(1);
});

test('a branch without a PR shows the plain branch glyph', async ({ page }) => {
  await seed(page, null);
  const glyph = page.locator('.work-tree-head .work-workspace-git.branch');
  await expect(glyph).toHaveCount(1);
  await expect(glyph).toHaveAttribute('title', /hollow\/rail · 1 commit ahead, no PR/);
});

test('a project row rolls up its conversations: the most urgent state, a count and their names (asks 10-11 A1)', async ({ page }) => {
  await seed(page, 'merged');
  // The seed runs once per tab, so this sticks across the reload.
  await page.evaluate(() => {
    const queue = JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]');
    queue.push({ ...queue[0], id: 10, sourceId: 's-20260928-git00003', status: 'running' });
    localStorage.setItem('mockRunQueue', JSON.stringify(queue));
  });
  await page.reload();
  await openWorking(page);
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  const head = rail.locator('.work-tree-head', { hasText: 'Weekly report' });
  const badge = head.locator('.session-rollup[data-state="running"]');
  await expect(badge).toBeVisible();
  await expect(badge.locator('.session-rollup-count')).toHaveText('2');
  await expect(head.getByRole('button', { name: /Weekly report, 2 chats running/ })).toBeVisible();
  // There is nothing to unfold (L8, 11-10): a click opens the project's lead,
  // and the row keeps saying it.
  await head.getByRole('button', { name: /^Weekly report/ }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Weekly report');
  await expect(badge).toBeVisible();
  await expect(rail.locator('.work-session-row')).toHaveCount(0);
});

test('a project whose only live work is a subagent shows it, named, on its row (11-11)', async ({ page }) => {
  await seed(page, 'merged');
  await page.evaluate(() => {
    const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]');
    // The same session as a subagent, running; the chat in the project is idle.
    const sub = { ...sessions[0], key: 's-20260928-sub00001', title: 'Check the numbers', cwd: '/Users/fixture/vault/workspaces/plain-2', parentKey: 's-20260928-git00003', forkKind: 'subagent' };
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([...sessions, sub]));
    const queue = JSON.parse(localStorage.getItem('mockRunQueue') ?? '[]');
    localStorage.setItem('mockRunQueue', JSON.stringify(queue.map((q: { sourceId: string }) => ({ ...q, sourceId: 's-20260928-sub00001' }))));
  });
  await page.reload();
  await openWorking(page);
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  const head = rail.locator('.work-tree-head', { hasText: 'Weekly report' });
  await expect(head.locator('.session-rollup[data-state="running"]')).toBeVisible();
  await expect(head.getByRole('button', { name: /^Weekly report, 1 subagent running/ })).toBeVisible();
  await page.locator('.work-rail').screenshot({ path: test.info().outputPath('project-row-running.png') });
});
