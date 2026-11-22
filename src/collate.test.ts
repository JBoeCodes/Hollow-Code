import { describe, expect, it } from 'vitest';
import { keyCompare, naturalCompare } from './collate';

const sample = ['Note 10', 'note 2', 'Note 1', 'Éclair', 'eclair', 'zebra', 'Zebra 3', 'Zebra 20', '', 'ENG-10', 'ENG-9', 'ai-100', 'ENG-2'];

describe('the shared collators order exactly as localeCompare with the same options did', () => {
  it('naturalCompare', () => {
    for (const a of sample) for (const b of sample) {
      expect(Math.sign(naturalCompare(a, b))).toBe(Math.sign(a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' })));
    }
  });
  it('keyCompare', () => {
    for (const a of sample) for (const b of sample) {
      expect(Math.sign(keyCompare(a, b))).toBe(Math.sign(a.localeCompare(b, undefined, { numeric: true })));
    }
  });
  it('read numbers as numbers, and ignore case for the Library order', () => {
    expect(['Note 10', 'Note 2', 'Note 1'].sort(naturalCompare)).toEqual(['Note 1', 'Note 2', 'Note 10']);
    expect(naturalCompare('Éclair', 'eclair')).toBe(0);
    expect(['ENG-10', 'ENG-9', 'ENG-2'].sort(keyCompare)).toEqual(['ENG-2', 'ENG-9', 'ENG-10']);
  });
});
