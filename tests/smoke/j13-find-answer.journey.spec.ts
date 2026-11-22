/**
 * Journey J13 (plan completion L5, UI review journey 5 "Find an answer in
 * existing work", matrix V7j; One Ask, 2026-11-03): with visible controls
 * only, ask from Work's Ask anything (New chat: the question is the COO's
 * next message since L8, 11-10), then keep an answer: a reply's More actions → Save
 * as note writes it into the workspace's Results, a second save gets the next
 * name instead of overwriting the first, and the toast's Open shows the note
 * in the Preview pane. Until One Ask this journey used the Ask your vault
 * view, which was removed (every question is a chat). A required spec:
 * desktop, native minimum window and Largest text; no skips
 * (scripts/check-required-specs.mjs).
 */
import { expect, test } from './fixtures/test';
import { choose } from './choose';

test('click-only: ask from Work, then save a reply as a note and open it', async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto('/?mock=1&demo=1');
  const form = page.getByRole('form', { name: 'Ask', exact: true });
  await form.getByRole('textbox', { name: 'Ask anything' }).fill('Who owns the Q3 floor price?');
  await choose(form.getByRole('combobox', { name: 'Where to ask' }), 'quick');
  await form.getByRole('button', { name: 'Start the chat', exact: true }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockLastChatPrompt') ?? '')).toContain('Who owns the Q3 floor price?');
  await expect(page.locator('.chat-main')).toContainText('Who owns the Q3 floor price?');

  // A workspace conversation's reply saves into that workspace's Results.
  // At the minimum window the sidebar starts hidden behind its button.
  const showSidebar = page.getByRole('button', { name: 'Show sidebar', exact: true });
  if (await showSidebar.isVisible()) await showSidebar.click();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click();
  await page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  const reply = page.getByRole('group', { name: 'Reply details' }).last();
  const save = async () => {
    await reply.hover();
    await reply.getByRole('button', { name: 'More actions' }).click();
    await page.locator('.ctx-menu').getByRole('menuitem', { name: 'Save as note' }).click();
  };
  await save();
  const saved = page.locator('.snackbar').filter({ hasText: 'Saved as a note: workspaces/' });
  await expect(saved.first()).toContainText(/\/outputs\/answers\/\d{4}-\d\d-\d\d-[a-z0-9-]+\.md/);

  // Saved again, it never overwrites: the next name.
  await save();
  const second = page.locator('.snackbar').filter({ hasText: /-2\.md/ });
  await expect(second).toBeVisible();

  // Open shows the saved note in the Preview pane.
  await second.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Preview', exact: true })).toBeVisible();
});
