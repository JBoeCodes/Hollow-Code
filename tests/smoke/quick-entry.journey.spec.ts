import { expect, test, type Page } from './fixtures/test';
import { chooseNew, openGlobalSearch } from './choose';

async function addSheet(page: Page) {
  await chooseNew(page, 'Add a task, note, link, file or recording…');
  return page.getByRole('dialog', { name: 'Add', exact: true });
}

// The quick-capture and Import windows were removed in S5 (2026-11-03): ⌘⌥⇧T
// and ⌘⌥⇧I bring Hollow forward on the Add sheet, set to a task or to a
// Library source. Their draft, failure and late-result rules are Add's,
// tested below. The native key arrives as the `app-menu` event lib.rs emits.
test('Quick capture opens Add set to a task, with the parsed fields the window showed (S5)', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(async url => {
    (await import(url)).api.parseQuickAdd = async (input: string) => ({
      title: input.replace(/ (fri 4pm|for 90m|p1|#Ops|@team|every monday)/g, ''),
      priority: 'P1', due: '2026-11-06', dueTime: '16:00', durationMin: 90, recurrence: 'every monday', project: 'Ops', labels: ['team'],
    });
  }, '/src/api.ts');
  await page.evaluate(async url => { (await import(url)).runMenuAction('quick-capture'); }, '/src/appMenu.ts');
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  await expect(sheet).toBeVisible();
  // The window's syntax line, and its single field with focus.
  await expect(sheet.locator('.add-sheet-why')).toContainText('fri 4pm · for 90m · p1 · #project · @label · every monday');
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  await expect(input).toBeFocused();
  // A line the guess would call a note stays a task: this opening chose it.
  await input.fill('Walk the level 2 gate with the night crew, write up what still sticks after rain and send the photos to Dana fri 4pm for 90m p1 #Ops @team every monday');
  await expect(sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Task', exact: true })).toHaveAttribute('aria-pressed', 'true');
  const preview = sheet.locator('.add-sheet-preview');
  for (const chip of ['due 2026-11-06 16:00', '90m', '↻ every monday', 'P1', '#Ops', '@team']) await expect(preview).toContainText(chip);
  // Enter adds it, as the window's did (S5 review); the hint says so.
  await expect(sheet.locator('.add-sheet-hint')).toHaveText('↩ adds · ⇧↩ new line');
  await input.press('Enter');
  await expect(sheet).toHaveCount(0);
  await expect(page.getByText('Task added to Tasks.', { exact: true }).first()).toBeVisible();
  const made = await page.evaluate(() => (window as unknown as { __mockCreatedTasks?: { title: string; durationMin?: number; recurrence?: string }[] }).__mockCreatedTasks ?? []);
  expect(made.at(-1)).toMatchObject({ durationMin: 90, recurrence: 'every monday' });
  // Opened by the system-wide key, it steps back to where the person was
  // (lib.rs decides where; the browser mock only records the request).
  await expect.poll(() => page.evaluate(() => (window as unknown as { __mockAfterCapture?: boolean[] }).__mockAfterCapture ?? [])).toEqual([true]);
});

test('a summoned sheet never moves Hollow, and any click in it keeps Hollow in front (S5 review)', async ({ page }) => {
  await page.goto('/?mock=1');
  const after = () => page.evaluate(() => (window as unknown as { __mockAfterCapture?: boolean[] }).__mockAfterCapture ?? []);
  const key = (id: string) => page.evaluate(async ({ url, id }) => { (await import(url)).runMenuAction(id); }, { url: '/src/appMenu.ts', id });
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  // A note from the key: no jump to Library; its notice offers Open.
  await key('quick-capture');
  await sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Note', exact: true }).click();
  await input.fill('Gate arm notes\nSticks after rain');
  await sheet.getByRole('button', { name: 'Add note', exact: true }).click();
  await expect(sheet).toHaveCount(0);
  const saved = page.locator('.snackbar').filter({ hasText: 'Note saved to Library.' });
  await expect(saved.getByRole('button', { name: 'Open' })).toBeVisible();
  await expect(page.locator('.library-reader')).toHaveCount(0);
  await expect.poll(after).toEqual([true]);
  // A click in Hollow within the 0.7 s keeps it in front.
  await key('quick-capture');
  await input.fill('Call Dana about the gate');
  await input.press('Enter');
  await expect(sheet).toHaveCount(0);
  await page.locator('body').click({ position: { x: 1, y: 1 } });
  await page.waitForTimeout(1000);
  expect(await after()).toEqual([true]);
});

test('a summoned import steps back only once it succeeds, and a failure brings the draft back (S5 review)', async ({ page }) => {
  await page.goto('/?mock=1');
  const after = () => page.evaluate(() => (window as unknown as { __mockAfterCapture?: boolean[] }).__mockAfterCapture ?? []);
  await page.evaluate(async url => {
    (await import(url)).api.ingestRun = () => new Promise((_resolve, reject) => {
      (window as unknown as { failImport: () => void }).failImport = () => reject('Disk unavailable: fixture');
    });
  }, '/src/api.ts');
  await page.evaluate(async url => { (await import(url)).runMenuAction('quick-ingest'); }, '/src/appMenu.ts');
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  await input.fill('https://example.com/report');
  await input.press('Enter');
  await expect(sheet).toHaveCount(0);
  // Still importing: Hollow stays put.
  await page.waitForTimeout(1000);
  expect(await after()).toEqual([]);
  await page.evaluate(() => (window as unknown as { failImport: () => void }).failImport());
  await expect.poll(after).toEqual([false]);
  await expect(sheet).toBeVisible();
  await expect(input).toHaveValue('https://example.com/report');
});

test('the Add keys reach an open Add sheet: it switches kind and keeps the draft (S5 review)', async ({ page }) => {
  await page.goto('/?mock=1');
  const key = (id: string) => page.evaluate(async ({ url, id }) => { (await import(url)).runMenuAction(id); }, { url: '/src/appMenu.ts', id });
  await key('quick-capture');
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  const kindButton = (name: string) => sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name, exact: true });
  await input.fill('https://example.com/report');
  await expect(kindButton('Task')).toHaveAttribute('aria-pressed', 'true');
  // ⌘⌥⇧I while it is open: no "Close the dialog first", the sheet follows.
  await kindButton('Task').focus();
  await key('quick-ingest');
  await expect(kindButton('Library source')).toHaveAttribute('aria-pressed', 'true');
  await expect(input).toHaveValue('https://example.com/report');
  await expect(input).toBeFocused();
  await expect(page.getByText('Close the dialog first')).toHaveCount(0);
  // A kind picked here holds against the next key.
  await kindButton('Note').click();
  await key('quick-capture');
  await expect(kindButton('Note')).toHaveAttribute('aria-pressed', 'true');
  // A Note keeps Enter for new lines; a single web address imports on Enter.
  await input.press('End'); await input.press('Enter');
  await expect(sheet).toBeVisible();
  await expect(sheet.locator('.add-sheet-hint')).toHaveText('⌘↩ adds');
  await input.fill('https://example.com/report');
  await kindButton('Library source').click();
  await expect(sheet.locator('.add-sheet-hint')).toHaveText('↩ imports · ⇧↩ new line');
  await input.press('Enter');
  await expect(sheet).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem('mockIngestLog') ?? '[]').length)).toBe(1);
});

test('behind another dialog an Add key names what to close (S5 review)', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openGlobalSearch(page);
  await page.evaluate(async url => { (await import(url)).runMenuAction('quick-capture'); }, '/src/appMenu.ts');
  await expect(page.getByText(/Close “Command palette” first, then press the key again/).first()).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Add', exact: true })).toHaveCount(0);
});

test('Add reports a save rejected after Escape and retains the draft (P1-37 review)', async ({ page }) => {
  await page.goto('/?mock=1');
  let sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('Keep my rejected task');
  await page.evaluate(async url => {
    (await import(url)).api.createTask = () => new Promise((_resolve, reject) => {
      (window as unknown as { rejectAdd: () => void }).rejectAdd = () => reject('Disk unavailable: fixture');
    });
  }, '/src/api.ts');
  await sheet.getByRole('button', { name: 'Add task', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as unknown as { rejectAdd?: unknown }).rejectAdd)).toBe('function');
  await sheet.getByRole('textbox', { name: 'What to add' }).press('Escape');
  await expect(sheet).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { rejectAdd: () => void }).rejectAdd());
  await expect(page.getByText("Couldn't add: Disk unavailable: fixture", { exact: true }).first()).toBeVisible();
  await expect(page.locator('.snackbar.error').filter({ hasText: "Couldn't add: Disk unavailable: fixture" })).toHaveAttribute('role', 'alert');
  sheet = await addSheet(page);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('Keep my rejected task');
});

test('Add reports a note saved after Escape without opening it (P1-37 review)', async ({ page }) => {
  await page.goto('/?mock=1');
  const sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('# Late note\n\nKeep this note.');
  await page.evaluate(async url => {
    (await import(url)).api.vaultCreateNote = () => new Promise(resolve => {
      (window as unknown as { finishNote: () => void }).finishNote = () => resolve(null);
    });
  }, '/src/api.ts');
  await sheet.getByRole('button', { name: 'Add note', exact: true }).click();
  await expect.poll(() => page.evaluate(() => typeof (window as unknown as { finishNote?: unknown }).finishNote)).toBe('function');
  await sheet.getByRole('textbox', { name: 'What to add' }).press('Escape');
  await expect(sheet).toHaveCount(0);
  await page.evaluate(() => (window as unknown as { finishNote: () => void }).finishNote());
  await expect(page.getByText('Note saved to Library.', { exact: true }).first()).toBeVisible();
  expect(await page.evaluate(() => window.__mockApiCalls?.vault_read_note ?? 0)).toBe(0);
});

test('Import a source in Add keeps pasted paragraphs and new lines, then imports the complete text (P1-37, S5)', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('button', { name: 'Library actions' }).click();
  await page.getByRole('menuitem', { name: 'Import a source' }).click();
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  const paragraphs = 'First paragraph.\n\nSecond paragraph.\n\nThird paragraph.';
  await input.fill(paragraphs);
  await expect(input).toHaveValue(paragraphs);
  await expect(input).toHaveJSProperty('tagName', 'TEXTAREA');
  // Enter and Shift+Enter add lines; only ⌘↩ (or the button) imports.
  await input.press('End'); await input.press('Shift+Enter'); await input.press('x'); await input.press('Enter'); await input.press('y');
  expect(await page.evaluate(() => window.__mockApiCalls?.ingest_run ?? 0)).toBe(0);
  const submitted = (await input.inputValue()).trim();
  await page.evaluate(async url => {
    const { api } = await import(url);
    const original = api.ingestRun;
    api.ingestRun = (source: unknown, options: unknown) => {
      (window as unknown as { ingested: unknown }).ingested = source;
      return original(source, options);
    };
  }, '/src/api.ts');
  await input.press('Meta+Enter');
  await expect(sheet).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as unknown as { ingested: { content?: string } }).ingested?.content)).toBe(submitted);
  await expect(page.getByText('Imported 1 source note into Library.').first()).toBeVisible();
});

test('Add keeps unsent text through Escape and reload, and Discard clears it (P1-37)', async ({ page }) => {
  await page.goto('/?mock=1');
  let sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('An unsent capture draft');
  await sheet.getByRole('textbox', { name: 'What to add' }).press('Escape');
  await expect(sheet).toHaveCount(0);
  sheet = await addSheet(page);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('An unsent capture draft');
  await page.reload(); sheet = await addSheet(page);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('An unsent capture draft');
  await sheet.getByRole('button', { name: 'Discard draft', exact: true }).click();
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('');
  await sheet.getByRole('button', { name: 'Close', exact: true }).click();
  sheet = await addSheet(page);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('');
});

test('Add shows a rejected picker and preserves the typed text (P1-37)', async ({ page }) => {
  await page.goto('/?mock=1');
  const sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('Keep this during the picker failure');
  await page.evaluate(() => {
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      invoke: async (command: string) => { if (command === 'plugin:dialog|open') throw 'Picker unavailable: fixture'; return null; },
    };
  });
  await sheet.getByRole('button', { name: 'Choose a file…', exact: true }).click();
  await expect(sheet).toContainText('Picker unavailable: fixture');
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('Keep this during the picker failure');
});

test('Add completion preserves a draft typed after reopening (P1-37)', async ({ page }) => {
  await page.goto('/?mock=1');
  let sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('First captured task');
  await page.evaluate(() => {
    const w = window as unknown as { releaseAdd: () => void; heldAdd: { delivered?: boolean } };
    const held = { command: 'create_task', release: new Promise<void>(resolve => { w.releaseAdd = resolve; }) };
    window.__mockHoldOnce = held; w.heldAdd = held;
  });
  await sheet.getByRole('button', { name: 'Add task', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__mockApiCalls?.create_task ?? 0)).toBe(1);
  await sheet.getByRole('textbox', { name: 'What to add' }).press('Escape');
  await expect(sheet).toHaveCount(0);
  sheet = await addSheet(page);
  await sheet.getByRole('textbox', { name: 'What to add' }).fill('Second unsent task');
  await page.evaluate(() => (window as unknown as { releaseAdd: () => void }).releaseAdd());
  await expect.poll(() => page.evaluate(() => (window as unknown as { heldAdd: { delivered?: boolean } }).heldAdd.delivered)).toBe(true);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('Second unsent task');
  await sheet.getByRole('button', { name: 'Add task', exact: true }).click();
  await expect(sheet).toHaveCount(0);
  sheet = await addSheet(page);
  await expect(sheet.getByRole('textbox', { name: 'What to add' })).toHaveValue('');
});
