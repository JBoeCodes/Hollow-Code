import { describe, expect, it } from 'vitest';
import { PUSH_TEAM_ID, deviceRows, forwardLines, keyIdFromPath, pairBlocked, phoneStateLabel, publishChoice, pushLines, pushStateLabel, qrImageSrc, revokeConfirm, serverLine, spelledCode, tailscaleLine } from './phoneAccess';
import type { CompanionDevice, PhoneAccessStatus, PhoneDevice, PhonePush, PhoneTailscale } from './types';

// Settings, Devices, Companion devices, iPhone app (the phone chat API, 2026-11-04):
// what the section says and offers for each state `phone_access_status`
// reports, and the one device list it shares with the menu-bar app.

const tailscale = (change: Partial<PhoneTailscale> = {}): PhoneTailscale => ({
  installed: true, running: true, dnsName: 'mac.tail1234.ts.net', login: 'person@github',
  published: false, funnel: false, other: null, tcpForwards: [], foregroundForwards: [], serviceForwards: [], error: null, ...change,
});

const status = (change: Partial<PhoneAccessStatus> = {}, ts: Partial<PhoneTailscale> = {}): PhoneAccessStatus => ({
  available: true, unavailableReason: null, enabled: true, listening: true, bindError: null, port: 47680,
  recordError: null, tailscale: tailscale(ts), baseUrl: 'https://mac.tail1234.ts.net:8443', macName: 'Mac',
  devices: [], pairing: null, ...change,
});

// The qrcode crate's SVG renderer, as `phone_access::qr_svg` builds it.
const QR = '<?xml version="1.0" standalone="yes"?><svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="264" height="264" viewBox="0 0 264 264" shape-rendering="crispEdges"><rect x="0" y="0" width="264" height="264" fill="#ffffff"/><path fill="#000000" d="M32 32h56v8H32V32"/></svg>';

describe('the state chip', () => {
  it('reads each state the backend reports', () => {
    expect(phoneStateLabel(null, true)).toEqual({ label: 'Checking…', tone: 'off' });
    expect(phoneStateLabel(null, false)).toEqual({ label: 'Status unavailable', tone: 'off' });
    expect(phoneStateLabel(status({ available: false }), false).label).toBe('Not available here');
    expect(phoneStateLabel(status({ recordError: 'damaged' }), false)).toEqual({ label: 'Needs attention', tone: 'warn' });
    expect(phoneStateLabel(status({ enabled: false, listening: false }), false)).toEqual({ label: 'Off', tone: 'off' });
    expect(phoneStateLabel(status({ listening: false, bindError: 'Port 47680 is in use.' }), false)).toEqual({ label: "Can't start", tone: 'warn' });
    expect(phoneStateLabel(status(), false)).toEqual({ label: 'Not published', tone: 'warn' });
    expect(phoneStateLabel(status({}, { published: true }), false)).toEqual({ label: 'Ready', tone: 'ok' });
    expect(phoneStateLabel(status({}, { published: true, funnel: true }), false)).toEqual({ label: 'Open to the internet', tone: 'warn' });
  });
});

describe('the status lines', () => {
  it('says whether this Mac listens, and why not', () => {
    expect(serverLine(status({ enabled: false, listening: false }))).toEqual({ text: "Off. Paired iPhones can't reach this Mac until you turn it on.", tone: 'plain' });
    expect(serverLine(status())).toEqual({ text: 'Listening on 127.0.0.1:47680, on this Mac only.', tone: 'plain' });
    expect(serverLine(status({ listening: false, bindError: 'Port 47680 is taken by another app.' }))).toEqual({ text: 'Port 47680 is taken by another app.', tone: 'warn' });
    expect(serverLine(status({ listening: false })).text).toBe("Hollow couldn't start listening on 127.0.0.1:47680.");
  });

  it('says what Tailscale publishes and what is in the way', () => {
    expect(tailscaleLine(status({}, { installed: false, running: false })).text).toMatch(/^Tailscale isn't installed on this Mac\./);
    expect(tailscaleLine(status({}, { running: false })).text).toBe("Tailscale isn't running or signed in on this Mac.");
    expect(tailscaleLine(status({}, { error: 'Tailscale said: permission denied.' }))).toEqual({ text: 'Tailscale said: permission denied.', tone: 'warn' });
    expect(tailscaleLine(status({}, { other: 'http://127.0.0.1:3000' })).text).toBe("Tailscale already serves http://127.0.0.1:3000 on port 8443. Hollow won't replace it; stop that first.");
    expect(tailscaleLine(status({}, { published: true, funnel: true })).text).toMatch(/open to the internet with Tailscale Funnel/);
    expect(tailscaleLine(status({}, { published: true }))).toEqual({ text: 'Published to your tailnet at https://mac.tail1234.ts.net:8443. Only your own Tailscale account can use it.', tone: 'plain' });
    expect(tailscaleLine(status())).toEqual({ text: 'Not published on Tailscale yet. Your iPhone reaches this Mac once it is.', tone: 'plain' });
  });
});

describe('the Tailscale button', () => {
  it('stops publishing what is published, even through Funnel', () => {
    expect(publishChoice(status({}, { published: true }))).toEqual({ kind: 'unpublish', label: 'Stop publishing', disabled: false, why: null });
    expect(publishChoice(status({}, { published: true, funnel: true }))?.kind).toBe('unpublish');
  });

  // The review of the third review's fixes (2026-11-05): Stop publishing
  // turned 8443 off whatever served it, and for another app's funnelled
  // 8443 Settings said to click it. It turns 8443 off only when it serves
  // Hollow now, so Settings offers it only then and says what to do instead.
  it("never offers Stop publishing for another app's 8443, Funnel or not", () => {
    const funnelled = status({}, { other: 'http://127.0.0.1:3000', funnel: true });
    expect(publishChoice(funnelled)).toEqual({ kind: 'publish', label: 'Publish on Tailscale', disabled: true, why: 'Port 8443 serves something else.' });
    expect(tailscaleLine(funnelled)).toEqual({
      text: "Tailscale already serves http://127.0.0.1:3000 on port 8443, open to the internet with Tailscale Funnel. Hollow won't replace or change it; stop that first.",
      tone: 'warn',
    });
    expect(phoneStateLabel(funnelled, false)).toEqual({ label: 'Not published', tone: 'warn' });
    // Funnel on 8443 with nothing of Hollow's there: Publish waits for it.
    const bare = status({}, { funnel: true });
    expect(publishChoice(bare)).toEqual({ kind: 'publish', label: 'Publish on Tailscale', disabled: true, why: 'Tailscale Funnel is on for port 8443.' });
    expect(tailscaleLine(bare).text).toBe("Tailscale Funnel is on for port 8443, which doesn't serve Hollow. Hollow won't publish there until Funnel is off for that port in Tailscale.");
    expect(tailscaleLine(bare).text).not.toMatch(/Stop publishing/);
    // Beside a saved forward to Hollow: Stop publishing, for the forward.
    expect(publishChoice(status({}, { other: 'http://127.0.0.1:3000', funnel: true, tcpForwards: [9000] }))?.kind).toBe('unpublish');
  });

  // Second review (2026-11-04): a raw TCP forward to Hollow's port passes on
  // nothing of the caller, so its requests pass as this Mac's own.
  it('shows a raw TCP forward to Hollow as a problem that Stop publishing removes', () => {
    const forwarded = status({}, { published: true, tcpForwards: [8443] });
    expect(phoneStateLabel(forwarded, false)).toEqual({ label: 'Needs attention', tone: 'warn' });
    expect(tailscaleLine(forwarded)).toEqual({
      text: "Tailscale forwards tailnet port 8443 straight to Hollow with --tcp, which doesn't say who is calling, so Hollow can't check that it's you. Choose Stop publishing to remove it.",
      tone: 'warn',
    });
    expect(tailscaleLine(status({}, { tcpForwards: [9000, 10000] })).text).toMatch(/^Tailscale forwards tailnet ports 9000, 10000 straight to Hollow.* Choose Stop publishing to remove them\.$/);
    expect(publishChoice(status({}, { tcpForwards: [9000] }))).toEqual({ kind: 'unpublish', label: 'Stop publishing', disabled: false, why: null });
    // A status from before the fields reads as no forward.
    const older = status({}, { published: true });
    delete (older.tailscale as Partial<PhoneTailscale>).tcpForwards;
    delete (older.tailscale as Partial<PhoneTailscale>).foregroundForwards;
    delete (older.tailscale as Partial<PhoneTailscale>).serviceForwards;
    expect(phoneStateLabel(older, false)).toEqual({ label: 'Ready', tone: 'ok' });
    expect(forwardLines(older)).toEqual([]);
  });

  // Third review (2026-11-05): `tailscale serve --tcp=<port> off` edits only
  // the saved config, so Stop publishing can't end a forward a terminal's
  // `tailscale serve` or a Tailscale service holds. Settings says how instead
  // of offering a button that can't work.
  it('says how to stop a forward Stop publishing can not reach, and offers no button for it', () => {
    const stop = 'tailscale serve --service=svc:hollow --tcp=7000 off';
    const elsewhere = status({}, { foregroundForwards: [10000], serviceForwards: [{ service: 'svc:hollow', port: 7000, stop }] });
    expect(phoneStateLabel(elsewhere, false)).toEqual({ label: 'Needs attention', tone: 'warn' });
    expect(forwardLines(elsewhere)).toEqual([
      {
        text: "A tailscale serve command still running in a terminal (started without --bg) forwards tailnet port 10000 straight to Hollow with --tcp, which doesn't say who is calling, so Hollow can't check that it's you. Stop publishing can't end it: press Control-C in that terminal, then choose Check again.",
        tone: 'warn',
      },
      {
        text: "Tailscale service svc:hollow forwards tailnet port 7000 straight to Hollow with --tcp, which doesn't say who is calling, so Hollow can't check that it's you. Stop publishing can't remove it: run this in a terminal, then choose Check again:",
        tone: 'warn',
        command: stop,
      },
    ]);
    expect(forwardLines(status({}, { foregroundForwards: [6000, 10000] }))[0].text).toMatch(/forwards tailnet ports 6000, 10000 straight/);
    // Nothing Stop publishing could remove: Publish stays the button, and
    // the main line says what 8443 does.
    expect(publishChoice(elsewhere)).toEqual({ kind: 'publish', label: 'Publish on Tailscale', disabled: false, why: null });
    expect(tailscaleLine(elsewhere).text).toBe('Not published on Tailscale yet. Your iPhone reaches this Mac once it is.');
    // Published beside one: Stop publishing still takes 8443 off, and the
    // forward's own line stays.
    const both = status({}, { published: true, foregroundForwards: [10000] });
    expect(publishChoice(both)?.kind).toBe('unpublish');
    expect(phoneStateLabel(both, false).label).toBe('Needs attention');
    expect(forwardLines(both)).toHaveLength(1);
    expect(forwardLines(status())).toEqual([]);
  });

  // The review of the third review's fixes (2026-11-05): a service's name
  // comes from Tailscale's status, so Hollow shows a command only for a
  // plain one (Rust sends `stop: null` for any other) and says the rest in
  // words, without the name.
  it('says in words how to remove a service forward whose name it puts in no command', () => {
    const odd = status({}, { serviceForwards: [{ service: 'svc:x;open -a Calculator', port: 7000, stop: null }] });
    const [line, ...more] = forwardLines(odd);
    expect(more).toEqual([]);
    expect(line).toEqual({
      text: "A Tailscale service with a name Hollow won't put in a command forwards tailnet port 7000 straight to Hollow with --tcp, which doesn't say who is calling, so Hollow can't check that it's you. Stop publishing can't remove it: in a terminal, find the service with tailscale serve status, remove its forward with tailscale serve, giving --service its name, --tcp=7000 and off, then choose Check again.",
      tone: 'warn',
    });
    expect(line.text).not.toContain('Calculator');
    expect(phoneStateLabel(odd, false).label).toBe('Needs attention');
  });

  it('publishes only when Tailscale can, and says why not', () => {
    expect(publishChoice(status())).toEqual({ kind: 'publish', label: 'Publish on Tailscale', disabled: false, why: null });
    expect(publishChoice(status({}, { installed: false, running: false }))).toEqual({ kind: 'publish', label: 'Publish on Tailscale', disabled: true, why: 'Install Tailscale first.' });
    expect(publishChoice(status({}, { running: false }))?.why).toBe('Start Tailscale and sign in first.');
    // Never replaces what 8443 already serves.
    expect(publishChoice(status({}, { other: 'http://127.0.0.1:3000' }))).toMatchObject({ disabled: true, why: 'Port 8443 serves something else.' });
    expect(publishChoice(status({ available: false }))).toBeNull();
    expect(publishChoice(null)).toBeNull();
  });
});

describe('pairing', () => {
  it('needs the server on', () => {
    expect(pairBlocked(status())).toBeNull();
    expect(pairBlocked(status({ enabled: false, listening: false }))).toBe('Turn on the iPhone app first.');
    expect(pairBlocked(status({ listening: false, bindError: 'taken' }))).toBe('Turn on the iPhone app first.');
    expect(pairBlocked(status({ available: false, unavailableReason: 'Another Hollow runs this vault.' }))).toBe('Another Hollow runs this vault.');
  });

  it('reads the code aloud one character at a time', () => {
    expect(spelledCode('K7M3-Q9TX')).toBe('K 7 M 3 Q 9 T X');
  });

  it('shows the QR code as an image, never as markup', () => {
    const src = qrImageSrc(QR);
    expect(src).toBe(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(QR)}`);
    expect(decodeURIComponent(src!.split(',')[1])).toBe(QR);
    expect(qrImageSrc(null)).toBeNull();
    expect(qrImageSrc('')).toBeNull();
    expect(qrImageSrc('<div>not a picture</div>')).toBeNull();
    expect(qrImageSrc('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>')).toBeNull();
    expect(qrImageSrc('<svg xmlns="http://www.w3.org/2000/svg" onload="alert(1)"></svg>')).toBeNull();
    expect(qrImageSrc('<svg xmlns="http://www.w3.org/2000/svg"><image href="https://example.com/x.png"/></svg>')).toBeNull();
  });
});

describe('the one device list', () => {
  const mac: CompanionDevice = { deviceId: 'dev-1', name: 'Menu bar', platform: 'macos', publicKey: 'k', pairedAt: '2026-11-01T09:00:00Z', lastOpAt: '2026-11-03T10:00:00Z' };
  const phone: PhoneDevice = { deviceId: 'ph-0a1b2c3d', name: 'Alex’s iPhone', model: 'iPhone 16 Pro', pairedAt: '2026-11-04T16:30:00.123456+00:00', lastSeenAt: '2026-11-04T16:40:00+00:00' };
  const older: PhoneDevice = { deviceId: 'ph-00000001', name: 'Old phone', model: null, pairedAt: '2026-10-30T08:00:00+00:00', lastSeenAt: null };

  it('merges companion devices and phones, oldest pairing first', () => {
    const rows = deviceRows([mac], [phone, older]);
    expect(rows.map(r => r.key)).toEqual(['phone:ph-00000001', 'companion:dev-1', 'phone:ph-0a1b2c3d']);
    expect(rows[0]).toEqual({ key: 'phone:ph-00000001', kind: 'phone', deviceId: 'ph-00000001', name: 'Old phone', what: 'iPhone app', pairedAt: older.pairedAt, last: null });
    expect(rows[1]).toMatchObject({ kind: 'companion', what: 'Mac', last: { label: 'last change', at: mac.lastOpAt } });
    expect(rows[2]).toMatchObject({ kind: 'phone', what: 'iPhone app · iPhone 16 Pro', last: { label: 'last seen', at: phone.lastSeenAt } });
    expect(deviceRows([], [])).toEqual([]);
  });

  it('keeps an unreadable pairing time last, in the order given', () => {
    const odd = { ...older, deviceId: 'ph-0000000f', pairedAt: 'not a time' };
    expect(deviceRows([mac], [odd, phone]).map(r => r.deviceId)).toEqual(['dev-1', 'ph-0a1b2c3d', 'ph-0000000f']);
  });

  it('asks before revoking, in words for what each device loses', () => {
    const [phoneRow, macRow] = [deviceRows([], [phone])[0], deviceRows([mac], [])[0]];
    expect(revokeConfirm(phoneRow).title).toBe('Revoke Alex’s iPhone?');
    expect(revokeConfirm(phoneRow).body).toMatch(/next request is refused and its open chats stop updating/);
    expect(revokeConfirm(macRow).body).toMatch(/^Its key is deleted/);
  });
});

describe('push notifications (v1.1)', () => {
  const push: PhonePush = { state: 'ready', keyId: 'ABC123DEFG', teamId: 'TEAM000000', phones: 1, lastSentAt: null, lastError: null, lastErrorAt: null };

  it('says ready, no key or error', () => {
    expect(pushStateLabel(push)).toEqual({ label: 'Push: ready', tone: 'ok' });
    expect(pushStateLabel({ ...push, state: 'noKey' })).toEqual({ label: 'Push: no key', tone: 'off' });
    expect(pushStateLabel({ ...push, state: 'error' })).toEqual({ label: 'Push: error', tone: 'warn' });
    expect(pushStateLabel(null).tone).toBe('off');
  });

  it('names the key and the phones that get notifications', () => {
    expect(pushLines(push)).toEqual(["Key ABC123DEFG, team TEAM000000, kept in this Mac's Keychain.", '1 paired iPhone gets notifications.']);
    expect(pushLines({ ...push, state: 'noKey', keyId: null, phones: 0 })).toEqual([
      'Import the APNs key (.p8) from your Apple developer account to turn notifications on.',
      'No paired iPhone has turned on notifications yet.',
    ]);
    expect(pushLines({ ...push, phones: 2 })[1]).toBe('2 paired iPhones get notifications.');
    expect(pushLines(undefined)).toEqual([]);
  });

  it("reads the Key ID from Apple's file name", () => {
    expect(keyIdFromPath('/Users/x/Downloads/AuthKey_abc123defg.p8')).toBe('ABC123DEFG');
    expect(keyIdFromPath('/Users/x/Downloads/key.p8')).toBeNull();
    expect(keyIdFromPath(null)).toBeNull();
    expect(PUSH_TEAM_ID).toBe('TEAM000000');
  });
});
