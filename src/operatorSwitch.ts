// The "Share About you with agents" switch at launch (wave 2 follow-up).
//
// The backend persists the switch (`operator-context.json` in Hollow's
// app-data folder) and its value is the source of truth: chat turns and
// workflow runs read it, including ones that start before this window has
// loaded. The window adopts the backend's value at launch and pushes only an
// explicit change (`setOperatorContextEnabled` in store/runtime.ts). Its
// localStorage copy is a display cache for the moment before the backend
// answers, and is never pushed back: it used to be, at every launch, and a
// lost key read as "on" turned a person's "off" back on.
//
// Pure (no store, no Tauri) so the launch path is unit-tested.

export const OPERATOR_SWITCH_KEY = 'operatorContextEnabled';

/** `operator_context_switch`: the persisted switch, and whether a choice
 *  is recorded (false only for a never-set switch, whose `enabled` is the
 *  backend's default). */
export interface OperatorSwitchState {
  enabled: boolean;
  chosen: boolean;
}

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

function readCopy(storage: Storage | null): string | null {
  try { return storage?.getItem(OPERATOR_SWITCH_KEY) ?? null; } catch { return null; }
}

/** What the window shows until the backend answers: its own copy, and off
 *  when that is missing or unreadable (a missing answer is not consent). */
export function cachedOperatorSwitch(storage: Storage | null): boolean {
  return readCopy(storage) === 'true';
}

/**
 * Adopt the backend's switch at launch. Returns the value to show, or `null`
 * when there is no backend (not in Tauri: keep the cached value).
 *
 * Nothing is pushed, with one exception: when the backend has never recorded
 * a choice and this window holds an explicit "off" (set before the backend
 * persisted the switch), that off is carried over once. A window copy can
 * only ever turn the switch off this way, never on.
 */
export async function adoptOperatorSwitch(deps: {
  get: () => Promise<OperatorSwitchState>;
  set: (enabled: boolean) => Promise<void>;
  storage: Storage | null;
}): Promise<boolean | null> {
  let state: OperatorSwitchState;
  try {
    state = await deps.get();
  } catch {
    return null;
  }
  let enabled = state.enabled;
  if (!state.chosen && enabled && readCopy(deps.storage) === 'false') {
    try {
      await deps.set(false);
      enabled = false;
    } catch { /* not carried over: show what the backend will do */ }
  }
  try { deps.storage?.setItem(OPERATOR_SWITCH_KEY, String(enabled)); } catch { /* storage unavailable */ }
  return enabled;
}
