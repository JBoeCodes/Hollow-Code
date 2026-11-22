import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ id: 'a', cwd: '/a', chat: '/foreign', draft: 'Foreign draft', focus: vi.fn(), flash: vi.fn(), request: vi.fn(), pane: vi.fn() }));
vi.mock('./runtime', () => ({
  chatCwd: () => state.chat, chatInput: () => state.draft,
  setChatInput: (value: string) => { state.draft = value; },
  requestChatFocus: state.request, flashFooter: state.flash,
}));
// Run 15 P1: in a narrow window the Preview pane covers the page; a draft closes it.
vi.mock('./preview', () => ({ closePreview: () => state.pane(false) }));
vi.mock('./workspaces', () => ({
  workspaceId: () => state.id, workspaceDetail: () => ({ workspace: { cwd: state.cwd } }),
  focusWorkspaceConversation: state.focus,
}));
import { draftInWorkspace, draftInCurrentConversation } from './workspaceDraft';

beforeEach(() => {
  Object.assign(state, { id: 'a', cwd: '/a', chat: '/foreign', draft: 'Foreign draft' });
  vi.clearAllMocks();
  vi.stubGlobal('window', { innerWidth: 1440 });
  state.focus.mockReset();
});

it('leaves the foreign draft intact until focus succeeds, coalesces pending and later duplicates', async () => {
  let finish!: (ok: boolean) => void;
  state.focus.mockImplementation(() => new Promise<boolean>(resolve => { finish = resolve; }));
  const first = draftInWorkspace('Revise result');
  expect(draftInWorkspace('Revise result')).toBe(first);
  await Promise.resolve();
  expect(state.draft).toBe('Foreign draft');
  state.chat = '/a'; state.draft = 'Workspace draft'; finish(true);
  expect(await first).toBe(true);
  state.focus.mockResolvedValue(true);
  await draftInWorkspace('Revise result');
  expect(state.draft).toBe('Workspace draft\n\nRevise result');
});

it.each(['false', 'throw', 'reject'])('preserves the draft on %s and allows a retry', async kind => {
  state.focus.mockImplementation(() => {
    if (kind === 'throw') throw new Error('read');
    return kind === 'reject' ? Promise.reject(new Error('read')) : false;
  });
  expect(await draftInWorkspace('Request')).toBe(false);
  expect(state.draft).toBe('Foreign draft');
  expect(state.flash).toHaveBeenCalledWith(expect.stringContaining('nothing was added'), 7000);
  state.chat = '/a'; state.focus.mockResolvedValue(true);
  expect(await draftInWorkspace('Request')).toBe(true);
});

it.each(['id', 'cwd', 'chat'])('refuses a changed %s after focus', async field => {
  state.focus.mockImplementation(async () => {
    state.chat = '/a'; state[field as 'id' | 'cwd' | 'chat'] = 'changed';
    return true;
  });
  expect(await draftInWorkspace('Request')).toBe(false);
  expect(state.draft).toBe('Foreign draft');
});

it('does not open a workspace after navigation before the microtask', async () => {
  const pending = draftInWorkspace('Request'); state.id = 'b';
  expect(await pending).toBe(false);
  expect(state.focus).not.toHaveBeenCalled();
});

it('keeps Library requests in the current conversation', () => {
  draftInCurrentConversation('Passage\n\n');
  expect(state.draft).toBe('Foreign draft\n\nPassage\n\n');
  expect(state.focus).not.toHaveBeenCalled();
});

it('reveals the composer after success at a narrow width, but preserves the panel on failure', async () => {
  vi.stubGlobal('window', { innerWidth: 720 });
  state.focus.mockResolvedValue(false);
  await draftInWorkspace('Request');
  expect(state.pane).not.toHaveBeenCalled();
  state.chat = '/a'; state.focus.mockResolvedValue(true);
  await draftInWorkspace('Request');
  expect(state.pane).toHaveBeenCalledWith(false);
});
