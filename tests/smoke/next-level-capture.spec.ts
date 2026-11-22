import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

/** Next-level P12: one Add entry routes text predictably, shows where it goes
 *  before saving, and keeps the input when a save fails. Browser mock. */
async function openAdd(page: Page) {
  await chooseNew(page, 'Add a task, note, link, file or recording…');
  const sheet = page.getByRole('dialog', { name: 'Add' });
  await expect(sheet).toBeVisible();
  return sheet;
}
test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the rail Add button is desktop-only');
  await page.goto('/?mock=1&demo=1');
});

test('a short dated line becomes a task, and the destination is shown first', async ({ page }) => {
  const sheet = await openAdd(page);
  await sheet.getByLabel('What to add').fill('Call Dana tomorrow about the budget');
  await expect(sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Task' })).toHaveAttribute('aria-pressed', 'true');
  await expect(sheet.locator('.add-sheet-dest')).toContainText('Tasks');
  await sheet.getByRole('button', { name: 'Add task' }).click();
  await expect(sheet).toHaveCount(0);
  const made = await page.evaluate(() => (window as unknown as { __mockCreatedTasks?: { title: string }[] }).__mockCreatedTasks?.map(t => t.title) ?? []);
  expect(made.length).toBe(1);
});

test('several lines become a note in Library; a switch overrides the guess', async ({ page }) => {
  const sheet = await openAdd(page);
  await sheet.getByLabel('What to add').fill('Site visit Garage 14\nGate arm sticks after rain\nTwo lights out on level 2');
  await expect(sheet.locator('.add-sheet-dest')).toContainText('library/Site visit Garage 14.md');
  await sheet.getByRole('button', { name: 'Add note' }).click();
  await expect(page.locator('.library-reader')).toContainText('Gate arm sticks after rain');
});

test('a link that fails to import comes back as the draft; a Plaud link needs a workspace', async ({ page }) => {
  await page.evaluate(() => localStorage.setItem('mockIngestFail', '1'));
  let sheet = await openAdd(page);
  await sheet.getByLabel('What to add').fill('https://example.com/garage-14-report');
  // A web address is a Library source (S5: the old Web source kind).
  await expect(sheet.locator('.add-sheet-why')).toContainText('library source');
  await expect(sheet.getByRole('textbox', { name: 'Domain (optional)' })).toBeVisible();
  // An import runs in the Activity dock, so the sheet closes at once; a
  // failure says so and puts the link back as the draft.
  await sheet.getByRole('button', { name: 'Import into Library' }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.locator('.snackbar.error').filter({ hasText: 'Import failed' }).first()).toContainText('Your text is back in Add.');
  sheet = await openAdd(page);
  await expect(sheet.getByLabel('What to add')).toHaveValue('https://example.com/garage-14-report');
  await page.evaluate(() => localStorage.removeItem('mockIngestFail'));
  await sheet.getByRole('button', { name: 'Import into Library' }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByText('Imported 1 source note into Library.').first()).toBeVisible();
  sheet = await openAdd(page);
  await expect(sheet.getByLabel('What to add')).toHaveValue('');
  await sheet.getByLabel('What to add').fill('https://web.plaud.ai/file/rec0a1b2c3d4e5f6');
  await expect(sheet.getByRole('button', { name: 'Add recording link' })).toBeDisabled();
  await sheet.getByRole('combobox', { name: 'Workspace' }).click();
  await page.getByRole('option', { name: 'Q4 hiring plan' }).click();
  await sheet.getByRole('button', { name: 'Add recording link' }).click();
  await expect(sheet).toHaveCount(0);
  const links = await page.evaluate(() => JSON.parse(localStorage.getItem('mockRecordingLinks') ?? '{}'));
  expect(links.rec0a1b2c3d4e5f6.workspaceId).toBe('local-hiring-plan');
});

/** S5: Import a source is the Add sheet set to a Library source; the separate
 *  Import source sheet and window are gone. */
test('Import a source opens Add set to a Library source, and its Domain and Purpose reach the import', async ({ page }) => {
  await page.evaluate(async url => {
    const { api } = await import(url);
    const original = api.ingestRun;
    api.ingestRun = (source: unknown, opts: unknown) => {
      (window as unknown as { ingested: unknown[] }).ingested = [source, opts];
      return original(source, opts);
    };
  }, '/src/api.ts');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'Library actions' }).click();
  await page.getByRole('menuitem', { name: 'Import a source' }).click();
  const sheet = page.getByRole('dialog', { name: 'Add' });
  await expect(sheet).toBeVisible();
  await expect(sheet.getByRole('heading', { name: 'Import source' })).toHaveCount(0);
  // Text that would be guessed a note stays a Library source: this opening chose it.
  await sheet.getByLabel('What to add').fill('Gate arm notes\nSticks after rain\nTwo lights out on level 2');
  await expect(sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Library source' })).toHaveAttribute('aria-pressed', 'true');
  await expect(sheet.locator('.add-sheet-dest')).toContainText('Library · source notes from the pasted text');
  await sheet.getByRole('textbox', { name: 'Domain (optional)' }).fill('Garage operations');
  await sheet.getByRole('textbox', { name: 'Purpose (optional)' }).fill('Fix the gate before winter');
  await sheet.getByRole('button', { name: 'Import into Library' }).click();
  await expect(sheet).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as unknown as { ingested?: unknown[] }).ingested)).toEqual([
    { kind: 'text', content: 'Gate arm notes\nSticks after rain\nTwo lights out on level 2', title: 'Gate arm notes' },
    { domain: 'Garage operations', purpose: 'Fix the gate before winter' },
  ]);
  await expect(page.getByText('Imported 1 source note into Library.').first()).toBeVisible();
  // The other kinds are one click away in the same sheet.
  await page.getByRole('button', { name: 'Library actions' }).click();
  await page.getByRole('menuitem', { name: 'Import a source' }).click();
  await sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Note' }).click();
  await expect(sheet.getByRole('textbox', { name: 'Domain (optional)' })).toHaveCount(0);
  await expect(sheet.getByRole('button', { name: 'Add note' })).toBeVisible();
});
