import { test, expect, type Page } from './fixtures/test';

// Settings → Devices → Companion devices → iPhone app → Keep Hollow running with the lid
// closed (2026-11-05; `src/mockPhoneLid.ts` plays `phone_lid_*`, a click there
// standing for the person's password in macOS's prompt): every action is a
// visible click in the iPhone app's own section, and the section says in
// words whether the lid can close now.

const PHONE = { deviceId: 'ph-0a1b2c3d', name: 'Alex’s iPhone', model: 'iPhone 16 Pro', pairedAt: '2026-11-04T16:30:00+00:00', lastSeenAt: '2026-11-04T16:40:00+00:00' };
const PAIRED = { enabled: true, listening: true, devices: [PHONE] };

async function open(page: Page, phone: Record<string, unknown> | null, lid?: Record<string, unknown>) {
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  await page.evaluate(async ({ phone, lid }) => {
    if (phone) localStorage.setItem('mockPhoneAccess', JSON.stringify(phone));
    else localStorage.removeItem('mockPhoneAccess');
    if (lid) localStorage.setItem('mockPhoneLid', JSON.stringify(lid));
    else localStorage.removeItem('mockPhoneLid');
    (await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');
  }, { phone, lid: lid ?? null });
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  // S5b-3: the lid row sits in the iPhone app's closed fold (with push notifications); open it.
  await page.locator('details.companion-phone-more').evaluate(el => { (el as HTMLDetailsElement).open = true; });
  const section = page.getByRole('group', { name: 'Keep Hollow running with the lid closed' });
  await expect(section).toBeVisible();
  return section;
}

test('it waits for the iPhone app, then turns on, adds battery and turns off by clicking', async ({ page }) => {
  let section = await open(page, null);
  const toggle = () => section.getByRole('switch', { name: 'Keep Hollow running with the lid closed' });
  await expect(toggle()).toHaveAttribute('aria-checked', 'false');
  await expect(toggle()).toBeDisabled();
  await expect(section.getByText('Available once the iPhone app is on.')).toBeVisible();
  await expect(section.getByText('Off: closing the lid sleeps this Mac, and paired iPhones can’t reach Hollow until it wakes.')).toBeVisible();

  section = await open(page, PAIRED);
  await expect(toggle()).toBeEnabled();
  await toggle().click();
  await expect(toggle()).toHaveAttribute('aria-checked', 'true');
  await expect(section.getByText('On. While an iPhone is paired, this Mac stays awake with its lid closed on power.')).toBeVisible();
  await expect(section.getByText('On power: you can close the lid. Hollow keeps running and your iPhone stays connected.')).toBeVisible();

  const battery = section.getByRole('checkbox', { name: 'On battery too' });
  await expect(section.getByText(/in a bag it can get hot/)).toBeVisible();
  // The box shows the helper's choice once macOS has said yes.
  await battery.click();
  await expect(battery).toBeChecked();
  await expect(section.getByText('On battery too: the Mac also stays awake with its lid closed on battery.')).toBeVisible();

  await toggle().click();
  await expect(toggle()).toHaveAttribute('aria-checked', 'false');
  await expect(battery).not.toBeChecked();
  await expect(section.getByText('Off. Closing the lid sleeps this Mac again.')).toBeVisible();
});

test('on battery it says the lid sleeps the Mac; a refused prompt keeps it off', async ({ page }) => {
  let section = await open(page, PAIRED, { enabled: true, source: 'battery', batteryPercent: 72 });
  await expect(section.getByText('On battery (72%): closing the lid sleeps this Mac, as usual. Plug in power to keep Hollow running with the lid closed.')).toBeVisible();

  section = await open(page, PAIRED, { fail: 'You cancelled macOS’s password prompt, so nothing changed.' });
  await section.getByRole('switch', { name: 'Keep Hollow running with the lid closed' }).click();
  await expect(section.getByText('You cancelled macOS’s password prompt, so nothing changed.')).toBeVisible();
  await expect(section.getByRole('switch', { name: 'Keep Hollow running with the lid closed' })).toHaveAttribute('aria-checked', 'false');
});

test('sleep Hollow turned off with its helper gone: Turn sleep back on', async ({ page }) => {
  const section = await open(page, PAIRED, { orphan: true });
  await expect(section.getByText(/Hollow turned system sleep off earlier and its helper is gone/)).toBeVisible();
  await section.getByRole('button', { name: 'Turn sleep back on' }).click();
  await expect(section.getByText('System sleep is back on.')).toBeVisible();
  await expect(section.getByRole('button', { name: 'Turn sleep back on' })).toHaveCount(0);
});

test('sleep someone else turned off is named and left alone', async ({ page }) => {
  const section = await open(page, PAIRED, { otherDisabled: true });
  await expect(section.getByText(/Hollow didn’t turn it off \(pmset disablesleep\)/)).toBeVisible();
  await expect(section.getByRole('button', { name: 'Turn sleep back on' })).toHaveCount(0);
});

test('sleep still off with the helper stopped: never says the lid sleeps the Mac, and Off turns sleep back on', async ({ page }) => {
  // The review's case: the helper switched off in Login Items while sleep
  // was off, then the iPhone app turned off.
  const section = await open(page, { ...PAIRED, enabled: false }, { enabled: true, heldOff: true, stopped: true });
  await expect(section.getByText('System sleep is still off and Hollow’s helper isn’t running, so closing the lid doesn’t sleep this Mac. Turn this off to turn sleep back on.')).toBeVisible();
  await expect(section.getByText(/closing the lid sleeps this Mac/i)).toHaveCount(0);
  const toggle = section.getByRole('switch', { name: 'Keep Hollow running with the lid closed' });
  await expect(toggle).toBeEnabled();
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await expect(section.getByText('Off. Closing the lid sleeps this Mac again.')).toBeVisible();
});

test('a helper from another version of Hollow: Update the helper replaces it with one click', async ({ page }) => {
  const section = await open(page, PAIRED, { enabled: true, outdated: true });
  await expect(section.getByText(/Hollow’s helper is from another version of Hollow/)).toBeVisible();
  await section.getByRole('button', { name: 'Update the helper' }).click();
  await expect(section.getByText('Hollow’s helper is up to date.')).toBeVisible();
  await expect(section.getByRole('button', { name: 'Update the helper' })).toHaveCount(0);
  await expect(section.getByRole('switch', { name: 'Keep Hollow running with the lid closed' })).toHaveAttribute('aria-checked', 'true');
});

test('system sleep Hollow can’t read, with its marker set: never says the lid sleeps the Mac', async ({ page }) => {
  // A macOS whose power report no longer has SleepDisabled, after the iPhone
  // app turned off: the helper restores on its marker alone.
  const section = await open(page, { ...PAIRED, enabled: false }, { enabled: true, heldOff: true, unreadable: true });
  await expect(section.getByText('System sleep is still off: Hollow’s helper should turn it back on within 10 seconds. Until then, closing the lid doesn’t sleep this Mac.')).toBeVisible();
  await expect(section.getByText('Hollow can’t read whether system sleep is off right now; its helper’s note says it turned sleep off, so Hollow treats it as off.')).toBeVisible();
  await expect(section.getByText(/closing the lid sleeps this Mac/i)).toHaveCount(0);
});
