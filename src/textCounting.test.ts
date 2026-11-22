import { describe, expect, it } from 'vitest';

/**
 * Agent effectiveness F4: one character-counting rule for Rust and the
 * frontend. Every character bound counts Unicode scalar values: Rust
 * `str::chars()`, JavaScript `[...text]` (never `.length`, which counts
 * UTF-16 units). The fixture is shared with `context_compiler.rs`'s test.
 *
 * The composer no longer builds the conversation bundle or the operator
 * profile (the backend's context compiler does, for every entry point), so
 * nothing in the frontend counts against those limits any more. The second
 * test keeps it that way.
 */
interface CountCase { name: string; text: string; scalars: number; utf16: number; last3: string }

const RAW = import.meta.glob('../src-tauri/fixtures/continuity/char-counting.json', { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
const fixture = JSON.parse(Object.values(RAW)[0]) as { cases: CountCase[] };

describe('the shared character-counting rule', () => {
  it('counts Unicode scalar values as Rust does, on astral, emoji and combining text', () => {
    expect(fixture.cases.length).toBeGreaterThanOrEqual(7);
    for (const c of fixture.cases) {
      expect([...c.text].length, c.name).toBe(c.scalars);
      expect([...c.text].slice(-3).join(''), c.name).toBe(c.last3);
      expect(c.text.length, c.name).toBe(c.utf16);
    }
    // The cases include text where UTF-16 length would count differently.
    expect(fixture.cases.some(c => c.utf16 !== c.scalars)).toBe(true);
  });

  it('leaves the conversation bundle and the operator profile to the backend', () => {
    const sources = import.meta.glob(['./**/*.ts', './**/*.tsx', '!./**/*.test.ts', '!./**/*.test.tsx', '!./generated/**', '!./mock*.ts'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;
    expect(Object.keys(sources).length).toBeGreaterThan(50);
    const composing = Object.entries(sources)
      .filter(([, text]) => text.includes('`<conversation-context>') || text.includes('`<operator-context>'))
      .map(([path]) => path);
    expect(composing).toEqual([]);
    expect(Object.keys(sources)).not.toContain('./chatContinuity.ts');
  });
});
