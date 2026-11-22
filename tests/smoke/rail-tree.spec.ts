import { createHash } from 'node:crypto';
import { openChat, openTasks, showWorkspaceList } from './choose';
import { test, expect, type Page } from './fixtures/test';

// The rail's projects: one row per project that opens its lead (L8, 11-10),
// with the project's menu on its right-click. The sidebar lists no
// conversations: a project's other chats are under the lead's Subagents
// chip, Earlier chats, and chats with no project under the COO's.
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string, status = 'active') =>
  ({ id, title, status, path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z' });
const session = (key: string, title: string, ws: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

const DEFAULT_WORKSPACES = [workspace('alpha-1', 'Alpha'), workspace('beta-2', 'Beta'), workspace('notion-abc00000000000000000000000000000', 'Mirrored')];
async function seed(page: Page, workspaces: Record<string, unknown>[] = DEFAULT_WORKSPACES, extra: Record<string, unknown>[] = []) {
  await page.addInitScript(() => {
    if (sessionStorage.getItem('railSeeded')) return;
    sessionStorage.setItem('railSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    // Workspaces used here, listed first; then any other with conversations.
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2', 'notion-abc00000000000000000000000000000']));
  });
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('railSeededData')) return;
    sessionStorage.setItem('railSeededData', '1');
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    for (const s of sessions as any[]) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [
    workspaces,
    [session('s-20260926-aaaa0001', 'Draft the launch memo', 'alpha-1', 5), session('s-20260926-aaaa0002', 'Review pricing', 'alpha-1', 180),
     ...Array.from({ length: 6 }, (_, i) => session(`s-20260926-bbbb000${i}`, `Beta thread ${i}`, 'beta-2', 60 * (i + 1))), ...extra],
  ] as const);
  await page.goto('/?mock=1');
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
/** A project's one sidebar row (its name, plus a live roll-up when one runs). */
const row = (page: Page, title: string) => rail(page).getByRole('button', { name: new RegExp(`^${title}(,|$)`) });
/** The project lead's key, as the backend makes it (`s-project-` and the first 48 hex of sha256(id)). */
const leadKey = (id: string) => `s-project-${createHash('sha256').update(id).digest('hex').slice(0, 48)}`;
const openKey = (page: Page) => page.evaluate(async moduleUrl => (await import(moduleUrl)).chatAgentSessionKey(), '/src/store/runtime.ts');
const chip = (page: Page) => page.getByRole('button', { name: /^Subagents:/ });
const subagentsTab = (page: Page) => page.getByRole('complementary', { name: 'Preview' }).getByRole('tabpanel', { name: 'Subagents' });
async function menu(page: Page, target: ReturnType<Page['locator']>, item: string) {
  await target.click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: item, exact: true }).click();
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('Working counts background projects and keeps the opened project outside its fold', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.removeItem('hollow.railSections.v1');
    localStorage.setItem('mockRunQueue', JSON.stringify([{ id: 900, source: 'chat', sourceId: 's-20260926-bbbb0000', label: 'Beta work', status: 'running', priority: 0, attempt: 1, coalescedCount: 0, requestedAt: '2026-11-16T12:00:00Z', cancelRequested: false }]));
  });
  await seed(page);
  const working = rail(page).getByRole('button', { name: 'Working projects (1)', exact: true });
  await expect(working).toHaveAttribute('aria-expanded', 'false');
  await expect(row(page, 'Beta')).toHaveCount(0);
  await working.focus();
  await page.keyboard.press('Enter');
  await expect(working).toHaveAttribute('aria-expanded', 'true');
  await expect(row(page, 'Beta')).toHaveCount(1);
  await expect(rail(page).getByRole('button', { name: 'Actions for project Beta', exact: true })).toBeVisible();
  await row(page, 'Beta').click();
  await expect(row(page, 'Beta')).toHaveCount(1);
  await expect(working).toHaveCount(0);
});

test('sidebar actions open by keyboard without switching conversations', async ({ page }) => {
  await seed(page);
  await row(page, 'Alpha').click();
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeFocused();
  const key = await openKey(page);
  const actions = rail(page).getByRole('button', { name: 'Actions for project Beta', exact: true });
  await actions.focus();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('menuitem', { name: 'Rename…', exact: true })).toBeVisible();
  await expect(page.getByRole('menuitem', { name: 'Rename…', exact: true })).toBeFocused();
  expect(await openKey(page)).toBe(key);
  const anchor = await actions.boundingBox();
  const menuBox = await page.locator('.ctx-menu').boundingBox();
  expect(menuBox!.x).toBeGreaterThanOrEqual(anchor!.x - 2);
  if (process.env.T3_CAPTURE_DIR) await page.screenshot({ path: `${process.env.T3_CAPTURE_DIR}/project-keyboard-menu.png` });
  await page.keyboard.press('Escape');
  await expect(actions).toBeFocused();
});

test('quick chat actions open by click without opening the chat', async ({ page }) => {
  await seed(page, [...DEFAULT_WORKSPACES, { ...workspace('quick-chats', 'Quick chats'), kind: 'quick' }],
    [session('s-20261116-0000aaaa', 'Quick example', 'quick-chats', 1)]);
  const actions = rail(page).getByRole('button', { name: 'Actions for chat Quick example', exact: true });
  const key = await openKey(page);
  await actions.click();
  await expect(page.getByRole('menuitem', { name: 'Move to project…', exact: true })).toBeVisible();
  expect(await openKey(page)).toBe(key);
  if (process.env.T3_CAPTURE_DIR) await page.screenshot({ path: `${process.env.T3_CAPTURE_DIR}/quick-chat-menu.png` });
});

test('a quick chat has one pinned home and an explicit snooze with Wake', async ({ page }) => {
  const key = 's-20261116-0000aabb';
  await seed(page, [...DEFAULT_WORKSPACES, { ...workspace('quick-chats', 'Quick chats'), kind: 'quick' }], [session(key, 'Organized chat', 'quick-chats', 1)]);
  const actions = () => rail(page).getByRole('button', { name: 'Actions for chat Organized chat', exact: true });
  await actions().click();
  await page.getByRole('menuitem', { name: 'Pin conversation', exact: true }).click();
  const pinned = rail(page).getByRole('group', { name: 'Pinned chats', exact: true });
  await expect(pinned.getByRole('button', { name: 'Organized chat', exact: true })).toBeVisible();
  await expect(rail(page).locator(`[data-session-key="${key}"]`)).toHaveCount(1);
  await actions().click();
  await page.getByRole('menuitem', { name: 'Unpin conversation', exact: true }).click();
  await expect(pinned).toHaveCount(0);
  await actions().click();
  await page.getByRole('menuitem', { name: 'Snooze until…', exact: true }).click();
  const future = await page.evaluate(() => {
    const d = new Date(Date.now() + 3_600_000), two = (n: number) => String(n).padStart(2, '0');
    return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}T${two(d.getHours())}:${two(d.getMinutes())}`;
  });
  await page.getByLabel('Or pick a date and time', { exact: true }).fill(future);
  await page.getByRole('dialog').getByRole('button', { name: 'Snooze', exact: true }).click();
  const snoozed = rail(page).getByRole('group', { name: 'Snoozed', exact: true });
  await expect(snoozed).toBeVisible();
  await snoozed.getByRole('button', { name: 'Snoozed (1)', exact: true }).click();
  await actions().click();
  await page.getByRole('menuitem', { name: 'Wake conversation', exact: true }).click();
  await expect(snoozed).toHaveCount(0);
  await expect.poll(() => openKey(page)).toBe(key);
});

test('a project row opens its lead, and its other chats are under Earlier chats', async ({ page }) => {
  await seed(page);
  // One row per project, and nothing under it: no conversation rows, no
  // Lead for row, no + and nothing to fold.
  await expect(rail(page).locator('.work-tree[data-project-id] .work-workspace-title')).toHaveText(['Alpha', 'Beta', 'Mirrored']);
  await expect(rail(page).locator('.work-tree-sessions, .work-session-row, .work-session-line, .work-project-main, .work-tree-add')).toHaveCount(0);
  await expect(rail(page).getByRole('button', { name: /^(Lead for |New conversation in |Show \d+ more$|Show older chats)/ })).toHaveCount(0);
  await expect(row(page, 'Alpha')).not.toHaveAttribute('aria-expanded');

  // A click opens the project's lead.
  await row(page, 'Alpha').click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(row(page, 'Alpha')).toHaveAttribute('aria-current', 'true');
  await expect.poll(() => openKey(page)).toBe(leadKey('alpha-1'));
  const composer = page.getByRole('textbox', { name: 'Chat message', exact: true });
  await expect(composer).toBeFocused();
  // Its draft stays with it across another project's lead.
  await composer.fill('Keep my project decision here');
  await row(page, 'Beta').click();
  await expect.poll(() => openKey(page)).toBe(leadKey('beta-2'));
  await expect(composer).toHaveValue('');
  await row(page, 'Alpha').click();
  await expect.poll(() => openKey(page)).toBe(leadKey('alpha-1'));
  await expect(composer).toHaveValue('Keep my project decision here');
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);

  // The project's other chats: the Subagents chip, then Earlier chats.
  await page.locator('.side-card-toggle').filter({ hasText: 'Subagents' }).click();
  await expect(chip(page)).toHaveAccessibleName('Subagents: 2 earlier chats');
  await chip(page).click();
  const tab = subagentsTab(page);
  await expect(tab.getByRole('heading', { name: 'Earlier chats · 2' })).toBeVisible();
  await expect(tab.getByRole('button', { name: /^Draft the launch memo/ })).toBeVisible();
  await expect(tab.getByRole('button', { name: /^Review pricing/ })).toBeVisible();
  await expect(tab.getByRole('button', { name: /^Beta thread/ })).toHaveCount(0);
  // A row opens that chat, with a box to message it.
  await tab.getByRole('button', { name: /^Review pricing/ }).click();
  const opened = page.getByRole('region', { name: 'Subagent Review pricing' });
  await expect(opened.getByRole('heading', { name: 'Review pricing' })).toBeVisible();
  await expect(opened).toContainText('Answer for Review pricing');
  await expect(opened.getByRole('textbox', { name: 'Message this subagent' })).toBeEnabled();
});

test('New chat opens the COO, and chats with no project are under its Earlier chats', async ({ page }) => {
  const at = (minutesAgo: number) => new Date(Date.now() - minutesAgo * 60_000).toISOString();
  const chat = (key: string, title: string, folder: string, minutesAgo: number) =>
    ({ key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(folder), createdAt: at(minutesAgo), lastActivityAt: at(minutesAgo), endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' });
  await seed(page, [
    ...DEFAULT_WORKSPACES,
    { ...workspace('main', 'COO'), kind: 'main' },
    { ...workspace('quick-chats', 'Chats'), kind: 'quick' },
  ], [
    chat('s-20261110-c00c0001', 'Morning check-in', 'main', 1),
    chat('s-20261110-qqqq0001', 'Cat names', 'quick-chats', 30),
    chat('s-20261110-qqqq0002', 'Garden plan', 'quick-chats', 90),
  ]);
  // Quick chats returned to the sidebar on 11-14; the COO route still works.
  await expect(rail(page).getByRole('group', { name: 'Chats', exact: true })).toHaveCount(0);
  await expect(rail(page).locator('.work-chats-head')).toHaveCount(0);
  await expect(rail(page).getByRole('group', { name: 'Quick chats', exact: true })).toContainText('Cat names');
  await expect(rail(page).locator('.work-tree[data-project-id] .work-workspace-title')).toHaveText(['Alpha', 'Beta', 'Mirrored']);

  // New chat is the COO's page (the orb since the sidebar's New chat went,
  // L1, 11-12), its box focused, with nothing sent.
  await expect(page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'New chat', exact: true })).toHaveCount(0);
  await page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^COO/ }).click();
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await expect.poll(() => openKey(page)).toBe('s-20261110-c00c0001');
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toBeFocused();
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);

  // The old chats with no project are the COO's Earlier chats.
  await page.locator('.side-card-toggle').filter({ hasText: 'Subagents' }).click();
  await expect(chip(page)).toHaveAccessibleName('Subagents: 2 earlier chats');
  await chip(page).click();
  const tab = subagentsTab(page);
  await expect(tab.getByRole('heading', { name: 'Earlier chats · 2' })).toBeVisible();
  await expect(tab.getByRole('button', { name: /^Cat names/ })).toBeVisible();
  await expect(tab.getByRole('button', { name: /^Garden plan/ })).toBeVisible();
  await tab.getByRole('button', { name: /^Garden plan/ }).click();
  await expect(page.getByRole('region', { name: 'Subagent Garden plan' })).toContainText('Answer for Garden plan');
});

test('workspace menu renames and hides from the rail; legacy Notion workspaces edit like any other', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Rename…');
  // The row turns into a field (3.6); Enter saves.
  const name = page.getByRole('textbox', { name: 'Rename Alpha', exact: true });
  await expect(name).toHaveValue('Alpha');
  await name.fill('Alpha launch');
  await page.keyboard.press('Enter');
  await expect(rail(page).getByRole('button', { name: 'Alpha launch', exact: true })).toBeVisible();

  await rail(page).getByRole('button', { name: 'Mirrored', exact: true }).click({ button: 'right' });
  const ctx = page.locator('.ctx-menu');
  await expect(ctx.getByRole('menuitem', { name: 'Rename…', exact: true })).toBeEnabled();
  await expect(ctx.getByRole('menuitem', { name: 'Canvas look…', exact: true })).toBeEnabled();
  await expect(ctx.getByRole('menuitem', { name: 'Delete…', exact: true })).toBeEnabled();
  await expect(ctx.getByRole('menuitem', { name: 'New conversation' })).toBeEnabled();
  await page.keyboard.press('Escape');

  // Hide from sidebar only takes it (and its conversations) out of the rail; its status is untouched.
  await menu(page, rail(page).getByRole('button', { name: 'Alpha launch', exact: true }), 'Hide from sidebar (keeps its status)');
  await expect(rail(page).getByRole('button', { name: 'Alpha launch', exact: true })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  const card = page.locator('.work-card').filter({ hasText: 'Alpha launch' });
  // Active is the default and shows no chip; hiding has not archived it.
  await expect(card.locator('.work-card-status')).toHaveCount(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]').find((w: {title: string}) => w.title === 'Alpha launch').status)).toBe('active');
  await card.click({ button: 'right' });
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Hide from sidebar (keeps its status)', exact: true })).toHaveCount(0);
  // Archive… is a separate item (plan completion L4, X14): off the active list.
  await expect(page.locator('.ctx-menu').getByRole('menuitem', { name: 'Archive… (off the active list)', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  // Starting a conversation there puts it back.
  await card.click();
  await page.getByLabel('Chat message').fill('Back to the launch');
  await page.getByRole('button', { name: 'Send', exact: true }).click();
  // The row's name gains the running turn's roll-up ("Alpha launch, 1 chat running").
  await expect(row(page, 'Alpha launch')).toBeVisible();
});

test('only workspaces with a conversation are in the rail; opening one moves it first', async ({ page }) => {
  await seed(page);
  await page.evaluate(() => {
    localStorage.removeItem('hollow.railWorkspaces');
    const all = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]');
    all.push({ id: 'gamma-3', title: 'Gamma', status: 'active', path: 'workspaces/gamma-3', cwd: '/Users/fixture/vault/workspaces/gamma-3', modified: '2026-09-28T12:00:00Z' });
    localStorage.setItem('mockWorkspaces', JSON.stringify(all));
  });
  await page.reload();
  // Gamma (newest, active) and Mirrored have no conversations: not listed.
  await expect(rail(page).locator('.work-tree .work-workspace-title')).toHaveText(['Alpha', 'Beta']);
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Beta' }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  // Opening Beta's newest conversation counts as using it here.
  await expect(rail(page).locator('.work-tree .work-workspace-title')).toHaveText(['Beta', 'Alpha']);
});

test('a long workspace list scrolls inside the rail; the bottom tool bar stays on screen', async ({ page }) => {
  // 400 high: the sidebar lists at most the newest 8 since L1 (11-12), so a
  // shorter window makes the list overflow.
  await page.setViewportSize({ width: 1280, height: 400 });
  await seed(page);
  await page.evaluate(() => {
    const all = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]');
    const sessions = JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]');
    const used: string[] = JSON.parse(localStorage.getItem('hollow.railWorkspaces') ?? '[]');
    for (let i = 0; i < 14; i++) {
      const id = `extra-${i}`;
      const cwd = `/Users/fixture/vault/workspaces/${id}`;
      all.push({ id, title: `Extra ${i}`, status: 'active', path: `workspaces/${id}`, cwd, modified: '2026-09-20T12:00:00Z' });
      const at = new Date().toISOString();
      sessions.push({ key: `s-20260926-eeee${String(i).padStart(4, '0')}`, title: `Extra thread ${i}`, agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' });
      used.push(id);
    }
    localStorage.setItem('mockWorkspaces', JSON.stringify(all));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(used));
  });
  await page.reload();
  // The newest 8 (and any pinned), no Show N more since L1 (11-12).
  await expect(rail(page).getByRole('button', { name: /^Show \d+ more projects?$/ })).toHaveCount(0);
  await expect.poll(() => rail(page).locator('.work-tree .work-workspace-row').count()).toBeGreaterThanOrEqual(8);
  const settings = page.getByRole('button', { name: 'Settings', exact: true });
  const box = await settings.boundingBox();
  expect(box && box.y + box.height).toBeLessThanOrEqual(400);
  // The list itself scrolls; the rail does not.
  const scroll = page.locator('.work-rail-scroll');
  expect(await scroll.evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
  expect(await page.locator('.work-rail').evaluate(el => el.scrollHeight <= el.clientHeight + 1)).toBe(true);
  await scroll.evaluate(el => { el.scrollTop = el.scrollHeight; });
  const after = await settings.boundingBox();
  expect(after?.y).toBe(box?.y);
});

test('delete asks first, then removes the workspace', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await expect(page.getByRole('dialog')).toContainText('Delete “Beta”?');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toBeVisible();
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await page.getByRole('button', { name: 'Move to trash', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toHaveCount(0);
});

test('Undo after delete puts the workspace back with its conversations (P1-41)', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await page.getByRole('button', { name: 'Move to trash', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toHaveCount(0);
  const bar = page.locator('.snackbar').filter({ hasText: 'Moved Beta to trash' });
  await bar.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toBeVisible();
  // Its conversations came back too: its lead lists them under Earlier chats.
  await rail(page).getByRole('button', { name: 'Beta', exact: true }).click();
  await page.locator('.side-card-toggle').filter({ hasText: 'Subagents' }).click();
  await expect(chip(page)).toHaveAccessibleName('Subagents: 6 earlier chats');
  await expect(page.locator('.snackbar').filter({ hasText: 'Restored Beta' }).first()).toBeVisible();
});

test('Recently deleted under All work survives a reload and restores (P1-41)', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await page.getByRole('button', { name: 'Move to trash', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toHaveCount(0);
  await page.reload();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  // The expanded project list keeps deleted projects reachable.
  await page.getByRole('checkbox', { name: 'Include archived' }).check();
  const deleted = page.getByRole('region', { name: 'Recently deleted' });
  await expect(deleted.getByRole('listitem')).toHaveCount(1);
  await expect(deleted.getByRole('listitem')).toContainText('Beta');
  await deleted.getByRole('button', { name: 'Restore Beta', exact: true }).click();
  await expect(page.locator('.work-card').filter({ hasText: 'Beta' })).toBeVisible();
  await expect(deleted).toHaveCount(0);
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toBeVisible();
});

test('with no workspace left, Home still shows Recently deleted (P1-41)', async ({ page }) => {
  await seed(page);
  await page.evaluate(() => localStorage.setItem('mockWorkspaces', JSON.stringify(JSON.parse(localStorage.getItem('mockWorkspaces')!).filter((w: { id: string }) => w.id === 'beta-2'))));
  await page.reload();
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await page.getByRole('button', { name: 'Move to trash', exact: true }).click();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  const deleted = page.getByRole('region', { name: 'Recently deleted' });
  await expect(deleted.getByRole('heading', { level: 2, name: 'Recently deleted' })).toBeVisible();
  await deleted.getByRole('button', { name: 'Restore Beta', exact: true }).click();
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toBeVisible();
});

test('a failed rename from the rail shows a toast on any page (P1-41)', async ({ page }) => {
  await seed(page);
  await openTasks(page);
  await page.evaluate(() => { window.__mockFailCommandOnce = 'update_workspace'; });
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Rename…');
  await page.getByRole('textbox', { name: 'Rename Alpha', exact: true }).fill('Alpha two');
  await page.keyboard.press('Enter');
  await expect(page.locator('.snackbar').filter({ hasText: 'mock update_workspace failure' }).first()).toBeVisible();
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toBeVisible();
});

test('a refused delete keeps you in the workspace (P0-13)', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Beta thread 0');
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await page.evaluate(() => { window.__mockFailCommandOnce = 'trash_workspace'; });
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Delete…');
  await page.getByRole('button', { name: 'Move to trash', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockFailCommandOnce ?? null)).toBeNull();
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await expect(page.locator('.chat-main')).toContainText('Answer for Beta thread 0');
  await expect(rail(page).getByRole('button', { name: 'Beta', exact: true })).toBeVisible();
});

// 2026-09-29 (user report): from Settings, clicking a conversation in the
// rail seemed to do nothing, and a workspace in Work wouldn't open. Since L8
// (11-10) the rail's rows are projects, each opening its lead.
test('the rail opens a project’s lead from any page', async ({ page }) => {
  await seed(page);
  const settings = () => page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings' }).click();
  await settings();
  await expect(page.locator('.settings-page')).toBeVisible();
  await row(page, 'Alpha').click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect.poll(() => openKey(page)).toBe(leadKey('alpha-1'));
  // The project already open, then another one.
  await settings();
  await row(page, 'Alpha').click();
  await expect(page.locator('.settings-page')).toHaveCount(0);
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await settings();
  await row(page, 'Beta').click();
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await expect.poll(() => openKey(page)).toBe(leadKey('beta-2'));
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library' }).click();
  await row(page, 'Alpha').click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
});

test('a workspace that fails to open says so on screen, from any page', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkspaceReadError', 'beta-2'));
  await seed(page);
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings' }).click();
  await row(page, 'Beta').click();
  // Lands on Work with the reason, and a toast in case the banner is scrolled away.
  await expect(page.locator('.settings-page')).toHaveCount(0);
  await expect(page.locator('.work-error')).toContainText("Couldn't open Beta: Workspace has over 100 source files");
  await expect(page.locator('.snackbar').filter({ hasText: "Couldn't open Beta" })).toBeVisible();

  // The Work card: same message, and nothing else breaks.
  await page.locator('.work-error').getByRole('button', { name: 'Dismiss' }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Beta' }).click();
  await expect(page.locator('.work-error')).toContainText("Couldn't open Beta");
  // Failed navigation remounts Home, so its full list starts folded again.
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Alpha' }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
});

test('launch reopens the last conversation; a gone one opens its workspace, a gone workspace Home (frontier F4)', async ({ page }) => {
  await seed(page);
  // Not the newest one: launch must reopen exactly what was on screen.
  await openChat(page, 'Review pricing');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
  await openTasks(page);
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
  await expect(row(page, 'Alpha')).toHaveAttribute('aria-current', 'true');

  await page.evaluate(() => localStorage.setItem('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha-1', key: 's-20260926-gone0001' })));
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(page.locator('.chat-main')).toContainText('Answer for Draft the launch memo');

  await page.evaluate(() => localStorage.setItem('hollow.lastConversation', JSON.stringify({ workspaceId: 'deleted-9', key: 's-20260926-aaaa0001' })));
  await page.reload();
  await expect(page.locator('.work-header h1')).toHaveText('Projects');
  await expect(page.locator('.work-error')).toHaveCount(0);
  // The gone record is dropped once the launch restore settles, which can
  // land after Home draws under a loaded machine: poll, don't read once.
  await expect.poll(() => page.evaluate(() => localStorage.getItem('hollow.lastConversation'))).toBeNull();
});

test('⌘N starts a conversation in the open workspace; the palette asks which workspace (frontier F4)', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  await expect(page.locator('.chat-main')).toContainText('Answer for Draft the launch memo');
  await page.locator('body').click({ position: { x: 700, y: 20 } });
  await page.keyboard.press('Meta+n');
  // A new, unsent conversation in Alpha (not its lead).
  await expect(page.locator('.chat-main')).not.toContainText('Answer for Draft the launch memo');
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(page.getByLabel('Chat message')).toBeFocused();
  expect(await openKey(page)).toBeNull();

  // From Home the palette's New conversation asks where (⌘N itself opens
  // the COO since L8, 11-10).
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('New conversation');
  await page.keyboard.press('Enter');
  const input = page.getByRole('combobox', { name: 'Choose a workspace for the new conversation' });
  await expect(input).toBeFocused();
  await input.fill('Beta');
  await page.keyboard.press('Enter');
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await expect(page.getByLabel('Chat message')).toBeFocused();
  expect(await openKey(page)).toBeNull();
});

// L8 (11-10): ⌘⇧] / ⌘⇧[ step between project leads in the sidebar's order.
test('⌘⇧] / ⌘⇧[ step through project leads, ⌘\\ toggles the side panel, ⌘K finds conversations (frontier F6)', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Review pricing');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
  await page.keyboard.press('Meta+Shift+BracketRight');
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await expect.poll(() => openKey(page)).toBe(leadKey('beta-2'));
  await expect(row(page, 'Beta')).toHaveAttribute('aria-current', 'true');
  await page.keyboard.press('Meta+Shift+BracketLeft');
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect.poll(() => openKey(page)).toBe(leadKey('alpha-1'));
  // Nothing before the first project.
  await page.keyboard.press('Meta+Shift+BracketLeft');
  await page.waitForTimeout(300);
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  expect(await openKey(page)).toBe(leadKey('alpha-1'));

  // The right pane is the Preview pane (run 15 P1): it toggles once something was opened.
  await page.evaluate(async () => {
    const store = '/src/store/preview.ts';
    (await import(/* @vite-ignore */ store)).openPreview({ kind: 'url', url: '' });
  });
  const pane = page.getByRole('complementary', { name: 'Preview' });
  await expect(pane).toBeVisible();
  await page.keyboard.press('Meta+Backslash');
  await expect(pane).toHaveCount(0);
  await page.keyboard.press('Meta+Backslash');
  await expect(pane).toBeVisible();

  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Beta thread 3');
  await expect(page.locator('.palette-row.selected')).toContainText('Beta');
  await page.keyboard.press('Enter');
  await expect(page.locator('.work-header h1')).toHaveText('Beta');
  await expect(page.locator('.chat-main')).toContainText('Answer for Beta thread 3');
});

test('Esc stops a running turn, but never while a menu owns Escape (frontier F6)', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  const composer = page.getByLabel('Chat message');
  await composer.fill('Keep going');
  await composer.press('Enter');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  const cancels = () => page.evaluate(() => (window as any).__mockApiCalls?.chat_cancel ?? 0);
  // An open menu takes the first Escape.
  await page.getByRole('button', { name: /^Model:/ }).click();
  await expect(page.locator('.model-menu [role="listbox"], .model-menu-pop').first()).toBeVisible();
  await page.keyboard.press('Escape');
  expect(await cancels()).toBe(0);
  // A typed message keeps the turn: Esc stops only with an empty box (FIX-ESC).
  await composer.fill('Wait');
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  expect(await cancels()).toBe(0);
  await composer.fill('');
  await page.keyboard.press('Escape');
  await expect.poll(cancels).toBe(1);
  // The palette lists Stop while the turn runs.
  await page.keyboard.press('Meta+k');
  await expect(page.locator('.palette-row').filter({ hasText: 'Stop the running turn' })).toBeVisible();
});

test('⌘K then ↵ during a running turn leaves the turn running (P0-02)', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  const composer = page.getByLabel('Chat message');
  await composer.fill('Keep going');
  await composer.press('Enter');
  await expect(page.getByRole('button', { name: 'Stop', exact: true })).toBeVisible();
  const cancels = () => page.evaluate(() => (window as any).__mockApiCalls?.chat_cancel ?? 0);
  await page.keyboard.press('Meta+k');
  const stopRow = page.locator('.palette-row').filter({ hasText: 'Stop the running turn' });
  // Stop is still offered, says what it does, and isn't the row Enter runs.
  await expect(stopRow).toBeVisible();
  await expect(stopRow).toContainText('Starts or stops agent work, which may cost money');
  await expect(page.locator('.palette-row.selected')).not.toContainText('Stop the running turn');
  const first = (await page.locator('.palette-row.selected .palette-row-label').textContent())?.trim();
  expect(first).toBeTruthy();
  await page.keyboard.press('Enter');
  await expect(page.locator('.palette-dialog')).toHaveCount(0);
  // The mock's chat_cancel ends nothing on its own, so the call count is the
  // proof: give a late cancel time to land before saying none came.
  await page.waitForTimeout(500);
  expect(await cancels()).toBe(0);
});

test('with the overlay title bar, the collapsed rail stays narrow and its top joins the title strip', async ({ page }) => {
  await page.goto('/?mock=1&chrome=overlay');
  await expect(page.locator('html')).toHaveAttribute('data-chrome', 'overlay');
  const railEl = page.locator('.work-rail');
  await expect(railEl).toBeVisible();
  await page.keyboard.press('Meta+b');
  await expect(railEl).toHaveClass(/collapsed/);
  const box = await railEl.boundingBox();
  expect(box?.width).toBeLessThanOrEqual(64);
  // No edge line through the lights: the rail's border is transparent and its
  // drawn edge starts below the 44px strip, whose background matches the top bar.
  const look = await railEl.evaluate(el => {
    const cs = getComputedStyle(el); const after = getComputedStyle(el, '::after');
    const bar = getComputedStyle(document.querySelector('.top-bar')!);
    return { border: cs.borderRightColor, afterTop: after.top, bg: cs.backgroundImage, barBg: bar.backgroundColor };
  });
  expect(look.border).toBe('rgba(0, 0, 0, 0)');
  expect(look.afterTop).toBe('44px');
  expect(look.bg).toContain('44px');
  // The top bar clears the lights' 8px overhang without a wide gap.
  const pad = await page.locator('.top-bar').evaluate(el => parseFloat(getComputedStyle(el).paddingLeft));
  expect(pad).toBeGreaterThanOrEqual(16);
  expect(pad).toBeLessThan(40);
});

test('Hide from sidebar and Archive… are different: one hides the row, the other takes it off the active list and asks first (L4, X14)', async ({ page }) => {
  await seed(page);
  const statusOf = (title: string) => page.evaluate(t => (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { title: string; status: string }[]).find(w => w.title === t)?.status, title);
  // Hide: off the sidebar, status untouched, and it says so.
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Hide from sidebar (keeps its status)');
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toHaveCount(0);
  await expect(page.getByText(/Hid Alpha from the sidebar\. Its status is unchanged; the Projects page lists it\./).first()).toBeVisible();
  expect(await statusOf('Alpha')).toBe('active');
  // Archive…: asks first, then the status changes; Cancel changes nothing.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  const card = page.locator('.work-card').filter({ hasText: 'Alpha' }).first();
  await card.click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Archive… (off the active list)', exact: true }).click();
  const dialog = page.locator('.confirm-backdrop');
  await expect(dialog).toContainText('Archive “Alpha”?');
  await expect(dialog).toContainText('nothing moves or is deleted');
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(await statusOf('Alpha')).toBe('active');
  await card.click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Archive… (off the active list)', exact: true }).click();
  await dialog.getByRole('button', { name: 'Archive', exact: true }).click();
  await expect.poll(() => statusOf('Alpha')).toBe('archived');
});

// P1-38: Archive, Hide, status and Merge each mean one thing.
const statusOf = (page: Page, title: string) => page.evaluate(t => (JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { title: string; status: string }[]).find(w => w.title === t)?.status, title);
async function allWork(page: Page) {
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.getByRole('checkbox', { name: 'Include archived' }).check();
}

test('Archive takes a workspace out of the sidebar; Unarchive brings it back with its old status (P1-38)', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Mark as planning');
  await expect.poll(() => statusOf(page, 'Alpha')).toBe('planning');
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Archive… (off the active list)');
  await page.locator('.confirm-backdrop').getByRole('button', { name: 'Archive', exact: true }).click();
  await expect.poll(() => statusOf(page, 'Alpha')).toBe('archived');
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toHaveCount(0);
  await allWork(page);
  await page.locator('.work-card').filter({ hasText: 'Alpha' }).first().click({ button: 'right' });
  const ctx = page.locator('.ctx-menu');
  await expect(ctx.getByRole('menuitem', { name: /^Mark as/ })).toHaveCount(0);
  await expect(ctx.getByRole('menuitem', { name: /^Archive…/ })).toHaveCount(0);
  await ctx.getByRole('menuitem', { name: 'Unarchive (back to planning)', exact: true }).click();
  await expect.poll(() => statusOf(page, 'Alpha')).toBe('planning');
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toBeVisible();
});

test('Hide from sidebar holds when the workspace is opened from Work (P1-38)', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Alpha', exact: true }), 'Hide from sidebar (keeps its status)');
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Alpha' }).first().click();
  await expect(page.locator('.work-header h1')).toHaveText('Alpha');
  await expect(rail(page).getByRole('button', { name: 'Alpha', exact: true })).toHaveCount(0);
});

test('Mark as done offers Undo, which puts the old status back (P1-38)', async ({ page }) => {
  await seed(page);
  await menu(page, rail(page).getByRole('button', { name: 'Beta', exact: true }), 'Mark as done');
  await expect.poll(() => statusOf(page, 'Beta')).toBe('done');
  const bar = page.locator('.snackbar').filter({ hasText: 'Beta: done' });
  await bar.getByRole('button', { name: 'Undo', exact: true }).click();
  await expect.poll(() => statusOf(page, 'Beta')).toBe('active');
});

test('the merge confirm starts on Cancel, sums up in one line, and a running conversation turns Merge off (P1-38)', async ({ page }) => {
  await seed(page);
  const merge = (blocked: string | null) => page.evaluate(async (why) => {
    const { api } = await import(/* @vite-ignore */ '/src/api.ts');
    api.previewWorkspaceMerge = async () => ({
      from: 'alpha-1', fromTitle: 'Alpha', into: 'beta-2', intoTitle: 'Beta', files: [], left: [], keepsWorktree: false, hash: 'h', blocked: why,
      sessions: [{ key: 's-20260926-aaaa0001', title: 'Draft the launch memo', archived: false }],
    });
    const { mergeWorkspaceInto } = await import(/* @vite-ignore */ '/src/store/workspaces.ts');
    void mergeWorkspaceInto('alpha-1', 'beta-2');
  }, blocked);
  await merge(null);
  const dialog = page.getByRole('dialog', { name: 'Merge “Alpha” into “Beta”?' });
  await expect(dialog.getByRole('button', { name: 'Cancel', exact: true })).toBeFocused();
  await expect(dialog).toHaveAccessibleDescription('Moves no files and 1 conversation into “Beta”, then archives “Alpha”.');
  await expect(dialog).toContainText('next turns run in the folder of “Beta”');
  await expect(dialog.getByRole('button', { name: 'Merge', exact: true })).toBeEnabled();
  await page.keyboard.press('Enter');
  await expect(dialog).toHaveCount(0);
  await merge('The conversation “Draft the launch memo” has a turn running or queued. Try again when it finishes.');
  await expect(dialog.getByRole('button', { name: 'Merge', exact: true })).toBeDisabled();
  await expect(dialog).toHaveAccessibleDescription(/then archives “Alpha”\. The conversation “Draft the launch memo” has a turn running or queued/);
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
});

test('a merged workspace says where it went, with Undo merge… on its row (P1-38)', async ({ page }) => {
  await seed(page, [{ ...workspace('alpha-1', 'Alpha', 'archived'), mergedInto: 'beta-2' }, workspace('beta-2', 'Beta')]);
  await allWork(page);
  const row = page.locator('.home-project-row').filter({ hasText: 'Alpha' });
  await expect(row).toContainText('Merged into Beta');
  await row.getByRole('button', { name: 'Undo merge… (Alpha)', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'Undo the merge of “Alpha”?' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  // (The target's Overview also offered it until the COO rebuild L4 removed
  // the workspace panel.)
});


test('the Projects heading plus opens creation and Cancel returns focus', async ({ page }) => {
  await seed(page);
  const plus = rail(page).locator('.work-rail-project-heading').getByRole('button', { name: 'New project', exact: true });
  await plus.click();
  const dialog = page.getByRole('dialog', { name: 'New project', exact: true });
  await expect(dialog.getByRole('textbox', { name: 'Project name' })).toBeFocused();
  await dialog.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(plus).toBeFocused();
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(0);
});
