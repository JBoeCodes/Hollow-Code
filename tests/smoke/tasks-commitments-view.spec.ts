import { expect, test } from './fixtures/test';
import { openTasks } from './choose';

test('Tasks has no Commitments view', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await openTasks(page);
  await expect(page.getByRole('navigation', { name: 'Tasks and commitments' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Commitments', exact: true })).toHaveCount(0);
});
