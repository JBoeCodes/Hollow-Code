import { test, expect, type Page } from './fixtures/test';

// P1-55: the composer's Dictate button keeps one name, says its state once
// beside it, stops when the message is sent, and says why it is off.

async function openComposer(page: Page) {
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  const composer = page.getByRole('textbox', { name: 'Chat message' });
  await expect(composer).toBeVisible();
  return composer;
}

test('Dictate keeps its name while listening, says Listening beside it, and stops on send', async ({ page }) => {
  const composer = await openComposer(page);
  const mic = page.getByRole('button', { name: 'Dictate', exact: true });
  await mic.click();
  await expect(mic).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Stop dictating' })).toHaveCount(0);
  await expect(mic.getByRole('status')).toHaveCount(0);
  await expect(page.locator('.dictate-status').filter({ hasText: 'Listening' })).toHaveCount(1);
  await expect(composer).not.toHaveValue('');
  await composer.press('Meta+Enter');
  await expect(mic).toHaveAttribute('aria-pressed', 'false');
  // Nothing more lands in the emptied box.
  await page.waitForTimeout(600);
  await expect(composer).toHaveValue('');
});

test('Dictate says why it is off', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockDictationOff', '1'));
  await openComposer(page);
  const mic = page.getByRole('button', { name: 'Dictate', exact: true });
  await expect(mic).toHaveAccessibleDescription(/On-device recognition isn’t available/);
  // Through aria-describedby, not only the hover title.
  const id = await mic.getAttribute('aria-describedby');
  expect(id).toBeTruthy();
  await expect(page.locator(`[id="${id}"]`)).toHaveText(/On-device recognition isn’t available/);
});
