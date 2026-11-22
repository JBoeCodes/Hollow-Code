// The top bar's "Actions for COO" menu (the COO page header band's "…" menu,
// removed 2026-11-11): which items it offers for what the COO is doing and
// what its workspace holds, and that Create default standing orders makes
// the file once and says what happened.
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  detail: null as { workspace: { path: string }; files: { name: string }[] } | null,
  acts: [] as unknown[], actsRead: 'ready' as 'ready' | 'loading' | 'error',
  status: { checkIns: true } as { checkIns: boolean } | null, failedSession: null as string | null,
  owner: 'coo', createOrders: vi.fn(), flash: vi.fn(), openNote: vi.fn(), goToPage: vi.fn(), openSettings: vi.fn(),
  openSession: vi.fn(), refresh: vi.fn(async () => {}),
}));
vi.mock('./api', () => ({ api: { createMainStandingOrders: h.createOrders } }));
vi.mock('./store/runtime', () => ({ flashFooter: h.flash, openVaultNote: h.openNote, openSettingsFromOutside: h.openSettings }));
vi.mock('./store/workspaces', () => ({
  goToPage: h.goToPage, openSessionByKey: h.openSession, refreshWorkspace: h.refresh,
  workspaceDetail: () => h.detail, workspaceId: () => h.owner,
}));
vi.mock('./store/mainAgent', () => ({
  mainAgentActs: () => h.acts, mainAgentActsRead: () => h.actsRead,
  mainAgentNow: () => ({ failedSession: h.failedSession }), mainAgentStatus: () => h.status,
}));

import { cooMenuItems } from './cooMenu';

type Item = ReturnType<typeof cooMenuItems>[number];
const labels = () => cooMenuItems().map(i => i.label ?? '---');
const item = (label: string | RegExp): Item =>
  cooMenuItems().find(i => i.label !== undefined && (typeof label === 'string' ? i.label === label : label.test(i.label)))!;

beforeEach(() => {
  vi.clearAllMocks();
  h.detail = { workspace: { path: '/vault/workspaces/main' }, files: [] };
  h.acts = []; h.actsRead = 'ready'; h.status = { checkIns: true }; h.failedSession = null; h.owner = 'coo';
});

describe('Actions for COO', () => {
  it('words the recent actions by how well they were read', () => {
    h.acts = [{}, {}, {}];
    expect(labels()[0]).toBe('Actions it took on its own · 3 recent');
    h.actsRead = 'error';
    expect(labels()[0]).toBe('Actions it took on its own · 3 last read · refresh failed');
    h.acts = []; h.actsRead = 'loading';
    expect(labels()[0]).toBe('Actions it took on its own · Reading…');
  });

  it('offers Check-in settings while check-ins are on, and turning them on when they are off', () => {
    expect(labels()).toContain('Check-in settings');
    h.status = { checkIns: false };
    expect(labels()).toContain('Turn check-ins on in Settings');
    expect(labels()).not.toContain('Check-in settings');
    // An unread status never claims check-ins are off.
    h.status = null;
    expect(labels()).toContain('Check-in settings');
  });

  it('sends the hard stop and check-in items to their Settings controls', () => {
    item('Change hard stop').action!();
    expect(h.openSettings).toHaveBeenCalledWith('usage', 'COO daily hard stop');
    item('Check-in settings').action!();
    expect(h.openSettings).toHaveBeenLastCalledWith('automation', 'COO check-ins');
  });

  it('adds Open failed conversation only when a run failed, and opens that conversation', () => {
    expect(labels()).not.toContain('Open failed conversation');
    h.failedSession = 'chat-9';
    item('Open failed conversation').action!();
    expect(h.openSession).toHaveBeenCalledWith('chat-9');
  });

  it('offers to create the standing orders when the file is missing, else to edit it', () => {
    expect(labels().slice(-2)).toEqual(['---', 'Create default standing orders']);
    h.detail = { workspace: { path: '/vault/workspaces/main' }, files: [{ name: 'STANDING_ORDERS.md' }] };
    expect(labels().slice(-2)).toEqual(['---', 'Edit standing orders']);
    item('Edit standing orders').action!();
    expect(h.openNote).toHaveBeenCalledWith('/vault/workspaces/main/STANDING_ORDERS.md');
  });

  it('offers neither standing-orders item before the workspace has loaded', () => {
    h.detail = null;
    expect(labels().filter(l => /standing orders/i.test(l))).toEqual([]);
  });
});

describe('Create default standing orders', () => {
  it('creates the file, refreshes the workspace and says so', async () => {
    h.createOrders.mockResolvedValue(true);
    item('Create default standing orders').action!();
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalled());
    expect(h.refresh).toHaveBeenCalledTimes(1);
    expect(h.flash).toHaveBeenCalledWith('Default standing orders created.');
  });

  it('keeps an existing file and says that instead', async () => {
    h.createOrders.mockResolvedValue(false);
    item('Create default standing orders').action!();
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalled());
    expect(h.flash).toHaveBeenCalledWith('Standing orders already exist; kept your file.');
  });

  it('does not refresh a workspace the person has left, and reports a failure for 5 s', async () => {
    h.createOrders.mockImplementation(async () => { h.owner = 'elsewhere'; return true; });
    item('Create default standing orders').action!();
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalled());
    expect(h.refresh).not.toHaveBeenCalled();
    h.owner = 'coo';
    h.createOrders.mockRejectedValue('disk full');
    item('Create default standing orders').action!();
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalledTimes(2));
    expect(h.flash).toHaveBeenLastCalledWith('Could not create standing orders: disk full', 5000);
  });

  it('ignores a second click while the first is still creating', async () => {
    let finish!: (v: boolean) => void;
    h.createOrders.mockImplementation(() => new Promise<boolean>(resolve => { finish = resolve; }));
    item('Create default standing orders').action!();
    item('Create default standing orders').action!();
    expect(h.createOrders).toHaveBeenCalledTimes(1);
    finish(true);
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalled());
    // The lock is released afterwards.
    h.createOrders.mockResolvedValue(true);
    item('Create default standing orders').action!();
    await vi.waitFor(() => expect(h.flash).toHaveBeenCalledTimes(2));
    expect(h.createOrders).toHaveBeenCalledTimes(2);
  });
});
