import { expect, test, type Page } from './fixtures/test';

/**
 * Overnight 11-12 L3 in the browser mock: Library opens on All, one
 * searchable list of notes, meeting notes, people, recordings, result files
 * and canvases, newest first, with Recent, Meetings, People and Outputs. A
 * click previews in the side panel; a note's Open goes to Notes. One source
 * failing keeps the rest of the list.
 */
const tab = (page: Page, name: string) => page.locator('.library-tabs').getByRole('button', { name, exact: true });
const rows = (page: Page) => page.getByRole('list', { name: 'Library items' }).locator('.library-all-open');
const row = (page: Page, title: string) => rows(page).filter({ hasText: title }).first();
const chip = (page: Page, name: string) => page.getByRole('group', { name: 'Library filter' }).getByRole('button', { name, exact: true });

async function library(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await expect(page.locator('.home-project-row').first()).toBeVisible();
  // The classified vault index, with a meeting note, a person and an old note.
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts', storePath = '/src/store/runtime.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const day = 86_400_000, at = (ago: number) => new Date(Date.now() - ago).toISOString();
    const item = (path: string, title: string, ago: number, kind = 'note') => ({ path, title, kind, surface: 'vault', folder: path.split('/')[0], editable: true, hidden: false, trust: 'authored', mtime: at(ago), size: 100, excerpt: '', drift: null });
    api.vaultItems = async () => [
      item('library/records/meetings/2026/kickoff.md', 'Kickoff with Garage 14', 1 * day),
      item('library/work/people/ana-ruiz.md', 'Ana Ruiz', 2 * day, 'person'),
      item('library/notes/lease-terms.md', 'Lease terms', 3 * 3_600_000),
      item('library/notes/old-policy.md', 'Old policy', 90 * day),
      item('workspaces/local-q3-review/sources/raw.md', 'Raw source', 1 * day),
    ];
    await (await import(/* @vite-ignore */ storePath)).refetchVaultItems();
  });
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Library', exact: true }).click();
}

test.beforeEach(({}, info) => { test.skip(info.project.name === 'chromium-narrow', 'desktop Library layout'); });

test('Library opens on All: one list, filters, search, and a click previews in the side panel', async ({ page }) => {
  await library(page);
  await expect(page.locator('.library-tabs > button').first()).toHaveText('All');
  await expect(tab(page, 'All')).toHaveAttribute('aria-pressed', 'true');
  await expect(chip(page, 'Recent')).toHaveAttribute('aria-pressed', 'true');
  // Notes, people, meetings and recordings in one list; workspace working files stay out.
  for (const title of ['Lease terms', 'Kickoff with Garage 14', 'Ana Ruiz', 'Weekly ops sync']) await expect(row(page, title)).toBeVisible();
  await expect(rows(page).filter({ hasText: 'Raw source' })).toHaveCount(0);
  // Recent is the last 30 days; Show older lifts it.
  await expect(rows(page).filter({ hasText: 'Old policy' })).toHaveCount(0);
  await page.getByRole('button', { name: /^Show older/ }).click();
  await expect(row(page, 'Old policy')).toBeVisible();

  await chip(page, 'People').click();
  await expect(rows(page)).toHaveCount(1);
  await expect(row(page, 'Ana Ruiz')).toContainText('Person · work/people');
  await chip(page, 'Meetings').click();
  await expect(row(page, 'Kickoff with Garage 14')).toBeVisible();
  await expect(row(page, 'Weekly ops sync')).toContainText('Recording · Plaud');
  await expect(rows(page).filter({ hasText: 'Ana Ruiz' })).toHaveCount(0);
  await chip(page, 'Outputs').click();
  await expect(row(page, 'Q3 regional operations review')).toBeVisible();
  await expect(rows(page).filter({ hasText: 'Lease terms' })).toHaveCount(0);

  // Search filters as you type; nothing found offers Clear search.
  await chip(page, 'Recent').click();
  const search = page.getByRole('searchbox', { name: 'Search your Library' });
  await search.fill('ana ruiz');
  await expect(rows(page)).toHaveCount(1);
  await search.fill('zzzz no such thing');
  await expect(page.getByText('Nothing matches your search')).toBeVisible();
  await page.getByRole('button', { name: 'Clear search', exact: true }).click();
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();

  // j moves between rows; Enter previews in the side panel.
  await rows(page).first().focus();
  await page.keyboard.press('j');
  await expect(rows(page).nth(1)).toBeFocused();
  await row(page, 'Ana Ruiz').focus();
  await page.keyboard.press('Enter');
  const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
  await expect(pane).toBeVisible();
  await expect(pane.getByRole('tab', { name: 'ana-ruiz.md', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Close preview', exact: true }).click();

  // A note's Open goes to Notes with it open, to read and edit. Notes is
  // All's view (L4): All stays selected and is what Library remembers.
  await page.getByRole('button', { name: 'Open Lease terms in Notes', exact: true }).click();
  await expect(tab(page, 'All')).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Back to All', exact: true })).toBeVisible();
  await expect(page.locator('.library-reader')).toBeVisible();
  expect(await page.evaluate(() => localStorage.getItem('hollow.library.mode'))).toBe('all');
});

test('one source failing keeps the rest of All and offers its own Retry', async ({ page }) => {
  await library(page);
  await expect(row(page, 'Weekly ops sync')).toBeVisible();
  await page.evaluate(async () => {
    const apiPath = '/src/api.ts';
    const { api } = await import(/* @vite-ignore */ apiPath);
    const real = api.listRecordings;
    (window as unknown as { restoreRecordings: () => void }).restoreRecordings = () => { api.listRecordings = real; };
    api.listRecordings = async () => { throw new Error('Plaud sign-in expired'); };
  });
  await page.getByRole('button', { name: 'Refresh the list', exact: true }).click();
  await expect(page.getByText('Recordings couldn’t be read')).toBeVisible();
  await expect(rows(page).filter({ hasText: 'Weekly ops sync' })).toHaveCount(0);
  await expect(row(page, 'Ana Ruiz')).toBeVisible();
  // People reads notes and result files: the recordings failure isn't its business.
  await chip(page, 'People').click();
  await expect(page.getByText('Recordings couldn’t be read')).toHaveCount(0);
  await chip(page, 'Recent').click();
  await page.evaluate(() => (window as unknown as { restoreRecordings: () => void }).restoreRecordings());
  await page.getByRole('button', { name: 'Retry recordings', exact: true }).click();
  await expect(row(page, 'Weekly ops sync')).toBeVisible();
  await expect(page.getByText('Recordings couldn’t be read')).toHaveCount(0);
});
