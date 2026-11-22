import { describe, expect, it } from 'vitest';
import ICONS from './icons.tsx?raw';
import CHAT_PREVIEW from './components/ChatPreview.tsx?raw';

// P0-26 (GAP5-03): an unknown icon name renders nothing, so the files
// shelf's image tiles were blank ('image' isn't an icon).
const PATHS = new Set([...ICONS.matchAll(/^ {2}(\w+): \(\) =>/gm)].map(m => m[1]));

/** Icon names a source passes as literals: `name="x"`, `icon="x"`, and the
 *  string results inside `<Icon name={…}>` (not the values it compares). */
function iconNames(source: string): string[] {
  const names = [...source.matchAll(/<Icon name="(\w+)"|(?<![\w-])icon="(\w+)"/g)].map(m => m[1] ?? m[2]);
  for (const m of source.matchAll(/<Icon name=\{([^}]*)\}/g)) {
    for (const lit of m[1].matchAll(/(?<![!=]==?\s*)["'](\w+)["']/g)) names.push(lit[1]);
  }
  return names;
}

describe('icon names', () => {
  it('reads the icon set', () => {
    expect(PATHS.has('fileImage')).toBe(true);
    expect(PATHS.size).toBeGreaterThan(50);
  });

  // Overnight 11-04 S1: 28 icons pick their name from state (a fold's
  // chevron, a busy glyph). Icon read the name once, so each kept its first
  // shape; the drawing now reads it where it renders.
  it('draws the current name, not the first one', () => {
    expect(ICONS).toContain('{PATHS[props.name]?.()}');
    expect(ICONS).not.toContain('const make = PATHS[props.name];');
  });

  it('every icon ChatPreview names exists', () => {
    const missing = iconNames(CHAT_PREVIEW).filter(n => !PATHS.has(n));
    expect(missing).toEqual([]);
  });
});
