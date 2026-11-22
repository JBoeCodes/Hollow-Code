import { expect, test } from './fixtures/test';
import { openSettingsIcons, openSettingsSection } from './choose';

for (const theme of ['light', 'dark']) test(`app artwork follows save, reload and reset without moving the sidebar in ${theme}`, async ({ page }, info) => {
  test.skip(info.project.name === 'chromium-narrow', 'the brand header is in the full sidebar; a narrow window starts on the icon rail');
  await page.addInitScript(t => localStorage.setItem('theme', t), theme);
  await page.goto('/?mock=1');
  const mark = page.locator('.work-brand-mark .hollow-mark');
  const rail = page.getByRole('complementary', { name: 'Main navigation' });
  await expect(mark).toBeVisible();
  const stock = await mark.getAttribute('src');
  const geometry = () => page.locator('.work-brand').evaluate(el => {
    const box = (x: Element) => { const r = x.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; };
    return { header: box(el), mark: box(el.querySelector('.work-brand-mark')!), name: box(el.querySelector('.studio-brand-name')!), text: el.textContent };
  });
  const before = await geometry();
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await openSettingsSection(page, 'General');
  await openSettingsIcons(page);
  const app = page.getByRole('region', { name: 'App icon', exact: true });
  await expect(app.getByRole('button', { name: 'Upload an icon', exact: true })).toBeEnabled();
  await rail.screenshot({ path: info.outputPath(`sidebar-before-${theme}.png`) });
  await app.getByRole('button', { name: 'Upload an icon', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: 'App icon', exact: true });
  await dialog.getByLabel('Choose icon file').setInputFiles({ name: 'jb.svg', mimeType: 'image/svg+xml', buffer: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#7050ac"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>') });
  await dialog.getByRole('button', { name: 'Apply icon' }).click();
  await expect(dialog).toBeHidden();
  await expect(mark).toHaveAttribute('src', /^data:image\/png;base64,/);
  const saved = await mark.getAttribute('src');
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', saved!);
  await expect(app.locator('.custom-icon-current img')).toHaveAttribute('src', saved!);
  expect(await geometry()).toEqual(before);
  await expect(mark).toHaveAttribute('alt', '');
  await expect(mark).toHaveAttribute('aria-hidden', 'true');
  await rail.screenshot({ path: info.outputPath(`sidebar-after-${theme}.png`) });
  await page.locator('.work-brand').screenshot({ path: info.outputPath(`brand-after-${theme}.png`) });
  await page.reload();
  await expect(mark).toHaveAttribute('src', saved!);
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', saved!);
  // The app choice cannot replace the independent Main agent choice.
  expect(await page.evaluate(() => localStorage.getItem('mockCustomIcon:mainAgent'))).toBeNull();
  await page.getByRole('navigation', { name: 'Shared tools' }).getByRole('button', { name: 'Settings', exact: true }).click();
  await openSettingsSection(page, 'General');
  await openSettingsIcons(page);
  await app.getByRole('button', { name: 'Reset to Fold' }).click();
  // Resetting a custom icon deletes its original, so it asks first.
  await page.locator('.confirm-dialog').getByRole('button', { name: 'Reset to Fold', exact: true }).click();
  await expect(mark).toHaveAttribute('src', stock!);
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', '/hollow-32.png');
  expect(await geometry()).toEqual(before);
});
