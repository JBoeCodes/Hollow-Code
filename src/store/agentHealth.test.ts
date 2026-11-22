import { beforeEach, expect, it, vi } from 'vitest';
import type { AgentCliStatus } from '../generated/ipc';
import { registerNavigationGuard } from './navigationGuard';
const calls = vi.hoisted(() => ({ page: vi.fn(), section: vi.fn(), support: vi.fn(), notice: vi.fn() }));
vi.mock('./workspaces', () => ({ goToPage: calls.page }));
vi.mock('./runtime', () => ({ requestOpenSettingsSection: calls.section }));
vi.mock('./chatRuntime', () => ({ ensureChatSteerSupport: calls.support }));
vi.mock('./snackbars', () => ({ flashWithUndo: calls.notice }));
import { openAgentUpdates, syncAgentHealthSupport, unavailableAgentNotice } from './agentHealth';

beforeEach(() => vi.clearAllMocks());

it('does not change the page or pending Settings target when the editor refuses navigation', async () => {
  let answer!: (allow: boolean) => void;
  const release = registerNavigationGuard(() => new Promise<boolean>(resolve => { answer = resolve; }));
  try {
    openAgentUpdates();
    expect(calls.page).not.toHaveBeenCalled();
    answer(false); await Promise.resolve();
    expect(calls.page).not.toHaveBeenCalled();
    expect(calls.section).not.toHaveBeenCalled();
  } finally { release(); }
});

it('the unavailable notice exposes the same guarded visible recovery route', () => {
  unavailableAgentNotice('Codex is updating.');
  const notice = calls.notice.mock.calls[0][0];
  expect(notice.message).toBe('Codex is updating.');
  expect(notice.actions[0].label).toBe('Agent updates');
  notice.actions[0].run();
  expect(calls.page).toHaveBeenCalledWith('settings');
  expect(calls.section).toHaveBeenCalledWith('defaults', 'Agent updates');
});

it('a changed installed version refreshes support, while identical polls can only retry a failed read', () => {
  const row = { agent: 'codex', installed: true, version: '0.162.1', state: 'idle' } as AgentCliStatus;
  syncAgentHealthSupport([row]);
  syncAgentHealthSupport([row]);
  syncAgentHealthSupport([{ ...row, version: '0.163.0' }]);
  expect(calls.support.mock.calls).toEqual([[true], [false], [true]]);
});
