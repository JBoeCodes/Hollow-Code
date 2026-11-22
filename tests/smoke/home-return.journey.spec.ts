import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { showWorkspaceList } from './choose';

test('recent project rows are accessible and open the named project', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const projects = page.getByRole('region', { name: 'Projects', exact: true });
  await expect(projects.locator('.home-project-row').first()).toBeVisible();
  expect((await new AxeBuilder({ page }).include('.home-projects').analyze()).violations).toEqual([]);
  await showWorkspaceList(page);
  await projects.getByRole('button', { name: 'Open Q3 regional operations review', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Workspace conversation' })).toBeVisible();
});
