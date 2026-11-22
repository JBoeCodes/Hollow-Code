import { describe, expect, it } from 'vitest';
import { BATTERY_WARNING, UNREAD, lidDone, lidLines, lidOrphan, lidSwitch, lidUpdate } from './phoneLid';
import type { PhoneLidStatus } from './types';

// Keep Hollow running with the lid closed (2026-11-05): what the section says
// for each state `phone_lid_status` reports, whether the lid can close now,
// on power or battery, and what happens.

const status = (change: Partial<PhoneLidStatus> = {}): PhoneLidStatus => ({
  available: true, unavailableReason: null, enabled: false, onBattery: false, iphoneOn: true, ready: true,
  armed: false, holding: false, source: 'power', batteryPercent: 100, sleepDisabled: false,
  disabledByHollow: false, lidClosed: false, otherCopy: null, lastPassSecs: 4, lastRestoreSecs: 3,
  stoppedAskingSecs: null, outdated: false, ...change,
});
/** On, armed and kept awake on power. */
const kept = (change: Partial<PhoneLidStatus> = {}) =>
  status({ enabled: true, armed: true, holding: true, sleepDisabled: true, disabledByHollow: true, ...change });
/** On, Hollow no longer asking, and system sleep still off with Hollow's marker. */
const stillOff = (change: Partial<PhoneLidStatus> = {}) =>
  status({ enabled: true, sleepDisabled: true, disabledByHollow: true, stoppedAskingSecs: 3, ...change });
const headline = (s: PhoneLidStatus) => lidLines(s)[0];
const texts = (s: PhoneLidStatus) => lidLines(s).map(l => l.text);
/** The words Settings must never show while system sleep is still off. */
const SLEEPS = /closing the lid sleeps this Mac|it sleeps with its lid closed/i;

const WITHIN = 'System sleep is still off: Hollow’s helper should turn it back on within 10 seconds. Until then, closing the lid doesn’t sleep this Mac.';
const NOT_RUNNING = 'System sleep is still off and Hollow’s helper isn’t running, so closing the lid doesn’t sleep this Mac. Turn this off to turn sleep back on.';

describe('the headline: can the lid close now', () => {
  it('off: the lid sleeps the Mac', () => {
    expect(headline(status())).toEqual({ tone: 'info', text: 'Off: closing the lid sleeps this Mac, and paired iPhones can’t reach Hollow until it wakes.' });
  });

  it('on power and kept awake: safe to close, and what unplugging does', () => {
    const lines = lidLines(kept());
    expect(lines[0]).toEqual({ tone: 'ok', text: 'On power: you can close the lid. Hollow keeps running and your iPhone stays connected.' });
    expect(lines.map(l => l.text)).toContain('Unplugged, it sleeps with its lid closed again: a closed Mac goes to sleep within 10 seconds of losing power.');
    expect(lines.map(l => l.text)).toContain('Idle sleep is held off too, so a Mac left open stays awake.');
  });

  it('on battery without On battery too: the lid sleeps the Mac as usual', () => {
    const s = status({ enabled: true, armed: true, source: 'battery', batteryPercent: 87, stoppedAskingSecs: 40 });
    expect(headline(s)).toEqual({ tone: 'info', text: 'On battery (87%): closing the lid sleeps this Mac, as usual. Plug in power to keep Hollow running with the lid closed.' });
    expect(lidLines(s)).toHaveLength(1);
  });

  it('on battery with On battery too: it can close, with the bag warning', () => {
    const s = kept({ source: 'battery', batteryPercent: 64, onBattery: true });
    expect(headline(s)).toEqual({ tone: 'warn', text: 'On battery (64%): you can close the lid. The Mac keeps running until you open it or its battery falls to 10%, when it sleeps; keep it out of a bag.' });
    expect(lidLines(s).map(l => l.text).join(' ')).not.toContain('Unplugged');
  });

  it('an outdated helper has no 10% floor: the line does not promise one', () => {
    const s = kept({ source: 'battery', batteryPercent: 64, onBattery: true, outdated: true });
    expect(headline(s).text).not.toContain('10%');
    expect(headline(s).text).toContain('its battery runs out');
  });

  it('armed but macOS has not turned sleep off yet: says it would still sleep', () => {
    expect(headline(status({ enabled: true, armed: true, holding: true }))).toEqual({
      tone: 'warn',
      text: 'Closing the lid would still sleep this Mac: macOS hasn’t turned system sleep off yet. Hollow’s helper checks every 10 seconds; if this stays, allow Hollow in System Settings → General → Login Items & Extensions.',
    });
  });

  it('armed with a helper that has stopped: says the helper isn’t running', () => {
    for (const lastPassSecs of [31, 600, null]) {
      expect(headline(status({ enabled: true, armed: true, holding: true, lastPassSecs }))).toEqual({
        tone: 'warn',
        text: 'Closing the lid would still sleep this Mac: Hollow’s helper isn’t running. Turn this off and on again; if that doesn’t help, allow Hollow in System Settings → General → Login Items & Extensions.',
      });
    }
  });

  it('on but waiting: the iPhone app off, or no phone paired', () => {
    expect(headline(status({ enabled: true, iphoneOn: false, ready: false })).text).toBe('On, but the iPhone app is off: closing the lid sleeps this Mac.');
    expect(headline(status({ enabled: true, ready: false })).text).toBe('On, waiting for a paired iPhone: until one pairs, closing the lid sleeps this Mac.');
  });

  it('ready but Hollow couldn’t arm its helper: says so', () => {
    expect(headline(status({ enabled: true }))).toEqual({
      tone: 'warn',
      text: 'Hollow couldn’t arm its helper: its folder in /Library/Application Support/Hollow is missing or closed to Hollow, so closing the lid sleeps this Mac. Turn this off and on again.',
    });
  });

  it('a helper set up for another copy of Hollow', () => {
    expect(headline(status({ enabled: true, otherCopy: '/Users/dev/dev/hollow' }))).toEqual({
      tone: 'warn', text: 'The helper was set up for another copy of Hollow (/Users/dev/dev/hollow), so closing the lid sleeps this Mac. Turn this off and on again.',
    });
  });

  it('a helper for another copy with system sleep off: never says the lid sleeps the Mac', () => {
    const s = stillOff({ otherCopy: '/Users/dev/dev/hollow' });
    expect(headline(s)).toEqual({
      tone: 'warn', text: 'The helper was set up for another copy of Hollow (/Users/dev/dev/hollow), and system sleep is off now, so closing the lid doesn’t sleep this Mac. Turn this off and on again to use it with this copy.',
    });
  });

  it('an unknown power source never claims the lid is safe', () => {
    expect(headline(status({ enabled: true, armed: true, source: 'unknown', batteryPercent: null })).tone).toBe('warn');
  });
});

describe('system sleep still off with Hollow’s marker while Hollow has stopped asking', () => {
  it('on, but the iPhone app off or no phone paired: says sleep should come back within 10 seconds', () => {
    for (const s of [stillOff({ iphoneOn: false, ready: false }), stillOff({ ready: false })]) {
      expect(headline(s)).toEqual({ tone: 'warn', text: WITHIN });
      expect(texts(s).join(' ')).not.toMatch(SLEEPS);
    }
  });

  it('on battery without On battery too: no "sleeps as usual" until sleep is back on', () => {
    const s = stillOff({ armed: true, source: 'battery', batteryPercent: 80 });
    expect(headline(s)).toEqual({ tone: 'warn', text: WITHIN });
    expect(texts(s).join(' ')).not.toMatch(SLEEPS);
  });

  it('after its passes stop: the helper isn’t running, and Off turns sleep back on', () => {
    for (const lastPassSecs of [31, 3600, null]) {
      const s = stillOff({ iphoneOn: false, ready: false, lastPassSecs, stoppedAskingSecs: 4000 });
      expect(headline(s)).toEqual({ tone: 'warn', text: NOT_RUNNING });
      expect(texts(s).join(' ')).not.toMatch(SLEEPS);
    }
  });

  it('a pass since Hollow stopped asking left it off: says so instead of promising 10 seconds', () => {
    const s = stillOff({ ready: false, lastPassSecs: 5, stoppedAskingSecs: 25 });
    expect(headline(s)).toEqual({
      tone: 'warn',
      text: 'System sleep is still off: Hollow’s helper ran since Hollow stopped asking and kept it off, so another process may be keeping it armed. Closing the lid doesn’t sleep this Mac. Turn this off to turn sleep back on.',
    });
    // A pass that was already running when Hollow stopped doesn't count.
    expect(headline(stillOff({ ready: false, lastPassSecs: 3, stoppedAskingSecs: 4 })).text).toBe(WITHIN);
  });

  it('kept awake on power with a helper that has stopped: no unplug promise', () => {
    const s = kept({ lastPassSecs: 120 });
    expect(texts(s)).toContain('Hollow’s helper isn’t running, so system sleep stays off until it does, unplugged or with the iPhone app off too. Turn this off to turn sleep back on.');
    expect(texts(s).join(' ')).not.toContain('Unplugged, it sleeps');
  });

  it('the unplug promise rests on the restore check: none once its passes stop', () => {
    for (const lastRestoreSecs of [31, 600, null]) {
      const s = kept({ lastRestoreSecs });
      expect(texts(s).join(' ')).not.toContain('Unplugged, it sleeps');
      expect(texts(s)).toContain('Hollow’s helper isn’t running, so system sleep stays off until it does, unplugged or with the iPhone app off too. Turn this off to turn sleep back on.');
    }
    // Sleep still off after Hollow stopped asking, the restore check stopped.
    for (const s of [stillOff({ ready: false, lastRestoreSecs: 90 }), stillOff({ armed: true, source: 'battery', lastRestoreSecs: null })]) {
      expect(headline(s)).toEqual({ tone: 'warn', text: NOT_RUNNING });
    }
    // Armed and asking, sleep not off yet, the restore check stopped.
    expect(headline(status({ enabled: true, armed: true, holding: true, lastRestoreSecs: 45 })).text).toContain('Hollow’s helper isn’t running');
  });
});

describe('system sleep Hollow can’t read, with Hollow’s marker', () => {
  const unread = (change: Partial<PhoneLidStatus> = {}) => stillOff({ sleepDisabled: null, ...change });

  it('never says closing the lid sleeps the Mac, in any state', () => {
    const cases = [
      unread(), unread({ iphoneOn: false, ready: false }), unread({ ready: false }), unread({ armed: true, source: 'battery', batteryPercent: 61 }),
      unread({ armed: true, source: 'unknown', batteryPercent: null }), unread({ otherCopy: '/Users/dev/dev/hollow' }),
      unread({ armed: true, holding: true, stoppedAskingSecs: null }), unread({ enabled: false }), unread({ enabled: false, iphoneOn: false, ready: false }),
      unread({ lastPassSecs: 600, lastRestoreSecs: 600 }), unread({ armed: true, holding: true, stoppedAskingSecs: null, outdated: true }),
    ];
    for (const s of cases) {
      expect(texts(s).join(' ')).not.toMatch(SLEEPS);
      expect(texts(s)).toContain(UNREAD);
    }
  });

  it('counts it as off: sleep should come back, the helper is gone, or the lid can close', () => {
    expect(headline(unread({ ready: false }))).toEqual({ tone: 'warn', text: WITHIN });
    expect(headline(unread({ enabled: false })).text).toBe('Off, but Hollow turned system sleep off earlier and its helper is gone: closing the lid doesn’t sleep this Mac, on battery too. Choose Turn sleep back on.');
    expect(lidOrphan(unread({ enabled: false }))).toBe(true);
    expect(texts(unread({ enabled: false }))).not.toContain('Hollow’s helper is gone but left its note that it turned system sleep off. Choose Turn sleep back on to make sure sleep is on.');
    const asking = unread({ armed: true, holding: true, stoppedAskingSecs: null });
    expect(headline(asking).text).toBe('On power: you can close the lid. Hollow keeps running and your iPhone stays connected.');
    expect(texts(asking).join(' ')).not.toContain('Unplugged');
    expect(headline(unread({ otherCopy: '/Users/dev/dev/hollow' })).text).toContain('system sleep is off now');
  });

  it('without the marker, says the helper can’t turn sleep off', () => {
    const s = status({ enabled: true, armed: true, holding: true, sleepDisabled: null });
    expect(headline(s)).toEqual({ tone: 'warn', text: 'Closing the lid may still sleep this Mac: Hollow can’t read whether system sleep is off, so its helper doesn’t turn it off.' });
    expect(texts(s)).not.toContain(UNREAD);
  });

  it('each click’s notice promises sleep only on a reading', () => {
    const next = unread({ armed: true, holding: true, stoppedAskingSecs: null, source: 'battery' });
    expect(lidDone(true, false, kept({ onBattery: true, source: 'battery' }), next)).toBe('On battery too is off.');
    expect(lidDone(true, false, kept({ onBattery: true }), status({ enabled: true, sleepDisabled: null }))).toBe('On battery too is off.');
    expect(lidDone(false, false, kept(), unread({ enabled: false }))).toBe('Off. Hollow’s helper is removed.');
    expect(lidDone(false, false, unread({ enabled: false }), unread({ enabled: false }))).toBe('Hollow’s helper and its note are removed.');
  });
});

describe('sleep Hollow did not turn off', () => {
  it('says so and leaves it alone, on or off', () => {
    const off = headline(status({ sleepDisabled: true }));
    expect(off.tone).toBe('warn');
    expect(off.text).toContain('Hollow didn’t turn it off (pmset disablesleep)');
    expect(off.text).toContain('Hollow leaves that setting alone.');
    const on = headline(status({ enabled: true, armed: true, sleepDisabled: true, source: 'battery' }));
    expect(on.text).toContain('closing the lid doesn’t sleep it, on power or battery');
  });

  it('never offers Turn sleep back on for someone else’s setting', () => {
    expect(lidOrphan(status({ sleepDisabled: true }))).toBe(false);
  });
});

describe('Hollow’s marker with its helper gone', () => {
  it('offers Turn sleep back on', () => {
    const s = status({ sleepDisabled: true, disabledByHollow: true });
    expect(lidOrphan(s)).toBe(true);
    expect(headline(s).text).toBe('Off, but Hollow turned system sleep off earlier and its helper is gone: closing the lid doesn’t sleep this Mac, on battery too. Choose Turn sleep back on.');
    const stale = status({ disabledByHollow: true });
    expect(lidOrphan(stale)).toBe(true);
    const staleLines = lidLines(stale);
    expect(staleLines[staleLines.length - 1]?.text).toBe('Hollow’s helper is gone but left its note that it turned system sleep off. Choose Turn sleep back on to make sure sleep is on.');
    expect(lidOrphan(kept())).toBe(false);
  });
});

describe('a helper from another version of Hollow', () => {
  it('says so and offers Update the helper', () => {
    const s = kept({ outdated: true });
    expect(texts(s)).toContain('Hollow’s helper is from another version of Hollow. Choose Update the helper to bring it up to date; macOS asks for your password once.');
    expect(lidUpdate(s)).toBe(true);
    expect(lidUpdate(kept())).toBe(false);
    expect(lidUpdate(status({ outdated: true }))).toBe(false);
    // ENG-139: not offered where it can't work (an update installs, which needs the iPhone app on).
    expect(lidUpdate(kept({ outdated: true, iphoneOn: false }))).toBe(false);
    // And an outdated helper's restore check isn't promised to sleep the Mac on unplug.
    expect(texts(s).join(' ')).not.toContain('Unplugged, it sleeps');
    expect(lidUpdate(null)).toBe(false);
  });

  it('without a last pass of its own, it isn’t called stopped', () => {
    const s = kept({ outdated: true, lastPassSecs: null });
    expect(texts(s).join(' ')).not.toContain('isn’t running');
  });
});

describe('the switch and the clicks', () => {
  it('turning on needs the iPhone app; turning off always works', () => {
    expect(lidSwitch(status({ iphoneOn: false, ready: false }), false)).toEqual({ disabled: true, why: 'Available once the iPhone app is on.' });
    expect(lidSwitch(status({ enabled: true, iphoneOn: false, ready: false }), false)).toEqual({ disabled: false, why: null });
    expect(lidSwitch(status(), false)).toEqual({ disabled: false, why: null });
    expect(lidSwitch(status(), true).disabled).toBe(true);
    expect(lidSwitch(status({ available: false }), false).disabled).toBe(true);
    expect(lidSwitch(null, false).disabled).toBe(true);
  });

  it('names what each click did', () => {
    expect(lidDone(true, false, status(), kept())).toBe('On. While an iPhone is paired, this Mac stays awake with its lid closed on power.');
    expect(lidDone(true, true, status(), kept({ onBattery: true }))).toBe('On. While an iPhone is paired, this Mac stays awake with its lid closed, on power and on battery.');
    expect(lidDone(true, true, kept(), kept({ onBattery: true }))).toBe('On battery too: the Mac also stays awake with its lid closed on battery.');
    expect(lidDone(true, false, kept({ onBattery: true }), kept())).toBe('On battery, closing the lid sleeps this Mac again.');
    expect(lidDone(false, false, kept(), status())).toBe('Off. Closing the lid sleeps this Mac again.');
    expect(lidDone(false, false, status({ disabledByHollow: true }), status())).toBe('System sleep is back on.');
    expect(lidDone(true, false, kept({ outdated: true }), kept())).toBe('Hollow’s helper is up to date.');
  });

  it('promises sleep only when system sleep reads on afterwards', () => {
    const otherOff = status({ sleepDisabled: true });
    expect(lidDone(false, false, kept(), otherOff)).toBe('Off. Hollow’s helper is removed.');
    expect(lidDone(false, false, kept(), status({ sleepDisabled: null }))).toBe('Off. Hollow’s helper is removed.');
    expect(lidDone(false, false, status({ disabledByHollow: true }), otherOff)).toBe('Hollow’s helper and its note are removed.');
    // On battery too off while someone else's setting, or a restore still
    // to come on battery, keeps sleep off.
    expect(lidDone(true, false, kept({ onBattery: true }), kept({ disabledByHollow: false }))).toBe('On battery too is off.');
    expect(lidDone(true, false, kept({ onBattery: true, source: 'battery' }), stillOff({ armed: true, source: 'battery' }))).toBe('On battery too is off.');
    for (const [enabled, onBattery, was, next] of [
      [false, false, kept(), otherOff], [false, false, status({ disabledByHollow: true }), otherOff],
      [true, false, kept({ onBattery: true }), kept({ disabledByHollow: false })],
    ] as const) {
      expect(lidDone(enabled, onBattery, was, next)).not.toMatch(SLEEPS);
    }
  });

  it('warns plainly about heat and the battery', () => {
    expect(BATTERY_WARNING).toMatch(/hot/);
    expect(BATTERY_WARNING).toMatch(/battery/);
  });

  it('says nothing for an unavailable Hollow', () => {
    expect(lidLines(status({ available: false }))).toEqual([]);
    expect(lidLines(null)).toEqual([]);
  });
});
