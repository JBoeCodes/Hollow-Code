// Import a source runs in the Activity dock after the Add sheet closes (S5,
// 2026-11-03; this spec first covered the separate Import window, second-pass
// review of #636). Its late outcome must not touch what is typed next: a late
// success leaves a new draft alone, and a late failure puts its own text back
// only when nothing new was typed.
import { expect, test, type Page } from './fixtures/test';
import { chooseNew } from './choose';

type Held = { command: string; release: Promise<unknown>; delivered?: boolean };
type W = { __mockHoldOnce?: Held; __held?: Held; __releaseHeld?: () => void; __mockApiCalls?: Record<string, number> };

async function importSource(page: Page) {
  await page.getByRole('button', { name: 'Library actions' }).click();
  await page.getByRole('menuitem', { name: 'Import a source' }).click();
  const sheet = page.getByRole('dialog', { name: 'Add', exact: true });
  await expect(sheet).toBeVisible();
  return sheet;
}

async function holdNextImport(page: Page) {
  await page.evaluate(() => {
    const w = window as unknown as W;
    let release!: () => void;
    const held: Held = { command: 'ingest_run', release: new Promise<void>(r => { release = r; }) };
    w.__mockHoldOnce = held; w.__held = held; w.__releaseHeld = release;
  });
}

test.beforeEach(async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
});

test('an import finishing after the sheet is reopened leaves the new line alone', async ({ page }) => {
  await holdNextImport(page);
  let sheet = await importSource(page);
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  await input.fill('https://example.com/a');
  await sheet.getByRole('button', { name: 'Import into Library', exact: true }).click();
  // The sheet closes at once; the import is still going.
  await expect(sheet).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as unknown as W).__mockApiCalls?.ingest_run ?? 0)).toBe(1);

  // Open it again: it reads empty, and what is typed now belongs to the person.
  sheet = await importSource(page);
  await expect(input).toHaveValue('');
  await input.fill('second');

  // The first import finishes: it says so, and neither closes the sheet nor clears the line.
  await page.evaluate(() => (window as unknown as W).__releaseHeld?.());
  await expect.poll(() => page.evaluate(() => (window as unknown as W).__held?.delivered ?? false)).toBe(true);
  await expect(page.getByText('Imported 1 source note into Library.').first()).toBeVisible();
  await expect(sheet).toBeVisible();
  await expect(input).toHaveValue('second');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('mockIngestLog') ?? '[]').length)).toBe(1);
});

test('a late failed import puts its text back only when nothing new was typed', async ({ page }) => {
  // The mock's hold delays an answer, not a throw, so the failure is held here.
  const failLater = () => page.evaluate(async url => {
    (await import(url)).api.ingestRun = () => new Promise((_resolve, reject) => {
      (window as unknown as { failImport: () => void }).failImport = () => reject('Disk unavailable: fixture');
    });
  }, '/src/api.ts');
  const failNow = () => page.evaluate(() => (window as unknown as { failImport: () => void }).failImport());
  const failure = page.locator('.snackbar.error').filter({ hasText: 'Import failed' });

  // Something new was typed: the failure says so and leaves the new line alone.
  await failLater();
  let sheet = await importSource(page);
  const input = sheet.getByRole('textbox', { name: 'What to add' });
  await input.fill('https://example.com/b');
  await sheet.getByRole('button', { name: 'Import into Library', exact: true }).click();
  await expect(sheet).toHaveCount(0);
  sheet = await importSource(page);
  await input.fill('a newer line');
  await failNow();
  await expect(failure.first()).toBeVisible();
  await expect(failure.first()).not.toContainText('Your text is back in Add.');
  await expect(input).toHaveValue('a newer line');

  // Nothing new: its own text comes back as the draft, with its kind, Domain
  // and Purpose (S5 review), even opened from New → Add.
  await sheet.getByRole('button', { name: 'Discard draft', exact: true }).click();
  await input.fill('Gate arm notes\nSticks after rain');
  await sheet.getByRole('textbox', { name: 'Domain (optional)' }).fill('Garage operations');
  await sheet.getByRole('textbox', { name: 'Purpose (optional)' }).fill('Fix the gate');
  await failLater();
  await sheet.getByRole('button', { name: 'Import into Library', exact: true }).click();
  await expect(sheet).toHaveCount(0);
  await failNow();
  await expect(failure.filter({ hasText: 'Your text is back in Add.' }).first()).toBeVisible();
  await chooseNew(page, 'Add a task, note, link, file or recording…');
  await expect(sheet).toBeVisible();
  await expect(input).toHaveValue('Gate arm notes\nSticks after rain');
  await expect(sheet.getByRole('group', { name: 'Add as' }).getByRole('button', { name: 'Library source', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(sheet.getByRole('textbox', { name: 'Domain (optional)' })).toHaveValue('Garage operations');
  await expect(sheet.getByRole('textbox', { name: 'Purpose (optional)' })).toHaveValue('Fix the gate');
});
