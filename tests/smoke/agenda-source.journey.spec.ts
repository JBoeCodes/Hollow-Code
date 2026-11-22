import { expect, test } from './fixtures/test';
import { choose, optionLabels } from './choose';

async function openCalendarSettings(page: import('@playwright/test').Page) {
  await page.getByLabel('Main navigation').getByRole('button', { name: /^Settings/ }).first().click();
  if ((page.viewportSize()?.width ?? 1440) <= 860)
    await choose(page.getByRole('combobox', { name: 'Settings section' }), 'connectors');
  else await page.getByRole('navigation', { name: 'Settings sections' }).getByRole('button', { name: 'Connectors', exact: true }).click();
}

// P4-26 click-only journeys: all IPC is the browser mock. No EventKit permission request or live
// account read belongs in this regression.
test('Agenda has one source choice and Off survives a reload', async ({ page }) => {
  await page.goto('/?mock=1');
  await openCalendarSettings(page);
  const source = page.getByRole('combobox', { name: 'Agenda calendar', exact: true });
  await expect(source).toBeVisible();
  expect(await optionLabels(source)).toEqual(['Calendars on this Mac', 'Microsoft 365', 'Off']);
  await choose(source, { label: 'Off' });
  await expect(source).toContainText('Off');
  await page.reload();
  await openCalendarSettings(page);
  await expect(source).toContainText('Off');
});

test('Mac calendar selection distinguishes all, none and unavailable calendars', async ({ page }) => {
  await page.goto('/?mock=1');
  await openCalendarSettings(page);
  await choose(page.getByRole('combobox', { name: 'Agenda calendar', exact: true }), 'mac');
  // The calendar picks are a sheet (Settings PR 5b), opened from the row.
  const choose_ = page.getByRole('button', { name: 'Choose calendars…', exact: true });
  await choose_.click();
  const all = page.getByRole('checkbox', { name: 'All calendars, including new ones' });
  await expect(all).toBeChecked();
  await all.uncheck();
  await expect(page.getByRole('checkbox', { name: 'Work · Example account' })).not.toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'Personal · On My Mac' })).not.toBeChecked();
  await page.getByRole('checkbox', { name: 'Work · Example account' }).check();
  await page.reload();
  await openCalendarSettings(page);
  await choose_.click();
  await expect(page.getByRole('checkbox', { name: 'Work · Example account' })).toBeChecked();
  await expect(all).not.toBeChecked();
  await page.keyboard.press('Escape');
  await expect(choose_).toBeFocused();
  await page.evaluate(() => localStorage.setItem('mockAgendaSource', JSON.stringify({ source: 'mac', calendarIds: ['missing'] })));
  await page.getByRole('button', { name: 'Refresh calendars' }).click();
  await choose_.click();
  const missing = page.getByRole('button', { name: 'Remove unavailable calendar 1' });
  await expect(missing).toBeVisible();
  await missing.click();
  await expect(missing).toHaveCount(0);
});

test('Mac denial has a System Settings route and refresh recovers without another source choice', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => localStorage.setItem('mockMacCalendarPermission', 'denied'));
  await openCalendarSettings(page);
  await choose(page.getByRole('combobox', { name: 'Agenda calendar', exact: true }), 'mac');
  await expect(page.getByText('Calendar access is denied.', { exact: false })).toBeVisible();
  await page.getByRole('button', { name: 'Open System Settings' }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockCalendarSettingsOpened'))).toBe('true');
  await page.evaluate(() => localStorage.setItem('mockMacCalendarPermission', 'granted'));
  await page.getByRole('button', { name: 'Refresh calendars' }).click();
  await page.getByRole('button', { name: 'Choose calendars…', exact: true }).click();
  await expect(page.getByRole('checkbox', { name: 'Work · Example account' })).toBeVisible();
});

test('Unanswered Mac access has an explicit retry and refresh never requests it', async ({ page }) => {
  await page.goto('/?mock=1');
  await page.evaluate(() => {
    localStorage.setItem('mockAgendaSource', JSON.stringify({ source: 'mac', calendarIds: null }));
    localStorage.setItem('mockMacCalendarPermission', 'notDetermined');
  });
  await openCalendarSettings(page);
  await page.getByRole('button', { name: 'Refresh calendars' }).click();
  expect(await page.evaluate(() => localStorage.getItem('mockCalendarAccessRequests'))).toBeNull();
  await page.getByRole('button', { name: 'Allow calendar access' }).click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem('mockCalendarAccessRequests'))).toBe('1');
  await page.getByRole('button', { name: 'Refresh calendars' }).click();
  expect(await page.evaluate(() => localStorage.getItem('mockCalendarAccessRequests'))).toBe('1');
});
