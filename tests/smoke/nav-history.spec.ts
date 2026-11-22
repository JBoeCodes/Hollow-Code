import { test, expect, type Page } from './fixtures/test';
import { openChat } from './choose';
import { openLibraryView } from './libraryView';

// P1-42: Back and Forward step through workspaces; the top bar's buttons
// name their target; the native menu offers Copy on selected text; the
// project's menu opens Conversation history for that workspace. The
// sidebar's Show older went with its conversation lists (L8, 11-10).
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z' });
const session = (key: string, title: string, ws: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: null, cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
};

async function seed(page: Page) {
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('navSeeded')) return;
    sessionStorage.setItem('navSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2']));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    // Beta has more conversations than the rail's list carries.
    localStorage.setItem('mockSessionTotals', JSON.stringify({ 'beta-2': 9 }));
    const conversations: Record<string, unknown> = {};
    for (const s of sessions as { key: string; title: string; createdAt: string }[]) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [
    [workspace('alpha-1', 'Alpha'), workspace('beta-2', 'Beta')],
    [session('s-20260926-aaaa0001', 'Draft the launch memo', 'alpha-1', 5),
     ...Array.from({ length: 6 }, (_, i) => session(`s-20260926-bbbb000${i}`, `Beta thread ${i}`, 'beta-2', 60 * (i + 1)))],
  ] as const);
  await page.goto('/?mock=1');
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
const main = (page: Page) => page.getByRole('navigation', { name: 'Main' });
const openTitle = (page: Page) => page.locator('.work-header h1');

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('⌘[ and ⌘] step through the workspaces opened, not past them', async ({ page }) => {
  await seed(page);
  await rail(page).getByRole('button', { name: 'Alpha', exact: true }).click();
  await expect(openTitle(page)).toHaveText('Alpha');
  await rail(page).getByRole('button', { name: 'Beta', exact: true }).click();
  await expect(openTitle(page)).toHaveText('Beta');
  await main(page).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();

  await page.locator('body').click({ position: { x: 1, y: 1 } });
  await page.keyboard.press('Meta+[');
  await expect(openTitle(page)).toHaveText('Beta');
  await page.keyboard.press('Meta+[');
  await expect(openTitle(page)).toHaveText('Alpha');
  await page.keyboard.press('Meta+]');
  await expect(openTitle(page)).toHaveText('Beta');
  // The top bar's Forward goes on to Tasks, and then has nowhere to go.
  const history = page.getByRole('group', { name: 'History' });
  await history.getByRole('button', { name: 'Forward', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect(history.getByRole('button', { name: 'Forward', exact: true })).toBeDisabled();
});

test('the sidebar names the project its buttons go to, and is the way back to a conversation', async ({ page }) => {
  await seed(page);
  // The top bar's New menu and Back to … conversation went (overnight 11-04
  // U1): New chat goes to the COO (its sidebar row went in L1, 11-12: the
  // orb is its home), and the project's own sidebar row (its lead) is the
  // way back. The rows have no + since L8 (11-10), so no New
  // conversation button is there to pick a project silently (P1-42).
  await main(page).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('button', { name: /^Back to .* conversation$/ })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'New chat', exact: true })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Everywhere' }).getByRole('button', { name: /^COO/ })).toBeVisible();
  await expect(rail(page).getByRole('button', { name: /^New conversation/ })).toHaveCount(0);
  for (const name of ['Alpha', 'Beta']) await expect(rail(page).getByRole('button', { name, exact: true })).toBeVisible();

  await rail(page).getByRole('button', { name: 'Beta', exact: true }).click();
  await expect(openTitle(page)).toHaveText('Beta');
  await main(page).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('button', { name: /^Back to .* conversation$/ })).toHaveCount(0);
  await rail(page).getByRole('button', { name: 'Beta', exact: true }).click();
  await expect(page.getByLabel('Chat message')).toBeFocused();
  await expect(openTitle(page)).toHaveText('Beta');
});

test('selected reply text gets the Mac menu; a rail row keeps Hollow’s', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Draft the launch memo');
  const reply = page.getByText('Answer for Draft the launch memo', { exact: true });
  await expect(reply).toBeVisible();
  // A right-click at the middle of `which`'s text, with that text selected
  // or not; true when Hollow suppressed the native menu.
  const suppressed = (which: 'reply' | 'row', select: boolean, below = 0) => page.evaluate(([which, select, below]) => {
    const el = which === 'row'
      ? document.querySelector('.work-workspace-row') as HTMLElement
      : [...document.querySelectorAll('p')].find(p => p.textContent === 'Answer for Draft the launch memo') as HTMLElement;
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    const range = document.createRange(); range.selectNodeContents(el);
    if (select) selection.addRange(range);
    const box = range.getBoundingClientRect();
    const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, button: 2, clientX: box.left + box.width / 2, clientY: box.top + box.height / 2 + (below as number) });
    el.dispatchEvent(event);
    return event.defaultPrevented;
  }, [which, select, below] as const);
  // Selected text, nothing of Hollow's handles it: the native menu (Copy, Look Up).
  expect(await suppressed('reply', true)).toBe(false);
  // No selection, or a click below the selected text: suppressed, as before.
  expect(await suppressed('reply', false)).toBe(true);
  expect(await suppressed('reply', true, 60)).toBe(true);
  // A rail row has its own menu, even over selected text.
  expect(await suppressed('row', true)).toBe(true);
  await expect(page.locator('.ctx-menu')).toBeVisible();
});

test('the project’s right-click opens Conversation history for it, with Show all', async ({ page }) => {
  await seed(page);
  // The project's menu keeps Conversation history, filtered to it.
  await rail(page).getByRole('button', { name: 'Beta', exact: true }).click({ button: 'right' });
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Open in Conversation history', exact: true }).click();
  // Library → Conversations since S4.
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Conversations', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const list = page.getByRole('complementary', { name: 'Conversations' });
  await expect(list.getByRole('status')).toContainText('Only Beta');
  await expect(list.locator('.chat-session-open .title')).toHaveText(Array.from({ length: 6 }, (_, i) => `Beta thread ${i}`));
  await list.getByRole('button', { name: 'Show all', exact: true }).click();
  await expect(list.getByRole('status')).toHaveCount(0);
  await expect(list.locator('.chat-session-open .title').filter({ hasText: 'Beta thread' })).toHaveCount(0);
});

// S4 (2026-11-03): Conversation history is Library → Conversations, a step of
// its own for Back and Forward; the rail's Library still opens Library's view.
test('Conversation history is a Library tab that Back and Forward step through', async ({ page }) => {
  await seed(page);
  const views = page.getByRole('navigation', { name: 'Library view' });
  const history = page.getByRole('heading', { name: 'Conversation history', exact: true });
  const notes = page.getByRole('complementary', { name: 'Library files', exact: true });
  await main(page).getByRole('button', { name: 'Library', exact: true }).click();
  // Library opens on All (overnight 11-12 L3; it was Results).
  await expect(views.getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  // Notes is All's view (overnight 11-12 L4): All stays selected, Back to All shows.
  await openLibraryView(page, 'Notes');
  await expect(notes).toBeVisible();
  await views.getByRole('button', { name: 'Conversations', exact: true }).click();
  await expect(history).toBeVisible();
  await expect(main(page).getByRole('button', { name: 'Library', exact: true })).toHaveAttribute('aria-current', 'page');
  await page.locator('body').click({ position: { x: 1, y: 1 } });
  await page.keyboard.press('Meta+[');
  await expect(notes).toBeVisible();
  await expect(views.getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(history).toHaveCount(0);
  await page.keyboard.press('Meta+]');
  await expect(history).toBeVisible();
  await main(page).getByRole('button', { name: 'Library', exact: true }).click();
  await expect(notes).toBeVisible();
  await expect(history).toHaveCount(0);
});
