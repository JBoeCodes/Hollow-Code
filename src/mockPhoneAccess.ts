// Browser-mock Settings, Devices, Companion devices, iPhone app (the phone chat API,
// 2026-11-04; backend `phone_access.rs`). Off with nothing paired and
// Tailscale running but not publishing, until a click changes it; the state
// lives in localStorage `mockPhoneAccess` so a reload keeps it, and a spec
// may seed it (any field of the status, plus `fail`: the next command's
// error, and `failAfter`: the next command's error once it has done its
// work, as a Stop publishing that tried every step and had one fail). Refuses
// what Rust refuses, with its words.

import type { PhoneAccessStatus, PhoneDevice } from './types';

const KEY = 'mockPhoneAccess';
const DNS = 'my-mac.tail0000.ts.net';
const QR = '<?xml version="1.0" standalone="yes"?><svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="200" height="200" viewBox="0 0 25 25" shape-rendering="crispEdges"><rect x="0" y="0" width="25" height="25" fill="#ffffff"/><path fill="#000000" d="M2 2h7v7H2V2M16 2h7v7H16V2M2 16h7v7H2V16M11 11h3v3H11V11"/></svg>';

type MockPhone = PhoneAccessStatus & { fail?: string | null; failAfter?: string | null };

function base(): MockPhone {
  return {
    available: true, unavailableReason: null, enabled: false, listening: false, bindError: null, port: 47680,
    recordError: null,
    tailscale: { installed: true, running: true, dnsName: DNS, login: 'user@github', published: false, funnel: false, other: null, tcpForwards: [], foregroundForwards: [], serviceForwards: [], error: null },
    baseUrl: `https://${DNS}:8443`, macName: 'Alex’s MacBook Pro', devices: [], pairing: null,
    push: { state: 'noKey', keyId: null, teamId: null, phones: 0, lastSentAt: null, lastError: null, lastErrorAt: null },
    machines: [], machinePairing: null, approvalsCode: null, approvalWaitSecs: 300, approvalsDropped: [],
  };
}

function load(): MockPhone {
  const seeded = localStorage.getItem(KEY);
  if (!seeded) return base();
  const saved = JSON.parse(seeded) as Partial<MockPhone>;
  return { ...base(), ...saved, tailscale: { ...base().tailscale, ...(saved.tailscale ?? {}) } };
}

function save(state: MockPhone) {
  localStorage.setItem(KEY, JSON.stringify(state));
}

function view(state: MockPhone): PhoneAccessStatus {
  const status: MockPhone = { ...state };
  delete status.fail;
  delete status.failAfter;
  if (status.pairing && Date.parse(status.pairing.expiresAt) <= Date.now()) status.pairing = null;
  if (status.machinePairing && Date.parse(status.machinePairing.expiresAt) <= Date.now()) status.machinePairing = null;
  if (status.approvalsCode && Date.parse(status.approvalsCode.expiresAt) <= Date.now()) status.approvalsCode = null;
  return status;
}

/** `phone_push_test` for the browser mock: what Rust says when it can't send,
 *  else how many iPhones Apple took. */
export function mockPhonePushTest(): string {
  const push = load().push;
  if (push?.state !== 'ready') throw new Error('Import the push key first.');
  if (!push.phones) throw new Error('No paired iPhone has turned on notifications yet. Open the Hollow app on the iPhone and allow them.');
  return push.phones === 1 ? 'Sent a test notification to 1 iPhone.' : `Sent a test notification to ${push.phones} iPhones.`;
}

/** `phone_access_*` and `phone_push_*` (v1.1) for the browser mock. */
export function mockPhoneAccess(command: string, args?: Record<string, unknown>): PhoneAccessStatus {
  const state = load();
  if (state.fail && command !== 'phone_access_status') {
    const message = state.fail;
    save({ ...state, fail: null });
    throw new Error(message);
  }
  const failAfter = command !== 'phone_access_status' ? state.failAfter : null;
  if (failAfter) state.failAfter = null;
  switch (command) {
    case 'phone_access_set_enabled': {
      const on = !!args?.enabled;
      state.enabled = on;
      state.listening = on && !state.bindError;
      if (!on) state.pairing = null;
      break;
    }
    case 'phone_access_pairing_start':
      if (!state.listening) throw new Error('Turn on the iPhone app first, so the iPhone can reach this Mac.');
      state.pairing = {
        code: 'K7M3-Q9TX',
        expiresAt: new Date(Date.now() + 600_000).toISOString(),
        link: state.baseUrl ? `hollow-companion://pair?url=${encodeURIComponent(state.baseUrl)}&code=K7M3Q9TX&mac=${encodeURIComponent(state.macName)}` : null,
        qrSvg: state.baseUrl ? QR : null,
      };
      break;
    case 'phone_access_pairing_cancel':
      state.pairing = null;
      break;
    case 'phone_access_revoke':
      state.devices = state.devices.filter((d: PhoneDevice) => d.deviceId !== String(args?.deviceId ?? '').trim());
      state.machines = (state.machines ?? []).filter(m => m.deviceId !== String(args?.deviceId ?? '').trim());
      break;
    // M2: Let another Mac connect (the server listens for it, switch or not).
    case 'phone_access_machine_pairing_start':
      state.machinePairing = { code: 'M4CX-7RQ2', expiresAt: new Date(Date.now() + 600_000).toISOString() };
      break;
    case 'phone_access_machine_pairing_cancel':
      state.machinePairing = null;
      break;
    // B4: approvals from the phone (the Mac's code, the keys, the wait).
    case 'phone_access_approvals_start':
      state.approvalsCode = { deviceId: String(args?.deviceId ?? ''), code: 'K7QD-4M2X', expiresAt: new Date(Date.now() + 600_000).toISOString() };
      break;
    case 'phone_access_approvals_cancel':
      state.approvalsCode = null;
      break;
    case 'phone_access_approvals_off':
      state.devices = state.devices.map((d: PhoneDevice) => d.deviceId === String(args?.deviceId ?? '') ? { ...d, approvals: null } : d);
      break;
    case 'phone_access_approvals_wait':
      state.approvalWaitSecs = Number(args?.seconds ?? 300);
      break;
    case 'phone_access_publish':
      if (!state.tailscale.installed) throw new Error("Tailscale isn't installed on this Mac (Hollow looks in /usr/local/bin, /opt/homebrew/bin and the Tailscale app).");
      state.tailscale.published = true;
      break;
    // A foreground or service forward stays: `off` reaches only the saved
    // config (third review, 2026-11-05). So does another app's 8443: Rust
    // turns 8443 off only when it serves Hollow.
    case 'phone_access_unpublish':
      if (state.tailscale.published) {
        state.tailscale.published = false;
        state.tailscale.funnel = false;
      }
      state.tailscale.tcpForwards = [];
      break;
    // v1.1: the push key, as Rust checks it (the file's contents aren't
    // read here: the harness has no file to pick).
    case 'phone_push_import_key': {
      const keyId = String(args?.keyId ?? '').trim().toUpperCase() || (/AuthKey_([A-Za-z0-9]{10})\.p8$/.exec(String(args?.path ?? ''))?.[1] ?? '').toUpperCase();
      if (!keyId) throw new Error('Enter the key\'s Key ID (Apple shows it beside the key, and names the file AuthKey_<Key ID>.p8).');
      if (!/^[A-Z0-9]{10}$/.test(keyId)) throw new Error('The Key ID is 10 letters and digits.');
      const teamId = String(args?.teamId ?? '').trim().toUpperCase() || 'TEAM000000';
      state.push = { ...(state.push ?? base().push!), state: 'ready', keyId, teamId, lastError: null, lastErrorAt: null };
      break;
    }
    case 'phone_push_remove_key':
      state.push = { ...(state.push ?? base().push!), state: 'noKey', keyId: null, teamId: null };
      break;
    default:
      break;
  }
  save(state);
  if (failAfter) throw new Error(failAfter);
  return view(state);
}
