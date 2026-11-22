// Overnight 10-17 S07: the replay bench's report
// (scripts/agent-eval/replay-bench.mjs) over a bench JSON shaped like
// src-tauri/src/replay_bench.rs's output (numbers from the recorded
// baseline, replayed side made up).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { PARTS, UNKNOWN_PART, formatReplayBench, main, partLines, partMapLines, summaryLine } from './agent-eval/replay-bench.mjs';

const FIXTURE = fileURLToPath(new URL('./agent-eval/fixtures/replay-bench/bench.json', import.meta.url));
const bench = () => JSON.parse(readFileSync(FIXTURE, 'utf8'));
const rust = readFileSync(new URL('../src-tauri/src/replay_bench.rs', import.meta.url), 'utf8');
const resume = readFileSync(new URL('../src-tauri/src/chat_resume.rs', import.meta.url), 'utf8');

test('the part vocabulary is OB03\'s (chat_resume.rs), in its order, and the bench\'s', () => {
  const block = resume.match(/pub\(crate\) const PARTS: \[&str; \d+\] = \[([^\]]*)\]/);
  assert.ok(block, 'PARTS in chat_resume.rs');
  assert.deepEqual([...block[1].matchAll(/"(\w+)"/g)].map((m) => m[1]), PARTS);
  assert.ok(PARTS.includes('contract') && PARTS.includes('skills'));
  assert.match(resume, new RegExp(`const UNKNOWN_PART: &str = "${UNKNOWN_PART}";`));
  assert.equal(UNKNOWN_PART, 'unknown');
  // The bench names resets with chat_resume's, never a copy of its own.
  assert.match(rust, /pub\(crate\) use crate::chat_resume::\{PARTS, UNKNOWN_PART\};/);
  assert.doesNotMatch(rust, /const (PARTS|UNKNOWN_PART):/);
});

test('each bucket prints recorded → replayed with its n', () => {
  const lines = formatReplayBench(bench());
  assert.equal(lines[0], 'Replay bench (schema 1), window 2026-10-09T00:00:00Z → 2026-10-16T21:16:00Z: recorded → replayed');
  for (const line of [
    'Claude: 20.2 → 5.35 config resets per 100 turns (n = 505 turns; 102 → 27 resets)',
    'Claude flips: 14.85 → 0 per 100 turns (n = 505 turns; 75 → 0 resets; at a mode change 75 → 4)',
    'Main agent: 24.77 → 0 config resets per 100 turns (n = 109 turns; 27 → 0 resets)',
    'Main agent flips: 18.35 → 0 per 100 turns (n = 109 turns; 20 → 0 resets; at a mode change 20 → 0)',
    'All agents: 17.74 → 5 config resets per 100 turns (n = 620 turns; 110 → 31 resets)',
    'Main-agent wakes that reset the session: 34 → 0 of 80',
    'Resets on a filled-in part: Claude 30 → 6, all agents 33 → 7',
    'Not counted: 2 skipped turns; recorded resets 1 before any turn, 3 at a turn before the window, 0 at a skipped turn',
    'Manifests: 200 of 620 counted turns (pool 500; 0 unreadable)',
    'Assumed, of 620 turns: model 12, mode 0, agent 12, servers 380 nearest / 40 default, Canvases 390 nearest / 40 default, folder 620, teammates 560',
    'Snapshot: quick_check ok, deleted true',
  ]) {
    assert.ok(lines.includes(line), `missing: ${line}\n${lines.join('\n')}`);
  }
});

test('parts print in the fingerprint\'s order, unknown last, names this file doesn\'t know after', () => {
  const lines = formatReplayBench(bench());
  const at = lines.indexOf('Claude resets by first part:');
  assert.ok(at > 0);
  assert.deepEqual(lines.slice(at + 1, at + 6), [
    '  contract: 0 → 1',
    '  model: 3 → 17',
    '  mode: 75 → 0',
    '  mcp: 0 → 9',
    '  unknown: 24 → 0',
  ]);
  // Every differing part, over every conversation: `mcp` that `mode` hid.
  const every = lines.indexOf('All resets by every differing part:');
  assert.equal(every, at + 6);
  assert.deepEqual(lines.slice(every + 1, every + 8), [
    '  contract: 0 → 1',
    '  model: 3 → 19',
    '  mode: 79 → 0',
    '  mcp: 12 → 11',
    '  caps: 12 → 11',
    '  skills: 0 → 1',
    '  unknown: 28 → 0',
  ]);
  // A name from before OB03 (`config`) is no part: it prints after them.
  assert.deepEqual(partMapLines({ config: 2, unknown: 1 }, {}), ['  unknown: 1 → 0', '  config: 2 → 0']);
  assert.deepEqual(partMapLines(undefined, { mode: 1 }), ['  mode: 0 → 1']);
  const extra = partLines({ recorded: { byPart: { zeta: 1, mode: 2 } }, replayed: { byPart: { alpha: 1 } } });
  assert.deepEqual(extra, ['  mode: 2 → 0', '  alpha: 0 → 1', '  zeta: 1 → 0']);
  assert.deepEqual(partLines(undefined), []);
});

test('a second file compares the recorded and replayed rates; the morning line is one line', () => {
  const other = bench();
  other.claude.recorded.per100 = 19.5;
  other.claude.replayed.per100 = 4.1;
  other.claude.replayed.flipsPer100 = 0.2;
  const lines = formatReplayBench(bench(), other);
  assert.deepEqual(lines.slice(-2), [
    'Recorded per 100 turns, this → other: Claude 20.2 → 19.5, Main agent 24.77 → 24.77, Claude flips 14.85 → 14.85',
    'Replayed per 100 turns, this → other: Claude 5.35 → 4.1, Main agent 0 → 0, Claude flips 0 → 0.2',
  ]);
  assert.equal(
    summaryLine(bench()),
    'Session resets per 100 turns, recorded → replayed: Claude 20.2 → 5.35 (n = 505), Main agent 24.77 → 0 (n = 109), mode flips 14.85 → 0',
  );
  // A report missing a bucket says so instead of throwing.
  assert.ok(formatReplayBench({ schema: 1 }).includes('Claude: no data'));
});

test('the command prints the report and refuses bad arguments', () => {
  const out = [];
  const err = [];
  assert.equal(main([FIXTURE], (l) => out.push(l), (l) => err.push(l)), 0);
  assert.match(out.join('\n'), /Claude: 20\.2 → 5\.35/);
  assert.equal(main([], (l) => out.push(l), (l) => err.push(l)), 2);
  assert.equal(main([`${FIXTURE}.missing`], (l) => out.push(l), (l) => err.push(l)), 1);
  assert.match(err.join('\n'), /usage:[\s\S]*replay-bench:/);
});
