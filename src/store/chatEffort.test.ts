import { beforeEach, expect, it, vi } from 'vitest';

// Settings PR 7 (one home each): the composer's Effort sets only that chat.
// Since 2026-11-12 the level lives in Rust under the conversation's session
// key (`chat_effort_set`, mocked here), the window keeps a cache, and the app
// default, saved only through Settings' saveAgentEffort, is never written.
const h = vi.hoisted(() => {
  const values = new Map<string, string>();
  vi.stubGlobal('localStorage', { getItem: (k: string) => values.get(k) ?? null, setItem: (k: string, v: string) => values.set(k, v), removeItem: (k: string) => values.delete(k) });
  return { values, setAgentSettings: vi.fn(async () => null), chatEffortSet: vi.fn(async () => undefined) };
});
vi.mock('../nativeEvents', () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock('../api', () => ({ api: new Proxy({ setAgentSettings: h.setAgentSettings, chatEffortSet: h.chatEffortSet }, { get: (t, k) => (k in t ? (t as Record<string | symbol, unknown>)[k] : vi.fn(async () => null)) }) }));
vi.mock('./runtime', () => ({ activePage: () => 'today', setActivePage: vi.fn(), closeChatPanel: vi.fn(), refetchShownSessions: vi.fn(), requestChatFocus: vi.fn(), setChatPanelOpen: vi.fn() }));
vi.mock('./overlays', () => ({ showConfirm: vi.fn() }));
vi.mock('./snackbars', () => ({ flashFooter: vi.fn() }));
import * as rt from './chatRuntime';
import menuSource from '../components/ChatModelMenu.tsx?raw';
import { effortOnKeyChange, parseStoredEffort } from '../ultracode';
import { resetChatEffortCache } from './chatEffortStore';

const A = 's-20261112-effort001';
const B = 's-20261112-effort002';

beforeEach(() => {
  h.values.clear();
  h.setAgentSettings.mockClear();
  h.chatEffortSet.mockClear();
  resetChatEffortCache();
  rt.resetChatState();
  rt.setChatAgentSessionKeyPersistent(A);
});

it('changing effort in a chat is that chat only: the app default is never written', () => {
  rt.setChatEffortPersistent('high');
  expect(rt.chatEffort()).toBe('high');
  expect(h.chatEffortSet).toHaveBeenCalledWith(A, 'high');
  expect([...h.values.keys()].some(k => k.startsWith('hollow.effort.'))).toBe(false);
  expect(h.setAgentSettings).not.toHaveBeenCalled();
  // Another conversation has none; the first still has its own.
  rt.setChatAgentSessionKeyPersistent(B);
  expect(rt.chatEffort()).toBeNull();
  rt.setChatAgentSessionKeyPersistent(A);
  expect(rt.chatEffort()).toBe('high');
});

it('survives a reload: reading the conversation back gives its level', async () => {
  rt.setChatEffortPersistent('max');
  expect(rt.storedChatEffort(A)).toBe('max');
  expect(rt.storedChatEffort(B)).toBeNull();
  rt.setChatEffortPersistent(null);
  expect(rt.storedChatEffort(A)).toBeNull();
  // Saves reach Rust in order: the clear goes once the first has answered.
  await vi.waitFor(() => expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, null));
});

it('a level picked in a new chat before its first send is stored under the minted key', () => {
  rt.setChatAgentSessionKeyPersistent(null);
  rt.setChatEffortPersistent('low');
  expect(h.chatEffortSet).not.toHaveBeenCalled();
  rt.setChatAgentSessionKeyPersistent(B, { ownTurn: true });
  expect(rt.chatEffort()).toBe('low');
  expect(h.chatEffortSet).toHaveBeenCalledWith(B, 'low');
});

it('New chat or a reset clears a level picked in an unsent chat, even with no key', () => {
  rt.setChatAgentSessionKeyPersistent(null);
  rt.setChatEffortPersistent('high');
  expect(rt.chatEffort()).toBe('high');
  rt.resetChatState();
  expect(rt.chatEffort()).toBeNull();
  // A conversation with a key keeps its own through the reset and a reopen.
  rt.setChatAgentSessionKeyPersistent(A);
  rt.setChatEffortPersistent('max');
  rt.resetChatState();
  expect(rt.chatEffort()).toBeNull();
  rt.setChatAgentSessionKeyPersistent(A);
  expect(rt.chatEffort()).toBe('max');
});

it('the menu checks the chat own choice, so Default is checked when the chat has none', () => {
  expect(menuSource).toContain("(chatEffort() ?? '') === level");
  expect(menuSource).not.toContain("(effortOf(chatAgent()) ?? '') === level");
});

it('the pure rules mirror Ultracode and ignore anything that is not a level', () => {
  expect(parseStoredEffort('xhigh')).toBe('xhigh');
  expect(parseStoredEffort('ultracode')).toBeNull();
  expect(parseStoredEffort(null)).toBeNull();
  expect(effortOnKeyChange('A', null, 'high', 'high', false).level).toBeNull();
  expect(effortOnKeyChange(null, 'B', 'high', null, false)).toEqual({ level: null, persist: false });
  expect(effortOnKeyChange('A', 'A', 'high', null, false)).toEqual({ level: 'high', persist: false });
});

it('a forward to the chain successor carries the level and stores it; an opened chat does not', () => {
  rt.setChatAgentSessionKeyPersistent(A);
  rt.setChatEffortPersistent('high');
  rt.setChatAgentSessionKeyPersistent(B, { ownTurn: true });
  expect(rt.chatEffort()).toBe('high');
  expect(h.chatEffortSet).toHaveBeenCalledWith(B, 'high');
  // Opening an unrelated chat reads its own (none), never the one left.
  rt.setChatAgentSessionKeyPersistent('C-unrelated');
  expect(rt.chatEffort()).toBeNull();
  expect(effortOnKeyChange('A', 'B', 'high', null, true)).toEqual({ level: 'high', persist: true });
  expect(effortOnKeyChange('A', 'B', 'high', 'low', true)).toEqual({ level: 'low', persist: false });
  expect(effortOnKeyChange('A', 'B', 'high', null, false)).toEqual({ level: null, persist: false });
});

it('switching the chat to an agent without its level clears the level, in the cache and in Rust (#1321)', async () => {
  rt.setAgentDescriptors([
    { id: 'claude', capabilities: { effortLevels: ['low', 'high', 'max'] } },
    { id: 'codex', capabilities: { effortLevels: ['low', 'medium', 'high'] } },
  ] as unknown as Parameters<typeof rt.setAgentDescriptors>[0]);
  rt.setChatAgentPersistent('claude');
  rt.setChatEffortPersistent('max');
  h.chatEffortSet.mockClear();
  rt.switchChatAgent('codex');
  expect(rt.chatEffort()).toBeNull();
  await vi.waitFor(() => expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, null));
  // A level the new agent also has stays.
  rt.setChatEffortPersistent('high');
  rt.switchChatAgent('claude');
  expect(rt.chatEffort()).toBe('high');
  rt.setAgentDescriptors([]);
});

// R11b review (2026-11-15): a failed save that lands after the person moved
// to another chat restores that chat by its key, never the one on screen.
it('a failed save restores its own chat by key, and leaves the open chat alone', async () => {
  rt.setChatUltracodePersistent(true);
  rt.setChatAgentSessionKeyPersistent(B);
  rt.setChatEffortPersistent('low');
  rt.restoreChatEffortChoice(A, 'high', true);
  expect(rt.storedChatUltracode(A)).toBe(true);
  expect(rt.storedChatEffort(A)).toBe('high');
  await vi.waitFor(() => expect(h.chatEffortSet).toHaveBeenLastCalledWith(A, 'high'));
  // B is on screen: its signals are untouched.
  expect(rt.chatEffort()).toBe('low');
  expect(rt.chatUltracode()).toBe(false);
  // Restoring the open chat updates what shows.
  rt.restoreChatEffortChoice(B, null, true);
  expect(rt.chatEffort()).toBeNull();
  expect(rt.chatUltracode()).toBe(true);
  expect(rt.storedChatUltracode(B)).toBe(true);
});

it('the menu restores only a failed save that is still the latest pick, by key', () => {
  expect(menuSource).toContain("=== 'failed-latest' && key");
  expect(menuSource).toContain('restoreChatEffortChoice(key, was, wasUltra)');
});
