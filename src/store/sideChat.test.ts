import { beforeEach, describe, expect, it, vi } from 'vitest';

const api = vi.hoisted(() => ({
  sideChatOpen: vi.fn(),
  sideChatList: vi.fn(),
  sideChatSend: vi.fn(),
  chatCancel: vi.fn(),
  readAgentConversation: vi.fn(),
}));
vi.mock('../api', () => ({ api, invoke: vi.fn() }));
vi.mock('../nativeEvents', () => ({ listen: vi.fn(() => Promise.resolve(() => {})), onNative: vi.fn(() => () => {}) }));

import {
  applySideEvent, bindSideChat, closeSideChat, newSideChat, openSideChat, resultForMain, sendSideChat, setSideChatSameAccess, sideChat,
  sideChatBlockedReason, sideChatOpen, sideChatSameAccess, toggleSideChat,
} from './sideChat';

const PARENT = 's-20261120-side0001';
const SIDE = 's-20261120-side0002';
const summary = (key: string, turnCount: number) => ({
  key, title: 'Side chat', agent: 'claude', harnessSessionId: null, cwd: '/w', createdAt: '', lastActivityAt: '', endedAt: null,
  turnCount, parentKey: PARENT, forkKind: 'side', adopted: false, stints: [], resumable: 'none',
});
const turn = (id: string, role: 'user' | 'assistant', text: string) => ({ id, role, text, tools: [], agent: 'claude', runId: 'r', startedAt: '', status: null, costUsd: null, error: null, receipt: null, transcriptIncomplete: false });

beforeEach(async () => {
  vi.clearAllMocks();
  const saved = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => saved.get(k) ?? null, setItem: (k: string, v: string) => { saved.set(k, v); }, removeItem: (k: string) => { saved.delete(k); } });
  api.sideChatOpen.mockResolvedValue(summary(SIDE, 0));
  api.sideChatList.mockResolvedValue([]);
  api.readAgentConversation.mockResolvedValue({ summary: summary(SIDE, 0), turns: [], eventsRetained: true });
  setSideChatSameAccess(false);
  await bindSideChat(null);
});

describe('opening', () => {
  it('opens the side chat of the main chat on screen and reads nothing for an empty one', async () => {
    await bindSideChat(PARENT);
    expect(api.sideChatOpen).toHaveBeenCalledWith(PARENT, false);
    expect(api.readAgentConversation).not.toHaveBeenCalled();
    expect(sideChat()).toMatchObject({ parent: PARENT, key: SIDE, turns: [], busy: false, loading: false, error: null });
  });

  it('reads the stored turns of a side chat that has some', async () => {
    api.sideChatOpen.mockResolvedValue(summary(SIDE, 2));
    api.readAgentConversation.mockResolvedValue({ summary: summary(SIDE, 2), turns: [turn('u', 'user', 'Why?'), turn('a', 'assistant', 'Because.')], eventsRetained: true });
    await bindSideChat(PARENT);
    expect(api.readAgentConversation).toHaveBeenCalledWith(SIDE);
    expect(sideChat().turns.map(t => t.text)).toEqual(['Why?', 'Because.']);
  });

  it('says why a chat with no first message has no side chat, and asks the backend for nothing', async () => {
    await bindSideChat(null);
    expect(api.sideChatOpen).not.toHaveBeenCalled();
    expect(sideChat().error).toBe(sideChatBlockedReason(null));
    expect(sideChat().key).toBeNull();
    expect(await sendSideChat('Hello?')).toBe(false);
    expect(api.sideChatSend).not.toHaveBeenCalled();
  });

  it('a backend refusal is shown in the pane', async () => {
    api.sideChatOpen.mockRejectedValue('The Main agent keeps its own page.');
    await bindSideChat(PARENT);
    expect(sideChat().error).toContain('own page');
    expect(sideChat().key).toBeNull();
  });

  it('a later bind for another chat wins over a slower earlier one', async () => {
    let release: (v: unknown) => void = () => {};
    api.sideChatOpen.mockImplementationOnce(() => new Promise(r => { release = r; }));
    const slow = bindSideChat(PARENT);
    api.sideChatOpen.mockResolvedValueOnce({ ...summary('s-20261120-side0009', 0), parentKey: 's-20261120-other001' });
    await bindSideChat('s-20261120-other001');
    release(summary(SIDE, 0));
    await slow;
    expect(sideChat().parent).toBe('s-20261120-other001');
    expect(sideChat().key).toBe('s-20261120-side0009');
  });

  it('New side chat asks the backend for a fresh one, never while a turn runs', async () => {
    await bindSideChat(PARENT);
    api.sideChatOpen.mockResolvedValue(summary('s-20261120-side0003', 0));
    await newSideChat();
    expect(api.sideChatOpen).toHaveBeenLastCalledWith(PARENT, true);
    applySideEvent({ type: 'runStarted', conversationKey: 's-20261120-side0003' });
    api.sideChatOpen.mockClear();
    await newSideChat();
    expect(api.sideChatOpen).not.toHaveBeenCalled();
  });
});

describe('isolation from the main chat', () => {
  it('only events naming its own conversation change it', async () => {
    await bindSideChat(PARENT);
    // The main chat's own events, another chat's, and untagged ones.
    applySideEvent({ type: 'assistantDelta', conversationKey: PARENT, text: 'main text' });
    applySideEvent({ type: 'runStarted', conversationKey: 's-20261120-other001' });
    applySideEvent({ type: 'assistantDelta', text: 'untagged' });
    expect(sideChat().live).toBeNull();
    expect(sideChat().busy).toBe(false);
    applySideEvent({ type: 'assistantDelta', conversationKey: SIDE, text: 'side ' });
    applySideEvent({ type: 'assistantDelta', conversationKey: SIDE, text: 'text' });
    expect(sideChat().live?.text).toBe('side text');
    expect(sideChat().busy).toBe(true);
  });

  it('streamed pieces win over whole blocks, so a reply is never shown twice', async () => {
    await bindSideChat(PARENT);
    applySideEvent({ type: 'assistantDelta', conversationKey: SIDE, text: 'One.' });
    applySideEvent({ type: 'assistantText', conversationKey: SIDE, text: 'One.' });
    expect(sideChat().live?.text).toBe('One.');
  });

  it('shows tools used and a question card pointer, and settles by re-reading its own turns', async () => {
    await bindSideChat(PARENT);
    applySideEvent({ type: 'runStarted', conversationKey: SIDE });
    applySideEvent({ type: 'toolUse', conversationKey: SIDE, name: 'Read' });
    expect(sideChat().live?.tools).toEqual(['Read']);
    applySideEvent({ type: 'interaction', conversationKey: SIDE });
    expect(sideChat().asked).toBe(true);
    api.readAgentConversation.mockResolvedValue({ summary: summary(SIDE, 2), turns: [turn('u', 'user', 'Q'), turn('a', 'assistant', 'A')], eventsRetained: true });
    applySideEvent({ type: 'done', conversationKey: SIDE });
    await vi.waitFor(() => expect(sideChat().turns).toHaveLength(2));
    expect(sideChat().busy).toBe(false);
    expect(api.readAgentConversation).toHaveBeenCalledWith(SIDE);
    expect(sideChat().turns.map(t => t.text)).toEqual(['Q', 'A']);
    expect(sideChat().live).toBeNull();
  });

  it('an event for the side chat it left (another main chat opened) is ignored', async () => {
    await bindSideChat(PARENT);
    api.sideChatOpen.mockResolvedValue({ ...summary('s-20261120-side0009', 0), parentKey: 's-20261120-other001' });
    await bindSideChat('s-20261120-other001');
    applySideEvent({ type: 'assistantDelta', conversationKey: SIDE, text: 'late' });
    expect(sideChat().live).toBeNull();
  });
});

describe('sending', () => {
  it('sends through the side chat command with its own key, never the main chat send, and read only by default', async () => {
    await bindSideChat(PARENT);
    api.sideChatSend.mockResolvedValue({ sessionKey: SIDE, queueId: 1, queuePosition: 1, phase: 'running' });
    expect(await sendSideChat('  Why Fly?  ')).toBe(true);
    expect(api.sideChatSend).toHaveBeenCalledWith(SIDE, 'Why Fly?', expect.any(String), false);
    expect(sideChat()).toMatchObject({ pending: 'Why Fly?', busy: true });
  });

  it('passes the same-access choice, which is off until the person turns it on', async () => {
    expect(sideChatSameAccess()).toBe(false);
    setSideChatSameAccess(true);
    await bindSideChat(PARENT);
    api.sideChatSend.mockResolvedValue({ sessionKey: SIDE, queueId: 1, queuePosition: 1, phase: 'running' });
    await sendSideChat('Edit it');
    expect(api.sideChatSend).toHaveBeenCalledWith(SIDE, 'Edit it', expect.any(String), true);
  });

  it('a refused send keeps the words (false) and shows why', async () => {
    await bindSideChat(PARENT);
    api.sideChatSend.mockRejectedValue('Send a first message in this chat first.');
    expect(await sendSideChat('Hi')).toBe(false);
    expect(sideChat()).toMatchObject({ pending: null, busy: false, live: null });
    expect(sideChat().error).toContain('first message');
  });

  it('one question at a time', async () => {
    await bindSideChat(PARENT);
    api.sideChatSend.mockResolvedValue({ sessionKey: SIDE, queueId: 1, queuePosition: 1, phase: 'running' });
    expect(await sendSideChat('One')).toBe(true);
    expect(await sendSideChat('Two')).toBe(false);
    expect(api.sideChatSend).toHaveBeenCalledTimes(1);
  });
});

describe('the pane', () => {
  it('opens, closes and remembers', () => {
    closeSideChat();
    expect(sideChatOpen()).toBe(false);
    openSideChat();
    expect(sideChatOpen()).toBe(true);
    expect(localStorage.getItem('hollow.sideChatOpen')).toBe('true');
    toggleSideChat();
    expect(sideChatOpen()).toBe(false);
    expect(localStorage.getItem('hollow.sideChatOpen')).toBe('false');
  });

  it('puts a reply in the main chat words with its source named, trimmed', () => {
    expect(resultForMain('  Use Fly.  ')).toBe('From the side chat:\n\nUse Fly.');
  });
});
