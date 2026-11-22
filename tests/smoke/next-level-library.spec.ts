import { expect, test, type Page } from './fixtures/test';
import { openLibraryView } from './libraryView';

/**
 * Next-level P06/P11 in the browser mock: Library views (Results across
 * workspaces, saved views, note templates) and the Recordings reader
 * (unavailable parts stated, explicit links, opt-in copy, idempotent task
 * proposals). Plaud and the vault are fixtures (src/mockRecordings.ts).
 * Also (repo features R14a) a connected project's PDF, read by Hollow.
 */
async function library(page: Page, view: string, init?: () => void) {
  if (init) await page.addInitScript(init);
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  // Notes, Results and Recordings open from All (overnight 11-12 L4).
  if (view === 'Notes' || view === 'Results' || view === 'Recordings') await openLibraryView(page, view);
  else await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: view, exact: true }).click();
}
test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop Library layout'); });

test('Results keeps its toolbar during first load and its cards during refresh', async ({ page }) => {
  await library(page, 'Notes');
  const holdListing = async () => page.evaluate(() => {
    let release!: () => void;
    window.__mockHoldOnce = { command: 'list_all_results', release: new Promise<void>(resolve => { release = resolve; }) };
    (window as unknown as { releaseResults: () => void }).releaseResults = release;
  });
  const releaseListing = async () => page.evaluate(() => (window as unknown as { releaseResults: () => void }).releaseResults());
  // All's Outputs filter reads list_all_results_peek, a different command, so
  // the hold on list_all_results (the Grid's listing) only catches Grid opening Results.
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'All', exact: true }).click();
  const all = page.getByRole('region', { name: 'All', exact: true });
  await all.getByRole('group', { name: 'Library filter' }).getByRole('button', { name: 'Outputs', exact: true }).click();
  await expect(all.getByRole('button', { name: 'Grid', exact: true })).toBeVisible();
  await expect(all.locator('.ui-skeleton, [aria-busy="true"]')).toHaveCount(0);
  await holdListing();
  await all.getByRole('button', { name: 'Grid', exact: true }).click();
  const loading = page.getByRole('status', { name: 'Loading results', exact: true });
  await expect(loading).toBeVisible();
  await expect(loading.locator('.ui-skeleton-bar')).toHaveCount(4);
  await expect(loading.locator('button, input, a, iframe, img, canvas, [tabindex]')).toHaveCount(0);
  const search = page.getByRole('searchbox', { name: 'Find a result' });
  await search.fill('Q3');
  await expect(search).toBeFocused();
  await releaseListing();
  await expect(loading).toHaveCount(0);
  const row = page.locator('button.library-result-row').first();
  await expect(row).toBeVisible();
  await expect(search).toHaveValue('Q3');
  await expect(search).toBeFocused();
  const original = await row.elementHandle();
  await holdListing();
  await row.focus();
  // Start the refresh without moving focus to its button, as a watcher would.
  await page.getByRole('button', { name: 'Refresh results', exact: true }).evaluate(button => button.click());
  await expect(row).toBeFocused();
  await expect(loading).toHaveCount(0);
  expect(await original!.evaluate(node => node.isConnected)).toBe(true);
  await expect(page.getByRole('button', { name: 'Refresh results', exact: true })).toBeDisabled();
  await releaseListing();
  await expect(page.getByRole('button', { name: 'Refresh results', exact: true })).toBeEnabled();
  await expect(loading).toHaveCount(0);
  expect(await original!.evaluate(node => node.isConnected)).toBe(true);
  await expect(row).toBeFocused();
});

test('Results previews a deliverable and keeps a direct path to its workspace', async ({ page }) => {
  await library(page, 'Results');
  const row = page.locator('.library-result-row').filter({ hasText: 'Q3 regional operations review' }).first();
  await expect(row).toContainText('Draft');
  await expect(page.locator('.library-results-count')).toContainText(/of [2-9]/);
  await row.click();
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  await expect(pane).toBeVisible();
  // A Result opens under its review bar (COO rebuild L4).
  await expect(pane.getByRole('region', { name: 'Result review', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close preview', exact: true }).click();
  await page.getByRole('button', { name: 'Open Q3 regional operations review in its workspace', exact: true }).first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');
  await expect(pane.getByRole('tab', { name: 'q3-regional-review.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  // Whiteboards were removed (2026-10-07): Library has no Boards view.
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Boards', exact: true })).toHaveCount(0);
});

test('a collection can be pinned as a saved view, and a note starts from a template', async ({ page }) => {
  await library(page, 'Notes');
  await page.getByRole('button', { name: 'Pin this collection' }).click();
  await expect(page.getByRole('group', { name: 'Saved views' }).locator('.library-view-chip')).toHaveCount(1);
  await page.reload();
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  // Notes is All's view (L4): Library reopens on All, and Browse folders keeps the pin.
  await expect(page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await openLibraryView(page, 'Notes');
  await expect(page.getByRole('group', { name: 'Saved views' }).locator('.library-view-chip')).toHaveCount(1);
  await page.getByRole('button', { name: 'New note' }).click();
  await page.getByLabel('Note name').fill('Ops sync 1 Oct');
  await page.getByRole('combobox', { name: 'Note template' }).click();
  await page.getByRole('option', { name: 'Meeting notes' }).click();
  await page.getByRole('button', { name: 'Create note' }).click();
  await expect(page.locator('.library-reader')).toContainText('Action items');
});

test('the recording reader states what is missing, links explicitly and creates tasks once', async ({ page }) => {
  await library(page, 'Recordings');
  await expect(page.locator('.recordings-source')).toContainText('Plaud · read only');
  // A recording with neither transcript nor summary says so.
  await page.getByRole('option', { name: /Board prep/ }).click();
  await expect(page.locator('.recording-reader')).toContainText('Plaud has no transcript for this recording.');
  await expect(page.locator('.recording-reader')).toContainText('Plaud has no summary for this recording.');
  await page.getByRole('option', { name: /Weekly ops sync/ }).click();
  const side = page.getByRole('complementary', { name: 'Proposed tasks' });
  await expect(side.locator('li')).toHaveCount(2);
  await expect(side).toContainText('Owner: Sam');
  await expect(side).toContainText('Owner unknown');
  await expect(side).toContainText('at 01:02');
  await side.getByRole('checkbox').first().check();
  await side.getByRole('checkbox').nth(1).check();
  await side.getByRole('button', { name: /Create 2 tasks/ }).click();
  await expect(side).toContainText('2 created');
  // Retrying makes no twins.
  await side.getByRole('checkbox').first().check();
  await side.getByRole('button', { name: /Create 1 task/ }).click();
  await expect(side).toContainText('0 created, 1 already existed (not duplicated)');
  const keys = await page.evaluate(() => Object.keys(JSON.parse(localStorage.getItem('mockTaskKeys') ?? '{}')));
  expect(keys).toHaveLength(2);
  // Link explicitly, then keep a copy only on request.
  await page.getByRole('combobox', { name: 'Link to a workspace' }).click();
  await page.getByRole('option', { name: 'Q4 hiring plan' }).click();
  await expect(page.getByRole('button', { name: 'Keep a transcript copy there' })).toBeVisible();
  await page.getByRole('button', { name: 'Keep a transcript copy there' }).click();
  await expect(page.locator('.recording-kept')).toContainText('workspaces/local-hiring-plan/sources/recording-');
  await expect(page.getByRole('option', { name: /Weekly ops sync/ })).toContainText('Q4 hiring plan');
});

test('a connected PDF is read by Hollow and opens to its text', async ({ page }) => {
  // Repo features R14a: src/mockDirectories.ts's q3-review.pdf is `extracted`.
  // R14b's content index is off by default; this one has it on.
  await page.addInitScript(() => localStorage.setItem('mockContentIndexOn', '1'));
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'Connected projects' }).click();
  await expect(page.getByRole('region', { name: 'Source coverage' })).toContainText('1 PDF and Office file is read by Hollow.');
  // Repo features R14b: the content index's share of the listed text.
  await expect(page.getByRole('region', { name: 'Source coverage' })).toContainText('Contents indexed: 2,803 of 2,803.');
  const files = page.getByRole('region', { name: 'Connected project files' });
  await files.getByLabel('Find connected file').fill('q3 review');
  const deck = files.getByRole('button', { name: /q3-review\.pdf/ }).first();
  await expect(deck).toContainText('read by Hollow · PDF');
  await deck.click();
  const reader = page.getByRole('region', { name: 'Connected file reader' });
  await expect(reader).toContainText(/q3-review\.pdf · read only/);
  await expect(reader).toContainText('Occupancy rose to 87% across the Launch garages.');
});

test('without a Plaud sign-in, Recordings says so and copies nothing', async ({ page }) => {
  await library(page, 'Recordings', () => localStorage.setItem('mockPlaudOff', '1'));
  await expect(page.locator('.recordings-list')).toContainText('Plaud isn’t connected');
});
