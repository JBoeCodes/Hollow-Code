import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { checkRepo, checkSource, checkReleaseNames, glossaryTable, glossaryTableIn, PLAIN_WORDS, WORDS } from './check-plain-words.mjs';

test('recent release instructions use the current surface names (P1-35)', () => {
  const md = readFileSync(new URL('../docs/WHATS_NEW.md', import.meta.url), 'utf8');
  assert.deepEqual(checkReleaseNames(md), []);
  const old = "# What's new: 2026-10-13\nYour work → Activity";
  assert.deepEqual(checkReleaseNames(old), []);
  assert.equal(checkReleaseNames("# What's new: 2026-10-25\nYour work → Activity").length, 1);
  assert.equal(checkReleaseNames("# What's new: 2026-11-04\nWork → Activity").length, 1);
  assert.equal(checkReleaseNames("# What's new: overnight 2026-10-14/15\nMission Control").length, 1);
});

test('the glossary and PLAIN_WORDS agree: one list writes both (P1-03)', () => {
  const glossary = readFileSync(new URL('../docs/reference/glossary.md', import.meta.url), 'utf8');
  assert.equal(glossaryTableIn(glossary), glossaryTable(), 'run node scripts/check-plain-words.mjs --glossary');
  assert.deepEqual(PLAIN_WORDS.map(([word]) => word), WORDS.filter(w => w.ban).map(w => w.ban));
  for (const [word] of PLAIN_WORDS) assert.ok(WORDS.find(w => w.ban === word && glossary.includes(`| ${w.say} | ${w.dont} |`)), String(word));
});

test('a lower-case fallback after || or ?? is read (P1-03)', () => {
  const problems = checkSource('src/x.tsx', [
    'const a = `What it brings couldn’t be listed: ${err() || \'the build has no comparison receipt\'}.`;',
    "const b = label ?? 'no session key yet';",
    "const c = x || 'receipt'; // one word, not a sentence",
    "const d = ok ? 'the session key' : 'x'; // a branch of ?: outside JSX is still not read",
    "label ??= 'no session key yet';",
  ].join('\n'));
  assert.deepEqual(problems.map(p => p.split(':')[1]), ['1', '2', '5'], problems.join('\n'));
});

test('a toast reads as a sentence: no "+ " token and no lower-case start (P1-04)', () => {
  const problems = checkSource('src/x.tsx', [
    "flashFooter('copied', 1000);",
    'flashFooter(`+ automation ${w.title}`, 1800);',
    "flashFooter(ok ? 'Saved' : 'couldn’t save');",
    "flashFooter(err || 'skill deleted');",
    "flashFooter('Copied', 1000);",
    'flashFooter(`${n} files copied`); // starts with a value: not judged',
    "flashFooter(message);",
    "flashFooter('× Couldn’t open it');",
  ].join('\n'));
  assert.deepEqual(problems.map(p => p.split(':')[1]), ['1', '2', '3', '4'], problems.join('\n'));
  assert.match(problems[0], /toast "copied"/);
});

test('P1-04 retires workflow, ingest, night shift and Vault History in shown text', () => {
  const problems = checkSource('src/x.tsx', [
    'const a = <p>Delete this workflow</p>;',
    'const b = <button>Quick ingest</button>;',
    "const c = 'Tonight’s night shift starts at 22:00';",
    "const d = 'Open Vault History';",
  ].join('\n'));
  assert.deepEqual(problems.map(p => p.split(':')[1]), ['1', '2', '3', '4'], problems.join('\n'));
});

test('the repository passes: no engineering words in what Hollow says', () => {
  assert.deepEqual(checkRepo(), []);
});

test('shown sentences are read: JSX text, labels, notices and messages built from templates', () => {
  const problems = checkSource('src/x.tsx', [
    'const a = <p>Show the run’s receipt</p>;',
    'const b = <Button label="Loading the receipt" />;',
    "flashFooter('Session id was not saved');",
    'const c = `Run ${n} wrote to stderr twice`;',
  ].join('\n'));
  assert.equal(problems.length, 4, problems.join('\n'));
  assert.match(problems[0], /^src\/x\.tsx:1: "receipt"/);
});

test('a lower-case fragment placed straight into JSX is read', () => {
  const problems = checkSource('src/x.tsx', [
    "const a = <p>{n} files changed{scope ? '; repository changes are in the run’s receipt' : ''}</p>;",
    "const b = <p>{busy && 'waiting for the session key'}</p>;",
    "const c = cond ? 'the session key' : 'x'; // not JSX content",
    "const d = <p title={c ? 'the session key' : ''}>x</p>; // an attribute: lower case isn't a sentence",
    "const e = <p>{'the session key' && 'ok go'}</p>; // the condition side isn't shown",
    "const f = <style>{`.receipt-panel { border: 0 }`}</style>;",
  ].join('\n'));
  assert.deepEqual(problems.map(p => p.split(':')[1]), ['1', '2'], problems.join('\n'));
});

test('code that is never shown is not read', () => {
  const problems = checkSource('src/x.tsx', [
    "import { receipt } from './Receipt Panel';",
    'const a = <div class="execution-receipt open" data-kind="Run receipt">x</div>;',
    "if (kind === 'Run receipt') {}",
    "const v = api.listEvidenceReceipts('Run receipt', 1);",
    "switch (k) { case 'Run receipt': break; }",
    "const k = 'receipt'; // one word, not a sentence",
  ].join('\n'));
  assert.deepEqual(problems, []);
});

test('an allowed agent-facing sentence passes only in its own file', () => {
  const line = "throw new Error('navigate: session needs the session key.');";
  assert.deepEqual(checkSource('src/appBridge.ts', line), []);
  assert.equal(checkSource('src/pages/Home.tsx', line).length, 0, 'lower-case start is not a sentence');
  assert.equal(checkSource('src/pages/Home.tsx', "toast('The session key is missing.');").length, 1);
});
