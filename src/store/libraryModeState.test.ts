import { beforeEach, expect, it, vi } from 'vitest';

// S4 (2026-11-03): Conversation history is Library → Conversations. The old
// page id `chat` shows the tab; any other way into Library leaves it for the
// view it was opened from; the tab is never the remembered Library view.
const saved = vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
    removeItem: (key: string) => { map.delete(key); },
  });
  return map;
});

const load = async () => { vi.resetModules(); return import('./libraryModeState'); };

beforeEach(() => saved.clear());

// Overnight 11-12 L3: All, one searchable list, is the default (it was Results).
it('opens new and invalid saved views on All', async () => {
  expect((await load()).libraryMode()).toBe('all');
  saved.set('hollow.library.mode', 'unknown');
  expect((await load()).libraryMode()).toBe('all');
});

it.each(['all', 'learnings', 'skills', 'history'] as const)('keeps an explicit saved %s choice', async view => {
  const m = await load();
  m.setLibraryMode(view, true);
  expect((await load()).libraryMode()).toBe(view);
});

// Overnight 11-12 L4: Notes, Results and Recordings are All's views, off the
// strip. Picking one remembers All, and one remembered before opens All.
it.each(['notes', 'results', 'recordings'] as const)('remembers All for its %s view, and opens All for an old saved one', async view => {
  const m = await load();
  m.setLibraryMode(view, true);
  expect(m.libraryMode()).toBe(view);
  expect(saved.get('hollow.library.mode')).toBe('all');
  saved.set('hollow.library.mode', view);
  expect((await load()).libraryMode()).toBe('all');
});

it('opens Conversations for the chat page id and goes back to the view it came from', async () => {
  const m = await load();
  m.setLibraryMode('results', true);
  m.placeLibraryView('chat');
  expect(m.libraryMode()).toBe('conversations');
  // Away and back to Library by its rail row (or ⌘3, or Back): Results again.
  m.placeLibraryView('today');
  expect(m.libraryMode()).toBe('conversations');
  m.placeLibraryView('vault');
  expect(m.libraryMode()).toBe('results');
});

it('leaves Conversations for a retired Library link too, and keeps the first view on a second chat', async () => {
  const m = await load();
  m.setLibraryMode('recordings');
  m.placeLibraryView('chat');
  m.placeLibraryView('chat');
  m.placeLibraryView('ingest');
  expect(m.libraryMode()).toBe('recordings');
  // A view picked while away from Conversations is left alone.
  m.setLibraryMode('connected');
  m.placeLibraryView('vault');
  expect(m.libraryMode()).toBe('connected');
});

it('never remembers Conversations as the Library view for the next launch', async () => {
  let m = await load();
  m.setLibraryMode('skills', true);
  m.setLibraryMode('conversations', true);
  expect(saved.get('hollow.library.mode')).toBe('skills');
  m = await load();
  expect(m.libraryMode()).toBe('skills');
  // An unsupported saved view uses the All landing view.
  saved.set('hollow.library.mode', 'conversations');
  m = await load();
  expect(m.libraryMode()).toBe('all');
});

it('a Back step into Library shows the tab it kept, after Conversations too', async () => {
  const m = await load();
  // Notes → Conversations → Results → Back → Back: Conversations, then Notes.
  m.placeLibraryView('chat');
  m.setLibraryMode('results', true);
  m.placeLibraryView('chat');
  expect(m.libraryMode()).toBe('conversations');
  m.placeLibraryView('vault', 'notes');
  expect(m.libraryMode()).toBe('notes');
  m.placeLibraryView('vault', 'results');
  expect(m.libraryMode()).toBe('results');
  // A kept tab never names Conversations, and pages outside Library change nothing.
  m.placeLibraryView('chat');
  m.placeLibraryView('vault', 'conversations');
  expect(m.libraryMode()).toBe('results');
  m.placeLibraryView('tasks', 'notes');
  expect(m.libraryMode()).toBe('results');
});

// Overnight 11-12 L2: Skills and History are Library tabs, remembered like the
// others; a Back step that kept either tab reopens it.
it('reads saved Skills and History back, and still refuses Connected projects and Conversations', async () => {
  for (const view of ['skills', 'history'] as const) {
    saved.set('hollow.library.mode', view);
    expect((await load()).savedMode()).toBe(view);
  }
  for (const view of ['connected', 'conversations']) {
    saved.set('hollow.library.mode', view);
    expect((await load()).savedMode()).toBe('all');
  }
  const m = await load();
  m.setLibraryMode('skills', true);
  m.setLibraryMode('connected', true);
  expect(saved.get('hollow.library.mode')).toBe('skills');
  m.placeLibraryView('vault', 'history');
  expect(m.libraryMode()).toBe('history');
});
