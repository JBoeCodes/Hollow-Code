/**
 * Journey J3. Incoming was cut overnight 11-08 S3.
 * Needs you is Waiting: All, Decisions, Results and Failures, each named
 * with its count ("All 3").
 * A required spec: it runs and may not skip.
 */
import { expect, test } from './fixtures/test';
import { navigateWaiting } from './waiting';

test('J3 click-only: Needs you is Waiting, with no Incoming', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await navigateWaiting(page);
  const filters = page.getByRole('navigation', { name: 'Waiting filters' });
  await expect(filters.getByRole('button', { name: /^All( \d+)?$/ })).toHaveAttribute('aria-pressed', 'true');
  await expect(filters.getByRole('button', { name: /^Decisions( \d+)?$/ })).toBeVisible();
  await expect(filters.getByRole('button', { name: /^Results( \d+)?$/ })).toBeVisible();
  await expect(filters.getByRole('button', { name: /^Failures( \d+)?$/ })).toBeVisible();
  await expect(filters.getByRole('button', { name: /^Incoming( \d+)?$/ })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Waiting on you' })).toBeVisible();
});
