import { describe, expect, it } from 'vitest';
import { buildTree, countLineMatches, fileSig, filterFiles, isViewed, parseViewed, scopeChoices, scopeKey, setViewed, splitMatches, viewedCount, type DiffFileRow } from './diffPanelModel';

const f = (path: string, a: number | null = 1, d: number | null = 0, status = 'modified'): DiffFileRow => ({ path, status, additions: a, deletions: d });

describe('the Diff tab (parity 5.3)', () => {
  it('builds a tree: folders first, single-child folders joined, files sorted', () => {
    const tree = buildTree([f('src/components/b.tsx'), f('src/components/a.tsx'), f('README.md'), f('docs/x.md'), f('docs/deep/y/z.md')]);
    expect(tree.map(n => `${n.kind}:${n.name}`)).toEqual(['dir:docs', 'dir:src/components', 'file:README.md']);
    const docs = tree[0];
    expect(docs.kind === 'dir' && docs.children.map(n => `${n.kind}:${n.name}`)).toEqual(['dir:deep/y', 'file:x.md']);
    const src = tree[1];
    expect(src.kind === 'dir' && src.path).toBe('src/components');
    expect(src.kind === 'dir' && src.children.map(n => n.name)).toEqual(['a.tsx', 'b.tsx']);
  });
  it('filters files by path and cuts lines into matches for Find', () => {
    expect(filterFiles([f('src/a.ts'), f('docs/B.md')], 'b.')).toEqual([f('docs/B.md')]);
    expect(filterFiles([f('a')], '  ')).toHaveLength(1);
    expect(splitMatches('Foo bar foo', 'FOO')).toEqual([{ text: 'Foo', hit: true }, { text: ' bar ', hit: false }, { text: 'foo', hit: true }]);
    expect(splitMatches('abc', '')).toEqual([{ text: 'abc', hit: false }]);
    expect(countLineMatches(['one', 'Two', 'twotwo'], 'two')).toBe(2);
  });
  it('lists scopes: all changes, then turns that changed files, newest first', () => {
    const rows = scopeChoices([
      { ord: 1, before: 'aaaaaaa', after: 'bbbbbbb', files: ['a'] },
      { ord: 2, before: 'bbbbbbb', after: 'ccccccc', files: ['a', 'b'] },
      { ord: 3, before: 'ccccccc', after: null, files: ['x'] },
      { ord: 4, before: 'ddddddd', after: 'eeeeeee', files: [] },
    ]);
    expect(rows.map(r => r.label)).toEqual(['All uncommitted changes', 'Turn 2: 2 files', 'Turn 1: 1 file']);
    expect(scopeKey(rows[1].scope)).toBe('turn:2:ccccccc');
  });
  it('keeps viewed per folder and scope, and clears it when the file changes', () => {
    const all = { kind: 'all' } as const;
    let store = parseViewed(null);
    store = setViewed(store, '/repo', all, f('a.ts'), true);
    expect(isViewed(store, '/repo', all, f('a.ts'))).toBe(true);
    expect(isViewed(store, '/repo', all, f('a.ts', 2, 0))).toBe(false);
    expect(isViewed(store, '/other', all, f('a.ts'))).toBe(false);
    expect(viewedCount(store, '/repo', all, [f('a.ts'), f('b.ts')])).toBe(1);
    store = setViewed(store, '/repo', all, f('a.ts'), false);
    expect(isViewed(store, '/repo', all, f('a.ts'))).toBe(false);
    expect(fileSig(f('x', null, null))).toBe('modified:?:?');
  });
  it('reads a saved store defensively and caps it', () => {
    expect(parseViewed('{')).toEqual({});
    expect(parseViewed('[1]')).toEqual({});
    expect(parseViewed('{"/r":{"all|a":"modified:1:0","bad":3},"/s":5}')).toEqual({ '/r': { 'all|a': 'modified:1:0' } });
    let store = {};
    for (let i = 0; i < 25; i++) store = setViewed(store, `/r${i}`, { kind: 'all' }, f('a'), true);
    expect(Object.keys(store)).toHaveLength(20);
    expect(Object.keys(store)).toContain('/r24');
    let big = {};
    for (let i = 0; i < 520; i++) big = setViewed(big, '/r', { kind: 'all' }, f(`f${i}`), true);
    expect(Object.keys((big as Record<string, object>)['/r'])).toHaveLength(500);
  });
});
