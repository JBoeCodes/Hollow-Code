import moduleSource from './linearLink.ts?raw';
import infoPlist from '../src-tauri/Info.plist?raw';
import linkSource from '../src-tauri/src/linear_link.rs?raw';
import { mockTrackerCommand } from './mockTracker';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const h = vi.hoisted(() => ({
  invoke: vi.fn(), prefill: vi.fn(), focus: vi.fn(), flash: vi.fn(),
  openProjectAgent: vi.fn(), askMainAgent: vi.fn(),
  open: 'atlas2.0' as string | null, projects: ['atlas2.0'], loaded: true, refresh: vi.fn(async () => {}),
}));
vi.mock('./api', () => ({ invoke: h.invoke }));
vi.mock('./store/runtime', () => ({ setChatPrefillRequest: h.prefill, requestChatFocus: h.focus, flashFooter: h.flash }));
vi.mock('./store/workspaces', () => ({
  workspaceDetail: () => (h.open ? {} : null), workspaceId: () => h.open,
  openProjectAgent: h.openProjectAgent, workspaces: () => h.projects.map(id => ({ id })),
  workspaceListLoaded: () => h.loaded, refreshWorkspaces: h.refresh,
}));
vi.mock('./store/todayBrief', () => ({ askMainAgent: h.askMainAgent }));

import { openLink, openPendingLinearLinks, type LinkOpen } from './linearLink';

const link = (over: Partial<LinkOpen> = {}): LinkOpen => ({
  seq: 1, source: 'Linear link', identifier: 'ENG-307', tracked: true, workspaceId: 'atlas2.0',
  draft: 'Linear issue ENG-307: Fix the lot map\n\nDo it', note: null, ...over,
});

beforeEach(() => {
  vi.clearAllMocks();
  h.open = 'atlas2.0'; h.projects = ['atlas2.0']; h.loaded = true;
  h.openProjectAgent.mockResolvedValue(true);
  h.askMainAgent.mockResolvedValue('opened');
});

describe('a Linear link opens a draft, never a send', () => {
  it('puts the draft in the issue’s project lead chat', async () => {
    expect(await openLink(link())).toBe(true);
    expect(h.openProjectAgent).toHaveBeenCalledWith('atlas2.0');
    expect(h.prefill).toHaveBeenCalledWith('Linear issue ENG-307: Fix the lot map\n\nDo it');
    expect(h.focus).toHaveBeenCalled();
    expect(h.askMainAgent).not.toHaveBeenCalled();
  });

  it('opens the COO when the issue has no project, with the note', async () => {
    await openLink(link({ workspaceId: null, tracked: false, draft: 'Note: Hollow doesn’t track LP-1 (Hollow tracks team AI only).\n\nx', note: 'Hollow doesn’t track LP-1 (Hollow tracks team AI only).' }));
    expect(h.askMainAgent).toHaveBeenCalledWith(expect.stringContaining('doesn’t track LP-1'));
    expect(h.openProjectAgent).not.toHaveBeenCalled();
    expect(h.flash).toHaveBeenCalledWith(expect.stringContaining('doesn’t track LP-1'), expect.any(Number));
  });

  it('falls back to the COO when the project’s chat will not open or is gone', async () => {
    h.openProjectAgent.mockResolvedValue(false);
    await openLink(link());
    expect(h.askMainAgent).toHaveBeenCalledOnce();
    expect(h.prefill).not.toHaveBeenCalled();
    vi.clearAllMocks(); h.askMainAgent.mockResolvedValue('opened');
    h.projects = [];
    await openLink(link());
    expect(h.openProjectAgent).not.toHaveBeenCalled();
    expect(h.askMainAgent).toHaveBeenCalledOnce();
  });

  it('opens nothing for a refused link and says why', async () => {
    expect(await openLink(link({ workspaceId: null, tracked: false, draft: '', note: 'Hollow ignored a link it doesn’t recognise.' }))).toBe(false);
    expect(h.openProjectAgent).not.toHaveBeenCalled();
    expect(h.askMainAgent).not.toHaveBeenCalled();
    expect(h.prefill).not.toHaveBeenCalled();
    expect(h.flash).toHaveBeenCalledWith(expect.stringContaining('ignored'), expect.any(Number));
  });

  it('drains the queue oldest first and ignores a repeated link', async () => {
    h.invoke.mockResolvedValueOnce([link({ seq: 10, draft: 'one' }), link({ seq: 11, draft: 'two' })]);
    await openPendingLinearLinks();
    expect(h.prefill.mock.calls.map(c => c[0])).toEqual(['one', 'two']);
    h.invoke.mockResolvedValueOnce([link({ seq: 11, draft: 'two' })]);
    await openPendingLinearLinks();
    expect(h.prefill).toHaveBeenCalledTimes(2);
    expect(h.invoke).toHaveBeenCalledWith('take_linear_links');
  });

  it('waits for the workspace list on a cold launch before choosing the chat', async () => {
    h.loaded = false;
    h.refresh.mockImplementationOnce(async () => { h.loaded = true; });
    await openLink(link());
    expect(h.refresh).toHaveBeenCalledWith({ join: true });
    expect(h.openProjectAgent).toHaveBeenCalledWith('atlas2.0');
    expect(h.askMainAgent).not.toHaveBeenCalled();
  });

  it('retries a chat that will not open yet, then says so by identifier', async () => {
    vi.useFakeTimers();
    try {
      h.askMainAgent.mockResolvedValue(false);
      h.invoke.mockResolvedValueOnce([link({ seq: 40, workspaceId: null })]);
      const pass = openPendingLinearLinks();
      await vi.advanceTimersByTimeAsync(5000);
      await pass;
      expect(h.askMainAgent).toHaveBeenCalledTimes(3);
      expect(h.flash).toHaveBeenCalledWith(expect.stringContaining('for ENG-307'), expect.any(Number));
      // One that opens on the second try is not reported.
      vi.clearAllMocks();
      h.askMainAgent.mockResolvedValueOnce(false).mockResolvedValueOnce('opened');
      h.invoke.mockResolvedValueOnce([link({ seq: 41, workspaceId: null })]);
      const again = openPendingLinearLinks();
      await vi.advanceTimersByTimeAsync(5000);
      await again;
      expect(h.askMainAgent).toHaveBeenCalledTimes(2);
      expect(h.flash).not.toHaveBeenCalled();
    } finally { vi.useRealTimers(); }
  });

  it('says so when a tracked link’s project is not in the list', async () => {
    h.projects = [];
    await openLink(link());
    expect(h.flash).toHaveBeenCalledWith(expect.stringContaining('went to the COO'), expect.any(Number));
    expect(h.askMainAgent).toHaveBeenCalledOnce();
  });

  it('never lets one stuck open stall the queue behind it', async () => {
    vi.useFakeTimers();
    try {
      h.askMainAgent.mockReturnValue(new Promise(() => {}));
      h.invoke.mockResolvedValueOnce([link({ seq: 50, workspaceId: null })]);
      const pass = openPendingLinearLinks();
      await vi.advanceTimersByTimeAsync(60_000);
      await pass;
      expect(h.flash).toHaveBeenCalledWith(expect.stringContaining('Click it in Linear again'), expect.any(Number));
    } finally { vi.useRealTimers(); }
  });

  it('survives a failed read and an empty queue', async () => {
    h.invoke.mockRejectedValueOnce(new Error('no ipc'));
    await expect(openPendingLinearLinks()).resolves.toBeUndefined();
    h.invoke.mockResolvedValueOnce(undefined);
    await expect(openPendingLinearLinks()).resolves.toBeUndefined();
  });

  it('has no way to send: the module touches no send path', () => {
    const source = moduleSource;
    for (const send of ['sendQuickAsk', 'setPendingChatSend', 'chat_send', 'sendChat', 'startRun', 'change_setting']) {
      expect(source, send).not.toContain(send);
    }
    expect(source).toContain('setChatPrefillRequest');
  });
});

describe('the setup the person pastes into Linear', () => {
  it('is the exact Custom link, the same text in the app, the mock and the COO’s action', () => {
    const value = 'hollow://issue?prompt={{prompt}}';
    expect(linkSource).toContain(`pub const SETUP_VALUE: &str = "${value}";`);
    expect((mockTrackerCommand('tracker_link_setup') as { value: string }).value).toBe(value);
  });

  it('is registered as a URL scheme in the bundle', () => {
    expect(infoPlist).toMatch(/<key>CFBundleURLSchemes<\/key>\s*<array>\s*<string>hollow<\/string>/);
  });
});
