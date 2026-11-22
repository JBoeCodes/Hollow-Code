import { expect, test, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { settled } from './settle';

/**
 * B04 (overnight 2026-10-16): an automation whose schedule can't be read
 * never runs, and until now its row said "—" and nothing else did. The
 * backend reports `scheduleProblem` (`validate_schedule`'s error); the mock
 * sets it when `mockWorkflowSchedules` gives a fixture a schedule that isn't
 * `manual`, a window or seven cron fields.
 */

const misses = (page: Page) => page.evaluate(() => window.__mockApiMisses ?? []);

function railButton(page: Page, name: string) {
  return page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name, exact: true });
}

async function openAutomations(page: Page, schedule: string) {
  await page.addInitScript(s => localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': s })), schedule);
  await page.goto('/?mock=1');
  await railButton(page, 'Automations').click();
  const row = page.locator('.workflow-row').filter({ hasText: 'Morning Brief' });
  await expect(row).toBeVisible();
  return row;
}

test('a schedule that can’t be read says so in the row and the detail pane, with the error under Details', async ({ page }) => {
  const row = await openAutomations(page, 'not-cron');
  const foot = row.locator('.workflow-row-foot');
  await expect(foot).toHaveText('Schedule can’t be read, so this won’t run on schedule');
  await expect(foot).toHaveClass(/workflow-next-warn/);

  await row.getByRole('button', { name: 'Open Morning Brief' }).click();
  const review = page.locator('.workflow-review');
  await expect(review).toBeVisible();
  const problem = review.locator('.workflow-schedule-problem');
  await expect(problem).toHaveAttribute('role', 'alert');
  await expect(problem).toContainText('This schedule can’t be read, so the automation won’t run on schedule. Use Edit to set it again.');
  // The stored next-run time would mislead: it never comes.
  await expect(review.locator('.workflow-cover-stat dt').filter({ hasText: 'Next run' })).toHaveCount(0);
  // The raw error waits behind Details.
  const raw = problem.locator('.inline-error-detail');
  await expect(raw).toBeHidden();
  const details = problem.getByRole('button', { name: 'Details' });
  await details.click();
  await expect(details).toHaveAttribute('aria-expanded', 'true');
  await expect(raw).toHaveText('Invalid expression: Invalid cron expression.');
  // Edit remains reachable through the named options menu.
  await review.getByRole('button', { name: 'Automation options' }).click();
  await expect(page.getByRole('menuitem', { name: 'Edit', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');

  // Let the cover's entrance finish first, or axe can sample a mid-fade colour.
  await settled(review.locator('.workflow-cover'));
  const axe = await new AxeBuilder({ page }).include('.workflow-cover').withTags(['wcag2a', 'wcag2aa']).analyze();
  expect(axe.violations).toEqual([]);
  expect(await misses(page)).toEqual([]);
});

test('a readable cron, manual or window schedule shows no warning', async ({ page }) => {
  for (const schedule of ['0 0 9 * * Mon-Fri *', 'manual', 'window:nightly']) {
    const row = await openAutomations(page, schedule);
    await expect(row.locator('.workflow-row-foot')).not.toContainText('can’t be read');
    await row.getByRole('button', { name: 'Open Morning Brief' }).click();
    await expect(page.locator('.workflow-review')).toBeVisible();
    await expect(page.locator('.workflow-schedule-problem')).toHaveCount(0);
  }
});
