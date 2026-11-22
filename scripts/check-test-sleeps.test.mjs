// TS1: no new wall-clock sleep or clock read in a test. The counts left when
// the check landed are a baseline that may only shrink.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { BASELINE, candidateFiles, compare, countAll, countOffenders, isRustTestFile, isTsTest, rustTestLines } from './check-test-sleeps.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

test('frontend and script tests: fixed sleeps and clock reads are counted, other lines are not', () => {
  const text = [
    "await page.waitForTimeout(500);",
    "await new Promise(r => setTimeout(r, 50));",
    "await new Promise<void>((resolve) => setTimeout(resolve, 10));",
    "await delay(25);",
    "const t = Date.now();",
    "const d = new Date();",
    "const fixed = new Date('2026-11-16T12:00:00Z');",
    "const zero = new Date(0);",
    "vi.advanceTimersByTime(500);",
    "await expect.poll(() => ready).toBe(true);",
    "// await page.waitForTimeout(500) is how this used to wait",
    " * Date.now() in a doc comment",
  ].join('\n');
  assert.deepEqual(countOffenders('src/store/x.test.ts', text), { sleep: 4, clock: 2 });
  assert.deepEqual(countOffenders('tests/smoke/y.spec.ts', text), { sleep: 4, clock: 2 });
  assert.deepEqual(countOffenders('scripts/z.test.mjs', text), { sleep: 4, clock: 2 });
  // A file that is not a test is not read.
  assert.deepEqual(countOffenders('src/store/x.ts', text), { sleep: 0, clock: 0 });
});

test('a line that says why it must sleep, on or above it, is accepted', () => {
  const text = [
    'await page.waitForTimeout(300); // sleep-ok: the CSS transition has no event',
    '// sleep-ok: measuring a real animation frame',
    'const t = Date.now();',
    'await page.waitForTimeout(300); // a plain comment is not a reason',
    '// sleep-ok:',
    'await page.waitForTimeout(100);',
  ].join('\n');
  assert.deepEqual(countOffenders('tests/smoke/a.spec.ts', text), { sleep: 2, clock: 0 });
});

test('Rust: a tests file counts every sleep, any other file only inside its cfg(test) block', () => {
  const body = [
    'fn production() {',
    '    std::thread::sleep(Duration::from_millis(500));',
    '}',
    '',
    '#[cfg(test)]',
    'mod tests {',
    '    use super::*;',
    '    #[test]',
    '    fn waits() {',
    '        std::thread::sleep(Duration::from_millis(50));',
    '        tokio::time::sleep(Duration::from_millis(5)).await;',
    '        // thread::sleep in a comment',
    '    }',
    '}',
    '',
    'fn more_production() {',
    '    tokio::time::sleep(Duration::from_secs(1)).await;',
    '}',
  ].join('\n');
  assert.deepEqual(countOffenders('src-tauri/src/thing.rs', body), { sleep: 2, clock: 0 });
  assert.deepEqual(countOffenders('src-tauri/src/thing_tests.rs', body), { sleep: 4, clock: 0 });
  assert.deepEqual(countOffenders('src-tauri/src/no_test_block.rs', 'fn f() { std::thread::sleep(d); }'), { sleep: 0, clock: 0 });
  assert.ok(isRustTestFile('src-tauri/src/a/b_tests.rs') && isRustTestFile('src-tauri/tests/x.rs') && !isRustTestFile('src-tauri/src/a.rs'));
  assert.ok(isTsTest('tests/smoke/helpers.ts') && isTsTest('src/a.test.tsx') && !isTsTest('src/a.tsx') && !isTsTest('tests/x.json'));
  assert.deepEqual([...rustTestLines(body)].sort((a, b) => a - b), [6, 7, 8, 9, 10, 11, 12]);
});

test('the baseline only ratchets down: a rise, a new file and a stale count all fail', () => {
  const baseline = { 'a.test.ts': { sleep: 2 }, 'b.test.ts': { clock: 1 } };
  assert.deepEqual(compare({ 'a.test.ts': { sleep: 2 }, 'b.test.ts': { clock: 1 } }, baseline), { grew: [], shrank: [] });
  assert.deepEqual(compare({ 'a.test.ts': { sleep: 3 }, 'b.test.ts': { clock: 1 } }, baseline).grew, ['a.test.ts sleep: 2 → 3']);
  assert.deepEqual(compare({ 'a.test.ts': { sleep: 2 }, 'b.test.ts': { clock: 1 }, 'c.test.ts': { sleep: 1 } }, baseline).grew, ['c.test.ts sleep: 0 → 1']);
  assert.deepEqual(compare({ 'a.test.ts': { sleep: 1 }, 'b.test.ts': { clock: 1 } }, baseline).shrank, ['a.test.ts sleep: 2 → 1']);
  assert.deepEqual(compare({ 'b.test.ts': { clock: 1 } }, baseline).shrank, ['a.test.ts sleep: 2 → 0']);
  assert.deepEqual(compare({ 'a.test.ts': { sleep: 2 }, 'b.test.ts': { clock: 1, sleep: 1 } }, baseline).grew, ['b.test.ts sleep: 0 → 1']);
});

test('this repo holds exactly the baseline: no new sleep or clock read in a test', () => {
  const baseline = JSON.parse(readFileSync(join(root, BASELINE), 'utf8'));
  assert.equal(baseline.version, 1);
  const files = candidateFiles(root);
  assert.ok(files.length > 500, `read ${files.length} files`);
  const counts = countAll(files, f => {
    try {
      return readFileSync(join(root, f), 'utf8');
    } catch {
      return null;
    }
  });
  assert.deepEqual(compare(counts, baseline.files), { grew: [], shrank: [] });
});
