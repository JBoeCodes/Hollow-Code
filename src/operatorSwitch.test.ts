import { describe, expect, it } from 'vitest';
import { OPERATOR_SWITCH_KEY, adoptOperatorSwitch, cachedOperatorSwitch } from './operatorSwitch';
import runtimeSource from './store/runtime.ts?raw';

/**
 * Wave 2 follow-up (red first). The backend persists the "Share About you
 * with agents" switch (`operator-context.json`) and is its source of truth.
 * The window used to push its own localStorage copy to the backend at every
 * launch, and that copy read a missing key (or a throwing localStorage) as
 * on, so a lost key turned a person's "off" back on at the next launch.
 */

/** A localStorage stand-in: `null` entries are missing keys. */
function storage(entries: Record<string, string> = {}) {
  const map = new Map(Object.entries(entries));
  return {
    map,
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
  };
}
const throwing = {
  getItem: (): string | null => { throw new Error('storage unavailable'); },
  setItem: (): void => { throw new Error('storage unavailable'); },
};

/** The backend: its persisted state, and every value pushed to it. */
function backend(enabled: boolean, chosen: boolean) {
  const pushed: boolean[] = [];
  return {
    pushed,
    get: async () => ({ enabled, chosen }),
    set: async (v: boolean) => { pushed.push(v); },
  };
}

describe('the About you switch at launch', () => {
  it('stays off when the backend says off and the window lost its copy', async () => {
    const be = backend(false, true);
    const store = storage();
    expect(await adoptOperatorSwitch({ ...be, storage: store })).toBe(false);
    expect(be.pushed, 'nothing is pushed at launch').toEqual([]);
    expect(store.map.get(OPERATOR_SWITCH_KEY)).toBe('false');
    // localStorage itself unavailable: still the backend's off.
    const be2 = backend(false, true);
    expect(await adoptOperatorSwitch({ ...be2, storage: throwing })).toBe(false);
    expect(be2.pushed).toEqual([]);
  });

  it("adopts the backend's on over a stale window copy", async () => {
    const be = backend(true, true);
    const store = storage({ [OPERATOR_SWITCH_KEY]: 'false' });
    expect(await adoptOperatorSwitch({ ...be, storage: store })).toBe(true);
    expect(be.pushed).toEqual([]);
    expect(store.map.get(OPERATOR_SWITCH_KEY)).toBe('true');
  });

  it("carries an earlier off over once, when the backend has never recorded a choice", async () => {
    const be = backend(true, false);
    const store = storage({ [OPERATOR_SWITCH_KEY]: 'false' });
    expect(await adoptOperatorSwitch({ ...be, storage: store })).toBe(false);
    expect(be.pushed, 'the one migration push is an off').toEqual([false]);
    // A window "on" never overrides the backend: it is the default anyway.
    const fresh = backend(true, false);
    expect(await adoptOperatorSwitch({ ...fresh, storage: storage({ [OPERATOR_SWITCH_KEY]: 'true' }) })).toBe(true);
    expect(fresh.pushed).toEqual([]);
    // Nothing recorded anywhere: the backend's own default.
    const none = backend(true, false);
    expect(await adoptOperatorSwitch({ ...none, storage: storage() })).toBe(true);
    expect(none.pushed).toEqual([]);
  });

  it('keeps the window copy when there is no backend (not in Tauri)', async () => {
    const offline = {
      get: async () => { throw new Error('not in tauri'); },
      set: async () => { throw new Error('not in tauri'); },
    };
    expect(await adoptOperatorSwitch({ ...offline, storage: storage() })).toBeNull();
  });

  it('shows off, not on, until the backend answers when the window copy is missing', () => {
    expect(cachedOperatorSwitch(storage())).toBe(false);
    expect(cachedOperatorSwitch(throwing)).toBe(false);
    expect(cachedOperatorSwitch(storage({ [OPERATOR_SWITCH_KEY]: 'true' }))).toBe(true);
    expect(cachedOperatorSwitch(storage({ [OPERATOR_SWITCH_KEY]: 'false' }))).toBe(false);
  });

  it('runtime.ts adopts the backend value at launch and pushes only on a change', () => {
    const flat = runtimeSource.replace(/\s+/g, '');
    expect(flat, 'the cached copy is never pushed at launch')
      .not.toContain('api.setOperatorContextEnabled(operatorContextEnabled())');
    expect(flat).toContain('adoptOperatorSwitch(');
    // Every push is the explicit setter's, or the adopter's `set` (its one
    // carried-over off, tested above).
    const setter = flat.slice(flat.indexOf('exportfunctionsetOperatorContextEnabled('));
    const setterBody = setter.slice(0, setter.indexOf('}export'));
    let at = flat.indexOf('api.setOperatorContextEnabled(');
    let pushes = 0;
    while (at !== -1) {
      pushes += 1;
      const inSetter = flat.indexOf(setterBody) <= at && at < flat.indexOf(setterBody) + setterBody.length;
      const inAdopter = flat.slice(at - 'set:v=>'.length, at) === 'set:v=>';
      expect(inSetter || inAdopter, `a push outside the setter and the adopter at ${at}`).toBe(true);
      at = flat.indexOf('api.setOperatorContextEnabled(', at + 1);
    }
    expect(pushes).toBe(2);
  });
});
