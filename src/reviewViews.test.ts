import { describe, expect, it } from 'vitest';
import type { NeedsYouItem } from './workspaceTypes';
import { REVIEW_VIEW_KINDS, reviewEmptyText, reviewTabCounts, reviewViewCount, reviewViewKinds, type WaitingDraft } from './waitingOnYou';

/** P0-09: each Review view asks the query for its own kinds, and every tab
 *  is counted by one rule. */

let n = 0;
const item = (kind: NeedsYouItem['kind'], state = 'pending'): NeedsYouItem => ({
  id: `${kind}:${++n}`, kind, sourceId: String(n), title: `${kind} ${n}`, detail: null, state,
  at: `2026-10-${String(10 + (n % 10)).padStart(2, '0')}T09:00:00Z`, workspaceId: null, sessionKey: null, path: null, category: null, link: null,
});

const fixture: NeedsYouItem[] = [
  item('interrupted'), item('approval'), item('approval'), item('approval'), item('question'), item('question', 'answered'),
  item('canvas'), item('review'), item('conflict'), item('copies'), item('result'), item('result'), item('failure'), item('failure'),
];
const drafts: WaitingDraft[] = [{ path: 'notes/a.md', savedAt: '2026-10-22T10:00:00Z' }, { path: 'notes/b.md', savedAt: '2026-10-22T11:00:00Z' }];
/** What the query answers for a view: its kinds, filtered before paging. */
const pageFor = (kinds: readonly NeedsYouItem['kind'][] | undefined) => {
  const items = fixture.filter(i => !kinds || kinds.includes(i.kind));
  return { total: items.length, items };
};

describe('Review views (P0-09)', () => {
  it('splits every kind into exactly one of Decisions, Results and Failures', () => {
    expect(REVIEW_VIEW_KINDS.decisions).toEqual(['interrupted', 'approval', 'question', 'canvas', 'review', 'conflict', 'copies', 'lesson_memory']);
    expect(REVIEW_VIEW_KINDS.results).toEqual(['result']);
    expect(REVIEW_VIEW_KINDS.failures).toEqual(['failure']);
    const kinds = [...REVIEW_VIEW_KINDS.decisions, ...REVIEW_VIEW_KINDS.results, ...REVIEW_VIEW_KINDS.failures];
    expect(new Set(kinds).size).toBe(kinds.length);
    for (const i of fixture) expect(kinds).toContain(i.kind);
  });

  it('asks the query for the view’s kinds, or review items for a page filter', () => {
    expect(reviewViewKinds('all', null)).toBeUndefined();
    expect(reviewViewKinds('failures', null)).toEqual(['failure']);
    expect(reviewViewKinds('results', 'tasks')).toEqual(['review']);
  });

  it('counts Decisions + Results + Failures as All, and drafts in All only (P1-14, D3)', () => {
    const count = (v: 'all' | 'decisions' | 'results' | 'failures') => reviewViewCount(v, pageFor(reviewViewKinds(v, null)), drafts);
    // 14 items, one an answered question (not actionable), plus 2 drafts.
    expect(count('all')).toBe(15);
    expect(count('decisions')).toBe(9);
    expect(count('results')).toBe(2);
    expect(count('failures')).toBe(2);
    expect(count('decisions') + count('results') + count('failures') + drafts.length).toBe(count('all'));
  });

  it('shows no tab count until its view has loaded, and All as the sum once all three have (review)', () => {
    // Before the narrower lists land: no false 0, and All is the row's count plus drafts.
    expect(reviewTabCounts(13, { decisions: undefined, results: 2, failures: undefined }, 2)).toEqual({ all: 15, decisions: undefined, results: 2, failures: undefined });
    // Loaded: All is their sum plus drafts, even when an unanswerable
    // question sits past All's first page (the row would still count it).
    expect(reviewTabCounts(14, { decisions: 9, results: 2, failures: 2 }, 2)).toEqual({ all: 15, decisions: 9, results: 2, failures: 2 });
    // The row's count unknown (a failed read) and a view not loaded: no All count either (P1-14).
    expect(reviewTabCounts(null, { decisions: undefined, results: 2, failures: undefined }, 2)).toEqual({ all: undefined, decisions: undefined, results: 2, failures: undefined });
  });

  it('names the view when it is empty', () => {
    expect(reviewEmptyText('failures', null, false)).toBe('No failures waiting.');
    expect(reviewEmptyText('results', null, false)).toBe('No results waiting.');
    expect(reviewEmptyText('decisions', null, false)).toBe('No decisions waiting.');
    expect(reviewEmptyText('all', null, false)).toBe('Nothing is waiting on you.');
    expect(reviewEmptyText('results', null, true)).toBe('Nothing waiting in the sources that loaded.');
    expect(reviewEmptyText('all', 'tasks', false)).toBe('No task suggestions are waiting.');
  });
});
