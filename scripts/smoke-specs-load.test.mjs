import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { closeSync, mkdtempSync, openSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

// Overnight 11-03: one smoke spec that failed to load (a duplicate
// declaration, 10-27) left every whole-suite `playwright test` with 0 of
// 3647 tests, and nothing in the gate runs Playwright, so nobody saw it for a
// week. Listing loads every spec file without a browser or the dev server, so
// a load error now fails the gate instead.
const root = fileURLToPath(new URL('..', import.meta.url));

test('every smoke spec loads, so a whole-suite run has tests to run', () => {
  // --forbid-only: a committed test.only fails here as it does in CI (the
  // config forbids it only when CI is set). One line per test: room to grow.
  // The list goes to a file, not a pipe: Playwright exits with output still
  // queued for a pipe, and under load the tail (with its Total line) was lost
  // at exit 0 (self-hosted Q5, 11-03). Writes to a file are synchronous.
  const dir = mkdtempSync(join(tmpdir(), 'specs-load-'));
  const file = join(dir, 'list.txt');
  const fd = openSync(file, 'w');
  let r;
  try {
    r = spawnSync(process.execPath, ['node_modules/@playwright/test/cli.js', 'test', '--list', '--forbid-only', '--reporter=list'], {
      cwd: root, stdio: ['ignore', fd, fd], timeout: 120_000,
    });
  } finally {
    closeSync(fd);
  }
  const out = readFileSync(file, 'utf8');
  rmSync(dir, { recursive: true, force: true });
  const errors = out.split('\n').filter(line => /Error/.test(line)).slice(0, 10).join('\n') || out.slice(-600);
  assert.equal(r.status, 0, `playwright test --list failed (${r.error ?? `exit ${r.status}`}):\n${errors}`);
  const total = /Total: (\d+) tests? in (\d+) files?/.exec(out);
  assert.ok(total && Number(total[1]) > 0, `playwright test --list found no tests:\n${out.slice(-600)}`);
});
