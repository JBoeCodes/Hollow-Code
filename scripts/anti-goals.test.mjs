// BI6: planners run candidate items through docs/DO_NOT_DO.md; a conflict is
// rejected with the rule's title and date named.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkRules, conflicts, parseRules, termMatches } from './anti-goals.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));

const RULES = [
  '# Do not do',
  '',
  'Intro text that is not a rule.',
  '',
  '## 2026-10-08 · Exports of the two banks',
  '',
  '- Decision: refused.',
  '- Cause: the person does not want bank files in the vault.',
  '- Match: bankb + export, banka + export, bank of america + export',
  '',
  '## 2026-09-27 · The h monogram',
  '',
  '- Decision: rejected as the mark.',
  '- Cause: the doorway silhouette is the identity.',
  '- Match: monogram',
].join('\n');

test('a rule is a dated heading with a Cause and a Match line', () => {
  const rules = parseRules(RULES);
  assert.equal(rules.length, 2);
  assert.equal(rules[0].date, '2026-10-08');
  assert.equal(rules[0].title, 'Exports of the two banks');
  assert.match(rules[0].cause, /bank files/);
  assert.deepEqual(rules[0].match, [['bankb', 'export'], ['banka', 'export'], ['bank of america', 'export']]);
  assert.deepEqual(rules[1].match, [['monogram']]);
});

test('a planted conflicting item is rejected with the rule and its date named', () => {
  const found = conflicts('Add BankB export to the Money tab', parseRules(RULES));
  assert.equal(found.length, 1);
  assert.match(found[0].message, /Exports of the two banks/);
  assert.match(found[0].message, /2026-10-08/);
  assert.match(found[0].message, /bank files/);
});

test('every part of an all-of term must be present, and an item that names none of a rule passes', () => {
  const rules = parseRules(RULES);
  assert.deepEqual(conflicts('Export the Hollow vault to a zip', rules), []);
  assert.deepEqual(conflicts('Show BankB balances', rules), []);
  assert.equal(conflicts('Bank of America statements: nightly EXPORTS', rules).length, 1);
});

test('matching is case-insensitive, starts at a word boundary and ignores a word inside another', () => {
  assert.equal(termMatches('Add a Monogram variant', ['monogram']), true);
  assert.equal(termMatches('Add monograms', ['monogram']), true);
  assert.equal(termMatches('Add a cryptomonogram', ['monogram']), false);
  assert.equal(termMatches('anything', []), false);
});

test('an item that conflicts with two rules reports both', () => {
  const found = conflicts('BankB export, drawn as an h monogram', parseRules(RULES));
  assert.equal(found.length, 2);
});

test('a rule with a bad date, no cause or no match is a problem', () => {
  const bad = ['## 2026-13-40 · Bad date', '- Cause: x', '- Match: y', '', '## 2026-10-01 · No cause', '- Match: y', '', '## 2026-10-02 · No match', '- Cause: x'].join('\n');
  const problems = checkRules(bad);
  assert.equal(problems.length, 3);
  assert.match(problems[0], /Bad date.*date/);
  assert.match(problems[1], /No cause.*Cause/);
  assert.match(problems[2], /No match.*Match/);
});

test('the command line rejects a planted item with the rule named, and passes a clean one', () => {
  const script = join(root, 'scripts/anti-goals.mjs');
  // docs/DO_NOT_DO.md was removed (ENG-674), so the CLI reads the fixture rules.
  const dir = mkdtempSync(join(tmpdir(), 'anti-goals-'));
  const rulesPath = join(dir, 'rules.md');
  writeFileSync(rulesPath, RULES);
  const bad = spawnSync(process.execPath, [script, '--rules', rulesPath, 'Add BankB export to the Money tab'], { encoding: 'utf8' });
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /rejected "Add BankB export to the Money tab"/);
  assert.match(bad.stderr, /2026-10-08/);
  const ok = spawnSync(process.execPath, [script, '--rules', rulesPath, 'Add a lane preflight script'], { encoding: 'utf8' });
  assert.equal(ok.status, 0);
  assert.match(ok.stdout, /no conflict/);
});
