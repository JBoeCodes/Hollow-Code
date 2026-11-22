// Settings, Sources, Task tracker (overhaul A14, board S2), against the
// browser mock (`src/mockTracker.ts`, seeded through localStorage
// `mockTracker`). Written in lane 3 of the COO rebuild without running it;
// the one full browser run on final main runs it.
import { expect, test, type Page } from './fixtures/test';
import { choose, openSettingsSection } from './choose';

async function openConnections(page: Page) {
  await page.getByRole('button', { name: /Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1000) <= 860) {
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'sources');
  } else {
    await page.locator('.settings-index').getByRole('button', { name: 'Sources', exact: true }).click();
  }
}

const seed = (page: Page, value: Record<string, unknown>) =>
  page.addInitScript(v => localStorage.setItem('mockTracker', JSON.stringify(v)), value);

test('a set-up tracker shows its team, key, sync times, requests left and Sync now', async ({ page }) => {
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await expect(row).toBeVisible();
  await expect(row.getByRole('status').first()).toHaveText('Linear is on and syncing Engineering (ENG).');
  await expect(row).toContainText('Engineering (ENG)');
  await expect(row).toContainText('Saved in Hollow’s Keychain bundle.');
  await expect(row.locator('.task-tracker-sync')).toContainText('Last sync');
  await expect(row.locator('.task-tracker-sync')).toContainText('ago');
  await expect(row.locator('.task-tracker-sync')).toContainText('2,431 of 2,500 requests left this hour');
  // Only the one allowed team: no team picker, and no button to change it.
  await expect(row.getByRole('combobox', { name: 'Team' })).toHaveCount(0);
  await expect(row.getByRole('button', { name: /^Use Engineering/ })).toHaveCount(0);
  await row.getByRole('button', { name: 'Sync now' }).click();
  await expect(row.locator('.task-tracker-note')).toHaveText('Synced: 0 pulled, 0 sent.');
  // The Service health table was cut (Settings PR 6, C12): the tracker's state is its own row's, above.
  await openSettingsSection(page, 'Connectors');
  await expect(page.getByRole('table', { name: 'Service health by capability' })).toHaveCount(0);
});

test('the interval saves 1 to 60 minutes', async ({ page }) => {
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await choose(row.getByRole('combobox', { name: 'Sync interval' }), '10');
  await expect(row.getByRole('combobox', { name: 'Sync interval' })).toContainText('10 minutes');
  await page.reload();
  await openConnections(page);
  await expect(page.locator('.task-tracker-row').getByRole('combobox', { name: 'Sync interval' })).toContainText('10 minutes');
});

test('a missing key leads, and saving one clears it; Sync now waits for setup', async ({ page }) => {
  await seed(page, { key: 'missing' });
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await expect(row.getByRole('status').first()).toHaveText('Linear is chosen but not set up: Save an API key.');
  await expect(row.getByRole('button', { name: 'Sync now' })).toBeDisabled();
  await expect(row.getByRole('button', { name: 'Remove key' })).toHaveCount(0);
  await row.getByRole('textbox', { name: 'Linear API key' }).fill('lin_api_test');
  await row.getByRole('button', { name: 'Save key' }).click();
  await expect(row.locator('.task-tracker-note')).toHaveText('Key saved in Hollow’s Keychain bundle.');
  await expect(row.getByRole('textbox', { name: 'Linear API key' })).toHaveValue('');
  await expect(row.getByRole('button', { name: 'Sync now' })).toBeEnabled();
});

test('Use the key saved in Keychain copies it; Remove key asks first', async ({ page }) => {
  await seed(page, { key: 'missing' });
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await row.getByRole('button', { name: 'Use the key saved in Keychain' }).click();
  await expect(row.locator('.task-tracker-note')).toHaveText('Key copied from Keychain.');
  await row.getByRole('button', { name: 'Remove key' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Remove the Linear key?');
  await dialog.getByRole('button', { name: 'Remove key' }).click();
  await expect(row).toContainText('No key saved.');
});

test('a wrong team offers only the Engineering team', async ({ page }) => {
  await seed(page, { teamId: '11111111-2222-4333-8444-555555555555', teamKey: 'LP' });
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await expect(row).toContainText('team LP');
  await row.getByRole('button', { name: 'Use Engineering (ENG)' }).click();
  await expect(row).toContainText('Engineering (ENG)');
  await expect(row.getByRole('button', { name: /^Use Engineering/ })).toHaveCount(0);
});

test('off: the tracker can be chosen, and an error says what failed', async ({ page }) => {
  await seed(page, { provider: null });
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await expect(row.getByRole('status').first()).toHaveText('Off. Tasks stay local until you choose a tracker.');
  await choose(row.getByRole('combobox', { name: 'Task tracker' }), 'linear');
  await expect(row).toContainText('Engineering (ENG)');
});

test('the last error shows on the row', async ({ page }) => {
  await seed(page, { error: 'Linear refused the key (401).' });
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  await expect(row.getByRole('status').first()).toContainText('The last Linear pass failed: Linear refused the key (401).');
  await expect(row.locator('.task-tracker-error')).toContainText('Linear refused the key (401).');
});

test('Find a setting finds the Task tracker row', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: /Settings/ }).first().click();
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('sync interval');
  await expect(page.locator('.settings-index')).toContainText('Task tracker');
});

// Two teams: the shared Engineering and the person's own Personal (PER).
// Written without a local run; the gate runs it.
test('Your own team: Add Personal saves the fixed HOL team, and the row then names it', async ({ page }) => {
  await page.goto('/?mock=1');
  await openConnections(page);
  const row = page.locator('.task-tracker-row');
  const own = row.locator('[data-team-role="personal"]');
  await expect(own).toContainText('Your own team');
  await expect(own).toContainText('Not added.');
  // One button and no free-text team field or team picker.
  await expect(row.getByRole('textbox', { name: /team/i })).toHaveCount(0);
  await expect(row.getByRole('combobox', { name: /team/i })).toHaveCount(0);
  await own.getByRole('button', { name: 'Add Personal' }).click();
  await expect(own).toContainText('Personal (PER)');
  await expect(own.getByRole('button', { name: 'Add Personal' })).toHaveCount(0);
  await expect(row.locator('.task-tracker-note')).toContainText('Personal added');
  // The shared team row is unchanged.
  await expect(row).toContainText('Engineering (ENG)');
  await page.reload();
  await openConnections(page);
  await expect(page.locator('.task-tracker-row [data-team-role="personal"]')).toContainText('Personal (PER)');
});

test('Your own team already added shows Personal (PER) and no Add button', async ({ page }) => {
  await seed(page, { personalTeamId: '22222222-2222-4222-8222-222222222222', personalTeamKey: 'PER' });
  await page.goto('/?mock=1');
  await openConnections(page);
  const own = page.locator('.task-tracker-row [data-team-role="personal"]');
  await expect(own).toContainText('Personal (PER)');
  await expect(own.getByRole('button')).toHaveCount(0);
});
