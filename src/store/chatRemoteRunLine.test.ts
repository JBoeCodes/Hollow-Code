import { expect, it, vi } from 'vitest';
import { remoteRunNotice } from '../remoteRunNotice';

// Overnight 11-07, lane D3: the runner's lines (`remoteRunNotice.ts`) load on
// first use, outside the shell chunk. A `remoteRun` line keeps its place in
// the transcript and gets its text when they land, on screen or in a chat
// sent to the background meanwhile.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return { listeners: new Map<string, (e: { payload: unknown }) => void>() };
});
vi.mock('../nativeEvents', () => ({ listen: (name: string, fn: (e: { payload: unknown }) => void) => { h.listeners.set(name, fn); return Promise.resolve(() => {}); } }));
vi.mock('../api', () => ({ api: new Proxy({}, { get: () => vi.fn(async () => null) }) }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));

const KEY = 's-20261107-runline01';
const emit = (payload: Record<string, unknown>) => h.listeners.get('chat-event')!({ payload: { conversationKey: KEY, sessionKey: KEY, queueId: 7, ...payload } });
const start = { type: 'remoteRun' as const, phase: 'start' as const, host: 'runner-host' };
const back = { type: 'remoteRun' as const, phase: 'back' as const, host: 'runner-host', outcome: 'unchanged' as const };

/** A fresh chat runtime (the runner's lines not loaded yet) with a turn running on screen. */
async function runningChat() {
  vi.resetModules();
  const rt = await import('./chatRuntime');
  rt.setChatBusy(false); rt.resetChatState();
  rt.setChatAgentPersistent('claude');
  rt.setChatAgentSessionKeyPersistent(KEY);
  rt.setChatQueuedTurn({
    clientTurnId: 'turn-7', queueId: 7, sessionKey: KEY, text: 'hello', attachments: [], requestedCwd: null,
    waitingFor: null, waitingSession: null, queuePosition: null, started: true, intent: null, adopted: false, recovered: false,
  });
  rt.setChatBusy(true);
  const lines = () => rt.chatTurns.map(t => [t.notice ?? t.role, t.text]);
  return { rt, lines };
}

it('the first runner line keeps its place before what follows and gets its text when the lines load', async () => {
  const { rt, lines } = await runningChat();
  emit(start);
  rt.pushChatTurn({ role: 'system', text: 'after', tools: [] });
  await vi.dynamicImportSettled();
  expect(lines()).toEqual([['remoteRun', remoteRunNotice(start)], ['system', 'after']]);
  expect(rt.chatTurns[0].remoteRunPending).toBeUndefined();
  // Loaded now: a later line has its text at once.
  emit(back);
  expect(lines().at(-1)).toEqual(['remoteRun', remoteRunNotice(back)]);
  expect(rt.chatTurns.at(-1)?.aboutEarlierTurn).toBe(true);
});

it('a chat sent to the background before the lines load has the text when it comes back', async () => {
  const { rt, lines } = await runningChat();
  emit(start);
  expect(rt.backgroundRunningChat()).toBe(true);
  rt.resetChatState();
  await vi.dynamicImportSettled();
  expect(rt.restoreBackgroundChat(KEY)).toBe(true);
  expect(lines()).toEqual([['remoteRun', remoteRunNotice(start)]]);
});
