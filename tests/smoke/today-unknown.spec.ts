import { expect, test } from './fixtures/test';

test('Today is not a screen', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await expect(page.getByRole('region', { name: 'Today content' })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: 'Projects views' }).getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
});
