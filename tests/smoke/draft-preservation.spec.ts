import { expect, test } from './fixtures/test';

/**
 * Plan completion item 1 (verification V01, 2026-10-13): an unsent draft is
 * kept until a handoff works or the person discards it. The Commitments New
 * form was cut with that screen (overnight 11-08 S3). Home's Ask keeps its
 * draft in the store, so leaving Work and coming back still shows it.
 */

test('Home keeps a half-written Ask, and Tasks has no Commitments draft', async ({ page }) => {
  await page.goto('/?mock=1');
  const ask = page.getByRole('textbox', { name: 'Ask anything' });
  await ask.fill('Send the venue shortlist');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Tasks', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Tasks', level: 1 })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Tasks and commitments' })).toHaveCount(0);
  await expect(page.getByRole('region', { name: 'Commitments', exact: true })).toHaveCount(0);
  await expect(page.getByRole('form', { name: 'New commitment' })).toHaveCount(0);
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Projects', exact: true }).click();
  await expect(page.getByRole('textbox', { name: 'Ask anything' })).toHaveValue('Send the venue shortlist');
});
