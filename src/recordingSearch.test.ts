import { expect, it } from 'vitest';
import { transcriptMatchParts } from './recordingSearch';

const marked = (text: string, query: string) => transcriptMatchParts(text, query).filter(p => p.match).map(p => p.text);
it('marks every literal case-insensitive occurrence, preserving original spelling', () => {
  expect(marked('Plan, PLAN and planning', ' plan ')).toEqual(['Plan', 'PLAN', 'plan']);
  expect(marked('Dana', 'dANA')).toEqual(['Dana']);
});
it('never interprets punctuation or HTML as patterns or markup', () => {
  expect(marked('Use [ and .* literally: [ .*', '[')).toEqual(['[', '[']);
  expect(marked('Use [ and .* literally: [ .*', '.*')).toEqual(['.*', '.*']);
  const text = '<img src=x onerror=alert(1)> & "/"';
  expect(transcriptMatchParts(text, 'img').map(p => p.text).join('')).toBe(text);
});
it('keeps whitespace, newlines and unmatched source bytes unchanged', () => {
  for (const query of ['', '  ', 'absent', 'a', 'aa']) {
    const text = '  aaaa\nA\t <b> ';
    expect(transcriptMatchParts(text, query).map(p => p.text).join('')).toBe(text);
  }
  expect(marked('abc', ' ')).toEqual([]);
  expect(marked('aaaa', 'aa')).toEqual(['aa', 'aa']);
});
it('maps lowercase expansions back to intact original Unicode characters', () => {
  expect(marked('İ then PLAN', 'plan')).toEqual(['PLAN']);
  expect(marked('İ then i', 'i')).toEqual(['İ', 'i']);
  expect(marked('ΟΣ then 😀', 'ος')).toEqual(['ΟΣ']);
  expect(marked('😀 twice 😀', '😀')).toEqual(['😀', '😀']);
});
