import { describe, expect, it } from 'vitest';
import type { PeerMachine, PeerMachinesView, PeerSessionRow } from '../generated/ipc';
import type { PeerRowsHeld } from '../store/peerMerge';
import { agoLabel, chatHomeGone, badgeLabel, libraryMacs, libraryRows, machineRows, machineWord, popoverInfo, remoteMenu, runningCount, stateText, stripRows } from './machineStrip';

const NOW = Date.parse('2026-11-15T12:00:00Z');
const ago = (min: number) => new Date(NOW - min * 60_000).toISOString();
const mini = (state: PeerMachine['state'], more: Partial<PeerMachine> = {}): PeerMachine => ({
  machineId: 'm-mini', slug: 'build-mac', name: 'Mac mini', sshHosts: ['runner-host', 'runner-host-lan'], remotePort: 47680,
  pairedAt: ago(600), lastSeenAt: ago(4), lastVersion: '0.5.17 (abc1234)', state, message: null, checkedAt: ago(1), alias: 'runner-host', moves: true, ...more,
});
const view = (...machines: PeerMachine[]): PeerMachinesView => ({
  thisMac: { machineId: 'm-book', slug: 'dev-mac', name: 'MacBook Pro' }, machines, suggestedAlias: 'runner-host', moveTo: null, sleepTo: null,
});
const row = (key: string, state: string, at: string): PeerSessionRow => ({
  home: { machineId: 'm-mini', slug: 'build-mac', name: 'Mac mini' }, key, title: key, agent: 'claude', workspace: null,
  createdAt: at, lastActivityAt: at, turnCount: 2, state, needsAnswer: false, preview: null, runsOn: null, movedTo: null, movedFrom: null,
});
const held = (state: PeerMachine['state'], rows: PeerSessionRow[], fetchedAt = NOW - 60_000): PeerRowsHeld => ({
  machine: { machineId: 'm-mini', slug: 'build-mac', name: 'Mac mini' }, state, rows, fetchedAt,
});

describe('the machine strip', () => {
  it('shows nothing while no Mac is paired', () => {
    expect(stripRows(null, [], 0, NOW)).toEqual([]);
    expect(stripRows(view(), [], 3, NOW)).toEqual([]);
  });

  it('puts this Mac first, then each paired Mac', () => {
    const rows = stripRows(view(mini('online')), [held('online', [row('a', 'running', ago(1)), row('b', 'running', ago(2)), row('c', 'idle', ago(3))])], 1, NOW);
    expect(rows.map(r => [r.name, r.dot, r.text, r.running, r.click])).toEqual([
      ['MacBook Pro', 'this-mac', 'This Mac', 1, 'none'],
      ['Mac mini', 'online', '2 running', 2, 'popover'],
    ]);
    expect(rows[1].title).toBe('Mac mini (build-mac) · 2 running');
    expect(rows[1].label).toBe('Mac mini, 2 running');
  });

  it('maps every state to its dot, words and click', () => {
    const cases: [PeerMachine['state'], Partial<PeerMachine>, string, string, string][] = [
      ['online', {}, 'online', 'Online', 'popover'],
      ['connecting', {}, 'connecting', 'Connecting…', 'popover'],
      ['offline', {}, 'offline', 'Offline · 4 min ago', 'popover'],
      ['offline', { lastSeenAt: null }, 'offline', 'Offline', 'popover'],
      ['pairing-needed', {}, 'attention', 'Pair again', 'pair'],
      ['update-needed', {}, 'attention', 'Needs an update', 'popover'],
    ];
    for (const [state, more, dot, text, click] of cases) {
      const r = stripRows(view(mini(state, more)), [], 0, NOW)[1];
      expect([r.dot, r.text, r.click], state).toEqual([dot, text, click]);
    }
  });

  it('reads a Mac never asked yet as not checked, not offline', () => {
    const m = mini('offline', { checkedAt: null, lastSeenAt: null });
    expect(stateText(m, 0, NOW)).toBe('Not checked yet');
    expect(popoverInfo(m, NOW, null)).toMatchObject({ status: 'Not checked yet. Retry asks Mac mini now.', retry: true, stale: true });
  });

  it('counts only fresh running rows; stale rows count none', () => {
    const rows = [row('a', 'running', ago(1))];
    expect(runningCount(held('online', rows), NOW)).toBe(1);
    expect(runningCount(held('offline', rows), NOW)).toBe(0);
    // Rows a day old from an unreachable Mac are dropped.
    expect(machineRows(held('offline', rows, NOW - 25 * 3600_000), NOW).rows).toEqual([]);
    expect(machineRows(held('pairing-needed', rows), NOW).rows).toEqual([]);
    expect(machineRows(held('online', [row('old', 'idle', ago(9)), row('new', 'idle', ago(1))]), NOW).rows.map(r => r.key)).toEqual(['new', 'old']);
  });

  it('never lists this Mac twice', () => {
    const self = mini('online', { machineId: 'm-book' });
    expect(stripRows(view(self, mini('online')), [], 0, NOW).map(r => r.machineId)).toEqual(['m-book', 'm-mini']);
  });

  it('says what the popover offers', () => {
    expect(popoverInfo(mini('offline', { message: "ssh: Could not resolve hostname runner-host" }), NOW, null))
      .toMatchObject({ status: 'Mac mini is offline, last seen 4 min ago.', detail: 'ssh: Could not resolve hostname runner-host', retry: true, stale: true });
    expect(popoverInfo(mini('online'), NOW, null)).toMatchObject({ status: 'Online, last seen 4 min ago', stale: false, retry: false });
    expect(popoverInfo(mini('update-needed'), NOW, null).status).toBe('Needs an update. Mac mini runs Hollow 0.5.17 (abc1234).');
  });
});

describe('the machine badge', () => {
  it('names the Mac by a short word', () => {
    expect(machineWord({ machineId: 'm', slug: 'build-mac', name: 'Mac mini' })).toBe('mini');
    expect(machineWord({ machineId: 'm', slug: null, name: 'Studio' })).toBe('studio');
    expect(machineWord({ machineId: 'm', slug: 'runner-host', name: '  ' })).toBe('runner');
    expect(machineWord({ machineId: 'm', slug: '  ', name: '' })).toBe('mac');
  });

  it('says where the chat lives and whether it waits for an answer', () => {
    const m = { machineId: 'm', slug: null, name: 'Mac mini' };
    expect(badgeLabel(m)).toBe('On Mac mini');
    expect(badgeLabel(m, true)).toBe('On Mac mini, waiting for your answer');
  });
});

describe('times', () => {
  it('reads how long ago', () => {
    expect(agoLabel(null, NOW)).toBeNull();
    expect(agoLabel('not a time', NOW)).toBeNull();
    expect(agoLabel(ago(0.5), NOW)).toBe('just now');
    expect(agoLabel(ago(4), NOW)).toBe('4 min ago');
    expect(agoLabel(ago(180), NOW)).toBe('3 h ago');
    expect(agoLabel(ago(60 * 24), NOW)).toBe('1 day ago');
    expect(agoLabel(ago(60 * 50), NOW)).toBe('2 days ago');
  });
});

describe('the remote chat menu', () => {
  const home = { machineId: 'm', slug: null, name: 'Mac mini' };
  it('offers Open, Stop while running, Copy; home-only actions disabled', () => {
    const items = remoteMenu({ state: 'running' }, home, false);
    expect(items.map(i => [i.label, i.enabled])).toEqual([
      ['Open', true], ['Stop', true], ['Copy conversation ID', true], ['Rename…', false], ['Move to project…', false], ['Archive', false],
    ]);
    expect(items.filter(i => !i.enabled).every(i => i.title === 'Do this on Mac mini')).toBe(true);
  });
  it('has no Stop when idle and a disabled one while unreachable', () => {
    expect(remoteMenu({ state: 'idle' }, home, false).some(i => i.kind === 'stop')).toBe(false);
    expect(remoteMenu({ state: 'running' }, home, true).find(i => i.kind === 'stop')).toMatchObject({ enabled: false, title: "Can't reach Mac mini" });
  });
});

describe('a remote chat whose Mac is gone', () => {
  it('closes once its Mac is no longer paired, never before the Macs are read', () => {
    const open = { machineId: 'm-mini' };
    expect(chatHomeGone(null, open)).toBe(false);
    expect(chatHomeGone(view(mini('online')), null)).toBe(false);
    expect(chatHomeGone(view(mini('offline')), open)).toBe(false);
    expect(chatHomeGone(view(), open)).toBe(true);
    expect(chatHomeGone(view(mini('online', { machineId: 'm-other' })), open)).toBe(true);
  });
});

describe('M2 leftovers', () => {
  it('offers Cancel only while a Mac connects', () => {
    expect(popoverInfo(mini('connecting'), NOW, null)).toMatchObject({ status: 'Connecting to Mac mini…', cancel: true, retry: false });
    for (const state of ['online', 'offline', 'update-needed', 'pairing-needed'] as const) {
      expect(popoverInfo(mini(state), NOW, null).cancel ?? false, state).toBe(false);
    }
  });

  it('names Library\'s choices from the registry, never this Mac', () => {
    expect(libraryMacs(null)).toEqual([]);
    const studio = mini('offline', { machineId: 'm-studio', name: 'Studio' });
    expect(libraryMacs(view(mini('online', { machineId: 'm-book' }), mini('online'), studio))).toEqual([
      { machineId: 'm-mini', label: 'On Mac mini' },
      { machineId: 'm-studio', label: 'On Studio' },
    ]);
  });

  it('lists a Mac\'s chats in Library by project and search, newest first', () => {
    const inProject = { ...row('p', 'idle', ago(5)), workspace: { id: 'w-1', title: 'Hollow', kind: 'project' }, preview: 'Gate is green' } as PeerSessionRow;
    const rows = [row('a', 'idle', ago(9)), inProject, row('b', 'running', ago(1))];
    expect(libraryRows(held('online', rows), NOW, { workspaceId: null, query: '' }).rows.map(r => r.key)).toEqual(['b', 'p', 'a']);
    expect(libraryRows(held('online', rows), NOW, { workspaceId: 'w-1', query: '' }).rows.map(r => r.key)).toEqual(['p']);
    expect(libraryRows(held('online', rows), NOW, { workspaceId: null, query: 'GATE' }).rows.map(r => r.key)).toEqual(['p']);
    expect(libraryRows(held('offline', rows), NOW, { workspaceId: null, query: '' }).stale).toBe(true);
    expect(libraryRows(undefined, NOW, { workspaceId: null, query: '' })).toEqual({ rows: [], stale: false });
  });
});
