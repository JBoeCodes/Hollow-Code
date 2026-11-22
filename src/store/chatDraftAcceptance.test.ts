import { beforeEach, expect, it, vi } from 'vitest';
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return { listeners: new Map<string, (e: { payload: unknown }) => void>(), acquire: vi.fn() };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (e: { payload: unknown }) => void) => { h.listeners.set(name, fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({ api: new Proxy({}, { get: () => vi.fn(async () => null) }) }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));
import * as rt from './chatRuntime';
import { bindFirstDraftSession } from './conversationDrafts';

const accepted = { sessionKey: 'first', queueId: 1, waitingFor: null, queuePosition: 1, phase: 'queued' as const };
function begin() {
  rt.beginQueuedChatTurn({ clientTurnId: 'mine', text: 'Sent', attachments: [], requestedCwd: '/alpha', sessionKey: null });
}
beforeEach(() => {
  rt.setChatBusy(false); rt.resetChatState(); rt.setChatCwdPersistent('/alpha');
  h.acquire.mockClear(); bindFirstDraftSession(h.acquire);
});
it('acquires the new-chat draft once on its own accepted send, before the key changes', () => {
  begin(); rt.setChatInput('Unsent follow-up');
  h.acquire.mockImplementation(() => expect(rt.chatAgentSessionKey()).toBeNull());
  expect(rt.acceptQueuedChatTurn('mine', accepted)).toBe(true);
  expect(h.acquire).toHaveBeenCalledTimes(1); expect(h.acquire).toHaveBeenCalledWith('/alpha', 'first');
  rt.acceptQueuedChatTurn('mine', accepted);
  expect(h.acquire).toHaveBeenCalledTimes(1); expect(rt.chatInput()).toBe('Unsent follow-up');
});
it('does not acquire on foreign, recovered, adopted or mere history keys', () => {
  begin(); expect(rt.acceptQueuedChatTurn('foreign', accepted)).toBe(false);
  rt.setChatQueuedTurn(q => ({ ...q!, adopted: true })); rt.acceptQueuedChatTurn('mine', accepted);
  expect(h.acquire).not.toHaveBeenCalled();
  rt.setChatAgentSessionKeyPersistent(null); begin();
  rt.setChatQueuedTurn(q => ({ ...q!, recovered: true })); rt.acceptQueuedChatTurn('mine', accepted);
  rt.setChatAgentSessionKeyPersistent('history', { ownTurn: true });
  expect(h.acquire).not.toHaveBeenCalled();
});
it.each(['runStarted', 'done'])('acquires when %s overtakes the accepted reply', type => {
  begin(); rt.setChatBusy(true);
  h.acquire.mockImplementation(() => {});
  h.listeners.get('chat-event')!({ payload: {
    type, clientTurnId: 'mine', sessionKey: 'first', conversationKey: 'first', queueId: 1,
    runId: 'run-first', recall: [], sessionId: null,
  } });
  expect(h.acquire).toHaveBeenCalledTimes(1); expect(h.acquire).toHaveBeenCalledWith('/alpha', 'first');
  rt.acceptQueuedChatTurn('mine', accepted);
  expect(h.acquire).toHaveBeenCalledTimes(1);
});
