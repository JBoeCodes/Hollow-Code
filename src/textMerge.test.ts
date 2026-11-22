import { describe, expect, it } from 'vitest';
import { diffLines, diffStats, hasConflictMarkers, merge3, withEol, MARK_MINE, MARK_THEIRS } from './textMerge';

const L = (...lines: string[]) => lines.join('\n');

describe('line diff (next-level P08)', () => {
  it('finds a minimal edit', () => {
    const ops = diffLines(['a', 'b', 'c'], ['a', 'x', 'c']);
    expect(ops).toEqual([{ kind: 'same', text: 'a' }, { kind: 'del', text: 'b' }, { kind: 'add', text: 'x' }, { kind: 'same', text: 'c' }]);
    expect(diffStats(ops)).toEqual({ add: 1, del: 1 });
  });
});

describe('three-way merge (next-level P08)', () => {
  const base = L('# Review', '', 'Section 1', 'Section 2', 'Section 3');
  it('keeps non-overlapping edits from both sides', () => {
    const theirs = L('# Review', '', 'Section 1 (finance)', 'Section 2', 'Section 3');
    const mine = L('# Review', '', 'Section 1', 'Section 2', 'Section 3 revised');
    const r = merge3(base, theirs, mine);
    expect(r.conflicts).toBe(0);
    expect(r.text).toBe(L('# Review', '', 'Section 1 (finance)', 'Section 2', 'Section 3 revised'));
  });
  it('marks overlapping edits instead of choosing a side', () => {
    const theirs = L('# Review', '', 'Section 1', 'Section 2 from disk', 'Section 3');
    const mine = L('# Review', '', 'Section 1', 'Section 2 from draft', 'Section 3');
    const r = merge3(base, theirs, mine);
    expect(r.conflicts).toBe(1);
    expect(r.text).toContain(MARK_MINE);
    expect(r.text).toContain('Section 2 from draft');
    expect(r.text).toContain('Section 2 from disk');
    expect(r.text).toContain(MARK_THEIRS);
    expect(hasConflictMarkers(r.text)).toBe(true);
    expect(hasConflictMarkers(L('a', 'b'))).toBe(false);
  });
  it('treats identical changes on both sides as clean', () => {
    const both = L('# Review', '', 'Section 1', 'Same fix', 'Section 3');
    expect(merge3(base, both, both)).toMatchObject({ conflicts: 0, text: both });
  });
  it('handles insertions at the end from both sides as a conflict', () => {
    const r = merge3(L('a'), L('a', 'disk'), L('a', 'mine'));
    expect(r.conflicts).toBe(1);
  });
  it('an unchanged draft takes the disk version cleanly', () => {
    const theirs = L('# Review', 'new');
    expect(merge3(base, theirs, base)).toMatchObject({ conflicts: 0, text: theirs });
  });
  it('a deleted file region and an untouched draft merge to the deletion', () => {
    const theirs = L('# Review', '', 'Section 1', 'Section 3');
    expect(merge3(base, theirs, base).text).toBe(theirs);
  });
});

describe('merges keep the file\'s line endings (review A, slice 4 item 1d)', () => {
  const C = (...lines: string[]) => lines.join('\r\n');
  it('a clean merge of a CRLF file stays CRLF', () => {
    const base = C('# Notes', 'one', 'two', 'three', '');
    const r = merge3(base, C('# Notes', 'one (disk)', 'two', 'three', ''), C('# Notes', 'one', 'two', 'three (mine)', ''));
    expect(r.conflicts).toBe(0);
    expect(r.eol).toBe('\r\n');
    expect(r.text).toBe(C('# Notes', 'one (disk)', 'two', 'three (mine)', ''));
  });
  it('conflict markers in a CRLF file use CRLF too', () => {
    const r = merge3(C('a', 'b'), C('a', 'disk'), C('a', 'mine'));
    expect(r.conflicts).toBe(1);
    expect(r.text).toBe(C('a', MARK_MINE, 'mine', '=======', 'disk', MARK_THEIRS));
    expect(r.text.replace(/\r\n/g, '')).not.toContain('\n');
  });
  it('follows the file on disk when the draft came back with LF', () => {
    const r = merge3(C('a', 'b', 'c'), C('a', 'b', 'c (disk)'), L('a (mine)', 'b', 'c'));
    expect(r.text).toBe(C('a (mine)', 'b', 'c (disk)'));
  });
  it('an LF file stays LF', () => {
    const r = merge3(L('a', 'b'), L('a', 'b', 'disk'), L('mine', 'b'));
    expect(r.eol).toBe('\n');
    expect(r.text).toBe(L('mine', 'b', 'disk'));
  });
  it('a hand-edited merge (a textarea hands back LF) is put back in the file\'s endings', () => {
    expect(withEol('a\nb\r\nc\n', '\r\n')).toBe('a\r\nb\r\nc\r\n');
    expect(withEol('a\r\nb\n', '\n')).toBe('a\nb\n');
  });
});
