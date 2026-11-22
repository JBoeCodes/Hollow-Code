/**
 * Library's views opened from All (overnight 11-12 L4). Notes, Results and
 * Recordings left the tab strip: Notes is All → Browse folders, Results is
 * All → Outputs → Grid, Recordings is All → Meetings → Recordings. Call it
 * with Library on screen; it clicks the All tab first, so it works from any
 * Library view.
 */
import type { Page } from './fixtures/test';

export type LibraryDetail = 'Notes' | 'Results' | 'Recordings';

export async function openLibraryView(page: Page, view: LibraryDetail) {
  await page.getByRole('navigation', { name: 'Library view' }).getByRole('button', { name: 'All', exact: true }).click();
  const all = page.getByRole('region', { name: 'All', exact: true });
  if (view === 'Notes') { await all.getByRole('button', { name: 'Browse folders', exact: true }).click(); return; }
  const filter = all.getByRole('group', { name: 'Library filter' });
  await filter.getByRole('button', { name: view === 'Results' ? 'Outputs' : 'Meetings', exact: true }).click();
  await all.getByRole('button', { name: view === 'Results' ? 'Grid' : 'Recordings', exact: true }).click();
}

/** The control above every view All opens, back to the list. */
export const backToAll = (page: Page) => page.getByRole('button', { name: 'Back to All', exact: true });
