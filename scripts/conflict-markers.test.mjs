// No merge-conflict markers in the repo's tracked text files (Markdown since
// overnight S5; every text file since A8, which also found none in code). Rebases that resolve
// docs/BUILD_LOG.md by hand (or replay a recorded resolution with rerere and
// the diff3 style) left 27 stray `||||||| base` / `||||||| parent of …` lines
// there, found 2026-10-10 (overnight S5). A marker is one at the start of a
// line followed by a space and a label, as git writes them; a bare `=======`
// is a Markdown heading underline and isn't checked. `tests/fixtures/` is
// skipped: Hollow writes conflict markers into vault notes itself (the
// conflict resolver), so a fixture may hold them on purpose.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));

export const MARKER = /^(?:<{7}|\|{7}|>{7}) \S/;

test('the conflict-marker pattern matches what git writes and nothing else', () => {
  for (const line of ['<<<<<<< HEAD', '||||||| base', '||||||| parent of 229dc2f6 (docs: x)', '>>>>>>> 640f640b (Linked edits)']) {
    assert.ok(MARKER.test(line), line);
  }
  for (const line of ['=======', '|||||||', 'a <<<<<<< HEAD', '<<<<<<<HEAD', '| col | col |']) {
    assert.ok(!MARKER.test(line), line);
  }
});

/** The text of a tracked file, or null for a binary, a missing or an oversized (> 5 MB) one;
 *  Markdown is never skipped for size (the build log grows every session). */
export function textOf(path) {
  let buf;
  try {
    if (!path.endsWith('.md') && statSync(path).size > 5 * 1024 * 1024) return null;
    buf = readFileSync(path);
  } catch (e) {
    if (e.code === 'ENOENT') return null; // deleted in the working tree
    throw e;
  }
  return buf.subarray(0, 8192).includes(0) ? null : buf.toString('utf8');
}

test('no tracked text file holds a merge-conflict marker', () => {
  // -z: paths unquoted, whatever characters they hold; --deduplicate: an
  // unmerged path once, not once per stage.
  const files = execFileSync('git', ['ls-files', '-z', '--deduplicate'], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\0')
    .filter(f => f && !f.startsWith('tests/fixtures/'));
  assert.ok(files.some(f => f.endsWith('.rs')) && files.some(f => f.endsWith('.md')), 'git lists code and Markdown');
  const found = [];
  for (const file of files) {
    const text = textOf(join(root, file));
    if (text === null) continue;
    text.split('\n').forEach((line, i) => { if (MARKER.test(line)) found.push(`${file}:${i + 1}: ${line.slice(0, 60)}`); });
  }
  assert.deepEqual(found, [], `resolve or delete these conflict markers:\n${found.join('\n')}`);
});

test('binary and missing files are skipped, text is read', () => {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-markers-'));
  try {
    writeFileSync(join(dir, 'a.bin'), Buffer.from([0x3c, 0, 0x3c]));
    writeFileSync(join(dir, 'a.txt'), 'plain');
    assert.equal(textOf(join(dir, 'a.bin')), null);
    assert.equal(textOf(join(dir, 'gone.txt')), null);
    assert.equal(textOf(join(dir, 'a.txt')), 'plain');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
