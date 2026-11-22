import { beforeEach, expect, it, vi } from 'vitest';

// Exact-id reopening and the Ask anything draft (merged plan slice 4, B·F15).
const state = vi.hoisted(() => {
  const saved = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => saved.get(key) ?? null,
    setItem: (key: string, value: string) => saved.set(key, value),
    removeItem: (key: string) => saved.delete(key),
  });
  return {
    cwd: null as string | null, key: null as string | null, draft: '',
    turns: [] as { role: string; text: string }[],
    read: vi.fn(), readConversation: vi.fn(), readSessions: vi.fn(async () => []), setActivePage: vi.fn(), sessionTarget: vi.fn(), ensureQuick: vi.fn(), openCoo: vi.fn(),
    applyNewMode: vi.fn(), setAgent: vi.fn(), mode: 'bypassPermissions', native: null as string | null,
    pendingSend: null as { text: string; cwd: string } | null, saved, resets: 0,
    busy: false, pending: null as string | null, shown: [] as string[],
    readGeneration: 0, appliedGeneration: 0,
  };
});
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => ({ api: {
  readWorkspace: state.read, readAgentConversation: state.readConversation, workspaceSessions: state.readSessions,
  sessionTarget: state.sessionTarget, ensureQuickChatsWorkspace: state.ensureQuick,
} }));
vi.mock('./runQueue', () => ({ refreshRunQueue: vi.fn(), runQueue: () => [] }));
vi.mock('./mainAgent', () => ({ openMainAgent: state.openCoo }));
vi.mock('./runtime', () => ({
  chatAgentSessionKey: () => state.key, chatCwd: () => state.cwd,
  chatBusy: () => state.busy, chatInput: () => state.draft,
  chatStoredTranscriptPending: () => state.pending,
  beginStoredTranscriptRead: () => ++state.readGeneration,
  storedTranscriptReadApplies: (_key: string, g: number) => g > state.appliedGeneration,
  markStoredTranscriptApplied: (_key: string, g: number) => { state.appliedGeneration = Math.max(state.appliedGeneration, g); },
  showStoredTranscript: (key: string, stored: typeof state.turns) => {
    state.shown.push(key); state.turns.splice(0, state.turns.length, ...stored, ...state.turns); state.pending = null;
  },
  chatAttachments: () => [], chatTurns: state.turns,
  setChatInput: (s: string) => { state.draft = s; }, setChatAttachments: vi.fn(),
  resetChatState: () => { state.resets++; state.key = null; state.cwd = null; state.turns.length = 0; },
  applyNewChatPermissionMode: state.applyNewMode,
  setChatCwdPersistent: (s: string) => { state.cwd = s; },
  setChatAgentPersistent: state.setAgent, setChatSessionIdPersistent: (id: string | null) => { state.native = id; },
  setChatPermissionMode: (mode: string) => { state.mode = mode; }, chatPermissionMode: () => state.mode,
  setChatAgentSessionKeyPersistent: (s: string) => { state.key = s; },
  setChatTurns: (t: typeof state.turns) => { state.turns.splice(0, state.turns.length, ...t); },
  rebuildChatArtifacts: vi.fn(), setChatSessionLoading: vi.fn(), setActivePage: state.setActivePage,
  setChatPanelOpen: vi.fn(), KNOWN_AGENTS: ['claude', 'codex'],
  canLeaveConversation: () => true, backgroundRunningChat: () => true,
  restoreBackgroundChat: () => false, isBackgrounded: () => false,
  backgroundChatCwd: () => undefined, backgroundedKeyForCwd: () => null, backgroundChatKeys: () => [],
  flashFooter: vi.fn(), requestChatFocus: vi.fn(), activePage: () => 'today', shownPage: () => 'today',
  setPendingChatSend: (v: { text: string; cwd: string }) => { state.pendingSend = v; },
  pendingChatSend: () => state.pendingSend,
  sidePaneHosted: () => true, sidePaneOpen: () => false, setSidePaneOpen: vi.fn(), openSidePane: vi.fn(),
}));
import {
  openProjectAgent, openSessionByKey, openSessionForAgent, selectWorkspace, workspaceId, workspaceError, setWorkspaceError, setWorkspaceDetail, setWorkspaceId,
  quickAskDraft, setQuickAskDraft, sendQuickAsk, askProject, setAskProject, newWorkspaceSessionOpened, railHiddenIds, setRailHiddenIds, refreshWorkspaceSessions,
} from './workspaces';
import { appSessionRequest, setAppSessionRequest, sessionRequestStep } from './sessionRequest';
import { acceptFirstDraftSession, rememberBeforeDraftReset, conversationDrafts } from './conversationDrafts';
import { rememberConversationMode } from '../permModes';

const cwd = '/vault/workspaces/alpha';
const newest = { key: 's-new', cwd, agent: 'claude', turnCount: 1 };
// Past the rail's 25 newest and the workspace page's newest 1,000: absent
// from every list, reachable only by its key.
const old = { key: 's-old', cwd, agent: 'claude', turnCount: 3 };
const alpha = { workspace: { id: 'alpha', cwd, title: 'Alpha', path: 'workspaces/alpha', status: 'active', modified: '' }, sessions: [newest] };

it('a conversation chip lookup cannot take over after its owner changes', async () => {
  let finish!: (value: { workspaceId: string; session: typeof old }) => void;
  let current = true;
  state.sessionTarget.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  const opening = openSessionByKey('s-old', () => current);
  current = false;
  finish({ workspaceId: 'alpha', session: old });
  state.read.mockClear();
  expect(await opening).toBe(false);
  expect(state.read).not.toHaveBeenCalled();
});

it('a conversation chip owner stays guarded through workspace loading', async () => {
  let finish!: (value: typeof alpha) => void;
  let current = true;
  state.sessionTarget.mockResolvedValueOnce({ workspaceId: 'alpha', session: old });
  state.read.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  const opening = openSessionByKey('s-old', () => current);
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  current = false;
  finish(alpha);
  expect(await opening).toBe(false);
  expect(state.key).not.toBe('s-old');
});
const quickCwd = '/vault/workspaces/quick-chats';
const quick = { id: 'quick-chats', cwd: quickCwd, title: 'Quick chats', path: 'workspaces/quick-chats', status: 'active', modified: '', kind: 'quick' };

it('restores recorded mode and native resume for the exact workspace conversation', async () => {
  state.readConversation.mockResolvedValue({ summary: { ...newest, resumable: 'native', harnessSessionId: 'native-plan' }, permissionMode: 'plan', turns: [] });
  await selectWorkspace('alpha', 's-new');
  expect(state.mode).toBe('plan'); expect(state.native).toBe('native-plan');
});

it('keeps two conversations in one project separate when switching drafts', async () => {
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  await selectWorkspace('alpha', 's-new');
  state.draft = 'Only for the newer chat';
  await selectWorkspace('alpha', 's-old');
  expect(state.draft).toBe('');
  state.draft = 'Only for the older chat';
  await selectWorkspace('alpha', 's-new');
  expect(state.draft).toBe('Only for the newer chat');
  await selectWorkspace('alpha', 's-old');
  expect(state.draft).toBe('Only for the older chat');
});

it('assigns a legacy workspace draft to its default conversation, not the first clicked chat', async () => {
  state.saved.set('hollow.draft.alpha', 'Legacy draft');
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  await selectWorkspace('alpha', 's-old');
  expect(state.draft).toBe('');
  await selectWorkspace('alpha', 's-new');
  expect(state.draft).toBe('Legacy draft');
});

it('clears the New chat slot after acceptance while keeping a newly typed follow-up with its session', async () => {
  await newWorkspaceSessionOpened('alpha');
  state.draft = 'Typed while send was accepted';
  acceptFirstDraftSession(cwd, 's-new'); state.key = 's-new';
  await newWorkspaceSessionOpened('alpha');
  expect(state.draft).toBe('');
  await selectWorkspace('alpha', 's-new');
  expect(state.draft).toBe('Typed while send was accepted');
});

it('saves the departing draft before a history reset clears the composer', async () => {
  await selectWorkspace('alpha', 's-new'); state.draft = 'Keep this unsent text';
  rememberBeforeDraftReset(); state.draft = ''; state.key = null; state.turns.length = 0;
  await selectWorkspace('alpha', 's-new');
  expect(state.draft).toBe('Keep this unsent text');
});

it('a changed session key alone cannot overwrite that conversations foreign draft', async () => {
  await selectWorkspace('alpha', 's-new');
  conversationDrafts.save('alpha', 'foreign', 'Keep foreign text', []);
  state.draft = 'Still the original composer'; state.key = 'foreign';
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  await selectWorkspace('alpha', 's-old');
  expect(conversationDrafts.read('alpha', 'foreign').text).toBe('Keep foreign text');
});

it('does not resume native under a changed local choice or unknown recorded mode', async () => {
  const { rememberConversationMode } = await import('../permModes');
  rememberConversationMode('s-old', 'plan');
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  state.readConversation.mockResolvedValue({ summary: { ...old, resumable: 'native', harnessSessionId: 'native-bypass' }, permissionMode: 'bypassPermissions', turns: [] });
  await selectWorkspace('alpha', 's-old');
  expect(state.mode).toBe('plan'); expect(state.native).toBeNull();
  state.readConversation.mockResolvedValue({ summary: { ...newest, resumable: 'native', harnessSessionId: 'unknown-native' }, turns: [] });
  await selectWorkspace('alpha', 's-new');
  expect(state.mode).toBe('acceptEdits'); expect(state.native).toBeNull();
});

// L8 (11-10): a project's one conversation is its lead, so Home's Ask with
// a project sends there, once the lead is open.
it('Home project selection sends to the project lead once it opens', async () => {
  state.read.mockResolvedValue({ ...alpha, projectAgentKey: 's-project-alpha' });
  setAskProject('alpha'); setQuickAskDraft('Plan a project');
  expect(await sendQuickAsk()).toBe(true);
  expect(state.key).toBe('s-project-alpha');
  expect(state.pendingSend).toMatchObject({text:'Plan a project',cwd,sessionKey:'s-project-alpha',preset:true});
  expect(quickAskDraft()).toBe('');
});

it('Home retains its draft when the project becomes unavailable', async () => {
  setAskProject('alpha'); setQuickAskDraft('Keep this ask');
  state.read.mockResolvedValue({...alpha,workspace:{...alpha.workspace,status:'archived'}});
  expect(await sendQuickAsk()).toBe(false);
  expect(quickAskDraft()).toBe('Keep this ask');
  expect(state.pendingSend).toBeNull();
});

it('a late project read cannot steal another conversation or its draft', async () => {
  let finish!: (value: typeof alpha) => void;
  state.read.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
  setAskProject('alpha'); setQuickAskDraft('Keep this ask');
  const send = sendQuickAsk();
  state.key = 'foreign'; state.cwd = '/foreign'; state.draft = 'Foreign';
  finish(alpha);
  expect(await send).toBe(false);
  expect(state.key).toBe('foreign'); expect(state.draft).toBe('Foreign');
  expect(state.pendingSend).toBeNull(); expect(quickAskDraft()).toBe('Keep this ask');
});

it('Home never replaces another pending send', async () => {
  setAskProject('alpha'); setQuickAskDraft('Keep this ask');
  const foreign = {text:'Foreign send',cwd:'/foreign'}; state.pendingSend = foreign;
  expect(await sendQuickAsk()).toBe(false);
  expect(state.pendingSend).toBe(foreign); expect(quickAskDraft()).toBe('Keep this ask');
});

it('a replaced session refresh waits for the latest successful refresh', async () => {
  let first!: (v: never[]) => void, second!: (v: never[]) => void;
  state.readSessions.mockReturnValueOnce(new Promise(resolve => { first = resolve; }));
  state.readSessions.mockReturnValueOnce(new Promise(resolve => { second = resolve; }));
  const a = refreshWorkspaceSessions(), b = refreshWorkspaceSessions();
  first([]); second([]);
  expect(await a).toBe(true); expect(await b).toBe(true);
});

it('a replaced session refresh reports the newest read failure', async () => {
  let first!: (v: never[]) => void, reject!: (e: Error) => void;
  state.readSessions.mockReturnValueOnce(new Promise(resolve => { first = resolve; }));
  state.readSessions.mockReturnValueOnce(new Promise((_resolve, fail) => { reject = fail; }));
  const a = refreshWorkspaceSessions(), b = refreshWorkspaceSessions();
  first([]); reject(new Error('No activity'));
  expect(await a).toBe(false); expect(await b).toBe(false);
});

beforeEach(() => {
  state.readSessions.mockReset().mockResolvedValue([]);
  state.mode = 'bypassPermissions'; state.native = null;
  state.saved.clear(); state.turns.length = 0;
  state.cwd = null; state.key = null; state.draft = ''; state.pendingSend = null;
  state.busy = false; state.pending = null; state.shown.length = 0;
  state.readGeneration = 0; state.appliedGeneration = 0;
  state.read.mockReset().mockImplementation(async (id: string) => {
    if (id === 'alpha') return alpha;
    if (id === 'quick-chats') return { workspace: quick, sessions: [] };
    throw new Error('Workspace not found');
  });
  state.readConversation.mockReset().mockImplementation(async (key: string) => ({ summary: { ...(key === 's-old' ? old : newest), resumable: 'none' }, turns: [{ role: 'user', text: `asked in ${key}`, tools: [] }] }));
  state.sessionTarget.mockReset().mockResolvedValue(null);
  state.ensureQuick.mockReset().mockResolvedValue(quick);
  setWorkspaceId(null); setWorkspaceDetail(null); setQuickAskDraft(''); setAskProject(null);
  state.setActivePage.mockReset(); setAppSessionRequest(null); setWorkspaceError('');
  state.applyNewMode.mockReset(); state.setAgent.mockReset();
});

// P1-38: every openSessionByKey caller is a person's click on one
// conversation (Overnight, Review, Runbooks, a canvas's source), so it is a
// pick: a hidden workspace comes back to the rail.
it('opening a conversation by its key brings a hidden workspace back to the rail', async () => {
  setRailHiddenIds(['alpha']);
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  expect(await openSessionByKey('s-old')).toBe(true);
  expect(railHiddenIds()).toEqual([]);
});

it('opens a conversation the lists no longer carry by its exact key', async () => {
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  expect(await openSessionByKey('s-old')).toBe(true);
  expect(state.sessionTarget).toHaveBeenCalledWith('s-old');
  expect(workspaceId()).toBe('alpha');
  expect(state.key).toBe('s-old');
  expect(state.readConversation).toHaveBeenCalledWith('s-old');
  expect(state.turns[0].text).toBe('asked in s-old');
  // An opened conversation keeps its own permission mode (P0-19).
  expect(state.applyNewMode).not.toHaveBeenCalled();
});

it("a workspace's new conversation starts in the mode for new chats (P0-19)", async () => {
  await selectWorkspace('alpha', undefined, true);
  expect(workspaceId()).toBe('alpha');
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(state.applyNewMode).toHaveBeenCalledTimes(1);
});

it('a cancelled creation open leaves the newer draft and error untouched (P1-36)', async () => {
  let finish!: (value: typeof alpha) => void;
  state.read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  let owned = true;
  const opening = newWorkspaceSessionOpened('alpha', () => owned);
  expect(state.read).toHaveBeenCalledWith('alpha');
  owned = false;
  state.draft = 'A newer page owns this draft';
  setWorkspaceError('A newer page owns this error');
  finish(alpha);
  expect(await opening).toBe(false);
  expect(workspaceId()).toBeNull();
  expect(state.draft).toBe('A newer page owns this draft');
  expect(workspaceError()).toBe('A newer page owns this error');
  expect(state.applyNewMode).not.toHaveBeenCalled();
});

it('never opens the newest conversation in place of a key it cannot find', async () => {
  expect(await openSessionByKey('s-gone')).toBe(false);
  expect(workspaceId()).toBeNull();
  expect(state.key).toBeNull();
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(workspaceError()).toContain('no longer exists');
});

it('refuses a key that belongs to another workspace', async () => {
  state.sessionTarget.mockResolvedValue({ workspaceId: 'beta', session: { ...old, cwd: '/vault/workspaces/beta' } });
  await selectWorkspace('alpha', 's-old');
  expect(state.key).toBeNull();
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(workspaceError()).toContain('no longer available');
});

it("opens an agent's navigate(session) by exact key past the rail, not in plain Chat", async () => {
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: old });
  expect(await openSessionForAgent('s-old')).toBe('workspace');
  expect(state.sessionTarget).toHaveBeenCalledWith('s-old');
  expect(workspaceId()).toBe('alpha');
  expect(state.key).toBe('s-old');
  expect(state.turns[0].text).toBe('asked in s-old');
});

it("sends only a conversation in no workspace to Chat, and tells the agent about an unknown key", async () => {
  const loose = { ...old, key: 's-loose', cwd: '/vault' };
  state.sessionTarget.mockResolvedValueOnce({ workspaceId: null, session: loose });
  expect(await openSessionForAgent('s-loose')).toBe('chat');
  expect(workspaceId()).toBeNull();
  expect(state.setActivePage).toHaveBeenLastCalledWith('chat');
  expect(appSessionRequest()).toEqual({ key: 's-loose', session: loose });
  await expect(openSessionForAgent('s-gone')).rejects.toThrow('no conversation has the key s-gone');
  expect(state.key).toBeNull();
  expect(state.readConversation).not.toHaveBeenCalled();
});

it('opens a conversation in no workspace on the Chat page, not as an error (Home, Activity)', async () => {
  // A vault-root chat waiting on the person: Home's and Activity's rows
  // must land on it, like navigate('session') does, never a dead-end toast.
  state.sessionTarget.mockResolvedValue({ workspaceId: null, session: { ...old, key: 's-loose', cwd: '/vault' } });
  expect(await openSessionByKey('s-loose')).toBe(true);
  expect(state.sessionTarget).toHaveBeenCalledWith('s-loose');
  expect(state.setActivePage).toHaveBeenLastCalledWith('chat');
  expect(appSessionRequest()?.key).toBe('s-loose');
  expect(workspaceId()).toBeNull();
  expect(workspaceError()).toBe('');
});

it("opens a no-workspace conversation Chat's own list doesn't carry (older than its 300, or archived)", async () => {
  // openSessionByKey reports success, so Chat must open it: the request
  // carries the summary found by exact key, and Chat opens that when its
  // list (here: only the newest conversation) lacks the key.
  const archived = { ...old, key: 's-archived', cwd: '/vault' };
  state.sessionTarget.mockResolvedValue({ workspaceId: null, session: archived });
  expect(await openSessionByKey('s-archived')).toBe(true);
  const want = appSessionRequest();
  expect(want).not.toBeNull();
  expect(sessionRequestStep(want!, [newest as never])).toEqual({ open: archived });
  // Before Chat's list has loaded it doesn't wait for a list that won't have it.
  expect(sessionRequestStep(want!, undefined)).toEqual({ open: archived });
});

// Quick chats (2026-11-14): a prompt with no project starts a quick chat
// again (L8 had sent it to the COO).
it('keeps the Ask anything draft until the quick chat exists', async () => {
  setQuickAskDraft('Plan the offsite');
  state.ensureQuick.mockRejectedValueOnce(new Error('disk full'));
  expect(await sendQuickAsk()).toBe(false);
  expect(quickAskDraft()).toBe('Plan the offsite');
  expect(state.pendingSend).toBeNull();

  // The workspace exists but the conversation does not open: still kept.
  state.read.mockRejectedValueOnce(new Error('unreadable'));
  expect(await sendQuickAsk()).toBe(false);
  expect(quickAskDraft()).toBe('Plan the offsite');
  expect(state.pendingSend).toBeNull();

  expect(await sendQuickAsk()).toBe(true);
  expect(workspaceId()).toBe('quick-chats');
  expect(state.pendingSend).toEqual({ text: 'Plan the offsite', cwd: quickCwd });
  expect(quickAskDraft()).toBe('');
  expect(state.openCoo).not.toHaveBeenCalled();
});


it('opens a project COO before its first message and preserves its separate draft', async () => {
  const project = { ...alpha, projectAgentKey: 's-project-alpha' };
  state.read.mockResolvedValue(project);
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.key).toBe('s-project-alpha');
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(state.setAgent).not.toHaveBeenCalled();
  state.draft = 'COO draft';
  await newWorkspaceSessionOpened('alpha');
  expect(state.key).toBeNull();
  expect(state.draft).toBe('');
  state.draft = 'Independent task draft';
  await openProjectAgent('alpha');
  expect(state.draft).toBe('COO draft');
  await newWorkspaceSessionOpened('alpha');
  expect(state.draft).toBe('Independent task draft');
});

it('brings an unsent project lead back from its row without resetting it, once this run opened it', async () => {
  state.read.mockResolvedValue({ ...alpha, projectAgentKey: 's-project-alpha' });
  expect(await openProjectAgent('alpha')).toBe(true);
  const resets = state.resets;
  state.draft = 'Lead draft';
  // The sidebar row again (L8): the same conversation, not a remount.
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.resets).toBe(resets);
  expect(state.key).toBe('s-project-alpha');
  expect(state.draft).toBe('Lead draft');
  // A launch restore holds the key but no detail yet: it still loads.
  setWorkspaceDetail(null);
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.resets).toBe(resets + 1);
});

it('reopens a project COO with its recorded provider, permission and native session', async () => {
  const main = { ...old, key: 's-project-alpha', agent: 'codex', resumable: 'native', harnessSessionId: 'codex-native' };
  state.read.mockResolvedValue({ ...alpha, projectAgentKey: main.key });
  state.sessionTarget.mockResolvedValue({ workspaceId: 'alpha', session: main });
  state.readConversation.mockResolvedValue({ summary: main, permissionMode: 'plan', turns: [{ role: 'user', text: 'Remember our decision', tools: [] }] });
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.key).toBe(main.key);
  expect(state.setAgent).toHaveBeenCalledWith('codex');
  expect(state.mode).toBe('plan');
  expect(state.native).toBe('codex-native');
  expect(state.turns[0].text).toBe('Remember our decision');
  expect(state.applyNewMode).not.toHaveBeenCalled();
});

// #1067 review: a mode chosen for an unsent project main agent survives
// reopening it; without one, the new-chat default stays.
it('reopens an unsent project COO in the mode chosen for it', async () => {
  state.read.mockResolvedValue({ ...alpha, projectAgentKey: 's-project-chosen' });
  state.mode = 'bypassPermissions';
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.applyNewMode).toHaveBeenCalled();
  expect(state.mode).toBe('bypassPermissions');
  rememberConversationMode('s-project-chosen', 'plan');
  await newWorkspaceSessionOpened('alpha');
  expect(await openProjectAgent('alpha')).toBe(true);
  expect(state.key).toBe('s-project-chosen');
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(state.mode).toBe('plan');
});

it('refuses a moved COO and a failed ownership lookup without replacing the current draft', async () => {
  state.read.mockResolvedValue({ ...alpha, projectAgentKey: 's-project-alpha' });
  state.draft = 'Keep this draft';
  state.sessionTarget.mockResolvedValue({ workspaceId: 'beta', session: { ...old, key: 's-project-alpha' } });
  expect(await openProjectAgent('alpha')).toBe(false);
  expect(state.key).toBeNull();
  expect(state.draft).toBe('Keep this draft');
  expect(workspaceError()).toContain('another project');
  state.sessionTarget.mockRejectedValue(new Error('History unavailable'));
  expect(await openProjectAgent('alpha')).toBe(false);
  expect(workspaceError()).toContain('History unavailable');
  expect(state.draft).toBe('Keep this draft');
});

it('does not let a delayed project agent open replace a newer selection', async () => {
  let finish!: (value: typeof alpha & { projectAgentKey: string }) => void;
  state.read.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const opening = openProjectAgent('alpha');
  await newWorkspaceSessionOpened('alpha');
  state.draft = 'The newer selection owns this';
  finish({ ...alpha, projectAgentKey: 's-project-alpha' });
  expect(await opening).toBe(false);
  expect(state.key).toBeNull();
  expect(state.draft).toBe('The newer selection owns this');
});

// Overnight 11-13 c-11: after a reload mid-turn the key and cwd come back
// from localStorage without turns, and the run-queue adoption (busy) or a
// live event (a bubble) lands before the open checks them. That used to
// count as "already open", so earlier turns never loaded.
it('loads the stored transcript of a conversation restored at launch even after its live turn arrived', async () => {
  state.key = 's-new'; state.cwd = cwd; state.pending = 's-new'; state.busy = true;
  state.turns.push({ role: 'assistant', text: 'Live reply so far' });
  state.readConversation.mockResolvedValue({ summary: newest, turns: [{ role: 'user', text: 'Earlier question', tools: [], status: 'success' }] });
  const resets = state.resets;
  await selectWorkspace('alpha', 's-new');
  expect(state.readConversation).toHaveBeenCalledWith('s-new');
  expect(state.shown).toEqual(['s-new']);
  // Stored first, the live running turn after it, nothing reset.
  expect(state.turns.map(t => t.text)).toEqual(['Earlier question', 'Live reply so far']);
  expect(state.resets).toBe(resets);
});

it('still treats a conversation whose transcript is loaded as already open', async () => {
  state.key = 's-new'; state.cwd = cwd; state.busy = true;
  state.turns.push({ role: 'assistant', text: 'Live reply so far' });
  await selectWorkspace('alpha', 's-new');
  expect(state.readConversation).not.toHaveBeenCalled();
  expect(state.turns.map(t => t.text)).toEqual(['Live reply so far']);
});

// Codex review: the open's first read can land after the re-read at the
// turn's end; that older copy must not replace the finished reply.
it('drops an open\'s stored read that lands after a newer read of the same transcript', async () => {
  state.key = 's-new'; state.cwd = cwd; state.pending = 's-new'; state.busy = true;
  state.turns.push({ role: 'assistant', text: 'Live reply so far' });
  let resolve!: (value: unknown) => void;
  state.readConversation.mockReturnValue(new Promise(r => { resolve = r; }));
  const resets = state.resets;
  const opening = selectWorkspace('alpha', 's-new');
  await vi.waitFor(() => expect(state.readConversation).toHaveBeenCalled());
  // The turn ends; its re-read takes a newer generation and shows the whole reply.
  state.readGeneration++; state.appliedGeneration = state.readGeneration; state.pending = null; state.busy = false;
  state.turns.splice(0, state.turns.length, { role: 'user', text: 'Earlier question' }, { role: 'assistant', text: 'The whole reply' });
  resolve({ summary: newest, turns: [{ role: 'user', text: 'Earlier question', tools: [], status: 'success' }] });
  await opening;
  expect(state.turns.map(t => t.text)).toEqual(['Earlier question', 'The whole reply']);
  expect(state.resets).toBe(resets);
});

// Codex re-check: a newer read that started but never wrote the screen (it
// failed, or gave up while another turn was busy) must not block this one.
it('applies an open\'s stored read when a newer read began but never wrote the screen', async () => {
  state.key = 's-new'; state.cwd = cwd; state.pending = 's-new'; state.busy = true;
  state.turns.push({ role: 'assistant', text: 'Live reply so far' });
  let resolve!: (value: unknown) => void;
  state.readConversation.mockReturnValue(new Promise(r => { resolve = r; }));
  const opening = selectWorkspace('alpha', 's-new');
  await vi.waitFor(() => expect(state.readConversation).toHaveBeenCalled());
  state.readGeneration++;
  resolve({ summary: newest, turns: [{ role: 'user', text: 'Earlier question', tools: [], status: 'success' }] });
  await opening;
  expect(state.turns.map(t => t.text)).toEqual(['Earlier question', 'Live reply so far']);
});
