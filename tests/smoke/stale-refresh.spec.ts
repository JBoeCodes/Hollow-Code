/**
 * X21 (2026-10-14): a refresh that fails must not look current. Lists read
 * through `preserveDuringRefetch` keep the last good value when a refetch
 * fails; the surface now says "Couldn’t refresh — showing what was here
 * before" with Retry (components/RefreshNotice.tsx), and a Retry that works
 * clears it. The refetch failure is forced with the mock's one-shot
 * `window.__mockFailCommandOnce`. Clicks only.
 */
import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

const STALE = 'Couldn’t refresh — showing what was here before.';
const failNext = (page: Page, command: string) =>
  page.evaluate(c => { (window as unknown as { __mockFailCommandOnce?: string }).__mockFailCommandOnce = c; }, command);
const calls = (page: Page, command: string) =>
  page.evaluate(c => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.[c] ?? 0, command);

test('Automations: a failed list refresh is marked, and Retry clears it', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Automations', exact: true }).click();
  const row = page.locator('.workflow-row').filter({ hasText: 'Morning Brief' });
  await expect(row).toBeVisible();
  const notice = page.getByRole('status', { name: 'Couldn’t refresh automations' });
  await expect(notice).toHaveCount(0);

  // Run now re-reads the list after the run; that read fails once.
  const before = await calls(page, 'list_workflows');
  await failNext(page, 'list_workflows');
  await row.hover();
  await row.getByRole('button', { name: 'Run Morning Brief now', exact: true }).click();
  // The row's Run now asks first (P0-13).
  await page.getByRole('dialog', { name: 'Run “Morning Brief” now?' }).getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => calls(page, 'list_workflows')).toBeGreaterThan(before);
  await expect(notice).toBeVisible();
  await expect(notice).toHaveText(new RegExp(STALE));
  // The last good list is still there, now marked.
  await expect(row).toBeVisible();

  await notice.getByRole('button', { name: 'Retry' }).click();
  await expect(notice).toHaveCount(0);
  await expect(row).toBeVisible();
});

test('Library → Skills: a failed list refresh is marked, and Retry brings the new skill in', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click(); // Skills moved to Library in Settings PR 4
  const list = page.getByRole('list', { name: 'Skills' });
  await expect(list.getByRole('button', { name: /^release-notes / })).toBeVisible();
  const notice = page.getByRole('status', { name: 'Couldn’t refresh skills' });
  await expect(notice).toHaveCount(0);

  // Creating a skill re-reads the list; that read fails once, so the new
  // skill is missing from a list that would otherwise look current.
  await page.getByRole('button', { name: 'New skill' }).click();
  await page.getByPlaceholder('my-skill').fill('stale-check');
  await failNext(page, 'list_skills');
  await page.getByRole('button', { name: 'Create skill' }).click();
  await expect(notice).toBeVisible();
  await expect(notice).toHaveText(new RegExp(STALE));
  await expect(list.getByRole('button', { name: /^release-notes / })).toBeVisible();
  await expect(list.getByRole('button', { name: /^stale-check / })).toHaveCount(0);

  await notice.getByRole('button', { name: 'Retry' }).click();
  await expect(notice).toHaveCount(0);
  await expect(list.getByRole('button', { name: /^stale-check / })).toBeVisible();
});
