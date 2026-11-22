import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { draftConflictKeys, readTaskDraft, rebaseTaskDraft, sameDraftValue, taskDraftKey, writeTaskDraft } from './taskDrafts';
import type { Task } from './types';

beforeEach(() => {
  const data = new Map<string, string>();
  vi.stubGlobal('sessionStorage', { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => data.set(k, v), removeItem: (k: string) => data.delete(k) });
});
afterEach(() => vi.unstubAllGlobals());

describe('task drafts', () => {
  it('compares date objects by value rather than insertion order', () => {
    expect(sameDraftValue({start:'2026-11-01',end:null,time_zone:null},{time_zone:null,end:null,start:'2026-11-01'})).toBe(true);
  });
  it('shares vault URL aliases and retains local fields when a mapped save clears its section', () => {
    expect(taskDraftKey('vault://tasks/a.md')).toBe(taskDraftKey('vault:tasks/a.md'));
    const local = { base: { labels: [] }, draft: { labels: ['kept'] }, hash: 'old' };
    writeTaskDraft('vault://tasks/a.md', 'local', local);
    writeTaskDraft('tasks/a.md', 'mapped', { base: { title: 'old' }, draft: { title: 'new' }, id: '1' });
    writeTaskDraft('vault:tasks/a.md', 'mapped', null);
    expect(readTaskDraft('tasks/a.md', 'local')).toEqual(local);
  });
  it('retains disjoint draft edits, follows current untouched values and requires overlap choices', () => {
    const base = { title: 'Original', due: 'Monday', labels: [] };
    const mine = { ...base, title: 'Mine', labels: ['draft'] };
    const current = { ...base, title: 'Current', due: 'Tuesday' };
    expect(draftConflictKeys(base, mine, current)).toEqual(['title']);
    expect(rebaseTaskDraft(base, mine, current, { title: 'mine' })).toEqual({ title: 'Mine', due: 'Tuesday', labels: ['draft'] });
    expect(rebaseTaskDraft(base, mine, current, { title: 'current' }).title).toBe('Current');
  });
  it('only carries the current mapped schema, including newly added fields', () => {
    expect(rebaseTaskDraft({ removed: 'old' }, { removed: 'draft' }, { added: 'current' }, {})).toEqual({ added: 'current' });
  });
});
