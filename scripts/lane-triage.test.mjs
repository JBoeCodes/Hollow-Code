// BI3: a lane ledger row carries a triage outcome, and a blocked row carries a
// checkable Unblock line with an owner.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { OUTCOMES, checkLedger, parseRecords } from './lane-triage.mjs';

const HEAD = ['| Row | Triage | PR | Merge sha | Gate | Review | Notes |', '|---|---|---|---|---|---|---|'];
const table = (...rows) => [...HEAD, ...rows].join('\n');

test('the four outcomes are work, close, block and drop', () => {
  assert.deepEqual([...OUTCOMES].sort(), ['block', 'close', 'drop', 'work']);
});

test('a blocked row with no Unblock line fails, naming the row', () => {
  const text = table('| NV5 | block | none | none | none | none | waiting on a key |');
  const problems = checkLedger(text);
  assert.equal(problems.length, 1);
  assert.match(problems[0], /NV5/);
  assert.match(problems[0], /Unblock/);
});

test('a blocked row with a condition but no owner fails, and one with an owner but no condition fails', () => {
  const noOwner = table('| NV5 | block | none | none | none | none | Unblock: key file exists at the documented path |');
  assert.match(checkLedger(noOwner).join('\n'), /NV5.*Owner/);
  const noCondition = table('| NV5 | block | none | none | none | none | Unblock: . Owner: Alex |');
  assert.match(checkLedger(noCondition).join('\n'), /NV5.*condition/);
});

test('a blocked row with a condition and an owner passes, in either order', () => {
  const a = table('| NV5 | block | none | none | none | none | Unblock: key file exists at the documented path. Owner: Alex |');
  const b = table('| NV5 | block | none | none | none | none | Owner: Alex; Unblock: key file exists at the documented path |');
  assert.deepEqual(checkLedger(a), []);
  assert.deepEqual(checkLedger(b), []);
});

test('an unfilled placeholder is not a condition or an owner', () => {
  const text = table('| NV5 | block | none | none | none | none | Unblock: <a condition someone can check>. Owner: <who> |');
  const problems = checkLedger(text).join('\n');
  assert.match(problems, /NV5.*condition/);
  assert.match(problems, /NV5.*Owner/);
});

test('a row closed as already done needs evidence, and a dropped row needs its reason', () => {
  const text = table(
    '| LF3 | close | none | none | none | none | already on main |',
    '| LF5 | drop | none | none | none | none | no longer wanted |',
    '| LF6 | close | none | none | none | none | Evidence: main c9c328e7a has the test |',
    '| LF7 | drop | none | none | none | none | Why: refused 2026-10-08 in DO_NOT_DO |',
  );
  const problems = checkLedger(text);
  assert.equal(problems.length, 2);
  assert.match(problems[0], /LF3.*Evidence/);
  assert.match(problems[1], /LF5.*Why/);
});

test('a work row needs nothing more, and an unknown outcome fails', () => {
  assert.deepEqual(checkLedger(table('| DOC1 | work | #1 | abc1234 | green 41 s | Not independently reviewed: docs | |')), []);
  assert.match(checkLedger(table('| DOC1 | delete | none | none | none | none | |')).join('\n'), /DOC1.*delete/);
});

test('held-row list items are read, and a held row with no triage outcome fails', () => {
  const text = [
    '## Held rows',
    '',
    '- BG4: Triage: block. Unblock: a Hollow install exists on main. Owner: Alex.',
    '- PM1: Triage: block.',
    '- UN2: held for the mini',
    '',
    '## Decisions',
    '',
    '- A note without a triage outcome outside the held section is not a row.',
  ].join('\n');
  const problems = checkLedger(text);
  assert.equal(problems.length, 2);
  assert.match(problems[0], /PM1.*Unblock/);
  assert.match(problems[1], /UN2.*no triage outcome/);
});

test('an indented continuation line belongs to the row above it', () => {
  const text = [
    '## Held rows',
    '',
    '- BG4: Triage: block.',
    '  Unblock: a Hollow install exists on main.',
    '  Owner: Alex.',
  ].join('\n');
  assert.deepEqual(checkLedger(text), []);
  assert.equal(parseRecords(text).length, 1);
});

test('a table whose header has no Triage column is ignored', () => {
  assert.deepEqual(checkLedger('| Name | Value |\n|---|---|\n| a | block |'), []);
});

test('an unfilled triage cell is a problem unless placeholders are allowed', () => {
  const text = table('| <row id> | <work, close, block or drop> | | | | | |');
  assert.match(checkLedger(text).join('\n'), /no triage outcome/);
  assert.deepEqual(checkLedger(text, { allowPlaceholders: true }), []);
});

