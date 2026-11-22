import { expect, type Locator, type Page } from '@playwright/test';

/** Settings section `label` from its index, or from the picker a narrow
 *  window shows instead (860 px and under, as the other specs). */
export async function openSettingsSection(page: Page, label: string) {
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), { label });
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: label, exact: true }).click();
}

/** Open one of Advanced's closed groups (Settings PR 3): 'this-mac',
 *  'model-calls', 'settings-file', 'diagnostics', or the 'indexes' part. */
export async function openAdvancedGroup(page: Page, id: string) {
  const group = page.locator(id === 'indexes' ? '#settings-indexes' : `#settings-group-${id}`);
  await expect(group).toBeAttached();
  if (await group.getAttribute('open') === null) await group.locator(':scope > summary').click();
  await expect(group).toHaveAttribute('open', '');
}

/** The icon rows stay mounted inside Interface's native disclosure. */
export async function openSettingsIcons(page: Page) {
  const fold = page.locator('.settings-icons');
  if (await fold.getAttribute('open') === null) await fold.locator('summary').click();
}

/** Tasks' visible route after Sam's sidebar port. */
export async function openTasks(page: Page) {
  // Tasks is back in the sidebar (overnight 11-04 S1), as before Sam's port.
  await page.getByRole('navigation', {name:'Main', exact:true}).getByRole('button', {name:'Tasks', exact:true}).click();
}

/** Tasks' list, shown. Below the 1100 px window breakpoint an open pane
 *  takes the list's place, and its Close is the way back (2026-11-12). */
export async function showTaskList(page: Page) {
  const list = page.locator('.task-collection');
  if (await list.isHidden()) await page.getByRole('button', { name: 'Close issue details' }).click();
  await expect(list).toBeVisible();
}

/** Open Home's complete list through its visible disclosure (P1-52). */
export async function showWorkspaceList(page: Page) {
  const list = page.getByRole('region', { name: 'Projects', exact: true });
  const toggle = list.getByRole('button', { name: /^(All projects|Show recent projects)$/ });
  await expect(toggle).toBeVisible();
  if (await toggle.getAttribute('aria-expanded') === 'false') await toggle.click();
}

/**
 * Choose an option in Hollow's themed Select (src/components/Select.tsx) —
 * the replacement for native <select> (audit R01). `value` matches an
 * option's data-value; `{ label }` matches its visible label. A MultiSelect
 * stays open while choosing, so it is closed with Escape afterwards.
 */
export async function choose(target: Locator, value: string | { label: string }) {
  await target.click();
  const pop = target.page().locator('.hs-pop');
  await expect(pop).toBeVisible();
  const option = typeof value === 'string'
    ? pop.locator(`[role=option][data-value="${value.replace(/["\\]/g, '\\$&')}"]`)
    : pop.getByRole('option', { name: new RegExp(`^${value.label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}`) });
  await option.first().click();
  if (await pop.count()) await target.page().keyboard.press('Escape');
}

/** The option labels a themed Select offers (opens and closes it). */
export async function optionLabels(target: Locator): Promise<string[]> {
  await target.click();
  const pop = target.page().locator('.hs-pop');
  await expect(pop).toBeVisible();
  const labels = await pop.locator('[role=option] .hs-option-label').allTextContents();
  await target.page().keyboard.press('Escape');
  return labels;
}

/** The option values a themed Select offers (opens and closes it). */
export async function optionValues(target: Locator): Promise<string[]> {
  await target.click();
  const pop = target.page().locator('.hs-pop');
  await expect(pop).toBeVisible();
  const values = await pop.locator('[role=option]').evaluateAll(els => els.map(e => e.getAttribute('data-value') ?? ''));
  await target.page().keyboard.press('Escape');
  return values;
}

/** The Preview pane (PreviewPane.tsx's `<aside>`). */
export function previewPane(page: Page): Locator {
  return page.getByRole('complementary', { name: 'Preview', exact: true });
}

/** The top bar's Project files (COO rebuild L4, 2026-11-10; it replaced the
 *  workspace panel button): pressed while the Preview pane shows a file or
 *  Changes. */
export function projectFilesButton(page: Page): Locator {
  return page.locator('.top-bar').getByRole('button', { name: 'Project files', exact: true });
}

/** The Preview pane header's Files picker (ChatPreview.tsx). */
export function filesPicker(page: Page): Locator {
  return previewPane(page).getByRole('combobox', { name: 'Files', exact: true });
}

/** Whether the open Preview pane covers the page (a narrow window, 1100 px
 *  and under: preview-pane.css hides the page beside it) rather than sitting
 *  beside it. */
export async function paneCovers(page: Page): Promise<boolean> {
  return page.locator('.canvas-split > .page').evaluate(el => getComputedStyle(el).display === 'none');
}

/** With the Preview pane open, the workspace conversation is beside it or,
 *  where the pane covers the page, mounted and hidden under it. */
export async function expectConversationWithPane(page: Page) {
  await expect(previewPane(page)).toBeVisible();
  const conversation = page.getByRole('region', { name: 'Workspace conversation' });
  // Counted with hidden ones: a region under a covering pane is display:none,
  // which getByRole leaves out by default.
  await expect(page.getByRole('region', { name: 'Workspace conversation', includeHidden: true })).toHaveCount(1);
  if (await paneCovers(page)) await expect(conversation).toBeHidden();
  else await expect(conversation).toBeVisible();
}

/** Open the project's files (nothing when the pane already shows one): the
 *  Preview pane opens on WORKSPACE.md beside the conversation. */
export async function openProjectFiles(page: Page) {
  const toggle = projectFilesButton(page);
  await expect(toggle).toBeVisible();
  if (await toggle.getAttribute('aria-pressed') !== 'true') await toggle.click();
  await expect(toggle).toHaveAttribute('aria-pressed', 'true');
  await expectConversationWithPane(page);
}

/** Open one of the project's notes (`WORKSPACE.md`, `PLAN.md`,
 *  `SOURCES.md`, `AGENTS.md`) through Project files and the Files picker. */
export async function openProjectFile(page: Page, name: string) {
  await openProjectFiles(page);
  await expect(filesPicker(page)).toBeVisible();
  await choose(filesPicker(page), { label: name });
  await expect(previewPane(page).getByRole('tab', { name, exact: true })).toHaveAttribute('aria-selected', 'true');
}

/** Hide the Preview pane with Project files: the same conversation stays. */
export async function closeProjectFiles(page: Page) {
  await projectFilesButton(page).click();
  await expect(previewPane(page)).toHaveCount(0);
  await expect(projectFilesButton(page)).toHaveAttribute('aria-pressed', 'false');
  await expect(page.getByRole('region', { name: 'Workspace conversation' })).toBeVisible();
}

/** Open a workspace's file (vault-relative, or absolute) in the Preview pane
 *  as the app's own result links do (`openWorkspaceResult`): a file in
 *  `outputs/` opens with its review bar. */
export async function openWorkspaceResult(page: Page, workspaceId: string, path: string) {
  await page.evaluate(async ({ id, path }) => {
    const store = '/src/store/workspaces.ts';
    const { openWorkspaceResult } = await import(/* @vite-ignore */ store);
    await openWorkspaceResult(id, path);
  }, { id: workspaceId, path });
  await expect(previewPane(page)).toBeVisible();
}

/** The review bar of a Result open in the Preview pane. */
export function resultReviewBar(page: Page): Locator {
  return previewPane(page).getByRole('region', { name: 'Result review', exact: true });
}

/** A build workspace's Changes in the Preview pane: the top bar's branch. */
export async function openChanges(page: Page, branch: string) {
  await page.locator('.top-bar').getByRole('button', { name: `Changes on ${branch}`, exact: true }).click();
  await expect(previewPane(page).getByRole('tab', { name: 'Changes', exact: true })).toHaveAttribute('aria-selected', 'true');
}

/** The sidebar (Rail.tsx's `<aside>`). */
export function sidebar(page: Page): Locator {
  return page.getByRole('complementary', { name: 'Main navigation' });
}

/** New chat's visible home: the + on the sidebar's Quick chats heading
 *  (quick chats came back 2026-11-14; the COO keeps the top bar's orb). */
export function newChatControl(page: Page): Locator {
  return page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: 'New quick chat', exact: true });
}

/** Search chats: the top bar's search icon beside the sidebar's toggle (L1,
 *  11-12; it was the sidebar's row). It opens the sidebar's search, showing
 *  a hidden sidebar first. */
export function sidebarSearch(page: Page): Locator {
  return page.locator('.top-bar').getByRole('button', { name: 'Search chats', exact: true });
}

/** A sidebar control, shown. Below 860 px the sidebar is an overlay behind
 *  the top bar's Show sidebar, clicked first as a person would (routeWalk.ts
 *  reaches ledger targets the same way). Says whether it opened it. */
export async function reachSidebar(page: Page, control: Locator): Promise<{ target: Locator; opened: boolean }> {
  const target = control.filter({ visible: true }).first();
  const show = page.getByRole('button', { name: 'Show sidebar', exact: true });
  await expect(target.or(show).first()).toBeVisible();
  if (await target.isVisible()) return { target, opened: false };
  await show.click();
  await expect(target).toBeVisible();
  return { target, opened: true };
}

/** Closes an overlay sidebar `reachSidebar` opened, unless the click already
 *  did (a destination slides it away) or a dialog now sits over it. */
async function closeOverlay(page: Page, opened: boolean) {
  if (!opened) return;
  const close = sidebar(page).getByRole('button', { name: 'Close sidebar', exact: true });
  await close.waitFor({ state: 'hidden', timeout: 1500 }).catch(async () => {
    if (await page.getByRole('dialog').filter({ visible: true }).count()) return;
    if (await close.isVisible()) await close.click();
  });
}

/** A page in the sidebar's Main navigation (Projects, Tasks, Library, …). */
async function openMain(page: Page, name: string | RegExp) {
  const row = page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', typeof name === 'string' ? { name, exact: true } : { name });
  const { target, opened } = await reachSidebar(page, row);
  await target.click();
  await closeOverlay(page, opened);
}

/** Library, from the sidebar. */
export async function openLibrary(page: Page) {
  await openMain(page, 'Library');
  await expect(page.getByRole('heading', { name: 'Library', level: 1 })).toBeVisible();
}

/** New project, the Add sheet and new conversations, from their visible
 *  homes since the top bar's New menu went (overnight 11-04 U1): the
 *  Quick chats + and Projects +, the open
 *  project's ⋯ → New conversation (its sidebar + went in L8), and Library
 *  actions → Add. Callers still name the menu's items. */
export async function chooseNew(page: Page, item: 'New project' | 'New conversation' | 'New chat' | 'Add a task, note, link, file or recording…') {
  if (item === 'Add a task, note, link, file or recording…') {
    await openLibrary(page);
    await page.getByRole('button', { name: 'Library actions', exact: true }).click();
    await page.getByRole('menuitem', { name: item, exact: true }).click();
    return;
  }
  if (item === 'New conversation') {
    // The open project's ⋯ beside its name (the top bar, or Work's header).
    const open = (await page.locator('.work-header h1').first().textContent({ timeout: 5000 }))?.trim();
    if (!open) throw new Error("chooseNew('New conversation'): open a project first; its ⋯ is the route");
    await page.locator('.work-more-btn').filter({ visible: true }).first().click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'New conversation', exact: true }).click();
    return;
  }
  if (item === 'New chat') { await newChatControl(page).click(); return; }
  const control = sidebar(page).getByRole('button', { name: 'New project', exact: true });
  const { target, opened } = await reachSidebar(page, control);
  await target.click();
  await closeOverlay(page, opened);
}

/** Opens a chat by its exact title from the sidebar's Search chats (L8,
 *  11-10: the sidebar lists no conversations, a project row opens its
 *  lead). In a narrow window the search shows the hidden sidebar and hides
 *  it again once the chat opens. */
export async function openChat(page: Page, title: string) {
  await sidebarSearch(page).click();
  const field = page.getByRole('combobox', { name: 'Search chats' });
  await expect(field).toBeFocused();
  await field.fill(title);
  const exact = new RegExp('^' + title.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '$');
  await page.getByRole('listbox', { name: 'Search results' }).getByRole('option')
    .filter({ has: page.locator('.work-rail-search-title', { hasText: exact }) }).first().click();
  await expect(field).toHaveCount(0);
}

/** Seeds, before the next load, one chat in the built-in Quick chats
 *  workspace (`quick-chats`); `openChat` opens it. Once per tab, so a reload keeps it. */
export async function seedChatsChat(page: Page, title: string) {
  await page.addInitScript(title => {
    if (sessionStorage.getItem('chatsChatSeeded')) return;
    sessionStorage.setItem('chatsChatSeeded', '1');
    const cwd = '/Users/fixture/vault/workspaces/quick-chats', at = new Date().toISOString();
    const s = { key: 's-20261110-quick001', title, agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
    const list = JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]') as { id: string }[];
    localStorage.setItem('mockWorkspaces', JSON.stringify([...list.filter(w => w.id !== 'quick-chats'), { id: 'quick-chats', title: 'Quick chats', kind: 'quick', status: 'active', path: 'workspaces/quick-chats', cwd, modified: at }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([...JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]'), s]));
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ ...JSON.parse(localStorage.getItem('mockWorkspaceConversations') ?? '{}'), [s.key]: { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked in ${title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: at, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: at, status: 'success', transcriptIncomplete: false },
    ] } }));
  }, title);
}

/** Global search (the command palette) from the sidebar's search entry,
 *  which replaced the top bar's Search anything (overnight 11-04 U1). S1's
 *  Search opens the palette itself; S2's Search chats keeps chat results in
 *  the sidebar and opens the palette from its last row, Search everything.
 *  Returns the open palette. */
export async function openGlobalSearch(page: Page): Promise<Locator> {
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await sidebarSearch(page).click();
  const name = /^Search everything/;
  const everything = sidebar(page).getByRole('button', { name })
    .or(sidebar(page).getByRole('option', { name }))
    .or(sidebar(page).getByRole('link', { name }));
  await expect(palette.or(everything).first()).toBeVisible();
  if (!(await palette.isVisible())) await everything.first().click();
  await expect(palette).toBeVisible();
  return palette;
}

/** Work → Running, live work's home, which also holds the background task
 *  log the top bar's activity dock showed (overnight 11-04 U1). Tolerates
 *  U5's Work naming. Returns the Running region. */
export async function openRunning(page: Page): Promise<Locator> {
  await openMain(page, /^(Your work|Work|Projects)\b/);
  await page.getByRole('navigation', { name: /^(Your work|Work|Projects) views$/ }).getByRole('button', { name: 'Running', exact: true }).click();
  const running = page.getByRole('region', { name: 'Running', exact: true });
  await expect(running).toBeVisible();
  return running;
}

/** Running in its List layout: one card (an article named "<title>,
 *  <state>") per running or queued turn. */
export async function runningList(page: Page): Promise<Locator> {
  const running = await openRunning(page);
  await running.getByRole('group', { name: 'Running layout' }).getByRole('button', { name: 'List', exact: true }).click();
  const list = running.getByRole('list', { name: 'Running and queued turns' });
  await expect(list).toBeVisible();
  return list;
}

/** Running's background task log: what trackTask records (Run now, sync,
 *  vault writes), formerly the dock's popover. Present only with entries. */
export function backgroundTasks(page: Page): Locator {
  return page.getByRole('region', { name: 'Running', exact: true }).getByRole('region', { name: 'Background tasks', exact: true });
}
export async function openBackgroundTasks(page: Page): Promise<Locator> {
  await openRunning(page);
  const log = backgroundTasks(page);
  await expect(log).toBeVisible();
  return log;
}
