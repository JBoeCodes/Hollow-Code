import { beforeEach, expect, it, vi } from 'vitest';

// Five asks B4: refreshWorkspaces runs one chain at a time. Mounts and the
// minute timer join a running refresh; a call after a write gets one that
// starts after the running one, shared by every call meanwhile.
const state = vi.hoisted(() => {
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => {}, removeItem: () => {} });
  return {
    key: null as string | null, cwd: null as string | null, draft: '', turns: [] as { role: string; text: string }[],
    setActivePage: vi.fn(), pendingSend: null as unknown,
    list: vi.fn(), sessions: vi.fn(async () => []), icons: vi.fn(async () => []), rail: vi.fn(async () => []),
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => ({ api: {
  listWorkspaces: state.list, workspaceSessions: state.sessions, workspaceIcons: state.icons, buildRailStates: state.rail,
} }));
vi.mock('./runQueue', () => ({ refreshRunQueue: vi.fn(), runQueue: () => [] }));
vi.mock('./runtime', () => ({
  chatAgentSessionKey: () => state.key, chatCwd: () => state.cwd,
  chatBusy: () => false, chatInput: () => state.draft,
  chatAttachments: () => [], chatTurns: state.turns,
  setChatInput: (s: string) => { state.draft = s; }, setChatAttachments: vi.fn(),
  resetChatState: () => { state.key = null; state.cwd = null; state.turns.length = 0; },
  setChatCwdPersistent: (s: string) => { state.cwd = s; },
  setChatAgentPersistent: vi.fn(), setChatSessionIdPersistent: vi.fn(),
  setChatPermissionMode: vi.fn(),
  setChatAgentSessionKeyPersistent: (s: string) => { state.key = s; },
  setChatTurns: (t: typeof state.turns) => { state.turns.splice(0, state.turns.length, ...t); },
  rebuildChatArtifacts: vi.fn(), setChatSessionLoading: vi.fn(), setActivePage: state.setActivePage,
  setChatPanelOpen: vi.fn(), KNOWN_AGENTS: ['claude', 'codex'],
  canLeaveConversation: () => true, backgroundRunningChat: () => true,
  restoreBackgroundChat: () => false, isBackgrounded: () => false,
  backgroundChatCwd: () => undefined, backgroundedKeyForCwd: () => null, backgroundChatKeys: () => [],
  flashFooter: vi.fn(), requestChatFocus: vi.fn(), activePage: () => 'today',
  setPendingChatSend: (v: { text: string; cwd: string }) => { state.pendingSend = v; },
  sidePaneHosted: () => true, sidePaneOpen: () => false, setSidePaneOpen: vi.fn(), openSidePane: vi.fn(),
}));
import { refreshWorkspaces, refreshRailGitStates, refreshWorkspaceSessions, workspaces, workspaceSessions, workspaceIcons, railGitStates, projectWorkspaces, setWorkspaces, setWorkspaceWarning, workspaceError } from './workspaces';
import { QUICK_CHATS_ID } from '../workspaceTypes';

const ws = (id: string, build = false) => ({ id, title: id, status: 'active', path: `workspaces/${id}`, cwd: `/v/${id}`, modified: '', ...(build ? { build: { exists: true } } : {}) });
function deferred<T>() { let resolve!: (v: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }

beforeEach(() => { state.list.mockReset(); state.sessions.mockClear(); state.rail.mockClear(); });

it('a failed list preserves last-good rows and an unrelated workspace warning through Retry', async () => {
  state.list.mockResolvedValueOnce([ws('last-good')]);
  await refreshWorkspaces();
  const before = workspaces();
  setWorkspaceWarning('The selected build folder moved');
  state.list.mockRejectedValueOnce(new Error('List unreadable'));
  await refreshWorkspaces();
  expect(workspaces()).toBe(before);
  expect(workspaceError()).toBe('The selected build folder moved');
  state.list.mockResolvedValueOnce([ws('last-good')]);
  await refreshWorkspaces();
  expect(workspaceError()).toBe('The selected build folder moved');
});

it('launch: the rail and the workspace page mounting at once run the chain once', async () => {
  const first = deferred<ReturnType<typeof ws>[]>();
  state.list.mockReturnValueOnce(first.promise);
  const rail = refreshWorkspaces({ join: true });
  const page = refreshWorkspaces({ join: true });
  first.resolve([ws('a')]);
  await Promise.all([rail, page]);
  expect(state.list).toHaveBeenCalledTimes(1);
  expect(state.sessions).toHaveBeenCalledTimes(1);
});

it('a call after a write never returns the stale running refresh; calls meanwhile share one follow-up', async () => {
  const first = deferred<ReturnType<typeof ws>[]>();
  state.list.mockReturnValueOnce(first.promise).mockResolvedValue([ws('a'), ws('created')]);
  const running = refreshWorkspaces({ join: true });
  const afterCreate = refreshWorkspaces();
  const afterWatcher = refreshWorkspaces();
  expect(afterCreate).toBe(afterWatcher);
  first.resolve([ws('a')]);
  await running;
  await afterCreate;
  expect(state.list).toHaveBeenCalledTimes(2);
  expect(workspaces().map(w => w.id)).toEqual(['a', 'created']);
  // Idle again: a plain call starts at once.
  await refreshWorkspaces();
  expect(state.list).toHaveBeenCalledTimes(3);
});

it('git states: a throttled ask within its window is skipped, a changed set or a forced ask is not', async () => {
  state.list.mockResolvedValue([ws('b1', true)]);
  await refreshWorkspaces();
  await Promise.resolve();
  const asked = state.rail.mock.calls.length;
  await refreshRailGitStates(50_000);
  expect(state.rail.mock.calls.length).toBe(asked);
  await refreshRailGitStates();
  expect(state.rail.mock.calls.length).toBe(asked + 1);
  state.list.mockResolvedValue([ws('b1', true), ws('b2', true)]);
  await refreshWorkspaces();
  await Promise.resolve();
  expect(state.rail.mock.calls.length).toBe(asked + 2);
});

it('git states: `fresh` reaches the backend, limited to build workspaces being asked about', async () => {
  state.list.mockResolvedValue([ws('b1', true), ws('plain')]);
  await refreshWorkspaces();
  await Promise.resolve();
  await refreshRailGitStates(0, ['b1', 'plain', 'gone']);
  expect(state.rail).toHaveBeenLastCalledWith(['b1'], ['b1']);
  await refreshRailGitStates();
  expect(state.rail).toHaveBeenLastCalledWith(['b1'], []);
});

// Overnight 10-11/12 P4 (big vaults): every workspace switch re-reads every
// workspace's conversations, and a vault change re-reads the list. A re-read
// that changes nothing must hand back the objects already on screen, or the
// rail and Work redraw all 200 workspaces' rows each time.
const session = (key: string, title = key) => ({ key, title, agent: 'claude', lastActivityAt: '2026-10-12T05:00:00Z', turnCount: 2 });
const group = (id: string, keys: string[]) => ({ workspaceId: id, total: keys.length, sessions: keys.map(k => session(k)) });

it('P4: an unchanged workspace list keeps its array; a changed one keeps every unchanged workspace', async () => {
  state.list.mockResolvedValue([ws('a'), ws('b')]);
  await refreshWorkspaces();
  const before = workspaces();
  state.list.mockResolvedValue([ws('a'), ws('b')]);
  await refreshWorkspaces();
  expect(workspaces()).toBe(before);
  state.list.mockResolvedValue([ws('a'), { ...ws('b'), title: 'B renamed' }, ws('c')]);
  await refreshWorkspaces();
  expect(workspaces()).not.toBe(before);
  expect(workspaces()[0]).toBe(before[0]);
  expect(workspaces()[1].title).toBe('B renamed');
  expect(workspaces().map(w => w.id)).toEqual(['a', 'b', 'c']);
});

it('P4: unchanged conversation groups keep their identity, down to each unchanged conversation', async () => {
  state.sessions.mockResolvedValue([group('a', ['a1', 'a2']), group('b', ['b1'])] as never);
  await refreshWorkspaceSessions();
  const before = workspaceSessions();
  state.sessions.mockResolvedValue([group('a', ['a1', 'a2']), group('b', ['b1'])] as never);
  await refreshWorkspaceSessions();
  expect(workspaceSessions()).toBe(before);
  // A new conversation in b: a's group is untouched, b1 keeps its object.
  state.sessions.mockResolvedValue([group('a', ['a1', 'a2']), group('b', ['b0', 'b1'])] as never);
  await refreshWorkspaceSessions();
  const after = workspaceSessions();
  expect(after).not.toBe(before);
  expect(after.a).toBe(before.a);
  expect(after.b).not.toBe(before.b);
  expect(after.b.sessions[1]).toBe(before.b.sessions[0]);
  expect(after.b.sessions.map(s => s.key)).toEqual(['b0', 'b1']);
  // A retitled conversation is a new object; a workspace that lost its group drops out.
  state.sessions.mockResolvedValue([{ ...group('b', ['b0', 'b1']), sessions: [session('b0'), session('b1', 'Renamed')] }] as never);
  await refreshWorkspaceSessions();
  expect(Object.keys(workspaceSessions())).toEqual(['b']);
  expect(workspaceSessions().b.sessions[0]).toBe(after.b.sessions[0]);
  expect(workspaceSessions().b.sessions[1].title).toBe('Renamed');
});

it('P4: unchanged git states and icons keep their records', async () => {
  state.list.mockResolvedValue([ws('b1', true)]);
  state.rail.mockResolvedValue([{ workspaceId: 'b1', branch: 'x', ahead: 1, prState: null, prNumber: null }] as never);
  await refreshWorkspaces();
  await refreshRailGitStates();
  expect(railGitStates().b1?.branch).toBe('x');
  const rail = railGitStates();
  await refreshRailGitStates();
  expect(railGitStates()).toBe(rail);
  state.icons.mockResolvedValue([{ workspaceId: 'b1', owner: 'o' }] as never);
  state.list.mockResolvedValue([ws('b1', true), ws('i1')]);
  await refreshWorkspaces();
  await new Promise(r => setTimeout(r, 0));
  const icons = workspaceIcons();
  expect(icons.b1).toBeTruthy();
  state.list.mockResolvedValue([ws('b1', true), ws('i2')]);
  await refreshWorkspaces();
  await new Promise(r => setTimeout(r, 0));
  expect(workspaceIcons()).toBe(icons);
});


it('an explicit fresh workspace bypasses the frontend throttle', async () => {
  state.list.mockResolvedValue([ws('fresh-b7', true)]);
  await refreshWorkspaces();
  await refreshRailGitStates();
  const before = state.rail.mock.calls.length;
  await refreshRailGitStates(50_000, ['fresh-b7']);
  expect(state.rail.mock.calls.length).toBe(before + 1);
  expect(state.rail).toHaveBeenLastCalledWith(['fresh-b7'], ['fresh-b7']);
});

it('an overlapping ordinary refresh carries an outstanding request to bypass the backend cache', async () => {
  state.list.mockResolvedValue([ws('race-b7', true)]);
  await refreshWorkspaces();
  await refreshRailGitStates();
  const first = deferred<never[]>();
  state.rail.mockReturnValueOnce(first.promise);
  const forced = refreshRailGitStates(0, ['race-b7']);
  await refreshRailGitStates();
  expect(state.rail).toHaveBeenLastCalledWith(['race-b7'], ['race-b7']);
  first.resolve([]);
  await forced;
  await refreshRailGitStates();
  expect(state.rail).toHaveBeenLastCalledWith(['race-b7'], []);
});


it('failed freshness stays pending for the next read and is cleared only after success', async () => {
  state.list.mockResolvedValue([ws('retry-b7', true)]);
  await refreshWorkspaces(); await refreshRailGitStates();
  state.rail.mockRejectedValueOnce(new Error('fake unavailable'));
  await refreshRailGitStates(0, ['retry-b7']);
  await refreshRailGitStates(50_000);
  expect(state.rail).toHaveBeenLastCalledWith(['retry-b7'], ['retry-b7']);
  await refreshRailGitStates();
  expect(state.rail).toHaveBeenLastCalledWith(['retry-b7'], []);
});

it('removing the final build workspace invalidates an outstanding reply', async () => {
  state.list.mockResolvedValue([ws('removed-b7', true)]);
  await refreshWorkspaces(); await refreshRailGitStates();
  const response = deferred<never[]>();
  state.rail.mockReturnValueOnce(response.promise);
  const old = refreshRailGitStates(0, ['removed-b7']);
  state.list.mockResolvedValue([ws('plain-b7')]);
  await refreshWorkspaces();
  response.resolve([{ workspaceId: 'removed-b7', branch: 'stale' }] as never);
  await old;
  expect(railGitStates()).toEqual({});
});

// ST2: rows ask for the list once each, and `distinguishing` indexes titles
// per array, so a fresh copy per call would rebuild that index for every row.
it('projectWorkspaces hands back the same array until the list changes, without the built-in ones', () => {
  setWorkspaces([ws('a'), ws(QUICK_CHATS_ID), ws('b')] as never);
  const first = projectWorkspaces();
  expect(projectWorkspaces()).toBe(first);
  expect(first.map(w => w.id)).toEqual(['a', 'b']);
  setWorkspaces([ws('a'), ws('c')] as never);
  const next = projectWorkspaces();
  expect(next).not.toBe(first);
  expect(next.map(w => w.id)).toEqual(['a', 'c']);
  expect(projectWorkspaces()).toBe(next);
});
