import { expect, test, type Page } from './fixtures/test';
import { backToAll, openLibraryView } from './libraryView';

/**
 * P1-19 in the browser mock: Library's tabs keep their state honest. By
 * meaning stays inside the chosen collection, Library → Results picks up a
 * new result by itself, File history opens from every tab, and the New note
 * form belongs to Notes.
 */
const DIR = 'workspaces/local-q3-review/outputs';
const tab = (page: Page, name: string) => page.locator('.library-tabs').getByRole('button', { name, exact: true });

/** Stand-in for Tauri's event bridge (as in turn-footer.spec.ts), so a
 *  test can deliver `vault-changed` to the app's own listeners. */
async function installEventBridge(page: Page) {
  await page.addInitScript(() => {
    const callbacks = new Map<number, (event: unknown) => void>();
    const listeners = new Map<string, number[]>();
    let next = 1;
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: unknown) => void) { const id = next++; callbacks.set(id, callback); return id; },
      invoke(command: string, args: { event?: string; handler?: number }) {
        if (command === 'plugin:event|listen' && args?.event && args.handler) {
          listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]);
          return Promise.resolve(next++);
        }
        if (command === 'plugin:event|unlisten') return Promise.resolve();
        return Promise.reject(new Error(`not in Tauri: ${command}`));
      },
    };
    w.__emitTauriEvent = (event: string, payload: unknown) => { for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, id, payload }); };
  });
}

async function library(page: Page, init?: () => void) {
  if (init) await page.addInitScript(init);
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
}

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop Library layout'); });

test('By meaning stays inside the chosen collection and says so', async ({ page }) => {
  await library(page, () => localStorage.setItem('semanticSearchEnabled', 'true'));
  await openLibraryView(page, 'Notes');
  // The on-device index answers with notes from several collections.
  await page.evaluate(async () => {
    const path = '/src/api.ts', { api } = await import(/* @vite-ignore */ path);
    const hit = (p: string) => ({ path: p, basename: p.split('/').pop()!.replace(/\.md$/, ''), title: null, modified: null, summary: null, tags: [], snippet: `About ${p}`, score: 0.9, kind: 'note' });
    api.semanticSearch = async () => ['notes/ana-ruiz.md', 'library/ana-notes.md', 'notes/team/bo-lee.md', 'workspaces/local-q3-review/outputs/ana.md'].map(hit);
  });
  // A plain folder: the People collection this used was cut (overnight 11-08 S3).
  await page.getByRole('combobox', { name: 'Library folder' }).click();
  await page.getByRole('option', { name: 'notes/', exact: true }).click();
  await page.getByRole('button', { name: 'By meaning', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Find a file' }).fill('who handles leases');
  const list = page.locator('.library-results .library-item');
  await expect(page.locator('.library-count')).toHaveText('2 in notes/ by meaning');
  await expect(list).toHaveCount(2);
  for (const title of await list.locator('strong').allTextContents()) expect(['ana-ruiz', 'bo-lee']).toContain(title);
  // Saved with By meaning on; the chip brings it back after a reload.
  await page.getByRole('button', { name: 'Save this view' }).click();
  await page.reload();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await openLibraryView(page, 'Notes');
  await page.getByRole('group', { name: 'Saved views' }).locator('.library-view-chip button').first().click();
  await expect(page.getByRole('button', { name: 'By meaning', exact: true })).toHaveAttribute('aria-pressed', 'true');
});

test('a new result appears in Library → Results without a reload, and Refresh reads again', async ({ page }) => {
  await installEventBridge(page);
  await library(page);
  await openLibraryView(page, 'Results');
  const rows = page.locator('.library-result-row');
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  await page.evaluate(path => {
    const w = window as unknown as { __mockOutsideWrite: (p: string, c: string, m?: string) => void; __emitTauriEvent: (n: string, x: unknown) => void };
    w.__mockOutsideWrite(path, '# Lane overtime appendix\n\nWeekly hours.\n', new Date().toISOString());
    w.__emitTauriEvent('vault-changed', { folders: ['workspaces'], paths: [path] });
  }, `${DIR}/lane-overtime-appendix.md`);
  await expect(rows.filter({ hasText: 'Lane overtime appendix' })).toBeVisible({ timeout: 2_000 });
  await expect(rows).toHaveCount(before + 1);
  // A change the watcher never reported: Refresh reads the list again.
  await page.evaluate(path => {
    (window as unknown as { __mockOutsideWrite: (p: string, c: string, m?: string) => void }).__mockOutsideWrite(path, '# Quiet appendix\n', new Date().toISOString());
  }, `${DIR}/quiet-appendix.md`);
  await page.getByRole('button', { name: 'Refresh results' }).click();
  await expect(rows.filter({ hasText: 'Quiet appendix' })).toBeVisible();
});

test('Show file history opens from every tab, in Notes, and its label flips to Hide', async ({ page }) => {
  await library(page);
  for (const name of ['Notes', 'Results', 'Recordings', 'Connected projects']) {
    if (name === 'Connected projects') await tab(page, name).click();
    else await openLibraryView(page, name as 'Notes' | 'Results' | 'Recordings');
    const remembered = await page.evaluate(() => localStorage.getItem('hollow.library.mode'));
    await page.getByRole('button', { name: 'Library actions' }).click();
    await page.getByRole('menuitem', { name: 'Show file history' }).click();
    await expect(backToAll(page)).toBeVisible();
    await expect(page.locator('.library-reader .history-recovery')).toBeVisible();
    expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe(remembered);
    await page.getByRole('button', { name: 'Library actions' }).click();
    await page.getByRole('menuitem', { name: 'Hide file history' }).click();
    await expect(page.locator('.history-recovery')).toHaveCount(0);
  }
});

// Overnight 11-12 L2: Settings' skills catalog and History & recovery are
// Library tabs too, remembered like the others, and the palette opens them.
test('Skills and History are Library tabs with the skills catalog and the vault’s history', async ({ page }) => {
  await library(page);
  await expect(page.locator('.library-tabs > button')).toHaveText(['All', 'Connected projects', 'Conversations', 'Skills', 'History', /^Learnings/]);
  await tab(page, 'Skills').click();
  await expect(page.getByRole('searchbox', { name: 'Find a skill' })).toBeVisible();
  await expect(page.getByRole('list', { name: 'Skills' }).getByText('release-notes').first()).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('skills');
  await tab(page, 'History').click();
  const runs = page.getByRole('listbox', { name: 'Automated-write runs' });
  await expect(runs.locator('.history-run-row')).toHaveCount(2);
  // The whole vault's runs, not one note's file history.
  await expect(page.locator('.library-reader')).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('history');
  // The palette opens either tab from another page.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Tasks', exact: true }).click();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Library: Skills');
  await page.getByRole('option', { name: /^Library: Skills/ }).click();
  await expect(tab(page, 'Skills')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('searchbox', { name: 'Find a skill' })).toBeVisible();
});

test('New note focuses its field and belongs to Notes; the folder sits under the search', async ({ page }) => {
  await library(page);
  await openLibraryView(page, 'Results');
  await page.getByRole('button', { name: 'New note' }).click();
  await expect(page.getByRole('complementary', { name: 'Library files' })).toBeVisible();
  await expect(page.getByLabel('Note name')).toBeFocused();
  await openLibraryView(page, 'Recordings');
  await expect(page.getByLabel('Note name')).toHaveCount(0);
  await openLibraryView(page, 'Notes');
  await expect(page.getByLabel('Note name')).toHaveCount(0);
  // Browse state: the folder select stacks under the search field.
  const search = await page.getByRole('searchbox', { name: 'Find a file' }).boundingBox();
  const folder = await page.getByRole('combobox', { name: 'Library folder' }).boundingBox();
  expect(folder!.y).toBeGreaterThanOrEqual(search!.y + search!.height);
  // The two collections that hold a tab's files are named apart from it and link to it.
  await page.getByRole('combobox', { name: 'Library folder' }).click();
  await page.getByRole('option', { name: 'Kept transcripts', exact: true }).click();
  await page.getByRole('button', { name: 'Open Recordings', exact: true }).click();
  await expect(page.getByRole('complementary', { name: 'Recordings', exact: true })).toBeVisible();
  await expect(backToAll(page)).toBeVisible();
});

// Overnight 11-12 L4: Notes, Results and Recordings left the strip. Each opens
// from All, keeps All selected, shows Back to All, and remembers All.
test('Notes, Results and Recordings open from All and come back to it', async ({ page }) => {
  await library(page, () => localStorage.setItem('hollow.library.mode', 'results'));
  // A Results remembered before L4 opens All.
  await expect(tab(page, 'All')).toHaveAttribute('aria-pressed', 'true');
  for (const name of ['Notes', 'Results', 'Recordings']) await expect(tab(page, name)).toHaveCount(0);
  const all = page.getByRole('region', { name: 'All', exact: true });
  const chip = (name: string) => all.getByRole('group', { name: 'Library filter' }).getByRole('button', { name, exact: true });
  // Grid shows only with Outputs, Recordings only with Meetings.
  await expect(all.getByRole('button', { name: 'Grid', exact: true })).toHaveCount(0);
  await expect(all.getByRole('button', { name: 'Recordings', exact: true })).toHaveCount(0);

  await all.getByRole('button', { name: 'Browse folders', exact: true }).click();
  await expect(page.getByRole('combobox', { name: 'Library folder' })).toBeVisible();
  await expect(tab(page, 'All')).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('all');
  await backToAll(page).click();
  await expect(all).toBeVisible();

  await chip('Outputs').click();
  await all.getByRole('button', { name: 'Grid', exact: true }).click();
  await expect(page.getByRole('group', { name: 'Result type' })).toBeVisible();
  await expect(page.locator('.library-results-count')).toHaveText(/^\d+ of \d+/);
  // Back to All returns to the filter the view was opened from.
  await backToAll(page).click();
  await expect(chip('Outputs')).toHaveAttribute('aria-pressed', 'true');

  await chip('Meetings').click();
  await all.getByRole('button', { name: 'Recordings', exact: true }).click();
  await expect(page.getByRole('listbox', { name: 'Recordings' })).toBeVisible();
  await backToAll(page).click();
  await expect(chip('Meetings')).toHaveAttribute('aria-pressed', 'true');

  // The palette still opens each view.
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Library: Results');
  await page.getByRole('option', { name: /^Library: Results/ }).click();
  await expect(page.getByRole('group', { name: 'Result type' })).toBeVisible();
  await backToAll(page).click();

  // New note sits beside the search and opens the form in Notes.
  await all.getByRole('button', { name: 'New note', exact: true }).click();
  await expect(page.getByLabel('Note name')).toBeFocused();
  await expect(backToAll(page)).toBeVisible();
});

// #1337: every Library view change goes through one path (showLibraryView).
test('palette Library: Skills from Conversations, then Back, returns to Conversations', async ({ page }) => {
  await library(page);
  await tab(page, 'Conversations').click();
  await expect(tab(page, 'Conversations')).toHaveAttribute('aria-pressed', 'true');
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Library: Skills');
  await page.getByRole('option', { name: /^Library: Skills/ }).click();
  await expect(tab(page, 'Skills')).toHaveAttribute('aria-pressed', 'true');
  await page.locator('body').click({ position: { x: 1, y: 1 } });
  await page.keyboard.press('Meta+[');
  await expect(tab(page, 'Conversations')).toHaveAttribute('aria-pressed', 'true');
});

test('a New note form or file history left by the palette does not come back with the tab', async ({ page }) => {
  await library(page);
  await openLibraryView(page, 'Notes');
  await page.getByRole('button', { name: 'Library actions' }).click();
  await page.getByRole('menuitem', { name: 'Show file history' }).click();
  await expect(page.locator('.library-reader .history-recovery')).toBeVisible();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Library: Skills');
  await page.getByRole('option', { name: /^Library: Skills/ }).click();
  await expect(tab(page, 'Skills')).toHaveAttribute('aria-pressed', 'true');
  await tab(page, 'All').click();
  await expect(page.locator('.history-recovery')).toHaveCount(0);
  // Same for the New note form.
  await page.getByRole('region', { name: 'All', exact: true }).getByRole('button', { name: 'New note', exact: true }).click();
  await expect(page.getByLabel('Note name')).toBeVisible();
  await page.keyboard.press('Meta+k');
  await page.locator('.palette-input').fill('Library: Skills');
  await page.getByRole('option', { name: /^Library: Skills/ }).click();
  await tab(page, 'All').click();
  await expect(page.getByLabel('Note name')).toHaveCount(0);
});

test('Settings “Open Library, History” opens the History tab', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: /^Advanced/ }).click();
  await page.locator('#settings-group-this-mac > summary').click();
  await page.getByRole('button', { name: 'Open Library, History', exact: true }).click();
  await expect(tab(page, 'History')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('listbox', { name: 'Automated-write runs' })).toBeVisible();
});
