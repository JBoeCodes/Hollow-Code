import { showWorkspaceList, projectFilesButton } from './choose';
/**
 * Journey J1 (consolidation lane B, 2026-10-13): resume the right workspace
 * when several share a name, using visible controls only, no palette, no
 * keys. A project folder, a Hollow workspace, a Notion mirror and a build
 * workspace are all called "Q3 review"; the rail, Home's "Pick up where you
 * left off" cards, Every workspace and the open workspace's top bar each say
 * which one is which (src/workspaceOrigin.ts), and opening one shows its own
 * conversation. A long shared title keeps its controls on screen. A required
 * spec: it runs at the desktop size and at the native minimum window, with
 * and without Largest text, and may not skip (scripts/check-required-specs.mjs).
 */
import { createHash } from 'node:crypto';
import { expect, test, type Locator, type Page } from './fixtures/test';

const LONG = 'Quarterly regional operations review for the Northeast, Mid-Atlantic and Southeast parking portfolios';
const NOTION = 'notion-0123456789abcdef0123456789abcdef';
const NOTION_LONG = 'notion-fedcba9876543210fedcba9876543210';
const at = (m: number) => new Date(Date.now() - m * 60_000).toISOString();
const ws = (id: string, title: string, cwd: string, minutesAgo: number, extra: Record<string, unknown> = {}) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd, modified: at(minutesAgo), ...extra });
const vault = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const WORKSPACES = [
  // First, so the mock's icon owner (example-user) is this folder's.
  ws('local-q3-folder', 'Q3 review', '/Users/fixture/Projects/q3-review', 50),
  ws('q3-hollow', 'Q3 review', vault('q3-hollow'), 40),
  ws(NOTION, 'Q3 review', vault('notion-q3'), 30),
  ws('q3-build', 'Q3 review', '/Users/fixture/Hollow/Worktrees/q3/q3-review', 20, { build: { repo: '/Users/fixture/Projects/q3', worktree: '/Users/fixture/Hollow/Worktrees/q3/q3-review', branch: 'feature/q3-review', base: 'origin/main', exists: true } }),
  ws('long-hollow', LONG, vault('long-hollow'), 15),
  ws(NOTION_LONG, LONG, vault('notion-long'), 10),
];
/** The project lead's key, as the backend makes it (`s-project-` and the
 *  first 48 hex of sha256(id)), as in rail-tree.spec.ts. */
const leadKey = (id: string) => `s-project-${createHash('sha256').update(id).digest('hex').slice(0, 48)}`;
/** One conversation per workspace, its lead (L8, 11-10: a project's sidebar
 *  row opens its lead, Home's card its newest chat), each saying whose it is. */
const SESSIONS = WORKSPACES.map(w => ({
  key: leadKey(w.id), title: `Notes from ${w.id}`, agent: 'claude', harnessSessionId: null, cwd: w.cwd,
  createdAt: w.modified, lastActivityAt: w.modified, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none',
}));

async function seed(page: Page) {
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('j1Seeded')) return;
    sessionStorage.setItem('j1Seeded', '1');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify((workspaces as { id: string }[]).map(w => w.id).reverse()));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    for (const s of sessions as { key: string; title: string; createdAt: string }[]) {
      const owner = s.title.replace('Notes from ', '');
      conversations[s.key] = { summary: s, eventsRetained: true, turns: [
        { id: `${s.key}-u`, role: 'user', text: `Where does ${owner} stand?`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
        { id: `${s.key}-a`, role: 'assistant', text: `This is the ${owner} conversation.`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
      ] };
    }
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [WORKSPACES, SESSIONS] as const);
  await page.goto('/?mock=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
/** At the minimum window the sidebar starts hidden: show it by its button
 *  (and say whether it was hidden, to put it back). */
async function showSidebar(page: Page): Promise<boolean> {
  const show = page.getByRole('button', { name: 'Show sidebar', exact: true });
  if (!(await show.isVisible())) return false;
  await show.click();
  await expect(page.getByRole('button', { name: 'Hide sidebar', exact: true })).toBeVisible();
  return true;
}
/** Over the page, the sidebar covers the top bar's toggle: it has its own
 *  Close (choosing a row also closes it). */
async function hideSidebar(page: Page, wasHidden: boolean) {
  const close = page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Close sidebar', exact: true });
  if (wasHidden && await close.isVisible()) await close.click();
  if (wasHidden) await expect(page.getByRole('button', { name: 'Show sidebar', exact: true })).toBeVisible();
}
const yourWork = (page: Page) => page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true });
const resume = (page: Page) => page.getByRole('region', { name: 'Projects' });
const crumb = (page: Page) => page.locator('.top-bar-crumbs .crumb-project');
async function expectOpen(page: Page, id: string, origin: string) {
  const title = WORKSPACES.find(w => w.id === id)!.title;
  await expect(page.locator('.work-header h1')).toHaveText(title);
  // The bar names the origin on hover and to screen readers; a visible label
  // there read "Hollow works…" (11-06). The rail and Home still show it.
  await expect(crumb(page)).toHaveAttribute('title', `${title} (${origin})`);
  await expect(crumb(page)).toContainText(`(${origin})`);
  await expect(crumb(page).locator('.crumb-origin')).toHaveCount(0);
  const chat = page.locator('.chat-main');
  await expect(chat).toContainText(`This is the ${id} conversation.`);
  for (const other of WORKSPACES.filter(w => w.id !== id)) await expect(chat).not.toContainText(`This is the ${other.id} conversation.`);
}
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}
/** Wholly inside the window, left to right: not clipped off the edge. */
async function onScreen(page: Page, target: Locator) {
  await expect(target).toBeVisible();
  const box = (await target.boundingBox())!;
  const width = page.viewportSize()!.width;
  expect(box.x, 'starts inside the window').toBeGreaterThanOrEqual(0);
  expect(box.x + box.width, 'ends inside the window').toBeLessThanOrEqual(width);
}

test('J1 click-only: resume the right workspace among same-named ones', async ({ page }) => {
  await seed(page);

  // The rail tells the four "Q3 review"s apart by where each comes from.
  const hidden = await showSidebar(page);
  for (const origin of ['Project folder · example-user', 'Hollow workspace', 'Notion project', 'Build · feature/q3-review']) {
    const row = rail(page).getByRole('button', { name: `Q3 review, ${origin}`, exact: true });
    await expect(row).toBeVisible();
    await expect(row.locator('.work-workspace-origin')).toHaveText(origin);
  }
  // One row each, with no conversations under it (L8, 11-10: a row opens its project's lead).
  await expect(rail(page).locator('.work-session-row, .work-tree-sessions')).toHaveCount(0);
  await hideSidebar(page, hidden);

  // Home's return cards say it too; resume the Hollow workspace from its card.
  await showWorkspaceList(page);
  const cards = resume(page);
  await expect(cards.getByRole('button', { name: /^Open Q3 review \(/ })).toHaveCount(4);
  const hollowCard = cards.getByRole('button', { name: 'Open Q3 review (Hollow workspace)', exact: true });
  await expect(hollowCard.locator('.ui-row-meta')).toContainText('Hollow workspace');
  await expect(cards.getByRole('button', { name: 'Actions for Q3 review (Hollow workspace)' })).toBeVisible();
  await hollowCard.click();
  await expectOpen(page, 'q3-hollow', 'Hollow workspace');
  await showSidebar(page);
  await expect(rail(page).getByRole('button', { name: 'Q3 review, Hollow workspace', exact: true })).toHaveAttribute('aria-current', 'true');
  await hideSidebar(page, hidden);
  await noHorizontalScroll(page);

  // Every workspace: the Notion mirror, by its line.
  await yourWork(page).click();
  const list = page.getByRole('region', { name: 'Projects' });
  await showWorkspaceList(page);
  const notionRow = list.locator('.work-card').filter({ hasText: 'Notion project' }).filter({ hasText: /^Q3 review/ });
  await expect(notionRow).toHaveCount(1);
  await expect(list.getByRole('button', { name: 'Actions for Q3 review (Notion project)' })).toBeVisible();
  await notionRow.click();
  await expectOpen(page, NOTION, 'Notion project');

  // The rail: the build workspace, which shows its branch as well.
  await showSidebar(page);
  await rail(page).getByRole('button', { name: 'Q3 review, Build · feature/q3-review', exact: true }).click();
  await expectOpen(page, 'q3-build', 'Build · feature/q3-review');
  await expect(page.locator('.work-branch')).toContainText('feature/q3-review');
  await hideSidebar(page, hidden);

  // A long shared title: cut with an ellipsis, its origin and controls on screen.
  await yourWork(page).click();
  await showWorkspaceList(page);
  const longCard = resume(page).getByRole('button', { name: `Open ${LONG} (Notion project)`, exact: true });
  await expect(longCard.locator('.ui-row-meta')).toContainText('Notion project');
  await onScreen(page, resume(page).getByRole('button', { name: `Actions for ${LONG} (Notion project)` }));
  await noHorizontalScroll(page);
  await longCard.click();
  await expectOpen(page, NOTION_LONG, 'Notion project');
  await expect(crumb(page)).toHaveAttribute('title', `${LONG} (Notion project)`);
  const everywhere = page.getByRole('group', { name: 'Everywhere' });
  // Project files stands where the workspace panel button was (COO rebuild
  // L4; overnight 11-04 U4 before it).
  // The global group's controls (Search left it for the sidebar in U1).
  for (const control of [everywhere.getByRole('button', { name: 'Notifications', exact: true }), everywhere.getByRole('button', { name: /^COO ·/ }), projectFilesButton(page)]) {
    await onScreen(page, control);
  }
  await expect(page.getByLabel('Chat message')).toBeVisible();
  await noHorizontalScroll(page);
});
