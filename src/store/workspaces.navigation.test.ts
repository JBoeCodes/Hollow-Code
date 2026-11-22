import { beforeEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => {
  const saved = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => saved.set(key, value),
    removeItem: (key: string) => saved.delete(key),
  });
  return {
    cwd: null as string | null, key: null as string | null, busy: false, page: 'today',
    draft: '', turns: [] as { role: string; text: string }[],
    snapshots: new Map<string, { cwd: string | null; busy: boolean; turns: { role: string; text: string }[] }>(),
    read: vi.fn(), readConversation: vi.fn(), sessionTarget: vi.fn(async (_key: string): Promise<{ workspaceId: string | null; session: { key: string; cwd: string; agent: string } } | null> => null), saved,
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => ({ api: { readWorkspace: state.read, readAgentConversation: state.readConversation, sessionTarget: state.sessionTarget, workspaceSessions: async () => [] } }));
vi.mock('./runQueue', () => ({ refreshRunQueue: vi.fn() }));
// A page jump still waiting for its view transition (#1311 re-review): the
// flush applies it, as viewTransitions.ts does for a real one.
const transition = vi.hoisted(() => ({ pending: null as null | (() => void) }));
vi.mock('../viewTransitions', async original => ({
  ...(await original<typeof import('../viewTransitions')>()),
  flushViewTransition: () => { const jump = transition.pending; transition.pending = null; jump?.(); },
}));
vi.mock('./runtime', () => ({
  chatAgentSessionKey: () => state.key, chatCwd: () => state.cwd,
  chatBusy: () => state.busy, chatInput: () => state.draft,
  chatAttachments: () => [], chatTurns: state.turns,
  setChatInput: (s: string) => { state.draft = s; }, setChatAttachments: vi.fn(),
  resetChatState: () => { state.key = null; state.cwd = null; state.turns.length = 0; },
  applyNewChatPermissionMode: vi.fn(),
  setChatCwdPersistent: (s: string) => { state.cwd = s; },
  setChatAgentPersistent: vi.fn(), setChatSessionIdPersistent: vi.fn(),
  setChatPermissionMode: vi.fn(), chatPermissionMode: () => 'acceptEdits',
  setChatAgentSessionKeyPersistent: (s: string) => { state.key = s; },
  setChatTurns: (t: typeof state.turns) => { state.turns.splice(0, state.turns.length, ...t); },
  rebuildChatArtifacts: vi.fn(), setChatSessionLoading: vi.fn(), setActivePage: vi.fn(),
  setChatPanelOpen: vi.fn(), KNOWN_AGENTS: ['claude', 'codex'],
  canLeaveConversation: () => true, requestChatFocus: vi.fn(),
  chatStoredTranscriptPending: () => null, showStoredTranscript: vi.fn(),
  beginStoredTranscriptRead: () => 1, storedTranscriptReadApplies: () => true, markStoredTranscriptApplied: vi.fn(),
  backgroundRunningChat: () => {
    if (state.busy && state.key) {
      state.snapshots.set(state.key, { cwd: state.cwd, busy: true, turns: [...state.turns] });
      state.busy = false;
    }
    return true;
  },
  restoreBackgroundChat: (key: string) => {
    const s = state.snapshots.get(key);
    if (!s) return false;
    state.snapshots.delete(key);
    state.key = key; state.cwd = s.cwd; state.busy = s.busy;
    state.turns.splice(0, state.turns.length, ...s.turns);
    return true;
  },
  backgroundChatKeys: () => [...state.snapshots].filter(([, s]) => s.busy).map(([key]) => key),
  isBackgrounded: (key: string) => state.snapshots.has(key),
  backgroundChatCwd: (key: string) => state.snapshots.get(key)?.cwd,
  backgroundedKeyForCwd: (cwd: string) => [...state.snapshots].find(([, s]) => s.cwd === cwd)?.[0] ?? null,
  flashFooter: vi.fn(), openCommandPalette: vi.fn(), activePage: () => state.page, shownPage: () => state.page,
  sidePaneHosted: () => true, sidePaneOpen: () => false, setSidePaneOpen: vi.fn(), openSidePane: vi.fn(),
  vaultRoot: async () => '/vault', setOpenVaultNoteRequest: vi.fn(),
}));
import { selectWorkspace, newWorkspaceSessionOpened, setWorkspaceDetail, setWorkspaceId, workspaceId, workspaceLoading, setRailWorkspaceIds, railWorkspaceIds, workspaceError, railHiddenIds, setRailHiddenIds, pickConversation, reopenWorkspaceConversation } from './workspaces';
import { setActivePage, setChatAgentPersistent, requestChatFocus } from './runtime';
import { visitWorkspace, visitWorkspaceConversation, openWorkspaceResult } from './workspaces';
import { closePreview, previewTarget, resetPreviewForTests } from './preview';
// Static, so it is the instance the workspaces store above bound (the
// launch-restore tests reset modules).
import { navQuiet, workspaceNav } from './navHistory';

// COO rebuild L4: a Result opens in the Files tab of the Preview pane (with
// its review bar), after its workspace; it was the Results section.
describe('workspace result navigation waits for draft consent', () => {
  it.each([false, true])('a refusal opens nothing (workspace loaded: %s)', async loaded => {
    const { registerNavigationGuard } = await import('./navigationGuard');
    if (loaded) await selectWorkspace('alpha');
    resetPreviewForTests();
    state.read.mockClear();
    vi.mocked(setActivePage).mockClear();
    const check = vi.fn(() => false);
    const release = registerNavigationGuard(check);
    try {
      await openWorkspaceResult('alpha', 'workspaces/alpha/outputs/refused.md');
      expect(check).toHaveBeenCalledOnce();
      expect(previewTarget()).toBeNull();
      expect(state.read).not.toHaveBeenCalled();
      expect(setActivePage).not.toHaveBeenCalled();
    } finally { release(); }
  });

  it.each([false, true])('an asynchronous choice opens the file only once allowed: %s', async allowed => {
    const { registerNavigationGuard } = await import('./navigationGuard');
    await selectWorkspace('alpha');
    resetPreviewForTests();
    state.read.mockClear();
    let answer!: (allowed: boolean) => void;
    const check = vi.fn(() => new Promise<boolean>(resolve => { answer = resolve; }));
    const release = registerNavigationGuard(check);
    vi.mocked(setActivePage).mockClear();
    try {
      const pending = openWorkspaceResult('alpha', 'workspaces/alpha/outputs/chosen.md');
      expect(setActivePage).not.toHaveBeenCalled();
      answer(allowed); await pending;
      expect(check).toHaveBeenCalledOnce();
      expect(previewTarget()).toEqual(allowed ? { kind: 'file', path: '/vault/workspaces/alpha/outputs/chosen.md' } : null);
      expect(vi.mocked(setActivePage).mock.calls).toEqual(allowed ? [['today']] : []);
    } finally { release(); closePreview(); }
  });

  it('a newer result request owns the destination after delayed consent', async () => {
    const { registerNavigationGuard } = await import('./navigationGuard');
    resetPreviewForTests();
    const answers: Array<(value: boolean) => void> = [];
    const check = vi.fn(() => new Promise<boolean>(resolve => answers.push(resolve)));
    const release = registerNavigationGuard(check);
    try {
      const old = openWorkspaceResult('alpha', 'workspaces/alpha/outputs/stale.md');
      const newest = openWorkspaceResult('alpha', 'workspaces/alpha/outputs/newest.md');
      answers[0](true); await old;
      expect(previewTarget()).toBeNull();
      answers[1](true); await newest;
      expect(previewTarget()).toEqual({ kind: 'file', path: '/vault/workspaces/alpha/outputs/newest.md' });
      expect(check).toHaveBeenCalledTimes(2);
    } finally { release(); closePreview(); }
  });
});

describe('Library canvas navigation keeps destination state behind the draft guard', () => {
  it('a refused Compare cannot leak into a later ordinary gallery visit', async () => {
    const { openCanvasGallery } = await import('./canvasGallery');
    const { takeGalleryCanvas } = await import('../canvas/galleryRequest');
    const { registerNavigationGuard } = await import('./navigationGuard');
    takeGalleryCanvas();
    state.page = 'workflows';
    const release = registerNavigationGuard(() => false);
    try {
      await openCanvasGallery('old', { workspaceId: 'old', id: 'rejected', compare: true, back: null });
      expect(takeGalleryCanvas()).toBeNull();
    } finally { release(); }
    await openCanvasGallery('new');
    expect(takeGalleryCanvas()).toBeNull();
  });

  it('a newer canvas destination supersedes an unanswered Compare', async () => {
    const { openCanvasGallery } = await import('./canvasGallery');
    const library = await import('./libraryMode');
    const { takeGalleryCanvas } = await import('../canvas/galleryRequest');
    const { registerNavigationGuard } = await import('./navigationGuard');
    takeGalleryCanvas();
    state.page = 'workflows';
    const answers: Array<(value: boolean) => void> = [];
    const release = registerNavigationGuard(() => new Promise<boolean>(resolve => answers.push(resolve)));
    const before = library.canvasOpenRequest();
    const newest = { workspaceId: 'new', id: 'kept', compare: true, back: { label: 'Return', go: vi.fn() } };
    try {
      const old = openCanvasGallery('old', { ...newest, workspaceId: 'old', id: 'stale' });
      const current = openCanvasGallery('new', newest);
      answers[0](true);
      await old;
      expect(takeGalleryCanvas()).toBeNull();
      expect(library.canvasOpenRequest()).toBe(before);
      answers[1](true);
      await current;
      expect(library.canvasStartWorkspace()).toBe('new');
      expect(library.canvasOpenRequest()).toBe(before + 1);
      expect(takeGalleryCanvas()).toBe(newest);
      expect(takeGalleryCanvas()).toBeNull();
    } finally { release(); }
  });

  it('an ordinary open clears an accepted Compare not consumed by an unmounted view', async () => {
    const { openCanvasGallery } = await import('./canvasGallery');
    const { takeGalleryCanvas } = await import('../canvas/galleryRequest');
    state.page = 'vault';
    await openCanvasGallery('old', { workspaceId: 'old', id: 'left', compare: true, back: null });
    await openCanvasGallery();
    expect(takeGalleryCanvas()).toBeNull();
  });

  it.each(['workflows', 'vault'])('a refusal on %s preserves the Library view and filter', async page => {
    const library = await import('./libraryMode');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = page;
    library.setLibraryMode('notes', true);
    library.setResultsType('files');
    const before = { workspace: library.canvasStartWorkspace(), request: library.canvasOpenRequest() };
    const check = vi.fn(() => false);
    const release = registerNavigationGuard(check);
    try {
      await library.openLibraryCanvases('rejected');
      expect(check).toHaveBeenCalledOnce();
      expect(library.libraryMode()).toBe('notes');
      expect(library.resultsType()).toBe('files');
      expect(library.canvasStartWorkspace()).toBe(before.workspace);
      expect(library.canvasOpenRequest()).toBe(before.request);
      expect(state.saved.get('hollow.library.mode')).toBe('all');
    } finally { release(); }
  });

  it.each([['vault',false],['vault',true],['workflows',false],['workflows',true]] as const)('Learnings on %s waits for navigation consent: %s', async (page, allowed) => {
    const library = await import('./libraryMode');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = page;
    library.setLibraryMode('notes', true);
    let resolve!: (value: boolean) => void;
    const release = registerNavigationGuard(() => new Promise<boolean>(done => { resolve = done; }));
    try {
      const pending = library.openLibraryLearnings();
      expect(library.libraryMode()).toBe('notes');
      expect(state.saved.get('hollow.library.mode')).toBe('all');
      resolve(allowed);
      await pending;
      expect(library.libraryMode()).toBe(allowed ? 'learnings' : 'notes');
      expect(state.saved.get('hollow.library.mode')).toBe(allowed ? 'learnings' : 'all');
    } finally { release(); }
  });

  // Overnight 11-12 L2: openLibraryTab is how the palette (and Settings' links) open Skills and History.
  it.each([['vault','skills'],['workflows','skills'],['vault','history'],['workflows','history']] as const)('openLibraryTab on %s opens %s and remembers it', async (page, tab) => {
    const library = await import('./libraryMode');
    state.page = page;
    library.setLibraryMode('notes', true);
    await library.openLibraryTab(tab);
    expect(library.libraryMode()).toBe(tab);
    expect(state.saved.get('hollow.library.mode')).toBe(tab);
  });

  // #1311 re-review: the palette, links and Library's own tab strip change
  // the view one way (openLibraryTab), so a palette jump matches a tab click.
  it('leaving Conversations for a tab from the palette is a Back step, as a tab click is', async () => {
    const library = await import('./libraryMode');
    state.page = 'vault';
    library.setLibraryMode('conversations');
    vi.mocked(setActivePage).mockClear();
    await library.openLibraryTab('skills');
    expect(library.libraryMode()).toBe('skills');
    expect(vi.mocked(setActivePage).mock.calls).toEqual([['vault']]);
  });

  it('Conversations from inside Library waits for the draft guard', async () => {
    const library = await import('./libraryMode');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = 'vault';
    library.setLibraryMode('notes', true);
    vi.mocked(setActivePage).mockClear();
    const release = registerNavigationGuard(() => false);
    try { await library.openLibraryTab('conversations'); } finally { release(); }
    expect(setActivePage).not.toHaveBeenCalled();
    await library.openLibraryTab('conversations');
    expect(vi.mocked(setActivePage).mock.calls).toEqual([['chat']]);
  });

  it.each(['tab', 'note', 'canvases'] as const)('a %s open inside Library clears its open file and forms, once accepted', async kind => {
    const library = await import('./libraryMode');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = 'vault';
    library.setLibraryMode('notes', true);
    const cleared = vi.fn();
    const unbind = library.bindLibraryViewReset(cleared);
    const open = () => kind === 'tab' ? library.openLibraryTab('skills') : kind === 'note' ? library.openLibraryNote('notes/a.md') : library.openLibraryCanvases();
    try {
      const release = registerNavigationGuard(() => false);
      try { await open(); } finally { release(); }
      expect(cleared).not.toHaveBeenCalled();
      await open();
      expect(cleared).toHaveBeenCalledOnce();
    } finally { unbind(); }
  });

  it('a page jump still waiting for its view transition lands before the tab', async () => {
    const library = await import('./libraryMode');
    state.page = 'vault';
    library.setLibraryMode('all', true);
    // Back or ⌘1 from Library, its transition not run yet.
    transition.pending = () => { state.page = 'today'; };
    vi.mocked(setActivePage).mockClear();
    await library.openLibraryTab('skills');
    expect(transition.pending).toBeNull();
    expect(library.libraryMode()).toBe('skills');
    expect(setActivePage).toHaveBeenLastCalledWith('vault');
  });

  it.each([false, true])('canvas navigation waits for an async choice: %s', async allowed => {
    const library = await import('./libraryMode');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = 'workflows';
    library.setLibraryMode('recordings', true);
    library.setResultsType('all');
    const request = library.canvasOpenRequest();
    let resolve!: (value: boolean) => void;
    const choice = new Promise<boolean>(done => { resolve = done; });
    const check = vi.fn(() => choice);
    const release = registerNavigationGuard(check);
    try {
      const pending = library.openLibraryCanvases('accepted');
      expect(library.libraryMode()).toBe('recordings');
      expect(library.canvasOpenRequest()).toBe(request);
      resolve(allowed);
      await pending;
      expect(check).toHaveBeenCalledOnce();
      expect(library.libraryMode()).toBe(allowed ? 'results' : 'recordings');
      expect(library.resultsType()).toBe(allowed ? 'canvases' : 'all');
      expect(library.canvasOpenRequest()).toBe(request + Number(allowed));
      expect(state.saved.get('hollow.library.mode')).toBe('all');
    } finally { release(); }
  });
});

// L6 (11-10): any agent may run the COO, so a fresh one keeps the chosen agent.
it('a fresh COO open keeps the chosen agent', async () => {
  vi.mocked(setChatAgentPersistent).mockClear();
  state.read.mockResolvedValue({ workspace: { id: 'main', kind: 'main', cwd: '/vault/workspaces/main' }, sessions: [] });
  expect(await newWorkspaceSessionOpened('main')).toBe(true);
  expect(setChatAgentPersistent).not.toHaveBeenCalled();
});
it('a failed fresh Main workspace open leaves the current provider alone', async () => {
  vi.mocked(setChatAgentPersistent).mockClear();
  state.read.mockRejectedValue(new Error('Read refused'));
  expect(await newWorkspaceSessionOpened('main')).toBe(false);
  expect(setChatAgentPersistent).not.toHaveBeenCalled();
});
it('reopening a recorded Main conversation preserves its Codex identity', async () => {
  vi.mocked(setChatAgentPersistent).mockClear();
  const summary = { key: 's-main', agent: 'codex', cwd: '/vault/workspaces/main' };
  state.read.mockResolvedValue({ workspace: { id: 'main', kind: 'main', cwd: summary.cwd }, sessions: [summary] });
  state.readConversation.mockResolvedValue({ summary, turns: [] });
  await selectWorkspace('main', summary.key);
  expect(setChatAgentPersistent).toHaveBeenCalledWith('codex');
  expect(setChatAgentPersistent).not.toHaveBeenCalledWith('claude');
});

const cwd = '/vault/workspaces/alpha';
const detail = {
  workspace: { id: 'alpha', cwd, title: 'Alpha', path: 'workspaces/alpha' },
  sessions: [{ key: 's-alpha', cwd, agent: 'claude' }],
};
beforeEach(() => {
  state.page = 'today';
  state.saved.clear(); state.snapshots.clear(); state.turns.length = 0;
  state.cwd = null; state.key = null; state.busy = false; state.draft = '';
  state.read.mockReset().mockResolvedValue(detail); state.readConversation.mockReset();
  state.sessionTarget.mockReset().mockResolvedValue(null);
  setWorkspaceId(null); setWorkspaceDetail(null); setRailWorkspaceIds([]);
});
it('repairs a mismatched live snapshot without dropping its running turn', async () => {
  state.key = 's-alpha'; state.cwd = '/wrong-context'; state.busy = true;
  state.turns.push({ role: 'assistant', text: 'Still streaming' });
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  await selectWorkspace('alpha', 's-alpha');
  expect(state.cwd).toBe(cwd);
  expect(state.key).toBe('s-alpha');
  expect(state.busy).toBe(true);
  expect(state.turns).toEqual([{ role: 'assistant', text: 'Still streaming' }]);
  expect(railWorkspaceIds()).toContain('alpha');
});
it('repairs an already-backgrounded snapshot using the verified workspace', async () => {
  state.snapshots.set('s-alpha', { cwd: '/wrong-context', busy: true, turns: [{ role: 'assistant', text: 'Buffered reply' }] });
  await selectWorkspace('alpha', 's-alpha');
  expect(state.cwd).toBe(cwd);
  expect(state.turns[0].text).toBe('Buffered reply');
  expect(state.readConversation).not.toHaveBeenCalled();
});
it('preserves an already-open conversation draft and makes its workspace reachable', async () => {
  state.cwd = cwd; state.key = 's-alpha'; state.draft = 'Unsent follow-up';
  state.turns.push({ role: 'assistant', text: 'Existing reply' });
  await selectWorkspace('alpha', 's-alpha');
  expect(state.draft).toBe('Unsent follow-up');
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(railWorkspaceIds()).toContain('alpha');
});
// P1-38: Hide from sidebar holds through an open from Work (no
// conversation picked); picking one of its conversations brings it back.
it('a hidden workspace stays hidden when opened without picking a conversation', async () => {
  setRailHiddenIds(['alpha']);
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  await selectWorkspace('alpha');
  expect(railHiddenIds()).toEqual(['alpha']);
  expect(railWorkspaceIds()).toEqual([]);
  // A reopen with its key (launch restore, a canvas Refresh, the Main
  // agent's way back) isn't a pick either.
  await selectWorkspace('alpha', 's-alpha');
  await reopenWorkspaceConversation();
  expect(railHiddenIds()).toEqual(['alpha']);
  expect(await pickConversation('alpha', 's-alpha')).toBe(true);
  expect(railHiddenIds()).toEqual([]);
  expect(railWorkspaceIds()).toContain('alpha');
});
// Overnight 11-03 R4 review: the main agent's key stays the project's after
// its conversation moves to another project; reopen falls back to the newest.
it('reopen falls back to the newest when the COO moved away', async () => {
  const moved = { ...detail, projectAgentKey: 's-main' };
  state.read.mockResolvedValue(moved);
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  await selectWorkspace('alpha');
  state.key = 's-main';
  state.sessionTarget.mockResolvedValue({ workspaceId: 'beta', session: { key: 's-main', cwd: '/vault/workspaces/beta', agent: 'claude' } });
  expect(await reopenWorkspaceConversation()).toBe(true);
  expect(state.key).toBe('s-alpha');
  expect(workspaceError()).toBe('');
});
it('a navigation while the moved COO is looked up wins over the reopen', async () => {
  state.read.mockResolvedValue({ ...detail, projectAgentKey: 's-main' });
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  await selectWorkspace('alpha');
  state.key = 's-main';
  let answer!: (v: { workspaceId: string; session: { key: string; cwd: string; agent: string } }) => void;
  state.sessionTarget.mockReturnValueOnce(new Promise(r => { answer = r; }));
  state.read.mockClear();
  const reopened = reopenWorkspaceConversation();
  state.page = 'tasks';
  answer({ workspaceId: 'beta', session: { key: 's-main', cwd: '/vault/workspaces/beta', agent: 'claude' } });
  expect(await reopened).toBe(false);
  expect(state.read).not.toHaveBeenCalled();
  expect(state.key).toBe('s-main');
});
it('a sidebar visit returns to the conversation and focuses its composer after opening', async () => {
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  vi.mocked(requestChatFocus).mockClear();
  await visitWorkspaceConversation('alpha');
  expect(workspaceId()).toBe('alpha');
  expect(requestChatFocus).toHaveBeenCalledTimes(1);
});
it('a failed sidebar visit does not take composer focus', async () => {
  state.read.mockRejectedValueOnce(new Error('unavailable'));
  vi.mocked(requestChatFocus).mockClear();
  await visitWorkspaceConversation('alpha');
  expect(requestChatFocus).not.toHaveBeenCalled();
});
it('a plain visit does not take composer focus', async () => {
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  vi.mocked(requestChatFocus).mockClear();
  await visitWorkspace('alpha');
  expect(requestChatFocus).not.toHaveBeenCalled();
});
it('an open without a picked conversation still lists a workspace that isn’t hidden', async () => {
  setRailHiddenIds([]);
  state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
  await selectWorkspace('alpha');
  expect(railWorkspaceIds()).toContain('alpha');
});
it('rejects an unrelated session key without rebinding it to this workspace', async () => {
  state.cwd = '/vault/workspaces/beta'; state.key = 's-beta';
  await selectWorkspace('alpha', 's-stranger');
  expect(state.cwd).toBe('/vault/workspaces/beta');
  expect(workspaceError()).toContain('no longer available');
  expect(railWorkspaceIds()).toEqual([]);
});

it('rejects a background session belonging to another workspace', async () => {
  state.snapshots.set('s-stranger', { cwd: '/vault/workspaces/beta', busy: true, turns: [] });
  await selectWorkspace('alpha', 's-stranger');
  expect(state.cwd).toBeNull();
  expect(state.snapshots.has('s-stranger')).toBe(true);
  expect(workspaceError()).toContain('no longer available');
});

describe('launch restore (frontier F4)', () => {
  const load = async () => { vi.resetModules(); return import('./workspaces'); };
  it('reopens the conversation that was on screen', async () => {
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha', key: 's-alpha' }));
    state.readConversation.mockResolvedValue({ summary: { ...detail.sessions[0], key: 's-alpha', resumable: 'none' }, turns: [] });
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceId()).toBe('alpha');
    expect(state.key).toBe('s-alpha');
    expect(state.cwd).toBe(cwd);
  });
  it('opens the workspace on its newest conversation when the saved one is gone', async () => {
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha', key: 's-archived' }));
    state.readConversation.mockResolvedValue({ summary: { ...detail.sessions[0], resumable: 'none' }, turns: [] });
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceId()).toBe('alpha');
    expect(state.key).toBe('s-alpha');
    expect(ws.workspaceError()).toBe('');
  });
  // Overnight 11-03 R4: a new project's main agent isn't listed before its
  // first turn, yet launch reopens it (with its draft); once moved to another
  // project it keeps this project's key, so launch opens the newest instead
  // of failing with "belongs to another project".
  it("reopens a project's COO before its first turn", async () => {
    state.read.mockResolvedValue({ ...detail, projectAgentKey: 's-main' });
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha', key: 's-main' }));
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceError()).toBe('');
    expect(ws.workspaceId()).toBe('alpha');
    expect(state.key).toBe('s-main');
    expect(state.readConversation).not.toHaveBeenCalled();
  });
  it('reopens a COO the project still owns (archived or older than its list)', async () => {
    state.read.mockResolvedValue({ ...detail, projectAgentKey: 's-main' });
    state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: { key: 's-main', cwd, agent: 'claude' } });
    state.readConversation.mockResolvedValue({ summary: { key: 's-main', cwd, agent: 'claude', resumable: 'none' }, turns: [] });
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha', key: 's-main' }));
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceError()).toBe('');
    expect(ws.workspaceId()).toBe('alpha');
    expect(state.key).toBe('s-main');
  });
  it('opens the newest when the COO moved to another project', async () => {
    state.read.mockResolvedValue({ ...detail, projectAgentKey: 's-main' });
    state.sessionTarget.mockResolvedValue({ workspaceId: 'beta', session: { key: 's-main', cwd: '/vault/workspaces/beta', agent: 'claude' } });
    state.readConversation.mockResolvedValue({ summary: { ...detail.sessions[0], resumable: 'none' }, turns: [] });
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'alpha', key: 's-main' }));
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceId()).toBe('alpha');
    expect(state.key).toBe('s-alpha');
    expect(ws.workspaceError()).toBe('');
  });
  it('falls back to Home when the workspace was deleted', async () => {
    state.saved.set('hollow.lastConversation', JSON.stringify({ workspaceId: 'gone', key: null }));
    state.read.mockRejectedValue(new Error('Workspace not found'));
    const ws = await load();
    await ws.restoreLastConversation();
    expect(ws.workspaceId()).toBeNull();
    expect(ws.workspaceLoading()).toBe(false);
    expect(state.saved.has('hollow.lastConversation')).toBe(false);
  });
});

// P1-05: every route to Work (the rail's rows, ⌘1, the palette, the
// native menu) lands on its overview, not whichever tab was last open; a view
// opened from elsewhere (Today, Running, the map) keeps the view it asked for.
describe('Work always lands on its overview (P1-05)', () => {
  it('does not ask to discard edits when the current non-overview page is chosen again', async () => {
    const ws = await import('./workspaces');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = 'workflows'; ws.setWorkspaceId(null);
    const guard = vi.fn(() => false);
    const unregister = registerNavigationGuard(guard);
    try {
      await ws.goToPage('workflows');
      expect(guard).not.toHaveBeenCalled();
    } finally { unregister(); state.page = 'today'; }
  });
  it('a synchronous refusal from another page preserves the selected view and popover', async () => {
    const ws = await import('./workspaces');
    const dest = await import('./destinations');
    const dock = await import('./activityDock');
    const { registerNavigationGuard } = await import('./navigationGuard');
    state.page = 'workflows'; ws.setWorkspaceId(null); dest.resetWorkView(); dock.setActivityDockOpen(true);
    const guard = vi.fn(() => false);
    const unregister = registerNavigationGuard(guard);
    try {
      await dock.openRunningActivity();
      expect(guard).toHaveBeenCalledOnce();
      expect(dest.workView()).toBe('home');
      expect(dock.activityDockOpen()).toBe(true);
    } finally { unregister(); state.page = 'today'; dock.setActivityDockOpen(false); }
  });
  it('selects the accepted view before exposing Work to page observers', async () => {
    const ws = await import('./workspaces');
    const dest = await import('./destinations');
    const dock = await import('./activityDock');
    const runtime = await import('./runtime');
    dest.resetWorkView(); ws.setWorkspaceId('guarded');
    const viewsAtActivation: string[] = [];
    vi.mocked(runtime.setActivePage).mockImplementation(() => { viewsAtActivation.push(dest.workView()); });
    try {
      await dock.openRunningActivity();
      expect(viewsAtActivation).toEqual(['activity']);
    } finally { vi.mocked(runtime.setActivePage).mockReset(); dest.resetWorkView(); ws.setWorkspaceId(null); }
  });
  it.each([false, true])('Manage waits for an asynchronous navigation answer: %s', async allowed => {
    const { registerNavigationGuard } = await import('./navigationGuard');
    const ws = await import('./workspaces');
    const dest = await import('./destinations');
    const dock = await import('./activityDock');
    dest.resetWorkView();
    ws.setWorkspaceId('guarded');
    dock.setActivityDockOpen(true);
    let resolve!: (value: boolean) => void;
    const unregister = registerNavigationGuard(() => new Promise<boolean>(r => { resolve = r; }));
    try {
      const pending = dock.openRunningActivity();
      expect(dest.workView()).toBe('home');
      expect(dock.activityDockOpen()).toBe(true);
      expect(ws.workspaceId()).toBe('guarded');
      resolve(allowed);
      await pending;
      expect(dest.workView()).toBe(allowed ? 'activity' : 'home');
      expect(dock.activityDockOpen()).toBe(!allowed);
      expect(ws.workspaceId()).toBe(allowed ? null : 'guarded');
    } finally { unregister(); dest.resetWorkView(); dock.setActivityDockOpen(false); ws.setWorkspaceId(null); }
  });
  it('a newer navigation invalidates a pending Manage request', async () => {
    const { registerNavigationGuard } = await import('./navigationGuard');
    const ws = await import('./workspaces');
    const dest = await import('./destinations');
    const dock = await import('./activityDock');
    dest.resetWorkView(); ws.setWorkspaceId('guarded'); dock.setActivityDockOpen(true);
    const answers: ((value: boolean) => void)[] = [];
    const unregister = registerNavigationGuard(() => new Promise<boolean>(r => answers.push(r)));
    try {
      const old = dock.openRunningActivity();
      const current = dest.openWorkView('home');
      answers[1](true); await current;
      answers[0](true); await old;
      expect(dest.workView()).toBe('home');
      expect(dock.activityDockOpen()).toBe(true);
    } finally { unregister(); dest.resetWorkView(); dock.setActivityDockOpen(false); ws.setWorkspaceId(null); }
  });
  it("goToPage('today') resets the view; openWorkView keeps its own", async () => {
    const ws = await import('./workspaces');
    const dest = await import('./destinations');
    ws.setWorkspaceId(null);
    dest.openWorkView('activity');
    expect(dest.workView()).toBe('activity');
    ws.goToPage('today');
    expect(dest.workView()).toBe('home');
    ws.setWorkspaceId('w1');
    dest.openWorkView('activity');
    await new Promise(r => setTimeout(r, 0));
    expect(dest.workView()).toBe('activity');
    ws.setWorkspaceId(null);
  });
});

// P1-42: Back and Forward step through workspaces. History lives in
// runtime.ts, which can't import this store; the store tells it which
// workspace and conversation are on screen and how to go back to one.
describe('workspace history entries (P1-42)', () => {
  it('names the open workspace and its conversation, and none on the overview', async () => {
    state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
    await selectWorkspace('alpha', 's-alpha');
    expect(workspaceNav().current()).toEqual({ workspaceId: 'alpha', sessionKey: 's-alpha' });
    setWorkspaceId(null); setWorkspaceDetail(null);
    expect(workspaceNav().current()).toEqual({});
  });
  it('an open records one step: the page change before the workspace loads is quiet', async () => {
    const quiet: boolean[] = [];
    vi.mocked(setActivePage).mockImplementation(() => { quiet.push(navQuiet()); });
    state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
    await selectWorkspace('alpha', 's-alpha');
    expect(quiet).toEqual([true, false]);
    vi.mocked(setActivePage).mockReset();
  });
  it('restores an entry: its workspace and conversation, or the overview', async () => {
    state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
    // The page alone does it when that conversation is already open.
    await selectWorkspace('alpha', 's-alpha');
    state.read.mockClear();
    workspaceNav().restore({ page: 'today', workspaceId: 'alpha', sessionKey: 's-alpha' });
    expect(state.read).not.toHaveBeenCalled();
    // An overview entry leaves the workspace.
    workspaceNav().restore({ page: 'today' });
    expect(workspaceId()).toBeNull();
    // A workspace entry opens it, with its conversation.
    state.cwd = null; state.key = null; state.turns.length = 0;
    workspaceNav().restore({ page: 'today', workspaceId: 'alpha', sessionKey: 's-alpha' });
    await vi.waitFor(() => expect(workspaceId()).toBe('alpha'));
    expect(state.key).toBe('s-alpha');
  });
  it('an entry with no key reopens the new conversation it was, not the newest', async () => {
    state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
    workspaceNav().restore({ page: 'today', workspaceId: 'alpha' });
    await vi.waitFor(() => expect(workspaceId()).toBe('alpha'));
    expect(state.key).toBeNull();
    expect(state.readConversation).not.toHaveBeenCalled();
  });
  it('Back landing elsewhere drops an open still loading for another entry', async () => {
    let land!: (d: typeof detail) => void;
    state.read.mockReturnValueOnce(new Promise(r => { land = r; }));
    workspaceNav().restore({ page: 'today', workspaceId: 'alpha', sessionKey: 's-alpha' });
    expect(workspaceLoading()).toBe(true);
    workspaceNav().restore({ page: 'tasks' });
    expect(workspaceLoading()).toBe(false);
    land(detail);
    await new Promise(r => setTimeout(r, 0));
    expect(workspaceId()).toBeNull();
  });
  it('… a quiet reopen too, which never shows loading', async () => {
    state.readConversation.mockResolvedValue({ summary: detail.sessions[0], turns: [] });
    await selectWorkspace('alpha', 's-alpha');
    let land!: (d: typeof detail) => void;
    state.read.mockReturnValueOnce(new Promise(r => { land = r; }));
    const reopened = reopenWorkspaceConversation();
    expect(workspaceLoading()).toBe(false);
    workspaceNav().restore({ page: 'tasks' });
    vi.mocked(setActivePage).mockClear();
    land(detail);
    await reopened;
    // It landed without pulling the page back to the workspace.
    expect(setActivePage).not.toHaveBeenCalled();
  });
});
