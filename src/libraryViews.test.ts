import { describe, expect, it } from 'vitest';
import { addView, meaningCountText, meaningInCollection, readViews, removeView, templateBody } from './libraryViews';

// The Library's source files, to show no copy names a Settings section that doesn't exist (P1-19).
const sources = import.meta.glob(['./**/*.ts', './**/*.tsx', '!./**/*.test.ts', '!./**/*.test.tsx'], { query: '?raw', import: 'default', eager: true }) as Record<string, string>;

describe('Library tabs and state (P1-19)', () => {
  it('filters meaning hits to the chosen collection and names it in the count', () => {
    const hits = [{ path: 'people/ana.md' }, { path: 'library/ana-notes.md' }, { path: 'people/team/bo.md' }, { path: 'workspaces/a/outputs/ana.md' }];
    expect(meaningInCollection(hits, 'people').map(h => h.path)).toEqual(['people/ana.md', 'people/team/bo.md']);
    expect(meaningInCollection(hits, 'outputs').map(h => h.path)).toEqual(['workspaces/a/outputs/ana.md']);
    expect(meaningInCollection(hits, 'all')).toHaveLength(4);
    expect(meaningInCollection(hits, 'library').map(h => h.path)).toEqual(['library/ana-notes.md']);
    expect(meaningCountText(12, 'people')).toBe('12 in people/ by meaning');
    expect(meaningCountText(1, 'library')).toBe('1 in library/ by meaning');
    expect(meaningCountText(3, 'all')).toBe('3 files by meaning');
    expect(meaningCountText(1, 'all')).toBe('1 file by meaning');
  });
  it('saves By meaning with a view, and an older view reads it as off', () => {
    expect(readViews(JSON.stringify([{ id: 'a', name: 'A', folder: 'people', query: 'ana' }]))[0].byMeaning).toBe(false);
    expect(readViews(JSON.stringify([{ id: 'a', name: 'A', folder: 'people', query: 'ana', byMeaning: true }]))[0].byMeaning).toBe(true);
    let v = addView([], { name: 'People: ana', folder: 'people', query: 'ana', byMeaning: false });
    v = addView(v, { name: 'People: ana, by meaning', folder: 'people', query: 'ana', byMeaning: true });
    expect(v.map(x => [x.name, x.byMeaning])).toEqual([['People: ana', false], ['People: ana, by meaning', true]]);
  });
  it('names no “Files & storage” section anywhere in the app', () => {
    expect(Object.keys(sources).length).toBeGreaterThan(100);
    for (const [file, text] of Object.entries(sources)) expect(text.includes('Files & storage'), file).toBe(false);
  });
});

describe('Library views and templates (next-level P06)', () => {
  it('adds, renames duplicates and removes saved views', () => {
    let v = addView([], { name: 'Finance', folder: 'sources/finance', query: '' });
    v = addView(v, { name: 'Finance sources', folder: 'sources/finance', query: '  ' });
    expect(v).toHaveLength(1);
    expect(v[0].name).toBe('Finance sources');
    v = addView(v, { name: 'Leases', folder: 'all', query: 'lease' });
    expect(v.map(x => x.name)).toEqual(['Finance sources', 'Leases']);
    expect(removeView(v, v[0].id).map(x => x.name)).toEqual(['Leases']);
    expect(addView(v, { name: '  ', folder: 'x', query: '' })).toHaveLength(2);
  });
  it('ignores malformed stored views', () => {
    expect(readViews('nope')).toEqual([]);
    expect(readViews(JSON.stringify([{ id: 'a', name: 'A', folder: 'f', query: '' }, { id: 1 }]))).toHaveLength(1);
  });
  it('fills a template with the title and date', () => {
    expect(templateBody('meeting', 'Ops sync', '2026-10-01')).toMatch(/^# Ops sync\n\n\*\*Date:\*\* 2026-10-01/);
    expect(templateBody('unknown', 'X', 'd')).toBe('# X\n\n');
    expect(templateBody('runbook', 'Synthetic checklist', '2026-10-24')).toBe('# Synthetic checklist\n\n');
  });
});
