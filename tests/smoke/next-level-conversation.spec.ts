import { showWorkspaceList } from './choose';
import { expect, test, type Page } from './fixtures/test';

/**
 * Next-level P04 against the demo fixtures: find in a conversation, the
 * outline of prompts, an honest "continue in a new conversation"
 * handoff, and bounded rendering of a 1,000-turn thread. Browser mock only.
 */

async function openConversation(page: Page, workspace: string, demo = '1') {
  await page.goto(`/?mock=1&demo=${demo}`);
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: workspace }).click();
  await expect(page.locator('.turn').first()).toBeVisible();
}

test.beforeEach(({}, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'conversation tools are checked at desktop width; narrow keeps the same DOM');
});

test('⌘F finds in the conversation and steps through matches', async ({ page }) => {
  await openConversation(page, 'Q3 regional operations review');
  await page.locator('.chat-scroll').click({ position: { x: 20, y: 20 } });
  await page.keyboard.press('ControlOrMeta+f');
  const find = page.getByRole('search', { name: 'Find in this conversation' });
  await expect(find).toBeVisible();
  await find.getByRole('searchbox').fill('section 3');
  await expect(find.locator('.chat-find-count')).toHaveText(/1 of [2-9]/);
  await expect(page.locator('.turn.turn-hit')).toHaveCount(1);
  // Overnight 11-04 (C4): the matched words are marked, the current turn's
  // apart (CSS Custom Highlight API, findHighlight.ts), and the strong marks
  // move with Enter to the turn the bar is on.
  const marks = (name: string) => page.evaluate(n => (CSS as unknown as { highlights?: Map<string, { size: number }> }).highlights?.get(n)?.size ?? 0, name);
  const markedTurn = () => page.evaluate(() => {
    const strong = (CSS as unknown as { highlights?: Map<string, Set<Range>> }).highlights?.get('chat-find-current');
    const range = strong ? [...strong][0] : undefined;
    return range?.startContainer.parentElement?.closest('[data-turn-index]')?.getAttribute('data-turn-index') ?? null;
  });
  const first = await page.locator('.turn.turn-hit').getAttribute('data-turn-index');
  await expect.poll(markedTurn).toBe(first);
  await find.getByRole('searchbox').press('Enter');
  await expect(find.locator('.chat-find-count')).toHaveText(/2 of/);
  const second = await page.locator('.turn.turn-hit').getAttribute('data-turn-index');
  expect(second).not.toBe(first);
  await expect.poll(markedTurn).toBe(second);
  await find.getByRole('searchbox').press('Escape');
  await expect(find).toHaveCount(0);
  await expect.poll(() => marks('chat-find-current')).toBe(0);
  expect(await marks('chat-find')).toBe(0);
});

test('the outline lists prompts and has no bookmarks', async ({ page }) => {
  await openConversation(page, 'Q3 regional operations review');
  await page.getByRole('button', { name: 'Actions for this conversation', exact: true }).click();
  await page.getByRole('menuitem', { name: 'Outline', exact: true }).click();
  const outline = page.getByRole('region', { name: 'Outline', exact: true });
  await expect(outline.locator('li').first()).toBeVisible();
  await expect(outline).not.toContainText('bookmarked');
  await expect(page.locator('.turn.turn-bookmarked')).toHaveCount(0);
  await expect(page.getByRole('menuitem', { name: 'Bookmark this reply' })).toHaveCount(0);
});

test('continue from here starts a new conversation with a labelled written handoff', async ({ page }) => {
  await openConversation(page, 'Q3 regional operations review');
  const footer = page.locator('.turn-footer').last();
  await footer.hover();
  await footer.getByRole('button', { name: 'More actions' }).click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Continue from here in a new conversation' }).click();
  const composer = page.getByLabel('Chat message');
  await expect(composer).toHaveValue(/^Context handoff from “Revise the labour section” \(a written summary, not a resumed session/);
  await expect(composer).toHaveValue(/Continue from here: $/);
  // The original conversation is untouched and still kept (the sidebar lists
  // no conversations since L8, 11-10).
  expect(await page.evaluate(() => (JSON.parse(localStorage.getItem('mockWorkspaceSessions') ?? '[]') as { title?: string; archived?: boolean }[])
    .some(s => s.title === 'Revise the labour section' && !s.archived))).toBe(true);
  await expect(page.locator('.turn')).toHaveCount(0);
});

test('a 1,000-turn thread renders a bounded window and find reaches early turns', async ({ page }) => {
  await page.goto('/?mock=1&demo=large');
  await showWorkspaceList(page);
  await page.locator('.work-card').filter({ hasText: 'Results view for workspaces' }).click();
  const started = Date.now();
  await expect(page.locator('.turn').first()).toBeVisible();
  const openMs = Date.now() - started;
  const rendered = await page.locator('.turn').count();
  expect(rendered).toBeLessThanOrEqual(80);
  await expect(page.getByRole('button', { name: /Show 80 earlier messages/ })).toBeVisible();
  await page.locator('.chat-scroll').click({ position: { x: 20, y: 20 } });
  await page.keyboard.press('ControlOrMeta+f');
  await page.getByRole('search', { name: 'Find in this conversation' }).getByRole('searchbox').fill('needle-17');
  await expect(page.locator('.turn.turn-hit')).toHaveAttribute('data-turn-index', '6');
  await expect(page.locator('.turn.turn-hit')).toBeInViewport();
  test.info().annotations.push({ type: 'perf', description: `open 1,000-turn conversation: ${openMs} ms to first turn; ${rendered} turns rendered` });
});
