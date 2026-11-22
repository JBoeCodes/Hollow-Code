import { test, expect } from './fixtures/test';

test('folder and connectors has two lines and a visible settings route', async ({ page }) => {
  await page.goto('/?mock=1&demo=1');
  await page.locator('.home-project-row').filter({ hasText: 'Q3 regional operations review' }).getByRole('button').first().click();
  const button = page.getByRole('button', { name: /^Folder and connectors:/ });
  await button.click();
  const menu = page.getByRole('menu', { name: /^Folder and connectors:/ });
  await expect(menu.getByRole('menuitem')).toHaveCount(2);
  const bounds = (await menu.boundingBox())!;
  expect(bounds.x).toBeGreaterThanOrEqual(0);
  expect(bounds.x + bounds.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  expect(await page.evaluate(() => (window as unknown as { __mockApiCalls?: Record<string, number> }).__mockApiCalls?.test_connector ?? 0)).toBe(0);
  await expect(menu.getByRole('menuitem').first()).toContainText('Folder ·');
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  await expect(button).toBeFocused();
  await button.click();
  await menu.getByRole('menuitem', { name: /^Connectors ·/ }).click();
  await expect(page.getByRole('heading', { name: 'Connectors', exact: true, level: 2 })).toBeVisible();
});
