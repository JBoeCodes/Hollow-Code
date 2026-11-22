import { beforeEach, expect, it, vi } from 'vitest';

// 2026-11-04 (Alex's Main chat): a check-in this window adopted ran in
// Edits only while the chat was Bypass, and its `done` cleared the native
// session, so his next turn started a fresh one and the check-in's question
// card couldn't be sent. An adopted turn's session now carries on when its
// mode is in the composer's mode's class, and only then.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return { listeners: new Map<string, (e: { payload: unknown }) => void>(), readConversation: vi.fn(async (_key: string): Promise<unknown> => null) };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (e: { payload: unknown }) => void) => { h.listeners.set(name, fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({ api: new Proxy({}, { get: (_, name) => name === 'readAgentConversation' ? h.readConversation : vi.fn(async () => null) }) }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));
import * as rt from './chatRuntime';

const KEY = 's-20261104-main0001';
const emit = (payload: Record<string, unknown>) => h.listeners.get('chat-event')!({ payload: { conversationKey: KEY, sessionKey: KEY, queueId: 7, ...payload } });

/** A check-in this window adopted (found queued), then its events: with
 *  `seen` false it was adopted mid-run, after its `runStarted`. */
function checkIn(mode: string | null, opts: { seen?: boolean; recovered?: boolean } = {}) {
  rt.setChatQueuedTurn({
    clientTurnId: 'adopted-7', queueId: 7, sessionKey: KEY, text: 'check-in', attachments: [], requestedCwd: null,
    waitingFor: null, waitingSession: null, queuePosition: null, started: true, intent: null, adopted: true, recovered: false,
  });
  rt.setChatBusy(true);
  if (opts.seen !== false) emit({ type: 'runStarted', runId: 'run-checkin', recall: [], permissionMode: mode });
  emit({ type: 'session', sessionId: 'checkin-native' });
  emit({ type: 'done', runId: 'run-checkin', sessionId: 'checkin-native', ...(opts.recovered ? { recovered: true } : {}) });
}

beforeEach(() => {
  rt.setChatBusy(false); rt.resetChatState();
  rt.setChatAgentPersistent('claude');
  rt.setChatAgentSessionKeyPersistent(KEY);
  rt.setChatSessionIdPersistent('person-native');
});

it('an Edits-only check-in in a Bypass chat leaves its session for the next turn', () => {
  rt.setChatPermissionMode('bypassPermissions');
  checkIn('acceptEdits');
  expect(rt.chatBusy()).toBe(false);
  expect(rt.chatSessionId()).toBe('checkin-native');
});

it('a check-in in another class, or one whose mode is unknown, still clears it', () => {
  rt.setChatPermissionMode('plan');
  checkIn('acceptEdits');
  expect(rt.chatSessionId()).toBeNull();
  rt.setChatPermissionMode('bypassPermissions');
  rt.setChatSessionIdPersistent('person-native');
  checkIn(null);
  expect(rt.chatSessionId()).toBeNull();
});

it('a Grok check-in under another sandbox profile clears it', () => {
  rt.setChatAgentPersistent('grok');
  rt.setChatAgentSessionKeyPersistent(KEY);
  rt.setChatPermissionMode('bypassPermissions');
  checkIn('acceptEdits');
  expect(rt.chatSessionId()).toBeNull();
});

it('keeps the turn mode on the queued turn, so it goes with the conversation into the background and back', () => {
  rt.setChatPermissionMode('bypassPermissions');
  rt.setChatQueuedTurn({
    clientTurnId: 'adopted-7', queueId: 7, sessionKey: KEY, text: 'check-in', attachments: [], requestedCwd: null,
    waitingFor: null, waitingSession: null, queuePosition: null, started: true, intent: null, adopted: true, recovered: false,
  });
  rt.setChatBusy(true);
  emit({ type: 'runStarted', runId: 'run-checkin', recall: [], permissionMode: 'acceptEdits' });
  expect(rt.chatQueuedTurn()?.permissionMode).toBe('acceptEdits');
  // Away to another conversation (a Plan one) while the check-in runs...
  expect(rt.backgroundRunningChat()).toBe(true);
  rt.resetChatState();
  rt.setChatAgentSessionKeyPersistent('s-20261104-plan0001');
  rt.setChatPermissionMode('plan');
  // ...and back: the check-in's mode came back with it, not the Plan chat's.
  rt.resetChatState();
  expect(rt.restoreBackgroundChat(KEY)).toBe(true);
  expect(rt.chatQueuedTurn()?.permissionMode).toBe('acceptEdits');
  expect(rt.chatPermissionMode()).toBe('bypassPermissions');
  emit({ type: 'session', sessionId: 'checkin-native' });
  emit({ type: 'done', runId: 'run-checkin', sessionId: 'checkin-native' });
  expect(rt.chatSessionId()).toBe('checkin-native');
});

it('a check-in adopted mid-run, or one whose end recovered the chat, still clears it', () => {
  rt.setChatPermissionMode('bypassPermissions');
  checkIn('acceptEdits', { seen: false });
  expect(rt.chatSessionId()).toBeNull();
  rt.setChatSessionIdPersistent('person-native');
  checkIn('acceptEdits', { recovered: true });
  expect(rt.chatSessionId()).toBeNull();
});

it('says whether a mode change starts a fresh session, and the change does what it says', () => {
  // Settings' Revert to Edits only words its notice from this (#1174 review).
  rt.setChatPermissionMode('bypassPermissions');
  expect(rt.permissionChangeStartsFresh('acceptEdits')).toBe(false);
  expect(rt.permissionChangeStartsFresh('plan')).toBe(true);
  expect(rt.setChatPermissionModePersistent('acceptEdits')).toBe('');
  expect(rt.chatSessionId()).toBe('person-native');
  rt.setChatAgentPersistent('grok');
  rt.setChatAgentSessionKeyPersistent(KEY);
  rt.setChatSessionIdPersistent('person-native');
  rt.setChatPermissionMode('bypassPermissions');
  expect(rt.permissionChangeStartsFresh('acceptEdits')).toBe(true);
  expect(rt.permissionChangeStartsFresh('auto')).toBe(false);
  expect(rt.setChatPermissionModePersistent('acceptEdits')).toBe('');
  expect(rt.chatSessionId()).toBeNull();
});

// Overnight 11-13 c-11: the store keeps an assistant reply's text only at
// finish, so a turn re-attached after a reload showed its tools with no text
// and its next delta began a second reply. Its end re-reads the store.
it('an adopted turn shows its stored transcript once it ends, the reply as one', async () => {
  rt.setChatPermissionMode('bypassPermissions');
  rt.setChatTurns([
    { role: 'user', text: 'check-in', tools: [] },
    { role: 'assistant', text: '', tools: [{ name: 'Read', id: 't1', status: 'done' }] },
    { role: 'assistant', text: 'second half', tools: [], partial: true },
  ]);
  h.readConversation.mockResolvedValueOnce({ turns: [
    { role: 'user', text: 'check-in', tools: [] },
    { role: 'assistant', text: 'First half, second half', tools: [{ name: 'Read', id: 't1' }], status: 'success' },
  ] });
  checkIn('bypassPermissions');
  await vi.waitFor(() => expect(rt.chatTurns.length).toBe(2));
  expect(h.readConversation).toHaveBeenCalledWith(KEY);
  expect(rt.chatTurns[1].text).toBe('First half, second half');
});

// Item 1's merge: the stored transcript goes in front of what arrived live.
it('puts a stored transcript in front of the live turn without doubling a tool', () => {
  rt.setChatTurns([{ role: 'assistant', text: 'Live so far', tools: [{ name: 'Read', id: 't1', status: 'done' }, { name: 'Edit', id: 't2', status: 'running' }], partial: true }]);
  rt.setChatStoredTranscriptPending(KEY);
  rt.showStoredTranscript(KEY, [
    { role: 'user', text: 'Earlier question', tools: [] },
    { role: 'assistant', text: '', tools: [{ name: 'Read', id: 't1', status: 'done' }] },
  ]);
  expect(rt.chatTurns.map(t => [t.text, t.tools.map(tool => tool.id)])).toEqual([
    ['Earlier question', []], ['', ['t1']], ['Live so far', ['t2']],
  ]);
  expect(rt.chatStoredTranscriptPending()).toBeNull();
});

// Codex review of the merge: a question runStarted added locally is not
// shown twice, and tool ids only match within the same run.
it('does not show the running turn\'s question twice when runStarted added it before the stored read', () => {
  rt.setChatLiveRunId('run-2');
  rt.setChatTurns([
    { role: 'user', text: 'check-in', tools: [], queueId: 7 },
    { role: 'assistant', text: 'Live so far', tools: [], partial: true, runId: 'run-2' },
  ]);
  rt.setChatStoredTranscriptPending(KEY);
  rt.showStoredTranscript(KEY, [
    { role: 'user', text: 'Earlier question', tools: [], runId: 'run-1' },
    { role: 'assistant', text: 'Earlier answer', tools: [], runId: 'run-1' },
    { role: 'user', text: 'check-in', tools: [], runId: 'run-2' },
  ]);
  expect(rt.chatTurns.map(t => t.text)).toEqual(['Earlier question', 'Earlier answer', 'check-in', 'Live so far']);
  rt.setChatLiveRunId(null);
});

it('keeps a live tool that reuses an earlier run\'s id, and takes the newer state of one both hold', () => {
  rt.setChatTurns([
    { role: 'assistant', text: '', tools: [{ name: 'Read', id: 't1', status: 'running' }, { name: 'Edit', id: 't3', status: 'done', result: 'edited' }], runId: 'run-2' },
  ]);
  rt.setChatStoredTranscriptPending(KEY);
  rt.showStoredTranscript(KEY, [
    { role: 'assistant', text: 'Earlier', tools: [{ name: 'Read', id: 't1', status: 'done', result: 'old' }], runId: 'run-1' },
    { role: 'assistant', text: '', tools: [{ name: 'Edit', id: 't3', status: 'running' }], runId: 'run-2' },
  ]);
  expect(rt.chatTurns.map(t => [t.runId, t.tools.map(tool => `${tool.id}:${tool.status}`)])).toEqual([
    ['run-1', ['t1:done']], ['run-2', ['t3:done']], ['run-2', ['t1:running']],
  ]);
});

// Codex re-check: a queued turn that has not started has no stored row, so
// its bubble stays even when it repeats the previous question word for word.
it('keeps a queued, not yet started question that repeats the last stored one', () => {
  rt.setChatQueuedTurn({
    clientTurnId: 'adopted-9', queueId: 9, sessionKey: KEY, text: 'Try again', attachments: [], requestedCwd: null,
    waitingFor: null, waitingSession: null, queuePosition: 1, started: false, intent: null, adopted: true, recovered: false,
  });
  rt.setChatTurns([{ role: 'user', text: 'Try again', tools: [], clientTurnId: 'adopted-9', queueId: 9 }]);
  rt.setChatStoredTranscriptPending(KEY);
  rt.showStoredTranscript(KEY, [
    { role: 'user', text: 'Try again', tools: [], status: 'success' },
    { role: 'assistant', text: 'Done', tools: [], status: 'success' },
  ]);
  expect(rt.chatTurns.map(t => t.text)).toEqual(['Try again', 'Done', 'Try again']);
  rt.setChatQueuedTurn(null);
});
