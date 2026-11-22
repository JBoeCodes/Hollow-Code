// Browser-mock Settings, Devices, Companion devices, iPhone app, Keep Hollow
// running with the lid closed (2026-11-05; backend `phone_access/lid.rs`).
// Off and on power, following the iPhone app's own mock (`mockPhoneAccess`
// in localStorage: its switch and paired phones); the state lives in
// localStorage `mockPhoneLid`, which a spec may seed: `enabled`,
// `onBattery`, `source`, `batteryPercent`, `otherDisabled` (system sleep
// turned off by someone else), `orphan` (Hollow's marker with no helper),
// `otherCopy`, `heldOff` (system sleep still off with Hollow's marker after
// Hollow stopped asking), `stopped` (the last pass of both of the helper's
// daemons 10 minutes ago), `outdated` (a helper from another version of
// Hollow), `unreadable` (`SleepDisabled` can't be read) and `fail` (the next
// click's error). A click here stands for the person's password in macOS's
// prompt. Refuses what Rust refuses, with its words.

import type { PhoneLidStatus } from './types';

const KEY = 'mockPhoneLid';

interface MockLid {
  enabled: boolean;
  onBattery: boolean;
  source: PhoneLidStatus['source'];
  batteryPercent: number | null;
  otherDisabled: boolean;
  orphan: boolean;
  otherCopy: string | null;
  heldOff: boolean;
  stopped: boolean;
  outdated: boolean;
  unreadable: boolean;
  fail: string | null;
}

function load(): MockLid {
  const base: MockLid = {
    enabled: false, onBattery: false, source: 'power', batteryPercent: 100, otherDisabled: false, orphan: false,
    otherCopy: null, heldOff: false, stopped: false, outdated: false, unreadable: false, fail: null,
  };
  const seeded = localStorage.getItem(KEY);
  return seeded ? { ...base, ...(JSON.parse(seeded) as Partial<MockLid>) } : base;
}

/** The iPhone app's mock: (on, on with a phone paired). */
function phone(): { on: boolean; ready: boolean } {
  const saved = JSON.parse(localStorage.getItem('mockPhoneAccess') ?? '{}') as { enabled?: boolean; devices?: unknown[] };
  const on = !!saved.enabled;
  return { on, ready: on && (saved.devices?.length ?? 0) > 0 };
}

function view(state: MockLid): PhoneLidStatus {
  const { on, ready } = phone();
  // A helper set up for another copy doesn't follow this one, which doesn't arm it.
  const armed = state.enabled && ready && !state.otherCopy;
  const asking = armed && (state.source === 'power' || state.onBattery);
  const heldOff = state.enabled && state.heldOff && !asking;
  return {
    available: true, unavailableReason: null,
    enabled: state.enabled, onBattery: state.onBattery,
    iphoneOn: on, ready, armed, holding: asking,
    source: state.source, batteryPercent: state.source === 'unknown' ? null : state.batteryPercent,
    sleepDisabled: state.unreadable ? null : asking || heldOff || state.otherDisabled || state.orphan,
    disabledByHollow: asking || heldOff || state.orphan,
    lidClosed: false,
    otherCopy: state.enabled ? state.otherCopy : null,
    lastPassSecs: state.enabled ? (state.stopped ? 600 : 4) : null,
    lastRestoreSecs: state.enabled ? (state.stopped ? 600 : 3) : null,
    // Hollow stopped asking a moment ago.
    stoppedAskingSecs: asking ? null : 3,
    outdated: state.enabled && state.outdated,
  };
}

/** `phone_lid_*` for the browser mock. */
export function mockPhoneLid(command: string, args?: Record<string, unknown>): PhoneLidStatus {
  const state = load();
  if (command === 'phone_lid_set') {
    if (state.fail) {
      const message = state.fail;
      localStorage.setItem(KEY, JSON.stringify({ ...state, fail: null }));
      throw new Error(message);
    }
    const enabled = !!args?.enabled;
    if (enabled && !phone().on) throw new Error('Turn on the iPhone app first: this keeps the Mac awake only for a paired iPhone.');
    const onBattery = enabled && !!args?.onBattery;
    // Rust asks nothing for the helper it already has; any other click
    // installs this version's helper (or removes it, turning sleep back on
    // with Hollow's marker).
    const unchanged = enabled && state.enabled && state.onBattery === onBattery && !state.outdated && !state.otherCopy;
    if (!unchanged) {
      state.enabled = enabled;
      state.onBattery = onBattery;
      state.otherCopy = null;
      state.outdated = false;
      state.stopped = false;
      state.heldOff = false;
      if (!enabled) state.orphan = false;
      localStorage.setItem(KEY, JSON.stringify(state));
    }
  }
  return view(state);
}
