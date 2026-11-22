import { beforeEach, expect, it, vi } from 'vitest';

// P0-19: Settings owns the default for new chats; this chat's mode changes
// only from the chat, and a refused change says why instead of nothing.
vi.hoisted(() => {
  const store = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, String(v)); },
    removeItem: (k: string) => { store.delete(k); },
  });
});
// What the window told the phone chat API about each chat's mode, and every
// report in the order it reached Hollow (`gate.hold` keeps the next one in
// flight until it resolves).
const reported = vi.hoisted(() => [] as unknown[][]);
const arrived = vi.hoisted(() => [] as string[]);
const gate = vi.hoisted(() => ({ hold: null as Promise<void> | null }));
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => {
  const own: Record<string, unknown> = {
    phoneAccessChatModes: async (...args: unknown[]) => {
      const hold = gate.hold; gate.hold = null;
      if (hold) await hold;
      reported.push(args); arrived.push(args[1] ? 'whole record' : 'one choice');
    },
    phoneAccessNewChatMode: async (mode: unknown) => { arrived.push(`new chats: ${String(mode)}`); },
  };
  return { api: new Proxy(own, { get: (target, key) => target[key as string] ?? vi.fn(async () => null) }) };
});
vi.mock('./runtime', () => ({
  activePage: () => 'chat', closeChatPanel: vi.fn(), requestChatFocus: vi.fn(),
  setActivePage: vi.fn(), setChatPanelOpen: vi.fn(),
}));
vi.mock('./overlays', () => ({ showConfirm: vi.fn(async () => true) }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));

const rt = await import('./chatRuntime');

beforeEach(() => {
  rt.setChatBusy(false);
  rt.resetChatState();
  rt.setChatPermissionMode('bypassPermissions');
  rt.setChatSessionIdPersistent('native-1');
});

it.each(['claude', 'codex', 'grok', 'antigravity'] as const)('keeps the saved new-chat mode of %s', agent => {
  rt.resetChatState(); rt.setChatAgentPersistent(agent);
  rt.setNewAgentPermissionModePersistent(agent, 'plan'); rt.applyNewChatPermissionMode();
  expect(rt.chatPermissionMode()).toBe('plan');
  expect(localStorage.getItem(`new${agent[0].toUpperCase()}${agent.slice(1)}PermissionMode`)).toBe('plan');
  rt.setChatPermissionModePersistent('acceptEdits');
  expect(rt.newChatPermissionMode()).toBe('plan');
});

// Review finding 2 (2026-11-04): a phone's send runs in the mode the window
// shows for its chat, so each choice made here reaches the phone chat API.
it("reports a chat's first-send mode and each later choice to the phone chat API", async () => {
  reported.length = 0;
  rt.setChatPermissionMode('plan');
  // No chat yet: nothing to report until its first send names it.
  expect(rt.setChatPermissionModePersistent('auto')).toBe('');
  rt.setChatAgentSessionKeyPersistent('s-20261104-0000abcd', { ownTurn: true });
  // A replayed turn's key is no choice.
  rt.setChatAgentSessionKeyPersistent('s-20261104-0000abcd');
  expect(rt.setChatPermissionModePersistent('bypassPermissions')).toBe('');
  await vi.waitFor(() => expect(reported).toHaveLength(2));
  expect(reported).toEqual([
    [[['s-20261104-0000abcd', 'auto']], false, false],
    [[['s-20261104-0000abcd', 'bypassPermissions']], false, false],
  ]);
});

// Second review, findings 2 and 3 (2026-11-04): the reports were fire and
// forget with no order, so two in flight could land the older one last. They
// go one at a time in the order they were made, and the whole record goes
// when Companion devices opens or Hollow asks (`phone-access-modes-wanted`).
it('sends its reports one at a time in order, and the whole record when asked', async () => {
  reported.length = 0; arrived.length = 0;
  let release!: () => void;
  gate.hold = new Promise<void>(resolve => { release = resolve; });
  rt.setChatPermissionMode('plan');
  rt.setChatAgentSessionKeyPersistent('s-20261104-0000beef', { ownTurn: true });
  expect(rt.setChatPermissionModePersistent('auto')).toBe('');
  rt.reportPhoneModes();
  await new Promise(resolve => setTimeout(resolve, 20));
  // The first is still in flight, and nothing overtakes it.
  expect(arrived).toEqual([]);
  release();
  await vi.waitFor(() => expect(arrived).toHaveLength(4));
  expect(arrived).toEqual(['one choice', 'one choice', `new chats: ${rt.newChatPermissionMode()}`, 'whole record']);
  expect(reported[1]).toEqual([[['s-20261104-0000beef', 'auto']], false, false]);
  expect(reported[2][1]).toBe(true);
  expect(reported[2][0]).toContainEqual(['s-20261104-0000beef', 'auto']);
});

it('persists the first send choice and a later explicit selection for this key only', async () => {
  const { conversationPermission } = await import('../permModes');
  rt.setChatPermissionMode('plan');
  rt.setChatAgentSessionKeyPersistent('first-send', { ownTurn: true });
  expect(conversationPermission('first-send', 'auto')).toEqual({ mode: 'plan', resumeNative: false });
  rt.setChatPermissionModePersistent('acceptEdits');
  expect(conversationPermission('first-send', 'plan').mode).toBe('acceptEdits');
  expect(rt.chatSessionId()).toBeNull();
});

// 2026-11-04: a change within one class keeps the native session (each
// process takes the new mode); Plan and the acting modes, or Grok's sandbox
// profiles, still restart with retained context.
it('keeps the session across modes of one class and restarts across classes', () => {
  rt.setChatAgentPersistent('claude');
  rt.setChatPermissionMode('acceptEdits');
  rt.setChatSessionIdPersistent('native-1');
  expect(rt.setChatPermissionModePersistent('bypassPermissions')).toBe('');
  expect(rt.chatSessionId()).toBe('native-1');
  expect(rt.setChatPermissionModePersistent('plan')).toBe('');
  expect(rt.chatSessionId()).toBeNull();
  rt.setChatAgentPersistent('grok');
  rt.setChatPermissionMode('acceptEdits');
  rt.setChatSessionIdPersistent('grok-native');
  expect(rt.setChatPermissionModePersistent('bypassPermissions')).toBe('');
  expect(rt.chatSessionId()).toBeNull();
});

it('restores the live background owner mode along with its native session', () => {
  rt.setChatAgentSessionKeyPersistent('live-mode'); rt.setChatPermissionMode('plan');
  rt.setChatSessionIdPersistent('live-native'); rt.setChatBusy(true);
  expect(rt.backgroundRunningChat()).toBe(true);
  rt.resetChatState(); rt.setChatPermissionMode('bypassPermissions');
  expect(rt.restoreBackgroundChat('live-mode')).toBe(true);
  expect(rt.chatPermissionMode()).toBe('plan');
  expect(rt.chatSessionId()).toBe('live-native');
  expect(rt.chatBusy()).toBe(true);
  rt.setChatBusy(false);
});

it('startup reads the current conversation choice before queue adoption can run', async () => {
  localStorage.setItem('hollow.chat.mode.migrated', '1');
  localStorage.setItem('hollow.chat.modes.v1', JSON.stringify([['startup-plan', 'plan']]));
  localStorage.setItem('chatAgentSessionKey', 'startup-plan');
  localStorage.setItem('chatPermissionMode', 'bypassPermissions');
  vi.resetModules();
  const reloaded = await import('./chatRuntime');
  expect(reloaded.chatPermissionMode()).toBe('plan');
});

it('events for an existing key cannot overwrite its selected mode', async () => {
  const { conversationPermission, rememberConversationMode } = await import('../permModes');
  rememberConversationMode('adopted-plan', 'plan');
  rt.setChatAgentSessionKeyPersistent('adopted-plan');
  rt.setChatPermissionMode('bypassPermissions');
  rt.setChatAgentSessionKeyPersistent('adopted-plan', { ownTurn: true });
  expect(conversationPermission('adopted-plan').mode).toBe('plan');
});

it('startup retains the paired Plan copy for a chat without an explicit choice', async () => {
  localStorage.setItem('hollow.chat.mode.migrated', '1');
  localStorage.setItem('hollow.chat.modes.v1', '[]');
  localStorage.setItem('chatAgentSessionKey', 'agent-started-plan');
  localStorage.setItem('chatPermissionMode', 'plan');
  vi.resetModules();
  const reloaded = await import('./chatRuntime');
  expect(reloaded.chatPermissionMode()).toBe('plan');
});

it('while the turn runs, Revert is refused with the reason and the session is kept', () => {
  rt.setChatBusy(true);
  expect(rt.permissionChangeBlocked()).toBe('Stop or finish the running turn first');
  expect(rt.setChatPermissionModePersistent('acceptEdits')).toBe('Stop or finish the running turn first');
  expect(rt.chatPermissionMode()).toBe('bypassPermissions');
  expect(rt.chatSessionId()).toBe('native-1');
});

it('once the turn is done, Revert applies to this chat and says nothing is refused', () => {
  expect(rt.permissionChangeBlocked()).toBe('');
  expect(rt.setChatPermissionModePersistent('acceptEdits')).toBe('');
  expect(rt.chatPermissionMode()).toBe('acceptEdits');
});

it('the new-chats mode never changes this chat or its session, busy or not', () => {
  for (const busy of [true, false]) {
    rt.setChatBusy(busy);
    rt.setNewAgentPermissionModePersistent(rt.chatAgent(), 'plan');
    expect(rt.newChatPermissionMode()).toBe('plan');
    expect(rt.chatPermissionMode()).toBe('bypassPermissions');
    expect(rt.chatSessionId()).toBe('native-1');
  }
});

it('a new chat starts in the new-chats mode; an opened one keeps its mode', () => {
  rt.resetChatState();
  rt.setNewAgentPermissionModePersistent(rt.chatAgent(), 'auto');
  rt.setChatSessionIdPersistent('opened-native');
  rt.applyNewChatPermissionMode();
  expect(rt.chatPermissionMode()).toBe('bypassPermissions');
  rt.resetChatState();
  rt.applyNewChatPermissionMode();
  expect(rt.chatPermissionMode()).toBe('auto');
});

it("a new chat whose agent can't run the new-chats mode starts in Accept edits", () => {
  rt.resetChatState();
  rt.setNewAgentPermissionModePersistent(rt.chatAgent(), 'auto');
  rt.setAgentDescriptors([{ id: rt.chatAgent(), capabilities: { unavailableModes: { auto: 'No Auto on this agent' } } }] as unknown as Parameters<typeof rt.setAgentDescriptors>[0]);
  rt.applyNewChatPermissionMode();
  expect(rt.chatPermissionMode()).toBe('acceptEdits');
  expect(rt.newChatPermissionMode()).toBe('auto');
  rt.setAgentDescriptors([]);
});

// Per-agent new-chat modes (2026-11-12): the agent's own saved mode, else its
// default. The single `newChatPermissionMode` from before is retired (owner
// decision 2026-11-12: it was seeded from the last chat's mode, never chosen).
const AGENT_KEYS = ['newClaudePermissionMode', 'newCodexPermissionMode', 'newGrokPermissionMode', 'newAntigravityPermissionMode'];
async function freshRuntime(storage: Record<string, string | null>) {
  for (const k of [...AGENT_KEYS, 'newChatPermissionMode', 'hollow.newChatModes.seeded', 'chatPermissionMode']) localStorage.removeItem(k);
  for (const [k, v] of Object.entries(storage)) if (v != null) localStorage.setItem(k, v);
  vi.resetModules();
  return import('./chatRuntime');
}

it('a fresh install starts Claude and Codex chats in Auto, Grok and Antigravity in Accept edits', async () => {
  const fresh = await freshRuntime({});
  expect((['claude', 'codex', 'grok', 'antigravity'] as const).map(a => [a, fresh.newChatModeFor(a), fresh.newChatModeSource(a)])).toEqual([
    ['claude', 'auto', 'default'], ['codex', 'auto', 'default'], ['grok', 'acceptEdits', 'default'], ['antigravity', 'acceptEdits', 'default'],
  ]);
  // A chat's own mode, saved as the chat's, never becomes the default at the next launch.
  localStorage.setItem('chatPermissionMode', 'bypassPermissions');
  vi.resetModules();
  const next = await import('./chatRuntime');
  expect(next.newChatModeFor('claude')).toBe('auto');
  expect(localStorage.getItem('newChatPermissionMode')).toBeNull();
  expect(localStorage.getItem('hollow.newChatModes.seeded')).toBeNull();
});

it('ignores the retired single setting: Claude and Codex start in Auto; an agent\'s own saved mode still wins', async () => {
  // This install's case: the old key was seeded from the last chat's mode.
  const r = await freshRuntime({ newChatPermissionMode: 'acceptEdits', chatPermissionMode: 'acceptEdits' });
  expect(r.newChatModeFor('claude')).toBe('auto');
  expect(r.newChatModeFor('codex')).toBe('auto');
  expect(r.newChatModeFor('grok')).toBe('acceptEdits');
  expect(r.newChatModeSource('claude')).toBe('default');
  r.resetChatState(); r.setChatAgentPersistent('codex'); r.applyNewChatPermissionMode();
  expect(r.chatPermissionMode()).toBe('auto');
  r.setNewAgentPermissionModePersistent('claude', 'plan');
  expect(r.newChatModeFor('claude')).toBe('plan');
  expect(r.newChatModeSource('claude')).toBe('own');
  expect(r.newChatModeFor('codex')).toBe('auto');
  expect(localStorage.getItem('newClaudePermissionMode')).toBe('plan');
  // Saved per agent, so it survives a relaunch; the old key still changes nothing.
  vi.resetModules();
  const again = await import('./chatRuntime');
  expect(again.newChatModeFor('claude')).toBe('plan');
  expect(again.newChatModeFor('codex')).toBe('auto');
});

it('a new chat starts in its agent\'s own mode, never one the agent can\'t run', async () => {
  const r = await freshRuntime({ newAntigravityPermissionMode: 'auto' });
  r.setAgentDescriptors([{ id: 'antigravity', capabilities: { unavailableModes: { auto: 'No Auto' } } }] as unknown as Parameters<typeof r.setAgentDescriptors>[0]);
  r.resetChatState(); r.setChatAgentPersistent('antigravity'); r.applyNewChatPermissionMode();
  expect(r.chatPermissionMode()).toBe('acceptEdits');
  r.resetChatState(); r.setChatAgentPersistent('codex'); r.applyNewChatPermissionMode();
  expect(r.chatPermissionMode()).toBe('auto');
  r.setAgentDescriptors([]);
});

it('switching a chat\'s agent moves its mode to the nearest one there and says so in one line', async () => {
  const r = await freshRuntime({});
  const { flashFooter } = await import('./snackbars');
  r.resetChatState(); r.setChatAgentPersistent('claude'); r.setChatPermissionMode('acceptEdits');
  expect(r.switchChatAgent('codex')).toBe('Permissions: Accept edits on Claude is Ask for approval on Codex.');
  expect(r.chatAgent()).toBe('codex');
  expect(r.chatPermissionMode()).toBe('default');
  expect(flashFooter).toHaveBeenLastCalledWith('Permissions: Accept edits on Claude is Ask for approval on Codex.', 6000);
  // Antigravity can't run Auto: the nearest narrower mode, and the reason.
  r.setAgentDescriptors([{ id: 'antigravity', capabilities: { unavailableModes: { auto: 'No Auto' } } }] as unknown as Parameters<typeof r.setAgentDescriptors>[0]);
  r.switchChatAgent('claude'); r.setChatPermissionModePersistent('auto');
  expect(r.switchChatAgent('antigravity')).toBe('Permissions: Antigravity can’t run Auto, so this chat now uses Accept edits.');
  expect(r.chatPermissionMode()).toBe('acceptEdits');
  // Same name on both: no line, no change.
  r.switchChatAgent('claude'); r.setChatPermissionModePersistent('plan');
  expect(r.switchChatAgent('grok')).toBe('');
  expect(r.chatPermissionMode()).toBe('plan');
  // Nothing moves while a turn runs.
  r.setChatBusy(true);
  expect(r.switchChatAgent('codex')).toBe('');
  expect(r.chatAgent()).toBe('grok');
  r.setChatBusy(false);
  r.setAgentDescriptors([]);
});
