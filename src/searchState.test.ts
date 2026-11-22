import { describe, expect, it } from 'vitest';
import { beginSearch, searchEmptyText, visibleResults, type SearchState } from './searchState';

describe('async search states (audit H09)', () => {
  it('a pending search is never a completed empty result', () => {
    const s = beginSearch<string>({ status: 'idle' }, 'budget review');
    expect(s.status).toBe('pending');
    expect(visibleResults(s)).toBeNull();
    expect(searchEmptyText(s, 'meaning')?.title).toBe('Searching by meaning…');
    expect(searchEmptyText(s, 'meaning')?.title).not.toMatch(/No matching|Nothing/);
  });
  it('keeps previous results visible while a newer request runs', () => {
    const done: SearchState<string> = { status: 'done', query: 'a', results: ['x.md'] };
    const next = beginSearch(done, 'ab');
    expect(visibleResults(next)).toEqual(['x.md']);
    expect(searchEmptyText(next, 'meaning')).toBeNull();
  });
  it('distinguishes completed-empty, unavailable and failed', () => {
    expect(searchEmptyText({ status: 'done', query: 'q', results: [] }, 'meaning')?.title).toBe('No matching files.');
    expect(searchEmptyText({ status: 'done', query: 'q', results: [] }, 'name')?.title).toBe('Nothing by that name.');
    expect(searchEmptyText({ status: 'unavailable', query: 'q', reason: 'Indexing 40%' }, 'meaning')?.detail).toBe('Indexing 40%');
    const err = searchEmptyText({ status: 'error', query: 'q', message: 'index locked' }, 'meaning');
    expect(err?.role).toBe('alert');
    expect(err?.title).not.toMatch(/No matching/);
  });
});
