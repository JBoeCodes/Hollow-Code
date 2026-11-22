import { expect, test } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';
import { openAdvancedGroup, openGlobalSearch } from './choose';

for (const theme of ['light', 'dark'] as const) test(`palette help and shortcuts meet AA in ${theme}`, async ({ page }) => {
  await page.emulateMedia({ reducedMotion:'reduce', colorScheme:theme });
  await page.addInitScript(theme => localStorage.setItem('theme', theme), theme);
  await page.goto('/?mock=1');
  await openGlobalSearch(page);
  const palette = page.locator('[aria-label="Command palette"]').first();
  await palette.getByRole('combobox').fill('version');
  await expect(palette.locator('.palette-row').first()).toContainText('Copy build identity');
  await expect(palette.locator('.palette-row-shortcut')).toContainText('g x');
  expect((await new AxeBuilder({ page }).include('[aria-label="Command palette"]').withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()).violations).toEqual([]);
});

for (const variant of ['clean', 'dirty', 'unknown', 'unavailable'] as const) {
  test(`System preserves full ${variant} build provenance`, async ({ page }) => {
    const text = variant === 'unknown' ? 'Hollow 1.0.0 · commit unknown'
      : `Hollow 1.0.0 · commit abc1234${variant === 'dirty' ? ' (uncommitted changes)' : ''} · built 2026-10-25T12:00:00Z · source abcdef…`;
    await page.addInitScript(({ text, variant }) => {
      localStorage.setItem('mockBuildIdentity', JSON.stringify({ version:'1.0.0', shortCommit:variant === 'unknown' ? 'unknown' : 'abc1234', dirty:variant === 'dirty', text }));
      Object.defineProperty(navigator, 'clipboard', { configurable:true, value: {
        writeText:async (value:string) => { (window as unknown as { copiedBuild:string }).copiedBuild = value; },
      } });
    }, { text, variant });
    await page.goto('/?mock=1');
    if (variant === 'unavailable') await page.evaluate(() => { window.__mockFailCommandOnce = 'build_identity'; });
    await page.getByRole('button', { name:'Settings', exact:true }).click();
    await expect(page.locator('.settings-section').first()).toBeVisible();
    const section = page.getByRole('combobox', { name:'Settings section', exact:true });
    if (await section.isVisible()) {
      await section.click();
      await page.getByRole('option', { name:'Advanced', exact:true }).click();
    } else await page.getByRole('navigation', { name:'Settings sections' }).getByRole('button', { name:'Advanced', exact:true }).click();
    // Build identity sits in Advanced's closed Diagnostics group (Settings PR 3).
    await openAdvancedGroup(page, 'diagnostics');
    const copy = page.getByRole('button', { name:'Copy build identity', exact:true });
    const row = page.locator('.settings-row').filter({ has:copy });
    await expect(row).toContainText(variant === 'unavailable' ? 'Unavailable in this build.' : text);
    if (variant === 'unavailable') await expect(copy).toBeDisabled();
    else {
      await copy.click();
      await expect.poll(() => page.evaluate(() => (window as unknown as { copiedBuild:string }).copiedBuild)).toBe(text);
    }
  });
}

test('version search copies the build identity through the shared action', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
      writeText: async (text: string) => { (window as unknown as { copiedBuild: string }).copiedBuild = text; },
    } });
  });
  await page.goto('/?mock=1');
  await openGlobalSearch(page);
  const palette = page.locator('[aria-label="Command palette"]').first();
  await palette.getByRole('combobox').fill('version');
  await expect(palette.locator('.palette-row').first()).toContainText('Copy build identity');
  await palette.getByRole('combobox').press('Enter');
  await expect(page.locator('.snackbar')).toContainText('Build identity copied');
  expect(await page.evaluate(() => (window as unknown as { copiedBuild: string }).copiedBuild))
    .toBe('Hollow 0.5.3 · commit f1x7ure · built fixture · source mock…');
});
