/**
 * Automation recovery (plan completion L2, 2026-10-13; review V03 and UI/UX
 * journey 4 "Delegate safely"): from a failed automation, using visible
 * controls only, read what the run did (changed files, results, checks,
 * known cost, recorded outside effects, what Hollow can't see), recover in
 * the failing run's context, and retry without repeating completed outside
 * actions. Then the one explanation of scheduled automations, the heartbeat,
 * Main agent check-ins and window jobs. Tonight was cut overnight 11-08 S4.
 *
 * A required spec: it runs at the desktop size and at the native minimum
 * window, with and without Largest text, and may not skip
 * (scripts/check-required-specs.mjs). Against src/mockApi.ts: the Morning
 * Brief's newest run failed with a 401 after writing a file;
 * `localStorage.mockOutsideEffects` picks what its receipt recorded beyond
 * its files.
 */
import { expect, test, type Page } from './fixtures/test';

const misses = (page: Page) => page.evaluate(() => (window as unknown as { __mockApiMisses?: string[] }).__mockApiMisses ?? []);
const lastRun = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockLastRunWorkflowArgs') ?? 'null'));
async function noHorizontalScroll(page: Page) {
  const [scroll, client] = await page.evaluate(() => [document.scrollingElement!.scrollWidth, document.scrollingElement!.clientWidth]);
  expect(scroll, 'no horizontal page scroll').toBeLessThanOrEqual(client);
}

async function openBrief(page: Page, effects?: string) {
  if (effects) await page.addInitScript(mode => localStorage.setItem('mockOutsideEffects', mode), effects);
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).getByRole('button', { name: 'Open Morning Brief' }).click();
  const review = page.locator('.workflow-review');
  await expect(review).toBeVisible();
  return review;
}

test('click-only: a failed run says what it did; a sign-in error never reads as nothing happened', async ({ page }) => {
  const review = await openBrief(page);
  await expect(review.locator('.workflow-status-line .ui-pill').first()).toContainText('Failed');

  // The fix stays in the failing run's context, and no longer claims the
  // run stopped before doing anything: its receipt shows a written file.
  const fix = review.getByRole('status', { name: 'What to fix' });
  await expect(fix).toContainText('could not sign in, after the run had already changed something');
  await expect(fix).not.toContainText('before doing any work');
  await expect(fix.getByRole('button', { name: 'Open Accounts' })).toBeVisible();

  const outcome = review.getByRole('region', { name: 'What happened' });
  await outcome.scrollIntoViewIfNeeded();
  await expect(outcome).toContainText('Latest run');
  await expect(outcome).toContainText('1 vault file');
  await expect(outcome).toContainText('2 files in app');
  await expect(outcome).toContainText('Checks failed');
  await expect(outcome).toContainText('$0.18');
  // A receipt from before outside effects were recorded is unknown, not none.
  await expect(outcome).toContainText('from before Hollow recorded what runs do beyond their files');
  await expect(outcome).toContainText('Not recorded for this run.');
  await expect(outcome.getByRole('button', { name: 'Run again without repeating' })).toHaveCount(0);
  await noHorizontalScroll(page);

  // Account recovery from the failing run: Settings → Accounts.
  await fix.getByRole('button', { name: 'Open Accounts' }).click();
  await expect(page.locator('#settings-accounts')).toBeVisible();
  expect(await misses(page)).toEqual([]);
});

test('click-only: outside actions are listed and the guarded retry carries the failed run', async ({ page }) => {
  const review = await openBrief(page, 'notion');
  const outcome = review.getByRole('region', { name: 'What happened' });
  await outcome.scrollIntoViewIfNeeded();
  const effects = outcome.getByRole('list', { name: 'Recorded outside effects' });
  await expect(effects).toContainText('Notion · create page “Morning brief 2026-07-07”');
  await expect(effects.locator('[data-outcome="done"]')).toContainText('won’t repeat');
  await expect(effects.locator('[data-outcome="uncertain"]')).toContainText('may have happened');
  await expect(outcome.getByRole('list', { name: 'Calls Hollow can’t see into' })).toContainText('2 shell commands');
  await expect(outcome).toContainText('can’t see what 2 calls did');

  const retry = outcome.getByRole('group', { name: 'Running it again' });
  await expect(retry).toContainText('Hollow refuses the identical call for the one it runs itself');
  await expect(retry).toContainText('check the other one before running again');
  await retry.getByRole('button', { name: 'Run again without repeating' }).click();
  await expect.poll(() => lastRun(page)).toEqual({ slug: 'morning-brief', retryOf: 'run-workflow-1' });

  // The plain Run now never carries a retry.
  await review.locator('.workflow-output-actions').getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => lastRun(page)).toEqual({ slug: 'morning-brief' });
  await noHorizontalScroll(page);
  expect(await misses(page)).toEqual([]);
});

test('click-only: a run that only changed files offers History before a plain run', async ({ page }) => {
  const review = await openBrief(page, 'none');
  const outcome = review.getByRole('region', { name: 'What happened' });
  await outcome.scrollIntoViewIfNeeded();
  await expect(outcome).toContainText('Nothing recorded.');
  const retry = outcome.getByRole('group', { name: 'Running it again' });
  await expect(retry).toContainText('starts from the files as they are now');
  await expect(retry.getByRole('button', { name: 'Run again without repeating' })).toHaveCount(0);
  await retry.getByRole('button', { name: 'Open in History' }).click();
  await expect(page.locator('.library-reader .history-recovery')).toBeVisible();
  expect(await misses(page)).toEqual([]);
});

test('click-only: one explanation of what runs in the background, with a route to each', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  const explainer = page.locator('details.background-work:not(.workflow-windows)');
  await explainer.locator('summary').click();
  for (const name of ['Scheduled automations', 'Silent heartbeat', 'COO check-ins', 'Window jobs']) {
    await expect(explainer.getByRole('heading', { name, exact: true })).toBeVisible();
  }
  await expect(explainer.getByRole('heading', { name: 'Tonight', exact: true })).toHaveCount(0);
  const heartbeat = explainer.locator('[data-system="heartbeat"]');
  await expect(heartbeat).toContainText('What it may do');
  await expect(heartbeat).toContainText('drafts/heartbeat/');
  await expect(page.getByRole('navigation', { name: 'Automations views' })).toHaveCount(0);
  await noHorizontalScroll(page);
  expect(await misses(page)).toEqual([]);
  // (Settings → Background work reads Main agent settings the browser mock
  // doesn't serve yet; only the route is checked past this point.)
  await explainer.locator('[data-system="mainAgent"]').getByRole('button', { name: 'Settings → Background work → COO' }).click();
  await expect(page.locator('#settings-background')).toBeVisible();
});
