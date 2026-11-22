import { expect, test } from './fixtures/test';
import { choose } from './choose';

// The day ring, Today and Agenda were cut (overnight 11-08 S3). Signing in
// again stays on Settings → Connectors, where the Microsoft 365 row reads
// m365_status's problem (the refused stored sign-in). The test title is the
// route ledger's verified string for settings.connectors.
test('a signed-out Microsoft 365 calendar says to sign in again and opens Connectors (overnight H6)', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockBuiltins', '1');
    localStorage.setItem('mockM365', 'you@example.com');
    localStorage.setItem('mockCalendarSignIn', '1');
  });
  await page.goto('/?mock=1');
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1440) <= 860)
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Your connectors' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Agenda calendar', exact: true })).toBeVisible();
  const m365 = page.locator('#connector-hollow_m365');
  await expect(m365).toContainText('The Microsoft sign-in stopped working');
  await expect(m365.getByRole('button', { name: 'Sign in again' }).first()).toBeVisible();
});
