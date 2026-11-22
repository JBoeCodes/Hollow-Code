import { test, expect } from './fixtures/test';
import { chooseNew } from './choose';

test('project creation stays in one bounded dialog and returns focus on cancel', async ({ page }, info) => {
  await page.goto('/?mock=1');
  await chooseNew(page, 'New project');
  const dialog = page.getByRole('dialog', { name: 'New project', exact: true });
  const name = dialog.getByRole('textbox', { name: 'Project name' });
  await expect(name).toBeFocused();
  await expect(dialog.locator('.project-create-options')).not.toHaveAttribute('open');
  await name.fill('Focused project');
  const bounds = await dialog.boundingBox();
  const size = page.viewportSize()!;
  expect(bounds).not.toBeNull();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(size.width);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(size.height);
  // Tabbing cycles within the dialog, including its optional-settings disclosure.
  for (let i = 0; i < 9; i++) {
    await page.keyboard.press('Tab');
    expect(await dialog.evaluate(el => el.contains(document.activeElement))).toBe(true);
  }
  await page.screenshot({ path: info.outputPath('project-dialog.png') });
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  expect(await page.evaluate(() => window.__mockApiCalls?.create_workspace ?? 0)).toBe(0);
});
