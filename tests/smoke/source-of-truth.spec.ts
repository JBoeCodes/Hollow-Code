import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

// The Hollow folder is a part of Sources since Settings PR 3, and Settings opens on General.
async function openSettings(page: Page, section = 'sources') {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), section);
  } else {
    await page.locator('.settings-index').getByRole('button', { name: section === 'connectors' ? 'Connectors' : 'Sources', exact: true }).click();
  }
}

test('first-run sets up one Hollow folder', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockSetupConfigured', 'false'));
  await page.goto('/?mock=1&setup=1');
  await expect(page.getByRole('heading', {name:'Bring your work into Hollow'})).toBeVisible();
  await expect(page.getByLabel('Hollow folder', {exact:true})).toHaveValue('~/Hollow');
  await page.getByLabel('Hollow folder', {exact:true}).fill('');
  await expect(page.getByRole('button', {name:'Set up Hollow folder'})).toBeDisabled();
  await page.getByLabel('Hollow folder', {exact:true}).fill('/Work/Hollow');
  await page.getByText('Bring existing folders').click();
  await page.getByLabel('Existing projects folder', {exact:true}).fill('/Work');
  await page.getByRole('button', {name:'Set up Hollow folder'}).click();
  await expect(page.getByRole('alert')).toContainText("can't be inside");
  await page.getByLabel('Existing projects folder', {exact:true}).fill('/Code');
  await page.getByRole('button', {name:'Set up Hollow folder'}).click();
  await expect(page.getByRole('button', {name:'Restart Hollow to use this folder'})).toBeVisible();
  const saved=await page.evaluate(()=>JSON.parse(localStorage.getItem('mockDirectories')??'{}'));
  expect(saved.homePath).toBe('/Work/Hollow');expect(saved.projectsPath).toBe('/Code');expect(saved.vaultPath).toBe('/Work/Hollow/Vault');
});

test('Settings shows the one Hollow folder and refresh without changing accounts', async ({ page }) => {
  await page.goto('/?mock=1');await openSettings(page);
  await expect(page.locator('.settings-section-title, h2, h3').filter({hasText:/^Hollow folder$/}).first()).toBeVisible();
  await expect(page.getByLabel('Hollow folder',{exact:true})).toHaveValue('/Hollow');
  await expect(page.getByRole('list',{name:'Inside the Hollow folder'})).toContainText('/Hollow/Projects');
  await page.getByRole('button',{name:'Refresh projects',exact:true}).click();
  await expect(page.locator('#settings-vault .directory-status')).toContainText('1 projects');
});

test('Hollow has no Notion import or setup in Connections (removed 2026-11-10)', async ({ page }) => {
  await page.goto('/?mock=1');
  await openSettings(page, 'connectors');
  await expect(page.getByRole('heading', { name: 'Services', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Import new items', exact: true })).toHaveCount(0);
  await expect(page.getByText('Configure Notion connection', { exact: true })).toHaveCount(0);
  await expect(page.getByLabel('Tasks data source ID', { exact: false })).toHaveCount(0);
});
