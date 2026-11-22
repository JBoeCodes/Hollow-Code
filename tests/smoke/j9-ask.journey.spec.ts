/** Journey J9: inspect project notes and ask in the intended workspace. */
import { expect, test, type Page } from './fixtures/test';
import { choose, openProjectFile, closeProjectFiles, paneCovers, previewPane, projectFilesButton } from './choose';

const composer = (page: Page) => page.getByRole('textbox', { name: 'Chat message' });
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('J9 click-only: ask with this workspace or across all work', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');

  await openProjectFile(page, 'PLAN.md');

  // In this workspace: its own message box, unsent until Send. The Preview
  // pane sits beside the conversation where there is room and covers it in
  // a narrow window; Project files, pressed, hides it and returns to the
  // same chat.
  await expect(projectFilesButton(page)).toHaveAttribute('aria-pressed', 'true');
  if (await paneCovers(page)) await expect(composer(page)).toBeHidden();
  else await expect(composer(page)).toBeVisible();
  await closeProjectFiles(page);
  await composer(page).fill('Which decision is still open?');
  await expect(composer(page)).toHaveValue('Which decision is still open?');
  await expect(page.locator('.chat-main .turn.user').filter({ hasText: 'Which decision is still open?' })).toHaveCount(0);

  // Work → Ask anything → Across all my work (P1-11, D2; run 15 P3
  // removed the drawer): the Main agent page opens and Enter sends the
  // person's own words there. ⌥-Enter only places the text in its box.
  // At the minimum window the sidebar starts hidden behind its button.
  const showSidebar = page.getByRole('button', { name: 'Show sidebar', exact: true });
  if (await showSidebar.isVisible()) await showSidebar.click();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  const ask = page.getByRole('form', { name: 'Ask' });
  const field = ask.getByRole('textbox', { name: 'Ask anything' });
  await choose(ask.getByRole('combobox', { name: 'Where to ask' }), 'main');
  await expect(field).toHaveAttribute('placeholder', /Enter sends it on its page/);
  await expect(ask.locator('#studio-ask-hint')).toContainText('⌥-Enter only puts it in its box');
  const send = ask.getByRole('button', { name: 'Ask the COO' });
  await expect(send).toHaveAttribute('title', /^The COO, on its page/);
  await field.fill('What slipped this week?');
  await field.press('Enter');
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(page.locator('.work-conversation .turn.user').filter({ hasText: 'What slipped this week?' }).first()).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'COO drawer' })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await expect(field).toHaveValue('');
  await field.fill('Draft the weekly note');
  await field.press('Alt+Enter');
  await expect(page.locator('.work-conversation')).toBeVisible();
  await expect(composer(page)).toHaveValue(/Draft the weekly note/);
  await expect(page.locator('.work-conversation .turn.user').filter({ hasText: 'Draft the weekly note' })).toHaveCount(0);
  await noHorizontalScroll(page);
});
