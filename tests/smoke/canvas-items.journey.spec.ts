/**
 * Copy to Plan or Tasks was cut overnight 11-08 S3.
 * A required spec: it runs and may not skip.
 */
import { expect, test } from './fixtures/test';

test('click-only: a canvas has no Copy to Plan or Tasks', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await expect(page.getByRole('region', { name: 'Projects' })).toBeVisible();
  await expect(page.getByRole('button', { name: /Add items to Plan or Tasks/ })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /Copy to Plan or Tasks/ })).toHaveCount(0);
});
