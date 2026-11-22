import { describe, expect, it } from 'vitest';
import { FILE_STATE_OPTIONS, connectedFileTag, findLine, listCountText } from './connectedFiles';

describe('connected file list and reader (next-level P07)', () => {
  it('tags unread files honestly and leaves readable text plain', () => {
    expect(connectedFileTag({ state: 'text', readable: true })).toBeNull();
    expect(connectedFileTag({ state: 'tooLarge', readable: false })?.label).toBe('too large · not read');
    expect(connectedFileTag({ state: 'needsReader', readable: false })?.label).toBe('needs reader');
    // An older row without a state still reads as needing a reader.
    expect(connectedFileTag({ readable: false } as never)?.label).toBe('needs reader');
  });
  it('R14a: tags a PDF or Office file Hollow reads with its kind, and filters on it', () => {
    expect(connectedFileTag({ state: 'extracted', readable: false, relative: 'Launch/decks/q3-review.pdf' })?.label).toBe('read by Hollow · PDF');
    expect(connectedFileTag({ state: 'extracted', readable: false, relative: 'A/plan.DOCX' })?.label).toBe('read by Hollow · Word');
    expect(connectedFileTag({ state: 'extracted', readable: false, relative: 'A/deck.pptx' })?.label).toBe('read by Hollow · PowerPoint');
    expect(connectedFileTag({ state: 'extracted', readable: false, relative: 'A/budget.xlsx' })?.label).toBe('read by Hollow · Excel');
    expect(connectedFileTag({ state: 'extracted', readable: false })?.label).toBe('read by Hollow · document');
    expect(FILE_STATE_OPTIONS.find(o => o.value === 'extracted')?.label).toBe('Read by Hollow (PDF, Office)');
  });
  it('opens a searched file at its best-matching line', () => {
    const text = '# Garage 14\n\nTerm: five years.\nRent: the garage lease has a 3% escalator.\n';
    const at = findLine(text, 'garage 14 lease escalator');
    expect(at?.number).toBe(4);
    expect(at?.line).toBe('Rent: the garage lease has a 3% escalator.');
    // Ties go to the first line: a heading naming the file wins.
    expect(findLine(text, 'garage 14')?.number).toBe(1);
    expect(`${at?.before}${at?.line}${at?.after}`).toBe(text);
    expect(findLine(text, 'nothing here')).toBeNull();
    expect(findLine('', 'garage')).toBeNull();
  });
  it('counts what is shown out of what matched', () => {
    expect(listCountText(100, 2804, 0, '')).toBe('Showing 100 of 2,804 files');
    expect(listCountText(1, 1, 0, 'garage')).toBe('1 file');
    expect(listCountText(0, 0, 2, 'garage')).toBe('No listed file matches · 2 more found directly');
    expect(listCountText(0, 0, 0, '')).toBe('No files');
  });
});
