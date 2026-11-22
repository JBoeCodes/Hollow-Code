import { fileURLToPath } from 'node:url';
// Lane reports → one morning summary (overnight D8).
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import test from 'node:test';
import { heading, labelOf, parseReport, summarize } from './overnight-summary.mjs';

const LANE_A = `# Overnight, lane A

## A1 Resume after restart — merged

- **PR:** [#422](https://github.com/example-org/hollow-code/pull/422), merged with a merge commit as main **\`f02dd3dd\`**.
- **Decisions:**
  - no Dismiss button;
  - a 24 h window.
- **Left:** depth 0 on resume.
- **Untested by hand:** a real quit mid-turn.

## D6 Weekend option — already built

- Nothing to change.

## A4 notes (for the successor)

- **Left:** this is not an item.
`;

const LANE_B = `# Overnight, lane B

## B1 · Workspace merge (#18) — merged

**Done.** [#423](https://github.com/example-org/hollow-code/pull/423), merged with a merge commit as main
\`b194ba45\` (after lane A).

- **Reviews:**
  - Documented, not fixed: a pushed-aside folder can split.
- **Decisions taken (no questions):**
  - **Undo is a merge record**, not History.
- **Accepted:** card timeouts are by design.

## S1 · part 2 merged (N4)

[#433](https://github.com/example-org/hollow-code/pull/433), main \`5424725e\`.

### S5 + S6 — merged

[#438](https://github.com/example-org/hollow-code/pull/438), main \`3e806b15\`.

- **Decision:** one checker. It skips:
  - binary files;
  - fixtures.

## B6 · Side-pane chat — skipped

## Clock

Nothing here is an item.
`;

test('headings: id, title, status; a section that is not an item is skipped', () => {
  assert.deepEqual(heading('A1', 'Resume after restart — merged'), { id: 'A1', title: 'Resume after restart', status: 'merged' });
  assert.deepEqual(heading('S5 + S6', '— merged'), { id: 'S5 + S6', title: 'S5 + S6', status: 'merged' });
  assert.deepEqual(heading('S1', 'part 2 merged (N4)'), { id: 'S1', title: 'part 2 merged (N4)', status: 'merged' });
  assert.equal(heading('A4', 'notes (for the successor)'), null);
});

test('labels the lanes write, bold or plain, and nothing else', () => {
  assert.deepEqual(labelOf('- **Left:** depth 0'), { depth: 0, kind: 'open', rest: 'depth 0' });
  assert.deepEqual(labelOf('  - Documented, not fixed: a split'), { depth: 2, kind: 'open', rest: 'a split' });
  assert.equal(labelOf('- **Decisions taken (no questions):**').kind, 'decisions');
  assert.equal(labelOf('- **Untested live:** x').kind, 'untested');
  assert.equal(labelOf('- **Gate:** green'), null);
  assert.equal(labelOf('- **Undo is a merge record**, not History.'), null);
});

test('items keep their PRs, wrapped commits, decisions, open findings and untested lists', () => {
  const a = parseReport('lane-a.md', LANE_A);
  assert.deepEqual(a.map((i) => i.id), ['A1', 'D6']);
  assert.deepEqual(a[0].prs, ['422']);
  assert.equal(a[0].commit, 'f02dd3dd');
  assert.deepEqual(a[0].decisions, ['no Dismiss button;', 'a 24 h window.']);
  assert.deepEqual(a[0].open, ['depth 0 on resume.']);
  assert.deepEqual(a[0].untested, ['a real quit mid-turn.']);
  const b = parseReport('lane-b.md', LANE_B);
  assert.deepEqual(b.map((i) => i.id), ['B1', 'S1', 'S5 + S6', 'B6']);
  assert.equal(b[0].commit, 'b194ba45', 'main and its hash across a line break');
  assert.deepEqual(b[0].open, ['a pushed-aside folder can split.', 'card timeouts are by design.']);
  assert.deepEqual(b[0].decisions, ['Undo is a merge record, not History.']);
  assert.deepEqual(b[2].decisions, ['one checker. It skips: binary files; fixtures.']);
});

test('the summary: merged one line each, the rest, and the three lists', () => {
  const text = summarize(
    [{ name: 'lane-a.md', text: LANE_A }, { name: 'lane-b.md', text: LANE_B }],
    '2026-10-10',
    new Date('2026-10-10T11:50:00Z'),
  );
  assert.match(text, /^# Overnight 2026-10-10: morning summary\n/);
  assert.match(text, /at 2026-10-10 11:50 UTC: 4 merged, 2 other\./);
  assert.match(text, /\n- \*\*A1\*\* \(A\) Resume after restart: #422 `f02dd3dd`\n/);
  assert.match(text, /\n- \*\*S5 \+ S6\*\* \(B\): #438 `3e806b15`\n/);
  assert.match(text, /## Not merged\n\n- \*\*D6\*\* \(A\) Weekend option: already built\n- \*\*B6\*\* \(B\) Side-pane chat: skipped\n/);
  assert.match(text, /## Decisions taken \(4\)/);
  assert.match(text, /## Open review findings \(not fixed\) \(3\)/);
  assert.match(text, /## Untested by hand \(1\)\n\n- \*\*A1\*\* \(A\): a real quit mid-turn\.\n/);
  assert.doesNotMatch(text, /not an item|Nothing here/);
});

test('the command writes the summary from a folder and refuses one with no reports', () => {
  const dir = mkdtempSync(join(tmpdir(), 'overnight-2026-10-10-'));
  try {
    writeFileSync(join(dir, 'lane-a.md'), LANE_A);
    writeFileSync(join(dir, 'notes.md'), '## Z9 Not a lane — merged\n');
    const out = join(dir, 'SUMMARY.md');
    execFileSync(process.execPath, [fileURLToPath(new URL('./overnight-summary.mjs', import.meta.url)), dir, '--out', out]);
    const text = readFileSync(out, 'utf8');
    assert.match(text, /# Overnight 2026-10-10: morning summary/);
    assert.match(text, /\*\*A1\*\* \(A\)/);
    assert.doesNotMatch(text, /Z9/);
    rmSync(join(dir, 'lane-a.md'));
    const none = spawnSync(process.execPath, [fileURLToPath(new URL('./overnight-summary.mjs', import.meta.url)), dir], { encoding: 'utf8' });
    assert.equal(none.status, 1);
    assert.match(none.stderr, /No lane-\*\.md in /);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

const SCRIPT = fileURLToPath(new URL('./overnight-summary.mjs', import.meta.url));

test('the lanes\' other labels: Alex notes, not built, documented, open; a label needs its colon', () => {
  const text = `## A8 Conflict markers — merged

- **PR:** [#439](https://github.com/example-org/hollow-code/pull/439), merged with a merge commit as main **\`212ea088\`**.
- **Decision:** don't keep two checkers. A8 widens [#438](https://github.com/example-org/hollow-code/pull/438)'s check.
- **For Owner:** the original checkout's worktrees appear in the list.
- **Not built:** stage 2.
- **Limit (decision):** a Wry handle only.
- **Documented residual:** the CLIs' own memory files.
- **Decision-free and next:** N3.
- Palette **Open workspace map** opens the map.

**Open:** confirm both skills are indexed.

## S7, S8 — skipped
`;
  const [a8, s7] = parseReport('lane-a.md', text);
  assert.deepEqual(a8.prs, ['439'], 'a PR the prose mentions is not the item\'s');
  assert.deepEqual(a8.alex, ["the original checkout's worktrees appear in the list."]);
  assert.deepEqual(a8.decisions, ["don't keep two checkers. A8 widens [#438](https://github.com/example-org/hollow-code/pull/438)'s check.", 'a Wry handle only.']);
  assert.deepEqual(a8.open, ['stage 2.', "the CLIs' own memory files.", 'confirm both skills are indexed.']);
  assert.deepEqual({ id: s7.id, status: s7.status }, { id: 'S7, S8', status: 'skipped' });
  const summary = summarize([{ name: 'lane-a.md', text }], '2026-10-10', new Date('2026-10-10T11:50:00Z'));
  assert.match(summary, /## For Owner \(1\)\n\n- \*\*A8\*\* \(A\): the original checkout/);
  assert.doesNotMatch(summary, /N3/);
});

test('a long list under a lead-in keeps every row', () => {
  const long = 'x'.repeat(120);
  const text = `## S5 Cross-PR review — merged

- **Left:** listed in the build log entry:
  - a ${long};
  - b ${long};
  - the mock for mutations is missing.
`;
  const [s5] = parseReport('lane-a.md', text);
  assert.equal(s5.open.length, 3);
  assert.equal(s5.open[2], 'the mock for mutations is missing.');
});

test('the command runs through a symlink and says what is wrong with its arguments', () => {
  const dir = mkdtempSync(join(tmpdir(), 'overnight-2026-10-10-'));
  try {
    writeFileSync(join(dir, 'lane-a.md'), LANE_A);
    const link = join(dir, 'summary-link.mjs');
    symlinkSync(SCRIPT, link);
    const run = spawnSync(process.execPath, [link, dir], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /\*\*A1\*\* \(A\)/);
    const before = spawnSync(process.execPath, [SCRIPT, '--out', join(dir, 'S.md'), dir], { encoding: 'utf8' });
    assert.equal(before.status, 0, before.stderr);
    assert.match(readFileSync(join(dir, 'S.md'), 'utf8'), /morning summary/);
    const missing = spawnSync(process.execPath, [SCRIPT, join(dir, 'nope')], { encoding: 'utf8' });
    assert.equal(missing.status, 2);
    assert.match(missing.stderr, /is not a folder/);
    const bare = spawnSync(process.execPath, [SCRIPT, dir, '--out'], { encoding: 'utf8' });
    assert.equal(bare.status, 2);
    assert.match(bare.stderr, /^usage:/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
