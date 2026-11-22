import { describe, expect, it } from 'vitest';
import { Text } from '@codemirror/state';
import editorSource from './MarkdownEditor.tsx?raw';

describe('MarkdownEditor performance & frontmatter hiding fixes', () => {
  it('memoizes knownSet via createMemo', () => {
    expect(editorSource).toContain('const knownSet = createMemo(() => new Set(noteIndex().map(n => n.basename.toLowerCase())));');
  });

  it('un-hides frontmatter when selection enters the frontmatter range and avoids doc.toString() on cursor moves', () => {
    const start = editorSource.indexOf('export function frontmatterRange');
    const end = editorSource.indexOf('// ── Table rendering', start);
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const section = editorSource.slice(start, end);
    expect(section).not.toContain('doc.toString()');
    expect(section).toContain('selection.ranges.some(r => r.from < end)');
    expect(section).toContain('if (shouldHide !== this.hidden)');

    // Execute frontmatterRange directly against CodeMirror Text objects.
    const fnEnd = editorSource.indexOf('const hideFrontmatter', start);
    const fnBody = editorSource
      .slice(start, fnEnd)
      .replace(/^export function frontmatterRange\([^)]*\)[^{]*\{/, 'function frontmatterRange(doc) {');
    const frontmatterRange = new Function(`${fnBody}\nreturn frontmatterRange;`)() as (doc: Text) => { end: number; endLine: number } | null;

    const withBody = Text.of(['---', 'title: Hello', '---', 'Body line']);
    expect(frontmatterRange(withBody)).toEqual({ end: withBody.line(4).from, endLine: 3 });

    const noBody = Text.of(['---', 'title: Hello', '---']);
    expect(frontmatterRange(noBody)).toEqual({ end: noBody.length, endLine: 3 });

    const plain = Text.of(['# Just a heading', '---', 'Body']);
    expect(frontmatterRange(plain)).toBeNull();

    const unclosed = Text.of(['---', 'title: Hello', 'no closing fence']);
    expect(frontmatterRange(unclosed)).toBeNull();
  });
});
