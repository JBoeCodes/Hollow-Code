import { test, expect, type Page } from './fixtures/test';

const first = 's-20261120-34aaaaaa', second = 's-20261120-34bbbbbb';
async function seed(page: Page) {
  await page.addInitScript(([first, second]) => {
    const cwd = '/Users/fixture/vault/workspaces/quick-chats', at = '2026-11-20T12:00:00Z';
    const sessions = [first, second].map((key, index) => ({ key, title: index ? 'Target chat' : 'Original chat', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' }));
    localStorage.removeItem('hollow.railCollapsed'); localStorage.removeItem('hollow.workspace'); localStorage.removeItem('hollow.railSections.v1');
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'quick-chats', title: 'Quick chats', kind: 'quick', status: 'active', path: 'workspaces/quick-chats', cwd, modified: at }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify(sessions));
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify(Object.fromEntries(sessions.map(session => [session.key, { summary: session, turns: [], eventsRetained: true }]))));
  }, [first, second]);
  await page.goto('/?mock=1');
}
const row = (page: Page, key = second) => page.locator(`.work-rail [data-session-key="${key}"]`).first();

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'Sidebar conversation rows are hidden when collapsed'); });

test('hover and keyboard focus expose the same metadata, with a persistent Details route', async ({ page }) => {
  await seed(page);
  await row(page).hover();
  await expect(page.getByRole('tooltip')).toContainText('Target chat');
  await row(page).locator('[data-key-item]').first().focus();
  await page.mouse.move(700, 300);
  await expect(page.getByRole('tooltip')).toContainText('Last activity');
  await row(page).getByRole('button', { name: 'Actions for chat Target chat', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Details', exact: true }).click();
  const details = page.getByRole('dialog', { name: 'Conversation details' });
  await expect(details).toBeVisible();
  await expect(details).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(details).toHaveCount(0);
  await expect(row(page).locator('[data-key-item]').first()).toBeFocused();
});

test('file drop attaches to the exact row and preserves the other draft', async ({ page }) => {
  await seed(page);
  await row(page, first).locator('[data-key-item]').first().click();
  await page.getByRole('textbox', { name: 'Chat message', exact: true }).fill('Keep my original draft');
  await row(page).evaluate(element => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(new File(['file for the target'], 'row-note.txt', { type: 'text/plain' }));
    element.dispatchEvent(new DragEvent('dragover', { bubbles: true, cancelable: true, dataTransfer }));
    element.dispatchEvent(new DragEvent('drop', { bubbles: true, cancelable: true, dataTransfer }));
  });
  await expect.poll(() => page.evaluate(async moduleUrl => (await import(moduleUrl)).chatAgentSessionKey(), '/src/store/runtime.ts')).toBe(second);
  await expect(page.locator('.chat-attachments .chat-attach-file')).toContainText('row-note.txt');
  await row(page, first).locator('[data-key-item]').first().click();
  await expect(page.getByRole('textbox', { name: 'Chat message', exact: true })).toHaveValue('Keep my original draft');
  await expect(page.locator('.chat-attachments .chat-attach-file')).toHaveCount(0);
});
