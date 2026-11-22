import { beforeEach, expect, it, vi } from 'vitest';
import { MAIN_AGENT_ID } from '../workspaceTypes';

// S4 review: the orb remembers where you were as the page id that opens it,
// so from Library → Conversations (`chat`) it goes back there, not to Notes.
const state = vi.hoisted(() => ({
  page: 'vault', shown: 'chat', workspaceId: null as string | null,
  setActivePage: vi.fn(), showWorkOverview: vi.fn(), selectWorkspace: vi.fn(async () => true),
}));
vi.mock('../api', () => ({ api: {
  ensureMainAgentWorkspace: async () => ({ id: 'main-agent', title: 'COO', status: 'active', path: 'workspaces/main-agent', cwd: '/vault/workspaces/main-agent', modified: '', kind: 'main' }),
  readWorkspace: async () => ({ sessions: [] }),
} }));
vi.mock('./runtime', () => ({
  activePage: () => state.page, shownPage: () => state.shown,
  backgroundChatCwd: () => undefined, canLeaveConversation: () => true, chatAgentSessionKey: () => null,
  chatBusy: () => false, chatCwd: () => null, flashFooter: vi.fn(), requestChatFocus: vi.fn(),
  setActivePage: state.setActivePage,
}));
vi.mock('./workspaces', () => ({
  newWorkspaceSessionOpened: async (id: string) => { state.page = 'today'; state.shown = 'today'; state.workspaceId = id; return true; },
  selectWorkspace: state.selectWorkspace, sessionWorking: () => false, setWorkspaceConversationVisible: vi.fn(),
  setWorkspaces: vi.fn(), showWorkOverview: state.showWorkOverview,
  workspaceDetail: () => (state.workspaceId ? { workspace: { cwd: '/vault/workspaces/main-agent' } } : null),
  workspaceId: () => state.workspaceId, workspaceSessions: () => ({}), workspaces: () => [],
}));
vi.mock('./runQueue', () => ({ runQueue: () => [] }));
vi.mock('./sessionStates', () => ({ sessionState: () => null }));
import { mainAgentOpen, openMainAgent, toggleMainAgent } from './mainAgent';

beforeEach(() => {
  state.page = 'vault'; state.shown = 'chat'; state.workspaceId = null;
  state.setActivePage.mockClear(); state.showWorkOverview.mockClear(); state.selectWorkspace.mockClear();
});

it('goes back from the COO to Conversation history, not to Library’s Notes', async () => {
  expect(await openMainAgent()).toBe(true);
  expect(state.workspaceId).toBe(MAIN_AGENT_ID);
  expect(mainAgentOpen()).toBe(true);
  await toggleMainAgent();
  expect(state.showWorkOverview).toHaveBeenCalled();
  expect(state.setActivePage).toHaveBeenLastCalledWith('chat');
});

it('goes back to Library’s own view by its page id', async () => {
  state.shown = 'vault';
  expect(await openMainAgent()).toBe(true);
  await toggleMainAgent();
  expect(state.setActivePage).toHaveBeenLastCalledWith('vault');
});
