import { describe, expect, it } from 'vitest';
import { askAboutHunk, hunkRange, hunkText, parseDiff, unquotePath } from './diffParse';

const MODIFIED = `diff --git a/src/app.ts b/src/app.ts
index 1111111..2222222 100644
--- a/src/app.ts
+++ b/src/app.ts
@@ -1,4 +1,5 @@ export function main() {
 const a = 1;
-const b = 2;
+const b = 3;
+const c = 4;
\x20
 run();
@@ -10,2 +11,2 @@
 x
-y
+z
`;

describe('parseDiff', () => {
  it('parses a modified file into hunks with line numbers', () => {
    const [f] = parseDiff(MODIFIED);
    expect(f.path).toBe('src/app.ts');
    expect(f.status).toBe('modified');
    expect([f.additions, f.deletions]).toEqual([3, 2]);
    expect(f.hunks).toHaveLength(2);
    const h = f.hunks[0];
    expect(h.section).toBe('export function main() {');
    expect(h.lines.map(l => [l.kind, l.oldNo, l.newNo])).toEqual([
      ['context', 1, 1], ['del', 2, null], ['add', null, 2], ['add', null, 3], ['context', 3, 4], ['context', 4, 5],
    ]);
    expect(h.lines[4].text).toBe('');
    expect(f.hunks[1].lines.map(l => [l.oldNo, l.newNo])).toEqual([[10, 11], [11, null], [null, 12]]);
  });

  it('handles new, deleted, renamed and binary files and the no-newline marker', () => {
    const text = `diff --git a/new file.txt b/new file.txt
new file mode 100644
index 0000000..0a207c0
--- /dev/null
+++ b/new file.txt\t
@@ -0,0 +1,2 @@
+a
+b
\\ No newline at end of file
diff --git a/gone.md b/gone.md
deleted file mode 100644
--- a/gone.md
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/old.rs b/new.rs
similarity index 90%
rename from old.rs
rename to new.rs
diff --git a/img.png b/img.png
index 1..2 100644
Binary files a/img.png and b/img.png differ
`;
    const files = parseDiff(text);
    expect(files.map(f => [f.path, f.status, f.binary])).toEqual([
      ['new file.txt', 'added', false], ['gone.md', 'deleted', false], ['new.rs', 'renamed', false], ['img.png', 'modified', true],
    ]);
    const added = files[0];
    expect(added.oldPath).toBeNull();
    expect(added.hunks[0].lines.at(-1)).toEqual({ kind: 'note', text: 'No newline at end of file', oldNo: null, newNo: null });
    expect(files[1].newPath).toBeNull();
    expect(files[2].oldPath).toBe('old.rs');
  });

  it('unquotes C-style paths', () => {
    expect(unquotePath('"a\\tb"')).toBe('a\tb');
    expect(unquotePath('"caf\\303\\251.txt"')).toBe('café.txt');
    expect(unquotePath('plain.txt\t')).toBe('plain.txt');
    const [f] = parseDiff('diff --git "a/caf\\303\\251" "b/caf\\303\\251"\n--- "a/caf\\303\\251"\n+++ "b/caf\\303\\251"\n@@ -1 +1 @@\n-a\n+b\n');
    expect(f.path).toBe('café');
  });

  it('tolerates redaction that folded lines (no hunk counts are trusted)', () => {
    const [f] = parseDiff('diff --git a/k b/k\n--- a/k\n+++ b/k\n@@ -1,3 +1,3 @@\n+[pem]\n x\n');
    expect(f.hunks[0].lines).toHaveLength(2);
  });

  it('returns nothing for empty text', () => {
    expect(parseDiff('')).toEqual([]);
  });
});

describe('asking about a hunk', () => {
  it('quotes the file, the new-side line range and the hunk', () => {
    const [f] = parseDiff(MODIFIED);
    expect(hunkRange(f.hunks[0])).toEqual({ side: 'new', start: 1, end: 5 });
    const draft = askAboutHunk(f, f.hunks[0]);
    expect(draft.startsWith('About this change in src/app.ts (lines 1-5):\n\n```diff\n@@ -1,4 +1,5 @@')).toBe(true);
    expect(draft).toContain('\n-const b = 2;\n+const b = 3;\n');
    expect(draft.trimEnd().endsWith('```')).toBe(true);
    expect(hunkText(f.hunks[1])).toBe('@@ -10,2 +11,2 @@\n x\n-y\n+z');
  });

  it('uses the old side for a pure deletion, and a longer fence around backticks', () => {
    const [f] = parseDiff('diff --git a/d b/d\ndeleted file mode 100644\n--- a/d\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-```\n-x\n');
    expect(hunkRange(f.hunks[0])).toEqual({ side: 'old', start: 1, end: 2 });
    const draft = askAboutHunk(f, f.hunks[0]);
    expect(draft).toContain('(lines 1-2, before the change)');
    expect(draft).toContain('````diff');
  });
});
