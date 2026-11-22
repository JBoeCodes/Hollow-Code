import { test, expect, type Page } from './fixtures/test';

const key = 's-20261120-32aaaaaa';
async function seed(page: Page) {
  await page.addInitScript(key => {
    const cwd = '/Users/fixture/vault/workspaces/quick-chats';
    const at = '2026-11-20T12:00:00Z';
    const session = { key, title: 'Drag example', agent: 'claude', harnessSessionId: null, cwd, createdAt: at, lastActivityAt: at, endedAt: null, turnCount: 1, parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'none' };
    localStorage.removeItem('hollow.railCollapsed');
    localStorage.removeItem('hollow.workspace');
    localStorage.removeItem('hollow.railSections.v1');
    localStorage.setItem('hollow.studio.panelAnimationMs', '200');
    localStorage.setItem('mockWorkspaces', JSON.stringify([{ id: 'quick-chats', title: 'Quick chats', kind: 'quick', status: 'active', path: 'workspaces/quick-chats', cwd, modified: at }]));
    localStorage.setItem('mockWorkspaceSessions', JSON.stringify([session]));
    localStorage.setItem('mockWorkspaceConversations', JSON.stringify({ [key]: { summary: session, turns: [], eventsRetained: true } }));
  }, key);
  await page.goto('/?mock=1');
}
const row = (page: Page) => page.locator(`.work-rail [data-session-key="${key}"]`).first();
async function drop(page: Page, section: string) {
  const source = await row(page).elementHandle();
  const data = await page.evaluateHandle(() => new DataTransfer());
  await row(page).dispatchEvent('dragstart', { dataTransfer: data });
  const destination = page.getByRole('group', { name: 'Move conversation between sections' }).locator('.rail-drop-zone').filter({ has: page.locator('span').filter({ hasText: new RegExp(`^${section}$`) }) });
  await expect(destination).toBeVisible();
  await destination.dispatchEvent('dragover', { dataTransfer: data });
  await expect(destination).toHaveClass(/rail-drop-active/);
  await destination.dispatchEvent('drop', { dataTransfer: data });
  await source?.evaluate(element => element.dispatchEvent(new DragEvent('dragend', { bubbles: true })));
  await data.dispose();
}

test.beforeEach(async ({}, info) => { test.skip(info.project.name.includes('narrow'), 'Sidebar conversation rows are hidden when collapsed'); });

test('drag Pin and Active share the visible menu commands', async ({ page }) => {
  await seed(page);
  await drop(page, 'Pinned');
  await expect(page.getByRole('group', { name: 'Pinned chats', exact: true }).locator(`[data-session-key="${key}"]`)).toBeVisible();
  await row(page).getByRole('button', { name: 'Actions for chat Drag example', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: 'Unpin conversation', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  await drop(page, 'Active');
  await expect(page.getByRole('group', { name: 'Quick chats', exact: true }).locator(`[data-session-key="${key}"]`)).toBeVisible();
});

test('reduced motion keeps the drag destinations usable and the row still', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await seed(page);
  await drop(page, 'Pinned');
  await expect(row(page)).toBeVisible();
  expect(await row(page).evaluate(element => element.getAnimations().length)).toBe(0);
  await row(page).getByRole('button', { name: 'Actions for chat Drag example', exact: true }).click();
  await expect(page.getByRole('menuitem', { name: /Settle conversation \(unpin/ })).toHaveAttribute('aria-disabled', 'true');
});
