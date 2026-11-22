import { describe, expect, it } from 'vitest';
import { blocksOf, firstSentence, linksOf, listItemOf, plainText, readSections, sha256Hex, sourceLines } from './sectionMarkdown';

describe('section markdown helpers (Canvas C5a)', () => {
  it('hashes UTF-8 exactly like sha256sum, across block boundaries', () => {
    // Expected values from `printf … | shasum -a 256`.
    expect(sha256Hex('')).toBe('e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855');
    expect(sha256Hex('abc')).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad');
    expect(sha256Hex('0'.repeat(55))).toBe('9f8ef876f51f5313c91cc3f6b8119af09d8bbdd72098fa149b2780eb3591d6be');
    expect(sha256Hex('0'.repeat(56))).toBe('bd03ac1428f0ea86f4b83a731ffc7967bb82866d8545322f888d2f6e857ffc18');
    expect(sha256Hex('0'.repeat(64))).toBe('60e05bd1b195af2f94112fa7197a5c88289058840ce7c6df9693756bc6250f55');
    expect(sha256Hex('- [ ] Café → 日本 🎉')).toBe('d4a964b0acd9be2fc97ce637d6a9ceb54baf57b129b36cafad4c4d9f7a6dcdfd');
  });

  it('numbers lines like an editor and drops CR, BOM and the final newline', () => {
    expect(sourceLines('')).toEqual([]);
    expect(sourceLines('\n')).toEqual([]);
    expect(sourceLines('﻿a\r\nb\r\n').map(l => [l.n, l.text])).toEqual([[1, 'a'], [2, 'b']]);
    expect(sourceLines('a\n\nb').map(l => l.text)).toEqual(['a', '', 'b']);
  });

  it('tracks fences the CommonMark way', () => {
    const fenced = (t: string) => sourceLines(t).map(l => l.fenced);
    // A tilde line doesn't close a backtick fence, and a shorter one doesn't either.
    expect(fenced('````\n~~~\n```\n````\nx')).toEqual([true, true, true, true, false]);
    // An inline ``` with a backtick in its info string isn't a fence.
    expect(fenced('``` a`b\nx')).toEqual([false, false]);
    // An unclosed fence runs to the end.
    expect(fenced('x\n```\n## not a heading')).toEqual([false, true, true]);
  });

  it('splits at # and ## headings outside code, frontmatter apart', () => {
    const doc = readSections('---\ntitle: x\n---\n\n# T\nlead\n\n## A\n### sub\n```\n## fenced\n```\n\n## B ##\n');
    expect(doc.frontmatter).toEqual({ text: 'title: x', line: 1, endLine: 3 });
    expect(doc.sections.map(s => [s.heading, s.level, s.line, s.endLine])).toEqual([['T', 1, 5, 6], ['A', 2, 8, 12], ['B', 2, 14, 14]]);
    expect(readSections('---\nnot closed').frontmatter).toBeNull();
    expect(readSections('just text').sections.map(s => [s.heading, s.level, s.line])).toEqual([[null, 0, 1]]);
  });

  it('keeps a fenced block with blank lines as one block', () => {
    expect(blocksOf(sourceLines('a\n\n```\n1\n\n2\n```\n\nb')).map(b => [b.line, b.endLine])).toEqual([[1, 1], [3, 7], [9, 9]]);
  });

  it('reads list items but not thematic breaks', () => {
    expect(listItemOf({ n: 1, text: '\t- [ ] x', fenced: false })).toEqual({ indent: 4, marker: '-', rest: '[ ] x' });
    expect(listItemOf({ n: 1, text: '12) y', fenced: false })?.marker).toBe('12)');
    expect(listItemOf({ n: 1, text: '***', fenced: false })).toBeNull();
    expect(listItemOf({ n: 1, text: '- - -', fenced: false })).toBeNull();
    expect(listItemOf({ n: 1, text: '- x', fenced: true })).toBeNull();
  });

  it('reduces markdown to a headline', () => {
    expect(plainText('**Done** in [#351](https://x/351) at `ed6d264`, *really*.')).toBe('Done in #351 at ed6d264, really.');
    expect(plainText('snake_case_name and 2*3*4')).toBe('snake_case_name and 2*3*4');
    expect(firstSentence('Wave 1 is in. Wave 2 runs.')).toBe('Wave 1 is in.');
    expect(firstSentence('v0.5.9 shipped e.g. today')).toBe('v0.5.9 shipped e.g. today');
    expect(linksOf('[a](x.md) ![img](y.png) [b](<z w.md>)')).toEqual([{ text: 'a', href: 'x.md' }, { text: 'b', href: 'z w.md' }]);
  });
});
