// The perf scripts' route through the current shell (the 11-17 sidebar: Main,
// Project navigation with the recent projects, the sidebar's Search chats).
// One place, so a shell redesign breaks one file, not four scripts silently;
// `scripts/perf-nav.test.mjs` pins the selectors against the source.

/** The sidebar's recent-project rows (buttons), the rail's row count. */
export const RAIL_ROWS = 'nav[aria-label="Project navigation"] button:not([aria-label^="Actions for"]):not([aria-label^="New "])';
/** The Work page's title. */
export const PAGE_TITLE = 'h1.page-title';
/** Home's start-a-conversation section: Home has drawn. */
export const HOME_READY = 'section[aria-label="Start a conversation"]';
/** A task row on the Tasks page. */
export const TASK_ROW = '.issue-row';

/** A recent project's row in the sidebar. */
export const railProject = (page, title) =>
  page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: title, exact: true }).first();

/** Open a chat from the sidebar's chat search: its row under the query is clicked. */
export async function openChatBySearch(page, query, title) {
  await page.getByRole('button', { name: 'Search chats' }).first().click();
  await page.getByRole('combobox', { name: 'Search chats' }).fill(query);
  const row = page.locator('.work-rail-search-chat', { hasText: title }).first();
  await row.waitFor();
  return row;
}

/** The command palette, from the sidebar search's last row (Search everything). */
export async function openPalette(page) {
  await page.getByRole('button', { name: 'Search chats' }).first().click();
  await page.getByRole('option', { name: /^Search everything/ }).first().click();
  const palette = page.getByRole('dialog', { name: 'Command palette' });
  await palette.waitFor();
  return palette;
}
