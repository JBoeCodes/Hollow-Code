import { describe, expect, it, vi } from 'vitest';
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../api', () => api);
vi.mock('../nativeEvents', () => ({ onNative: vi.fn(() => () => {}) }));
import type { PeerFailure, PeerMachine, PeerMachinesView, PeerSessionsPage } from '../generated/ipc';
import { cameOnline, followPeerSessions, heldAfterFailure, heldAfterMachines, heldAfterRows, isPeerFailure, loadPeerMachines, peerErrors, peerRowsHeld, refreshPeerSessions } from './peerSessions';
import { mergeChatRows } from './peerMerge';

const machine = (id: string, state: PeerMachine['state']): PeerMachine => ({
  machineId: id, slug: null, name: id, sshHosts: ['runner'], remotePort: 47680, pairedAt: '2026-11-15T00:00:00Z',
  lastSeenAt: null, lastVersion: null, state, message: null, checkedAt: null, alias: null, moves: false,
});
const view = (...machines: PeerMachine[]): PeerMachinesView => ({ thisMac: null, machines, suggestedAlias: null, moveTo: null, sleepTo: null });
const page = (id: string): PeerSessionsPage => ({
  home: { machineId: id, slug: null, name: id },
  sessions: [{ home: { machineId: id, slug: null, name: id }, key: 's-1', title: null, agent: 'claude', workspace: null, createdAt: '', lastActivityAt: '', turnCount: 0, state: 'idle', needsAnswer: false, preview: null, runsOn: null, movedTo: null, movedFrom: null }],
  truncated: false,
});
const failure = (code: PeerFailure['code'], state: PeerFailure['state']): PeerFailure => ({ code, message: "Can't reach it.", state, attemptsLeft: null });

describe('peer rows held in memory', () => {
  it('keeps rows stale when a fetch fails offline, and an answered error changes nothing', () => {
    let held = heldAfterMachines({}, view(machine('m-a', 'online')));
    held = heldAfterRows(held, page('m-a'), 1000);
    expect(held['m-a']).toMatchObject({ state: 'online', fetchedAt: 1000 });
    const offline = heldAfterFailure(held, 'm-a', failure('offline', 'offline'));
    expect(offline['m-a']).toMatchObject({ state: 'offline', fetchedAt: 1000 });
    expect(offline['m-a'].rows).toHaveLength(1);
    expect(heldAfterFailure(held, 'm-a', failure('busy', 'online'))).toBe(held);
    // Answered while marked offline: online again, rows and their time kept.
    const answered = heldAfterFailure(offline, 'm-a', failure('internal', 'online'));
    expect(answered['m-a']).toMatchObject({ state: 'online', fetchedAt: 1000 });
    expect(cameOnline(answered, view(machine('m-a', 'online')))).toEqual([]);
    expect(heldAfterFailure(held, 'm-unknown', failure('offline', 'offline'))).toBe(held);
  });

  it('drops rows when a Mac needs pairing or is forgotten', () => {
    let held = heldAfterRows(heldAfterMachines({}, view(machine('m-a', 'online'), machine('m-b', 'online'))), page('m-a'), 1);
    const revoked = heldAfterFailure(held, 'm-a', failure('pairing-needed', 'pairing-needed'));
    expect(revoked['m-a']).toMatchObject({ state: 'pairing-needed', rows: [], fetchedAt: null });
    held = heldAfterMachines(held, view(machine('m-a', 'pairing-needed'), machine('m-b', 'offline')));
    expect(held['m-a'].rows).toEqual([]);
    expect(held['m-b'].state).toBe('offline');
    held = heldAfterMachines(held, view(machine('m-b', 'offline')));
    expect(Object.keys(held)).toEqual(['m-b']);
  });

  it('knows a peer command error from anything else', () => {
    expect(isPeerFailure(failure('offline', 'offline'))).toBe(true);
    expect(isPeerFailure('Offline')).toBe(false);
    expect(isPeerFailure({ code: 'offline' })).toBe(false);
    expect(isPeerFailure(null)).toBe(false);
  });
});

describe('review fixes', () => {
  it('keeps rows from before a Mac came back online stale until a fresh list', () => {
    let held = heldAfterRows(heldAfterMachines({}, view(machine('m-a', 'online'))), page('m-a'), 1000);
    held = heldAfterFailure(held, 'm-a', failure('offline', 'offline'));
    const back = view(machine('m-a', 'online'));
    expect(cameOnline(held, back)).toEqual(['m-a']);
    held = heldAfterMachines(held, back);
    const opts = { thisMac: null, knownWorkspaces: new Set<string>(), now: 2000 };
    expect(mergeChatRows([], Object.values(held), opts).map(r => r.stale)).toEqual([true]);
    held = heldAfterRows(held, page('m-a'), 3000);
    expect(cameOnline(held, back)).toEqual([]);
    expect(mergeChatRows([], Object.values(held), opts).map(r => r.stale)).toEqual([false]);
  });

  it('drops a late list for a Mac just forgotten', () => {
    let held = heldAfterMachines({}, view(machine('m-a', 'online')));
    held = heldAfterMachines(held, view());
    expect(heldAfterRows(held, page('m-a'), 1)).toBe(held);
    expect(heldAfterRows({}, page('m-a'), 1)).toEqual({});
  });

  it('starts no stream when the last reader left before the Macs were read', async () => {
    let answer!: (v: PeerMachinesView) => void;
    api.invoke.mockReset();
    api.invoke.mockImplementation((command: string) =>
      command === 'peer_machines' ? new Promise(resolve => { answer = resolve; }) : Promise.resolve(null));
    const dispose = followPeerSessions();
    expect(api.invoke).toHaveBeenCalledWith('peer_machines', { check: false });
    dispose();
    answer(view(machine('m-a', 'online')));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(api.invoke.mock.calls.map(c => c[0])).toEqual(['peer_machines']);
    // A reader that stays: the list is fetched and followed.
    const again = followPeerSessions();
    answer(view(machine('m-a', 'online')));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(api.invoke.mock.calls.map(c => c[0]).sort()).toEqual(['peer_machines', 'peer_machines', 'peer_sessions', 'peer_watch']);
    again();
  });

  it('fetches a Mac once at a time and orders its watch before its unwatch', async () => {
    api.invoke.mockReset();
    let release!: () => void;
    const log: string[] = [];
    api.invoke.mockImplementation((command: string) => {
      log.push(command);
      if (command === 'peer_sessions') return new Promise(resolve => { release = () => resolve(page('m-a')); });
      if (command === 'peer_watch') return new Promise(resolve => setTimeout(() => { log.push('watched'); resolve(null); }, 20));
      if (command === 'peer_machines') return Promise.resolve(view(machine('m-a', 'online')));
      return Promise.resolve(null);
    });
    const one = refreshPeerSessions('m-a');
    const two = refreshPeerSessions('m-a');
    const three = refreshPeerSessions('m-a');
    expect(two).toBe(one);
    expect(three).toBe(one);
    expect(api.invoke.mock.calls.filter(c => c[0] === 'peer_sessions')).toHaveLength(1);
    release();
    // Asked again while it ran: fetched once more after, not twice.
    await vi.waitFor(() => expect(api.invoke.mock.calls.filter(c => c[0] === 'peer_sessions')).toHaveLength(2));
    release();
    await one;
    // The last reader leaves while its watch is still in flight.
    api.invoke.mockClear();
    const dispose = followPeerSessions();
    await new Promise(resolve => setTimeout(resolve, 0));
    dispose();
    await new Promise(resolve => setTimeout(resolve, 60));
    // The unwatch is sent only once the watch it ends has landed.
    expect(log.filter(c => c !== 'peer_machines' && c !== 'peer_sessions')).toEqual(['peer_watch', 'watched', 'peer_unwatch']);
    release();
  });

  it('drops a failure from a fetch that started before the Mac was paired again', async () => {
    api.invoke.mockReset();
    let fail!: (e: unknown) => void;
    let paired = '2026-11-15T00:00:00Z';
    api.invoke.mockImplementation((command: string) => {
      if (command === 'peer_machines') return Promise.resolve(view({ ...machine('m-b', 'online'), pairedAt: paired }));
      if (command === 'peer_sessions') return new Promise((_, reject) => { fail = reject; });
      return Promise.resolve(null);
    });
    await loadPeerMachines();
    const fetching = refreshPeerSessions('m-b');
    // Paired again while the old token's fetch runs; it then answers 401.
    paired = '2026-11-15T01:00:00Z';
    await loadPeerMachines();
    fail(failure('pairing-needed', 'pairing-needed'));
    await fetching;
    expect(peerRowsHeld().find(h => h.machine.machineId === 'm-b')?.state).toBe('online');
    expect(peerErrors()['m-b']).toBeUndefined();
    // A failure from a fetch of the current pairing still counts.
    const now = refreshPeerSessions('m-b');
    await vi.waitFor(() => expect(fail).toBeTypeOf('function'));
    fail(failure('pairing-needed', 'pairing-needed'));
    await now;
    expect(peerRowsHeld().find(h => h.machine.machineId === 'm-b')?.state).toBe('pairing-needed');
  });
});
