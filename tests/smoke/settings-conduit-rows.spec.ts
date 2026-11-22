import { expect, test, type Locator, type Page } from './fixtures/test';
import { openSettingsSection } from './choose';

/**
 * Overnight 11-07 (lane D3): a Settings row led by a status conduit (the Plaud
 * recordings row; the Notion import row went with Notion sync, 11-10) has a
 * bar, a label and an actions block.
 * `.settings-row` is a two-track grid, so for weeks the bar took the label's
 * track and pushed the label to the right, with its buttons wrapped underneath;
 * no spec looked at where things sit. Geometry only: the bar beside the label,
 * the actions to the right of it on the same row in a wide window, and under
 * the label (the bar's track kept narrow) in a narrow one.
 */
async function box(locator: Locator) {
  const b = await locator.boundingBox();
  if (!b) throw new Error('no box');
  return b;
}

async function openConnections(page: Page, section = 'Connectors') {
  await page.goto('/?mock=1');
  await page.getByRole('navigation', { name: 'Main' }).waitFor();
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  await openSettingsSection(page, section);
}

// The Task tracker row (lane 3) keeps its controls under its label and has no
// actions block, so the only row left is the Plaud recordings one.
for (const row of ['.settings-row:has(> .settings-conduit):not(.task-tracker-row)']) {
  test(`wide: the conduit sits beside the label and the actions on the right (${row})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await openConnections(page);
    const r = page.locator(row).first();
    await r.scrollIntoViewIfNeeded();
    await expect(r).toBeVisible();
    const bar = await box(r.locator(':scope > .settings-conduit'));
    const label = await box(r.locator(':scope > .settings-row-label'));
    const actions = await box(r.locator(':scope > :last-child'));
    // The label starts right after the bar (not a column away).
    expect(label.x).toBeGreaterThanOrEqual(bar.x + bar.width - 1);
    expect(label.x - (bar.x + bar.width)).toBeLessThan(24);
    // The actions are to the right of the label and start on the label's row.
    expect(actions.x).toBeGreaterThanOrEqual(label.x + 120);
    expect(actions.y).toBeLessThan(label.y + label.height);
  });

  test(`narrow: the actions drop under the label and the bar stays thin (${row})`, async ({ page }) => {
    await page.setViewportSize({ width: 820, height: 1000 });
    await openConnections(page);
    const r = page.locator(row).first();
    await r.scrollIntoViewIfNeeded();
    await expect(r).toBeVisible();
    const bar = await box(r.locator(':scope > .settings-conduit'));
    const label = await box(r.locator(':scope > .settings-row-label'));
    const actions = await box(r.locator(':scope > :last-child'));
    expect(bar.width).toBeLessThan(8);
    expect(label.x).toBeGreaterThanOrEqual(bar.x + bar.width - 1);
    expect(label.x - (bar.x + bar.width)).toBeLessThan(24);
    // Under the label, starting at its left edge (the second track).
    expect(actions.y).toBeGreaterThanOrEqual(label.y + label.height - 1);
    expect(Math.abs(actions.x - label.x)).toBeLessThan(2);
  });
}

// The same grid, two more rows written for the flex row `.settings-row` used to be (found by the audit that
// lists every row with more than two children): Remote runner's Set up again and Turn off, and the iPhone
// app's key and backend forms.
test('Remote runner: Set up again and Turn off sit together at the right of its text', async ({ page }) => {
  await page.addInitScript(() => { localStorage.setItem('mockRemoteRunner', 'runner-host'); });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openConnections(page);
  await openSettingsSection(page, 'Devices'); // the runner moved from Models to Devices (Settings S5b)
  const row = page.locator('.settings-row.remote-runner');
  await row.scrollIntoViewIfNeeded();
  const label = await box(row.locator(':scope > .settings-row-label'));
  const setUp = await box(row.getByRole('button', { name: 'Set up again', exact: true }));
  const off = await box(row.getByRole('button', { name: 'Turn off', exact: true }));
  expect(setUp.x).toBeGreaterThan(label.x + label.width - 8);
  expect(off.x).toBeGreaterThan(setUp.x);
  // Beside the text, on its first line, not under it.
  expect(off.y).toBeLessThan(label.y + 60);
  expect(Math.abs(off.y - setUp.y)).toBeLessThan(8);
});

test('the iPhone app key form lays its fields side by side with the button at the end', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('mockCompanionBackend', JSON.stringify({ url: 'https://hollow-companion.example.ts.net', connected: true, lastContactAt: null, lastError: null, mirrorError: null, mirroredTasks: 3, lastPushAt: null, mirroredDocs: 2, docsError: null }));
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await openConnections(page, 'Devices'); // Companion devices sit in Devices (Settings PR 3)
  // S5b-3: push notifications sit in the iPhone app's fold, closed at rest.
  await page.locator('details.companion-phone-more').evaluate(el => { (el as HTMLDetailsElement).open = true; });
  const form = page.getByRole('form', { name: 'Import the push key' });
  await form.scrollIntoViewIfNeeded();
  const first = await box(form.getByLabel('Key ID'));
  const second = await box(form.getByLabel('Team ID'));
  const button = await box(form.getByRole('button', { name: /^Import key/ }));
  expect(second.x).toBeGreaterThan(first.x + first.width - 2);
  expect(button.x).toBeGreaterThan(second.x + second.width - 2);
  expect(Math.abs((button.y + button.height) - (first.y + first.height))).toBeLessThan(12);
});
