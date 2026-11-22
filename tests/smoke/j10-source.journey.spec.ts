/** Journey J10: attach a snapshot and inspect its recorded provenance. */
import { expect, test, type Page } from './fixtures/test';
import { openProjectFile, previewPane } from './choose';

const PICKED = '/Users/fixture/Documents/staffing-memo.md';

async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

test('J10 click-only: attach a source and read its provenance', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  await expect(page.locator('.top-bar-crumbs')).toContainText('Q3 regional operations review');

  // The native file picker answers with a local file (the browser mock has
  // no Tauri dialog), and its original has moved on since: the mock can't
  // stat it, so the fixture says so.
  await page.evaluate(picked => {
    localStorage.setItem('mockSourceFreshness', JSON.stringify({ 'sources/staffing-memo.md': 'changed' }));
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (command: string) => command === 'plugin:dialog|open' ? picked : null,
    };
  }, PICKED);
  const conversation = page.getByRole('region', { name: 'Workspace conversation' });
  await conversation.getByRole('button', { name: 'Attach and commands', exact: true }).click();
  await page.getByRole('menu').getByRole('menuitem', { name: 'Add to Sources…', exact: true }).click();
  await expect(page.getByText('Added to Sources.', { exact: true })).toBeVisible();
  const attached = await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.attach_workspace_source ?? 0);
  expect(attached, 'one snapshot attached').toBe(1);

  await openProjectFile(page, 'SOURCES.md');
  const pane = previewPane(page);
  await expect(pane.getByRole('tab', { name: 'SOURCES.md', exact: true })).toHaveAttribute('aria-selected', 'true');
  await expect(pane).toContainText('staffing-memo.md');
  await expect(pane).toContainText(PICKED);
  await expect(pane).toContainText('SHA-256');
  await noHorizontalScroll(page);
});
