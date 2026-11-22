import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { MOCK_MARKERS, findMockMarkers } from './check-no-mock-bundle.mjs';

function dist(files) {
  const dir = mkdtempSync(join(tmpdir(), 'hollow-no-mock-'));
  for (const [name, text] of Object.entries(files)) {
    mkdirSync(join(dir, name, '..'), { recursive: true });
    writeFileSync(join(dir, name), text);
  }
  return dir;
}

test('a clean production bundle passes', () => {
  const dir = dist({ 'assets/index-abc.js': 'const a=1;', 'assets/App-def.js': 'export{}', 'index.html': '__mockApiCalls in html is not a chunk' });
  try { assert.deepEqual(findMockMarkers(dir), []); } finally { rmSync(dir, { recursive: true }); }
});

test('every marker is reported in any nested chunk', () => {
  for (const marker of MOCK_MARKERS) {
    const dir = dist({ 'assets/deep/mockApi-x1.js': `window.${marker}??={}` });
    try {
      const hits = findMockMarkers(dir);
      assert.equal(hits.length, 1, marker);
      assert.equal(hits[0].marker, marker);
      assert.match(hits[0].file, /mockApi-x1\.js$/);
    } finally { rmSync(dir, { recursive: true }); }
  }
});
