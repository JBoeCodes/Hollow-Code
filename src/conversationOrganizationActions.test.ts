import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ change: vi.fn(), select: vi.fn(), before: vi.fn(), toast: vi.fn(), failed: vi.fn(), refresh: vi.fn() }));
vi.mock('./api', () => ({ api: { readAgentSession: async () => ({ summary: { key: 's-chat', lastActivityAt: '2030-01-01T00:00:00Z' } }) } }));
const saved = vi.hoisted(() => ({ row: undefined as unknown }));
vi.mock('./store/conversationOrganization', () => ({ conversationOrganization: () => ({ 's-chat': saved.row }) }));
vi.mock('./conversationOrganizationLoad', () => ({ organizationApi: { change: state.change }, refreshConversationOrganization: async () => true, acceptOrganization: vi.fn() }));
vi.mock('./store', () => ({ chatAgentSessionKey: () => 's-chat', dropChatInteractionsFor: vi.fn(), flashFooter: vi.fn(), flashWithUndo: state.toast }));
vi.mock('./store/organizationUndo', () => ({ ORGANIZATION_UNDO_MS: 5000, rememberOrganizationUndo: vi.fn() }));
vi.mock('./snoozePicker', () => ({ pickSnoozeTime: async () => '2031-01-01T09:00:00.000Z' }));
vi.mock('./store/workspaces', () => ({ lifecycleFailed: state.failed, openSessionByKey: vi.fn(), refreshWorkspaceSessions: state.refresh, selectWorkspace: state.select, workspaceId: () => 'quick-chats' }));
import { organizeConversation } from './conversationOrganizationActions';
const settled = { key: 's-chat', revision: 7, settled: true, session: { lastActivityAt: '2030-01-01T00:00:00Z' } };
beforeEach(() => { vi.clearAllMocks(); saved.row = undefined; state.change.mockResolvedValue(settled); });
it('starts a fresh rail chat only after settlement succeeds', async () => {
  await organizeConversation('s-chat', 'settle', undefined, { fresh: 'quick-chats', beforeSettled: state.before });
  expect(state.before).toHaveBeenCalledOnce();
  expect(state.select).toHaveBeenCalledWith('quick-chats', undefined, true);
  expect(state.toast.mock.calls[0][0].message).toContain('closed questions stay closed');
});
it('a refused settlement leaves the open chat and focus hooks alone', async () => {
  state.change.mockRejectedValue('CONVERSATION_CHANGED');
  await organizeConversation('s-chat', 'settle', undefined, { fresh: 'quick-chats', beforeSettled: state.before });
  expect(state.before).not.toHaveBeenCalled();
  expect(state.select).not.toHaveBeenCalled();
  expect(state.failed).toHaveBeenCalledWith('This conversation changed. Reopen its menu and try again.');
});
it('Undo carries the exact accepted revision and activity', async () => {
  await organizeConversation('s-chat', 'settle');
  await state.toast.mock.calls[0][0].undo();
  expect(state.change).toHaveBeenLastCalledWith({ key: 's-chat', action: 'wake', expectedRevision: 7, expectedActivity: '2030-01-01T00:00:00Z' });
});
it('every organization notice lasts 5 seconds', async () => {
  for (const action of ['settle', 'snooze', 'pin'] as const) {
    state.toast.mockClear();
    await organizeConversation('s-chat', action);
    expect(state.toast.mock.calls[0][0].ttlMs).toBe(5000);
  }
});
it('Undo on pin unpins at the accepted revision', async () => {
  await organizeConversation('s-chat', 'pin');
  await state.toast.mock.calls[0][0].undo();
  expect(state.change).toHaveBeenLastCalledWith({ key: 's-chat', action: 'unpin', expectedRevision: 7, expectedActivity: '2030-01-01T00:00:00Z' });
});
it('Undo on wake puts a settled chat back, or a snooze to its old time', async () => {
  saved.row = { revision: 3, settled: true, snoozedUntil: null };
  await organizeConversation('s-chat', 'wake');
  await state.toast.mock.calls[0][0].undo();
  expect(state.change).toHaveBeenLastCalledWith({ key: 's-chat', action: 'settle', expectedRevision: 7, expectedActivity: '2030-01-01T00:00:00Z' });
  state.toast.mockClear();
  saved.row = { revision: 3, settled: false, snoozedUntil: '2999-01-01T09:00:00.000Z' };
  await organizeConversation('s-chat', 'wake');
  await state.toast.mock.calls[0][0].undo();
  expect(state.change).toHaveBeenLastCalledWith({ key: 's-chat', action: 'snooze', until: '2999-01-01T09:00:00.000Z', expectedRevision: 7, expectedActivity: '2030-01-01T00:00:00Z' });
});
it('a wake with nothing to put back offers no Undo', async () => {
  await organizeConversation('s-chat', 'wake');
  expect(state.toast).not.toHaveBeenCalled();
});
