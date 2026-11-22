import { openChat } from './choose';
import { test, expect, type Page } from './fixtures/test';

// The sidebar like ChatGPT's and Claude's (overnight 11-04, lane S): Tasks
// among the pages, projects that fold, Search chats in place with Search
// everything, and a remembered width. Since L8 (11-10) it lists no
// conversations: a project's row opens its lead, and the Chats section went
// (chats with no project are under the COO's Earlier chats). Since L1
// (11-12) Work is Projects, the sidebar has no New chat, no Show N more and
// no All projects row (the Projects page lists them all), its project list
// is headed Recent, and Search chats is the top bar's icon (⌘⇧K).

async function seed(page: Page, projects = 2) {
  await page.addInitScript((count: number) => {
    if (sessionStorage.getItem('sidebarSeeded')) return;
    sessionStorage.setItem('sidebarSeeded', '1');
    for (const key of ['hollow.workspace', 'hollow.railCollapsed', 'hollow.railExpanded.v1', 'hollow.railSections.v1', 'hollow.railWidth', 'hollow.chatPins.v1']) localStorage.removeItem(key);
    const now = Date.now(), DAY = 86_400_000;
    const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
    const ws = (id: string, title: string, extra: Record<string, unknown> = {}) =>
      ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: new Date(now).toISOString(), ...extra });
    const session = (key: string, title: string, id: string, ago: number) => {
      const at = new Date(now - ago).toISOString();
      return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(id), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
    };
    const ids = Array.from({ length: count }, (_, i) => `proj-${i}`);
    const workspaces = [ws('quick-chats', 'Quick chats', { kind: 'quick' }), ...ids.map((id, i) => ws(id, `Project ${i}`))];
    const sessions = [
      session('s-20261104-quick001', 'Cat names', 'quick-chats', 60_000),
      session('s-20261104-quick002', 'Garden plan', 'quick-chats', 3 * DAY),
      session('s-20261104-quick003', 'Old recipe', 'quick-chats', 40 * DAY),
      ...ids.map((id, i) => session(`s-20261104-proj${String(i).padStart(3, '0')}`, `Thread ${i}`, id, (i + 1) * 3_600_000)),
    ];
    const conversations: Record<string, unknown> = {};
    for (const s of sessions) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked about ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(ids));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, projects);
  await page.goto('/?mock=1');
}

const aside = (page: Page) => page.getByRole('complementary', { name: 'Main navigation' });
/** Search chats: the top bar's icon by the sidebar's toggle (L1, 11-12). */
const searchIcon = (page: Page) => page.locator('.top-bar').getByRole('button', { name: 'Search chats', exact: true });
const pages = (page: Page) => page.getByRole('navigation', { name: 'Main', exact: true });
const lists = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
const menu = (page: Page) => page.locator('.ctx-menu');
/** A project's one row (L8, 11-10): a click opens its lead. */
const project = (page: Page, title: string) => lists(page).getByRole('button', { name: new RegExp(`^${title}(,|$)`) });
const projectRows = (page: Page) => lists(page).locator('.work-tree[data-project-id]');

// Desktop checks run once, at the shown sidebar's width; the narrow one runs
// in chromium-narrow, where the sidebar starts hidden.
const desktopOnly = (info: { project: { name: string } }) => test.skip(info.project.name !== 'chromium-desktop', 'the shown sidebar at desktop width');

test('the sidebar reads the pages with Projects and Tasks, then Recent and no Chats, New chat or Search row', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page);
  await expect(pages(page).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await expect(aside(page).getByRole('button', { name: 'New chat', exact: true })).toHaveCount(0);
  await expect(aside(page).getByRole('button', { name: 'Search chats', exact: true })).toHaveCount(0);
  await expect(searchIcon(page)).toBeVisible();
  await expect(lists(page).locator('.work-rail-section-toggle .work-rail-label')).toHaveText(['Recent', 'Quick chats']);
  await expect(projectRows(page).locator('.work-workspace-title')).toHaveText(['Project 0', 'Project 1']);
  // Chats with no project list under Quick chats (back 2026-11-14), not under a project.
  await expect(lists(page).getByRole('group', { name: 'Chats', exact: true })).toHaveCount(0);
  await expect(lists(page)).toContainText('Cat names');
  await pages(page).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(pages(page).getByRole('button', { name: 'Tasks', exact: true })).toHaveAttribute('aria-current', 'page');
});

test('Recent folds from its heading and stays folded after a reload', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page);
  await expect(projectRows(page)).toHaveCount(2);
  await lists(page).getByRole('button', { name: 'Recent', exact: true }).click();
  await expect(projectRows(page)).toHaveCount(0);
  await page.reload();
  await expect(lists(page).getByRole('button', { name: 'Recent', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await expect(projectRows(page)).toHaveCount(0);
});

test('a folded Recent heading keeps its chats’ live state, as the rows do unfolded', async ({ page }, info) => {
  desktopOnly(info);
  await page.addInitScript(() => {
    const at = new Date().toISOString();
    localStorage.setItem('mockRunQueue', JSON.stringify([{ id: 9150, source: 'chat', sourceId: 's-20261104-proj000', label: 'Chat', detail: 'Thread 0', reason: '', status: 'failed', priority: 0, position: null, attempt: 1, coalescedCount: 0, requestedAt: at, claimedAt: at, finishedAt: at, historyRunId: 'sidebar-failed', error: 'Fixture failure', waitingSince: null, waitingHolder: null, heldReason: null, cancelRequested: false }]));
  });
  await seed(page);
  const heading = lists(page).locator('.work-rail-project-heading');
  const badge = heading.getByRole('img', { name: '1 failed: Thread 0', exact: true });
  // Unfolded, the project's own row says it; the heading adds nothing.
  await expect(project(page, 'Project 0')).toHaveAccessibleName(/^Project 0, .*failed/);
  await expect(badge).toHaveCount(0);
  await heading.getByRole('button', { name: 'Recent', exact: true }).click();
  await expect(projectRows(page)).toHaveCount(0);
  await expect(badge).toBeVisible();
  await heading.getByRole('button', { name: 'Recent', exact: true }).click();
  await expect(badge).toHaveCount(0);
  await expect(project(page, 'Project 0')).toHaveAccessibleName(/^Project 0, .*failed/);
});

test('Search chats finds and opens a chat in place; Search everything carries the words to the palette', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page);
  await searchIcon(page).click();
  const field = page.getByRole('combobox', { name: 'Search chats' });
  await expect(field).toBeFocused();
  await expect(pages(page)).toBeHidden();
  await field.fill('garden');
  await expect(page.getByRole('listbox', { name: 'Search results' }).getByRole('option', { name: /Garden plan/ })).toBeVisible();
  await field.press('Enter');
  await expect(page.locator('.chat-main')).toContainText('Answer for Garden plan');
  await expect(pages(page)).toBeVisible();

  await searchIcon(page).click();
  await page.getByRole('combobox', { name: 'Search chats' }).fill('recipe');
  await page.getByRole('option', { name: /^Search everything for/ }).click();
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeVisible();
  await expect(page.locator('.palette-input')).toHaveValue('recipe');
  await page.keyboard.press('Escape');
  await expect(searchIcon(page)).toBeFocused();
});

test('⌘⇧K opens Search chats from a hidden sidebar; Escape hides it again and focus is on the icon', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page);
  const rail = page.locator('.work-rail');
  await page.getByRole('button', { name: 'Hide sidebar', exact: true }).click();
  await expect(rail).toHaveClass(/collapsed/);
  await searchIcon(page).focus();
  await page.keyboard.press('ControlOrMeta+Shift+K');
  const field = page.getByRole('combobox', { name: 'Search chats' });
  await expect(field).toBeFocused();
  await expect(rail).not.toHaveClass(/collapsed/);
  await field.press('Escape');
  await expect(rail).toHaveClass(/collapsed/);
  await expect(searchIcon(page)).toBeFocused();
});

test('a long project list shows the newest 8 and no Show more or All projects; the open project stays listed', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page, 10);
  await expect(projectRows(page)).toHaveCount(8);
  await expect(project(page, 'Project 9')).toHaveCount(0);
  // No project row folds or unfolds any more (L8, 11-10).
  await expect(project(page, 'Project 0')).not.toHaveAttribute('aria-expanded');
  // The rest are on the Projects page (L1, 11-12).
  await expect(lists(page).getByRole('button', { name: /^Show \d+ more projects?$/ })).toHaveCount(0);
  await expect(lists(page).getByRole('button', { name: /^All projects/ })).toHaveCount(0);
  // A chat opened in a project past the fold lists that project.
  await openChat(page, 'Thread 9');
  await expect(page.locator('.chat-main')).toContainText('Answer for Thread 9');
  await expect(project(page, 'Project 9')).toHaveAttribute('aria-current', 'true');
});

test('the sidebar’s edge resizes it, the width survives a reload, and a double-click resets it', async ({ page }, info) => {
  desktopOnly(info);
  await seed(page);
  const width = async () => Math.round((await aside(page).boundingBox())!.width);
  const before = await width();
  await aside(page).getByRole('separator', { name: 'Resize sidebar' }).focus();
  await page.keyboard.press('ArrowRight');
  await page.keyboard.press('ArrowRight');
  await expect.poll(width).toBe(before + 32);
  await page.reload();
  await expect.poll(width).toBe(before + 32);
  await aside(page).getByRole('separator', { name: 'Resize sidebar' }).dblclick();
  await expect.poll(width).toBe(before);
});

test('in a narrow window, Search chats shows the hidden sidebar for the search and hides it again', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-narrow', 'the narrow window starts with the sidebar hidden');
  await seed(page);
  const rail = page.locator('.work-rail');
  const search = searchIcon(page);
  await expect(rail).toHaveClass(/collapsed/);
  await search.click();
  const field = page.getByRole('combobox', { name: 'Search chats' });
  await expect(field).toBeFocused();
  await expect(rail).not.toHaveClass(/collapsed/);
  // Escape on an empty field closes the search and hides the sidebar again.
  await field.press('Escape');
  await expect(rail).toHaveClass(/collapsed/);
  await expect(search).toBeFocused();
  // Opening a result leaves the sidebar hidden, with the chat on screen.
  await search.click();
  await page.getByRole('combobox', { name: 'Search chats' }).fill('garden');
  await page.getByRole('option', { name: /Garden plan/ }).click();
  await expect(page.locator('.chat-main')).toContainText('Answer for Garden plan');
  await expect(rail).toHaveClass(/collapsed/);
});

test('in a narrow window, ⌘⇧K from the page opens the search with the field focused; typing finds a chat; Escape returns focus', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-narrow', 'the narrow window starts with the sidebar hidden');
  await seed(page);
  const rail = page.locator('.work-rail');
  const toggle = page.locator('.top-bar').getByRole('button', { name: 'Show sidebar', exact: true });
  await expect(rail).toHaveClass(/collapsed/);
  await toggle.focus();
  await page.keyboard.press('ControlOrMeta+Shift+K');
  const field = page.getByRole('combobox', { name: 'Search chats' });
  await expect(field).toBeFocused();
  await expect(rail).not.toHaveClass(/collapsed/);
  await field.pressSequentially('garden');
  await expect(page.getByRole('listbox', { name: 'Search results' }).getByRole('option', { name: /Garden plan/ })).toBeVisible();
  await field.fill('');
  await field.press('Escape');
  await expect(rail).toHaveClass(/collapsed/);
  // Focus goes back to where the shortcut was pressed (the toggle's label follows the sidebar).
  await expect(page.locator('.top-bar').getByRole('button', { name: 'Show sidebar', exact: true })).toBeFocused();
});

test('in a narrow window, Escape on a project row’s menu closes the menu and leaves the sidebar shown', async ({ page }, info) => {
  test.skip(info.project.name !== 'chromium-narrow', 'the narrow window shows the sidebar over the page');
  await seed(page);
  const rail = page.locator('.work-rail');
  await expect(rail).toHaveClass(/collapsed/);
  await page.getByRole('button', { name: 'Show sidebar', exact: true }).click();
  await expect(rail).not.toHaveClass(/collapsed/);
  await project(page, 'Project 0').click({ button: 'right' });
  await expect(menu(page)).toBeVisible();
  // The menu's Escape closes the menu only (it used to hide the sidebar too).
  await page.keyboard.press('Escape');
  await expect(menu(page)).toHaveCount(0);
  await expect(rail).not.toHaveClass(/collapsed/);
  // The next Escape hides the sidebar, as before.
  await page.keyboard.press('Escape');
  await expect(rail).toHaveClass(/collapsed/);
});
