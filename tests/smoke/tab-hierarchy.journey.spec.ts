import { openTasks } from './choose';
import { expect, test } from './fixtures/test';
import { openLibraryView } from './libraryView';

test('destination tabs and secondary choices retain independent native selection', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  const rail = page.getByRole('navigation', { name: 'Main' });
  await openTasks(page);
  await expect(page.getByRole('navigation', { name: 'Tasks and commitments' })).toHaveCount(0);
  await rail.getByRole('button', { name: 'Library', exact: true }).click();
  const library = page.getByRole('navigation', { name: 'Library view', exact: true });
  await expect(library).toHaveClass(/ui-level-primary/);
  await openLibraryView(page, 'Results');
  const type = page.getByRole('group', { name: 'Result type', exact: true });
  await expect(type).toHaveClass(/ui-level-secondary/);
  const canvases = type.getByRole('button', { name: 'Canvases', exact: true });
  await canvases.focus(); await page.keyboard.press('Enter');
  await expect(canvases).toHaveAttribute('aria-pressed', 'true');
  await expect(library.getByRole('button', { name: 'All', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await rail.getByRole('button', { name: 'Projects', exact: true }).click();
  await page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Running', exact: true }).click();
  const runningLayout = page.getByRole('group', { name: 'Running layout', exact: true });
  await expect(runningLayout).toHaveClass(/ui-level-secondary/);
  await runningLayout.getByRole('button').nth(1).focus(); await page.keyboard.press('Space');
  await expect(runningLayout.getByRole('button').nth(1)).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => document.scrollingElement!.scrollWidth <= innerWidth)).toBe(true);
});
