import { describe, expect, it } from 'vitest';
import { HIGHLIGHT_MAX, highlights, prettyJson, textViewOf } from './previewKinds';

describe('how the Preview pane shows a text file', () => {
  it('shows CSV and TSV as a table and JSON pretty-printed', () => {
    expect(textViewOf('/v/data/sales.csv')).toEqual({ kind: 'table' });
    expect(textViewOf('/v/data/SALES.TSV')).toEqual({ kind: 'table' });
    expect(textViewOf('/v/a/package.json')).toEqual({ kind: 'json' });
    expect(textViewOf('/v/a/map.geojson')).toEqual({ kind: 'json' });
  });

  it('highlights code it has a grammar for, and leaves other text plain', () => {
    expect(textViewOf('/r/src/App.tsx')).toEqual({ kind: 'code', lang: 'tsx' });
    expect(textViewOf('/r/src-tauri/src/lib.rs')).toEqual({ kind: 'code', lang: 'rust' });
    expect(textViewOf('/r/scripts/run.py')).toEqual({ kind: 'code', lang: 'python' });
    expect(textViewOf('/r/fix.patch')).toEqual({ kind: 'code', lang: 'diff' });
    expect(textViewOf('/v/notes/todo.txt')).toEqual({ kind: 'code', lang: undefined });
    expect(textViewOf('/v/Makefile')).toEqual({ kind: 'code', lang: undefined });
  });

  it('pretty-prints JSON and says null for anything that is not JSON', () => {
    expect(prettyJson('{"a":1,"b":[true,null]}')).toBe('{\n  "a": 1,\n  "b": [\n    true,\n    null\n  ]\n}');
    expect(prettyJson('{"a":')).toBeNull();
    expect(prettyJson('')).toBeNull();
  });

  it('skips highlighting very large files and unknown languages', () => {
    expect(highlights('let a = 1;', 'ts')).toBe(true);
    expect(highlights('plain words', undefined)).toBe(false);
    expect(highlights('x'.repeat(HIGHLIGHT_MAX + 1), 'ts')).toBe(false);
  });
});
