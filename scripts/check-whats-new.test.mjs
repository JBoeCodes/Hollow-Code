import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { WHATS_NEW, staleInstallWords } from './check-whats-new.mjs';

test('the repository passes: What’s new never says a release is not installed', () => {
  assert.deepEqual(staleInstallWords(readFileSync(WHATS_NEW, 'utf8')), []);
});

test('"installed yet" is refused, also across a line wrap and as "not yet installed"', () => {
  const md = [
    '# What’s new: overnight',
    '',
    'What the night added. None of it is installed yet; it arrives with the next install.',
    'What the afternoon added. None of it is installed',
    'yet. These arrive with the next install.',
    'Not yet installed.',
  ].join('\n');
  const problems = staleInstallWords(md);
  assert.equal(problems.length, 3, problems.join('\n'));
  assert.match(problems[0], /^line 3: "installed yet"/);
  assert.match(problems[1], /^line 4: "installed yet"/);
  assert.match(problems[2], /^line 6: "yet installed"/);
});

test('"not in the installed app" is refused too, also across a line wrap (P0-05)', () => {
  const md = [
    '# What’s new: overnight 2026-10-20',
    '',
    'Not in the installed app until the next install.',
    'Merged after the 10-18 install; not in the installed',
    'app until the next one.',
    'It isn’t in the installed app, and it is not yet in the installed app either.',
  ].join('\n');
  const problems = staleInstallWords(md);
  assert.equal(problems.length, 4, problems.join('\n'));
  assert.match(problems[0], /^line 3: "Not in the installed app"/);
  assert.match(problems[1], /^line 4: "not in the installed app"/);
  assert.match(problems[2], /^line 6: "isn’t in the installed app"/);
  assert.match(problems[3], /^line 6: "not yet in the installed app"/);
});

test('a release that says what it added, and nothing about installs, passes', () => {
  const md = [
    'What the night of 10-13/14 added.',
    'Built overnight 2026-10-14/15; the build log and install receipts say which install has it.',
    "If Plaud's server isn't installed, the row says what to install. Yet another line.",
  ].join('\n');
  assert.deepEqual(staleInstallWords(md), []);
});
