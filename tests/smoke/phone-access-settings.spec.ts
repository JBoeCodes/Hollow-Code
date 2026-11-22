import { test, expect, type Page } from './fixtures/test';
import AxeBuilder from '@axe-core/playwright';

// Settings → Devices → Companion devices → iPhone app (the phone chat API, 2026-11-04;
// `src/mockPhoneAccess.ts` plays `phone_access_*`): every action is a visible
// click, the paired phones share the one device list, and the section stays
// accessible in each theme.

const PHONE = { deviceId: 'ph-0a1b2c3d', name: 'Alex’s iPhone', model: 'iPhone 16 Pro', pairedAt: '2026-11-04T16:30:00+00:00', lastSeenAt: '2026-11-04T16:40:00+00:00' };

async function open(page: Page, seed?: Record<string, unknown>) {
  await page.goto('/?mock=1');
  await expect(page.locator('.app')).toBeVisible();
  await page.evaluate(async seed => {
    if (seed) localStorage.setItem('mockPhoneAccess', JSON.stringify(seed));
    else localStorage.removeItem('mockPhoneAccess');
    (await import('/src/store/runtime.ts')).requestOpenSettingsSection('companions');
  }, seed ?? null);
  await page.getByRole('button', { name: 'Settings', exact: true }).click();
  const section = page.locator('.companion-phone');
  await expect(section.getByRole('heading', { name: 'iPhone app' })).toBeVisible();
  // S5b-3: push notifications and the lid row sit in a fold that is closed at rest (absent while the app is unavailable).
  const more = section.locator('details.companion-phone-more');
  if (await more.waitFor({ state: 'attached', timeout: 2000 }).then(() => true, () => false)) await more.evaluate(el => { (el as HTMLDetailsElement).open = true; });
  return section;
}

test('turn it on, publish on Tailscale, pair an iPhone and cancel, all by clicking', async ({ page }) => {
  const section = await open(page);
  const chip = section.locator('.companion-connection-state');
  await expect(chip).toHaveText('Off');
  await expect(section.getByText("Off. Paired iPhones can't reach this Mac until you turn it on.")).toBeVisible();
  // Pairing needs the server on, and says so beside the button.
  await expect(section.getByRole('button', { name: 'Pair an iPhone' })).toBeDisabled();
  await expect(section.getByText('Turn on the iPhone app first.')).toBeVisible();

  await section.getByRole('switch', { name: 'Serve the iPhone app' }).click();
  await expect(section.getByRole('switch', { name: 'Serve the iPhone app' })).toHaveAttribute('aria-checked', 'true');
  await expect(section.getByText('Listening on 127.0.0.1:47680, on this Mac only.')).toBeVisible();
  await expect(chip).toHaveText('Not published');

  await section.getByRole('button', { name: 'Publish on Tailscale' }).click();
  await expect(chip).toHaveText('Ready');
  await expect(section.getByText('Published to your tailnet at https://my-mac.tail0000.ts.net:8443. Only your own Tailscale account can use it.')).toBeVisible();
  await expect(section.getByRole('button', { name: 'Stop publishing' })).toBeVisible();

  await section.getByRole('button', { name: 'Pair an iPhone' }).click();
  const pairing = section.getByRole('group', { name: 'Pair an iPhone' });
  await expect(pairing.locator('code')).toHaveText('K7M3-Q9TX');
  await expect(pairing.locator('code')).toHaveAttribute('aria-label', 'Pairing code K 7 M 3 Q 9 T X');
  await expect(pairing.getByText('https://my-mac.tail0000.ts.net:8443')).toBeVisible();
  const qr = pairing.getByRole('img', { name: /QR code of the pairing link/ });
  await expect(qr).toBeVisible();
  expect(await qr.getAttribute('src')).toMatch(/^data:image\/svg\+xml;charset=utf-8,/);
  // The QR code drew (an image that failed to load has no natural size).
  expect(await qr.evaluate((img: HTMLImageElement) => img.naturalWidth)).toBeGreaterThan(0);

  await pairing.getByRole('button', { name: 'Cancel pairing' }).click();
  await expect(section.getByRole('button', { name: 'Pair an iPhone' })).toBeEnabled();

  await section.getByRole('button', { name: 'Stop publishing' }).click();
  await expect(chip).toHaveText('Not published');
  await section.getByRole('switch', { name: 'Serve the iPhone app' }).click();
  await expect(chip).toHaveText('Off');
});

test('a paired iPhone sits in the one device list and Revoke asks first', async ({ page }) => {
  await open(page, { enabled: true, listening: true, devices: [PHONE] });
  const list = page.getByRole('list', { name: 'Paired devices' });
  const row = list.getByRole('listitem').filter({ hasText: 'Alex’s iPhone' });
  await expect(row).toContainText('iPhone app · iPhone 16 Pro');
  await expect(row).toContainText('paired');
  await expect(row).toContainText('last seen');
  // There is no second device list for phones.
  await expect(page.locator('.companion-devices ul.companion-device-list')).toHaveCount(1);

  await row.getByRole('button', { name: 'Revoke Alex’s iPhone' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Revoke Alex’s iPhone?');
  await expect(dialog).toContainText('its open chats stop updating');
  await dialog.getByRole('button', { name: 'Revoke', exact: true }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByText('No devices are paired.')).toBeVisible();
});

test('a taken port, missing Tailscale and a refused click say why in words', async ({ page }) => {
  const section = await open(page, {
    enabled: true, listening: false, bindError: 'Port 47680 is in use by another app, so Hollow can’t listen for the iPhone app. Quit that app, then turn the switch off and on.',
    tailscale: { installed: false, running: false, dnsName: null, login: null },
    baseUrl: null,
  });
  await expect(section.locator('.companion-connection-state')).toHaveText("Can't start");
  await expect(section.getByText('Port 47680 is in use by another app', { exact: false })).toBeVisible();
  await expect(section.getByText("Tailscale isn't installed on this Mac. Install it and sign in, then publish.")).toBeVisible();
  await expect(section.getByRole('button', { name: 'Publish on Tailscale' })).toBeDisabled();
  await expect(section.getByText('Install Tailscale first.')).toBeVisible();

  // A click Hollow refuses keeps the section and shows Hollow's sentence.
  await page.evaluate(() => {
    const saved = JSON.parse(localStorage.getItem('mockPhoneAccess') ?? '{}');
    localStorage.setItem('mockPhoneAccess', JSON.stringify({ ...saved, fail: 'Hollow couldn’t save the iPhone app’s record: disk full.' }));
  });
  await section.getByRole('switch', { name: 'Serve the iPhone app' }).click();
  await expect(section.getByRole('alert')).toContainText('disk full');
  await expect(section.getByRole('switch', { name: 'Serve the iPhone app' })).toHaveAttribute('aria-checked', 'true');
});

// Second review (2026-11-04): a raw TCP forward (`tailscale serve --tcp`) to
// Hollow's port passes on nothing of who is calling, so Settings shows it as
// a problem until Stop publishing removes it.
test('a raw TCP forward to Hollow shows as a problem until Stop publishing removes it', async ({ page }) => {
  const section = await open(page, { enabled: true, listening: true, tailscale: { published: true, tcpForwards: [8443] } });
  const chip = section.locator('.companion-connection-state');
  await expect(chip).toHaveText('Needs attention');
  const warning = section.getByText('Tailscale forwards tailnet port 8443 straight to Hollow with --tcp', { exact: false });
  await expect(warning).toBeVisible();
  await section.getByRole('button', { name: 'Stop publishing' }).click();
  await expect(chip).toHaveText('Not published');
  await expect(warning).toHaveCount(0);
});

// Third review (2026-11-05): `tailscale serve --tcp=<port> off` edits only
// the saved config, so a forward a terminal's `tailscale serve` or a
// Tailscale service holds outlives Stop publishing. Settings says how to stop
// each and offers no button that can't.
test('a forward Stop publishing can not reach says how to stop it', async ({ page }) => {
  const stop = 'tailscale serve --service=svc:hollow --tcp=7000 off';
  const section = await open(page, {
    enabled: true, listening: true,
    tailscale: { published: true, tcpForwards: [9000], foregroundForwards: [10000], serviceForwards: [{ service: 'svc:hollow', port: 7000, stop }] },
  });
  const chip = section.locator('.companion-connection-state');
  await expect(chip).toHaveText('Needs attention');
  const saved = section.getByText('Tailscale forwards tailnet port 9000 straight to Hollow with --tcp', { exact: false });
  const terminal = section.getByText('A tailscale serve command still running in a terminal (started without --bg) forwards tailnet port 10000', { exact: false });
  const service = section.getByText('Tailscale service svc:hollow forwards tailnet port 7000', { exact: false });
  await expect(saved).toBeVisible();
  await expect(terminal).toContainText('press Control-C in that terminal');
  await expect(service).toBeVisible();
  await expect(section.locator('.companion-phone-lines code')).toHaveText(stop);

  await section.getByRole('button', { name: 'Stop publishing' }).click();
  await expect(saved).toHaveCount(0);
  await expect(terminal).toBeVisible();
  await expect(service).toBeVisible();
  await expect(chip).toHaveText('Needs attention');
  await expect(section.getByRole('button', { name: 'Stop publishing' })).toHaveCount(0);
  await expect(section.getByRole('button', { name: 'Publish on Tailscale' })).toBeEnabled();
});

// The review of the third review's fixes (2026-11-05): Stop publishing
// turned 8443 off whatever served it. Another app's 8443, funnelled or not,
// now stays, and Settings neither offers the button for it nor says to
// click it.
test("Stop publishing leaves another app's 8443 alone", async ({ page }) => {
  const section = await open(page, {
    enabled: true, listening: true,
    tailscale: { other: 'http://127.0.0.1:3000', funnel: true, tcpForwards: [9000] },
  });
  const chip = section.locator('.companion-connection-state');
  await expect(chip).toHaveText('Needs attention');
  const forward = section.getByText('Tailscale forwards tailnet port 9000 straight to Hollow with --tcp', { exact: false });
  await expect(forward).toBeVisible();
  await section.getByRole('button', { name: 'Stop publishing' }).click();
  await expect(forward).toHaveCount(0);
  await expect(section.getByText("Tailscale already serves http://127.0.0.1:3000 on port 8443, open to the internet with Tailscale Funnel. Hollow won't replace or change it; stop that first.")).toBeVisible();
  await expect(chip).toHaveText('Not published');
  await expect(section.getByRole('button', { name: 'Stop publishing' })).toHaveCount(0);
  await expect(section.getByRole('button', { name: 'Publish on Tailscale' })).toBeDisabled();
  await expect(section.getByText('Port 8443 serves something else.')).toBeVisible();
});

// Stop publishing tries every step even when one fails, so after a failure
// the section reads the status again: what went through is gone, and the
// error stays.
test('a Stop publishing that fails part way shows its error and what is left', async ({ page }) => {
  const section = await open(page, {
    enabled: true, listening: true,
    failAfter: "Couldn't remove the --tcp forward on tailnet port 9000 (Tailscale said: error: serve config does not exist).",
    tailscale: { published: true, tcpForwards: [9000] },
  });
  const chip = section.locator('.companion-connection-state');
  await expect(chip).toHaveText('Needs attention');
  await section.getByRole('button', { name: 'Stop publishing' }).click();
  await expect(section.getByRole('alert')).toContainText('serve config does not exist');
  await expect(chip).toHaveText('Not published');
  await expect(section.getByRole('button', { name: 'Publish on Tailscale' })).toBeEnabled();
});

test('push notifications: a test goes to the phones, and Remove key asks first (v1.1)', async ({ page }) => {
  const section = await open(page, {
    enabled: true, listening: true, devices: [{ ...PHONE, push: true }],
    push: { state: 'ready', keyId: 'ABC123DEFG', teamId: 'TEAM000000', phones: 1, lastSentAt: null, lastError: null, lastErrorAt: null },
  });
  const push = section.getByRole('group', { name: 'Push notifications' });
  const chip = push.locator('.companion-push-state');
  await expect(chip).toHaveText('Push: ready');
  // The section's own chip is still the one the section shows.
  await expect(section.locator('.companion-connection-state')).toHaveCount(1);
  await expect(push.getByText("Key ABC123DEFG, team TEAM000000, kept in this Mac's Keychain.")).toBeVisible();
  await expect(push.getByText('1 paired iPhone gets notifications.')).toBeVisible();
  await expect(push.getByLabel('Team ID')).toHaveValue('TEAM000000');
  await push.getByRole('button', { name: 'Send test notification' }).click();
  await expect(section.getByText('Sent a test notification to 1 iPhone.')).toBeVisible();

  await push.getByRole('button', { name: 'Remove key' }).click();
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Remove the push key?');
  await dialog.getByRole('button', { name: 'Remove', exact: true }).click();
  await expect(chip).toHaveText('Push: no key');
  await expect(push.getByRole('button', { name: 'Send test notification' })).toBeDisabled();
  await expect(push.getByRole('button', { name: 'Import key…' })).toBeEnabled();
});

test('push notifications: a test with no phone registered says why in words (v1.1)', async ({ page }) => {
  const section = await open(page, {
    enabled: true, listening: true, devices: [PHONE],
    push: { state: 'ready', keyId: 'ABC123DEFG', teamId: 'TEAM000000', phones: 0, lastSentAt: null, lastError: 'Apple refused the push to Alex’s iPhone (410 Unregistered)', lastErrorAt: '2026-11-05T19:40:00Z' },
  });
  const push = section.getByRole('group', { name: 'Push notifications' });
  await expect(push.getByText('No paired iPhone has turned on notifications yet.')).toBeVisible();
  await expect(push.getByRole('alert')).toContainText('Last error: Apple refused the push to Alex’s iPhone (410 Unregistered)');
  await expect(push.getByRole('button', { name: 'Send test notification' })).toBeDisabled();
});

for (const theme of ['light', 'dark', 'glass']) test(`the iPhone app section is accessible: ${theme}`, async ({ page }) => {
  await page.addInitScript(theme => {
    localStorage.setItem('theme', theme === 'light' ? 'light' : 'dark');
    localStorage.setItem('glass', theme === 'glass' ? 'on' : 'off');
  }, theme);
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const section = await open(page, { enabled: true, listening: true, devices: [PHONE] });
  // Glass draws over the native window's backdrop, which a browser page has
  // not got (as companion-settings.spec.ts does).
  if (theme === 'glass') await page.addStyleTag({ content: 'html[data-glass] { background:var(--canvas); }' });
  await section.getByRole('button', { name: 'Pair an iPhone' }).click();
  await expect(section.getByRole('group', { name: 'Pair an iPhone' })).toBeVisible();
  expect((await new AxeBuilder({ page }).include('.companion-devices').analyze()).violations).toEqual([]);
});
