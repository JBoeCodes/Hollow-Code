import { describe, expect, it } from 'vitest';
import { fileMention, mentionAt, mentionedFiles, replaceMention } from './composerMentions';

describe('composer mentions', () => {
  it('finds only the token at the caret, including mid-draft edits', () => {
    expect(mentionAt('Read @plan then continue', 10)).toEqual({ kind: '@', query: 'plan', from: 5, to: 10 });
    expect(mentionAt('Use $humanizer', 14)?.kind).toBe('$');
    expect(mentionAt('a@b.com', 7)).toBeNull();
    expect(mentionAt('cost$50', 7)).toBeNull();
  });
  it('preserves both sides of a completion and returns the new caret', () => {
    const text = 'Read @pl then continue';
    const result = replaceMention(text, mentionAt(text, 8)!, 'PLAN.md');
    expect(result).toEqual({ text: 'Read PLAN.md  then continue', caret: 13 });
  });
  it('round trips spaces, brackets, percent signs and angle brackets in file chips', () => {
    const name = 'notes [final].md';
    const path = '/work/a > b/100%/notes [final]#?.md';
    const token = fileMention(name, path);
    expect(mentionedFiles('Read ' + token)).toEqual([{ name, path, from: 5, to: 5 + token.length }]);
    expect(mentionedFiles('[@bad](<%zz>)')).toEqual([]);
  });
});
