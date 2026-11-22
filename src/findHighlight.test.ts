import { describe, expect, it } from 'vitest';
import { FIND_TEXT, MAX_MARKS, clearFindMarks, matchOffsets, paintFindMarks, sameHits } from './findHighlight';
import CHAT_BODY from './components/ChatBody.tsx?raw';

describe('find marks the words it matched (overnight 11-04, C4)', () => {
  it('finds every non-overlapping match, ignoring case', () => {
    expect(matchOffsets('Section 3 and section 3b', 'section 3')).toEqual([0, 14]);
    expect(matchOffsets('aaaa', 'aa')).toEqual([0, 2]);
    expect(matchOffsets('Overtime', ' overtime ')).toEqual([0]);
  });
  it('needs two characters, as the find bar does', () => {
    expect(matchOffsets('a b c', 'a')).toEqual([]);
    expect(matchOffsets('a b c', '  ')).toEqual([]);
    expect(matchOffsets('', 'ab')).toEqual([]);
    expect(matchOffsets('a', 'ab')).toEqual([]);
  });
  it('marks nothing where lower-casing changes the length', () => {
    // "İ" lower-cases to two code units, so offsets would drift.
    expect(matchOffsets('İstanbul İstanbul', 'stanbul')).toEqual([]);
    expect(matchOffsets('istanbul', 'İstanbul')).toEqual([]);
  });
  it('is a no-op without the Custom Highlight API (node, older WebKit)', () => {
    expect(paintFindMarks(null, 'section', 0)).toBe(0);
    expect(() => clearFindMarks()).not.toThrow();
  });
  it('walks only the containers that hold searched text, and is bounded', () => {
    for (const s of ['.md-view', '.chat-codeblock-pre', '.chat-user-chip .text']) expect(FIND_TEXT).toContain(s);
    expect(FIND_TEXT).not.toMatch(/footer|label|button/);
    expect(MAX_MARKS).toBeGreaterThan(100);
  });
  it('compares hit lists by value, so a streamed token is no new list', () => {
    expect(sameHits([1, 4, 9], [1, 4, 9])).toBe(true);
    expect(sameHits([1, 4], [1, 4, 9])).toBe(false);
    expect(sameHits([1, 4], [4, 1])).toBe(false);
    expect(sameHits([], [])).toBe(true);
    expect(CHAT_BODY).toContain('{ equals: sameHits }');
  });
  it('is painted while the find bar is open and cleared when it closes', () => {
    expect(CHAT_BODY).toContain('paintFindMarks(');
    expect(CHAT_BODY).toContain('clearFindMarks()');
  });
});
