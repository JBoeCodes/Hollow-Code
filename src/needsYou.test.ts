import { describe, expect, it } from 'vitest';
import { createNeedsYouList, shownResultPath } from './needsYou';
import type { NeedsYouItem, NeedsYouPage } from './workspaceTypes';

const results = [{ path: 'workspaces/a/outputs/newest.md' }, { path: 'workspaces/a/outputs/asked.md' }];

describe('shownResultPath (B·F15)', () => {
  it('shows the result opened by its path, not the newest', () => {
    expect(shownResultPath(results, 'workspaces/a/outputs/asked.md', true)).toBe('workspaces/a/outputs/asked.md');
  });
  it('shows nothing in place of an exactly opened result that is gone', () => {
    expect(shownResultPath(results, 'workspaces/a/outputs/deleted.md', true)).toBe('');
  });
  it('falls back to the newest only for a remembered selection or none', () => {
    expect(shownResultPath(results, 'workspaces/a/outputs/deleted.md', false)).toBe('workspaces/a/outputs/newest.md');
    expect(shownResultPath(results, undefined, false)).toBe('workspaces/a/outputs/newest.md');
    expect(shownResultPath([], undefined, false)).toBe('');
  });
});

describe('createNeedsYouList (the waiting-on-you surface, paged not capped)', () => {
  const question = (i: number): NeedsYouItem => ({
    id: `question:q-${i}`, kind: 'question', sourceId: `q-${i}`, title: `Question ${i}`, detail: null, state: 'pending',
    at: null, workspaceId: null, sessionKey: `s-${i}`, path: null, category: 'question', link: null,
  });
  // A keyset-paged fake of list_needs_you over `all`: the cursor is the last id shown.
  const fake = (all: NeedsYouItem[]) => async (cursor: string | null, limit: number): Promise<NeedsYouPage> => {
    const start = cursor ? all.findIndex(q => q.id === cursor) + 1 : 0;
    const items = all.slice(start, start + limit);
    return { items, total: all.length, nextCursor: start + items.length < all.length ? items[items.length - 1].id : null, unavailable: [] };
  };

  it("reaches every waiting question beyond Activity's old cap of 30 with Show more", async () => {
    const all = Array.from({ length: 45 }, (_, i) => question(i));
    const list = createNeedsYouList(fake(all), 30);
    await list.load();
    expect(list.items()).toHaveLength(30);
    expect(list.total()).toBe(45);
    expect(list.next()).not.toBeNull();
    await list.load(true);
    expect(list.items().map(q => q.sourceId)).toEqual(all.map(q => q.sourceId));
    expect(list.next()).toBeNull();
    // A refresh keeps as many rows as were shown, not one page.
    await list.load();
    expect(list.items()).toHaveLength(45);
  });

  it('drops a response a newer load overtook, and keeps the list when a load fails', async () => {
    let release!: (p: NeedsYouPage) => void;
    const calls: number[] = [];
    const list = createNeedsYouList((_, limit) => {
      calls.push(limit);
      return calls.length === 1 ? new Promise(r => { release = r; }) : calls.length === 2 ? fake([question(1)])(null, limit) : Promise.reject(new Error('gone'));
    }, 30);
    const slow = list.load();
    await list.load();
    release({ items: [question(9)], total: 1, nextCursor: null, unavailable: [] });
    await slow;
    expect(list.items().map(q => q.sourceId)).toEqual(['q-1']);
    await list.load();
    expect(list.items().map(q => q.sourceId)).toEqual(['q-1']);
    // The kept list never passes for a fresh one.
    expect(list.failed()).toBe(true);
  });

  // Every fetch waits until the test answers it, in whatever order it likes.
  const deferred = () => {
    const calls: { cursor: string | null; limit: number; answer: (p: NeedsYouPage) => void; fail: (e: Error) => void }[] = [];
    const fetchPage = (cursor: string | null, limit: number) => new Promise<NeedsYouPage>((answer, fail) => { calls.push({ cursor, limit, answer, fail }); });
    return { calls, fetchPage };
  };
  const page = (items: NeedsYouItem[], nextCursor: string | null, total: number): NeedsYouPage => ({ items, total, nextCursor, unavailable: [] });
  const shown = (list: { items: () => NeedsYouItem[] }) => list.items().map(q => q.sourceId);

  it('Show more during a reload does nothing, so the fresh list is never thrown away (S01d)', async () => {
    const { calls, fetchPage } = deferred();
    const list = createNeedsYouList(fetchPage, 30);
    const first = list.load();
    calls[0].answer(page([question(1)], 'question:q-1', 3));
    await first;
    // A bulk accept reloads, and Show more is clicked before the reload answers.
    const reload = list.load();
    expect(list.reloading()).toBe(true);
    const more = list.load(true);
    expect(calls).toHaveLength(2);
    calls[1].answer(page([question(2)], 'question:q-2', 2));
    calls[2]?.answer(page([question(3)], null, 3));
    await Promise.all([reload, more]);
    // Before: the reload was dropped and q-3 appended to the old list.
    expect(shown(list)).toEqual(['q-2']);
    expect(list.total()).toBe(2);
    expect(list.next()).toBe('question:q-2');
    expect(list.reloading()).toBe(false);
  });

  it('a reload during Show more drops its page; Show more after the reload appends to the fresh list (S01d)', async () => {
    const { calls, fetchPage } = deferred();
    const list = createNeedsYouList(fetchPage, 30);
    const first = list.load();
    calls[0].answer(page([question(1)], 'question:q-1', 3));
    await first;
    const more = list.load(true);
    expect(calls[1].cursor).toBe('question:q-1');
    // A second click while the page is out asks for nothing (same cursor, same page).
    await list.load(true);
    expect(calls).toHaveLength(2);
    const reload = list.load();
    calls[2].answer(page([question(5)], 'question:q-5', 2));
    await reload;
    // The page answers after the reload: it followed the old list, so it is dropped.
    calls[1].answer(page([question(2)], null, 3));
    await more;
    expect(shown(list)).toEqual(['q-5']);
    expect(list.next()).toBe('question:q-5');
    expect(list.total()).toBe(2);
    const again = list.load(true);
    expect(calls[3].cursor).toBe('question:q-5');
    calls[3].answer(page([question(6)], null, 2));
    await again;
    expect(shown(list)).toEqual(['q-5', 'q-6']);
    expect(list.next()).toBeNull();
  });

  it('a reload overtaken by a newer one, or one that fails, never leaves Show more stuck (S01d)', async () => {
    const { calls, fetchPage } = deferred();
    const list = createNeedsYouList(fetchPage, 30);
    const first = list.load();
    calls[0].answer(page([question(1)], 'question:q-1', 3));
    await first;
    const older = list.load();
    const newer = list.load();
    calls[1].answer(page([question(9)], null, 1));
    await older;
    // The newer reload is still out.
    expect(list.reloading()).toBe(true);
    calls[2].fail(new Error('gone'));
    await newer;
    expect(list.reloading()).toBe(false);
    expect(list.failed()).toBe(true);
    expect(shown(list)).toEqual(['q-1']);
    const more = list.load(true);
    expect(calls[3].cursor).toBe('question:q-1');
    calls[3].answer(page([question(2)], null, 2));
    await more;
    expect(shown(list)).toEqual(['q-1', 'q-2']);
  });

  it('names the sources the last answer could not read, and clears the failure on success', async () => {
    let fail = true;
    const list = createNeedsYouList(async () => {
      if (fail) throw new Error('down');
      return { items: [question(1)], total: 1, nextCursor: null, unavailable: ['questions', 'Notion sync'] };
    }, 30);
    await list.load();
    expect(list.failed()).toBe(true);
    fail = false;
    await list.load();
    expect(list.failed()).toBe(false);
    expect(list.unavailable()).toEqual(['questions', 'Notion sync']);
  });
});
