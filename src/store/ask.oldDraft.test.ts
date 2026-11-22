import { expect, it, vi } from 'vitest';
// The browser build: the server build vitest resolves runs no effects.
// @ts-expect-error: the browser build has no type declarations of its own.
vi.mock('solid-js', () => import('solid-js/dist/solid.js'));

// 2026-10-21: the top bar's Ask ⌄ panel is gone. A draft it kept on this Mac
// shows in Work's Ask field instead, and its saved copy stays until that
// field moves on, so a reload before then doesn't lose it.
const state = vi.hoisted(() => {
  const store = new Map<string, string>();
  (globalThis as unknown as { localStorage: Storage }).localStorage = {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => { store.set(k, v); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => store.clear(), key: () => null, length: 0,
  } as Storage;
  store.set('hollow.ask.draft', JSON.stringify({ text: 'Kept from before', scope: 'notes', beside: false, workspaceId: null, workspaceTitle: null, kept: true }));
  return { store };
});
vi.mock('./runtime', () => ({ setChatPrefillRequest: () => {}, requestChatFocus: () => {} }));
vi.mock('./workspaces', async () => {
  const { createSignal } = await import('solid-js');
  const [quickAskDraft, setQuickAskDraft] = createSignal('');
  return { quickAskDraft, setQuickAskDraft };
});
vi.mock('./mainAgent', () => ({ openMainAgent: async () => true }));

it('an old Ask draft shows in Work’s field, and its saved copy goes once the field moves on', async () => {
  await import('./ask');
  const { quickAskDraft, setQuickAskDraft } = await import('./workspaces');
  expect(quickAskDraft()).toBe('Kept from before');
  expect(state.store.has('hollow.ask.draft'), 'kept until the field changes').toBe(true);
  setQuickAskDraft('');
  await Promise.resolve();
  expect(state.store.has('hollow.ask.draft')).toBe(false);
});
