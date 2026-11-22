import { test, expect, type Page } from './fixtures/test';
import { showWorkspaceList } from './choose';

// Consolidation D (2026-10-13): no action is right-click only, and Help is
// reachable by clicking. Each "⋯" opens the same menu as the right-click.
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const at = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
async function seed(page: Page) {
  await page.addInitScript(() => {
    if (sessionStorage.getItem('menusSeeded')) return;
    sessionStorage.setItem('menusSeeded', '1');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1']));
  });
  await page.addInitScript(([ws, sessions]) => {
    if (sessionStorage.getItem('menusData')) return;
    sessionStorage.setItem('menusData', '1');
    localStorage.setItem('mockWorkspaces', JSON.stringify(ws));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    for (const x of sessions as any[]) conversations[x.key] = { summary: x, eventsRetained: true, turns: [
      { id: `${x.key}-u`, role: 'user', text: `Asked in ${x.title}`, tools: [], agent: 'claude', runId: `run-${x.key}`, startedAt: x.createdAt, transcriptIncomplete: false },
      { id: `${x.key}-a`, role: 'assistant', text: `Answer for ${x.title}`, tools: [], agent: 'claude', runId: `run-${x.key}`, startedAt: x.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [
    [{ id: 'alpha-1', title: 'Alpha', status: 'active', path: 'workspaces/alpha-1', cwd: cwd('alpha-1'), modified: at(5) }],
    [{ key: 's-20261013-aaaa0001', title: 'Draft the memo', agent: 'claude', harnessSessionId: null, cwd: cwd('alpha-1'), createdAt: at(5), lastActivityAt: at(5), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' }],
  ] as const);
  await page.goto('/?mock=1');
}

test('Home rows, the open workspace and its open conversation each have a visible menu button', async ({ page, viewport }) => {
  test.skip((viewport?.width ?? 1000) < 860, 'the desktop layout; the click-only journeys cover the minimum window');
  await seed(page);
  await showWorkspaceList(page);
  const homeMenu = page.getByRole('region', { name: 'Projects', exact: true }).getByRole('button', { name: 'Actions for Alpha' });
  await homeMenu.click();
  const menu = page.getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: 'Open project' })).toBeVisible();
  // Another conversation in a project is this menu's item since its sidebar + went (L8, 11-10).
  await expect(menu.getByRole('menuitem', { name: 'New conversation', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(menu).toHaveCount(0);
  // Keyboard: Enter on the "⋯" opens the menu with its first item focused.
  await homeMenu.focus();
  await page.keyboard.press('Enter');
  await expect(menu.getByRole('menuitem').first()).toBeFocused();
  await page.keyboard.press('Escape');

  // The sidebar lists no conversations (L8, 11-10): Work's card opens the
  // project's newest one, and the "⋯" beside its name is its menu.
  await page.locator('.work-card').filter({ hasText: 'Alpha' }).first().click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(page.locator('.chat-main')).toContainText('Answer for Draft the memo');
  const title = page.getByRole('group', { name: 'Conversation', exact: true });
  await expect(title.locator('.chat-title')).toHaveText('Draft the memo');
  await title.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await expect(page.getByRole('menu').getByRole('menuitem', { name: /^Move to project…/ })).toBeVisible();
  await page.keyboard.press('Escape');
});

test('Settings → Keyboard shortcuts → Show all keys opens every key', async ({ page, viewport }) => {
  test.skip((viewport?.width ?? 1000) <= 860, 'below 860px Settings sections are a selector (required.minimum covers it)');
  await page.goto('/?mock=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Keyboard shortcuts', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Take the tour' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Show all keys', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeVisible();
  await page.keyboard.press('Escape');
});
