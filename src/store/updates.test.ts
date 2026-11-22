import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { UpdatePhase, UpdateState } from '../generated/ipc';

const api = vi.hoisted(() => ({
  updateGetState: vi.fn(), updateLastResult: vi.fn(), updateInstallNow: vi.fn(),
  updateInstallWhenIdle: vi.fn(), updateCancelWait: vi.fn(), updateDismiss: vi.fn(),
  updateCheckNow: vi.fn(), updateAckLastResult: vi.fn(),
}));
const events = vi.hoisted(() => ({
  handler: undefined as ((state: UpdateState) => void) | undefined,
  off: vi.fn(),
  onNative: vi.fn(),
}));
vi.mock('../api', () => ({ api }));
vi.mock('../nativeEvents', () => ({ onNative: events.onNative }));
vi.mock('../utils', () => ({ errMsg: (error: unknown) => error instanceof Error ? error.message : String(error) }));

const release = {
  version: '1.2.3', notes: 'Update notes', pubDate: null,
  channel: 'main' as const, commit: null, downgrade: false, local: null,
};
const state = (phase: UpdatePhase = { phase: 'ready', release }, extra: Partial<UpdateState> = {}): UpdateState => ({
  phase, runningVersion: '1.2.2', channel: 'main', configured: true, connected: true, pendingConnect: false,
  waiting: false, goBackWaiting: false, dismissed: false, skippedVersion: null, downgradeAllowed: false,
  lastCheckedAt: null, note: null, allowedChannels: ['main', 'nightly'], personName: null, ...extra,
});
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
let store: typeof import('./updates');
let stop: (() => void) | undefined;

beforeEach(async () => {
  vi.resetModules();
  vi.resetAllMocks();
  events.handler = undefined;
  events.onNative.mockImplementation((_name: string, handler: (value: UpdateState) => void) => {
    events.handler = handler;
    return events.off;
  });
  api.updateGetState.mockResolvedValue(state());
  api.updateLastResult.mockResolvedValue(null);
  store = await import('./updates');
});

afterEach(() => { stop?.(); stop = undefined; });

describe('updates store', () => {
  it('offers only a ready, undismissed update and hides an installation in progress', async () => {
    expect(store.updateReady()).toBe(false);
    expect(store.updateRelease()).toBeNull();
    stop = store.startUpdates();
    await flush();
    expect(store.updateReady()).toBe(true);
    expect(store.updateRelease()).toEqual(release);
    events.handler!(state(undefined, { dismissed: true }));
    expect(store.updateReady()).toBe(false);
    for (const phase of [
      { phase: 'idle' }, { phase: 'checking' }, { phase: 'available', release },
      { phase: 'downloading', release, percent: 50 }, { phase: 'installing', release },
      { phase: 'failed', reason: 'Unavailable' },
    ] satisfies UpdatePhase[]) {
      events.handler!(state(phase));
      expect(store.updateReady()).toBe(false);
      expect(store.updateRelease()).toEqual('release' in phase ? release : null);
    }
    events.handler!(state());
    let finish!: (outcome: { outcome: 'started'; version: string }) => void;
    api.updateInstallNow.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const installing = store.restartToUpdate();
    expect(store.updateReady()).toBe(false);
    finish({ outcome: 'started', version: release.version });
    await installing;
    expect(store.updateReady()).toBe(false);
  });

  it('receives native state and waiting changes', async () => {
    stop = store.startUpdates();
    await flush();
    const next = state(undefined, { waiting: true });
    events.handler!(next);
    expect(store.updateState()).toEqual(next);
    expect(store.updateWaiting()).toBe(true);
    expect(events.onNative).toHaveBeenCalledWith('update-state', expect.any(Function), expect.any(Function));
  });

  it.each(['workRunning', 'workChanged'] as const)('returns %s and retains its running work without throwing', async outcome => {
    const work = [{ key: 'chat-1', name: 'Project chat' }];
    const result = { outcome, work };
    api.updateInstallNow.mockResolvedValueOnce(result);
    expect(store.updateWork()).toEqual([]);
    await expect(store.restartToUpdate(['chat-1'])).resolves.toEqual(result);
    expect(store.updateWork()).toEqual(work);
    expect(store.updateError()).toBeNull();
    expect(api.updateInstallNow).toHaveBeenCalledWith(['chat-1']);
  });

  it('shares a subscription until every caller releases its own reference', async () => {
    stop = store.startUpdates();
    const secondStop = store.startUpdates();
    await flush();
    expect(api.updateGetState).toHaveBeenCalledTimes(1);
    expect(api.updateLastResult).toHaveBeenCalledTimes(1);
    expect(events.onNative).toHaveBeenCalledTimes(1);
    stop(); stop();
    expect(events.off).not.toHaveBeenCalled();
    events.handler!(state(undefined, { waiting: true }));
    expect(store.updateWaiting()).toBe(true);
    secondStop(); secondStop();
    expect(events.off).toHaveBeenCalledTimes(1);
    stop = store.startUpdates();
    await flush();
    expect(events.onNative).toHaveBeenCalledTimes(2);
  });

  it('keeps a dismissed armed update visible and cancels before dismissing it', async () => {
    stop = store.startUpdates();
    await flush();
    events.handler!(state(undefined, { waiting: true, dismissed: true }));
    expect(store.updateReady()).toBe(true);
    let finish!: (value: UpdateState) => void;
    api.updateCancelWait.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    api.updateDismiss.mockResolvedValueOnce(state(undefined, { dismissed: true }));
    const dismissing = store.dismissUpdate();
    expect(api.updateCancelWait).toHaveBeenCalledTimes(1);
    expect(api.updateDismiss).not.toHaveBeenCalled();
    finish(state());
    await dismissing;
    expect(api.updateDismiss).toHaveBeenCalledWith(release.version);
    expect(store.updateReady()).toBe(false);
  });

  it('does not dismiss an armed update when cancelling fails', async () => {
    stop = store.startUpdates();
    await flush();
    events.handler!(state(undefined, { waiting: true }));
    api.updateCancelWait.mockRejectedValueOnce(new Error('Could not cancel'));
    await expect(store.dismissUpdate()).resolves.toBeNull();
    expect(api.updateDismiss).not.toHaveBeenCalled();
    expect(store.updateReady()).toBe(true);
  });

  it('frees the button when the backend rolls an install back to ready', async () => {
    stop = store.startUpdates();
    await flush();
    api.updateInstallNow.mockResolvedValue({ outcome: 'started', version: '1.2.3' });
    await store.restartToUpdate();
    expect(store.updateReady()).toBe(false);
    events.handler!(state({ phase: 'installing', release }));
    events.handler!(state({ phase: 'ready', release }, { note: 'Work started, so the update waits.' }));
    expect(store.updateReady()).toBe(true);
  });

  it('keeps an armed wait in view while a recheck changes the phase', async () => {
    stop = store.startUpdates();
    await flush();
    events.handler!(state({ phase: 'checking' }, { waiting: true }));
    expect(store.updateReady()).toBe(true);
    events.handler!(state({ phase: 'checking' }, { waiting: false }));
    expect(store.updateReady()).toBe(false);
  });

  it('clears running work immediately on cancel, on a wait ending, and outside ready', async () => {
    stop = store.startUpdates();
    await flush();
    const work = [{ key: 'chat-1', name: 'Project chat' }];
    api.updateInstallNow.mockResolvedValue({ outcome: 'workRunning', work });
    await store.restartToUpdate();
    api.updateCancelWait.mockResolvedValueOnce(state());
    const cancelling = store.cancelUpdateWait();
    expect(store.updateWork()).toEqual([]);
    await cancelling;
    await store.restartToUpdate();
    events.handler!(state(undefined, { waiting: true }));
    expect(store.updateWork()).toEqual(work);
    events.handler!(state());
    expect(store.updateWork()).toEqual([]);
    await store.restartToUpdate();
    events.handler!(state({ phase: 'checking' }));
    expect(store.updateWork()).toEqual([]);
  });

  it('keeps an event newer than the initial read', async () => {
    let finish!: (value: UpdateState) => void;
    api.updateGetState.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    stop = store.startUpdates();
    const next = state(undefined, { waiting: true });
    events.handler!(next);
    finish(state({ phase: 'idle' }));
    await flush();
    expect(store.updateState()).toEqual(next);
  });

  it('exposes backend failures as readable errors', async () => {
    api.updateInstallNow.mockRejectedValueOnce(new Error('Installer unavailable'));
    await expect(store.restartToUpdate()).resolves.toBeNull();
    expect(store.updateError()).toBe('Installer unavailable');
    api.updateGetState.mockRejectedValueOnce(new Error('State unavailable'));
    await expect(store.refreshUpdateState()).resolves.toBeNull();
    expect(store.updateError()).toBe('State unavailable');
    store.clearUpdateError();
    expect(store.updateError()).toBeNull();
  });

  it('dismisses the ready version and applies wait, cancel and check responses', async () => {
    stop = store.startUpdates();
    await flush();
    api.updateDismiss.mockResolvedValueOnce(state(undefined, { dismissed: true }));
    await store.dismissUpdate();
    expect(api.updateDismiss).toHaveBeenCalledWith(release.version);
    expect(store.updateReady()).toBe(false);
    api.updateInstallWhenIdle.mockResolvedValueOnce(state(undefined, { waiting: true }));
    await store.updateWhenChatsFinish();
    expect(store.updateWaiting()).toBe(true);
    api.updateCancelWait.mockResolvedValueOnce(state());
    await store.cancelUpdateWait();
    expect(store.updateWaiting()).toBe(false);
    api.updateCheckNow.mockResolvedValueOnce(state({ phase: 'checking' }));
    await store.checkForUpdatesNow();
    expect(store.updateState()?.phase.phase).toBe('checking');
  });

  it('loads the last result and clears it only after a successful acknowledgement', async () => {
    const result = { kind: 'updated', version: release.version, notes: release.notes, channel: 'main', at: '2026-11-15' };
    api.updateLastResult.mockResolvedValueOnce(result);
    stop = store.startUpdates();
    await flush();
    expect(store.lastUpdateResult()).toEqual(result);
    api.updateAckLastResult.mockRejectedValueOnce(new Error('Could not acknowledge'));
    await store.ackLastUpdateResult();
    expect(store.lastUpdateResult()).toEqual(result);
    api.updateAckLastResult.mockResolvedValueOnce(undefined);
    await store.ackLastUpdateResult();
    expect(store.lastUpdateResult()).toBeNull();
  });
});
