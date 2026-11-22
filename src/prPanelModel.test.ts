import { describe, expect, it } from 'vitest';
import { checksSummary, commentDraft, mergeWords, prStateWords, reviewStateWords, reviewWords, safeLink } from './prPanelModel';

const c = (state: string) => ({ name: 'x', state, url: null });

describe('the PR tab words (parity 5.4)', () => {
  it('names the PR state', () => {
    expect(prStateWords({ state: 'open', draft: false })).toBe('Open');
    expect(prStateWords({ state: 'open', draft: true })).toBe('Draft');
    expect(prStateWords({ state: 'merged', draft: false })).toBe('Merged');
    expect(prStateWords({ state: 'closed', draft: true })).toBe('Closed');
  });
  it('summarizes checks and picks the worst state', () => {
    expect(checksSummary([])).toEqual({ words: 'No checks', worst: 'none' });
    expect(checksSummary([c('success'), c('success')])).toEqual({ words: '2 of 2 passed', worst: 'success' });
    expect(checksSummary([c('success'), c('failure'), c('pending'), c('skipped')])).toEqual({ words: '1 of 4 passed, 1 failing, 1 running, 1 skipped', worst: 'failure' });
    expect(checksSummary([c('success'), c('pending')]).worst).toBe('pending');
  });
  it('words the review and merge state, silent for the unremarkable', () => {
    expect(reviewWords('CHANGES_REQUESTED')).toBe('Changes requested');
    expect(reviewWords('')).toBeNull();
    expect(reviewWords(null)).toBeNull();
    expect(mergeWords('DIRTY')).toBe('Has merge conflicts');
    expect(mergeWords('HAS_HOOKS')).toBeNull();
    expect(reviewStateWords('COMMENTED')).toBe('Reviewed');
  });
  it('drafts a quoted comment for the composer', () => {
    expect(commentDraft({ number: 7, title: 'Add it' }, { author: 'ann', kind: 'comment', state: null, body: 'Line one\nLine two' }))
      .toBe("About ann's comment on PR #7 (Add it):\n\n> Line one\n> Line two\n\n");
    expect(commentDraft({ number: 7, title: 'T' }, { author: 'bob', kind: 'review', state: 'CHANGES_REQUESTED', body: 'Fix' })).toContain("bob's review (requested changes)");
  });
  it('opens only https links', () => {
    expect(safeLink('https://github.com/o/r/pull/1')).toBe('https://github.com/o/r/pull/1');
    expect(safeLink('javascript:alert(1)')).toBeNull();
    expect(safeLink('http://x')).toBeNull();
    expect(safeLink(null)).toBeNull();
  });
});
