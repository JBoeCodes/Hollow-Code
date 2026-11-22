// LF4: a merge lane's brief carries the verdict and findings, not the report.
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { contradictions, extract } from './lane-extract.mjs';

const dir = () => mkdtempSync(join(tmpdir(), 'lane-extract-'));
const noise = n => Array.from({ length: n }, (_, i) => `Reviewer walked file ${i}: read the diff, ran the case, saw nothing to add on line ${i * 7}.`).join('\n');

/** A replayed review report: long method notes around a short verdict and findings. */
const REVIEW = ['# Review of PR 1541', '', '## Method', noise(120), '', '## Verdict', 'Verdict: changes requested', '', '## Findings', '1. Evidence path not checked for symlinks.', '2. Missing test for empty record.', '', '## Appendix', noise(80), ''].join('\n');

test('a replayed review brief keeps verdict and findings and is far smaller than the file', () => {
  const f = join(dir(), 'review.md');
  writeFileSync(f, REVIEW);
  const r = extract([`${f}::Verdict|Findings`]);
  assert.deepEqual(r.problems, []);
  assert.match(r.brief, /Verdict: changes requested/);
  assert.match(r.brief, /symlinks/);
  assert.doesNotMatch(r.brief, /walked file 5:/);
  assert.ok(r.briefBytes / r.sourceBytes < 0.1, `ratio ${r.briefBytes / r.sourceBytes}`);
});

test('a missing source, missing section and empty section each refuse with nothing built', () => {
  const d = dir();
  const f = join(d, 'l.md');
  writeFileSync(f, '## Verdict\n\n## Rows\nok\n');
  const r = extract([join(d, 'nope.md::Verdict'), `${f}::Verdict|Gate|Rows`]);
  assert.equal(r.brief, '');
  assert.deepEqual(r.problems.map(p => p.replace(d, '')), ['/nope.md: extract source is missing', '/l.md: section "Verdict" is empty', '/l.md: section "Gate" is missing']);
});

test('a file that gives two different verdicts is refused; the same verdict twice is fine', () => {
  const d = dir();
  const f = join(d, 'r.md');
  writeFileSync(f, '## Summary\n- Verdict: approved\n\n## Verdict\nVerdict: changes requested\n');
  const r = extract([`${f}::Verdict`]);
  assert.match(r.problems[0], /contradicts itself: verdict has 2 different values/);
  assert.deepEqual(contradictions('Verdict: approved\n- **Verdict:** Approved.\n'), []);
});

test('a section runs to the next heading of the same or higher level, keeping subheadings', () => {
  const f = join(dir(), 'r.md');
  writeFileSync(f, '## Findings\nintro\n### One\nbody one\n## Other\nnot this\n');
  const r = extract([`${f}::Findings`]);
  assert.match(r.brief, /body one/);
  assert.doesNotMatch(r.brief, /not this/);
});

test('exact heading names: a prefix does not match, and an empty twin does not shadow a filled one', () => {
  const f = join(dir(), 'r.md');
  writeFileSync(f, '## Verdict\n\n## Verdicts and more\nno\n\n## Verdict\nreal\n');
  assert.match(extract([`${f}::Verdict`]).brief, /real/);
  assert.match(extract([`${f}::Verd`]).problems[0], /section "Verd" is missing/);
});
