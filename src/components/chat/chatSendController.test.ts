import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChatInteraction, InteractionAnswer } from '../../types';
// The browser build: the server build vitest resolves runs no effects.
// @ts-expect-error: the browser build has no type declarations of its own.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// Run 15 P3 second-pass review: every composer send holds its conversation on
// screen (`holdConversationForSend`) from its first await (the agent check,
// the working-folder preflight, a card's answers) until its turn is queued,
// and gives the hold back on every way out. Left during those awaits, the
// send read the next conversation's key and folder and landed there (Ask
// "Across all my work" sends this way on the Main agent page).
const state = vi.hoisted(() => {
  function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (err: unknown) => void;
    const promise = new Promise<T>((ok, fail) => { resolve = ok; reject = fail; });
    return { promise, resolve, reject };
  }
  return {
    deferred,
    held: 0,
    log: [] as string[],
    descriptors: [{ id: 'claude' }] as unknown[],
    turns: [] as Record<string, unknown>[],
    preflight: null as null | { resolve: (stale: string | null) => void; reject: (err: unknown) => void },
    discovery: null as null | { resolve: () => void; reject: (err: unknown) => void },
    // C8a: the conversation on screen, and a chat_send the test answers.
    key: 'conv-a' as string | null,
    reply: null as null | { resolve: (accepted: unknown) => void; reject: (err: unknown) => void },
    // C10: the queued turn with no row yet shown on screen, and what Stop
    // did to a stopped send's accepted row (`withdrawStoppedSend`).
    queuedOnScreen: null as string | null,
    outcome: 'withdrawn' as string,
  };
});

vi.mock('../../api', () => ({
  api: {
    chatPreflight: vi.fn(() => {
      const d = state.deferred<string | null>();
      state.preflight = d;
      return d.promise;
    }),
    chatSend: vi.fn(async (...args: unknown[]) => {
      state.log.push(`sent:${String(args[6])}:held=${state.held}`);
      if (!state.reply) return null;
      const d = state.deferred<unknown>();
      state.reply = d;
      return d.promise;
    }),
    reopenInteraction: vi.fn(async () => ({ status: 'pending', sessionKey: 'conv-a' })),
    resolveInteraction: vi.fn(async () => {
      state.log.push(`resolve:held=${state.held}`);
      return { prompt: 'My answers' };
    }),
  },
}));
vi.mock('../../store', async () => {
  const { createSignal } = await import('solid-js');
  const [busy, setBusy] = createSignal(false);
  const [input, setInput] = createSignal('');
  const [attachments, setAttachments] = createSignal<unknown[]>([]);
  const [pending, setPending] = createSignal(null);
  // The store's Stop (chatRuntime.ts `stopChatRun`): a send getting ready
  // takes it; otherwise it goes to chat_cancel.
  const { stopPreparingSend } = await import('../../chatQueue');
  return {
    chatTurns: state.turns,
    chatSessionId: () => 'native-a', chatAgentSessionKey: () => state.key, chatContinuedFrom: () => [],
    chatCwd: () => '/vault/workspaces/a', chatBusy: busy, setChatBusy: setBusy,
    chatInput: input, setChatInput: setInput, chatModel: () => 'model', chatAgent: () => 'claude',
    AGENT_LABELS: { claude: 'Claude', codex: 'Codex' }, agentSettingsError: () => '',
    agentDescriptors: () => state.descriptors,
    refreshAgentSettings: vi.fn(() => {
      const d = state.deferred<void>();
      state.discovery = d;
      return d.promise.then(() => { state.descriptors = [{ id: 'claude' }]; });
    }),
    chatAttachments: attachments, setChatAttachments: setAttachments, chatSessionLoading: () => false,
    chatPermissionMode: () => 'default', chatUltracode: () => false, chatEffort: () => null, chatCanvases: () => 'auto',
    pushChatTurn: vi.fn(), recoverChatContext: vi.fn(),
    acceptBackgroundSend: vi.fn(), endBackgroundSend: vi.fn(), returnUnsentAnswersTo: vi.fn(),
    chatQueuedTurn: () => null, isVisiblyQueued: () => false, isPreparingTurn: () => false, turnIsQueuedTurn: () => false,
    stopChatRun: vi.fn(() => {
      if (!busy()) return;
      if (stopPreparingSend(state.key, state.queuedOnScreen)) return;
      state.log.push(`chat_cancel:${String(state.key)}`);
    }),
    withdrawStoppedSend: vi.fn(async () => { state.log.push('withdrawn'); return state.outcome; }),
    beginQueuedChatTurn: vi.fn((turn: { sessionKey: string | null }) => { state.log.push(`queued:${turn.sessionKey}:held=${state.held}`); }),
    acceptQueuedChatTurn: vi.fn(), abandonQueuedChatTurn: vi.fn(), removeChatUserTurn: vi.fn(),
    setChatHistoryCursor: vi.fn(), pushChatHistory: vi.fn(),
    pendingChatSend: pending, setPendingChatSend: setPending, flashFooter: vi.fn(),
    pageContext: () => null, chatContextEnabled: () => false, agentsAvailable: () => ['claude'],
    refreshChatInteractions: vi.fn(), removeChatInteraction: vi.fn(), returnUnsentAnswers: vi.fn(),
    chatActivity: () => null,
    ensureChatSteerSupport: vi.fn(), steersNatively: () => false, steeringAgent: (picked: string) => picked, chatSteerSupport: () => null, chatTurnSteerSupport: () => null,
    addHeldChip: vi.fn(), applyOwnSteer: vi.fn(), adoptBusySend: vi.fn(), takeEarlyHeld: vi.fn(),
    registerBusySend: vi.fn(), forgetBusySend: vi.fn(), returnToConversation: vi.fn(),
    holdConversationForSend: () => {
      state.held += 1;
      let held = true;
      return () => { if (held) { held = false; state.held -= 1; } };
    },
  };
});
vi.mock('../../store/agentHealth', () => ({ unavailableAgentNotice: vi.fn() }));
vi.mock('../../store/dictation', () => ({ dictationSent: vi.fn() }));
vi.mock('./slashCommands', () => ({ busyCommandRefusal: () => '', busyLocalCommand: () => null }));

const { createRoot } = await import('solid-js');
const { createChatSend } = await import('./chatSendController');
const store = await import('../../store');
const { setChatBusy } = store;

function controller() {
  return createRoot(() => createChatSend({ workspace: true }, {
    textarea: () => undefined, followSend: () => {}, handleLocalCommand: () => false,
    suggesting: () => false, suggestion: () => null, placeSuggestion: () => {},
  }));
}
const settle = () => new Promise(resolve => setTimeout(resolve, 0));

beforeEach(() => {
  state.held = 0;
  state.log.length = 0;
  state.turns.length = 0;
  state.descriptors = [{ id: 'claude' }];
  state.preflight = null;
  state.discovery = null;
  state.key = 'conv-a';
  state.reply = null;
  state.queuedOnScreen = null;
  state.outcome = 'withdrawn';
  setChatBusy(false);
  store.setChatInput('');
  vi.clearAllMocks();
});

describe('a send holds its conversation until the turn is queued', () => {
  it('held through the preflight, released just before the turn is queued in the same conversation', async () => {
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    expect(state.held).toBe(1);
    expect(state.log).toEqual([]);
    state.preflight!.resolve(null);
    await expect(sent).resolves.toBe(true);
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
    expect(state.held).toBe(0);
  });

  it('a second Enter during the preflight sends nothing more (ENG-73)', async () => {
    store.setChatInput('Hello');
    const chat = controller();
    const first = chat.send();
    await settle();
    expect(store.chatBusy()).toBe(true);
    await expect(chat.send()).resolves.toBe(false);
    state.preflight!.resolve(null);
    await expect(first).resolves.toBe(true);
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
  });

  it('a send that stops at the preflight (folder gone, or the check failed) gives the hold back and queues nothing', async () => {
    const chat = controller();
    const reset = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve('/old/folder');
    await expect(reset).resolves.toBe(false);
    expect(state.held).toBe(0);
    const failed = chat.send(undefined, 'Hello');
    await settle();
    expect(state.held).toBe(1);
    state.preflight!.reject(new Error('disk unavailable'));
    await expect(failed).resolves.toBe(false);
    expect(state.held).toBe(0);
    expect(state.log).toEqual([]);
  });

  it('the first send after launch is held while the agents are found, and released when that fails', async () => {
    state.descriptors = [];
    const chat = controller();
    const failing = chat.send(undefined, 'Hello');
    await settle();
    expect(state.held).toBe(1);
    state.discovery!.reject(new Error('no agents'));
    await expect(failing).rejects.toThrow('no agents');
    expect(state.held).toBe(0);
    state.descriptors = [];
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.discovery!.resolve();
    await settle();
    expect(state.held).toBe(1);
    state.preflight!.resolve(null);
    await expect(sent).resolves.toBe(true);
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
  });

  it('a card’s answers are held through the preflight and the resolve, then queued in the same conversation', async () => {
    const chat = controller();
    const card = { id: 'card-1', nativeSessionId: 'native-a' } as unknown as ChatInteraction;
    const submitted = chat.submitInteraction(card, { kind: 'choice', value: 'Yes' } as unknown as InteractionAnswer);
    await settle();
    expect(state.held).toBe(1);
    state.preflight!.resolve(null);
    await submitted;
    expect(state.log).toEqual(['resolve:held=1', 'queued:conv-a:held=0', 'sent:conv-a:held=0']);
    expect(state.held).toBe(0);
  });

  it('Continue is held through its preflight, then queued in the same conversation', async () => {
    state.turns.push({ role: 'assistant', text: 'Stopped', tools: [], status: 'cancelled' });
    const chat = controller();
    expect(chat.continueOffered()).toBe(true);
    const continued = chat.continueRun();
    await settle();
    expect(state.held).toBe(1);
    state.preflight!.resolve(null);
    await continued;
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
    expect(state.held).toBe(0);
  });
});

// C8a (overnight 11-13): `chat_send` is in flight after the hold is released,
// so the person can open chat B meanwhile (A goes to the background, busy,
// its turn queued with no id). The reply or error is A's: before, it landed
// on screen, putting A's words in B's box, A's notice in B's transcript and
// clearing B's busy, while A kept its working mark for good.
describe('a chat_send answered after its conversation was left', () => {
  /** Send "Hello" from conv-a, then open conv-b (a running background chat
   *  brought back: busy) while chat_send is pending. */
  async function sendThenLeave() {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
    state.key = 'conv-b';
    setChatBusy(true);
    vi.mocked(store.pushChatTurn).mockClear();
    return { sent };
  }
  const clientTurnId = () => vi.mocked(store.beginQueuedChatTurn).mock.calls[0][0].clientTurnId;

  it('a failure ends the turn in conversation A and gives A its words; B is untouched', async () => {
    const { sent } = await sendThenLeave();
    state.reply!.reject(new Error('A turn is already waiting.'));
    await expect(sent).resolves.toBe(false);
    expect(store.chatBusy()).toBe(true);
    expect(store.chatInput()).toBe('');
    expect(store.pushChatTurn).not.toHaveBeenCalled();
    expect(store.removeChatUserTurn).not.toHaveBeenCalled();
    expect(store.endBackgroundSend).toHaveBeenCalledWith('conv-a', clientTurnId(), 'Your queued message was not sent: A turn is already waiting. It is back in the composer.');
    expect(store.returnToConversation).toHaveBeenCalledWith('conv-a', 'Hello', [], "Your message wasn't sent: A turn is already waiting.");
  });

  it('a reply is applied to conversation A in the background, not to the one on screen', async () => {
    const { sent } = await sendThenLeave();
    const accepted = { sessionKey: 'conv-a', queueId: 7, queuePosition: 1, phase: 'queued', waitingFor: null };
    state.reply!.resolve(accepted);
    await expect(sent).resolves.toBe(true);
    expect(store.acceptQueuedChatTurn).not.toHaveBeenCalled();
    expect(store.acceptBackgroundSend).toHaveBeenCalledWith('conv-a', clientTurnId(), accepted);
    expect(store.chatBusy()).toBe(true);
  });

  it('a forwarded reply ends A\'s turn without clearing B\'s busy or opening the successor', async () => {
    const { sent } = await sendThenLeave();
    state.reply!.resolve({ sessionKey: 'conv-next', queueId: 8, queuePosition: 1, phase: 'queued', waitingFor: null, forwardedFrom: 'conv-a', forwardedTitle: 'Next' });
    await expect(sent).resolves.toBe(true);
    expect(store.chatBusy()).toBe(true);
    expect(store.removeChatUserTurn).not.toHaveBeenCalled();
    expect(store.endBackgroundSend).toHaveBeenCalledWith('conv-a', clientTurnId(), null);
    expect(store.flashFooter).toHaveBeenCalledWith('A message from another chat went to “Next”, where that conversation continues.', 5000);
  });

  it('a card\'s answers that fail after leaving go back to A\'s card, never B\'s box', async () => {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const card = { id: 'card-1', nativeSessionId: 'native-a' } as unknown as ChatInteraction;
    const submitted = chat.submitInteraction(card, { kind: 'choice', value: 'Yes' } as unknown as InteractionAnswer);
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.key = 'conv-b';
    setChatBusy(true);
    vi.mocked(store.pushChatTurn).mockClear();
    state.reply!.reject(new Error('No queue'));
    await submitted;
    expect(store.returnUnsentAnswers).not.toHaveBeenCalled();
    expect(store.returnUnsentAnswersTo).toHaveBeenCalledWith('conv-a', 'card-1', 'My answers', 'Your answers were not sent: No queue.');
    expect(store.pushChatTurn).not.toHaveBeenCalled();
    expect(store.chatBusy()).toBe(true);
  });

  // PR #1391 review: left, reopened and a new draft typed before the
  // rejection. The words came back only into an empty box, so they vanished.
  it('a failure after leaving and coming back keeps the words beside the new draft', async () => {
    const { sent } = await sendThenLeave();
    state.key = 'conv-a';
    store.setChatInput('A new draft');
    state.reply!.reject(new Error('A turn is already waiting.'));
    await expect(sent).resolves.toBe(false);
    expect(store.chatInput()).toBe('Hello\n\nA new draft');
    expect(store.endBackgroundSend).not.toHaveBeenCalled();
  });

  // PR #1391 review: a forwarded send that failed held came back without the
  // images, files or shared pages taken from the composer.
  it('a forwarded send that failed held, after leaving, gives its attachments back with the words', async () => {
    state.reply = state.deferred<unknown>();
    const image = { id: 1, name: 'a.png', mime: 'image/png', dataUrl: '', saving: false, path: '/tmp/a.png' };
    store.setChatInput('Hello');
    store.setChatAttachments([image]);
    const chat = controller();
    const sent = chat.send();
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.key = 'conv-b';
    vi.mocked(store.takeEarlyHeld).mockReturnValueOnce({ type: 'heldFailed', heldId: 'held-f', error: 'Gone.', prompt: 'Hello', sessionKey: 'conv-next' });
    state.reply!.resolve({ sessionKey: 'conv-next', queueId: 0, queuePosition: 0, phase: 'queued', waitingFor: null, heldId: 'held-f', forwardedFrom: 'conv-a' });
    await expect(sent).resolves.toBe(false);
    expect(store.returnToConversation).toHaveBeenCalledWith('conv-a', 'Hello', [image], "Your message wasn't sent: Gone.");
    store.setChatAttachments([]);
  });

  // PR #1391 re-check: the same failure with the chat on screen again (left
  // and reopened) gave back only the words.
  it('a forwarded send that failed held, back on screen, gives its attachments back with the words', async () => {
    state.reply = state.deferred<unknown>();
    const image = { id: 2, name: 'b.png', mime: 'image/png', dataUrl: '', saving: false, path: '/tmp/b.png' };
    store.setChatInput('Hello');
    store.setChatAttachments([image]);
    const chat = controller();
    const sent = chat.send();
    await settle();
    state.preflight!.resolve(null);
    await settle();
    vi.mocked(store.takeEarlyHeld).mockReturnValueOnce({ type: 'heldFailed', heldId: 'held-g', error: 'Gone.', prompt: 'Hello', sessionKey: 'conv-next' });
    state.reply!.resolve({ sessionKey: 'conv-next', queueId: 0, queuePosition: 0, phase: 'queued', waitingFor: null, heldId: 'held-g', forwardedFrom: 'conv-a' });
    await expect(sent).resolves.toBe(false);
    expect(store.chatInput()).toBe('Hello');
    expect(store.chatAttachments()).toEqual([image]);
    store.setChatAttachments([]);
  });

  // PR #1391 re-check: a canvas's answers (box false) that fail held after a
  // forward stay in the canvas; they never fill the composer.
  it('a canvas\'s answers that fail held after a forward stay out of the box', async () => {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.sendAnswers('My canvas answers', () => false, () => {});
    await settle();
    state.preflight!.resolve(null);
    await settle();
    vi.mocked(store.pushChatTurn).mockClear();
    vi.mocked(store.takeEarlyHeld).mockReturnValueOnce({ type: 'heldFailed', heldId: 'held-h', error: 'Gone.', prompt: 'My canvas answers', sessionKey: 'conv-next' });
    state.reply!.resolve({ sessionKey: 'conv-next', queueId: 0, queuePosition: 0, phase: 'queued', waitingFor: null, heldId: 'held-h', forwardedFrom: 'conv-a' });
    await expect(sent).resolves.toBe(false);
    expect(store.chatInput()).toBe('');
    expect(store.pushChatTurn).toHaveBeenCalledWith({ role: 'system', text: "Your answers weren't sent: Gone. They're still in the canvas.", tools: [] });
  });

  it('still on screen: a failure comes back to the box as before', async () => {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.reply!.reject(new Error('A turn is already waiting.'));
    await expect(sent).resolves.toBe(false);
    expect(store.chatInput()).toBe('Hello');
    expect(store.chatBusy()).toBe(false);
    expect(store.endBackgroundSend).not.toHaveBeenCalled();
    expect(store.removeChatUserTurn).toHaveBeenCalled();
  });
});

// Q-C12 (R8b): "Send as next" of a refused steer, while the box holds a
// draft. The send reports false (so the steer row brings its actions back
// and keeps its text) and the steer's words go back to their conversation;
// the draft is not touched.
describe('Send as next of a refused steer', () => {
  it('reports nothing sent, hands the words back to their chat and leaves the draft alone', async () => {
    setChatBusy(true);
    store.setChatInput('My draft');
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.sendAsNext('The refused steer');
    await settle();
    state.reply!.reject(new Error('A turn is already waiting.'));
    await expect(sent).resolves.toBe(false);
    expect(store.returnToConversation).toHaveBeenCalledWith('conv-a', 'The refused steer', [], expect.stringContaining('A turn is already waiting'));
    expect(store.chatInput()).toBe('My draft');
  });
});

// C10 (overnight 11-13): Stop shows while a send gets ready (busy through the
// agent check, the preflight and chat_send), but chat_cancel had nothing to
// find (a fresh chat has no key; an existing one answered "nothing to stop")
// and the turn then ran. Stop now belongs to that one send.
describe('Stop while a send is getting ready', () => {
  const stoppedNotice = { role: 'system', text: 'Stopped before it was sent.', tools: [] };
  const clientTurnId = () => vi.mocked(store.beginQueuedChatTurn).mock.calls[0][0].clientTurnId;

  it('during the preflight: nothing is queued, the words and attachments stay in the box', async () => {
    const image = { id: 3, name: 'c.png', mime: 'image/png', dataUrl: '', saving: false, path: '/tmp/c.png' };
    store.setChatInput('Hello');
    store.setChatAttachments([image]);
    const chat = controller();
    const sent = chat.send();
    await settle();
    expect(store.chatBusy()).toBe(true);
    chat.stop();
    state.preflight!.resolve(null);
    await expect(sent).resolves.toBe(false);
    expect(state.log).toEqual([]);
    expect(store.beginQueuedChatTurn).not.toHaveBeenCalled();
    expect(store.chatInput()).toBe('Hello');
    expect(store.chatAttachments()).toEqual([image]);
    expect(store.chatBusy()).toBe(false);
    expect(state.held).toBe(0);
    expect(store.pushChatTurn).toHaveBeenCalledWith(stoppedNotice);
    store.setChatAttachments([]);
  });

  it('during the agent check of a fresh chat: no chat_cancel, and a preset goes into the box beside the draft', async () => {
    state.key = null;
    state.descriptors = [];
    store.setChatInput('My draft');
    const chat = controller();
    const sent = chat.send(undefined, 'Ship it');
    await settle();
    chat.stop();
    state.discovery!.resolve();
    await expect(sent).resolves.toBe(false);
    expect(state.preflight).toBeNull();
    expect(state.log).toEqual([]);
    expect(store.chatInput()).toBe('Ship it\n\nMy draft');
    expect(store.chatBusy()).toBe(false);
  });

  it('while chat_send is in flight: the accepted row is withdrawn and the send reports nothing sent', async () => {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.queuedOnScreen = clientTurnId();
    chat.stop();
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0']);
    const accepted = { sessionKey: 'conv-a', queueId: 9, queuePosition: 1, phase: 'queued', waitingFor: null };
    state.reply!.resolve(accepted);
    await expect(sent).resolves.toBe(false);
    expect(store.acceptQueuedChatTurn).toHaveBeenCalledWith(clientTurnId(), accepted);
    expect(store.withdrawStoppedSend).toHaveBeenCalledWith(clientTurnId(), accepted);
  });

  it('stopped in flight, then left: the withdrawn send ends in its own chat with its words', async () => {
    state.reply = state.deferred<unknown>();
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.queuedOnScreen = clientTurnId();
    chat.stop();
    state.key = 'conv-b';
    state.queuedOnScreen = null;
    state.reply!.resolve({ sessionKey: 'conv-a', queueId: 9, queuePosition: 1, phase: 'queued', waitingFor: null });
    await expect(sent).resolves.toBe(false);
    expect(store.endBackgroundSend).toHaveBeenCalledWith('conv-a', clientTurnId(), 'Your queued message was not sent: you stopped it. It is back in the composer.');
    expect(store.returnToConversation).toHaveBeenCalledWith('conv-a', 'Hello', [], "Your message wasn't sent: you stopped it.");
  });

  // Codex review of 9a15569c5: a forwarded held send drain_held had already
  // taken escaped Stop, and the controller reported a normal send (chip, follow).
  async function stopForwardedInFlight(outcome: string) {
    state.reply = state.deferred<unknown>();
    state.outcome = outcome;
    const chat = controller();
    const sent = chat.send(undefined, 'Hello');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    state.queuedOnScreen = clientTurnId();
    chat.stop();
    vi.mocked(store.pushChatTurn).mockClear();
    state.reply!.resolve({ sessionKey: 'conv-next', queueId: 0, queuePosition: 0, phase: 'queued', waitingFor: null, heldId: 'held-s', forwardedFrom: 'conv-a' });
    return sent;
  }

  it('a forwarded send Stop came too late for is not reported as a normal send', async () => {
    await expect(stopForwardedInFlight('sent')).resolves.toBe(true);
    expect(store.addHeldChip).not.toHaveBeenCalled();
    expect(store.removeChatUserTurn).toHaveBeenCalledWith({ clientTurnId: clientTurnId(), queueId: null });
    expect(store.pushChatTurn).toHaveBeenCalledWith({ role: 'system', text: 'Stop came too late. It had already been sent.', tools: [] });
    expect(store.chatBusy()).toBe(false);
    expect(store.chatInput()).toBe('');
  });

  it('a forwarded send stopped after it started there says so, with no chip', async () => {
    await expect(stopForwardedInFlight('stopped')).resolves.toBe(false);
    expect(store.addHeldChip).not.toHaveBeenCalled();
    expect(store.pushChatTurn).toHaveBeenCalledWith({ role: 'system', text: 'Stopped after it started in the chat it went to.', tools: [] });
  });

  it('a stop belongs to its one send: the next send runs, a later Stop goes to chat_cancel, another chat\'s Stop misses it', async () => {
    const chat = controller();
    const first = chat.send(undefined, 'One');
    await settle();
    chat.stop();
    state.preflight!.resolve(null);
    await expect(first).resolves.toBe(false);
    // The next send is not stopped by the earlier intent.
    state.reply = state.deferred<unknown>();
    const second = chat.send(undefined, 'Two');
    await settle();
    state.preflight!.resolve(null);
    await settle();
    // Left for conv-b while chat_send is in flight: conv-b's Stop is its own.
    state.key = 'conv-b';
    chat.stop();
    state.reply!.resolve({ sessionKey: 'conv-a', queueId: 10, queuePosition: 1, phase: 'queued', waitingFor: null });
    await expect(second).resolves.toBe(true);
    expect(store.withdrawStoppedSend).not.toHaveBeenCalled();
    expect(store.acceptBackgroundSend).toHaveBeenCalled();
    // Once queued, Stop names the row's chat as before.
    state.key = 'conv-a';
    chat.stop();
    expect(state.log).toEqual(['queued:conv-a:held=0', 'sent:conv-a:held=0', 'chat_cancel:conv-b', 'chat_cancel:conv-a']);
  });
});
