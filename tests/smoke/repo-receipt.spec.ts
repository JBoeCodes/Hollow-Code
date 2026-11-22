import { expect, test, type Page } from './fixtures/test';

/** Paperclip PR 16: a workflow receipt lists the run's repository changes
 * and loads their patches once, on demand, from the run's sidecar. The mock
 * morning-brief receipt carries a linked project `app` with a modify and an
 * untracked create; `read_history_repo_changes` answers for that run only. */

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

test('workflow receipt shows repository changes and loads patches', async ({ page }) => {
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();

  const receipt = page
    .locator('.execution-receipt')
    // Anchor on the repository block, which stays mounted (the Show patches
    // button is removed once the patches load). The details element starts
    // closed, so match by class: a role query skips hidden elements.
    .filter({ has: page.locator('.execution-receipt-repo') })
    .first();
  await expect(receipt.locator('summary.execution-receipt-summary')).toContainText('2 repo changes');
  await receipt.locator('summary.execution-receipt-summary').click();
  await expect(receipt.locator('.toolcall-section-label', { hasText: 'Repository changes' })).toBeVisible();
  const repo = receipt.locator('.execution-receipt-repo');
  await expect(repo).toContainText('app');
  await expect(repo).toContainText('src/lib.rs');
  await expect(repo).toContainText('notes/todo.md');
  await expect(repo).toContainText('+3/−1');
  await expect(repo).toContainText('filesystem observation');

  const calls = () =>
    page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.read_history_repo_changes ?? 0);
  expect(await calls()).toBe(0);
  await receipt.getByRole('button', { name: 'Show patches' }).click();
  const patches = receipt.locator('pre.execution-receipt-patch');
  await expect(patches).toHaveCount(2);
  await expect(patches.first()).toContainText('+fn brief() {}');
  await expect(receipt.getByRole('button', { name: 'Show patches' })).toHaveCount(0);
  expect(await calls()).toBe(1);
});
