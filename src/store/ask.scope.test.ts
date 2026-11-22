import { beforeEach, describe, expect, it, vi } from 'vitest';
// The browser build: the server build vitest resolves runs no effects.
// @ts-expect-error: the browser build has no type declarations of its own.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// P1-11: the last scope is remembered on every submit, a quick chat
// included. "Across all my work" asks the Main agent on its page (run 15 P3:
// the drawer beside the page is gone): Enter sends there once its
// conversation is ready, bound to that conversation and withdrawn if the
// person leaves it first; ⌥-Enter only places the text.
type Req = { text: string; cwd: string; preset?: boolean; sessionKey?: string | null; done?: (ok: boolean) => void };
const state = vi.hoisted(() => {
  const store = new Map<string, string>();
  (globalThis as unknown as { localStorage: Storage }).localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, v); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => store.clear(), key: () => null, length: 0,
  } as Storage;
  return {
    store,
    openOk: true,
    busy: false,
    canLeave: true,
    navigate: true,
    project: null as string | null,
    calls: [] as string[],
    sends: [] as Req[],
    footer: [] as string[],
    // Filled by the runtime mock below with real signals.
    sig: {} as {
      pending: () => Req | null; setPending: (v: Req | null) => void;
      mainOpen: () => boolean; setMainOpen: (v: boolean) => void;
      key: () => string | null; setKey: (v: string | null) => void;
    },
  };
});
vi.mock('./runtime', async () => {
  const { createSignal } = await import('solid-js');
  const [pending, setPendingRaw] = createSignal<Req | null>(null);
  const [mainOpen, setMainOpen] = createSignal(false);
  const [key, setKey] = createSignal<string | null>('main-key');
  state.sig = { pending, setPending: v => setPendingRaw(() => v), mainOpen, setMainOpen, key, setKey };
  return {
    setChatPrefillRequest: (t: string) => { state.calls.push(`page-prefill:${t}`); },
    pendingChatSend: pending,
    setPendingChatSend: (v: Req | null) => { if (v) { state.calls.push(`page-send:${v.text}`); state.sends.push(v); } setPendingRaw(() => v); },
    requestChatFocus: () => {},
    chatCwd: () => '/vault/workspaces/main',
    chatAgentSessionKey: key,
    chatBusy: () => state.busy,
    canLeaveConversation: () => state.canLeave,
    flashFooter: (t: string) => { state.footer.push(t); },
  };
});
vi.mock('./workspaces', async () => {
  const { createSignal } = await import('solid-js');
  const [quickAskDraft, setQuickAskDraft] = createSignal('');
  return {
    quickAskDraft, setQuickAskDraft,
    askProject: () => state.project,
    workspaces: () => [{ id: 'q3-ops', title: 'Q3 regional operations review' }],
    sendQuickAsk: async (text?: string) => { state.calls.push(`quick:${text ?? quickAskDraft()}`); return true; },
    // A refused navigation (an unsaved edit kept) never calls onNavigate.
    goToPage: (page: string, opts: { onNavigate?: () => void } = {}) => { state.calls.push(`page:${page}`); if (state.navigate) opts.onNavigate?.(); },
  };
});
vi.mock('./mainAgent', () => ({
  openMainAgent: async () => { state.calls.push('open-page'); if (state.openOk) state.sig.setMainOpen(true); return state.openOk; },
  mainAgentOpen: () => state.sig.mainOpen(),
}));

beforeEach(() => {
  state.calls.length = 0; state.sends.length = 0; state.footer.length = 0;
  state.openOk = true; state.busy = false; state.canLeave = true; state.navigate = true; state.project = null;
  state.sig.setPending?.(null); state.sig.setMainOpen?.(false); state.sig.setKey?.('main-key');
});

const tick = () => new Promise(r => setTimeout(r, 0));

describe('Ask remembers its scope and asks the COO on its page (P1-11, run 15 P3)', () => {
  it('a quick submit is remembered, and a reload starts on it', async () => {
    state.store.set('hollow.ask.lastScope', 'hollow');
    const { rememberAskScope } = await import('./ask');
    rememberAskScope('quick');
    expect(state.store.get('hollow.ask.lastScope')).toBe('quick');
    vi.resetModules();
    const { lastAskScope } = await import('./ask');
    const { defaultAskScope } = await import('../askScopes');
    expect(defaultAskScope(lastAskScope())).toBe('quick');
  });

  it('Across all my work + Enter opens the page and sends there, bound to that conversation', async () => {
    const { runAsk } = await import('./ask');
    const done = runAsk('main', '  What is due today?  ');
    await tick();
    expect(state.calls).toEqual(['open-page', 'page-send:What is due today?']);
    // Sent as typed words (the page's own draft stays), only in the Main
    // agent conversation that opened (its folder and its key).
    expect(state.sends[0]).toMatchObject({ text: 'What is due today?', cwd: '/vault/workspaces/main', preset: true, sessionKey: 'main-key' });
    expect(state.store.get('hollow.ask.lastScope')).toBe('main');
    // The composer takes it and its send queues a turn: Ask says it went
    // (Home's field may clear).
    state.sig.setPending(null);
    state.sends[0].done?.(true);
    expect(await done).toBe(true);
  });

  it('a send the composer took but could not queue (agent unavailable, folder gone) keeps the words in Ask', async () => {
    const { runAsk } = await import('./ask');
    let settled: boolean | null = null;
    const done = runAsk('main', 'What slipped?').then(ok => { settled = ok; return ok; });
    await tick();
    state.sig.setPending(null);
    await tick();
    // Taken is not sent: Ask waits for the composer's own answer.
    expect(settled).toBeNull();
    state.sends[0].done?.(false);
    expect(await done).toBe(false);
  });

  it('leaving that conversation before it went withdraws the send, and Ask keeps the words', async () => {
    const { runAsk } = await import('./ask');
    state.busy = true;
    const done = runAsk('main', 'What slipped?');
    await tick();
    expect(state.footer.join(' ')).toMatch(/finishes its current turn/);
    state.sig.setKey('another-main-chat');
    expect(await done).toBe(false);
    expect(state.sig.pending()).toBeNull();
  });

  it('leaving the COO page before it went withdraws it too', async () => {
    const { runAsk } = await import('./ask');
    const done = runAsk('main', 'What slipped?');
    await tick();
    state.sig.setMainOpen(false);
    expect(await done).toBe(false);
    expect(state.sig.pending()).toBeNull();
  });

  it('⌥-Enter opens the page and only places the text in its box', async () => {
    const { runAsk } = await import('./ask');
    expect(await runAsk('main', 'Draft the weekly note', { place: true })).toBe(true);
    expect(state.calls).toEqual(['open-page', 'page-prefill:Draft the weekly note']);
  });

  it('on the COO’s own space the text goes into its box, as before', async () => {
    state.sig.setMainOpen(true);
    const { runAsk } = await import('./ask');
    expect(await runAsk('main', 'Hello')).toBe(true);
    expect(state.calls).toEqual(['open-page', 'page-prefill:Hello']);
  });

  it('a page that could not open gets nothing sent or placed', async () => {
    state.openOk = false;
    const { runAsk } = await import('./ask');
    expect(await runAsk('main', 'Hello')).toBe(false);
    expect(state.calls).toEqual(['open-page']);
  });

  it('a saved Answer from my notes starts on a quick chat, and Plan it in Hollow on the COO (One Ask)', async () => {
    for (const [saved, scope] of [['notes', 'quick'], ['hollow', 'main'], ['main', 'main'], ['quick', 'quick'], ['elsewhere', 'quick']] as const) {
      state.store.set('hollow.ask.lastScope', saved);
      vi.resetModules();
      const { currentAskScope } = await import('./ask');
      expect(currentAskScope(), saved).toBe(scope);
    }
  });

  it('the command bar’s row asks where Home’s field is set to: a quick chat, or the COO', async () => {
    state.store.set('hollow.ask.lastScope', 'quick');
    vi.resetModules();
    const { askHere, setHomeAskScope } = await import('./ask');
    expect(await askHere('What is due today?')).toBe(true);
    expect(state.calls).toEqual(['quick:What is due today?']);
    state.calls.length = 0;
    setHomeAskScope('main');
    const done = askHere('What slipped?');
    await tick();
    expect(state.calls).toEqual(['open-page', 'page-send:What slipped?']);
    state.sig.setPending(null);
    state.sends[0].done?.(true);
    expect(await done).toBe(true);
    expect(state.store.get('hollow.ask.lastScope')).toBe('main');
  });

  it('asks nothing while a message is still being sent, and says why', async () => {
    state.canLeave = false;
    const { askHere } = await import('./ask');
    expect(await askHere('What slipped?')).toBe(false);
    expect(state.calls).toEqual([]);
    expect(state.footer.join(' ')).toMatch(/still being sent/);
  });

  it('g v and Ask anything… open Work and ask its field for focus', async () => {
    const { openHomeAsk, homeAskFocusRequest, setHomeAskFocusRequest } = await import('./ask');
    setHomeAskFocusRequest(false);
    openHomeAsk();
    expect(state.calls).toEqual(['page:today']);
    expect(homeAskFocusRequest()).toBe(true);
  });

  it('a refused navigation leaves no focus request and no text behind', async () => {
    state.navigate = false;
    const { openHomeAsk, homeAskFocusRequest, setHomeAskFocusRequest } = await import('./ask');
    const { quickAskDraft, setQuickAskDraft } = await import('./workspaces');
    setHomeAskFocusRequest(false); setQuickAskDraft('');
    openHomeAsk('What slipped this week?');
    expect(homeAskFocusRequest()).toBe(false);
    expect(quickAskDraft()).toBe('');
  });

  it('Ask anything… puts the palette’s sentence in an empty field, never over a draft', async () => {
    const { openHomeAsk } = await import('./ask');
    const { quickAskDraft, setQuickAskDraft } = await import('./workspaces');
    setQuickAskDraft('');
    openHomeAsk('  What slipped this week?  ');
    expect(quickAskDraft()).toBe('What slipped this week?');
    setQuickAskDraft('My own draft');
    openHomeAsk('Something else entirely');
    expect(quickAskDraft()).toBe('My own draft');
  });

  it('the palette row names where it sends, as Home shows it', async () => {
    state.store.set('hollow.ask.lastScope', 'quick');
    vi.resetModules();
    const { askPlaceLabel, setHomeAskScope } = await import('./ask');
    expect(askPlaceLabel()).toBe('New chat');
    state.project = 'q3-ops';
    expect(askPlaceLabel()).toBe('New chat in Q3 regional operations review');
    state.project = 'gone';
    expect(askPlaceLabel()).toBe('New chat in an unavailable project');
    setHomeAskScope('main');
    expect(askPlaceLabel()).toBe('Across all my work');
  });
});
