// Settings, Devices, Companion devices, iPhone app, Keep Hollow
// running with the lid closed (2026-11-05; backend `phone_access/lid.rs`):
// the section's words. Pure, unit-tested in `phoneLid.test.ts`; the control
// is `settings/PhoneLidClosed.tsx`. Nothing here says closing the lid sleeps
// the Mac while system sleep reads off, or can't be read while Hollow's
// marker says its helper turned it off.

import type { PhoneLidStatus } from './types';

export type LidLine = { text: string; tone: 'ok' | 'warn' | 'info' };

/** On battery too's warning, beside its box. */
export const BATTERY_WARNING = 'A closed Mac that can’t sleep keeps running: in a bag it can get hot, and on battery it runs down until it shuts off. Tick this only when the Mac sits out in the open.';

/** What the switch row says. */
export const LID_ABOUT = 'Off until you turn it on. On: while an iPhone is paired, this Mac stays awake with its lid closed when it’s on power, so the iPhone keeps reaching Hollow. macOS asks for your password to turn it on and off.';

/** A helper whose last finished pass is older than this has stopped: three
 *  of its 10-second checks. */
export const HELPER_LATE_SECS = 30;

/** Said when `SleepDisabled` can't be read while Hollow's marker is set. */
export const UNREAD = 'Hollow can’t read whether system sleep is off right now; its helper’s note says it turned sleep off, so Hollow treats it as off.';

const ok = (text: string): LidLine => ({ text, tone: 'ok' });
const warn = (text: string): LidLine => ({ text, tone: 'warn' });
const info = (text: string): LidLine => ({ text, tone: 'info' });

const onBattery = (s: PhoneLidStatus) => (s.batteryPercent === null ? 'On battery' : `On battery (${s.batteryPercent}%)`);

/** Hollow asks its helper to keep system sleep off now: armed, and on power
 *  or with On battery too (the helper's own rule). */
const asking = (s: PhoneLidStatus) => s.armed && (s.source === 'power' || s.onBattery);

/** The helper hasn't finished a pass in three of its checks: either of its
 *  two daemons, the one that keeps sleep off or the restore check that turns
 *  it back on unplugged. Not said of a helper from another version of
 *  Hollow, which may not record its passes. */
const helperStopped = (s: PhoneLidStatus) =>
  !s.outdated && [s.lastPassSecs, s.lastRestoreSecs].some(secs => secs === null || secs > HELPER_LATE_SECS);

/** System sleep reads off, or can't be read while Hollow's marker says its
 *  helper turned it off (the helper restores on the marker alone). */
const sleepOff = (s: PhoneLidStatus) => s.sleepDisabled === true || (s.sleepDisabled === null && s.disabledByHollow);

/** A helper pass finished after this Hollow stopped asking (with 2 seconds
 *  for a pass already running then) and still left sleep off. */
const ranSinceStop = (s: PhoneLidStatus) =>
  s.stoppedAskingSecs !== null && s.lastPassSecs !== null && s.lastPassSecs + 2 < s.stoppedAskingSecs;

/** System sleep still off with Hollow's marker while Hollow no longer asks
 *  for it: when sleep comes back, or why it doesn't. */
function stillOff(s: PhoneLidStatus): LidLine {
  if (helperStopped(s)) {
    return warn('System sleep is still off and Hollow’s helper isn’t running, so closing the lid doesn’t sleep this Mac. Turn this off to turn sleep back on.');
  }
  if (ranSinceStop(s)) {
    return warn('System sleep is still off: Hollow’s helper ran since Hollow stopped asking and kept it off, so another process may be keeping it armed. Closing the lid doesn’t sleep this Mac. Turn this off to turn sleep back on.');
  }
  return warn('System sleep is still off: Hollow’s helper should turn it back on within 10 seconds. Until then, closing the lid doesn’t sleep this Mac.');
}

/** The headline: whether the lid can close now with Hollow kept running,
 *  on power or battery, and what happens when it does. */
function headline(s: PhoneLidStatus): LidLine {
  if (!s.enabled) {
    if (sleepOff(s) && s.disabledByHollow) {
      return warn('Off, but Hollow turned system sleep off earlier and its helper is gone: closing the lid doesn’t sleep this Mac, on battery too. Choose Turn sleep back on.');
    }
    if (s.sleepDisabled === true) {
      return warn('Off. System sleep is turned off on this Mac, and Hollow didn’t turn it off (pmset disablesleep): closing the lid doesn’t sleep it, on battery too. Hollow leaves that setting alone.');
    }
    return info('Off: closing the lid sleeps this Mac, and paired iPhones can’t reach Hollow until it wakes.');
  }
  if (s.sleepDisabled === true && !s.disabledByHollow) {
    return warn('System sleep is turned off on this Mac, and Hollow didn’t turn it off (pmset disablesleep): closing the lid doesn’t sleep it, on power or battery. Hollow leaves that setting alone.');
  }
  if (s.otherCopy) {
    return sleepOff(s)
      ? warn(`The helper was set up for another copy of Hollow (${s.otherCopy}), and system sleep is off now, so closing the lid doesn’t sleep this Mac. Turn this off and on again to use it with this copy.`)
      : warn(`The helper was set up for another copy of Hollow (${s.otherCopy}), so closing the lid sleeps this Mac. Turn this off and on again.`);
  }
  if (sleepOff(s) && !asking(s)) return stillOff(s);
  if (!s.iphoneOn) return info('On, but the iPhone app is off: closing the lid sleeps this Mac.');
  if (!s.ready) return info('On, waiting for a paired iPhone: until one pairs, closing the lid sleeps this Mac.');
  if (!s.armed) {
    return warn('Hollow couldn’t arm its helper: its folder in /Library/Application Support/Hollow is missing or closed to Hollow, so closing the lid sleeps this Mac. Turn this off and on again.');
  }
  if (s.source !== 'power' && !s.onBattery) {
    return s.source === 'battery'
      ? info(`${onBattery(s)}: closing the lid sleeps this Mac, as usual. Plug in power to keep Hollow running with the lid closed.`)
      : warn('Hollow can’t tell whether this Mac is on power. On battery, closing the lid sleeps it.');
  }
  if (!sleepOff(s)) {
    if (s.sleepDisabled === null) {
      return warn('Closing the lid may still sleep this Mac: Hollow can’t read whether system sleep is off, so its helper doesn’t turn it off.');
    }
    return helperStopped(s)
      ? warn('Closing the lid would still sleep this Mac: Hollow’s helper isn’t running. Turn this off and on again; if that doesn’t help, allow Hollow in System Settings → General → Login Items & Extensions.')
      : warn('Closing the lid would still sleep this Mac: macOS hasn’t turned system sleep off yet. Hollow’s helper checks every 10 seconds; if this stays, allow Hollow in System Settings → General → Login Items & Extensions.');
  }
  if (s.source !== 'power') {
    // The 10% floor is in this version's helper only; an outdated one runs the battery down.
    return warn(
      s.outdated
        ? `${onBattery(s)}: you can close the lid. The Mac keeps running until you open it or its battery runs out, so keep it out of a bag.`
        : `${onBattery(s)}: you can close the lid. The Mac keeps running until you open it or its battery falls to 10%, when it sleeps; keep it out of a bag.`,
    );
  }
  return ok('On power: you can close the lid. Hollow keeps running and your iPhone stays connected.');
}

/** The section's sentences, the headline first. */
export function lidLines(s: PhoneLidStatus | null): LidLine[] {
  if (!s?.available) return [];
  const lines = [headline(s)];
  const kept = s.enabled && !s.otherCopy && asking(s) && sleepOff(s) && s.disabledByHollow;
  if (kept && helperStopped(s)) {
    lines.push(warn('Hollow’s helper isn’t running, so system sleep stays off until it does, unplugged or with the iPhone app off too. Turn this off to turn sleep back on.'));
  } else if (kept && !s.outdated && s.sleepDisabled === true && s.source === 'power' && !s.onBattery) {
    // The restore check's promise: it waits on nothing the helper does. Not made
    // for a helper from another version, whose restore check may differ (ENG-139).
    lines.push(info('Unplugged, it sleeps with its lid closed again: a closed Mac goes to sleep within 10 seconds of losing power.'));
  }
  if (s.sleepDisabled === null && s.disabledByHollow) {
    lines.push(warn(UNREAD));
  }
  if (s.holding) lines.push(info('Idle sleep is held off too, so a Mac left open stays awake.'));
  if (s.enabled && s.outdated) {
    lines.push(warn('Hollow’s helper is from another version of Hollow. Choose Update the helper to bring it up to date; macOS asks for your password once.'));
  }
  if (!s.enabled && s.disabledByHollow && s.sleepDisabled === false) {
    lines.push(warn('Hollow’s helper is gone but left its note that it turned system sleep off. Choose Turn sleep back on to make sure sleep is on.'));
  }
  return lines;
}

/** The switch: whether it can move now and why not. Turning it off always
 *  works; turning it on needs the iPhone app on. */
export function lidSwitch(s: PhoneLidStatus | null, busy: boolean): { disabled: boolean; why: string | null } {
  if (!s?.available || busy) return { disabled: true, why: null };
  if (!s.enabled && !s.iphoneOn) return { disabled: true, why: 'Available once the iPhone app is on.' };
  return { disabled: false, why: null };
}

/** Turn sleep back on: offered when Hollow's helper is gone and its marker
 *  says it turned system sleep off. */
export const lidOrphan = (s: PhoneLidStatus | null) => !!s?.available && !s.enabled && s.disabledByHollow;

/** Update the helper: offered when the installed helper is from another
 *  version of Hollow (one password prompt, the same choices), and only with
 *  the iPhone app on: an update installs, which the backend refuses without
 *  it (ENG-139). */
export const lidUpdate = (s: PhoneLidStatus | null) => !!s?.available && s.enabled && s.outdated && s.iphoneOn;

/** What a click did, in the status line, from the readings before (`was`)
 *  and after (`next`). It says closing the lid sleeps the Mac only when
 *  system sleep reads on afterwards; otherwise the headline says why not. */
export function lidDone(enabled: boolean, onBattery: boolean, was: PhoneLidStatus | null, next: PhoneLidStatus | null): string {
  const sleepOn = next?.sleepDisabled === false;
  if (!enabled) {
    if (!was?.enabled) return sleepOn ? 'System sleep is back on.' : 'Hollow’s helper and its note are removed.';
    return sleepOn ? 'Off. Closing the lid sleeps this Mac again.' : 'Off. Hollow’s helper is removed.';
  }
  if (was?.enabled && was.outdated && !was.otherCopy && was.onBattery === onBattery) return 'Hollow’s helper is up to date.';
  if (was?.enabled && was.onBattery !== onBattery) {
    if (onBattery) return 'On battery too: the Mac also stays awake with its lid closed on battery.';
    // Someone else's setting, or a restore still to come on battery, keeps
    // sleep off; so may a reading Hollow can't take.
    const keptOff = !!next && next.sleepDisabled !== false && (!next.disabledByHollow || next.source !== 'power');
    return keptOff ? 'On battery too is off.' : 'On battery, closing the lid sleeps this Mac again.';
  }
  return onBattery
    ? 'On. While an iPhone is paired, this Mac stays awake with its lid closed, on power and on battery.'
    : 'On. While an iPhone is paired, this Mac stays awake with its lid closed on power.';
}
