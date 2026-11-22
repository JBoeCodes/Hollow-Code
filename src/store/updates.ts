import { createSignal } from 'solid-js';
import { api } from '../api';
import type { UpdateInstallOutcome, UpdateLastResult, UpdateState, UpdateWork } from '../generated/ipc';
import { onNative } from '../nativeEvents';
import { errMsg } from '../utils';

const [updateState, setUpdateState] = createSignal<UpdateState | null>(null);
const [updateWork, setUpdateWork] = createSignal<UpdateWork[]>([]);
const [lastUpdateResult, setLastUpdateResult] = createSignal<UpdateLastResult | null>(null);
const [updateError, setUpdateError] = createSignal<string | null>(null);
const [installing, setInstalling] = createSignal(false);
export { updateState, updateWork, lastUpdateResult, updateError, installing as updateInstalling };

export function clearUpdateError() { setUpdateError(null); }

export function updateRelease() {
  const phase = updateState()?.phase;
  return phase?.phase === 'ready' || phase?.phase === 'installing' || phase?.phase === 'available' || phase?.phase === 'downloading' ? phase.release : null;
}

export function updateReady(): boolean {
  const state = updateState();
  if (!state || installing()) return false;
  // An armed wait keeps its Cancel in view whatever the phase (a recheck can run while it waits).
  if (state.waiting) return true;
  return state.phase.phase === 'ready' && !state.dismissed;
}

export const updateWaiting = () => updateState()?.waiting ?? false;

let revision = 0;
let stop: (() => void) | undefined;
let subscribers = 0;

function acceptState(state: UpdateState) {
  revision += 1;
  const was = updateState()?.phase.phase;
  if (state.phase.phase !== 'ready' || (updateWaiting() && !state.waiting)) setUpdateWork([]);
  setUpdateState(state);
  // A backend rollback (Installing, then Ready again with a note) must free the button.
  if ((was === 'installing' && state.phase.phase !== 'installing') || (state.phase.phase !== 'ready' && state.phase.phase !== 'installing')) setInstalling(false);
}

async function readState(read: () => Promise<UpdateState>): Promise<UpdateState | null> {
  setUpdateError(null);
  const before = revision;
  try {
    const state = await read();
    // A native event delivered during the read owns the newer state.
    if (revision === before) acceptState(state);
    return state;
  } catch (error) {
    setUpdateError(errMsg(error));
    return null;
  }
}

export function refreshUpdateState() {
  return readState(api.updateGetState);
}

/** One shared subscription and initial read for each active lifetime. */
export function startUpdates(): () => void {
  subscribers += 1;
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    subscribers -= 1;
    if (subscribers === 0) stop?.();
  };
  if (stop) return release;
  let active = true;
  const before = revision;
  const off = onNative<UpdateState>('update-state', acceptState, error => {
    if (active) setUpdateError(errMsg(error));
  });
  stop = () => {
    if (!active) return;
    active = false;
    off();
    stop = undefined;
  };
  setUpdateError(null);
  void api.updateGetState().then(state => {
    if (active && revision === before) acceptState(state);
  }).catch(error => { if (active) setUpdateError(errMsg(error)); });
  void api.updateLastResult().then(result => {
    if (active) setLastUpdateResult(result);
  }).catch(error => { if (active) setUpdateError(errMsg(error)); });
  return release;
}

export async function restartToUpdate(confirmedKeys?: string[] | null): Promise<UpdateInstallOutcome | null> {
  setUpdateError(null);
  setInstalling(true);
  try {
    const outcome = await api.updateInstallNow(confirmedKeys);
    if (outcome.outcome === 'workRunning' || outcome.outcome === 'workChanged') {
      setUpdateWork(outcome.work);
      setInstalling(false);
    } else {
      setUpdateWork([]);
    }
    return outcome;
  } catch (error) {
    setInstalling(false);
    setUpdateError(errMsg(error));
    return null;
  }
}

export function updateWhenChatsFinish() {
  return readState(api.updateInstallWhenIdle);
}

export function cancelUpdateWait() {
  setUpdateWork([]);
  return readState(api.updateCancelWait);
}

export async function dismissUpdate() {
  if (updateWaiting()) {
    const state = await cancelUpdateWait();
    if (!state || state.waiting) return null;
  }
  const release = updateRelease();
  return release ? readState(() => api.updateDismiss(release.version)) : Promise.resolve(null);
}

/** Changes, Hollow app: Show in Update button (row U10). Throws the reason, for the panel. */
export async function offerLocalBuild(id: string): Promise<UpdateState> {
  const state = await api.updateOfferLocalBuild(id);
  acceptState(state);
  return state;
}

/** The build id of the Local build waiting in the Update button, if any. */
export function localBuildInButton(): string | null {
  const phase = updateState()?.phase;
  return phase?.phase === 'ready' || phase?.phase === 'installing' ? phase.release.local?.buildId ?? null : null;
}

export function checkForUpdatesNow() {
  return readState(api.updateCheckNow);
}

export async function ackLastUpdateResult() {
  setUpdateError(null);
  try {
    await api.updateAckLastResult();
    setLastUpdateResult(null);
  } catch (error) {
    setUpdateError(errMsg(error));
  }
}
