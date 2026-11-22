import { test, expect, type Page } from './fixtures/test';
import { showSettingsPart } from './settingsPart';

/**
 * Self-hosted models Q4 (2026-11-03): Settings → Models → Self-hosted models
 * by clicks alone. Add a self-hosted model, Use the Qwen on Cloud Run preset,
 * Save; Test says it is starting the model (the row's switch, Edit and
 * Remove wait meanwhile), then its health and first byte times; the switch
 * turns it off; Remove asks first. Row buttons are named for their model
 * (Test Qwen3.8 27B). The mock keeps the list in memory, as the app keeps
 * `self-hosted-models.json`.
 */
test.skip(({ viewport }) => (viewport?.width ?? 1000) <= 860, 'the desktop rail and Settings index');

const calls = (page: Page, name: string) => page.evaluate(n => window.__mockApiCalls?.[n] ?? 0, name);

async function openPart(page: Page) {
  await page.goto('/?mock=1');
  await expect(page.getByRole('navigation', { name: 'Main', exact: true }).getByRole('button', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.locator('.settings-index').getByRole('button', { name: /^Models/ }).click();
  await showSettingsPart(page, 'selfhosted');
  return page.locator('#settings-selfhosted');
}

test('add the Qwen preset, test it, switch it off and remove it, all by clicks', async ({ page }) => {
  const part = await openPart(page);
  await expect(part.locator('.settings-part-head .settings-subhead-title')).toHaveText('Self-hosted models');
  await expect(part).toContainText('No self-hosted models yet.');

  await part.getByRole('button', { name: 'Add a self-hosted model', exact: true }).click();
  // The form is a sheet (Settings PR 5), outside the part.
  const form = page.getByRole('group', { name: 'Add a self-hosted model' });
  // Nothing typed: the form says what is missing and saves nothing.
  await form.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(form).toContainText('The context window must be a whole number of tokens.');
  expect(await calls(page, 'save_self_hosted_model')).toBe(0);

  await form.getByRole('button', { name: 'Use the Qwen on Cloud Run preset', exact: true }).click();
  await expect(form.getByLabel('Address', { exact: true })).toHaveValue('https://qwen-27b-x6jmsblvea-uc.a.run.app');
  await expect(form.getByLabel('Served model', { exact: true })).toHaveValue('qwen3.8-27b');
  await expect(form.getByLabel('Arguments', { exact: true })).toHaveValue('auth print-identity-token');
  await form.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(form).toBeHidden();

  const row = part.locator('[data-self-hosted="qwen"]');
  await expect(row).toContainText('Qwen3.8 27B');
  await expect(row).toContainText('qwen3.8-27b · 131,072 tokens · token from gcloud');

  await row.getByRole('button', { name: 'Test Qwen3.8 27B', exact: true }).click();
  await expect(row).toContainText('Starting the model, this can take about 5 minutes.');
  // While its Test runs the row's switch, Edit and Remove wait.
  await expect(row.getByRole('switch', { name: 'Qwen3.8 27B on' })).toBeDisabled();
  await expect(row.getByRole('button', { name: 'Edit Qwen3.8 27B', exact: true })).toBeDisabled();
  await expect(row.getByRole('button', { name: 'Remove Qwen3.8 27B', exact: true })).toBeDisabled();
  await expect(row).toContainText('Passed');
  await expect(row).toContainText('Health check 200 in 4.2 s.');
  await expect(row).toContainText('First byte 200 in 812 ms.');

  const on = row.getByRole('switch', { name: 'Qwen3.8 27B on' });
  await expect(on).toHaveAttribute('aria-checked', 'true');
  await on.click();
  await expect(row.getByRole('switch', { name: 'Qwen3.8 27B on' })).toHaveAttribute('aria-checked', 'false');

  // Edit keeps the id fixed; while the form is open the row's switch and
  // Edit wait, so the form's switch is never a stale copy.
  await row.getByRole('button', { name: 'Edit Qwen3.8 27B', exact: true }).click();
  const edit = page.getByRole('group', { name: 'Edit Qwen3.8 27B' });
  await expect(edit.getByLabel('Id', { exact: true })).toBeDisabled();
  await expect(edit.getByRole('switch', { name: 'Use this model', exact: true })).toHaveAttribute('aria-checked', 'false');
  await expect(row.getByRole('switch', { name: 'Qwen3.8 27B on' })).toBeDisabled();
  await expect(row.getByRole('button', { name: 'Edit Qwen3.8 27B', exact: true })).toBeDisabled();
  await edit.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(edit).toBeHidden();

  await row.getByRole('button', { name: 'Remove Qwen3.8 27B', exact: true }).click();
  const confirm = page.getByRole('dialog', { name: 'Remove Qwen3.8 27B?' });
  await confirm.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(confirm).toBeHidden();
  expect(await calls(page, 'remove_self_hosted_model')).toBe(0);
  await row.getByRole('button', { name: 'Remove Qwen3.8 27B', exact: true }).click();
  await confirm.getByRole('button', { name: 'Remove model', exact: true }).click();
  await expect(part.locator('[data-self-hosted="qwen"]')).toHaveCount(0);
  await expect(part).toContainText('No self-hosted models yet.');
});

test('the part’s id opens it from a link, on its heading', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.evaluate(async () => (await import('/src/store/runtime.ts')).requestOpenSettingsSection('selfhosted'));
  const heading = page.locator('#settings-selfhosted > .settings-part-head .settings-subhead-title');
  await expect(heading).toHaveText('Self-hosted models');
  await expect(heading).toBeFocused();
});
