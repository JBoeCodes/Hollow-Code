// The replay bench's report (overnight 10-17 S07): config-changed session
// resets as the Sessions DB recorded them → as HEAD's resume plan replays
// the same turns. It reads the JSON `src-tauri/src/replay_bench.rs` writes
// (its ignored real run; the command is in docs/reference/build-and-ship.md):
//
//   node scripts/agent-eval/replay-bench.mjs <bench.json> [<other bench.json>]
//
// One line per bucket (Claude conversations, the Main agent's, every
// conversation) with its counted turns, the mode flips for Claude and the
// Main agent (resets with `mode` among their differing parts, and how many
// resets merely sat at a turn whose mode changed), the Main agent's wakes,
// the Claude resets by the first part of the configuration that changed
// (OB03's fingerprint parts, `unknown` when none can be named; PARTS order,
// so `mode` hides `mcp`),
// every conversation's resets by every part that changed, the resets
// resting on a filled-in part, and what the bench couldn't count (skipped
// turns, unmatched closes). With a second file, the recorded and replayed
// rates are also compared, this file → the other (a run before a change and
// one after). The morning report can
// import `summaryLine` for its one line. The bench's JSON is numbers only
// (no session key, path or text), and this file only reads it.
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

/** The fingerprint parts a reset is named by (OB03's `chat_resume::PARTS`,
 *  which replay_bench.rs re-exports). */
export const PARTS = ['contract', 'agent', 'model', 'mode', 'cwd', 'mcp', 'caps', 'canvases', 'workspace', 'skills'];
/** A reset none of PARTS explains (`chat_resume::UNKNOWN_PART`). */
export const UNKNOWN_PART = 'unknown';

const num = (x) => (Number.isFinite(x) ? String(x) : '?');
const arrow = (a, b) => `${num(a)} → ${num(b)}`;

/** `label: recorded → replayed config resets per 100 turns (n = …)`. */
export function bucketLine(label, bucket) {
  if (!bucket) return `${label}: no data`;
  const { turns, recorded = {}, replayed = {} } = bucket;
  return `${label}: ${arrow(recorded.per100, replayed.per100)} config resets per 100 turns (n = ${num(turns)} turns; ${arrow(recorded.resets, replayed.resets)} resets)`;
}

/** The bucket's mode-flip resets (`mode` among the differing parts),
 *  recorded → replayed, and the resets at a turn whose mode changed. */
export function flipLine(label, bucket) {
  if (!bucket) return `${label} flips: no data`;
  const { turns, recorded = {}, replayed = {} } = bucket;
  return (
    `${label} flips: ${arrow(recorded.flipsPer100, replayed.flipsPer100)} per 100 turns (n = ${num(turns)} turns; ` +
    `${arrow(recorded.flips, replayed.flips)} resets; at a mode change ${arrow(recorded.modeChangedResets, replayed.modeChangedResets)})`
  );
}

/** One `  part: recorded → replayed` line per part either side counted
 *  in the bucket's `byPart` (each reset by its first differing part). */
export function partLines(bucket) {
  return partMapLines(bucket?.recorded?.byPart, bucket?.replayed?.byPart);
}

/** One `  part: recorded → replayed` line per part either map counts,
 *  PARTS order, then `unknown`, then any part this file doesn't know. */
export function partMapLines(recordedMap, replayedMap) {
  const recorded = recordedMap ?? {};
  const replayed = replayedMap ?? {};
  const known = [...PARTS, UNKNOWN_PART];
  const extra = [...new Set([...Object.keys(recorded), ...Object.keys(replayed)])].filter((p) => !known.includes(p)).sort();
  return [...known, ...extra]
    .filter((part) => (recorded[part] ?? 0) > 0 || (replayed[part] ?? 0) > 0)
    .map((part) => `  ${part}: ${arrow(recorded[part] ?? 0, replayed[part] ?? 0)}`);
}

/** The report's lines; with `other`, also the replayed rates this → other. */
export function formatReplayBench(report, other = null) {
  const out = [];
  const window = report.window ? `, window ${report.window.from} → ${report.window.to}` : '';
  out.push(`Replay bench (schema ${num(report.schema)})${window}: recorded → replayed`);
  out.push(bucketLine('Claude', report.claude));
  out.push(flipLine('Claude', report.claude));
  out.push(bucketLine('Main agent', report.mainAgent));
  out.push(flipLine('Main agent', report.mainAgent));
  out.push(bucketLine('All agents', report.all));
  const wakes = report.mainWakes;
  if (wakes) {
    out.push(`Main-agent wakes that reset the session: ${arrow(wakes.recordedResets, wakes.replayedResets)} of ${num(wakes.turns)}`);
  }
  const parts = partLines(report.claude);
  if (parts.length) out.push('Claude resets by first part:', ...parts);
  const every = partMapLines(report.recordedByPartAll, report.replayedByPartAll);
  if (every.length) out.push('All resets by every differing part:', ...every);
  if (report.claude || report.all) {
    const filled = (bucket) => arrow(bucket?.recorded?.assumedPartResets, bucket?.replayed?.assumedPartResets);
    out.push(`Resets on a filled-in part: Claude ${filled(report.claude)}, all agents ${filled(report.all)}`);
  }
  if ('skippedTurns' in report || report.unmatched) {
    const u = report.unmatched ?? {};
    out.push(
      `Not counted: ${num(report.skippedTurns)} skipped turns; recorded resets ${num(u.noTurn)} before any turn, ` +
        `${num(u.preWindowTurn)} at a turn before the window, ${num(u.skippedTurn)} at a skipped turn`,
    );
  }
  const m = report.manifests;
  if (m) {
    out.push(`Manifests: ${num(m.turnsWithManifest)} of ${num(m.turns)} counted turns (pool ${num(m.pool)}; ${num(m.unreadable)} unreadable)`);
  }
  const a = report.assumed;
  if (a) {
    out.push(
      `Assumed, of ${num(a.turns)} turns: model ${num(a.model)}, mode ${num(a.perm)}, agent ${num(a.agentFromSession)}, ` +
        `servers ${num(a.mcpFromSession)} nearest / ${num(a.mcpDefault)} default, ` +
        `Canvases ${num(a.canvasesFromSession)} nearest / ${num(a.canvasesDefault)} default, ` +
        `folder ${num(a.cwdSessionLevel)}, teammates ${num(a.teammatesInstalled)}`,
    );
  }
  if ('snapshotDeleted' in report) {
    out.push(`Snapshot: quick_check ${report.snapshotQuickCheck ?? '?'}, deleted ${report.snapshotDeleted}`);
  }
  if (other) {
    for (const [label, side] of [
      ['Recorded', 'recorded'],
      ['Replayed', 'replayed'],
    ]) {
      const rate = (r, key, field = 'per100') => r?.[key]?.[side]?.[field];
      out.push(
        `${label} per 100 turns, this → other: Claude ${arrow(rate(report, 'claude'), rate(other, 'claude'))}, ` +
          `Main agent ${arrow(rate(report, 'mainAgent'), rate(other, 'mainAgent'))}, ` +
          `Claude flips ${arrow(rate(report, 'claude', 'flipsPer100'), rate(other, 'claude', 'flipsPer100'))}`,
      );
    }
  }
  return out;
}

/** The morning report's one line: recorded → replayed per 100 turns. */
export function summaryLine(report) {
  const c = report.claude ?? {};
  const m = report.mainAgent ?? {};
  return (
    `Session resets per 100 turns, recorded → replayed: Claude ${arrow(c.recorded?.per100, c.replayed?.per100)} (n = ${num(c.turns)}), ` +
    `Main agent ${arrow(m.recorded?.per100, m.replayed?.per100)} (n = ${num(m.turns)}), ` +
    `mode flips ${arrow(c.recorded?.flipsPer100, c.replayed?.flipsPer100)}`
  );
}

/** A bench JSON file, parsed. */
export function readBench(file) {
  return JSON.parse(readFileSync(file, 'utf8'));
}

export function main(argv, log = console.log, error = console.error) {
  if (argv.length < 1 || argv.length > 2) {
    error('usage: node scripts/agent-eval/replay-bench.mjs <bench.json> [<other bench.json>]');
    return 2;
  }
  let report;
  let other = null;
  try {
    report = readBench(argv[0]);
    if (argv[1]) other = readBench(argv[1]);
  } catch (e) {
    error(`replay-bench: ${e.message}`);
    return 1;
  }
  log(formatReplayBench(report, other).join('\n'));
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
