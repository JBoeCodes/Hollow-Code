import { expect, test, type Page } from './fixtures/test';
import { choose } from './choose';

/**
 * Overnight Spare S10: Automation windows (Settings PR 4, 11-12: on the Automations page, reached
 * from Settings → Background work → Open windows and run queue). The named
 * windows and the overnight cap, edited as a draft and saved through the
 * person's own `set_automation_windows` (the `?mock=1` IPC keeps them in
 * localStorage `mockAutomationWindows`). Loaded when the section opens, never
 * at launch.
 */

const narrow = (page: Page) => (page.viewportSize()?.width ?? 1000) <= 860;
const calls = (page: Page, command: string) => page.evaluate(c => window.__mockApiCalls?.[c] ?? 0, command);
const saved = (page: Page) => page.evaluate(() => JSON.parse(localStorage.getItem('mockAutomationWindows') ?? 'null'));

async function openAutomation(page: Page) {
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  if (narrow(page)) await choose(page.getByRole('combobox', { name: 'Settings section' }), 'background');
  else await page.locator('.settings-index').getByRole('button', { name: 'Background work', exact: true }).click();
  await page.getByRole('button', { name: 'Open windows and run queue', exact: true }).click();
  await expect(page.getByRole('list', { name: 'Automation windows' })).toBeVisible();
}

const windowGroup = (page: Page, title: RegExp) => page.getByRole('list', { name: 'Automation windows' }).getByRole('group', { name: title });

test('edits a window, saves it, and the saved hours survive a reload', async ({ page }) => {
  // The mock's one automation runs on friday-pm, so Automations names it.
  await page.addInitScript(() => localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': 'window:friday-pm' })));
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  expect(await calls(page, 'set_automation_windows')).toBe(0);
  const before = await calls(page, 'automation_windows');
  await openAutomation(page);
  // Read when the section opened, not at launch.
  expect(await calls(page, 'automation_windows')).toBeGreaterThan(before);

  await expect(page.getByRole('region', { name: 'Automation windows and the run queue' }).getByText('America/New_York')).toBeVisible();
  const friday = windowGroup(page, /^Friday afternoon/);
  await expect(friday.getByText('Fridays 15:00–18:00')).toBeVisible();
  // Built-ins keep their names: no Remove, no name field.
  await expect(friday.getByRole('button', { name: /^Remove/ })).toHaveCount(0);
  await expect(friday.getByRole('textbox', { name: 'Name automations use' })).toHaveCount(0);

  await friday.getByLabel('Starts').fill('14:00');
  await friday.getByRole('group', { name: 'Days' }).getByRole('button', { name: 'Thu' }).click();
  await expect(friday.getByRole('group', { name: 'Days' }).getByRole('button', { name: 'Thu' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('status').filter({ hasText: 'Unsaved changes.' })).toBeVisible();
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Saved. Windows run in America/New_York.' })).toBeVisible();

  const s = await saved(page);
  const fri = s.windows.find((w: { name: string }) => w.name === 'friday-pm');
  expect(fri).toEqual({ name: 'friday-pm', label: 'Friday afternoon (Thu, Fri 14:00–18:00)', days: ['thu', 'fri'], start: '14:00', end: '18:00' });
  // Untouched windows keep their saved labels.
  expect(s.windows.find((w: { name: string }) => w.name === 'nightly').label).toBe('Nightly (02:00–05:00)');
  expect(s.overnightCapUsd).toBe(40);

  await page.reload();
  await openAutomation(page);
  await expect(windowGroup(page, /^Friday afternoon/).getByLabel('Starts')).toHaveValue('14:00');
  await expect(windowGroup(page, /^Friday afternoon/).getByText('Thu, Fri 14:00–18:00')).toBeVisible();
  await expect(windowGroup(page, /^Friday afternoon/).getByText('1 automation uses this window')).toBeVisible();
  // Automations read the saved labels when the page opens.
  await page.getByRole('navigation', { name: 'Main' }).getByRole('button', { name: /^Automations/ }).click();
  await expect(page.locator('.workflow-schedule-tag').filter({ hasText: 'Friday afternoon (Thu, Fri 14:00–18:00)' }).first()).toBeVisible();
  expect((await page.evaluate(() => window.__mockApiMisses ?? [])).filter(c => /window/.test(c))).toEqual([]);
});

test('a refused save keeps the edits and says why next to the control', async ({ page }) => {
  await page.goto('/?mock=1');
  await openAutomation(page);

  // The backend's refusal lands on the window it names; the draft stays.
  await page.evaluate(() => localStorage.setItem('mockAutomationWindowsError', "Window 'nightly' needs start and end as HH:MM."));
  const nightly = windowGroup(page, /^Nightly/);
  await nightly.getByLabel('Ends').fill('06:00');
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(nightly.getByRole('alert')).toContainText('needs start and end');
  await expect(page.getByRole('status').filter({ hasText: 'Not saved: your changes are still here.' })).toBeVisible();
  await expect(nightly.getByLabel('Ends')).toHaveValue('06:00');
  expect(await saved(page)).toBeNull();

  // Fixing it (here: saving again) clears the error and saves the edit; the
  // overnight cap (set in Usage → Budgets since P1-07) stays as stored.
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Saved.' })).toBeVisible();
  await expect(nightly.getByRole('alert')).toHaveCount(0);
  const s = await saved(page);
  expect(s.overnightCapUsd).toBe(40);
  expect(s.windows.find((w: { name: string }) => w.name === 'nightly')).toMatchObject({ end: '06:00', label: 'Nightly (every day 02:00–06:00)' });
});

test('adds a custom window, names it from its label, and removes it again', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': 'window:lunch-break' })));
  await page.goto('/?mock=1');
  await openAutomation(page);
  await page.getByRole('button', { name: 'Add a window' }).click();
  const added = windowGroup(page, /^New window/);
  await added.getByRole('textbox', { name: 'Label' }).fill('Lunch break');
  const lunch = windowGroup(page, /^Lunch break/);
  await expect(lunch.getByRole('textbox', { name: 'Name automations use' })).toHaveValue('lunch-break');
  // No days: refused before it is sent.
  for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri']) await lunch.getByRole('group', { name: 'Days' }).getByRole('button', { name: day }).click();
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(lunch.getByRole('alert')).toHaveText('Pick at least one day.');
  await expect(lunch.getByRole('group', { name: 'Days' }).getByRole('button', { name: 'Mon' })).toBeFocused();
  // A label edit doesn't clear the days error.
  await lunch.getByRole('textbox', { name: 'Label' }).fill('Lunch break');
  await expect(lunch.getByRole('alert')).toHaveText('Pick at least one day.');
  await lunch.getByRole('group', { name: 'Days' }).getByRole('button', { name: 'Wed' }).click();
  await expect(lunch.getByRole('alert')).toHaveCount(0);
  await lunch.getByLabel('Starts').fill('12:00');
  await lunch.getByLabel('Ends').fill('13:00');
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Saved.' })).toBeVisible();
  expect((await saved(page)).windows.at(-1)).toEqual({ name: 'lunch-break', label: 'Lunch break (Wednesdays 12:00–13:00)', days: ['wed'], start: '12:00', end: '13:00' });
  // Saved: its name is fixed now (automations say window:lunch-break).
  await expect(lunch.getByRole('textbox', { name: 'Name automations use' })).toHaveCount(0);
  await expect(lunch.getByText('window:lunch-break', { exact: true })).toBeVisible();

  // An automation runs on it: Remove asks first, inline.
  await expect(lunch.getByText('1 automation uses this window')).toBeVisible();
  await lunch.getByRole('button', { name: 'Remove Lunch break' }).click();
  const confirm = lunch.getByRole('group', { name: 'Remove Lunch break?' });
  await expect(confirm).toContainText('An automation runs on this window; removing it stops it until it gets another window.');
  await expect(confirm.getByRole('button', { name: 'Remove anyway' })).toBeFocused();
  await confirm.getByRole('button', { name: 'Keep it' }).click();
  await expect(lunch.getByRole('textbox', { name: 'Label' })).toBeFocused();
  await lunch.getByRole('button', { name: 'Remove Lunch break' }).click();
  await lunch.getByRole('button', { name: 'Remove anyway' }).click();
  // The last row: focus goes to Add a window.
  await expect(page.getByRole('button', { name: 'Add a window' })).toBeFocused();
  await expect(page.getByRole('status').filter({ hasText: 'Lunch break is removed when you save.' })).toBeVisible();
  await page.getByRole('button', { name: 'Discard changes' }).click();
  await expect(windowGroup(page, /^Lunch break/)).toBeVisible();
  await windowGroup(page, /^Lunch break/).getByRole('button', { name: 'Remove Lunch break' }).click();
  await windowGroup(page, /^Lunch break/).getByRole('button', { name: 'Remove anyway' }).click();
  await page.getByRole('button', { name: 'Save windows' }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Saved.' })).toBeVisible();
  expect((await saved(page)).windows.map((w: { name: string }) => w.name)).toEqual(['tonight', 'weekday-evening', 'friday-pm', 'nightly']);
});

test('Settings search finds the windows link, which opens them on Automations', async ({ page }) => {
  test.skip(narrow(page), 'one layout is enough for search');
  await page.goto('/?mock=1');
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('schedule window');
  await page.locator('.settings-index').getByRole('button', { name: 'Automations, windows and the run queue are on the Automations page', exact: true }).click();
  await page.getByRole('button', { name: 'Open windows and run queue', exact: true }).click();
  await expect(page.getByRole('list', { name: 'Automation windows' })).toBeVisible();
});

test('the overnight cap is set in Usage → Budgets, which search finds too (P1-07)', async ({ page }) => {
  test.skip(narrow(page), 'one layout is enough for a link');
  await page.goto('/?mock=1');
  await openAutomation(page);
  await expect(page.getByRole('textbox', { name: 'Overnight cap' })).toHaveCount(0);
  await page.locator('.aw-cap').getByRole('button', { name: 'Set in Usage → Budgets' }).click();
  const row = page.locator('[data-cap="overnight"]');
  await expect(row).toBeVisible();
  await expect(row).toHaveClass(/settings-found/);
  await page.getByRole('searchbox', { name: 'Find a setting' }).fill('overnight cap');
  await page.locator('.settings-index').getByRole('button', { name: 'Overnight cap', exact: true }).click();
  await expect(row).toBeVisible();
});

for (const theme of ['light', 'dark'] as const) {
  test(`screenshot (${theme})`, async ({ page }) => {
    test.skip(!process.env.AW_SHOTS, 'set AW_SHOTS=<dir> to write the editor screenshots');
    await page.addInitScript(t => {
      localStorage.setItem('theme', t);
      localStorage.setItem('mockWorkflowSchedules', JSON.stringify({ 'morning-brief': 'window:new-window' }));
    }, theme);
    await page.goto('/?mock=1');
    await openAutomation(page);
    await page.getByRole('button', { name: 'Add a window' }).click();
    await page.getByRole('button', { name: 'Remove New window' }).click();
    await page.getByRole('heading', { name: 'Automation windows' }).scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${process.env.AW_SHOTS}/automation-windows-${theme}.png`, fullPage: false });
  });
}
