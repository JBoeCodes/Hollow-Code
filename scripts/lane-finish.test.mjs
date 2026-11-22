// LF2: a lane cannot finish on last night's ledger or an unfilled template.
import assert from 'node:assert/strict';
import { mkdtempSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { test } from 'node:test';
import { resolve as require_resolve } from 'node:path';
import { checkFinish, makeBaseline, placeholders } from './lane-finish.mjs';

const GOOD = '# Ledger\n\n## State\n\nDone with the rows.\n\n## Rows\n\n| a | b |\n|---|---|\n| X1 | closed |\n';
const dir = () => mkdtempSync(join(tmpdir(), 'lane-finish-'));

test('a planted stale ledger blocks, naming the file, even when touched', () => {
  const d = dir();
  const f = join(d, 'lane-m.md');
  writeFileSync(f, GOOD);
  const base = makeBaseline([f], '2026-11-17T00:00:00Z');
  utimesSync(f, new Date(), new Date(Date.now() + 5000)); // sleep-ok: sets a file mtime a few seconds ahead, no waiting
  const p = checkFinish(base, [`${f}::State|Rows`]);
  assert.equal(p.length, 1);
  assert.match(p[0], /lane-m\.md: unchanged since the baseline/);
});

test('a rewritten ledger passes', () => {
  const f = join(dir(), 'l.md');
  writeFileSync(f, GOOD);
  const base = makeBaseline([f]);
  writeFileSync(f, GOOD.replace('closed', 'merged #1'));
  assert.deepEqual(checkFinish(base, [`${f}::State|Rows`]), []);
});

test('a planted placeholder blocks, in angle and TODO forms, but code and html do not', () => {
  const f = join(dir(), 'l.md');
  writeFileSync(f, GOOD);
  const base = makeBaseline([f]);
  writeFileSync(f, `${GOOD}\n| <row id> | ok |\nTODO: write notes\n`);
  const p = checkFinish(base, [f]);
  assert.match(p[0], /unfilled placeholder <row id>, TODO/);
  assert.deepEqual(placeholders('use `<name>` here <br> and <https://x.y> ok\n```\n<in fence> TODO\n```\n'), []);
});

test('missing file, empty file, missing and empty sections are each named', () => {
  const d = dir();
  const gone = join(d, 'gone.md');
  const empty = join(d, 'empty.md');
  const thin = join(d, 'thin.md');
  writeFileSync(empty, '  \n');
  writeFileSync(thin, '## State\n\n## Rows\nfine\n');
  const base = makeBaseline([thin]);
  writeFileSync(thin, '## State\n\n## Rows\nfine, rewritten\n');
  const p = checkFinish(base, [gone, empty, `${thin}::State|Notes|Rows`]);
  assert.deepEqual(p, [`${gone}: missing`, `${empty}: empty`, `${thin}: section "State" is empty`, `${thin}: missing section "Notes"`]);
});

test('a required path missing from the baseline fails; a relative spelling of a baselined path matches', () => {
  const d = dir();
  const a = join(d, 'a.md');
  const b = join(d, 'b.md');
  writeFileSync(a, GOOD);
  writeFileSync(b, GOOD);
  const base = makeBaseline([a]);
  const p = checkFinish(base, [b]);
  assert.match(p[0], /b\.md: not in the baseline/);
  const rel = relative(process.cwd(), a);
  assert.match(checkFinish(base, [rel])[0], /unchanged since the baseline/);
});

test('sections: exact names, subsection-only is not empty, an empty twin does not shadow a filled one', () => {
  const f = join(dir(), 's.md');
  writeFileSync(f, '## Rows\n\n## State\nfilled later\n\n## Notes\n### Sub\ntext under a subsection\n\n## Rows\nreal rows\n\n## Stateful thing\nx\n');
  const p = checkFinish({ files: { [require_resolve(f)]: null } }, [`${f}::Rows|State|Notes|Sta`]);
  assert.deepEqual(p, [`${f}: missing section "Sta"`]);
});

test('an artifact absent at baseline counts as new once it exists', () => {
  const f = join(dir(), 'new.md');
  const base = makeBaseline([f]);
  assert.equal(base.files[f], null);
  writeFileSync(f, GOOD);
  assert.deepEqual(checkFinish(base, [f]), []);
});
