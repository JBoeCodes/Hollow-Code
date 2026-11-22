import { expect, test, type Page } from './fixtures/test';
import { openSettingsSection } from './choose';

/**
 * Settings, Sources, Context, Personal: "Set it up for me" (CX18) is a card
 * only. With no personal remote (the mock's default) the row offers it; the
 * card shows the repo name, Private, the one `gh repo create` line with Copy,
 * the address to connect (a placeholder login: the status carries none) and
 * what Connect does. Opening, editing and closing it calls no command that
 * writes, and Close returns focus to the row's button.
 */

const WRITES = /^(configure_|set_|write_|save_|create_|delete_|context_sync_now|context_install_check|context_remove_check|context_personal_local_only|mac_context_connect)/;

async function openContext(page: Page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
      writeText: async (value: string) => { (window as unknown as { copiedSetup: string }).copiedSetup = value; },
    } });
  });
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await expect(page.locator('.settings-page')).toBeVisible();
  await openSettingsSection(page, 'Sources');
  const fold = page.locator('details#settings-layers');
  if (await fold.getAttribute('open') === null) await fold.locator(':scope > summary').click();
  const row = fold.locator('[data-layer="personal"]');
  await expect(row).toBeVisible();
  return row;
}

const writes = (page: Page) => page.evaluate(re => Object.entries(window.__mockApiCalls ?? {})
  .filter(([name]) => new RegExp(re).test(name)).map(([name, n]) => `${name}:${n}`).sort(), WRITES.source);

test('Set it up for me opens a card that creates nothing', async ({ page }) => {
  const row = await openContext(page);
  const before = await writes(page);
  const opener = row.getByRole('button', { name: 'Set it up for me', exact: true });
  await expect(opener).toHaveAttribute('aria-expanded', 'false');
  await opener.click();
  await expect(opener).toHaveAttribute('aria-expanded', 'true');

  const card = row.getByRole('region', { name: 'Set up a private repo' });
  await expect(card).toBeVisible();
  await expect(card).toContainText('Hollow doesn’t create the repo; you run this once.');
  await expect(card).toContainText('Private, always');
  const name = card.getByLabel('Repo name', { exact: true });
  await expect(name).toBeFocused();
  await expect(name).toHaveValue('personal-context');
  await expect(card.locator('[data-setup-command]')).toHaveText('gh repo create personal-context --private');
  await expect(card.locator('[data-setup-url]')).toHaveText('git@github.com:<your-github-login>/personal-context.git');
  await expect(card).toContainText('Connect saves the address and turns sync on.');

  // The name is checked, and the command follows it.
  await name.fill('my context');
  await expect(card).toContainText('Letters, digits, dots, dashes and underscores only.');
  await expect(card.getByRole('button', { name: 'Copy command' })).toBeDisabled();
  await name.fill('my.context_2');
  await expect(card.locator('[data-setup-command]')).toHaveText('gh repo create my.context_2 --private');

  await card.getByRole('button', { name: 'Copy command' }).click();
  await expect(card.getByRole('status')).toHaveText('Command copied. Paste it into Terminal.');
  expect(await page.evaluate(() => (window as unknown as { copiedSetup?: string }).copiedSetup)).toBe('gh repo create my.context_2 --private');

  // No login known: the Connect field is left alone.
  await expect(row.getByLabel('Personal context repo URL')).toHaveValue('');

  await card.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect(opener).toBeFocused();
  await expect(opener).toHaveAttribute('aria-expanded', 'false');
  expect(await writes(page)).toEqual(before);
});

test('Escape closes the card and focus returns to its button', async ({ page }) => {
  const row = await openContext(page);
  const opener = row.getByRole('button', { name: 'Set it up for me', exact: true });
  await opener.click();
  await expect(row.getByLabel('Repo name', { exact: true })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(row.getByRole('region', { name: 'Set up a private repo' })).toHaveCount(0);
  await expect(opener).toBeFocused();
});

test('a connected personal repo offers no setup card', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockContextLayers', 'connected'));
  const row = await openContext(page);
  await expect(row.getByRole('button', { name: 'Sync now' })).toBeVisible();
  await expect(row.getByRole('button', { name: 'Set it up for me', exact: true })).toHaveCount(0);
});
