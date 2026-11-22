import { expect, test, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

/**
 * P1-18: Library notes can be renamed, moved and trashed from File actions
 * and from a row's ⋯, with Undo for the trash; snapshots offer none of it.
 * The vault is the browser mock's (src/mockApi.ts: vault_move_note,
 * vault_delete_note).
 */
async function newNote(page: Page, name: string) {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'New note' }).click();
  await page.getByLabel('Note name').fill(name);
  await page.getByRole('button', { name: 'Create note' }).click();
  await expect(page.getByRole('region', { name: 'File reader' }).locator('.work-file-name')).toHaveText(`${name}.md`);
}
const reader = (page: Page) => page.getByRole('region', { name: 'File reader' });
const fileAction = async (page: Page, item: string) => {
  await reader(page).getByRole('button', { name: 'File actions' }).click();
  await reader(page).getByRole('menuitem', { name: item }).click();
};

test('rename updates the list and the reader', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'at 390 px the Library list overlays the open reader, covering its File actions menu and editor');
  await newNote(page, 'Plan draft');
  await fileAction(page, 'Rename…');
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('Launch plan');
  await dialog.getByRole('button', { name: 'Rename' }).click();
  await expect(reader(page).locator('.work-file-name')).toHaveText('Launch plan.md');
  const list = page.getByRole('complementary', { name: 'Library files' });
  await expect(list.locator('.library-item[title="library/Launch plan.md"]')).toHaveAttribute('aria-current', 'true');
  await expect(list.locator('.library-item[title="library/Plan draft.md"]')).toHaveCount(0);
});

test('a row’s ⋯ moves a note to a folder and the reader follows it', async ({ page }) => {
  await newNote(page, 'Vendor notes');
  const list = page.getByRole('complementary', { name: 'Library files' });
  await list.getByRole('button', { name: 'More actions for Vendor notes' }).click();
  await list.getByRole('menuitem', { name: 'Move to folder…' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByRole('textbox').fill('library/vendors');
  await dialog.getByRole('button', { name: 'Move' }).click();
  await expect(list.locator('.library-item[title="library/vendors/Vendor notes.md"]')).toHaveCount(1);
  await expect(list.locator('.library-item[title="library/Vendor notes.md"]')).toHaveCount(0);
  await expect(reader(page).locator('.work-file-name')).toHaveText('Vendor notes.md');
});

test('trash closes the reader and Undo restores the note', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'at 390 px the Library list overlays the open reader, covering its File actions menu and editor');
  await newNote(page, 'Old memo');
  await fileAction(page, 'Move to trash');
  await page.getByRole('dialog').getByRole('button', { name: 'Move to trash' }).click();
  // The reader closes: Library goes back to its full-width list.
  await expect(page.locator('.library-page')).not.toHaveClass(/library-has-file/);
  const list = page.getByRole('complementary', { name: 'Library files' });
  await expect(list.locator('.library-item[title="library/Old memo.md"]')).toHaveCount(0);
  await page.getByRole('button', { name: 'Undo' }).first().click();
  await expect(list.locator('.library-item[title="library/Old memo.md"]')).toHaveCount(1);
  await expect(reader(page).locator('.work-file-name')).toHaveText('Old memo.md');
});

test('a note with an unsaved draft is not renamed, even when it isn’t open', async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'at 390 px the Library list overlays the open reader, covering its File actions menu and editor');
  await newNote(page, 'Half written');
  await reader(page).getByRole('button', { name: 'Edit', exact: true }).click();
  const editor = reader(page).locator('.cm-content[contenteditable=true]');
  await editor.click();
  await page.keyboard.type(' more words');
  await expect(reader(page).locator('.work-file-state')).toHaveText('Unsaved draft');
  await reader(page).getByRole('button', { name: /All files/ }).click();
  const list = page.getByRole('complementary', { name: 'Library files' });
  await list.getByRole('button', { name: 'More actions for Half written' }).click();
  await list.getByRole('menuitem', { name: 'Rename…' }).click();
  await expect(page.getByText('“Half written” has an unsaved draft.', { exact: false }).first()).toBeVisible();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(list.locator('.library-item[title="library/Half written.md"]')).toHaveCount(1);
});

test('a snapshot offers none of the items', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockVaultNotes', JSON.stringify([
    { path: 'workspaces/q3/sources/policy.md', basename: 'policy', title: 'Policy snapshot' },
  ])));
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Notes');
  await page.getByRole('button', { name: 'Browse all files' }).click();
  const list = page.getByRole('complementary', { name: 'Library files' });
  await expect(list.getByRole('button', { name: /^Policy snapshot/ })).toBeVisible();
  await expect(list.getByRole('button', { name: 'More actions for Policy snapshot' })).toHaveCount(0);
  await list.getByRole('button', { name: /^Policy snapshot/ }).click();
  await reader(page).getByRole('button', { name: 'File actions' }).click();
  const menu = reader(page).getByRole('menu');
  await expect(menu.getByRole('menuitem', { name: 'Show file in Finder' })).toBeVisible();
  for (const item of ['Rename…', 'Move to folder…', 'Move to trash']) await expect(menu.getByRole('menuitem', { name: item })).toHaveCount(0);
});
