import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/** Overnight A09: inline error lines that printed a raw error now say it in
 * plain words, with the raw text under "Details" (ErrorText). Two of the six
 * sites, reached on `?mock=1`: the skill reader (a failed read used to land in
 * the Markdown body as "(read failed: …)") and a run receipt's patches. The
 * failure is forced per command with `window.__mockScenario.unavailable`,
 * which the mock reads on every call. Both widths. */

async function failCommand(page: Page, command: string, message: string) {
  await page.evaluate(([c, m]) => {
    (window as unknown as { __mockScenario?: unknown }).__mockScenario = { names: [], delayMs: {}, unavailable: { [c]: m } };
  }, [command, message] as const);
}

test('a skill that can’t be read says so in plain words, with the raw error under Details', async ({ page }) => {
  // The same way in as stale-refresh.spec.ts, at either width.
  await page.goto('/?mock=1&demo=1');
  await failCommand(page, 'read_skill', 'No such file or directory (os error 2)');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
  await page.getByRole('button', { name: 'Open Library, Skills', exact: true }).click(); // Skills moved to Library in Settings PR 4
  await page.getByRole('list', { name: 'Skills' }).getByRole('button', { name: /^release-notes / }).click();

  const detail = page.getByRole('region', { name: 'Skill details' });
  const error = detail.locator('.skill-read-error');
  await expect(error).toHaveAttribute('role', 'alert');
  await expect(error).toContainText('Couldn’t read this skill: That file or folder isn’t there any more (it may have been moved or deleted). Refresh, then try again.');
  const raw = error.locator('code.inline-error-detail');
  await expect(raw).toBeHidden();
  await error.getByRole('button', { name: 'Details' }).click();
  await expect(raw).toBeVisible();
  await expect(raw).toHaveText('No such file or directory (os error 2)');
  // The error is its own line; nothing of it went into the instructions.
  await expect(detail).not.toContainText('read failed');
});

test('patches that can’t be loaded say so in plain words, with the raw error under Details', async ({ page }) => {
  await page.goto('/?mock=1');
  await failCommand(page, 'read_history_repo_changes', 'database is locked');
  await page.getByRole('complementary', { name: 'Main navigation' }).getByRole('button', { name: 'Automations', exact: true }).click();
  await page.locator('.workflow-row').filter({ hasText: 'Morning Brief' }).click();

  // The receipt with the repository block (see repo-receipt.spec.ts).
  const receipt = page.locator('.execution-receipt').filter({ has: page.locator('.execution-receipt-repo') }).first();
  await receipt.locator('summary.execution-receipt-summary').click();
  await receipt.getByRole('button', { name: 'Show patches' }).click();

  const error = receipt.locator('p.toolcall-result.error');
  await expect(error).toHaveAttribute('role', 'alert');
  await expect(error).toContainText('Patches could not be loaded: Hollow’s database was busy. Wait a moment, then try again.');
  const raw = error.locator('code.inline-error-detail');
  await expect(raw).toBeHidden();
  await error.getByRole('button', { name: 'Details' }).click();
  await expect(raw).toBeVisible();
  await expect(raw).toHaveText('database is locked');
});
