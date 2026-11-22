import { expect, test } from './fixtures/test';

test('Home keeps stale projects with a list error, and Retry recovers', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const projects = page.getByRole('region', { name: 'Projects', exact: true });
  await expect(projects.locator('.home-project-row').first()).toBeVisible();
  const before = await projects.locator('.home-project-row').count();
  await page.evaluate(async () => {
    const path = '/src/store/workspaces.ts';
    const store = await import(/* @vite-ignore */ path);
    store.setWorkspaceListError('Synthetic index failure');
  });
  await expect(projects.getByRole('alert')).toContainText('Couldn’t load projects');
  await expect(projects.locator('.home-project-row')).toHaveCount(before);
  await expect(projects.getByText('Loading projects…')).toHaveCount(0);
  await projects.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(projects.getByRole('alert')).toHaveCount(0);
});

test('Home never starts removed digest polling', async ({ page }) => {
  await page.clock.install();
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects', exact: true })).toBeVisible();
  await page.clock.runFor(120_000);
  expect(await page.evaluate(() => window.__mockApiCalls?.workspace_digests ?? 0)).toBe(0);
});

test('a background refresh preserves an empty search without list markup or loading flicker', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const projects = page.getByRole('region', { name: 'Projects', exact: true });
  await projects.getByRole('button', { name: 'All projects', exact: true }).click();
  await projects.getByRole('searchbox', { name: 'Find a project' }).fill('No such project 82919');
  await expect(projects.getByText('No matching projects')).toBeVisible();
  await page.evaluate(async () => {
    const path = '/src/store/workspaces.ts';
    (await import(/* @vite-ignore */ path)).setWorkspaceListLoading(true);
  });
  await expect(projects.getByText('No matching projects')).toBeVisible();
  await expect(projects.getByText('Loading projects…')).toHaveCount(0);
  await expect(projects.getByRole('list')).toHaveCount(0);
});
