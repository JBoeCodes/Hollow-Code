import { describe, expect, it } from 'vitest';
import { QUOTE_LIMIT, quoteMarkdown } from './selectionQuote';

describe('quoteMarkdown (D1 gap 9)', () => {
  it('quotes one line and leaves a blank line under it for the caret', () => {
    expect(quoteMarkdown('Use the staging key.')).toBe('> Use the staging key.\n\n');
  });

  it('quotes every line, keeps one bare ">" for blank lines and folds a run of them', () => {
    expect(quoteMarkdown('First line\nSecond line\n\n\n\nNew paragraph')).toBe('> First line\n> Second line\n>\n> New paragraph\n\n');
  });

  it('trims the selection and each line, and turns CRLF and no-break spaces into plain ones', () => {
    expect(quoteMarkdown('  \r\nA\xa0B   \r\nC  \n')).toBe('> A B\n> C\n\n');
  });

  it('returns nothing for a blank selection', () => {
    expect(quoteMarkdown('')).toBe('');
    expect(quoteMarkdown(' \n\xa0 \n')).toBe('');
  });

  it('cuts a long selection at the limit with an ellipsis', () => {
    const quote = quoteMarkdown('word '.repeat(2000));
    expect(quote.endsWith('…\n\n')).toBe(true);
    expect(quote.length).toBeLessThanOrEqual(QUOTE_LIMIT + 8);
    expect(quoteMarkdown('short', 10)).toBe('> short\n\n');
    expect(quoteMarkdown('0123456789abc', 10)).toBe('> 0123456789…\n\n');
  });

  it('never cuts an emoji in half: a cut between its two code units backs up one', () => {
    const smile = String.fromCodePoint(0x1f600);
    // The limit lands after the high half: the pair is dropped whole, no lone surrogate before the ellipsis.
    expect(quoteMarkdown(`${'a'.repeat(9)}${smile}tail`, 10)).toBe(`> ${'a'.repeat(9)}…\n\n`);
    // The limit lands after the whole pair: it stays.
    expect(quoteMarkdown(`${'a'.repeat(8)}${smile}tail`, 10)).toBe(`> ${'a'.repeat(8)}${smile}…\n\n`);
  });
});

describe('the Quote bar follows inner scrollers (ENG-93)', () => {
  it('listens for scroll in capture, so a wide table or code box moving sideways moves the bar', async () => {
    const source = (await import('./components/QuoteSelection.tsx?raw')).default;
    expect(source).toContain("el.addEventListener('scroll', follow, { passive: true, capture: true });");
    expect(source).toContain("el.removeEventListener('scroll', follow, { capture: true });");
  });
});
