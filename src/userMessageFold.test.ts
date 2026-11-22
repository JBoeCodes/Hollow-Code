import { describe, expect, it } from 'vitest';
import { FOLDED_LINES, FOLD_CHARS, FOLD_LINES, findKeepsOpen, findShowsWhole, foldsUserText } from './userMessageFold';
import { findInTurns } from './conversationTools';
import { canvasAnswerText } from './canvas/askAnswer';
import CSS from './styles/chat-workspace.css?raw';

const lines = (n: number, eol = '\n') => Array.from({ length: n }, (_, i) => `Line ${i + 1}`).join(eol);

describe('long messages of yours fold behind Show more (overnight 2026-11-04, C3 follow-up)', () => {
  it('folds past twelve lines, not at twelve', () => {
    expect(FOLD_LINES).toBe(12);
    expect(foldsUserText(lines(12))).toBe(false);
    expect(foldsUserText(lines(13))).toBe(true);
    expect(foldsUserText(lines(200))).toBe(true);
    // Blank lines show as lines, so they count.
    expect(foldsUserText(`Intro${'\n'.repeat(12)}End`)).toBe(true);
  });

  it('folds past the character threshold, not at it', () => {
    expect(foldsUserText('a'.repeat(FOLD_CHARS))).toBe(false);
    expect(foldsUserText('a'.repeat(FOLD_CHARS + 1))).toBe(true);
    // A long paragraph with a few line breaks folds by its length.
    const paragraph = 'The rollout plan covers every region and each step of the migration. ';
    expect(foldsUserText(Array.from({ length: 4 }, () => paragraph.repeat(6)).join('\n'))).toBe(true);
    expect(foldsUserText(paragraph.repeat(3))).toBe(false);
  });

  it('folds a long single line, such as a pasted log or URL with no spaces', () => {
    expect(foldsUserText(`https://example.com/${'x'.repeat(2000)}`)).toBe(true);
    expect(foldsUserText('x'.repeat(400))).toBe(false);
  });

  it('counts a CRLF as one line break, as LF', () => {
    expect(foldsUserText(lines(12, '\r\n'))).toBe(false);
    expect(foldsUserText(lines(13, '\r\n'))).toBe(true);
  });

  it('never folds blank text, and does not count trailing blank lines or spaces', () => {
    for (const blank of ['', '   ', '\n\n\n', ' \r\n\t ', null, undefined]) expect(foldsUserText(blank)).toBe(false);
    expect(foldsUserText(`${lines(12)}\n\n\n\n   \n`)).toBe(false);
    expect(foldsUserText(`${lines(12)}\r\n\r\n\r\n`)).toBe(false);
    expect(foldsUserText(`${'a'.repeat(FOLD_CHARS)}${' '.repeat(40)}`)).toBe(false);
  });

  it('shows whole while Find searches, with the floor Find itself uses', () => {
    expect(findShowsWhole(true, 'zebra')).toBe(true);
    expect(findShowsWhole(false, 'zebra')).toBe(false);
    for (const query of ['', ' ', 'z', ' z ', 'ze', '  ze  ']) {
      expect(findShowsWhole(true, query), JSON.stringify(query)).toBe(findInTurns([{ role: 'user', text: 'zebra' }], query).length > 0);
    }
  });

  it('keeps the long message Find was on open as Find closes, and nothing else', () => {
    const turns = [
      { role: 'user', text: lines(40) },
      { role: 'assistant', text: lines(40) },
      { role: 'user', text: 'A short question' },
      { role: 'user', text: lines(13, '\r\n') },
    ];
    expect(findKeepsOpen(turns, [0, 1, 2, 3], 0)).toBe(0);
    expect(findKeepsOpen(turns, [0, 1, 2, 3], 3)).toBe(3);
    // A reply, a short message, or no match at all: nothing to keep open.
    expect(findKeepsOpen(turns, [0, 1, 2, 3], 1)).toBeNull();
    expect(findKeepsOpen(turns, [0, 1, 2, 3], 2)).toBeNull();
    expect(findKeepsOpen(turns, [], 0)).toBeNull();
    expect(findKeepsOpen(turns, [9], 0)).toBeNull();
  });

  it('decides an ask-canvas answer by the lines it shows, not its hidden block', () => {
    const answer = canvasAnswerText({ canvasId: 'release-check', revision: 1, answers: { notes: 'x'.repeat(4000) } }, 'Notes: short');
    expect(answer.length).toBeGreaterThan(FOLD_CHARS);
    expect(findKeepsOpen([{ role: 'user', text: answer }], [0], 0)).toBeNull();
  });

  it('is nine lines tall, clipped so nothing scrolls inside it, faded by a mask, and whole in print', () => {
    expect(FOLDED_LINES).toBe(9);
    expect(FOLDED_LINES).toBeLessThan(FOLD_LINES);
    expect(CSS).toContain(`max-height: ${FOLDED_LINES}lh;`);
    expect(CSS).toMatch(/\.chat-user-clipped \{[^}]*\bmask-image: linear-gradient\(/);
    const rule = CSS.match(/\.chat-user-clipped \{[^}]*\}/)?.[0] ?? '';
    // `clip` (a drag selection or VoiceOver can't scroll it), after `hidden`
    // for an engine without it; the fade in `em` before `lh` for the same reason.
    expect(rule).toMatch(/overflow: hidden; overflow: clip;/);
    expect(rule).toMatch(/[^-]mask-image: linear-gradient\(to bottom, var\(--black\) calc\(100% - 3\.2em\), transparent\); mask-image: linear-gradient\(to bottom, var\(--black\) calc\(100% - 2lh\), transparent\);/);
    const print = CSS.slice(CSS.indexOf('@media print'));
    expect(print).toMatch(/\.chat-user-clipped \{[^}]*max-height: none;/);
    expect(print).toMatch(/\.chat-user-more \{ display: none; \}/);
  });
});
