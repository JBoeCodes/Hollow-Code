import { describe, expect, it } from 'vitest';
import type { MachineRef, PeerSessionRow } from '../generated/ipc';
import { STALE_DROP_MS, mergeChatRows, rowsForMachine, rowsForWorkspace, runningOn, type LocalChatRow, type PeerRowsHeld } from './peerMerge';

const HERE: MachineRef = { machineId: 'm-3f9c1a07e2b45d18', slug: 'macbook', name: 'MacBook Pro' };
const MINI: MachineRef = { machineId: 'm-8d21c4be90f17a3e', slug: 'build-mac', name: 'Mac mini' };
const STUDIO: MachineRef = { machineId: 'm-00000000000000aa', slug: null, name: 'Studio' };
const NOW = Date.parse('2026-11-16T12:00:00Z');

const local = (key: string, at: string, workspaceId: string | null = 'w-local'): LocalChatRow => ({ key, title: key, lastActivityAt: at, workspaceId });
const remote = (home: MachineRef, key: string, at: string, workspace: string | null, extra: Partial<PeerSessionRow> = {}): PeerSessionRow => ({
  home, key, title: key, agent: 'claude', workspace: workspace ? { id: workspace, title: workspace, kind: 'project' } : null,
  createdAt: at, lastActivityAt: at, turnCount: 1, state: 'idle', needsAnswer: false, preview: null, runsOn: null, movedTo: null, movedFrom: null, ...extra,
});
const held = (machine: MachineRef, rows: PeerSessionRow[], state: PeerRowsHeld['state'] = 'online', fetchedAt: number | null = NOW): PeerRowsHeld => ({ machine, state, rows, fetchedAt });
const opts = { thisMac: HERE, knownWorkspaces: new Set(['w-local', 'w-shared']), now: NOW };

describe('mergeChatRows', () => {
  it('shows a moved chat once, at its new home, and keeps the old side while that Mac lists nothing', () => {
    const moved = { ...local('s-m', '2026-11-16T10:00:00Z'), movedTo: MINI.machineId };
    const there = remote(MINI, 's-m', '2026-11-16T11:00:00Z', 'w-local', { movedFrom: HERE.machineId });
    const rows = mergeChatRows([moved, local('s-k', '2026-11-16T09:00:00Z')], [held(MINI, [there])], opts);
    expect(rows.map(r => r.id)).toEqual([`${MINI.machineId}:s-m`, `${HERE.machineId}:s-k`]);
    expect(rows[0].movedFrom).toBe(HERE.machineId);
    // Stale rows still count: the chat shows there, dimmed.
    const stale = mergeChatRows([moved], [held(MINI, [there], 'offline', NOW - 60_000)], opts);
    expect(stale.map(r => [r.id, r.stale])).toEqual([[`${MINI.machineId}:s-m`, true]]);
    // That Mac's rows gone (a day offline, or forgotten): the old side stays, marked moved.
    for (const peers of [[held(MINI, [there], 'offline', NOW - STALE_DROP_MS - 1)], [held(MINI, [there], 'pairing-needed')], []]) {
      const kept = mergeChatRows([moved], peers, opts);
      expect(kept.map(r => [r.id, r.movedTo])).toEqual([[`${HERE.machineId}:s-m`, MINI.machineId]]);
    }
    // A chat that moved back from another Mac: the Mac it left hides its side.
    const back = remote(STUDIO, 's-b', '2026-11-16T08:00:00Z', null, { movedTo: HERE.machineId });
    const home = mergeChatRows([local('s-b', '2026-11-16T12:00:00Z')], [held(STUDIO, [back])], opts);
    expect(home.map(r => r.id)).toEqual([`${HERE.machineId}:s-b`]);
  });

  it('keeps the same key on two Macs as two rows, each with its home', () => {
    const rows = mergeChatRows(
      [local('s-1', '2026-11-16T10:00:00Z')],
      [held(MINI, [remote(MINI, 's-1', '2026-11-16T11:00:00Z', 'w-shared')]), held(STUDIO, [remote(STUDIO, 's-1', '2026-11-16T09:00:00Z', null)])],
      opts,
    );
    expect(rows.map(r => r.id)).toEqual([`${MINI.machineId}:s-1`, `${HERE.machineId}:s-1`, `${STUDIO.machineId}:s-1`]);
    expect(rows.map(r => r.home?.name ?? 'here')).toEqual(['Mac mini', 'here', 'Studio']);
    expect(new Set(rows.map(r => r.id)).size).toBe(3);
  });

  it('orders newest first across Macs, ties by id, unreadable times last', () => {
    const rows = mergeChatRows(
      [local('s-a', '2026-11-16T08:00:00Z'), local('s-b', 'not a time')],
      [held(MINI, [remote(MINI, 's-c', '2026-11-16T09:00:00Z', null), remote(MINI, 's-d', '2026-11-16T08:00:00Z', null)])],
      opts,
    );
    expect(rows.map(r => r.key)).toEqual(['s-c', 's-a', 's-d', 's-b']);
  });

  it('puts a row of a workspace known here in that project, an unknown one under its Mac', () => {
    const rows = mergeChatRows(
      [local('s-l', '2026-11-16T08:00:00Z', 'w-shared')],
      [held(MINI, [
        remote(MINI, 's-known', '2026-11-16T09:00:00Z', 'w-shared'),
        remote(MINI, 's-mini-only', '2026-11-16T10:00:00Z', 'w-mini'),
        remote(MINI, 's-none', '2026-11-16T07:00:00Z', null, { state: 'running', needsAnswer: true }),
      ])],
      opts,
    );
    expect(rowsForWorkspace(rows, 'w-shared').map(r => r.key)).toEqual(['s-known', 's-l']);
    expect(rowsForMachine(rows, MINI.machineId).map(r => r.key)).toEqual(['s-mini-only', 's-none']);
    const none = rows.find(r => r.key === 's-none')!;
    expect(none.needsAnswer).toBe(true);
    expect(runningOn(rows, MINI.machineId)).toBe(1);
    expect(rows.find(r => r.key === 's-l')!.home).toBeNull();
  });

  it('marks an unreachable Mac stale, drops its rows after 24 h and hides them when it needs pairing', () => {
    const rows = [remote(MINI, 's-x', '2026-11-16T09:00:00Z', null, { state: 'running' })];
    const offline = mergeChatRows([], [held(MINI, rows, 'offline', NOW - 4 * 60_000)], opts);
    expect(offline.map(r => [r.key, r.stale])).toEqual([['s-x', true]]);
    expect(runningOn(offline, MINI.machineId)).toBe(0);
    for (const state of ['connecting', 'update-needed'] as const) {
      expect(mergeChatRows([], [held(MINI, rows, state)], opts)[0].stale).toBe(true);
    }
    expect(mergeChatRows([], [held(MINI, rows, 'offline', NOW - STALE_DROP_MS - 1)], opts)).toEqual([]);
    expect(mergeChatRows([], [held(MINI, rows, 'pairing-needed')], opts)).toEqual([]);
    expect(mergeChatRows([], [held(MINI, rows, 'online', null)], opts)).toEqual([]);
    expect(mergeChatRows([], [held(MINI, rows, 'online')], opts)[0].stale).toBe(false);
  });

  it('never shows this Mac twice, and works before this Mac is known', () => {
    const self = mergeChatRows([local('s-1', '2026-11-16T08:00:00Z')], [held(HERE, [remote(HERE, 's-1', '2026-11-16T08:00:00Z', null)])], opts);
    expect(self.map(r => r.id)).toEqual([`${HERE.machineId}:s-1`]);
    const unknown = mergeChatRows([local('s-1', '2026-11-16T08:00:00Z')], [], { ...opts, thisMac: null });
    expect(unknown[0].id).toBe('this-mac:s-1');
  });
});
