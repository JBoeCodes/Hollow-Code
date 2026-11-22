import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

// One welcome page at desktop and narrow widths. Browser execution belongs
// to the run's final suite, not the individual parity PR.
const sheet = (page: Page) => page.getByRole('dialog', { name: 'Welcome to Hollow' });

async function openSection(page: Page, id: string, label: string) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), id);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: label, exact: true }).click();
  }
}

test('one welcome page offers agents, optional setup and an explicit no-agent start', async ({ page }) => {
  await page.goto('/?mock=1&firstRun=1');
  const s = sheet(page);
  await expect(s.getByRole('list', { name: 'Agents on this Mac' })).toBeVisible();
  await expect(s.getByRole('button', { name: 'Add more agents', exact: true })).toBeVisible();
  await s.getByText('Details', { exact: true }).first().click();
  await expect(s.getByRole('button', { name: 'Manage in Accounts' }).first()).toBeVisible();
  await s.getByRole('button', { name: 'Start without an agent' }).click();
  await expect(sheet(page)).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole('button', { name: /Settings/ }).first()).toBeVisible();
  await expect(sheet(page)).toHaveCount(0);
});

test('closing leaves setup open and a reload returns to the welcome page', async ({ page }) => {
  await page.goto('/?mock=1&firstRun=1');
  await sheet(page).getByRole('button', { name: 'Close', exact: true }).click();
  await expect(sheet(page)).toHaveCount(0);
  await page.reload();
  await expect(sheet(page).getByRole('heading', { name: 'Your agents' })).toBeVisible();
});

test('welcome shows a phone QR only after explicit pairing and can cancel it', async ({ page }) => {
  await page.goto('/?mock=1&firstRun=1');
  const s = sheet(page);
  await s.getByText('Set up your iPhone', { exact: true }).click();
  await expect(s.getByRole('img', { name: /QR code of the pairing link/ })).toHaveCount(0);
  await s.getByRole('switch', { name: 'Serve the iPhone app', exact: true }).click();
  await s.getByRole('button', { name: 'Pair an iPhone', exact: true }).click();
  await expect(s.getByRole('img', { name: /QR code of the pairing link/ })).toBeVisible();
  await s.getByRole('button', { name: 'Cancel pairing', exact: true }).click();
  await expect(s.getByRole('img', { name: /QR code of the pairing link/ })).toHaveCount(0);
});

test('welcome keeps usable import rows while showing the scan limit', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSessionImportCapped', '1'));
  await page.goto('/?mock=1&firstRun=1');
  const s = sheet(page);
  await s.getByText('Import conversations and context', { exact: true }).click();
  const source = s.getByRole('region', { name: 'Claude Code', exact: true });
  await source.getByRole('button', { name: 'Look', exact: true }).click();
  await expect(source).toContainText('Scan limit reached. This list is incomplete.');
  await expect(source.getByRole('button', { name: 'Import 2 conversations' })).toBeEnabled();
});

test('imports ask before reading and keep selections when their section is collapsed', async ({ page }) => {
  await page.goto('/?mock=1&firstRun=1');
  const s = sheet(page);
  await s.getByText('Import conversations and context', { exact: true }).click();
  const claude = s.getByRole('region', { name: 'Claude Code', exact: true });
  await expect(claude).toContainText('Nothing is read until you click Look.');
  await claude.getByRole('button', { name: 'Look', exact: true }).click();
  await expect(claude).toContainText('4 conversations: 2 new, 1 already imported');
  await claude.getByRole('button', { name: 'Select none' }).click();
  await s.getByText('Import conversations and context', { exact: true }).click();
  await s.getByText('Import conversations and context', { exact: true }).click();
  await expect(claude.getByRole('button', { name: 'Import', exact: true })).toBeDisabled();
  await claude.getByRole('button', { name: 'Select all' }).click();
  await claude.getByRole('button', { name: 'Import 3 conversations' }).click();
  await expect(claude).toContainText('Imported 2, 1 already there.');
  await s.getByRole('button', { name: 'Scan for instruction and memory files' }).click();
  await expect(s.getByRole('region', { name: 'Found context' })).toBeVisible();
  await expect(s).toContainText('Imported 2 conversations.');
  await s.getByRole('button', { name: 'Open Chats', exact: true }).click();
  await expect(sheet(page)).toHaveCount(0);
});

test('optional context and sources retain their controls', async ({ page }) => {
  await page.goto('/?mock=1&firstRun=1');
  const s = sheet(page);
  await s.getByText('Connect your context', { exact: true }).click();
  await expect(s.locator('.context-layers')).toBeVisible();
  await s.getByText('Where your work lives', { exact: true }).click();
  await expect(s.locator('.sources-panel').getByRole('heading', { name: 'Tasks and projects' })).toBeVisible();
  await s.getByRole('button', { name: 'Connect a task tracker' }).click();
  await expect(s.getByRole('button', { name: 'Hide the task tracker' })).toHaveAttribute('aria-expanded', 'true');
});

test('About reopens welcome, and Sources keeps its existing route', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSection(page, 'about', 'About');
  await page.getByRole('button', { name: 'Open welcome page' }).click();
  await expect(sheet(page)).toBeVisible();
  await sheet(page).getByRole('button', { name: 'Start without an agent' }).click();
  await openSection(page, 'sources', 'Sources');
  await page.locator('#settings-group-where-work-lives > summary').click();
  await page.getByRole('button', { name: 'Start setup' }).click();
  await expect(sheet(page)).toBeVisible();
});
