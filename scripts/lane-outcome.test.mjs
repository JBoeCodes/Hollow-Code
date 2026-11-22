// BI2: a claim is not evidence. An unpushed branch and a missing merge each
// keep a row open with the reason; the board facts name ready, waiting,
// blocked and due rows.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { boardFacts, checkOutcome, gitRunner } from './lane-outcome.mjs';

/** A fake git: `table` maps the joined args prefix to `[code, stdout]`. */
const fake = table => args => {
  const key = args.join(' ');
  const hit = Object.entries(table).find(([k]) => key.startsWith(k));
  return hit ? { code: hit[1][0], stdout: hit[1][1] ?? '' } : { code: 1, stdout: '' };
};
const CLEAN = { 'status --porcelain': [0, ''] };

test('a merged, pushed, clean claim passes', () => {
  const git = fake({ ...CLEAN, 'cat-file': [0], 'merge-base': [0], 'show -s': [0, 'Add handoff (#1541)\n'] });
  assert.deepEqual(checkOutcome({ pr: '1541', sha: 'abc1234' }, git), { ok: true, reasons: [] });
});

test('a planted missing merge keeps the row open with the reason', () => {
  const git = fake({ ...CLEAN, 'cat-file': [0], 'merge-base': [1], 'show -s': [0, 'unrelated\n'] });
  const r = checkOutcome({ pr: '1541', sha: 'abc1234' }, git);
  assert.equal(r.ok, false);
  assert.deepEqual(r.reasons, ['commit abc1234 is not on origin/main', 'commit abc1234 subject does not name PR #1541']);
  const none = checkOutcome({ pr: '1541' }, fake({ ...CLEAN, log: [0, ''] }));
  assert.deepEqual(none.reasons, ['PR #1541 has no merge commit on origin/main']);
});

test('a planted unpushed branch keeps the row open with the count', () => {
  const git = fake({ ...CLEAN, 'rev-parse --verify --quiet refs/heads/o17b/x': [0, 'deadbeefcafe\n'], 'merge-base': [0], 'rev-list --count': [0, '2\n'] });
  const r = checkOutcome({ branch: 'o17b/x' }, git);
  assert.deepEqual(r.reasons, ['branch o17b/x has 2 unpushed commits']);
});

test('a dirty tree and an unknown branch are each a reason', () => {
  const git = fake({ 'status --porcelain': [0, ' M src/a.rs\n'] });
  const r = checkOutcome({ branch: 'gone' }, git);
  assert.deepEqual(r.reasons, ['branch gone exists neither here nor on origin', 'working tree is not clean (1 path, first: src/a.rs)']);
  assert.equal(checkOutcome({}, fake(CLEAN)).ok, false);
});

test('malformed claims are refused before git runs', () => {
  const never = () => assert.fail('git must not run');
  for (const claim of [{ pr: '1541; rm' }, { pr: '-1' }, { sha: 'HEAD' }, { sha: 'origin/main' }, { sha: '--all' }, { sha: 'ABCDEF1' }, { branch: '--upload-pack=x' }, { branch: 'a b' }]) {
    const r = checkOutcome(claim, never);
    assert.equal(r.ok, false, JSON.stringify(claim));
  }
  assert.equal(checkOutcome({ pr: '1' }, never, '--evil').ok, false);
});

test('against real git: an unpushed branch and a missing merge are caught, a pushed merge passes', () => {
  const root = mkdtempSync(join(tmpdir(), 'lane-outcome-'));
  const run = (cwd, ...a) => execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', ...a], { cwd, encoding: 'utf8' }).trim();
  const origin = join(root, 'origin.git');
  const work = join(root, 'work');
  mkdirSync(origin);
  run(origin, 'init', '--bare', '-b', 'main');
  run(root, 'clone', origin, work);
  writeFileSync(join(work, 'a.txt'), '1');
  run(work, 'add', '.');
  run(work, 'commit', '-m', 'first (#7)');
  run(work, 'push', '-u', 'origin', 'HEAD:main');
  const merged = run(work, 'rev-parse', 'HEAD');
  const git = gitRunner(work);
  writeFileSync(join(work, 'm.txt'), 'm');
  run(work, 'add', '.');
  run(work, 'commit', '-m', 'unrelated change', '-m', 'Follow-up to the work in (#9)');
  run(work, 'push', 'origin', 'HEAD:main');
  assert.match(checkOutcome({ pr: '9' }, git).reasons[0], /PR #9 has no merge commit/, 'a body mention is not a merge');
  assert.deepEqual(checkOutcome({ pr: '7' }, git), { ok: true, reasons: [] });
  assert.deepEqual(checkOutcome({ pr: '7', sha: merged }, git), { ok: true, reasons: [] });
  assert.match(checkOutcome({ pr: '8' }, git).reasons[0], /PR #8 has no merge commit/);
  run(work, 'checkout', '-b', 'feat');
  writeFileSync(join(work, 'b.txt'), '2');
  run(work, 'add', '.');
  run(work, 'commit', '-m', 'second');
  const r = checkOutcome({ branch: 'feat' }, git);
  assert.deepEqual(r.reasons, ['branch feat (' + run(work, 'rev-parse', 'HEAD').slice(0, 9) + ') is not merged into origin/main', 'branch feat has 1 unpushed commit']);
});

const BOARD = `# Board

| Row | Status | Depends | Due | Note |
|---|---|---|---|---|
| A1 | done | none | | shipped |
| A2 | ready | A1 | 2026-11-20 | ok |
| A3 | ready | A4 | | waits on A4 |
| A4 | open | none | 2026-11-17 | due today |
| A5 | blocked | none | | needs key |
| A6 | waiting | none | 2026-11-01 | late |
| A7 | merged | | 2026-11-01 | not due: done |

| Other | Thing |
|---|---|
| x | y |
`;

test('board facts list ready, waiting, blocked and due rows, and the next one', () => {
  const f = boardFacts(BOARD, '2026-11-17T12:00:00Z');
  assert.deepEqual(f, { ready: ['A2', 'A4'], waiting: ['A3', 'A6'], blocked: ['A5'], due: ['A4', 'A6'], next: 'A2', done: ['A1', 'A7'] });
});

test('a board with no Status column yields nothing', () => {
  assert.deepEqual(boardFacts('| a | b |\n|---|---|\n| 1 | 2 |\n', '2026-11-17').ready, []);
});
