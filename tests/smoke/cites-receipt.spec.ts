/**
 * Repo features R5: a run's Run record has a Cites row, the `path:line`
 * cites in its final reply checked when it ended. Against src/mockApi.ts:
 * the Morning Brief's newest receipt (`workflowReceipt`) cites three lines,
 * one past the end of its file. Visible controls only: Automations → Morning
 * Brief → Run record → Cites.
 */
import { expect, test } from './fixtures/test';

test('click-only: the Run record lists the reply cite that is past the end of its file', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).getByRole('button', { name: 'Open Morning Brief' }).click();
  const review = page.locator('.workflow-review');
  await expect(review).toBeVisible();

  const receipt = review.locator('.execution-receipt').first();
  await receipt.locator('.execution-receipt-summary').click();
  const cites = receipt.locator('.execution-receipt-cites');
  await expect(cites).toContainText('Cites · 3 checked · 2 seen · 1 past end of file');
  await cites.locator('summary').click();
  await expect(cites).toContainText('src/lib.rs:412 · past end of file');
  await expect(cites).toContainText('Checked when the run ended.');
});
