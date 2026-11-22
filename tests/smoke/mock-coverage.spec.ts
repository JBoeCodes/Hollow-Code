import { expect, test, type Page } from './fixtures/test';

/**
 * The fixture-coverage net. `mockApi` throws on any command it has no fixture
 * for, and records the name on `window.__mockApiMisses`. Both halves matter:
 * plenty of call sites catch their own IPC errors, so a throw on its own can be
 * swallowed and leave the gap invisible — which is exactly how 108 of 209
 * commands stayed unmocked while nine smoke tests passed.
 *
 * This walks every rail destination and asserts the miss list stays empty, so
 * adding a page-local resource without a fixture fails the suite instead of
 * silently rendering an empty surface.
 */

const PAGES = [
  'Today', 'Calendar', 'Tasks',
  'Projects', 'Workflows', 'Chat', 'Vault', 'People', 'Settings',
];

async function misses(page: Page): Promise<string[]> {
  return page.evaluate(() => window.__mockApiMisses ?? []);
}

test('every rail destination renders with a complete mock fixture set', async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on('console', msg => {
    if (msg.type() === 'error') consoleErrors.push(msg.text());
  });

  await page.goto('/?mock=1');
  await expect(page.getByLabel('Main navigation')).toBeVisible();

  for (const name of PAGES) {
    const row = page.getByRole('button', { name: new RegExp(name) }).first();
    if (!(await row.count())) continue;
    await row.click();
    // Let the page's own resources fire before sampling.
    await page.waitForTimeout(250);
  }

  const unmocked = await misses(page);
  expect(
    unmocked,
    `mockApi has no fixture for these commands:\n  ${unmocked.join('\n  ')}\n` +
    'Add a case to src/mockApi.ts (or NO_OP_COMMANDS) for each.',
  ).toEqual([]);

  // A thrown-and-caught IPC error still shows up here, so this catches the
  // swallowed half of the same problem.
  const mockErrors = consoleErrors.filter(t => t.includes('mockApi: no fixture'));
  expect(mockErrors, `unhandled mock commands logged to console:\n${mockErrors.join('\n')}`).toEqual([]);
});

test('the command palette and overlay windows stay fixture-complete', async ({ page }) => {
  await page.goto('/?mock=1');
  // Global search's route (overnight 11-04 S2): Search chats (the top bar's
  // icon since L1, 11-12), then its last row, Search everything.
  await page.locator('.top-bar').getByRole('button', { name: 'Search chats', exact: true }).click();
  await page.getByRole('option', { name: 'Search everything', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Command palette' })).toBeVisible();
  await page.locator('.palette-input').fill('workflow');
  await page.waitForTimeout(250);
  await page.keyboard.press('Escape');

  expect(await misses(page)).toEqual([]);
});
