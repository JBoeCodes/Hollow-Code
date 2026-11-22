import { expect, test, type Page } from './fixtures/test';
test.use({ timezoneId: 'America/New_York' });
async function open(page: Page, expanded = true) {
  await page.clock.setFixedTime(new Date('2026-10-16T21:00:00Z'));
  await page.addInitScript(() => localStorage.setItem('mockAutomationWindowsZone', 'Asia/Tokyo'));
  await page.goto('/?mock=1&demo=1');
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: 'Automations', exact: true }).click();
  const clock = page.getByRole('region', { name: 'Background work windows' });
  if (expanded) await clock.locator('summary').click();
  return clock;
}
test('configured zone, midnight tail, keyboard owner routes and retained queue rules', async ({ page }) => {
  const clock = await open(page);
  await expect(clock).toContainText('2026-10-17');
  await expect(clock).toContainText('Asia/Tokyo');
  const tonight = clock.getByRole('button', { name: /^Tonight \(weekdays .+\): .*Open Automations$/ });
  await expect(tonight).toContainText('00:00–07:00 (from yesterday)');
  await tonight.focus(); await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'New automation', exact: true })).toBeVisible();
  await expect(page.getByRole('navigation', { name: 'Automations views' })).toHaveCount(0);
  const other = clock.getByRole('button', { name: /Open Automations$/ }).nth(1);
  await other.focus(); await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'New automation', exact: true })).toBeVisible();
  await expect(page.locator('details.background-work[open]')).toHaveCount(0);
  expect(await page.evaluate(() => document.scrollingElement!.scrollWidth <= innerWidth)).toBe(true);
  const box = await clock.boundingBox();
  // Chromium's transformed layout can report 110.000015px at Largest text.
  // Permit subpixel rounding while retaining the actual card-height bound.
  expect(box!.height).toBeLessThanOrEqual(190.1);
  if (page.viewportSize()!.height <= 700) expect(box!.height).toBeLessThanOrEqual(110.1);
  await page.getByRole('button', { name: 'Open Morning Brief', exact: true }).click();
  await expect(page.locator('.workflow-review')).toBeVisible();
});
test('failed window reads stay unknown and retry without suspending navigation', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockAutomationWindowsReadError', '1'));
  const clock = await open(page);
  await expect(clock).toContainText('Couldn’t read background work windows');
  await expect(clock.locator('svg')).toHaveCount(0);
  await expect(page.getByRole('heading', { name: 'Automations', exact: true })).toBeVisible();
  await page.evaluate(() => localStorage.removeItem('mockAutomationWindowsReadError'));
  await clock.getByRole('button', { name: 'Retry windows' }).click();
  await expect(clock).toContainText('Asia/Tokyo');
});
test('minute ticks preserve the focused window button', async ({ page }) => {
  await page.clock.install();
  const clock = await open(page);
  const tonight = clock.getByRole('button', { name: /^Tonight \(weekdays .+\): .*Open Automations$/ });
  await tonight.focus();
  await page.clock.runFor(61_000);
  await expect(tonight).toBeFocused();
});

test('the clock starts as one disclosure line at 900 by 600', async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 600 });
  const clock = await open(page, false);
  const disclosure = clock.locator('details');
  await expect(disclosure).not.toHaveAttribute('open', '');
  await expect(clock.getByRole('button', { name: 'Refresh windows', exact: true })).toBeHidden();
  expect((await clock.boundingBox())!.height).toBeLessThan(50);
  await clock.locator('summary').click();
  await expect(clock.getByRole('button', { name: 'Refresh windows', exact: true })).toBeVisible();
});

test('closed window disclosure announces a failed read', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockAutomationWindowsReadError', '1'));
  const clock = await open(page, false);
  await expect(clock.locator('summary').getByRole('status')).toContainText('Could not read');
  await expect(clock.locator('details')).not.toHaveAttribute('open', '');
});
