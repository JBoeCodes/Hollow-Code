// Tests for scripts/release/notes.mjs (updates row U2): the port of
// install_brings.rs's PR parsing and build-log notes, the What's new diff,
// and notes between two commits of a temp repo with PR merges.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { collectNotes, notesFor, notesText, prOf, untestedLines, whatsNewEntries } from './release/notes.mjs';

const GIT_ENV = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1', GIT_AUTHOR_NAME: 'T', GIT_AUTHOR_EMAIL: 't@example.test', GIT_COMMITTER_NAME: 'T', GIT_COMMITTER_EMAIL: 't@example.test' };
const g = (cwd, ...args) => execFileSync('git', args, { cwd, env: GIT_ENV, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const put = (root, path, text) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), text); };

test('prOf reads both merge subject shapes like install_brings::pr_of', () => {
  assert.deepEqual(prOf('Merge #451: Fix the thing', ''), { number: 451, title: 'Fix the thing' });
  assert.deepEqual(prOf('Merge pull request #12 from me/branch', '\nAdd the frobnicator\n\nmore'), { number: 12, title: 'Add the frobnicator' });
  assert.deepEqual(prOf('Merge pull request #12 from me/branch', ''), { number: 12, title: 'Merge pull request #12 from me/branch' });
  assert.equal(prOf('Merge #x: nope', ''), null);
  assert.equal(prOf('Merge #45 no colon', ''), null);
  assert.equal(prOf('Fix a typo', ''), null);
});

test('notesFor finds the entry by overnight tag or title and keeps its untested sentences', () => {
  const entries = [
    '2026-11-15 · Add the frobnicator\n\n- **What:** it frobs. Not checked by hand on a real Mac. It works.\n- Verified: tests.',
    '2026-11-14 · Something (overnight S12)\n\nThe dialog wasn\'t tested with VoiceOver. Fine otherwise.',
  ];
  assert.deepEqual(notesFor(entries, 'Add the frobnicator'), { found: true, untested: ['Not checked by hand on a real Mac.'] });
  assert.deepEqual(notesFor(entries, 'Other title (overnight S12)'), { found: true, untested: ["The dialog wasn't tested with VoiceOver."] });
  assert.deepEqual(notesFor(entries, 'Unknown change'), { found: false, untested: [] });
  assert.equal(untestedLines('h\n- a. b. c\n- Never tested one.\n- Never tested two.\n- Never tested three.\n- Never tested four.').length, 3);
});

test('whatsNewEntries takes each ### heading with its What line', () => {
  const text = '# What\'s new: 2026-11-15\n\n### One\n\n- **What:** first.\n- **Where:** x\n\n### Two\n\n- **What:** second.\n';
  assert.deepEqual(whatsNewEntries(text), [{ title: 'One', what: 'first.' }, { title: 'Two', what: 'second.' }]);
});

test('notes between two commits list only the PRs, commits and What\'s new entries added since', (t) => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-notes-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  g(root, 'init', '-q', '-b', 'main');
  put(root, 'docs/WHATS_NEW.md', '# What\'s new\n\n### Old feature\n\n- **What:** old.\n');
  put(root, 'docs/BUILD_LOG.md', '# Build log\n');
  g(root, 'add', '.');
  g(root, 'commit', '-q', '-m', 'Start');
  const from = g(root, 'rev-parse', 'HEAD');

  g(root, 'checkout', '-q', '-b', 'feature');
  put(root, 'docs/WHATS_NEW.md', '# What\'s new\n\n### Frobnicate notes\n\n- **What:** notes frob.\n\n### Old feature\n\n- **What:** old.\n');
  put(root, 'docs/BUILD_LOG.md', '# Build log\n\n## 2026-11-15 · Add the frobnicator\n\n- Not run on the mini yet.\n');
  g(root, 'add', '.');
  g(root, 'commit', '-q', '-m', 'Frob');
  g(root, 'checkout', '-q', 'main');
  g(root, 'merge', '--no-ff', '-q', 'feature', '-m', 'Merge pull request #77 from me/feature', '-m', 'Add the frobnicator');
  put(root, 'README', 'x');
  g(root, 'add', '.');
  g(root, 'commit', '-q', '-m', 'Direct fix');
  const to = g(root, 'rev-parse', 'HEAD');

  const notes = collectNotes({ git: (args) => g(root, ...args), from, to });
  assert.equal(notes.note, null);
  assert.deepEqual(notes.pullRequests.map(p => [p.number, p.title, p.untested]), [[77, 'Add the frobnicator', ['Not run on the mini yet.']]]);
  assert.deepEqual(notes.commits.map(c => c.subject), ['Direct fix'], 'the feature branch commit is not on the first-parent line');
  assert.deepEqual(notes.whatsNew, [{ title: 'Frobnicate notes', what: 'notes frob.' }]);
  const text = notesText(notes);
  assert.match(text, /Frobnicate notes: notes frob\./);
  assert.match(text, /#77 Add the frobnicator/);
  assert.doesNotMatch(text, /Old feature/);

  const first = collectNotes({ git: (args) => g(root, ...args), from: null, to });
  assert.match(first.note, /No previous build/);
  assert.equal(first.commits.length + first.pullRequests.length, 3);
  assert.ok(notesText({ ...notes, commits: Array.from({ length: 500 }, (_, i) => ({ subject: `commit number ${i} with words` })) }, 300).length <= 300);
});
