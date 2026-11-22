import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ invoke: vi.fn(), toast: vi.fn(), change: vi.fn(), refresh: vi.fn(), failed: vi.fn() }));
vi.mock('./api', () => ({ api: {}, invoke: state.invoke }));
vi.mock('./conversationOrganizationLoad', () => ({ acceptOrganization: vi.fn(), refreshConversationOrganization: vi.fn(), organizationApi: { change: state.change } }));
vi.mock('./store/conversationOrganization', () => ({ conversationOrganization: () => ({ 's-busy': {}, 's-idle': {} }) }));
vi.mock('./store', () => ({ chatAgentSessionKey: () => 's-current', flashWithUndo: state.toast, flashFooter: vi.fn(), showConfirm: vi.fn() }));
vi.mock('./store/workspaces', () => ({ refreshWorkspaceSessions: state.refresh, lifecycleFailed: state.failed }));
vi.mock('./store/sessionStates', () => ({ sessionState: (key: string) => key === 's-busy' ? 'waiting' : 'idle' }));
import { checkSettlement } from './conversationSettlement';
beforeEach(() => { vi.clearAllMocks(); state.invoke.mockResolvedValue([]); });
it('protects current and attention-needed keys', async () => {
  await checkSettlement();
  expect(state.invoke).toHaveBeenCalledWith('sweep_conversation_settlement', { protected: ['s-busy', 's-current'] });
  expect(state.toast).not.toHaveBeenCalled();
});
it('coalesces overlapping ticks and Undo uses each accepted revision', async () => {
  let finish!: (value: unknown) => void;
  state.invoke.mockReturnValue(new Promise(resolve => { finish = resolve; }));
  const check = checkSettlement();
  await checkSettlement();
  expect(state.invoke).toHaveBeenCalledOnce();
  finish([{ key: 's-idle', revision: 8, session: { lastActivityAt: '2030-01-01' } }]);
  await check;
  state.change.mockResolvedValue({});
  await state.toast.mock.calls[0][0].undo();
  expect(state.change).toHaveBeenCalledWith({ key: 's-idle', action: 'wake', expectedRevision: 8, expectedActivity: '2030-01-01' });
});
it('failed reads do not announce settlement', async () => {
  state.invoke.mockRejectedValue(new Error('offline'));
  await checkSettlement();
  expect(state.toast).not.toHaveBeenCalled();
  expect(state.refresh).not.toHaveBeenCalled();
});
