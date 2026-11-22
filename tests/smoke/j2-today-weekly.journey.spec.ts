/**
 * Journey J2. Today and Weekly review were cut overnight 11-08 S3.
 * Work is Home and Running. The workspace map was cut overnight 11-08 S4.
 * A required spec: it runs and may not skip.
 */
import { expect, test } from './fixtures/test';

test('J2 click-only: Work is Home and Running, with no Today', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  const views = page.getByRole('navigation', { name: 'Projects views' });
  await expect(views.getByRole('button', { name: 'Home', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(views.getByRole('button', { name: 'Running', exact: true })).toBeVisible();
  await expect(views.getByRole('button', { name: 'Today', exact: true })).toHaveCount(0);
  await expect(page.getByRole('group', { name: 'Today range' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Projects' }).getByRole('button', { name: 'Map', exact: true })).toHaveCount(0);
  await views.getByRole('button', { name: 'Running', exact: true }).click();
  await expect(page.getByRole('region', { name: 'Running', exact: true })).toBeVisible();
});
