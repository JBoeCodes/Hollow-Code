/**
 * Plan L3 (2026-10-14; matrix D3, review U workspace "keep draft-preservation
 * and side-panel replacement behaviour at narrow widths"): a half-written
 * message survives the workspace/Result loop. Asking for a revision adds to
 * the draft rather than replacing it, a second click doesn't add the request
 * twice, and the draft is still there after the Preview pane took the
 * conversation's place (narrow and minimum windows) and after leaving the
 * workspace and coming back. Runs at every size, Largest text included.
 * The Result opens in the Preview pane under its review bar (COO rebuild L4:
 * it was the workspace panel's Results section).
 */
import { expect, test, type Page } from './fixtures/test';
import { closeProjectFiles, openWorkspaceResult, previewPane, resultReviewBar } from './choose';

const Q3 = 'local-q3-review';
const REVIEW = 'workspaces/local-q3-review/outputs/q3-regional-review.md';

async function openQ3(page: Page) {
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
}

/** The conversation, on screen: narrow windows show the Preview pane in its
 *  place, and Project files (pressed) hides it. */
async function showConversation(page: Page) {
  const message = page.getByLabel('Chat message');
  await expect(message.or(previewPane(page)).first()).toBeVisible();
  if (!(await message.isVisible())) await closeProjectFiles(page);
  await expect(message).toBeVisible();
  return message;
}

test('a draft survives asking for a revision, the Preview pane and leaving the workspace', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await openQ3(page);
  const message = await showConversation(page);
  await message.fill('Keep the regional split as it is.');

  await openWorkspaceResult(page, Q3, REVIEW);
  const ask = resultReviewBar(page).getByRole('button', { name: 'Ask for a revision', exact: true });
  await ask.click();
  // Filling the message box keeps the conversation on screen, draft and all:
  // a covering Preview pane closes, one beside it stays; opening the Result
  // again brings it back for a second click.
  if (!(await ask.isVisible())) {
    await expect(message).toBeVisible();
    await openWorkspaceResult(page, Q3, REVIEW);
  }
  await ask.click();

  await showConversation(page);
  const text = await message.inputValue();
  expect(text.startsWith('Keep the regional split as it is.')).toBe(true);
  expect(text.match(/Revise /g)?.length, 'the revision request is added once').toBe(1);

  // Away and back: the draft is the conversation's, not the pane's.
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Projects/ }).first().click();
  await openQ3(page);
  await showConversation(page);
  await expect(message).toHaveValue(text);
});

/* ── P0-25 (moved from results-cards.spec.ts when the Results section went):
   the review bar's Ask for a revision opens the Result's workspace
   conversation before it touches the draft. ── */

test('P0-25 revision request opens the workspace conversation before changing its draft', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openQ3(page);
  await openWorkspaceResult(page, Q3, REVIEW);
  const cwd = await page.evaluate(async () => (await import(/* @vite-ignore */ '/src/store/workspaces.ts' as string)).workspaceDetail()!.workspace.cwd);
  await page.evaluate(async () => {
    const runtime = await import(/* @vite-ignore */ '/src/store/runtime.ts' as string); const { api } = await import(/* @vite-ignore */ '/src/api.ts' as string);
    runtime.setChatCwdPersistent('/Users/mock/foreign'); runtime.setChatInput('Foreign draft stays here');
    const read = api.readWorkspace; (window as any).__beforeDraftOpen = [];
    api.readWorkspace = async (...args: unknown[]) => { (window as any).__beforeDraftOpen.push(runtime.chatInput()); return read(...args); };
  });
  await resultReviewBar(page).getByRole('button', { name: 'Ask for a revision', exact: true }).click();
  await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ '/src/store/runtime.ts' as string)).chatCwd())).toBe(cwd);
  expect(await page.evaluate(() => (window as any).__beforeDraftOpen[0])).toBe('Foreign draft stays here');
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await expect(composer).toHaveValue(/Revise .*Q3 regional operations review/);
  await openWorkspaceResult(page, Q3, REVIEW);
  await resultReviewBar(page).getByRole('button', { name: 'Ask for a revision', exact: true }).click();
  expect((await composer.inputValue()).match(/Revise /g)).toHaveLength(1);
});

test('P0-25 a failed workspace open leaves the foreign draft untouched', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openQ3(page);
  await openWorkspaceResult(page, Q3, REVIEW);
  await page.evaluate(async () => {
    const runtime = await import(/* @vite-ignore */ '/src/store/runtime.ts' as string); const { api } = await import(/* @vite-ignore */ '/src/api.ts' as string);
    runtime.setChatCwdPersistent('/Users/mock/foreign'); runtime.setChatInput('Keep this foreign draft');
    api.readWorkspace = async () => { throw new Error('Workspace unavailable'); };
  });
  await resultReviewBar(page).getByRole('button', { name: 'Ask for a revision', exact: true }).click();
  await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ '/src/store/runtime.ts' as string)).chatInput())).toBe('Keep this foreign draft');
  await expect(page.getByText("Couldn't open this workspace's conversation, so nothing was added.", { exact: true })).toBeVisible();
});

test('P0-25 Library passage Ask keeps the current conversation for a workspace file', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the Library file list keeps its 280 px column beside an open file, so at 390 px the reader is 50 px wide (below the 720 px window minimum)');
  await page.goto('/?mock=1&demo=1');
  await openQ3(page);
  await page.evaluate(async () => {
    const runtime = await import(/* @vite-ignore */ '/src/store/runtime.ts' as string);
    const ws = (await import(/* @vite-ignore */ '/src/store/workspaces.ts' as string)).workspaceDetail()!.workspace;
    runtime.setActivePage('vault');
    runtime.setOpenVaultNoteRequest(`${ws.path}/outputs/q3-regional-review.md`);
  });
  const region = page.locator('.library-reader .selection-actions-region');
  await expect(region).toBeVisible();
  await page.evaluate(async () => {
    const runtime = await import(/* @vite-ignore */ '/src/store/runtime.ts' as string);
    runtime.setChatCwdPersistent('/Users/mock/foreign'); runtime.setChatInput('Library draft');
  });
  await region.evaluate(el => {
    const range = document.createRange(); range.selectNodeContents(el.querySelector('p')!);
    const selection = window.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    el.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
  });
  await page.getByRole('button', { name: 'Ask about this', exact: true }).click();
  await expect.poll(() => page.evaluate(async () => {
    const runtime = await import(/* @vite-ignore */ '/src/store/runtime.ts' as string);
    return { page: runtime.activePage(), cwd: runtime.chatCwd(), draft: runtime.chatInput() };
  })).toMatchObject({ page: 'vault', cwd: '/Users/mock/foreign', draft: expect.stringMatching(/^Library draft\n\nAbout this passage/) });
});
