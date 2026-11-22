import { beforeEach, describe, expect, it, vi } from 'vitest';

// Working, not Queued (2026-11-16): the chat shows its working indicator
// ("Preparing") from the moment the queue has the row, and the Queued badge
// only while the backend names a real wait (a chat-count limit, memory,
// another run holding its folder).
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
import * as rt from './chatRuntime';
import { queuedBadgeText } from '../chatQueue';

const accepted = (waitingFor: string | null) => ({ sessionKey: 'first', queueId: 1, waitingFor, queuePosition: 1, phase: 'queued' as const });

beforeEach(() => {
  rt.setChatBusy(false); rt.resetChatState(); rt.setChatCwdPersistent('/alpha');
  rt.beginQueuedChatTurn({ clientTurnId: 'mine', text: 'Sent', attachments: [], requestedCwd: '/alpha', sessionKey: null });
});

describe('the queued badge and the working indicator', () => {
  it('before the queue has the row nothing is queued', () => {
    const q = rt.chatQueuedTurn();
    expect(rt.isVisiblyQueued(q)).toBe(false);
    expect(rt.isPreparingTurn(q)).toBe(false);
  });

  it('an accepted turn with no real wait is Preparing, with no Queued badge', () => {
    rt.acceptQueuedChatTurn('mine', accepted(null));
    const q = rt.chatQueuedTurn();
    expect(rt.isVisiblyQueued(q)).toBe(false);
    expect(rt.isPreparingTurn(q)).toBe(true);
  });

  it.each([
    'memory',
    'a free chat slot: Hollow is at its limit of 10 chats at once (Settings → Chat → Chats at once)',
    'a free chat slot: this workspace is at its limit of 5 chats at once (Settings → Chat → Chats at once)',
    'a free subagent slot: this chat is at its limit of 6 subagents at once (Settings → Chat → Chats at once)',
    'a free swarm slot: the swarm is at its limit of 3 parts at once (Settings → Background work → Swarm parts at once)',
  ])('a real wait shows Queued and names it: %s', reason => {
    rt.acceptQueuedChatTurn('mine', accepted(reason));
    const q = rt.chatQueuedTurn();
    expect(rt.isVisiblyQueued(q)).toBe(true);
    expect(rt.isPreparingTurn(q)).toBe(false);
    expect(queuedBadgeText(q!)).toBe(`Queued · waiting for ${reason}`);
  });

  it('the wait clears into Preparing, then Working once the run starts', () => {
    rt.acceptQueuedChatTurn('mine', accepted('memory'));
    expect(rt.isVisiblyQueued(rt.chatQueuedTurn())).toBe(true);
    rt.setChatQueuedTurn(q => ({ ...q!, waitingFor: null }));
    expect(rt.isPreparingTurn(rt.chatQueuedTurn())).toBe(true);
    h.listeners.get('chat-event')!({ payload: {
      type: 'runStarted', clientTurnId: 'mine', sessionKey: 'first', conversationKey: 'first', queueId: 1,
      runId: 'run-first', recall: [], sessionId: null,
    } });
    const q = rt.chatQueuedTurn();
    expect(rt.isVisiblyQueued(q)).toBe(false);
    expect(rt.isPreparingTurn(q)).toBe(false);
  });
});

describe('a permission-held row (2026-11-17, a #1499 leftover)', () => {
  it('reads paused in the composer like the dock, never Preparing', () => {
    rt.acceptQueuedChatTurn('mine', accepted(null));
    expect(rt.isPreparingTurn(rt.chatQueuedTurn())).toBe(true);
    // The queue refresh sees the permission mode hold the row.
    rt.reconcileQueuedChatRows([{
      id: 1, source: 'chat', sourceId: 'first', label: 'Chat', detail: 'Sent', reason: 'chat', status: 'queued',
      priority: 0, position: 1, attempt: 0, coalescedCount: 0, requestedAt: '2026-11-17T01:00:00Z', claimedAt: null,
      finishedAt: null, historyRunId: null, error: null, waitingSince: null, waitingHolder: null,
      heldReason: 'automation settings changed; review them in Automations → Permission changes',
      cancelRequested: false, waitingReason: null,
    }]);
    const q = rt.chatQueuedTurn();
    expect(rt.isPreparingTurn(q)).toBe(false);
    expect(rt.isVisiblyQueued(q)).toBe(true);
    expect(q?.heldReason).toBe('automation settings changed; review them in Automations → Permission changes');
  });
});
