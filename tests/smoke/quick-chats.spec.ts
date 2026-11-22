import { choose, openChat } from './choose';
import { showWorkspaceList } from './choose';
import { test, expect, type Page } from './fixtures/test';

// Chats with no project and moving conversations (2026-09-30): ⌘N from
// Home, Ask anything and Move to project… (Add to project… until overnight
// 11-04 S3). Since L8 (11-10) a prompt with no project goes to the COO, and
// the sidebar lists no conversations (its Chats section went), so a chat
// opens from Search chats and its menu is the ⋯ beside its name.
const cwd = (id: string) => `/Users/fixture/vault/workspaces/${id}`;
const workspace = (id: string, title: string) =>
  ({ id, title, status: 'active', path: `workspaces/${id}`, cwd: cwd(id), modified: '2026-09-26T12:00:00Z' });
const session = (key: string, title: string, ws: string, minutesAgo: number) => {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return { key, title, agent: 'claude', harnessSessionId: 'sess-native', cwd: cwd(ws), createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' };
};

async function seed(page: Page) {
  await page.addInitScript(([workspaces, sessions]) => {
    if (sessionStorage.getItem('quickSeeded')) return;
    sessionStorage.setItem('quickSeeded', '1');
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.setItem('hollow.railWorkspaces', JSON.stringify(['alpha-1', 'beta-2']));
    localStorage.setItem('mockWorkspaces', JSON.stringify(workspaces));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    const conversations: Record<string, unknown> = {};
    for (const s of sessions as any[]) conversations[s.key] = { summary: s, eventsRetained: true, turns: [
      { id: `${s.key}-u`, role: 'user', text: `Asked in ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, transcriptIncomplete: false },
      { id: `${s.key}-a`, role: 'assistant', text: `Answer for ${s.title}`, tools: [], agent: 'claude', runId: `run-${s.key}`, startedAt: s.createdAt, status: 'success', transcriptIncomplete: false },
    ] };
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(conversations));
  }, [
    [workspace('alpha-1', 'Alpha'), workspace('beta-2', 'Beta')],
    [session('s-20260930-aaaa0001', 'Draft the launch memo', 'alpha-1', 5), session('s-20260930-aaaa0002', 'Review pricing', 'alpha-1', 180),
     session('s-20260930-bbbb0001', 'Beta thread', 'beta-2', 60)],
  ] as const);
  await page.goto('/?mock=1');
}

const rail = (page: Page) => page.getByRole('navigation', { name: 'Project navigation' });
const header = (page: Page) => page.locator('.work-header h1');
/** The open conversation's menu: the ⋯ beside its name. */
async function conversationMenu(page: Page, item: string) {
  await page.getByRole('group', { name: 'Conversation', exact: true }).getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: item, exact: true }).click();
}

test('conversation drafts stay separate across project switches and reload', async ({ page }) => {
  await seed(page);
  const composer = page.getByLabel('Chat message', { exact: true });
  await openChat(page, 'Draft the launch memo');
  await expect(page.locator('.chat-main')).toContainText('Answer for Draft the launch memo');
  await composer.fill('Unsent launch follow-up');
  await openChat(page, 'Review pricing');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
  await expect(composer).toHaveValue('');
  await composer.fill('Unsent pricing follow-up');
  await openChat(page, 'Draft the launch memo');
  await expect(composer).toHaveValue('Unsent launch follow-up');
  await page.reload();
  await expect(composer).toHaveValue('Unsent launch follow-up');
  await openChat(page, 'Review pricing');
  await expect(composer).toHaveValue('Unsent pricing follow-up');
});

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'The rail tree collapses to icons on narrow screens'); });

test('the sidebar lists Recent and Quick chats; ⌘N from Home opens a quick chat at once', async ({ page }) => {
  await seed(page);
  // Projects under Recent (L1, 11-12), then the Quick chats section (back
  // 2026-11-14), whose + starts a chat with no project.
  await expect.poll(() => rail(page).locator('.work-rail-section-toggle .work-rail-label').allTextContents()).toEqual(['Recent', 'Quick chats']);
  await expect(rail(page).getByRole('group', { name: 'Chats', exact: true })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'New chat', exact: true })).toHaveCount(0);
  // Work never lists the COO as a project.
  await showWorkspaceList(page);
  await expect(page.locator('.work-card')).toHaveText([/Alpha|Beta/, /Alpha|Beta/]);

  await page.locator('body').click({ position: { x: 700, y: 20 } });
  await page.keyboard.press('Meta+n');
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await expect(page.getByLabel('Chat message')).toBeFocused();
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toHaveCount(0);
  // The Quick chats workspace is made on first use.
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]').some((w: any) => w.id === 'quick-chats'))).toBe(true);
  expect(await page.evaluate(() => window.__mockApiCalls?.chat_send ?? 0)).toBe(0);
});

test('Ask anything on Work sends its text to a new quick chat', async ({ page }) => {
  await seed(page);
  await page.getByRole('textbox', { name: 'Ask anything' }).fill('What is a good name for a cat?');
  // The button says what it does (L4, X15): New chat sends it at once.
  await page.getByRole('form', { name: 'Ask' }).getByRole('button', { name: 'Start the chat', exact: true }).click();
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockChatSendCalled'))).toBe('true');
  const args = await page.evaluate(() => JSON.parse(localStorage.getItem('mockLastChatArgs') ?? '{}'));
  expect(args.prompt).toContain('What is a good name for a cat?');
  expect(args.cwd).toBe(cwd('quick-chats'));
  await expect(page.locator('.chat-main')).toContainText('What is a good name for a cat?');
});

test('Move to project confirms a selected destination and preserves the new conversation picker', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Review pricing');
  await conversationMenu(page, 'Move to project…');
  const picker = page.getByRole('dialog', {name:'Move to project',exact:true});
  await choose(picker.getByRole('combobox', {name:'Project',exact:true}), 'beta-2');
  expect(await page.evaluate(() => localStorage.getItem('mockLastMove'))).toBeNull();
  await picker.getByRole('button', {name:'Move conversation',exact:true}).click();
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockLastMove') ?? 'null'))).toEqual({ key: 's-20260930-aaaa0002', to: 'beta-2' });
  await expect(page.locator('.snackbar').filter({ hasText: 'Moved to Beta' })).toBeVisible();
  await expect(picker).toHaveCount(0);
  // The open conversation follows it.
  await expect(header(page)).toHaveText('Beta');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');

  // The palette's New conversation (nothing open) lists Chats first, which
  // opens the COO since L8 (11-10).
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('New conversation');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('combobox', { name: 'Choose a workspace for the new conversation' })).toBeFocused();
  await expect(page.locator('.palette-row .palette-row-label').first()).toHaveText('The COO');
  await page.keyboard.press('Enter');
  await expect(page.locator('.work-conversation').getByRole('textbox', { name: 'Chat message' })).toBeVisible();
});

test('New project from this chat creates it from the conversation and opens it there', async ({ page }) => {
  await seed(page);
  await openChat(page, 'Review pricing');
  await conversationMenu(page, 'New project from this chat');
  await expect(header(page)).toHaveText('Review pricing');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
  await expect(rail(page).getByRole('button', { name: 'Review pricing', exact: true })).toHaveAttribute('aria-current', 'true');
  const brief = await page.evaluate(() => Object.entries(JSON.parse(localStorage.getItem('mockWorkspaceNotes') ?? '{}')).find(([p]) => p.endsWith('WORKSPACE.md') && String(p).includes('work-'))?.[1]);
  expect(String(brief)).toContain('Asked in Review pricing');
});

// P1-39: a move that fails after the workspace was made takes that workspace
// back out, and says why, instead of leaving an empty one behind.
test('New project from this chat leaves no workspace when the move fails, saying why', async ({ page }) => {
  await seed(page);
  await page.evaluate(() => localStorage.setItem('mockMoveError', 'The conversation record is locked by another write'));
  await openChat(page, 'Review pricing');
  await conversationMenu(page, 'New project from this chat');
  await expect(page.locator('.snackbar').filter({ hasText: 'The conversation record is locked by another write' }).first()).toContainText('The new workspace was removed');
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]').filter((w: any) => w.title === 'Review pricing').length)).toBe(0);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockTrashedWorkspaces') ?? '[]').map((r: any) => r.workspace.title))).toEqual(['Review pricing']);
  await expect(rail(page).getByRole('button', { name: 'Review pricing', exact: true })).toHaveCount(0);
  await expect(header(page)).toHaveText('Alpha');
  await expect(page.locator('.chat-main')).toContainText('Answer for Review pricing');
});
