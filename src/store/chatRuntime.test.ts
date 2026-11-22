import { beforeEach, describe, expect, it, vi } from 'vitest';
import chatRuntimeSource from './chatRuntime.ts?raw';

type Listener<T> = (event: { payload: T }) => void;
const listeners = vi.hoisted(() => new Map<string, Listener<unknown>>());

vi.mock('../nativeEvents', () => ({
  listen: vi.fn((event: string, cb: Listener<unknown>) => {
    listeners.set(event, cb);
    return Promise.resolve(() => { listeners.delete(event); });
  }),
}));

vi.mock('../api', () => ({
  api: {
    listPendingInteractions: vi.fn(async () => []),
    phoneAccessChatModes: vi.fn(async () => {}),
    phoneAccessNewChatMode: vi.fn(async () => {}),
    chatAckHeldFailed: vi.fn(async () => false),
    agentSessionLive: vi.fn(async () => null),
    chatEffortsList: vi.fn(async () => []),
    chatEffortsImport: vi.fn(async () => {}),
    chatEffortSet: vi.fn(async () => {}),
    conversationChain: vi.fn(async () => null),
    setupStatus: vi.fn(async () => ({ vaultPath: '/vault' })),
    readAgentConversation: vi.fn(async () => null),
  },
}));

vi.mock('./runtime', () => ({
  requestChatFocus: vi.fn(),
}));

vi.mock('./snackbars', () => ({
  flashFooter: vi.fn(),
}));

vi.mock('./overlays', () => ({
  showConfirm: vi.fn(async () => true),
}));

vi.mock('./preview', () => ({
  closePreview: vi.fn(),
  dropConversationPreviews: vi.fn(),
  dropSubagentPreviews: vi.fn(),
  subagentTabParents: vi.fn(() => []),
  newPreviewIntent: vi.fn(() => 1),
  openPreview: vi.fn(),
  previewNonce: vi.fn(() => 0),
  previewTarget: vi.fn(() => null),
  setPreviewNonce: vi.fn(),
}));

vi.mock('./conversationDrafts', () => ({
  acceptFirstDraftSession: vi.fn(),
  rememberBeforeDraftReset: vi.fn(),
}));

import {
  BACKGROUND_DONE_CAP,
  backgroundRunningChat,
  chatAgentSessionKey,
  chatBusy,
  chatInput,
  chatPanel,
  chatTurns,
  discardBackgroundChat,
  flushStashedForCurrentChat,
  isBackgrounded,
  resetChatState,
  restoreBackgroundChat,
  returnToConversation,
  setChatAgentSessionKeyPersistent,
  setChatBusy,
  setChatInput,
} from './chatRuntime';

function emitChatEvent(payload: Record<string, unknown>) {
  const cb = listeners.get('chat-event');
  if (!cb) throw new Error('chat-event listener not registered');
  cb({ payload });
}

describe('chatRuntime fixes', () => {
  beforeEach(() => {
    setChatBusy(false);
    resetChatState();
  });

  it('does not leak a stashed returned message into another conversation if the user switches before the microtask runs', async () => {
    setChatAgentSessionKeyPersistent('s-other');
    setChatInput('');

    // Return a message for s-target while s-other is on screen: stashed under s-target.
    returnToConversation('s-target', 'unsent message for target', [], 'Returned');
    expect(chatInput()).toBe('');

    // Briefly switch to s-target (triggers microtask) and immediately switch away to s-other before microtask runs.
    setChatAgentSessionKeyPersistent('s-target');
    flushStashedForCurrentChat();
    setChatAgentSessionKeyPersistent('s-other');
    await Promise.resolve();
    await Promise.resolve();

    // s-other must NOT receive s-target's message.
    expect(chatAgentSessionKey()).toBe('s-other');
    expect(chatInput()).toBe('');

    // Reopening s-target and staying on it delivers its stashed message.
    setChatAgentSessionKeyPersistent('s-target');
    flushStashedForCurrentChat();
    await Promise.resolve();
    await Promise.resolve();
    expect(chatAgentSessionKey()).toBe('s-target');
    expect(chatInput()).toBe('unsent message for target');
    expect(chatRuntimeSource).toContain('queueMicrotask(() => returnToConversation(key, back.text, back.atts, null))');
    expect(chatRuntimeSource).toContain('createEffect(flushStashedForCurrentChat)');
  });

  it('snapshots, resets, and restores chatPanels across background switches', () => {
    setChatAgentSessionKeyPersistent('s-panel-a');
    setChatBusy(true);
    emitChatEvent({
      type: 'panel',
      conversationKey: 's-panel-a',
      sessionKey: 's-panel-a',
      panel: 'custom:a',
      snapshot: { stage: 'analyzing' },
    });
    expect(chatPanel('custom:a')).toEqual({ stage: 'analyzing' });

    expect(backgroundRunningChat()).toBe(true);
    resetChatState();
    expect(chatPanel('custom:a')).toBeUndefined();

    // Switch to another conversation with its own panel.
    setChatAgentSessionKeyPersistent('s-panel-b');
    setChatBusy(true);
    emitChatEvent({
      type: 'panel',
      conversationKey: 's-panel-b',
      sessionKey: 's-panel-b',
      panel: 'custom:b',
      snapshot: { stage: 'building' },
    });
    expect(chatPanel('custom:b')).toEqual({ stage: 'building' });

    // Background s-panel-b and restore s-panel-a: panel A returns and panel B is cleared.
    expect(backgroundRunningChat()).toBe(true);
    // Also send a panel update to s-panel-a while it is backgrounded.
    emitChatEvent({
      type: 'panel',
      conversationKey: 's-panel-a',
      sessionKey: 's-panel-a',
      panel: 'custom:a-bg',
      snapshot: { step: 2 },
    });

    expect(restoreBackgroundChat('s-panel-a')).toBe(true);
    expect(chatPanel('custom:a')).toEqual({ stage: 'analyzing' });
    expect(chatPanel('custom:a-bg')).toEqual({ step: 2 });
    expect(chatPanel('custom:b')).toBeUndefined();

    // Clean up backgrounded s-panel-b.
    discardBackgroundChat('s-panel-b');
    setChatBusy(false);
  });

  it('batches buffered event replay during restoreBackgroundChat', () => {
    setChatAgentSessionKeyPersistent('s-replay');
    setChatBusy(true);
    expect(backgroundRunningChat()).toBe(true);
    resetChatState();

    // Buffer several distinct events while backgrounded.
    emitChatEvent({ type: 'assistantText', conversationKey: 's-replay', text: 'Part 1' });
    emitChatEvent({ type: 'toolUse', conversationKey: 's-replay', id: 't1', name: 'Read', input: { file_path: '/a.md' } });
    emitChatEvent({ type: 'toolResult', conversationKey: 's-replay', id: 't1', content: 'ok', isError: false });
    emitChatEvent({ type: 'turnBoundary', conversationKey: 's-replay' });
    emitChatEvent({ type: 'assistantText', conversationKey: 's-replay', text: 'Part 2' });
    emitChatEvent({ type: 'done', conversationKey: 's-replay' });

    expect(restoreBackgroundChat('s-replay')).toBe(true);
    expect(chatBusy()).toBe(false);
    expect(chatTurns.map(t => t.text)).toEqual(['Part 1', 'Part 2']);
    expect(chatTurns[0].tools).toHaveLength(1);
    expect(chatTurns[0].tools[0]).toMatchObject({ id: 't1', name: 'Read', status: 'done', result: 'ok' });
    expect(chatRuntimeSource).toMatch(/batch\(\(\) => \{\s*for \(const event of chat\.events\) handleChatEvent\(event\);\s*\}\);/);
  });

  it('evicts oldest finished backgroundChats entries LRU while keeping running entries', () => {
    const created: string[] = [];
    const bg = (key: string) => {
      created.push(key);
      setChatAgentSessionKeyPersistent(key);
      setChatBusy(true);
      expect(backgroundRunningChat()).toBe(true);
      resetChatState();
    };

    try {
      // One running chat that never finishes must never be evicted.
      bg('s-still-running');

      const totalDone = BACKGROUND_DONE_CAP + 3;
      for (let i = 0; i < totalDone; i += 1) {
        const key = `s-done-${i}`;
        bg(key);
        emitChatEvent({ type: 'done', conversationKey: key });
      }

      expect(isBackgrounded('s-still-running')).toBe(true);
      // Oldest 3 finished entries were evicted.
      expect(isBackgrounded('s-done-0')).toBe(false);
      expect(isBackgrounded('s-done-1')).toBe(false);
      expect(isBackgrounded('s-done-2')).toBe(false);
      // Most recent BACKGROUND_DONE_CAP finished entries remain.
      for (let i = 3; i < totalDone; i += 1) {
        expect(isBackgrounded(`s-done-${i}`), `s-done-${i}`).toBe(true);
      }
    } finally {
      for (const key of created) discardBackgroundChat(key);
    }
  });
});
