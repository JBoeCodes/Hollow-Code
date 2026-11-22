import { expect, test, type Page } from './fixtures/test';
import { choose, chooseNew } from './choose';

// A08 (overnight 2026-10-16): the Work page banner puts an OS error in plain
// words, with the raw text under Details, while a build workspace's warnings
// (Hollow's own sentences about the repo) show exactly as written.

const RAW = 'No such file or directory (os error 2)';
const PLAIN = 'That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.';
// Raw git text inside, as the real warning has (build_workspaces.rs): plain
// words would rewrite it, so this proves the warning path stays verbatim.
const WARNING = 'Could not fetch origin (Connection refused (os error 61)); the branch starts from what this Mac last saw.';

/** The Work page's own banner, not another surface's `.work-error`. */
const banner = (page: Page) => page.locator('.work-page > .work-error');

test.beforeEach(async ({ page }) => { await page.goto('/?mock=1'); });

test('an OS error in the Work banner reads as a sentence, with the raw text under Details', async ({ page }) => {
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Check the banner.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Banner check');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Banner check');
  await expect(banner(page)).toHaveCount(0);

  // Reveal in Finder fails as the backend would, with a string rejection.
  await page.evaluate((raw) => {
    (window as unknown as { __mockScenario?: unknown }).__mockScenario = { names: [], delayMs: {}, unavailable: { vault_reveal_in_finder: raw } };
  }, RAW);
  // By keyboard: at 390px the top bar's right edge overlaps this button for a pointer. The top bar can
  // still re-render as the new project's detail lands, which drops a focus set a moment earlier (the
  // Enter then goes nowhere: 3 runs in 30 timed out here), so focus and Enter are tried together
  // until the menu is there.
  const actions = page.getByRole('button', { name: 'Actions for Banner check', exact: true }).first();
  await expect(async () => {
    await actions.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByRole('menu')).toBeVisible({ timeout: 1500 });
  }).toPass({ timeout: 20_000 });
  await page.getByRole('menu').getByRole('menuitem', { name: 'Reveal in Finder', exact: true }).click();

  await expect(banner(page)).toContainText(PLAIN);
  const toggle = banner(page).getByRole('button', { name: 'Details', exact: true });
  const detail = banner(page).locator('.inline-error-detail');
  await expect(detail).toBeHidden();
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(detail).toBeVisible();
  await expect(detail).toHaveText(RAW);

  await banner(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  await expect(banner(page)).toHaveCount(0);
});

test('a build workspace warning shows exactly as written, with no Details', async ({ page }) => {
  // The mock's create_build_workspace returns its offline-fetch warning.
  await page.evaluate(() => localStorage.setItem('mockBuildFetchFails', '1'));
  await chooseNew(page, 'New project');
  await page.getByRole('textbox', { name: 'Project instructions' }).fill('Fix the About page layout.');
  await page.getByRole('textbox', { name: 'Project name' }).fill('Offline build');
  await page.locator('.project-create-options > summary').click();
  await page.getByRole('radio', { name: 'Repository worktree' }).click();
  await choose(page.getByRole('combobox', { name: 'Repository' }), '/Users/fixture/Projects/site');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.locator('.work-header h1')).toHaveText('Offline build');

  // Shown after the workspace opens (opening clears the banner).
  await expect(banner(page)).toContainText(WARNING);
  await expect(banner(page).getByRole('button', { name: 'Details', exact: true })).toHaveCount(0);
  await expect(banner(page).locator('.inline-error-details')).toHaveCount(0);

  // Dismiss clears it, and a later error is plain again.
  await banner(page).getByRole('button', { name: 'Dismiss', exact: true }).click();
  await expect(banner(page)).toHaveCount(0);
  await page.evaluate(async (raw) => {
    const ws = await import('/src/store/workspaces.ts');
    ws.setWorkspaceError(raw);
  }, RAW);
  await expect(banner(page)).toContainText(PLAIN);
  await expect(banner(page).getByRole('button', { name: 'Details', exact: true })).toHaveCount(1);
});
