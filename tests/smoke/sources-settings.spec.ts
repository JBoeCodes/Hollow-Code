import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

// Settings, Sources, Where your work lives (H4b with T5): three role rows, a
// switch shown as a dry run before anything is written, and Roll back.
async function openSources(page: Page, query = '') {
  await page.goto(`/?mock=1${query}`);
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'sources');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Sources', exact: true }).click();
  }
}

// Browser fix 2 (11-19): the role rows sit in a closed group so Sources keeps
// its 1.3 screens (settings-height.spec.ts); open it to work with them.
async function openRoles(page: Page) {
  await page.locator('#settings-group-where-work-lives > summary').click();
  await expect(page.locator('#settings-group-where-work-lives')).toHaveAttribute('open', '');
}

test('shows the three role rows with their labels', async ({ page }) => {
  await openSources(page);
  await openRoles(page);
  const panel = page.locator('.sources-panel');
  await expect(panel.getByRole('heading', { name: 'Tasks and projects' })).toBeVisible();
  await expect(panel.getByRole('heading', { name: 'Knowledge' })).toBeVisible();
  await expect(panel.getByRole('heading', { name: 'Personal context' })).toBeVisible();
  await expect(panel).toContainText('Tracked in Linear');
  await expect(panel).toContainText('Always in your Hollow folder');
  await expect(panel).toContainText('Read-only from Notion');
  await expect(panel.getByRole('group', { name: 'Where tasks live' }).getByRole('button', { name: 'Vault plus Linear' })).toHaveAttribute('aria-pressed', 'true');
});

test('a switch shows its dry run first and writes nothing until confirmed', async ({ page }) => {
  await openSources(page);
  await openRoles(page);
  const panel = page.locator('.sources-panel');
  await panel.getByRole('group', { name: 'Where tasks live' }).getByRole('button', { name: 'Vault only' }).click();
  const plan = page.getByRole('region', { name: 'Switch dry run' });
  await expect(plan).toContainText('12 tasks and 3 projects mirrored now');
  await expect(plan).toContainText('Nothing is sent to the tracker');
  await plan.getByRole('button', { name: 'Cancel' }).click();
  await expect(panel).toContainText('Tracked in Linear');

  await panel.getByRole('group', { name: 'Where tasks live' }).getByRole('button', { name: 'Vault only' }).click();
  await page.getByRole('region', { name: 'Switch dry run' }).getByRole('button', { name: 'Switch to Vault only' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Switch', exact: true }).click();
  const tasksRow = panel.locator('[data-settings-control="Tasks and projects"]');
  await expect(tasksRow).toContainText('Stored in Hollow');
  await expect(tasksRow.getByRole('button', { name: 'Vault only' })).toHaveAttribute('aria-pressed', 'true');
  await panel.getByRole('button', { name: 'Roll back' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Roll back', exact: true }).click();
  await expect(tasksRow).toContainText('Tracked in Linear');
});

test('the settings search lands on each role row', async ({ page }) => {
  await openSources(page);
  for (const name of ['Knowledge', 'Personal context', 'Workspace task sources']) {
    await page.getByRole('searchbox', { name: 'Find a setting' }).fill(name);
    // The control's own hit (as settings-shell.spec.ts): Enter opens the top
    // hit, which for "Knowledge" is a section whose search words name it.
    await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name, exact: true }).and(page.locator('.settings-index-control')).click();
    await expect(page.locator('.sources-panel').getByRole('heading', { name })).toBeInViewport();
  }
});
