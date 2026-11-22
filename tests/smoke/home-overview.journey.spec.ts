import { expect, test } from './fixtures/test';
import { showWorkspaceList } from './choose';

test('Home starts with Ask, five recent projects and an explicit full list', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('textbox', { name: 'Ask anything' })).toBeVisible();
  const projects = page.getByRole('region', { name: 'Projects', exact: true });
  await expect(projects.locator('.home-project-row').first()).toBeVisible();
  expect(await projects.locator('.home-project-row').count()).toBeLessThanOrEqual(5);
  await expect(page.locator('.doorway-landscape--home')).toHaveCount(0);
  await showWorkspaceList(page);
  await projects.getByRole('searchbox', { name: 'Find a project' }).fill('Q3');
  await expect(projects.locator('.home-project-row').first()).toContainText('Q3');
  await projects.getByRole('button', { name: 'Show recent projects' }).click();
  await expect(projects.getByRole('searchbox')).toHaveCount(0);
  await expect(projects.getByRole('button', { name: 'All projects' })).toBeVisible();
});

test('the sidebar Projects row, then All projects, expands projects and focuses Find', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  // Wait for the top bar's toggle: an instant isVisible() before it renders
  // skipped Show sidebar at the minimum window, where the rail starts as icons.
  const toggle = page.getByRole('button', { name: /^(Show|Hide) sidebar$/ });
  await expect(toggle).toBeVisible();
  if (await toggle.getAttribute('aria-label') === 'Show sidebar') await toggle.click();
  await expect(toggle).toHaveAttribute('aria-label', 'Hide sidebar');
  // The sidebar's All projects row went in L1 (11-12): its Projects row
  // opens the page, whose All projects lists every project.
  await expect(page.getByRole('navigation', { name: 'Project navigation' }).getByRole('button', { name: /^All projects/ })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Projects/ }).click();
  await page.getByRole('region', { name: 'Projects', exact: true }).getByRole('button', { name: 'All projects', exact: true }).click();
  await expect(page.getByRole('searchbox', { name: 'Find a project' })).toBeFocused();
});

test('project row descriptions retain status, merged destination and conversation facts', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const row = page.getByRole('button', { name: 'Open Q3 regional operations review', exact: true });
  // Patch the mock's own store after the first list read, then re-read it the
  // way the app does, so no later refresh can put the unpatched row back.
  await expect(row).toBeVisible();
  await page.evaluate(async () => {
    const patch = (w: any) => w.title === 'Q3 regional operations review' ? { ...w, status: 'blocked', mergedInto: 'other-project' } : w;
    localStorage.setItem('mockWorkspaces', JSON.stringify(JSON.parse(localStorage.getItem('mockWorkspaces') ?? '[]').map(patch)));
    const path = '/src/store/workspaces.ts';
    const store = await import(/* @vite-ignore */ path);
    await store.refreshWorkspaces();
  });
  await expect(row).toHaveAccessibleDescription(/blocked.*Merged into other-project/);
  await expect(row.locator('.work-card-status')).toHaveClass(/is-blocked/);
});
