import { describe, expect, it } from 'vitest';
import type { AnnotationSet } from './types';
import { annotationText, type Mark } from './annotate';
import {
  DRAFTS_KEY, MAX_DRAFTS, NO_ANSWER_MS, captureLabel, captureSentence, captureTime, followsPageUrl, latestLink, layerCovers, loadDrafts, lruVictim, nextActive,
  noAnswerMessage, progressLabel, removeDraft, requestedChanges, reviewProgress, saveDraft, stoppedTab, tabAfterEvent, tabsInScope, unanswered,
  type AnnotationDraft,
} from './browserReview';

const memory = (limit = Infinity) => {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { if (v.length > limit) throw new Error('QuotaExceededError'); map.set(k, v); },
    removeItem: (k: string) => void map.delete(k),
  };
};

const now = new Date(2026, 9, 1, 15, 0);
const at = (h: number, m: number, day = 1) => new Date(2026, 9, day, h, m).toISOString();

describe('captures', () => {
  it('labels a snapshot as a capture with its time and viewport', () => {
    expect(captureLabel(at(14, 2), { width: 1280, height: 720 }, now)).toMatch(/^Capture · 14:02 · 1280×720$/);
    // Another day names the day.
    expect(captureTime(at(9, 5, 28), now)).toMatch(/9.*09:05|Sep.*09:05|28.*09:05/);
    expect(captureSentence(at(14, 2), { width: 800.4, height: 600 }, now)).toBe('Captured 14:02 at a 800×600 viewport (a snapshot, not the live page).');
    expect(captureLabel('not a time', { width: 10, height: 10 }, now)).toBe('Capture · 10×10');
  });
  it('tells the chat it was a capture', () => {
    const text = annotationText({ url: 'http://localhost:5173/', title: 'Home' }, [], null, 'Captured 14:02 at a 1280×720 viewport (a snapshot, not the live page).');
    expect(text).toBe('I marked up “Home” (http://localhost:5173/); the annotated screenshot is attached. Captured 14:02 at a 1280×720 viewport (a snapshot, not the live page).');
  });
});

describe('tabs by workspace', () => {
  const tabs = [{ id: 'a', scope: 'w1' }, { id: 'b', scope: 'w2' }, { id: 'c', scope: 'w1' }, { id: 'd', scope: 'chat' }];
  it('lists only the scope’s tabs and picks the neighbour on close', () => {
    expect(tabsInScope(tabs, 'w1').map(t => t.id)).toEqual(['a', 'c']);
    expect(tabsInScope(tabs, 'w3')).toEqual([]);
    const rest = tabs.filter(t => t.id !== 'a');
    expect(nextActive(rest, 'w1', 0)).toBe('c');
    expect(nextActive(rest.filter(t => t.id !== 'c'), 'w1', 0)).toBeNull();
    // Never a tab of another workspace.
    expect(nextActive(rest, 'w2', 3)).toBe('b');
  });
});

describe('annotation drafts', () => {
  const draft = (over: Partial<AnnotationDraft>): AnnotationDraft => ({
    scope: 'w1', tabId: 't1', url: 'http://localhost:5173/', title: 'Home', capturedAt: at(14, 0), view: { width: 1280, height: 720 },
    imageSize: { width: 2560, height: 1440 }, image: 'data:image/png;base64,AAAA', marks: [{ kind: 'pin', x: 1, y: 2, note: 'here' }], savedAt: at(14, 1), ...over,
  });
  it('keeps the newest per capture and scope, bounded, and forgets on remove', () => {
    const store = memory();
    saveDraft(store, draft({}));
    saveDraft(store, draft({ savedAt: at(14, 3), marks: [] }));
    expect(loadDrafts(store, 'w1')).toHaveLength(1);
    expect(loadDrafts(store, 'w1')[0].marks).toEqual([]);
    for (let i = 0; i < MAX_DRAFTS + 2; i++) saveDraft(store, draft({ tabId: `t${i + 2}`, savedAt: at(14, 10 + i) }));
    expect(loadDrafts(store, 'w1')).toHaveLength(MAX_DRAFTS);
    expect(loadDrafts(store, 'w1')[0].tabId).toBe(`t${MAX_DRAFTS + 3}`);
    saveDraft(store, draft({ scope: 'w2' }));
    expect(loadDrafts(store, 'w2')).toHaveLength(1);
    removeDraft(store, draft({ scope: 'w2' }));
    expect(loadDrafts(store, 'w2')).toHaveLength(0);
    expect(loadDrafts(store, 'w1')).toHaveLength(MAX_DRAFTS);
  });
  it('drops older drafts when storage is full, and reports a draft that cannot fit', () => {
    const one = JSON.stringify([draft({})]).length;
    const store = memory(one + 50);
    expect(saveDraft(store, draft({}))).toBe(true);
    expect(saveDraft(store, draft({ tabId: 't9', savedAt: at(14, 20) }))).toBe(true);
    expect(loadDrafts(store, 'w1').map(d => d.tabId)).toEqual(['t9']);
    expect(saveDraft(memory(10), draft({}))).toBe(false);
  });
  it('ignores a corrupt store', () => {
    const store = memory();
    store.setItem(DRAFTS_KEY, '{oops');
    expect(loadDrafts(store, 'w1')).toEqual([]);
  });
});

describe('before / after', () => {
  const marks: Mark[] = [{ kind: 'pin', x: 1, y: 1, note: 'Blue' }, { kind: 'box', x: 0, y: 0, w: 4, h: 4, note: 'Gap' }, { kind: 'pin', x: 2, y: 2, note: '' }];
  const cap = (id: string, m: Mark[] = []) => ({ id, url: 'http://x.test/', title: '', capturedAt: at(14, 0), viewport: { width: 10, height: 10 }, imageSize: { width: 20, height: 20 }, image: `${id}.png`, marks: m, parent: null, imageDataUrl: null });
  const set: Pick<AnnotationSet, 'captures' | 'links'> = {
    captures: [cap('c1', marks), cap('c2'), cap('c3')],
    links: [
      { capture: 'c1', mark: 0, result: 'c2', status: 'notDone', note: '', at: at(14, 5) },
      { capture: 'c1', mark: 1, result: 'c2', status: 'done', note: '', at: at(14, 5) },
      { capture: 'c1', mark: 0, result: 'c3', status: 'done', note: 'now blue', at: at(14, 9) },
    ],
  };
  it('links each requested change to the newest capture that shows its result', () => {
    expect(latestLink(set, 'c1', 0)?.result).toBe('c3');
    const changes = requestedChanges(set);
    expect(changes.map(c => [c.mark.note, c.link?.status ?? null, c.link?.result ?? null])).toEqual([['Blue', 'done', 'c3'], ['Gap', 'done', 'c2'], ['', null, null]]);
    expect(reviewProgress(set)).toEqual({ requested: 3, done: 2, open: 1 });
    expect(progressLabel(reviewProgress(set))).toBe('2 of 3 changes done');
    expect(progressLabel({ requested: 0, done: 0 })).toBe('No changes requested');
    expect(requestedChanges({ captures: [], links: [] })).toEqual([]);
  });
});

describe('tabs that work (run 15 P2)', () => {
  it('makes room at the cap by closing the tab shown longest ago, never the one on show', () => {
    const tabs = [{ id: 'a', usedAt: 30 }, { id: 'b', usedAt: 10 }, { id: 'c', usedAt: 20 }, { id: 'd', usedAt: 10 }];
    expect(lruVictim(tabs, 'c')).toBe('b');
    expect(lruVictim(tabs, 'b')).toBe('d');
    expect(lruVictim([{ id: 'a', usedAt: 1 }], 'a')).toBeNull();
    expect(lruVictim([], null)).toBeNull();
  });
  it('counts a page as answered only once a load event arrives', () => {
    expect(tabAfterEvent({}, 'loading', 'http://localhost:5173/', 50)).toEqual({ loading: true, committed: true, answeredAt: 50, error: null, timedOut: false, url: 'http://localhost:5173/' });
    expect(tabAfterEvent({ error: null }, 'loaded', undefined, 60)).toEqual({ loading: false, committed: true, answeredAt: 60, error: null, timedOut: false });
  });
  it('lets a late page clear only the time limit’s error', () => {
    expect(tabAfterEvent({ error: 'x didn’t answer', timedOut: true }, 'loaded', 'https://x.test/', 70)).toMatchObject({ error: null, url: 'https://x.test/' });
    // A refused address (or a Stop) keeps its error and its address: the event is the old page's.
    expect(tabAfterEvent({ error: 'Nothing is answering at localhost:5173.' }, 'loaded', 'https://a.test/', 70)).toEqual({ committed: true, loading: false });
    // While a newer load's command is pending, the old page's events change nothing.
    expect(tabAfterEvent({ opening: true }, 'loaded', 'https://a.test/', 80)).toEqual({});
  });
  it('keeps the asked-for address while its load waits or after it failed', () => {
    const tab = { loading: true, openedAt: 100, answeredAt: 50, error: null };
    expect(followsPageUrl(tab)).toBe(false);
    expect(followsPageUrl({ ...tab, answeredAt: 150 })).toBe(true);
    expect(followsPageUrl({ ...tab, loading: false })).toBe(true);
    expect(followsPageUrl({ ...tab, loading: false, error: 'Stopped before b.test answered.' })).toBe(false);
  });
  it('fails a load nothing answered, and only that load', () => {
    const tab = { loading: true, openedAt: 1_000, answeredAt: 0 };
    expect(unanswered(tab, 1_000 + NO_ANSWER_MS - 1, NO_ANSWER_MS)).toBe(false);
    expect(unanswered(tab, 1_000 + NO_ANSWER_MS, NO_ANSWER_MS)).toBe(true);
    expect(unanswered({ ...tab, answeredAt: 1_500 }, 99_000, NO_ANSWER_MS)).toBe(false);
    expect(unanswered({ ...tab, loading: false }, 99_000, NO_ANSWER_MS)).toBe(false);
    expect(noAnswerMessage('https://intranet.example/x')).toBe('intranet.example didn’t answer in 20 seconds. Check the address, or open it in your browser to see why.');
  });
  it('lets Stop end a load: a page that never answered says so', () => {
    expect(stoppedTab({ committed: true, url: 'https://example.com/' })).toEqual({ loading: false });
    expect(stoppedTab({ committed: false, url: 'https://example.com/' })).toEqual({ loading: false, error: 'Stopped before example.com answered.' });
  });
  it('hides the native page under any floating layer that overlaps it, however small', () => {
    const page = { left: 600, top: 100, right: 1000, bottom: 800 };
    // A 24 px tooltip between the old sample points still counts.
    expect(layerCovers(page, [{ left: 700, top: 130, right: 724, bottom: 154 }])).toBe(true);
    expect(layerCovers(page, [{ left: 200, top: 100, right: 600, bottom: 300 }]), 'touching the edge').toBe(false);
    expect(layerCovers(page, [{ left: 650, top: 200, right: 650, bottom: 260 }]), 'no area').toBe(false);
    expect(layerCovers(page, [])).toBe(false);
  });
});
