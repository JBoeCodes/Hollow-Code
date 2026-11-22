import { describe, expect, it } from 'vitest';
import { contextHealthFromSizes, isResultBookkeeping } from './workspaceRecap';
import RECAP_SOURCE from './workspaceRecap.ts?raw';

// FIX-AC: workspaceRecap.ts is on the App chunk (the browser mock reads it);
// sectionMarkdown.ts belongs to lazy chunks.
describe('workspaceRecap stays light (FIX-AC)', () => {
  it('does not import the Markdown section reader', () => {
    expect(RECAP_SOURCE).not.toMatch(/from '\.\/(briefModel|sectionMarkdown)'/);
  });
});

describe('result bookkeeping', () => {
  it('a saved canvas\'s record, its hidden revisions and a removed curated view\'s files are not Results (C3 limit)', () => {
    // A state save rewrites only the sidecar: nothing changed for the person.
    expect(isResultBookkeeping('outputs/canvases/plan.canvas.json')).toBe(true);
    expect(isResultBookkeeping('outputs/canvases/.revisions/plan/abc.canvas.html')).toBe(true);
    // A new revision rewrites the visible copy: that is a Result.
    expect(isResultBookkeeping('outputs/canvases/plan.canvas.html')).toBe(false);
    // Files a curated section canvas left (C6 · K1, removed by COO rebuild L4) stay out.
    expect(isResultBookkeeping('outputs/canvases/_plan.canvas.html')).toBe(true);
    expect(isResultBookkeeping('outputs/canvases/_results.canvas.html')).toBe(false);
  });
});

describe('context health (durability D3)', () => {
  it('matches the Rust arithmetic: under, near and over budget', () => {
    expect(contextHealthFromSizes([{ name: 'WORKSPACE.md', bytes: 9_000 }, { name: 'PLAN.md', bytes: 5_000 }]))
      .toEqual([{ name: 'WORKSPACE.md', bytes: 9_000, included: 9_000, limit: 43_000 }, { name: 'PLAN.md', bytes: 5_000, included: 5_000, limit: 24_000 }]);
    const near = contextHealthFromSizes([{ name: 'WORKSPACE.md', bytes: 9_000 }, { name: 'PLAN.md', bytes: 36_000 }, { name: 'AGENTS.md', bytes: 2_000 }]);
    expect(near[1]).toMatchObject({ included: 36_000, limit: 37_000 });
    const cut = contextHealthFromSizes([{ name: 'WORKSPACE.md', bytes: 13_900 }, { name: 'PLAN.md', bytes: 35_137 }, { name: 'SOURCES.md', bytes: 160 }, { name: 'AGENTS.md', bytes: 700 }]);
    expect(cut.map(d => d.included)).toEqual([13_900, 33_240, 160, 700]);
  });
});
