import { expect, test } from './fixtures/test';

// End-run coverage only. Native cursor painting requires the installed app;
// this fixture verifies retained content, controls and native placement calls.
for (const width of [640, 1280]) {
  test(`Preview mini player retains its tab at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 600 });
    await page.goto('/?mock=1');
    await expect(page.locator('.canvas-split')).toBeVisible();
    await page.evaluate(async () => {
      const module = '/src/store/preview.ts';
      (await import(/* @vite-ignore */ module)).openPreview({ kind: 'url', url: 'http://localhost:5173' });
    });
    const pane = page.getByRole('complementary', { name: 'Preview', exact: true });
    await expect(pane.getByRole('textbox', { name: 'Address' })).toHaveValue('http://localhost:5173');
    await pane.locator('.browser-pane').evaluate(el => { el.setAttribute('data-retained-test', 'yes'); });
    const opens = () => page.evaluate(() => (JSON.parse(localStorage.getItem('mockBrowserLog') ?? '[]') as {command: string}[]).filter(c => c.command === 'browser_open').length);
    const before = await opens();
    await pane.getByRole('button', { name: 'Minimize preview', exact: true }).click();
    await expect(pane).toHaveClass(/preview-mini/);
    await expect(page.locator('.canvas-split > .page')).toBeVisible();
    await expect(pane.getByRole('button', { name: 'Expand preview', exact: true })).toBeFocused();
    await expect(pane.getByRole('button', { name: 'Close preview', exact: true })).toBeVisible();
    const limit = await page.evaluate(() => Math.min(24 * Number.parseFloat(getComputedStyle(document.documentElement).fontSize), innerWidth));
    expect((await pane.boundingBox())!.width).toBeLessThanOrEqual(limit);
    await expect(pane.locator('.browser-pane')).toHaveAttribute('data-retained-test', 'yes');
    await pane.getByRole('button', { name: 'Expand preview', exact: true }).press('Enter');
    await expect(pane).not.toHaveClass(/preview-mini/);
    await expect(pane.getByRole('textbox', { name: 'Address' })).toBeVisible();
    await expect(pane.locator('.browser-pane')).toHaveAttribute('data-retained-test', 'yes');
    expect(await opens()).toBe(before);
    await pane.getByRole('button', { name: 'Minimize preview', exact: true }).click();
    await page.keyboard.press('Meta+l');
    await expect(pane).not.toHaveClass(/preview-mini/);
    await expect(pane.getByRole('textbox', { name: 'Address' })).toBeFocused();
    await pane.getByRole('button', { name: 'Minimize preview', exact: true }).click();
    await pane.getByRole('button', { name: 'Close preview', exact: true }).click();
    await expect(pane).toHaveCount(0);
    await expect(page.locator('.canvas-split > .page')).toBeVisible();
  });
}
