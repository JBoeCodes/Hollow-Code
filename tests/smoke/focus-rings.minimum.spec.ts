import { expect, test } from './fixtures/test';
import { navigateWaiting } from './waiting';

async function ring(button: import('@playwright/test').Locator) {
  await button.page().keyboard.press('Tab');
  await button.focus();
  await expect(button).toBeFocused();
  await expect(button).toHaveCSS('outline-style', 'solid');
  await expect(button).toHaveCSS('outline-width', '2px');
}

test('a selected Waiting filter keeps a solid keyboard ring', async ({ page }) => {
  await page.goto('/?mock=1');
  await navigateWaiting(page);
  await expect(page.getByRole('navigation', { name: 'Review views' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Incoming', exact: true })).toHaveCount(0);
  const tab = page.getByRole('navigation', { name: 'Waiting filters' }).getByRole('button', { name: /^All\b/ });
  await expect(tab).toHaveAttribute('aria-pressed', 'true');
  await ring(tab);
  await expect(tab).toHaveCSS('outline-offset', '-2px');
});

test('automation quick actions are visible at rest and at least 24px', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  const quick = page.locator('.row-quick-btn').first();
  await expect(quick).toBeVisible();
  await page.mouse.move(0, 0);
  await expect(quick).toHaveCSS('opacity', '0.5');
  const bounds = await quick.boundingBox();
  expect(bounds!.width).toBeGreaterThanOrEqual(24);
  expect(bounds!.height).toBeGreaterThanOrEqual(24);
});
