import { describe, expect, it } from 'vitest';
import { PROJECT_COLORS, defaultProjectColor, monogram, nextProjectColor, projectColor } from './projectIconColors';

describe('project monograms and colors (4.4, A8)', () => {
  it('takes initials of two words, else the first two letters, else a question mark', () => {
    expect(monogram('Hollow')).toBe('HO');
    expect(monogram('Shift Log')).toBe('SL');
    expect(monogram('atlas 2.0')).toBe('A2');
    expect(monogram('  --  ')).toBe('?');
    expect(monogram('Éclair des îles')).toBe('ÉD');
    expect(monogram('Q')).toBe('Q');
  });

  it('gives a project the same color every time and spreads ids over the set', () => {
    expect(defaultProjectColor('work-1')).toBe(defaultProjectColor('work-1'));
    const seen = new Set(Array.from({ length: 60 }, (_, i) => defaultProjectColor(`workspace-${i}`).id));
    expect(seen.size).toBeGreaterThan(4);
  });

  it('prefers a picked color and ignores a stale one', () => {
    expect(projectColor('a', 'teal').id).toBe('teal');
    expect(projectColor('a', 'gone')).toBe(defaultProjectColor('a'));
    expect(projectColor('a', undefined)).toBe(defaultProjectColor('a'));
  });

  it('cycles through every color and back', () => {
    let c = PROJECT_COLORS[0];
    const ids = [c.id];
    for (let i = 1; i < PROJECT_COLORS.length; i++) { c = nextProjectColor(c); ids.push(c.id); }
    expect(new Set(ids).size).toBe(PROJECT_COLORS.length);
    expect(nextProjectColor(c).id).toBe(PROJECT_COLORS[0].id);
  });
});
