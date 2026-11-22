import { beforeEach, expect, it, vi } from 'vitest';

// Run 15 P3 second-pass review: a composer send still getting ready (busy,
// nothing queued yet) keeps its conversation on screen. Left meanwhile, it
// was backgrounded as if a turn ran, and the send carried on into the
// conversation opened next, with that one's key and folder.
vi.hoisted(() => {
  const store = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, String(v)); },
    removeItem: (k: string) => { store.delete(k); },
  });
});
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
// One mock per method, so a test can set what it answers and see its calls.
vi.mock('../api', () => {
  const fns = new Map<PropertyKey, ReturnType<typeof vi.fn>>();
  return { api: new Proxy({}, { get: (_, k) => { if (!fns.has(k)) fns.set(k, vi.fn(async () => null)); return fns.get(k); } }) };
});
vi.mock('./runtime', () => ({
  activePage: () => 'chat', closeChatPanel: vi.fn(), requestChatFocus: vi.fn(),
  setActivePage: vi.fn(), setChatPanelOpen: vi.fn(),
}));
vi.mock('./overlays', () => ({ showConfirm: vi.fn(async () => true) }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));

const rt = await import('./chatRuntime');
const { api } = await import('../api');
const { beginPreparingSend, endPreparingSend } = await import('../chatQueue');

beforeEach(() => {
  rt.resetChatState();
  rt.setChatBusy(false);
  rt.setChatAgentSessionKeyPersistent('conv-a');
});

it('a running turn with its key may still be left (it goes to the background)', () => {
  rt.setChatBusy(true);
  expect(rt.canLeaveConversation()).toBe(true);
});

it('while a send gets ready, its conversation is neither left nor backgrounded', () => {
  rt.setChatBusy(true);
  const release = rt.holdConversationForSend();
  expect(rt.canLeaveConversation()).toBe(false);
  expect(rt.backgroundRunningChat()).toBe(false);
  expect(rt.isBackgrounded('conv-a')).toBe(false);
  expect(rt.chatBusy()).toBe(true);
  release();
  expect(rt.canLeaveConversation()).toBe(true);
});

it('each hold is released once: a second release changes nothing', () => {
  const first = rt.holdConversationForSend();
  const second = rt.holdConversationForSend();
  first();
  first();
  expect(rt.canLeaveConversation()).toBe(false);
  second();
  expect(rt.canLeaveConversation()).toBe(true);
  expect(rt.backgroundRunningChat()).toBe(true);
});

// C8a (overnight 11-13): a send's chat_send reply that comes back after the
// person left its conversation goes to that conversation's background entry
// (chatSendController.ts `sentAway` / `failedAway`), never the one on screen.
function leaveWithQueuedSend(clientTurnId: string) {
  rt.setChatBusy(true);
  rt.pushChatTurn({ role: 'user', text: 'Hello', tools: [], clientTurnId });
  rt.beginQueuedChatTurn({ clientTurnId, text: 'Hello', attachments: [], requestedCwd: null, sessionKey: 'conv-a', interactionId: null });
  expect(rt.backgroundRunningChat()).toBe(true);
  rt.resetChatState();
  rt.setChatAgentSessionKeyPersistent('conv-b');
  expect(rt.backgroundChatKeys()).toContain('conv-a');
}

it('a failed send ends its backgrounded conversation: no working mark, no bubble, not busy when back', () => {
  leaveWithQueuedSend('ct-fail');
  rt.endBackgroundSend('conv-a', 'ct-fail', 'Your queued message was not sent: no queue.');
  expect(rt.backgroundChatKeys()).not.toContain('conv-a');
  rt.resetChatState();
  expect(rt.restoreBackgroundChat('conv-a')).toBe(true);
  expect(rt.chatBusy()).toBe(false);
  expect(rt.chatQueuedTurn()).toBeNull();
  expect(rt.chatTurns.map(t => t.text)).toEqual(['Your queued message was not sent: no queue.']);
});

it('an accepted send gives its backgrounded queued turn its queue id, so Edit and Discard work back on screen', () => {
  leaveWithQueuedSend('ct-ok');
  rt.acceptBackgroundSend('conv-a', 'ct-ok', { sessionKey: 'conv-a', queueId: 7, queuePosition: 2, phase: 'queued', waitingFor: 'another turn' });
  expect(rt.backgroundChatKeys()).toContain('conv-a');
  rt.resetChatState();
  expect(rt.restoreBackgroundChat('conv-a')).toBe(true);
  expect(rt.chatBusy()).toBe(true);
  expect(rt.chatQueuedTurn()).toMatchObject({ clientTurnId: 'ct-ok', queueId: 7, queuePosition: 2, waitingFor: 'another turn' });
  expect(rt.chatTurns.find(t => t.clientTurnId === 'ct-ok')?.queueId).toBe(7);
});

// PR #1391 review: a late failure (or forwarded reply) for an earlier send
// must not end a later send's background entry.
it('a late end for another send leaves the backgrounded queued turn running', () => {
  leaveWithQueuedSend('ct-later');
  rt.endBackgroundSend('conv-a', 'ct-earlier', null);
  expect(rt.backgroundChatKeys()).toContain('conv-a');
  rt.resetChatState();
  expect(rt.restoreBackgroundChat('conv-a')).toBe(true);
  expect(rt.chatBusy()).toBe(true);
  expect(rt.chatQueuedTurn()?.clientTurnId).toBe('ct-later');
  expect(rt.chatTurns.some(t => t.clientTurnId === 'ct-later')).toBe(true);
});

// C10 (overnight 11-13): Stop while a send gets ready (busy, nothing queued)
// had nothing for chat_cancel to find, and the turn then ran.
it('Stop while a send gets ready marks that send and calls no chat_cancel; once it has queued, Stop is chat_cancel again', () => {
  rt.setChatBusy(true);
  const prepared = beginPreparingSend('ct-ready', 'conv-a');
  vi.mocked(api.chatCancel).mockClear();
  rt.stopChatRun();
  expect(prepared.stopped).toBe(true);
  expect(api.chatCancel).not.toHaveBeenCalled();
  endPreparingSend(prepared);
  rt.stopChatRun();
  expect(api.chatCancel).toHaveBeenCalledWith('conv-a');
  rt.setChatBusy(false);
});

it('a send stopped in flight is withdrawn by its row: words back in the box, bubble gone, not busy', async () => {
  rt.setChatBusy(true);
  rt.setChatInput('Typed since');
  rt.pushChatTurn({ role: 'user', text: 'Hello', tools: [], clientTurnId: 'ct-stop' });
  rt.beginQueuedChatTurn({ clientTurnId: 'ct-stop', text: 'Hello', attachments: [], requestedCwd: null, sessionKey: 'conv-a', interactionId: null });
  const accepted = { sessionKey: 'conv-a', queueId: 11, queuePosition: 1, phase: 'queued' as const, waitingFor: null };
  rt.acceptQueuedChatTurn('ct-stop', accepted);
  vi.mocked(api.cancelQueuedRun).mockResolvedValueOnce({ id: 11, status: 'cancelled' } as never);
  await expect(rt.withdrawStoppedSend('ct-stop', accepted)).resolves.toBe('withdrawn');
  expect(api.cancelQueuedRun).toHaveBeenCalledWith(11);
  expect(rt.chatQueuedTurn()).toBeNull();
  expect(rt.chatBusy()).toBe(false);
  expect(rt.chatInput()).toBe('Hello\n\nTyped since');
  expect(rt.chatTurns.map(t => t.text)).toEqual(['Stopped before it started. Your message is back in the composer.']);
});

// Codex review of 9a15569c5: cancel_queued_run already stops a running row;
// the extra key-wide chat_cancel could stop a later turn that started since.
it('a send stopped in flight that already started is stopped by its row only, never by its key', async () => {
  rt.setChatBusy(true);
  const accepted = { sessionKey: 'conv-a', queueId: 12, queuePosition: 1, phase: 'queued' as const, waitingFor: null };
  vi.mocked(api.cancelQueuedRun).mockResolvedValueOnce({ id: 12, status: 'running' } as never);
  vi.mocked(api.chatCancel).mockClear();
  await expect(rt.withdrawStoppedSend('ct-ran', accepted)).resolves.toBe('stopped');
  expect(api.cancelQueuedRun).toHaveBeenCalledWith(12);
  expect(api.chatCancel).not.toHaveBeenCalled();
  rt.setChatBusy(false);
});
