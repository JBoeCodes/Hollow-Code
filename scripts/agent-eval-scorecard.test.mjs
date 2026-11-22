// Evening 2026-10-16 HF01: the free efficiency scorecard
// (scripts/agent-eval/scorecard.mjs) over a fixture built from the real
// session_store.rs schema: six receipts, six manifests, two transcripts (one
// with a subagent file). Every expected number is worked out by hand in the
// comments next to it.
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import * as morningReport from './agent-eval/morning-report.mjs';
import {
  ALWAYS_LOAD,
  CARRIED_KINDS,
  DEF_PATTERN,
  ERROR_FAMILIES,
  PULSE_OK_SQL,
  REASK_MS,
  REFUSAL_LIKE,
  RESULT_PREFIX,
  STEP_ERROR_FIELDS,
  USAGE_STATS_SQL,
  commandShape,
  compareScorecards,
  dist,
  errorClass,
  expectedNonzero,
  isCheck,
  isMemoryPath,
  isPoll,
  isPollOnly,
  isPythonEdit,
  main,
  maskedBlock,
  maskedRunner,
  openDb,
  parseArgs,
  readCalls,
  readDb,
  refusalClass,
  renderComparison,
  renderSummary,
  commandHead,
  runIdTime,
  shellCommandOf,
  shellLookup,
  suggestionsFileFor,
  summaryFromVault,
  weight,
} from './agent-eval/scorecard.mjs';

const fixture = fileURLToPath(new URL('./agent-eval/fixtures/scorecard/', import.meta.url));
const vault = join(fixture, 'vault');
const transcripts = join(fixture, 'transcripts');
const rust = (file) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');
const since = Date.parse('2026-10-10T00:00:00Z');
const until = Date.parse('2026-10-11T00:00:00Z');

/** The store's own DDL: every literal batch and added column in
 *  session_store.rs's apply_schema, plus the step spine and friction tables,
 *  so the fixture breaks when a column the scorecard reads is renamed. */
function schema() {
  const unwrap = (s) => s.replace(/\\\n\s*/g, ' ');
  const store = rust('session_store.rs');
  const body = store.slice(store.indexOf('fn apply_schema'), store.indexOf('\nfn ', store.indexOf('fn apply_schema') + 1));
  const sql = [...body.matchAll(/execute_batch\(\s*"([\s\S]*?)",?\s*\)/g)].map((m) => unwrap(m[1]));
  for (const m of body.matchAll(/ensure_column\(\s*conn,\s*"(\w+)",\s*"\w+",\s*"([^"]+)",?\s*\)/g)) sql.push(`ALTER TABLE ${m[1]} ADD COLUMN ${m[2]}`);
  for (const m of body.matchAll(/for \(column, ddl\) in \[([\s\S]*?)\] \{\s*ensure_column\(conn, "(\w+)", column, ddl\)/g)) {
    for (const c of m[1].matchAll(/\("\w+", "([^"]+)"\)/g)) sql.push(`ALTER TABLE ${m[2]} ADD COLUMN ${c[1]}`);
  }
  for (const file of ['step_spine.rs', 'friction.rs']) sql.push(unwrap(rust(file).match(/pub\(crate\) const SCHEMA: &str = "([\s\S]*?)";/)[1]));
  return sql.join(';\n');
}

function buildDb(dir, extra) {
  const file = join(dir, 'sessions.db');
  const db = new DatabaseSync(file);
  db.exec(schema());
  const D = '2026-10-10T';
  const ledger = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, parent_surface, run_id, agent, native_session, billing, outcome, spawned, basis,
    input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status, workspace_id) values (?,?,?,?,?,?,?,?,'subscription',?,1,'per_run',?,?,?,?,?,?)`);
  // key, at, started, surface, parent, run, agent, native, outcome, input(incl. cache writes), cache read, output, cost, status, workspace
  ledger.run('k1', D + '10:01:00Z', D + '10:00:00Z', 'chat', null, 'run-A', 'claude', 'ns-1', 'success', 1000, 9000, 100, 1.0, 'estimated', 'ws-1');
  ledger.run('k2', D + '11:02:00Z', D + '11:00:00Z', 'chat', null, 'run-B', 'claude', 'ns-1', 'success', 2100, 418000, 200, 3.0, 'estimated', 'ws-1');
  // Cancelled at a budget kill: partial, with the guard's estimate as cost.
  ledger.run('k3', D + '12:00:30Z', D + '12:00:00Z', 'chat', null, 'run-C', 'claude', 'ns-2', 'cancelled', 500, 0, 10, 0.5, 'partial', 'ws-1');
  ledger.run('k4', D + '13:05:00Z', D + '13:00:00Z', 'chat', null, 'run-D', 'claude', 'ns-2', 'success', 100, 1000, 30, 2.0, 'estimated', 'ws-2');
  ledger.run('k5', D + '14:01:00Z', D + '14:00:00Z', 'chat', null, 'run-E', 'claude', null, 'error', null, null, null, null, 'unknown', 'ws-2');
  ledger.run('k6', D + '15:01:00Z', D + '15:00:00Z', 'chat', null, 'run-F', 'codex', null, 'success', 50, 50, 5, null, 'unpriced', 'ws-1');
  // A title oneshot reusing run-A's id: must not inherit its timings/tools.
  ledger.run('k7', D + '10:00:05Z', D + '10:00:02Z', 'oneshot', null, 'run-A', 'claude', null, 'success', 10, 0, 5, 0.1, 'estimated', null);
  // Outside the window.
  ledger.run('k8', '2026-10-09T10:01:00Z', '2026-10-09T10:00:00Z', 'chat', null, 'run-Z', 'claude', 'ns-1', 'success', 1, 1, 1, 9.0, 'estimated', 'ws-1');
  // Still running (no outcome yet), and interrupted with the reconciler's
  // late `at`, whose window would cover run-D's call m6 on ns-2.
  ledger.run('k9', D + '16:00:30Z', D + '16:00:00Z', 'chat', null, 'run-G', 'claude', 'ns-1', 'running', null, null, null, null, 'partial', 'ws-1');
  ledger.run('k10', D + '23:00:00Z', D + '12:30:00Z', 'chat', null, 'run-H', 'claude', 'ns-2', 'interrupted', null, null, null, null, 'partial', 'ws-1');
  // Two ask_agent consultations under run-A: one timed out.
  ledger.run('k11', D + '10:00:40Z', D + '10:00:20Z', 'delegate', 'chat', 'run-A', 'claude', null, 'timeout', null, null, null, null, 'partial', 'ws-1');
  ledger.run('k12', D + '10:00:50Z', D + '10:00:30Z', 'delegate', 'chat', 'run-A', 'claude', null, 'success', 10, 10, 10, 0.2, 'estimated', 'ws-1');
  db.prepare(`insert into run_queue (source, label, status, priority, requested_at, history_run_id, phase_timings_json) values ('chat','x','done',0,?,?,?)`)
    .run(D + '15:00:00Z', 'run-F', JSON.stringify({ queueWaitMs: 7, firstTextMs: 700, turnMs: 900 }));
  // s1 is the Main agent's session (its cwd is the vault's workspaces/main).
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('s1','claude','/v/workspaces/main',?,?), ('s2','codex','/v/workspaces/other',?,?)`).run(D, D, D, D);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, status, permission_mode) values (?,?,?,?,?,?)');
  const modes = ['bypassPermissions', 'acceptEdits', 'acceptEdits', 'bypassPermissions', 'bypassPermissions'];
  ['A', 'B', 'C', 'D', 'E'].forEach((r, i) => turn.run('s1', i + 1, `run-${r}`, `${D}1${i}:00:00Z`, 'success', modes[i]));
  turn.run('s2', 1, 'run-F', D + '15:00:00Z', 'success', 'bypassPermissions');
  turn.run('s1', 9, 'run-Z', '2026-10-09T10:00:00Z', 'success', 'bypassPermissions');
  // As the store writes them: the reason is on the stint that was closed,
  // with its ended_at. One started before the window and was reset inside
  // it; one was reset before the window.
  const stint = db.prepare('insert into stints (session_key, agent, started_at, ended_at, reason) values (?,?,?,?,?)');
  stint.run('s1', 'claude', '2026-10-08T10:00:00Z', '2026-10-09T09:00:00Z', 'config changed');
  stint.run('s1', 'claude', '2026-10-09T09:00:00Z', D + '11:00:00Z', 'config changed');
  stint.run('s1', 'claude', D + '11:00:00Z', D + '12:00:00Z', 'config changed');
  stint.run('s1', 'claude', D + '12:00:00Z', D + '13:00:00Z', 'moved');
  stint.run('s1', 'claude', D + '13:00:00Z', null, null);
  const step = db.prepare("insert into steps (session_key, turn_ord, seq, run_id, kind, name, started_at, status) values (?,?,?,?,?,?,'t',?)");
  let seq = 0;
  for (const [run, kind, name, status] of [['run-A', 'tool', 'Bash', 'ok'], ['run-A', 'tool', 'Bash', 'ok'], ['run-A', 'tool', 'Read', 'ok'], ['run-A', 'tool', 'Read', 'error'], ['run-A', 'model', null, 'error'], ['run-B', 'tool', 'Bash', 'ok'], ['run-B', 'tool', 'Bash', 'ok'], ['run-F', 'tool', 'Bash', 'error'], ['run-F', 'tool', 'Bash', 'error']]) {
    step.run('s1', 1, ++seq, run, kind, name, status);
  }
  const delegation = db.prepare('insert into delegations (run_id, at, record_json) values (?,?,?)');
  for (const [tool, when] of [['start_session', D + '10:00:10Z'], ['handover', D + '11:00:10Z'], ['ask_agent', D + '10:00:20Z'], ['handover', '2026-10-09T10:00:00Z']]) {
    delegation.run('run-A', when, JSON.stringify({ kind: tool === 'ask_agent' ? 'consultation' : 'childTask', tool, packetSha256: 'SECRET-MESSAGE-TEXT' }));
  }
  const friction = db.prepare(`insert into friction (signature, tool, error_class, message, count, chats, first_seen, last_seen, examples, sources, updated_at)
    values (?,?,?,'SECRET-MESSAGE-TEXT',?,1,?,?,'[]','[]',?)`);
  friction.run('f1', 'Bash', 'permission', 5, D, D + '12:00:00Z', D);
  friction.run('f2', 'Bash', 'error', 3, D, D + '12:00:00Z', D);
  friction.run('f3', 'Read', 'permission', 2, D, D + '12:00:00Z', D);
  friction.run('f4', 'Bash', 'error', 100, '2026-10-01', '2026-10-01T00:00:00Z', D);
  // Queue rows that waited for a lease (no history run: they join nothing).
  // 379 waits over 3.6 s (the 2026-10-13 Notion sync row), 1 over 2 s, 5
  // over 10 s never claimed (the last wait ends it), and one before the window.
  const waited = db.prepare(`insert into run_queue (source, label, status, priority, requested_at, claimed_at, waiting_since, waiting_holder, wait_count, last_waited_at)
    values ('chat','x','done',0,?,?,?,?,?,?)`);
  waited.run(D + '17:00:00Z', D + '17:00:03.600Z', D + '17:00:00.000Z', 'Notion sync', 379, D + '17:00:03.590Z');
  waited.run(D + '18:00:00Z', D + '18:00:02Z', D + '18:00:00Z', 'Notion sync', 1, D + '18:00:00Z');
  waited.run(D + '19:00:00Z', null, D + '19:00:00Z', 'chat turn', 5, D + '19:00:10Z');
  // An episode started (start_waiting_episodes) and claimed before any tick counted a wait.
  waited.run(D + '20:00:00Z', D + '20:00:01Z', D + '20:00:00Z', 'Notion sync', 0, D + '20:00:00Z');
  waited.run('2026-10-09T19:00:00Z', '2026-10-09T19:00:01Z', '2026-10-09T19:00:00Z', 'Notion sync', 50, '2026-10-09T19:00:01Z');
  extra?.(db);
  db.close();
  return file;
}

/** A ledger row with scope and model (a Jev request has agent NULL). */
function ledgerRow(db, { key, at: end, started, surface = 'chat', parent = null, run = null, agent = 'claude', scope = null, model = null, outcome = 'success', cost = null, status = 'unknown' }) {
  db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, scope, parent_surface, run_id, agent, model, billing, outcome, spawned, basis, cost_usd, cost_status)
    values (?,?,?,?,?,?,?,?,?,'subscription',?,1,'per_run',?,?)`).run(key, end, started, surface, scope, parent, run, agent, model, outcome, cost, status);
}

async function withScorecard(fn, roots = [transcripts], { extra, overlap = false, from = since, to = until, vault: vaultDir = vault, suggestions, codexHome = null } = {}) {
  const dir = mkdtempSync(join(tmpdir(), 'scorecard-'));
  try {
    const dbFile = buildDb(dir, extra);
    return await fn(await summaryFromVault({ vault: vaultDir, dbFile, transcriptRoots: roots, codexHome, since: from, until: to, overlap, suggestionsFile: suggestions }), dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('nearest-rank distributions and run-id times', () => {
  assert.deepEqual(dist([5, 1, 4, 2, 3]), { n: 5, p50: 3, p95: 5 });
  assert.deepEqual(dist([null, NaN]), { n: 0, p50: null, p95: null });
  assert.equal(runIdTime('run-1792185071536746000-22175-447'), 1792185071536);
  assert.equal(runIdTime('run-A'), null);
});

test('runs, outcomes, latency and cost per surface and agent', async () => {
  await withScorecard((s) => {
    assert.equal(s.counts.ledgerRows, 11); // k1–k7, k9–k12; k8 is before the window
    const c = s.groups['chat/claude'];
    assert.equal(c.runs, 7);
    assert.deepEqual(c.outcomes, { success: 3, cancelled: 1, error: 1, running: 1, interrupted: 1 });
    // Over the 6 finished runs (the running one has no outcome yet).
    assert.equal(c.rates.success, 0.5);
    assert.equal(c.rates.interrupted, 0.1667);
    assert.equal(c.rates.other, 0);
    // firstText 1000…5000: nearest-rank p50 = 3rd = 3000, p95 = 5th = 5000.
    assert.deepEqual(c.latency.firstTextMs, { n: 5, p50: 3000, p95: 5000 });
    assert.deepEqual(c.latency.spawnMs, { n: 5, p50: 1500, p95: 2500 });
    assert.deepEqual(c.latency.preparationMs, { n: 5, p50: 300, p95: 500 });
    assert.deepEqual(c.latency.retrievalMs, { n: 2, p50: 50, p95: 70 });
    // Known 1, 3, 2: floor 6. The partial run's $0.50 is not known cost:
    // with four runs lacking a known cost there is no total.
    assert.equal(c.cost.totalUsd, null);
    assert.equal(c.cost.floorUsd, 6);
    assert.equal(c.cost.knownRuns, 3);
    assert.deepEqual(c.cost.unknownByStatus, { partial: 3, unknown: 1 });
    assert.equal(c.cost.partialCostUsd, 0.5);
    assert.equal(c.cost.tokensWithoutCost, 510); // k3's 500 + 0 + 10
    assert.equal(c.cost.p50Usd, 2);
    assert.equal(c.cost.p95Usd, 3);
    // Tools: run-A 4 calls/1 error, run-B 2/0 → 1 error over the 5 top-level
    // turns, 1 per 6 calls (a Read error: a failure). ns-1's calls carry no
    // check command: no clean check result to mask a failure.
    assert.deepEqual(c.tools, {
      turns: 5,
      turnsWithTools: 2,
      calls: 6,
      errors: 1,
      errorsPerTurn: 0.2,
      errorsPer100Calls: 16.67,
      expectedNonzero: 0,
      failures: 1,
      failuresPer100Calls: 16.67,
      checkFailed: 0,
      maskedFailures: { cleanCheckResults: 0, masked: 0, byRunner: {} },
    });
    // Retrieval: notes read 80 and 100 of 100 and 200 eligible.
    assert.deepEqual(c.retrieval, { runs: 2, notesRead: { n: 2, p50: 80, p95: 100 }, bytesRead: { n: 2, p50: 1000, p95: 3000 }, prefilterPassRate: { n: 2, p50: 0.5, p95: 0.8 } });
    // run-F's receipt has no timings: the queue row's are used.
    const x = s.groups['chat/codex'];
    assert.deepEqual(x.latency.firstTextMs, { n: 1, p50: 700, p95: 700 });
    assert.equal(x.cost.totalUsd, null);
    assert.deepEqual(x.cost.unknownByStatus, { unpriced: 1 });
    assert.equal(x.tools.errorsPerTurn, 2);
    assert.equal(x.tools.errorsPer100Calls, 100); // 2 errors in 2 calls
    assert.equal(x.tools.maskedFailures, null); // no transcript
    // The oneshot sharing run-A's id gets none of run-A's timings or tools,
    // and its cost is a total because every run in it has one.
    const o = s.groups['oneshot/claude'];
    assert.equal(o.latency.firstTextMs.n, 0);
    assert.equal(o.tools, null);
    assert.equal(o.cost.totalUsd, 0.1);
  });
});

// A run that started 5 minutes before --since and ended inside the window,
// and a 1-hour top-level run inside it (with queue timings).
const straddleAndLong = (db) => {
  ledgerRow(db, { key: 'k20', started: '2026-10-09T23:55:00Z', at: '2026-10-10T00:05:00Z', run: 'run-S', cost: 4, status: 'estimated' });
  ledgerRow(db, { key: 'k21', started: '2026-10-10T17:00:00Z', at: '2026-10-10T18:00:00Z', run: 'run-L', cost: 10, status: 'estimated' });
  db.prepare(`insert into run_queue (source, label, status, priority, requested_at, history_run_id, phase_timings_json) values ('chat','x','done',0,?,?,?)`)
    .run('2026-10-10T17:00:00Z', 'run-L', JSON.stringify({ queueWaitMs: 1, firstTextMs: 9000, turnMs: 3_600_000 }));
};

test('a run straddling --since shows under straddling, and under groups with --overlap', async () => {
  await withScorecard((s) => {
    assert.equal(s.window.overlap, false);
    assert.equal(s.straddling.runs, 1);
    assert.equal(s.straddling.cost.totalUsd, 4);
    assert.deepEqual(s.straddling.groups, { 'chat/claude': 1 });
    // Windowed by start: k20 is not in the groups (7 + run-L).
    assert.equal(s.groups['chat/claude'].runs, 8);
    assert.equal(s.counts.ledgerRows, 12);
    assert.match(renderSummary(s), /Straddling the window's start: 1 run, \$4\.00/);
  }, [transcripts], { extra: straddleAndLong });
  await withScorecard((s) => {
    assert.equal(s.window.overlap, true);
    assert.equal(s.groups['chat/claude'].runs, 9);
    assert.equal(s.straddling.runs, 1);
    assert.equal(s.counts.ledgerRows, 13); // k8 (10-09 10:00–10:01) still out
  }, [transcripts], { extra: straddleAndLong, overlap: true });
});

test('top-level runs over 30 minutes are a longRuns group, kept out of latency percentiles', async () => {
  await withScorecard((s) => {
    // run-L only: the interrupted k10's `at` is the reconciler's, not an end.
    assert.equal(s.longRuns.runs, 1);
    assert.equal(s.longRuns.thresholdMs, 1_800_000);
    assert.equal(s.longRuns.cost.totalUsd, 10);
    assert.deepEqual(s.longRuns.groups, { 'chat/claude': 1 });
    assert.deepEqual(s.longRuns.minutes, { n: 1, p50: 60, p95: 60 });
    const c = s.groups['chat/claude'];
    assert.equal(c.longRuns, 1);
    // run-L's 9 s first text and 1-hour turn stay out: A–E only.
    assert.deepEqual(c.latency.firstTextMs, { n: 5, p50: 3000, p95: 5000 });
    assert.equal(c.latency.turnMs.n, 5);
    assert.match(renderSummary(s), /Long runs \(over 30 min, out of latency\): 1 run, \$10\.00/);
  }, [transcripts], { extra: straddleAndLong });
});

// Three Jev requests as jev.rs meters them: oneshot, scope and model `jev`, no agent.
const jevRows = (db) => {
  ledgerRow(db, { key: 'j1', started: '2026-10-10T09:00:00.000Z', at: '2026-10-10T09:00:00.200Z', surface: 'oneshot', agent: null, scope: 'jev', model: 'jev' });
  ledgerRow(db, { key: 'j2', started: '2026-10-10T09:01:00.000Z', at: '2026-10-10T09:01:00.400Z', surface: 'oneshot', agent: null, scope: 'jev', model: 'jev' });
  ledgerRow(db, { key: 'j3', started: '2026-10-10T09:02:00.000Z', at: '2026-10-10T09:02:01.000Z', surface: 'oneshot', agent: null, scope: 'jev', model: 'jev', outcome: 'error' });
};

test('Jev requests land in oneshot/jev, unpriced by design, with unknownRuns 0', async () => {
  await withScorecard((s) => {
    assert.equal(s.schema, 2);
    assert.equal(s.groups['oneshot/unknown'], undefined);
    const j = s.groups['oneshot/jev'];
    assert.equal(j.runs, 3);
    assert.deepEqual(j.outcomes, { success: 2, error: 1 });
    assert.equal(j.cost.unknownRuns, 0);
    assert.equal(j.cost.unpricedByDesign, 3);
    // Unpriced is not $0: no total.
    assert.equal(j.cost.totalUsd, null);
    // Ledger start → end: 200, 400, 1000 ms.
    assert.deepEqual(j.wallMs, { n: 3, p50: 400, p95: 1000 });
    const md = renderSummary(s);
    assert.match(md, /\| oneshot\/jev \| 3 \|.*unpriced by design \(3\)/);
    assert.match(md, /Jev requests: 3, latency p50\/p95 0\.4 s \/ 1\.0 s; success 2, error 1; unpriced by design/);
  }, [transcripts], { extra: jevRows });
});

test('every rate renders with its n', async () => {
  await withScorecard((s) => {
    // chat/claude: 3 success, 1 cancelled, 1 error, 0 timeout of 6 finished.
    assert.match(renderSummary(s), /\| chat\/claude \| 7 \| 50\.0% \(3\/6\) \| 16\.7% \(1\/6\) \| 16\.7% \(1\/6\) \| 0\.0% \(0\/6\) \|/);
    // ask_agent: 1 of 2 timed out.
    assert.match(renderSummary(s), /ask_agent 2 runs, 1 timed out \(50\.0% of 2\)/);
  });
});

test('resets per 100 turns, by when they happened, with mode flips', async () => {
  await withScorecard((s) => {
    // 6 turns in the window (5 claude, 1 codex). Stints started in it: 3.
    // Resets ended in it: 11:00 and 12:00 config changed, 13:00 moved (the
    // first stint started on 10-09); the 10-09 reset is outside.
    // 11:00 starts run-B in acceptEdits after run-A in bypass: a flip.
    // 12:00 run-C follows run-B in the same mode. 13:00 is a move: run-D's
    // mode differs from run-C's, but a move is never a flip.
    assert.deepEqual(s.continuity.all, {
      turns: 6,
      stints: 3,
      resets: 3,
      causes: { 'config changed': 2, moved: 1 },
      flipResets: 1,
      flips: { 'bypassPermissions->acceptEdits': 1 },
      resetsPer100Turns: 50,
      configChangedPer100Turns: 33.33,
      flipResetsPer100Turns: 16.67,
      // No stint holds OB03's parts: each config-changed reset is unknown.
      resetsByPart: { unknown: 2 },
    });
    assert.equal(s.continuity.claude.resetsPer100Turns, 60);
    assert.equal(s.continuity.codex.resets, 0);
    // s1 runs in workspaces/main.
    assert.equal(s.continuity.mainAgent.turns, 5);
    assert.equal(s.continuity.mainAgent.configChangedPer100Turns, 40);
  });
});

test('a config-changed reset counts under the parts that differ from the next stint (OF03)', async () => {
  // s1's stints as OB03 stamps them: the 11:00 reset changed model and mode
  // against the stint after it; the 12:00 reset's next stint holds the same
  // parts (nothing it can name); the 13:00 move is not a config reset.
  const parts = (model, mode) => JSON.stringify({ contract: 'c1', model, mode, cwd: 'w1' });
  const stamp = (rows) => (db) => {
    const set = db.prepare("update stints set config_parts = ? where session_key = 's1' and started_at = ?");
    for (const [text, started] of rows) set.run(text, started);
  };
  const stamped = stamp([[parts('m1', 'act'), '2026-10-09T09:00:00Z'], [parts('m2', 'plan'), '2026-10-10T11:00:00Z'], [parts('m2', 'plan'), '2026-10-10T12:00:00Z'], ['not json', '2026-10-10T13:00:00Z']]);
  await withScorecard((s) => {
    assert.deepEqual(s.continuity.all.resetsByPart, { mode: 1, model: 1, unknown: 1 });
    assert.deepEqual(s.continuity.mainAgent.resetsByPart, { mode: 1, model: 1, unknown: 1 });
    assert.deepEqual(s.continuity.codex.resetsByPart, {});
    assert.match(renderSummary(s), /Config-changed resets by what changed: mode 1, model 1, unknown 1\./);
    const c = compareScorecards(s, s);
    assert.deepEqual(c.continuity.resetsByPart.b, { mode: 1, model: 1, unknown: 1 });
    assert.match(renderComparison(c), /Config-changed resets by what changed: mode 1, model 1, unknown 1 → mode 1, model 1, unknown 1\./);
  }, [transcripts], { extra: stamped });
  // The 12:00 reset's next stint holds unparseable parts: unknown.
  const oneSided = stamp([[parts('m1', 'act'), '2026-10-09T09:00:00Z'], [parts('m2', 'plan'), '2026-10-10T11:00:00Z'], ['not json', '2026-10-10T12:00:00Z']]);
  await withScorecard((s) => {
    assert.deepEqual(s.continuity.all.resetsByPart, { mode: 1, model: 1, unknown: 1 });
  }, [transcripts], { extra: oneSided });
  // Parts on one side only: the 11:00 reset's next stint never stamped (and
  // so the 12:00 reset's closed stint has none either).
  const unstamped = stamp([[parts('m1', 'act'), '2026-10-09T09:00:00Z']]);
  await withScorecard((s) => {
    assert.deepEqual(s.continuity.all.resetsByPart, { unknown: 2 });
  }, [transcripts], { extra: unstamped });
});

test('a turn counts under the agent of the stint it ran in, not the session\'s current agent', async () => {
  // s3 ran on Claude until 18:00, then switched to Codex (its current agent).
  const switched = (db) => {
    db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('s3','codex','/v/workspaces/third',?,?)`).run('2026-10-10T17:00:00Z', '2026-10-10T18:30:00Z');
    const stint = db.prepare('insert into stints (session_key, agent, started_at, ended_at, reason) values (?,?,?,?,?)');
    stint.run('s3', 'claude', '2026-10-10T17:00:00Z', '2026-10-10T18:00:00Z', 'agent changed');
    stint.run('s3', 'codex', '2026-10-10T18:00:00Z', null, null);
    const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, status, permission_mode) values (?,?,?,?,?,?)');
    turn.run('s3', 1, 'run-S1', '2026-10-10T17:30:00Z', 'success', 'bypassPermissions');
    turn.run('s3', 2, 'run-S2', '2026-10-10T18:30:00Z', 'success', 'bypassPermissions');
  };
  await withScorecard((s) => {
    assert.equal(s.continuity.all.turns, 8);
    assert.equal(s.continuity.claude.turns, 6); // s1's 5 and s3's first
    assert.equal(s.continuity.codex.turns, 2); // s2's (no stint: the session's agent) and s3's second
  }, [transcripts], { extra: switched });
});

test('queue waits per second by waiting holder', async () => {
  await withScorecard((s) => {
    // 379 / 3.6 s = 105.3; 5 / 10 s (no claim: to the last wait) = 0.5.
    // The 1-wait row and the 0-wait episode count as rows that waited but
    // have no rate (it takes two waits to time one).
    assert.deepEqual(s.queue, {
      waits: 385,
      rows: 4,
      maxWaitsPerSecond: 105.3,
      maxRateWaits: 379,
      byHolder: {
        'Notion sync': { waits: 380, rows: 3, maxWaitsPerSecond: 105.3, maxRateWaits: 379 },
        'chat turn': { waits: 5, rows: 1, maxWaitsPerSecond: 0.5, maxRateWaits: 5 },
      },
    });
    assert.match(renderSummary(s), /Queue waits: 385 over 4 rows, max 105\.3\/s \(379 waits\)/);
  });
});

test('a single wait has no rate: the first wait opens the episode', async () => {
  // One wait, claimed 10 ms after it: waiting_since is that wait's own time,
  // so 1 / 0.01 s would read as 100 waits a second.
  const fastSingle = (db) => db.prepare(`insert into run_queue (source, label, status, priority, requested_at, claimed_at, waiting_since, waiting_holder, wait_count, last_waited_at)
    values ('chat','x','done',0,?,?,?,?,?,?)`).run('2026-10-10T21:00:00Z', '2026-10-10T21:00:00.010Z', '2026-10-10T21:00:00.000Z', 'Plaud sync', 1, '2026-10-10T21:00:00.000Z');
  await withScorecard((s) => {
    assert.deepEqual(s.queue.byHolder['Plaud sync'], { waits: 1, rows: 1, maxWaitsPerSecond: null, maxRateWaits: null });
    assert.equal(s.queue.waits, 386);
    assert.equal(s.queue.maxWaitsPerSecond, 105.3);
  }, [transcripts], { extra: fastSingle });
});

// A lane turn still running since before --since (its `at` is its last
// checkpoint, written before the window), and an interrupted run that began
// before --since whose `at` is the reconciler's sweep inside the window.
const runningAndInterrupted = (db) => {
  ledgerRow(db, { key: 'k30', started: '2026-10-09T22:00:00Z', at: '2026-10-09T22:10:00Z', run: 'run-R', outcome: 'running', status: 'partial' });
  ledgerRow(db, { key: 'k31', started: '2026-10-09T23:00:00Z', at: '2026-10-10T03:00:00Z', run: 'run-I', outcome: 'interrupted', status: 'partial' });
};

test('a still-running row has not ended; an interrupted one never straddles', async () => {
  await withScorecard((s) => {
    // k30 began before the window and has not ended: it straddles. k31's
    // `at` is the reconciler's, not an end inside the window.
    assert.equal(s.straddling.runs, 1);
    assert.deepEqual(s.straddling.groups, { 'chat/claude': 1 });
    assert.equal(s.groups['chat/claude'].runs, 7);
    assert.equal(s.counts.ledgerRows, 11);
    // k9 in the groups and k30 straddling.
    assert.equal(s.counts.stillRunning, 2);
    assert.match(renderSummary(s), /; still running 2\./);
  }, [transcripts], { extra: runningAndInterrupted });
  await withScorecard((s) => {
    // With --overlap k30 counts (it runs on past the window's start); k31
    // does not: it began before the window and its end is unknown.
    const c = s.groups['chat/claude'];
    assert.equal(c.runs, 8);
    assert.equal(c.outcomes.running, 2);
    assert.equal(c.outcomes.interrupted, 1); // k10, which began inside
    assert.equal(s.straddling.runs, 1);
    assert.equal(s.counts.stillRunning, 2);
  }, [transcripts], { extra: runningAndInterrupted, overlap: true });
});

test('an agent-NULL oneshot that is not Jev stays oneshot/unknown, counted as unknown cost', async () => {
  const titleRow = (db) => {
    jevRows(db);
    ledgerRow(db, { key: 't1', started: '2026-10-10T09:05:00Z', at: '2026-10-10T09:05:01Z', surface: 'oneshot', agent: null, scope: 'title' });
  };
  await withScorecard((s) => {
    const u = s.groups['oneshot/unknown'];
    assert.equal(u.runs, 1);
    assert.equal(u.cost.unknownRuns, 1);
    assert.equal(u.cost.unpricedByDesign, 0);
    assert.equal(s.groups['oneshot/jev'].runs, 3);
  }, [transcripts], { extra: titleRow });
});

test('failures, delegation and history per workspace', async () => {
  await withScorecard((s) => {
    assert.deepEqual(s.failures.byTool, { Bash: 2, Read: 1 });
    // The friction families seen in the window, by count; the 10-01 row is out.
    assert.deepEqual(s.failures.frictionTop, [
      { tool: 'Bash', errorClass: 'permission', count30d: 5 },
      { tool: 'Bash', errorClass: 'error', count30d: 3 },
      { tool: 'Read', errorClass: 'permission', count30d: 2 },
    ]);
    // run-A's handover record names no successor: no first turn to judge.
    assert.deepEqual(s.delegation, {
      askAgent: { runs: 2, timeouts: 1, timeoutRate: 0.5 },
      records: { start_session: 1, handover: 1, ask_agent: 1 },
      handovers: 1,
      handover: { records: 1, childFirstTurnOk: { ok: 0, n: 0, share: null }, noEndedFirstTurn: 1 },
    });
    // Delegate rows under run-A take none of its timings.
    assert.equal(s.groups['delegate/claude'].latency.firstTextMs.n, 0);
    // ws-1: run-A 10, run-B 20, run-C 30 ms; ws-2: run-D 40, run-E 50 ms.
    assert.deepEqual(s.history['ws-1'].baselineSnapshotMs, { n: 3, p50: 20, p95: 30 });
    assert.deepEqual(s.history['ws-2'].finalizeDiffMs, { n: 2, p50: 80, p95: 100 });
  });
});

test('injected bytes per layer and by session state', async () => {
  await withScorecard((s) => {
    // Totals 1500, 200, 300, 1600, 400, 100 → sorted p50 = 3rd = 300, p95 = 6th = 1600.
    assert.deepEqual(s.injected.total, { n: 6, p50: 300, p95: 1600 });
    assert.deepEqual(s.injected.layers.profile, { n: 2, p50: 500, p95: 600 });
    assert.deepEqual(s.injected.bySession.resume.total, { n: 3, p50: 300, p95: 400 });
    assert.equal(s.injected.bySession.reset.runs, 1);
    assert.equal(s.injected.bySession.none.runs, 1);
    assert.deepEqual(Object.fromEntries(Object.entries(s.injected.byEntry).map(([k, v]) => [k, v.runs])), { appSession: 4, composer: 1, none: 1 });
    // run-A's workspace brief was cut (50 of 100 bytes granted), run-B's not.
    assert.deepEqual(s.injected.workspaceCut, { runs: 2, cut: 1, rate: 0.5 });
  });
});

test('transcripts: calls, context, cache, large-context share and reconciliation', async () => {
  await withScorecard((s) => {
    const t = s.groups['chat/claude'].transcripts;
    assert.equal(t.joinedRuns, 4); // run-E has no native session; run-G is running; run-H interrupted
    assert.equal(t.calls, 6); // m1 (two lines, one call), m2, m3, m4 (subagent), m5, m6; m0 is outside every run
    assert.equal(t.subagentCalls, 1);
    assert.deepEqual(t.callsPerTurn, { n: 4, p50: 1, p95: 2 });
    assert.deepEqual(t.mainCallsPerTurn, { n: 4, p50: 1, p95: 2 }); // 2, 1, 1, 1
    // Main-thread context 1000, 9000, 420000, 500, 1100 → p50 1100, p95 420000.
    assert.deepEqual(t.contextPerCall, { n: 5, p50: 1100, p95: 420000 });
    assert.equal(t.cacheWriteTokens, 3090); // 990 + 2000 + 100
    assert.equal(t.cacheReadTokens, 428000);
    assert.equal(t.cacheReadShare, 0.9914); // 428000 / (610 + 3090 + 428000)
    assert.equal(t.largeContextCalls, 1);
    // Weights 1547.5, 1100, 45050, 350, 550, 325 → 45050 / 48922.5.
    assert.equal(t.costShareAtLarge, 0.9208);
    // Ledger 10100 + 420300 + 510 + 1130 = 432040; transcripts 432030.
    assert.deepEqual(t.reconciliation, { ledgerTokens: 432040, transcriptTokens: 432030, ratio: 1 });
    assert.ok(Math.abs(1 - t.reconciliation.transcriptTokens / t.reconciliation.ledgerTokens) < 0.05);
  });
});

test('no message text reaches the report', async () => {
  await withScorecard((s) => {
    const out = JSON.stringify(s) + renderSummary(s);
    assert.ok(readFileSync(join(transcripts, 'proj', 'ns-1.jsonl'), 'utf8').includes('SECRET-MESSAGE-TEXT'));
    assert.ok(!out.includes('SECRET-MESSAGE-TEXT'));
    assert.ok(!out.includes('/proj/'));
    assert.ok(!out.includes('should-not-leak'));
  });
});

// Overnight 9 A14: a Codex run joined to its rollout by thread id. Calls
// 100,000 (80,000 cached) and 30,000; the 09:00:04.5 repeat, the `info:
// null` line and the 09:30 call (after the turn) don't count. One
// compaction, one `exec` call; no message text, argument or path leaks.
const codexHome = join(fixture, 'codex');
const CODEX_THREAD_ID = '0199aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee';
const codexRun = (db) => {
  db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_status, input_tokens, cached_input_tokens, output_tokens)
    values ('cx-1','2026-10-10T09:01:00Z','2026-10-10T09:00:00Z','chat','run-CX1','codex',?,'unknown','success',1,'per_run','unpriced',50000,80000,700)`).run(CODEX_THREAD_ID);
  db.prepare("insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values ('cx',1,'run-CX1','2026-10-10T09:00:00Z','2026-10-10T09:01:00Z','success','bypassPermissions')").run();
};

test('A14: a Codex run reads its rollout: context per call, compactions and tools, by name only', async () => {
  let claudeTurns;
  await withScorecard((s) => {
    const g = s.groups['chat/codex'];
    assert.equal(g.transcripts.joinedRuns, 1);
    assert.equal(g.transcripts.calls, 2);
    assert.deepEqual(g.transcripts.contextPerCall, { n: 2, p50: 30000, p95: 100000 });
    assert.equal(g.transcripts.cacheReadTokens, 80000);
    // Ledger 50,000 + 80,000 + 700; the rollout's two calls the same.
    assert.deepEqual(g.transcripts.reconciliation, { ledgerTokens: 130700, transcriptTokens: 130700, ratio: 1 });
    assert.deepEqual(g.codex, { joinedRuns: 1, compactions: 1, compactedRuns: 1, toolCalls: { exec: 1 } });
    assert.equal(s.codex.turns, 1);
    assert.equal(s.codex.compactions, 1);
    assert.equal(s.codex.firstCall.bySession.new.n, 1);
    // Codex writes no cache: a cache-write figure is null, never a measured 0.
    assert.equal(s.codex.firstCall.bySession.new.cacheWriteP50, null);
    assert.equal(s.codex.firstCall.bySession.new.cacheReadP50, 80000);
    assert.equal(s.codex.coldResume.n, 0);
    claudeTurns = s.transcripts?.turns ?? null;
    const out = JSON.stringify(s) + renderSummary(s);
    assert.ok(readFileSync(join(codexHome, 'sessions/2026/10/10', `rollout-2026-10-10T09-00-00-${CODEX_THREAD_ID}.jsonl`), 'utf8').includes('SECRET-MESSAGE-TEXT'));
    for (const leak of ['SECRET-MESSAGE-TEXT', 'should-not-leak', 'gpt-6-astra', 'cat ']) assert.ok(!out.includes(leak), leak);
  }, [transcripts], { extra: codexRun, codexHome });
  // Without a Codex home the run stays unjoined, as before, and the Claude
  // tool pass counts the same turns either way.
  await withScorecard((s) => {
    assert.equal(s.groups['chat/codex'].transcripts, null);
    assert.equal(s.codex, null);
    assert.equal(s.transcripts?.turns ?? null, claudeTurns);
  }, [transcripts], { extra: codexRun });
});

test('A14: a Bash head skips a leading set, with a newline, ; or &&', () => {
  for (const [c, head] of [['set -e; rg foo', 'rg'], ['set -euo pipefail\nrg -n x src', 'rg'], ["/bin/zsh -lc 'set -euo pipefail\ngit status'", 'git status'], ['set -e && cargo test', 'cargo test'], ['set -e ; rg x', 'rg'], ['set -x', 'set']]) {
    assert.equal(commandHead(c), head, c);
  }
});

test('without transcripts the block is null, not zero', async () => {
  await withScorecard((s) => {
    assert.equal(s.groups['chat/claude'].transcripts, null);
    assert.equal(s.counts.transcriptRunsJoined, 0);
    assert.equal(s.transcripts, null);
  }, []);
});

test('compare flags a p95 rise over 10% and keeps unpaired groups', async () => {
  await withScorecard((a) => {
    const b = structuredClone(a);
    b.groups['chat/claude'].latency.firstTextMs.p95 = 5600; // +12%
    b.groups['chat/claude'].latency.turnMs.p95 = a.groups['chat/claude'].latency.turnMs.p95 * 1.05; // +5%: no flag
    delete b.groups['oneshot/claude'];
    const c = compareScorecards(a, b);
    assert.equal(c.groups['chat/claude'].latency.firstTextMs.p95Change, 0.12);
    assert.deepEqual(c.flags.map((f) => `${f.kind} ${f.group} ${f.metric}`), ['p95-rise chat/claude firstTextMs']);
    // 5 first-text samples (and 6 finished runs): a low-n flag.
    assert.equal(c.flags[0].lowN, true);
    assert.match(renderComparison(c), /firstTextMs: 0\.12 \(low n\)/);
    assert.deepEqual(c.unpaired, ['A:oneshot/claude']);
    assert.equal(c.groups['chat/claude'].success.diff, 0);
    assert.deepEqual(c.continuity.flipResetsPer100Turns, { a: 16.67, b: 16.67 });
    assert.deepEqual(c.continuity.mainAgentConfigChangedPer100Turns, { a: 40, b: 40 });
    assert.match(renderComparison(c), /p95-rise chat\/claude firstTextMs: 0\.12/);
    assert.throws(() => compareScorecards(a, { kind: 'other' }), /two scorecard JSON files/);
  });
});

test('a schema-1 against schema-2 compare pairs the Jev groups and marks low n', async () => {
  await withScorecard((b) => {
    // A schema-1 scorecard: Jev rows were `oneshot/unknown`, counted unknown;
    // no finished count, straddling, longRuns or queue block.
    const a = structuredClone(b);
    a.schema = 1;
    const { unpricedByDesign, ...cost } = a.groups['oneshot/jev'].cost;
    a.groups['oneshot/unknown'] = { ...a.groups['oneshot/jev'], cost: { ...cost, unknownRuns: unpricedByDesign, unknownByStatus: { unknown: unpricedByDesign } } };
    delete a.groups['oneshot/jev'];
    for (const g of Object.values(a.groups)) delete g.finished;
    delete a.straddling;
    delete a.longRuns;
    delete a.queue;
    const c = compareScorecards(a, b);
    assert.deepEqual(c.unpaired, []);
    const j = c.groups['oneshot/jev'];
    assert.deepEqual(j.runs, { a: 3, b: 3 });
    assert.deepEqual(j.cost.unpricedByDesign, { a: 3, b: 3 });
    assert.deepEqual(j.cost.unknownRuns, { a: 0, b: 0 });
    // n is the finished runs (schema 1: all runs); under 20 is low n.
    assert.deepEqual(j.n, { a: 3, b: 3 });
    assert.equal(j.lowN, true);
    assert.deepEqual(c.groups['chat/claude'].n, { a: 7, b: 6 });
    assert.deepEqual(c.queue.maxWaitsPerSecond, { a: null, b: 105.3 });
    assert.match(renderComparison(c), /\| oneshot\/jev \(low n\) \| 3→3 \(n 3→3\) \|/);
  }, [transcripts], { extra: jevRows });
});

// ── OF28: OB05's call figures, OB07's partial-run floor, OB08a's first model event
//
// A day of its own (10-14). Chat run-P1 (claude, $2 estimated): usage_json
// with peakContextTokens 400,000, apiCalls 40 and a subagents object, beside
// a note and raw block the report must never see, and a floor of $0.90 that
// its known cost must keep out of the floor sum; its receipt has
// firstModelMs 2000 and toolsBeforeText 3. Chat run-P2 (claude): cancelled
// at a kill, cost_usd NULL and partial, with OB07's floor $0.40 and a
// usage_json that isn't JSON. Chat run-P3 (codex): usage_json with none of
// the keys, as Codex rows are.
const D14 = '2026-10-14T';
const of28 = { from: Date.parse(D14 + '00:00:00Z'), to: Date.parse('2026-10-15T00:00:00Z') };
const OF28_SECRET = (tag) => `SECRET-OF28-${tag}`;
const hasColumn = (db, table, c) => db.prepare(`pragma table_info(${table})`).all().some((x) => x.name === c);
const of28Rows = (db) => {
  // OB07's column, until session_store.rs's schema has it.
  if (!hasColumn(db, 'usage_ledger', 'cost_floor_usd')) db.exec('alter table usage_ledger add column cost_floor_usd REAL');
  const row = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, billing, outcome, spawned, basis, cost_usd, cost_status, cost_floor_usd, usage_json)
    values (?,?,?,'chat',?,?,'subscription',?,1,'per_run',?,?,?,?)`);
  row.run('k-P1', D14 + '06:10:00Z', D14 + '06:00:00Z', 'run-P1', 'claude', 'success', 2, 'estimated', 0.9,
    JSON.stringify({ basis: 'per_run', apiCalls: 40, peakContextTokens: 400_000, cacheWriteTokens: 9000, subagents: { calls: 3, tokens: 12_000 }, note: OF28_SECRET('NOTE'), raw: { text: OF28_SECRET('RAW') } }));
  row.run('k-P2', D14 + '07:01:00Z', D14 + '07:00:00Z', 'run-P2', 'claude', 'cancelled', null, 'partial', 0.4, `{"apiCalls": 7, ${OF28_SECRET('BAD')}`);
  row.run('k-P3', D14 + '08:01:00Z', D14 + '08:00:00Z', 'run-P3', 'codex', 'success', null, 'unpriced', null, JSON.stringify({ basis: 'per_run', note: OF28_SECRET('NOTE') }));
};
// of28Rows and two rows the window-wide ledger block leaves out (finished
// top-level runs only), each with a larger peak than run-P1's: an ask_agent
// consultation under run-P1 and chat run-P5's running checkpoint.
const of28AndOthers = (db) => {
  of28Rows(db);
  const row = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, parent_surface, run_id, agent, billing, outcome, spawned, basis, cost_status, usage_json)
    values (?,?,?,?,?,?,'claude','subscription',?,1,'per_run','partial',?)`);
  row.run('k-P4', D14 + '06:05:00Z', D14 + '06:04:00Z', 'delegate', 'chat', 'run-P1', 'success', JSON.stringify({ apiCalls: 90, peakContextTokens: 900_000 }));
  row.run('k-P5', D14 + '09:01:00Z', D14 + '09:00:00Z', 'chat', null, 'run-P5', 'running', JSON.stringify({ apiCalls: 80, peakContextTokens: 800_000 }));
};
function of28Vault(dir) {
  const v = join(dir, 'vault');
  mkdirSync(v);
  writeFileSync(join(v, '.evidence-receipts.json'), JSON.stringify({ schemaVersion: 1, receipts: [
    { runId: 'run-P1', source: 'chat', startedAt: D14 + '06:00:00Z', status: 'success', phaseTimings: { spawnMs: 500, firstModelMs: 2000, toolsBeforeText: 3, firstTextMs: 2500, turnMs: 600_000 } },
  ] }));
  return v;
}
// A store before OB05 and OB07: neither column.
const preOf28Rows = (db) => {
  for (const c of ['usage_json', 'cost_floor_usd']) if (hasColumn(db, 'usage_ledger', c)) db.exec(`alter table usage_ledger drop column ${c}`);
};

test('OF28: peak context, API calls and subagents from usage_json, the partial-run floor apart from known cost, and the first model event', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-of28-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  // readDb names usage_json only inside USAGE_STATS_SQL, and keeps no text.
  const { db } = await openDb(buildDb(dir, of28Rows));
  const prepare = DatabaseSync.prototype.prepare;
  const seen = [];
  let rows;
  DatabaseSync.prototype.prepare = function (sql) {
    seen.push(sql);
    return prepare.call(this, sql);
  };
  try {
    rows = readDb(db, { since: of28.from, until: of28.to });
  } finally {
    DatabaseSync.prototype.prepare = prepare;
    db.close();
  }
  const named = seen.filter((sql) => /usage_json/i.test(sql));
  assert.equal(named.length, 1);
  assert.ok(!/usage_json/i.test(named[0].split(USAGE_STATS_SQL).join(' ')), 'a usage_json read outside USAGE_STATS_SQL');
  assert.deepEqual(rows.ledger.map((r) => [r.run_key, r.usage_stats, r.cost_floor_usd]).sort((x, y) => x[0].localeCompare(y[0])), [
    ['k-P1', { apiCalls: 40, peakContextTokens: 400_000, subagents: { calls: 3, tokens: 12_000 } }, 0.9],
    ['k-P2', null, 0.4], // not JSON: no figures
    ['k-P3', { apiCalls: null, peakContextTokens: null, subagents: null }, null],
  ]);
  assert.ok(!/SECRET/.test(JSON.stringify(rows)), 'readDb kept a text');

  await withScorecard((s) => {
    assert.equal(s.counts.ledgerRows, 5);
    assert.deepEqual(s.sources, { usageJson: true, costFloorUsd: true });
    // Overall, over the 3 finished top-level runs: run-P1 alone carries the
    // figures (the consultation's 900K and the checkpoint's 800K stay out).
    assert.deepEqual(s.ledger, { runs: 3, peakContext: { n: 1, p50: 400_000, p95: 400_000 }, apiCalls: { n: 1, p50: 40, p95: 40 }, subagents: { runs: 1, calls: 3, tokens: 12_000 } });
    // A group's block keeps every row of its own, the checkpoint included.
    const c = s.groups['chat/claude'];
    assert.equal(c.ledger.runs, 3);
    assert.deepEqual(c.ledger.peakContext, { n: 2, p50: 400_000, p95: 800_000 });
    assert.deepEqual(s.groups['delegate/claude'].ledger.peakContext, { n: 1, p50: 900_000, p95: 900_000 });
    assert.deepEqual(s.groups['chat/codex'].ledger, { runs: 1, peakContext: null, apiCalls: null, subagents: null });
    // The floor stays apart: known cost is run-P1's $2 alone, and run-P1's
    // own $0.90 floor (it has a known cost) is not in the floor sum.
    assert.equal(c.cost.partialFloorUsd, 0.4);
    assert.equal(c.cost.partialFloorRuns, 1);
    assert.equal(c.cost.floorUsd, 2);
    assert.equal(c.cost.knownRuns, 1);
    assert.equal(c.cost.p50Usd, 2);
    assert.equal(c.cost.totalUsd, null);
    assert.equal(c.cost.partialCostUsd, 0);
    assert.equal(s.groups['chat/codex'].cost.partialFloorUsd, null);
    assert.deepEqual(c.latency.firstModelMs, { n: 1, p50: 2000, p95: 2000 });
    assert.deepEqual(c.latency.firstTextMs, { n: 1, p50: 2500, p95: 2500 });
    assert.deepEqual(c.toolsBeforeText, { n: 1, p50: 3, p95: 3 });
    const md = renderSummary(s);
    assert.match(md, /^- Peak context over top-level runs: p50\/p95 400K \/ 400K \(n 1 of 3 runs\); API calls per run: p50\/p95 40 \/ 40 \(n 1 of 3 runs\); subagents in 1 run, 3 calls, 12K tokens\.$/m);
    assert.match(md, /^- Partial-run cost floor \(never in known cost\): \$0\.40 over 1 run\.$/m);
    assert.match(md, /^- First model event p50\/p95: chat\/claude 2\.0 s \/ 2\.0 s \(n 1\), tools before text p50 3\.$/m);
    assert.ok(!/SECRET/.test(JSON.stringify(s) + md), 'a usage_json text reached the report');
  }, [], { extra: of28AndOthers, vault: of28Vault(dir), from: of28.from, to: of28.to });
});

test('OF28: a store before OB05 and OB07 and receipts before OB08a read null, never 0, and render "not installed"', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-of28-old-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const { db } = await openDb(buildDb(dir, preOf28Rows));
  let rows;
  try {
    assert.equal(hasColumn(db, 'usage_ledger', 'usage_json'), false);
    rows = readDb(db, { since, until });
  } finally {
    db.close();
  }
  assert.equal(rows.ledger.length, 11);
  assert.ok(rows.ledger.every((r) => r.usage_stats === null && r.cost_floor_usd === null));
  await withScorecard((s) => {
    assert.equal(s.counts.ledgerRows, 11);
    assert.deepEqual(s.sources, { usageJson: false, costFloorUsd: false });
    // Over the 7 finished top-level runs: k1–k6 and interrupted k10 (not
    // oneshot k7, running k9 or the consultations k11, k12).
    assert.deepEqual(s.ledger, { runs: 7, peakContext: null, apiCalls: null, subagents: null });
    for (const g of Object.values(s.groups)) {
      assert.equal(g.ledger.peakContext, null);
      assert.equal(g.cost.partialFloorUsd, null);
      assert.equal(g.cost.partialFloorRuns, 0);
      assert.equal(g.latency.firstModelMs.p50, null);
      assert.equal(g.latency.firstModelMs.n, 0);
    }
    // Known cost as before (k1, k2, k4).
    assert.equal(s.groups['chat/claude'].cost.floorUsd, 6);
    const md = renderSummary(s);
    assert.match(md, /^- Peak context over top-level runs: not installed \(0 of 7 runs carry it\); API calls per run: not installed \(0 of 7 runs carry it\)\.$/m);
    assert.match(md, /^- Partial-run cost floor \(never in known cost\): not installed \(0 of 11 runs carry it\)\.$/m);
    assert.match(md, /^- First model event: not installed \(0 of 11 runs carry it\)\.$/m);
  }, [], { extra: preOf28Rows });
  // A store with OB07's column but no floor in the window: "none", not
  // "not installed".
  const floorColumnOnly = (db) => {
    if (!hasColumn(db, 'usage_ledger', 'cost_floor_usd')) db.exec('alter table usage_ledger add column cost_floor_usd REAL');
  };
  await withScorecard((s) => {
    assert.equal(s.sources.costFloorUsd, true);
    for (const g of Object.values(s.groups)) assert.equal(g.cost.partialFloorUsd, null);
    assert.match(renderSummary(s), /^- Partial-run cost floor \(never in known cost\): none in this window \(0 of 11 runs\)\.$/m);
  }, [], { extra: floorColumnOnly });
});

test('OF28: compare reads null for a scorecard without the keys, and prints a line for each', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-of28-cmp-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  await withScorecard((b) => {
    // A scorecard from before OF28.
    const a = structuredClone(b);
    delete a.ledger;
    for (const g of Object.values(a.groups)) {
      delete g.ledger;
      delete g.toolsBeforeText;
      delete g.latency.firstModelMs;
      delete g.cost.partialFloorUsd;
      delete g.cost.partialFloorRuns;
    }
    const c = compareScorecards(a, b);
    assert.deepEqual(c.ledger, {
      peakContext: { a: null, b: { p50: 400_000, p95: 400_000, n: 1 }, p50Change: null, p95Change: null },
      apiCalls: { a: null, b: { p50: 40, p95: 40, n: 1 }, p50Change: null, p95Change: null },
      subagentCalls: { a: null, b: 3 },
      partialFloorUsd: { a: null, b: 0.4 },
    });
    const g = c.groups['chat/claude'];
    assert.deepEqual(g.latency.firstModelMs, { a: { p50: null, p95: null, n: null }, b: { p50: 2000, p95: 2000, n: 1 }, p50Change: null, p95Change: null });
    assert.equal(g.peakContext.a, null);
    assert.deepEqual(g.partialFloorUsd, { a: null, b: 0.4 });
    assert.deepEqual(c.flags, []);
    const md = renderComparison(c);
    assert.match(md, /^Peak context p50\/p95 — → 400K \/ 400K \(n 1\); API calls per run — → 40 \/ 40 \(n 1\); partial-run cost floor — → \$0\.40\.$/m);
    assert.match(md, /^First model event p50 chat\/claude — \(n —\) → 2\.0 s \(n 1\)\.$/m);
    // The other way round, and both sides new.
    assert.equal(compareScorecards(b, a).ledger.peakContext.b, null);
    const same = compareScorecards(b, b);
    assert.equal(same.ledger.peakContext.p95Change, 0);
    assert.equal(same.groups['chat/claude'].latency.firstModelMs.p95Change, 0);
    // Neither side has them: nulls and no lines.
    const old = compareScorecards(a, a);
    assert.deepEqual(old.ledger.partialFloorUsd, { a: null, b: null });
    assert.doesNotMatch(renderComparison(old), /Peak context|First model event/);
  }, [], { extra: of28Rows, vault: of28Vault(dir), from: of28.from, to: of28.to });
});

// ── OF02: the tool pass ──────────────────────────────────────────────────
//
// Two transcripts (fixtures/scorecard/tool-transcripts): ns-t1 is chat s5's
// four turns, ns-t2 chat s6's two. s5: T1 is the session's first turn (new);
// T2 starts 0.5 s after T1 ended (resume < 5 min), and T1's last call a3b
// sits inside T2's 1 s slack but before T1's end, so it stays T1's; T3
// (started 20:00:00) resets 5 s in: the old stint closes and the new one
// opens at 20:00:05, as chat.rs announce_reset does inside the turn, so the
// stint covering T3's start is the old one (reset); T4 runs wholly in the
// new stint and starts 30 min after T3 ended (resume 5–60, not reset). T3
// hands over to s7 (no transcript; its first turn errored) and T4 to s6: U1
// (new), then U2 100 min after U1 ended (resume > 60). A fourth handover
// record names a run with no turn. Only main-thread calls count.

const toolTranscripts = join(fixture, 'tool-transcripts');
const toolRows = (db) => {
  const D = '2026-10-10T';
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('s5','claude','/v/workspaces/tools',?,?), ('s6','claude','/v/workspaces/tools',?,?), ('s7','claude','/v/workspaces/tools',?,?)`).run(D, D, D, D, D, D);
  const stint = db.prepare('insert into stints (session_key, agent, started_at, ended_at, reason) values (?,?,?,?,?)');
  stint.run('s5', 'claude', D + '19:00:00Z', D + '20:00:05Z', 'config changed');
  stint.run('s5', 'claude', D + '20:00:05Z', null, null);
  stint.run('s7', 'claude', D + '20:30:00Z', null, null);
  stint.run('s6', 'claude', D + '21:10:00Z', null, null);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values (?,?,?,?,?,?,?)');
  const ledger = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_status)
    values (?,?,?,'chat',?,'claude',?,'subscription','success',1,'per_run','unknown')`);
  for (const [key, ord, run, native, from, to, status, mode] of [
    ['s5', 1, 'run-T1', 'ns-t1', '19:00:00', '19:11:59.500', 'success', 'bypassPermissions'],
    ['s5', 2, 'run-T2', 'ns-t1', '19:12:00', '19:20:00', 'success', 'acceptEdits'],
    ['s5', 3, 'run-T3', 'ns-t1', '20:00:00', '20:30:00', 'success', 'bypassPermissions'],
    ['s5', 4, 'run-T4', 'ns-t1', '21:00:00', '21:10:00', 'success', 'bypassPermissions'],
    ['s7', 1, 'run-V1', null, '20:31:00', '20:40:00', 'error', 'bypassPermissions'],
    ['s6', 1, 'run-U1', 'ns-t2', '21:10:00', '21:20:00', 'success', 'bypassPermissions'],
    ['s6', 2, 'run-U2', 'ns-t2', '23:00:00', '23:05:00', 'success', 'bypassPermissions'],
  ]) {
    turn.run(key, ord, run, `${D}${from}Z`, `${D}${to}Z`, status, mode);
    if (native) ledger.run(`k-${run}`, `${D}${to}Z`, `${D}${from}Z`, run, native);
  }
  const delegation = db.prepare('insert into delegations (run_id, at, record_json) values (?,?,?)');
  delegation.run('run-T3', D + '20:29:00Z', JSON.stringify({ kind: 'childTask', tool: 'handover', childSession: 's7', packet: 'SECRET-MESSAGE-TEXT' }));
  delegation.run('run-T4', D + '21:09:00Z', JSON.stringify({ kind: 'childTask', tool: 'handover', childSession: 's6', packet: 'SECRET-MESSAGE-TEXT' }));
  // A handover from a run with no turn row (and no successor named).
  delegation.run('run-gone', D + '22:00:00Z', JSON.stringify({ kind: 'childTask', tool: 'handover', packet: 'SECRET-MESSAGE-TEXT' }));
};

test('the tool pass over two transcripts gives the hand-computed numbers', async () => {
  await withScorecard((s) => {
    const t = s.transcripts;
    // Joined Claude chat turns: T1–T4 and U1–U2 (V1 has no native session;
    // the base fixture's ns-1/ns-2 are not under this root).
    assert.equal(t.turns, 6);
    // Main-thread tool uses: T1 4 (tu1–tu4; a3b has none), T2 4 (tu5–tu8),
    // T3 3 (tu9–tu11), T4 3 (tu12–tu14; the sidechain ToolSearch at 21:05
    // is not main thread), U1 4 (tu20–tu23), U2 1 (tu24) = 19.
    assert.equal(t.toolCalls, 19);
    // ToolSearch tu1, tu5 (select), tu8 (keyword): 3 × 100 / 6 = 50, in T1 and T2.
    assert.equal(t.toolSearchPer100Turns, 50);
    assert.deepEqual(t.toolSearch, {
      calls: 3,
      selectCalls: 2,
      keywordCalls: 1,
      turnsWith: 2,
      turnsWithShare: 0.3333,
      // tu1 selects two, tu5 one plus an entry that is no tool name (counted
      // as `other`, its text dropped); tu8's keyword query is never split.
      selectedByTool: { mcp__hollow_app__wait_for_session: 2, mcp__hollow_tasks__list_tasks: 1, other: 1 },
    });
    // run_action T2, list_sessions T4, read_session U1, start_session T3,
    // send_to_session T4, describe_action none; one of the six: T2, T3, T4, U1.
    assert.deepEqual(t.alwaysLoad, {
      n: 6,
      turnsUsing: { run_action: 1, list_sessions: 1, read_session: 1, describe_action: 0, start_session: 1, send_to_session: 1, any: 4 },
      usedShare: { run_action: 0.1667, list_sessions: 0.1667, read_session: 0.1667, describe_action: 0, start_session: 0.1667, send_to_session: 0.1667, any: 0.6667 },
    });
    // First calls (cache write, cache read): new T1 a1 (3000, 0), U1 b1
    // (9000, 0) → p50 = 1st of 2 = 3000, 0, cold 2/2. Reset T3 a7 (6000, 0):
    // s5's stint closed 'config changed' at 20:00:05, inside T3 (20:00:00–
    // 20:30:00). Resume T2 a4 (500, 3200; not T1's a3b (77777, 0), which
    // is before T1's end), T4 a10 (800, 6100; the stint it runs in opened
    // after T3 started, which is not a reset of T4), U2 b3 (400, 0) → write
    // p50 = 2nd of 400/500/800 = 500, read p50 = 2nd of 0/3200/6100 = 3200,
    // cold 1/3.
    assert.deepEqual(t.firstCall.bySession, {
      new: { n: 2, cacheWriteP50: 3000, cacheReadP50: 0, coldShare: 1 },
      reset: { n: 1, cacheWriteP50: 6000, cacheReadP50: 0, coldShare: 1 },
      resume: { n: 3, cacheWriteP50: 500, cacheReadP50: 3200, coldShare: 0.3333 },
    });
    // Gaps: T2 19:12:00 − T1's 19:11:59.5 = 0.5 s; T4 21:00 − T3's 20:30 =
    // 30 min; U2 23:00 − U1's 21:20 = 100 min.
    assert.deepEqual(t.firstCall.resumeByGap, {
      lt5m: { n: 1, cacheWriteP50: 500, cacheReadP50: 3200, coldShare: 0 },
      m5to60: { n: 1, cacheWriteP50: 800, cacheReadP50: 6100, coldShare: 0 },
      gt60m: { n: 1, cacheWriteP50: 400, cacheReadP50: 0, coldShare: 1 },
      unknown: { n: 0, cacheWriteP50: null, cacheReadP50: null, coldShare: null },
    });
    // tu2 (wait_for_session, finished false, status running) and tu7
    // (run_action sessions.wait, finished true).
    assert.deepEqual(t.waitOnlyCalls, { calls: 2, stillRunning: 1, stillRunningByStatus: { queued: 0, running: 1, other: 0 }, ended: 1, unclassified: 0, stillRunningShare: 0.5 });
    // Bash: tu3 (T1 bypass, "Permission … denied": permission), tu6 (T2
    // acceptEdits, "Exit code 128" then git's "Permission denied
    // (publickey)": the command ran, so exit-code, not refused), tu10 (T3
    // bypass, "requires approval": permission), tu24 (U2 bypass, ok). The
    // orphan result (tu-before-compaction) answers no tool use here and is
    // never read.
    assert.deepEqual(t.bashRefusalRate, {
      acceptEdits: { bashCalls: 1, errors: 1, refused: 0, rate: 0 },
      all: { bashCalls: 4, errors: 3, refused: 2, rate: 0.5 },
      bypassPermissions: { bashCalls: 3, errors: 2, refused: 2, rate: 0.6667 },
    });
    // T3's handover at 20:29: its last call at or before is a8 (1 + 100 +
    // 6000 = 6101; a9 at 20:29:30 is after). T4's at 21:09: a11 (1 + 300 +
    // 6900 = 7201). Unjoined: run-gone has no turn; run-A's turn has no
    // joined transcript here; the successor's first turn isn't joined for
    // run-A (names none) and T3 (s7's V1 has no transcript).
    assert.deepEqual(t.handoverContext, { n: 2, p50: 6101, p95: 7201, unjoined: { noPredTurnInWindow: 1, noTranscript: 1, noSuccessorJoined: 2 } });
    // T4 → s6: U1 Read a.md and c.md (a.md twice: distinct files); s5 had
    // touched a.md (T1 Read), b.md (T3 Read) and c.md (T4 Edit) → 2 of 2,
    // predecessor set 3.
    assert.deepEqual(t.handoverRereadOverlap, { handovers: 1, successorReads: 2, predecessorTouched: 3, reread: 2, share: 1 });
    // s6's first turn U1 succeeded, s7's V1 errored; run-A's and run-gone's
    // records name none.
    assert.deepEqual(s.delegation.handover, { records: 4, childFirstTurnOk: { ok: 1, n: 2, share: 0.5 }, noEndedFirstTurn: 2 });
    const md = renderSummary(s);
    assert.match(md, /- Handover successors whose first turn ended ok: 50\.0% \(1\/2\)\./);
    assert.match(md, /- Tools over 6 Claude turns: ToolSearch 50 per 100 turns, in 33\.3% \(2\/6\) of turns; always-loaded tools used in run_action 16\.7%/);
    assert.match(md, /wait_for_session calls, 50\.0% \(1\/2\) ended with the child still running\. Bash permission refusals: acceptEdits 0\.0% \(0\/1\), all 50\.0% \(2\/4\), bypassPermissions 66\.7% \(2\/3\)\./);
    // Nothing from a message, tool input, tool result or path reaches the report.
    const out = JSON.stringify(s) + md;
    assert.ok(!/SECRET|should-not-leak|\/repo\//.test(out), 'tool pass leaked text');
  }, [toolTranscripts], { extra: toolRows });
});

test('compare prints the tool pass lines, and — for a scorecard without them', async () => {
  await withScorecard((a) => {
    const b = structuredClone(a);
    b.transcripts.toolSearchPer100Turns = 20;
    const c = compareScorecards(a, b);
    assert.deepEqual(c.tools.toolSearchPer100Turns, { a: 50, b: 20 });
    assert.deepEqual(c.tools.firstCallColdShare.resume, { a: 0.3333, b: 0.3333, n: { a: 3, b: 3 } });
    const md = renderComparison(c);
    assert.match(md, /ToolSearch per 100 turns 50 → 20 \(turns 6 → 6\)\./);
    assert.match(md, /First-call cold share new 100\.0% \(n 2\) → 100\.0% \(n 2\); reset 100\.0% \(n 1\) → 100\.0% \(n 1\); resume 33\.3% \(n 3\) → 33\.3% \(n 3\)\./);
    // An evening baseline written before OF02 has no transcripts block.
    const old = structuredClone(a);
    delete old.transcripts;
    delete old.delegation.handover;
    const o = compareScorecards(old, b);
    assert.deepEqual(o.tools.toolSearchPer100Turns, { a: null, b: 20 });
    assert.match(renderComparison(o), /ToolSearch per 100 turns — → 20/);
    // Neither side has it: no tool lines.
    delete b.transcripts;
    assert.doesNotMatch(renderComparison(compareScorecards(old, b)), /ToolSearch per 100 turns/);
  }, [toolTranscripts], { extra: toolRows });
});

// ── OF19: the context anatomy ────────────────────────────────────────────
//
// Built per test in a temp folder (sizes exact by construction): two main
// transcripts, one subagent and one Workflow-tool agent. Every string a
// prompt, text, tool input, tool result or attachment holds starts with
// SECRET, padded to an exact byte count.
//
// ns-a1 is chat s8: W1 (19:00–19:10, bypass, new) and W2 (21:00–21:10,
// acceptEdits, 110 min later: a cold resume). ns-a2 is chat s9: X1 (19:30–
// 19:40, bypass, new) and X2 (19:42–19:50, bypass, 2 min later: warm), with
// a compaction inside X1. Calls (input, cache write [1-hour part], read,
// output) and what each brings into the thread after it:
//   c1 2/5000[4000]/0/10    text 120, Bash vitest `| tail` 200 → result 1000 "Tests 2 failed" (masked)
//   c2 1/300[300]/5000/20   thinking 80, Read 100 → result 2000
//   c3 1/200[0]/7300/30     Bash python3 -c edit script 300 → result 400
//   c4 1/100/7500/10        wait_for_session 60 → result 100 (a poll-only call)
//   c5 2/8000[8000]/0/10    Bash `rg` 100 → result 200 holding "Tests 2 failed" (not a check)
//   c6 1/50/8000/10         text 40
//   d1 3/4000/0/10          Bash `npx vitest run` 120 → error result 300
//   d2 1/100/4000/10        Bash vitest `| tail` 140 → result 160 (passed)
//   (compaction; summary 500)
//   d3 1/600/2000/10        text 40
//   d4 1/300/2500/10        Bash `sleep 30; …` 80 → result 200 (a poll-only call)
//   d5 1/50/2900/10         text 40
// Before c1: prompt 4000 (first), instructions 800, skill listing 400, an
// environment attachment 40 (rendered) and a date attachment (not rendered).
// Before c5: prompt 600. Before d1: a meta message 100, then the first
// prompt 2000 (a text part). Before d4: prompt 400.

const SECRET = (tag, n) => `SECRET-${tag}-`.padEnd(n, 'x');
const A = '2026-10-10T';

/** The anatomy transcripts under `dir`/proj; returns the root. */
function anatomyTranscripts(dir) {
  const root = join(dir, 'anatomy');
  const proj = join(root, 'proj');
  mkdirSync(join(proj, 'ns-a1', 'subagents', 'workflows', 'wf_1'), { recursive: true });
  const usage = ([input, write, read, output], oneHour) => ({
    input_tokens: input,
    cache_creation_input_tokens: write,
    cache_read_input_tokens: read,
    output_tokens: output,
    ...(oneHour == null ? {} : { cache_creation: { ephemeral_1h_input_tokens: oneHour, ephemeral_5m_input_tokens: write - oneHour } }),
  });
  const call = (id, t, u, oneHour, content, extra = {}) => JSON.stringify({ type: 'assistant', isSidechain: false, timestamp: A + t, uuid: `${id}-u`, message: { id, role: 'assistant', content, usage: usage(u, oneHour) }, ...extra });
  const prompt = (t, content, extra = {}) => JSON.stringify({ type: 'user', timestamp: A + t, message: { role: 'user', content }, ...extra });
  const result = (t, id, content, isError = false) => JSON.stringify({ type: 'user', timestamp: A + t, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: id, content, ...(isError ? { is_error: true } : {}) }] }, toolUseResult: SECRET('USE-RESULT', 40) });
  const attachment = (t, type, rendered) => JSON.stringify({ type: 'attachment', timestamp: A + t, attachment: { type, body: SECRET('ATTACHMENT', 30) }, ...(rendered == null ? {} : { rendered }) });
  const text = (n) => ({ type: 'text', text: SECRET('TEXT', n) });
  // A tool input of exactly n bytes as JSON: {"command":"…"} is 14 + the command.
  const bash = (id, head, n) => ({ type: 'tool_use', id, name: 'Bash', input: { command: `${head} # ${SECRET('CMD', 0)}`.padEnd(n - 14, 'x') } });
  const tail = (n) => `${'Test Files  1 failed | 3 passed (4)\n      Tests  2 failed | 8 passed (10)\n'}${SECRET('RESULT', 0)}`.padEnd(n, 'x');
  const passed = `${'Test Files  4 passed (4)\n      Tests  10 passed (10)\n'}${SECRET('RESULT', 0)}`.padEnd(160, 'x');
  const wait = JSON.stringify({ finished: false, status: 'running', pad: SECRET('WAIT', 54) });
  assert.equal(wait.length, 100);
  writeFileSync(join(proj, 'ns-a1.jsonl'), `${[
    prompt('19:00:01Z', SECRET('PROMPT', 4000)),
    attachment('19:00:01Z', 'instructions', SECRET('CLAUDE-MD', 800)),
    attachment('19:00:01Z', 'skill_listing', SECRET('SKILLS', 400)),
    attachment('19:00:01Z', 'environment', SECRET('ENV', 40)),
    attachment('19:00:01Z', 'date', null),
    call('c1', '19:00:10Z', [2, 5000, 0, 10], 4000, [text(120)]),
    call('c1', '19:00:11Z', [2, 5000, 0, 10], 4000, [bash('tu-a', 'cd /r && npx vitest run 2>&1 | tail -5', 200)]),
    result('19:00:30Z', 'tu-a', tail(1000)),
    call('c2', '19:01:00Z', [1, 300, 5000, 20], 300, [{ type: 'thinking', thinking: SECRET('THINK', 80) }]),
    call('c2', '19:01:01Z', [1, 300, 5000, 20], 300, [{ type: 'tool_use', id: 'tu-b', name: 'Read', input: { file_path: `/repo/${SECRET('PATH', 78)}` } }]),
    result('19:01:30Z', 'tu-b', [{ type: 'text', text: SECRET('FILE', 2000) }]),
    call('c3', '19:02:00Z', [1, 200, 7300, 30], 0, [bash('tu-c', "python3 -c 'from pathlib import Path; Path(1).write_text(2)'", 300)]),
    result('19:02:30Z', 'tu-c', SECRET('PYOUT', 400)),
    call('c4', '19:03:00Z', [1, 100, 7500, 10], null, [{ type: 'tool_use', id: 'tu-d', name: 'mcp__hollow_app__wait_for_session', input: { key: SECRET('KEY', 50) } }]),
    result('19:04:00Z', 'tu-d', wait),
    prompt('21:00:01Z', SECRET('PROMPT2', 600)),
    call('c5', '21:00:10Z', [2, 8000, 0, 10], 8000, [bash('tu-e', 'rg -n failed docs', 100)]),
    result('21:00:30Z', 'tu-e', `Tests  2 failed ${SECRET('GREP', 0)}`.padEnd(200, 'x')),
    call('c6', '21:01:00Z', [1, 50, 8000, 10], null, [text(40)]),
  ].join('\n')}\n`);
  writeFileSync(join(proj, 'ns-a1', 'subagents', 'agent-s.jsonl'), `${call('s1', '19:05:00Z', [10, 1000, 0, 100], null, [{ type: 'tool_use', id: 'tu-s', name: 'Read', input: { file_path: SECRET('SUBPATH', 30) } }], { isSidechain: true })}\n`);
  // Two calls in the window (the second after W2 ended: the Workflow tool
  // runs in the background) and one the next day.
  writeFileSync(join(proj, 'ns-a1', 'subagents', 'workflows', 'wf_1', 'agent-w.jsonl'), `${[
    call('w1', '19:06:00Z', [0, 2000, 1000, 200], 2000, [text(50)], { isSidechain: true }),
    call('w2', '21:20:00Z', [0, 0, 3000, 100], null, [text(50)], { isSidechain: true }),
    call('w3', '01:00:00Z', [0, 0, 9999, 9999], null, [text(50)], { isSidechain: true, timestamp: '2026-10-11T01:00:00Z' }),
  ].join('\n')}\n`);
  writeFileSync(join(proj, 'ns-a2.jsonl'), `${[
    prompt('19:30:00.500Z', SECRET('META', 100), { isMeta: true }),
    prompt('19:30:01Z', [text(2000)]),
    call('d1', '19:30:10Z', [3, 4000, 0, 10], null, [bash('tu-f', 'npx vitest run', 120)]),
    result('19:30:30Z', 'tu-f', `Exit code 1\n Tests  1 failed ${SECRET('ERR', 0)}`.padEnd(300, 'x'), true),
    call('d2', '19:31:00Z', [1, 100, 4000, 10], null, [bash('tu-g', 'cd /r && npx vitest run 2>&1 | tail -3', 140)]),
    result('19:31:30Z', 'tu-g', passed),
    JSON.stringify({ type: 'system', subtype: 'compact_boundary', timestamp: A + '19:35:00Z', content: SECRET('COMPACT', 20) }),
    prompt('19:35:01Z', SECRET('SUMMARY', 500), { isCompactSummary: true }),
    call('d3', '19:36:00Z', [1, 600, 2000, 10], null, [text(40)]),
    prompt('19:42:01Z', SECRET('PROMPT3', 400)),
    call('d4', '19:42:10Z', [1, 300, 2500, 10], null, [bash('tu-h', 'sleep 30; gh pr checks 1', 80)]),
    result('19:42:50Z', 'tu-h', SECRET('CHECKS', 200)),
    call('d5', '19:43:00Z', [1, 50, 2900, 10], null, [text(40)]),
  ].join('\n')}\n`);
  return root;
}

const anatomyRows = (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('s8','claude','/v/workspaces/anat',?,?), ('s9','claude','/v/workspaces/anat',?,?)`).run(A, A, A, A);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values (?,?,?,?,?,?,?)');
  const ledger = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_usd, cost_status)
    values (?,?,?,'chat',?,'claude',?,'subscription','success',1,'per_run',1,'estimated')`);
  for (const [key, ord, run, native, from, to, mode] of [
    ['s8', 1, 'run-W1', 'ns-a1', '19:00:00', '19:10:00', 'bypassPermissions'],
    ['s8', 2, 'run-W2', 'ns-a1', '21:00:00', '21:10:00', 'acceptEdits'],
    ['s9', 1, 'run-X1', 'ns-a2', '19:30:00', '19:40:00', 'bypassPermissions'],
    ['s9', 2, 'run-X2', 'ns-a2', '19:42:00', '19:50:00', 'bypassPermissions'],
  ]) {
    turn.run(key, ord, run, `${A}${from}Z`, `${A}${to}Z`, 'success', mode);
    ledger.run(`k-${run}`, `${A}${to}Z`, `${A}${from}Z`, run, native);
  }
};

async function withAnatomy(fn) {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-anatomy-${process.pid}-`));
  try {
    const root = anatomyTranscripts(dir);
    return await withScorecard((s) => fn(s, root), [root], { extra: anatomyRows });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('the context anatomy over two transcripts, a subagent and a workflow agent gives the hand-computed numbers', async () => {
  await withAnatomy((s) => {
    const t = s.transcripts;
    assert.equal(t.turns, 4);
    // Cache writes: W1's first call c1 5000 and X1's d1 4000 open a session
    // (9000); W2's c5 8000 resumes 110 min after W1 ended (cold); X2's d4
    // 300 resumes after 2 min (warm); the rest are incremental (c2–c4 600,
    // c6 50, d2–d3 700, d5 50 = 1400). Of 18,700. The 1-hour parts: c1
    // 4000, c2 300, c5 8000.
    assert.deepEqual(t.cacheWrite, {
      tokens: 18700,
      oneHourTokens: 12300,
      byCause: {
        sessionFirst: { tokens: 9000, share: 0.4813 },
        coldResume: { tokens: 8000, share: 0.4278 },
        warmResume: { tokens: 300, share: 0.016 },
        resumeUnknownGap: { tokens: 0, share: 0 },
        incremental: { tokens: 1400, share: 0.0749 },
      },
    });
    assert.deepEqual(t.coldResume, { turns: 1, cacheWriteP50: 8000, warmCacheWriteP50: 300, warmTurns: 1, cacheWriteTokens: 8000, share: 0.4278 });
    // Carried bytes per call, summed over the 11 main calls (÷ 4 = tokens):
    // first prompts 4000 × 6 (c1–c6) + 2000 × 2 (d1, d2; the compaction
    // drops it) = 28000 → 7000; instructions 800 × 6 → 1200; skill listing
    // 400 × 6 → 600; the rendered environment 40 × 6 → 60 (the date
    // attachment has no rendering); assistant text 120 × 5 (c2–c6) + 40 × 2
    // (d4, d5) = 680 → 170; thinking 80 × 4 (c3–c6) → 80; Bash commands
    // c2 200, c3 200, c4 500, c5 500, c6 600, d2 120, d5 80 = 2200 → 550;
    // other tool inputs c3 100, c4 100, c5 160, c6 160 = 520 → 130; tool
    // results c2 1000, c3 3000, c4 3400, c5 3500, c6 3700, d2 300, d5 200 =
    // 15100 → 3775; prompts c5, c6 600 each, d1, d2 the meta 100 each, d4,
    // d5 400 each = 2200 → 550; the compaction summary d3–d5 500 × 3 → 375.
    // Context: 5002 + 5301 + 7501 + 7601 + 8002 + 8051 + 4003 + 4101 + 2601
    // + 2801 + 2951 = 57915. The subagent's and workflow's calls carry none.
    const ca = t.carried;
    assert.equal(ca.calls, 11);
    assert.equal(ca.contextTokens, 57915);
    assert.deepEqual(Object.keys(ca.byKind), CARRIED_KINDS);
    assert.deepEqual(Object.fromEntries(Object.entries(ca.byKind).map(([k, v]) => [k, v.tokens])), {
      toolResults: 3775,
      bashCommands: 550,
      toolInputs: 130,
      firstPrompts: 7000,
      prompts: 550,
      compactSummary: 375,
      instructions: 1200,
      skillListing: 600,
      otherAttachments: 60,
      assistantText: 170,
      thinking: 80,
    });
    assert.deepEqual(ca.byKind.toolResults, { tokens: 3775, share: 0.0652 });
    assert.deepEqual(ca.byKind.firstPrompts, { tokens: 7000, share: 0.1209 });
    // Read 2000 × 4 (c3–c6) → 2000; Bash c2 1000, c3 1000, c4 1400, c5
    // 1400, c6 1600, d2 300, d5 200 = 6900 → 1725; the wait 100 × 2 → 50.
    assert.deepEqual(ca.toolResultsByTool, {
      Read: { tokens: 2000, share: 0.0345 },
      Bash: { tokens: 1725, share: 0.0298 },
      mcp__hollow_app__wait_for_session: { tokens: 50, share: 0.0009 },
    });
    // c3's script, carried by c4, c5, c6: 300 × 3 → 225.
    assert.deepEqual(ca.pythonEditScripts, { tokens: 225, share: 0.0039 });
    assert.equal(ca.measuredShare, 0.2502); // 14490 / 57915
    // c4 (a wait; 7601) and d4 (a sleep; 2801), both in bypass turns.
    assert.deepEqual(t.pollOnlyCalls, {
      all: { calls: 2, contextTokens: 10402 },
      acceptEdits: { calls: 0, contextTokens: 0 },
      bypassPermissions: { calls: 2, contextTokens: 10402 },
    });
    // Bash: W1 tu-a, tu-c (a script); W2 tu-e; X1 tu-f, tu-g; X2 tu-h.
    assert.deepEqual(t.pythonEditScripts, {
      all: { bashCalls: 6, scripts: 1, per100Bash: 16.67, editCalls: 0 },
      acceptEdits: { bashCalls: 1, scripts: 0, per100Bash: 0, editCalls: 0 },
      bypassPermissions: { bashCalls: 5, scripts: 1, per100Bash: 20, editCalls: 0 },
    });
    // Bash results 1000, 400, 200, 300, 160, 200 (main thread only).
    assert.deepEqual(t.bashResultBytes, { n: 6, p50: 200, p95: 1000, p99: 1000, total: 2260 });
    assert.deepEqual(Object.keys(t.toolResultBytes.byTool), ['Bash', 'Read', 'mcp__hollow_app__wait_for_session']);
    assert.deepEqual(t.toolResultBytes.byTool.Read, { n: 1, p50: 2000, p95: 2000, p99: 2000, total: 2000 });
    assert.equal(t.costShareAtLarge, 0);
    // The workflow agent's w1 and w2 (w3 is the next day): weights 2000 × 2
    // + 1000 × 0.1 + 200 × 5 = 5100 and 300 + 500 = 800. The joined calls'
    // weight (1-hour writes at 2): c1 9302, c2 1201, c3 1131, c4 926, c5
    // 16052, c6 913.5, d1 5053, d2 576, d3 1001, d4 676, d5 403.5, subagent
    // s1 1760 = 38995; four runs at $1 each.
    assert.deepEqual(t.workflowAgents, {
      agents: 1,
      sessions: 1,
      calls: 2,
      tokens: { input: 0, cacheWrite: 2000, cacheRead: 4000, output: 300 },
      weightedTokens: 5900,
      weightShare: 0.1314, // 5900 / 44895
      usdAtJoinedRate: 0.61, // 5900 × 4 / 38995
    });
    const md = renderSummary(s);
    assert.match(md, /- Main-thread cache writes 19K \(1-hour 65\.8%\) by cause: sessionFirst 48\.1%, coldResume 42\.8%, warmResume 1\.6%, resumeUnknownGap 0\.0%, incremental 7\.5%\./);
    assert.match(md, /- Workflow agents: 1 in 1 sessions, 2 calls, 13\.1% of joined weighted tokens, about \$0\.61/);
  });
});

test('a 1-hour cache write weighs 2, a 5-minute one 1.25', async () => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-weight-${process.pid}-`));
  try {
    // A usage-only read (my-context.mjs) never touches message.content.
    const touched = [];
    const parse = (line) => {
      const v = JSON.parse(line);
      if (v?.message && 'content' in v.message) {
        const content = v.message.content;
        Object.defineProperty(v.message, 'content', { get: () => (touched.push(v.type), content) });
      }
      return v;
    };
    const calls = await readCalls(join(anatomyTranscripts(dir), 'proj', 'ns-a1.jsonl'), false, { parse });
    assert.deepEqual(touched, []);
    // c1: 2 + 1000 × 1.25 + 4000 × 2 + 10 × 5 (6302 with every write at
    // 1.25); c2: 1 + 300 × 2 + 5000 × 0.1 + 20 × 5; c3 (split, none 1-hour)
    // and c4, c6 (no split) at 1.25; c5: 2 + 8000 × 2 + 10 × 5.
    assert.deepEqual(calls.map(weight), [9302, 1201, 1131, 926, 16052, 913.5]);
    assert.deepEqual(calls.map((c) => c.cacheWrite1h), [4000, 300, 0, 0, 8000, 0]);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a piped "Tests 2 failed" at exit 0 counts 1 masked failure', async () => {
  await withAnatomy((s) => {
    // tu-a (vitest piped into tail, clean exit) holds vitest's failure
    // line; tu-g passed; tu-f errored (not masked: its error shows); tu-e's
    // `rg` output holds the same words but runs no check.
    assert.deepEqual(s.toolErrors.maskedFailures, { cleanCheckResults: 2, masked: 1, byRunner: { vitest: 1 } });
    assert.match(renderSummary(s), /masked failures 1 of 2 clean check results/);
  });
});

test('a check is decided by a segment head, not by a runner name anywhere in the command', async () => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-check-${process.pid}-`));
  try {
    // Four clean Bash results, each holding vitest's failure line.
    const fail = 'Test Files  1 failed | 3 passed (4)\n      Tests  2 failed | 8 passed (10)';
    const cases = [
      ['grep -n "vitest" docs/BUILD_LOG.md', `412: ${fail}`],
      ['cat tsconfig.json', fail],
      ['npm run dev', fail],
      ['npm test 2>&1 | tail -3', fail],
    ];
    const usage = { input_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 1 };
    const lines = cases.flatMap(([command, out], i) => [
      JSON.stringify({ type: 'assistant', timestamp: `${A}19:0${i}:00Z`, message: { id: `m${i}`, role: 'assistant', content: [{ type: 'tool_use', id: `t${i}`, name: 'Bash', input: { command } }], usage } }),
      JSON.stringify({ type: 'user', timestamp: `${A}19:0${i}:30Z`, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: `t${i}`, content: out }] } }),
    ]);
    const file = join(dir, 'checks.jsonl');
    writeFileSync(file, `${lines.join('\n')}\n`);
    const calls = await readCalls(file, false, { toolPass: true });
    assert.deepEqual(calls.flatMap((c) => c.uses.map((u) => [u.check, u.masked])), [[false, null], [false, null], [false, null], [true, 'vitest']]);
    assert.deepEqual(maskedBlock(calls), { cleanCheckResults: 1, masked: 1, byRunner: { vitest: 1 } });
    // Without the grep, cat and dev server: 0 masked.
    assert.deepEqual(maskedBlock(calls.slice(0, 3)), { cleanCheckResults: 0, masked: 0, byRunner: {} });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  for (const c of [
    'npx vitest run',
    "/bin/zsh -lc 'cd /r && npx vitest run 2>&1 | tail -5'",
    'FORCE_COLOR=0 npm run test:unit',
    'npm run gate > /tmp/g.log',
    'pnpm lint',
    'yarn typecheck',
    'bun test',
    'cargo test -p hollow',
    'cargo +nightly clippy',
    'node --test scripts/x.test.mjs',
    'npx playwright test e2e',
    'npx tsc --noEmit',
    'python3 -m pytest -q',
    'pytest tests',
    'go test ./...',
    'timeout 600 cargo build --release',
    '(cd app && npm test)',
    '{ cargo test; }',
  ]) assert.equal(isCheck(c), true, c);
  for (const c of [
    'grep -n "vitest" docs/BUILD_LOG.md',
    'rg -n "cargo test" src',
    'cat tsconfig.json',
    'cat jest.config.js',
    'sed -n 1,40p eslint.config.js',
    'npm run dev',
    'npm install',
    'git commit -m "npm test passes"',
    'echo npx vitest run',
    'node scripts/build.mjs --test',
    'cargo run --bin x',
    'npx playwright install chromium',
    // An allow-list: a runner outside it is not a check.
    'make test',
    'npm --prefix app test',
  ]) assert.equal(isCheck(c), false, c);
});

test('no output string holds a prompt, text, tool input, tool result or attachment', async () => {
  await withAnatomy((s, root) => {
    const out = JSON.stringify(s) + renderSummary(s);
    // Every string in a prompt, a text or thinking part, a tool input or
    // result, or an attachment in the fixture (not tool names or ids).
    const strings = new Set();
    const walk = (v) => {
      if (typeof v === 'string') strings.add(v);
      else if (v && typeof v === 'object') for (const x of Object.values(v)) walk(x);
    };
    for (const f of ['ns-a1.jsonl', 'ns-a2.jsonl', 'ns-a1/subagents/agent-s.jsonl', 'ns-a1/subagents/workflows/wf_1/agent-w.jsonl']) {
      for (const line of readFileSync(join(root, 'proj', f), 'utf8').split('\n').filter(Boolean)) {
        const e = JSON.parse(line);
        const content = e.message?.content;
        if (typeof content === 'string') strings.add(content);
        for (const p of Array.isArray(content) ? content : []) [p.text, p.thinking, p.input, p.content].forEach(walk);
        [e.attachment?.body, e.rendered, e.toolUseResult, e.content].forEach(walk);
      }
    }
    assert.ok(strings.size > 30);
    for (const x of strings) if (x.length >= 8) assert.ok(!out.includes(x), `output holds fixture text (${x.slice(0, 20)}…)`);
    assert.ok(!out.includes('SECRET'), 'output holds a SECRET marker');
  });
});

test('the Python-edit and poll classifiers are shared with the morning report', () => {
  assert.equal(morningReport.isPythonEdit, isPythonEdit);
  assert.equal(morningReport.isPoll, isPoll);
  assert.equal(morningReport.isPollOnly, isPollOnly);
  assert.equal(isPoll('Bash', 'sleep 30; gh pr checks 1'), true);
  assert.equal(isPoll('mcp__hollow_app__wait_for_session', null), true);
  // A wait behind shell setup still opens the command.
  assert.equal(isPoll('Bash', 'cd /r/wt && sleep 60; gh pr checks 731'), true);
  assert.equal(isPoll('Bash', 'cd "/r/a b"; set -e; export GH_PAGER=; sleep 90 && gh run list'), true);
  assert.equal(isPoll('Bash', 'PR=731; sleep 45; gh pr view $PR'), true);
  assert.equal(isPoll('Bash', "/bin/zsh -lc 'cd /r && sleep 30; git status'"), true);
  assert.equal(isPoll('Bash', 'cd /r && ./scripts/wait-for-ci.sh 731'), true);
  // A wait after real work is not a poll; nor is a script named after one.
  assert.equal(isPoll('Bash', 'cd /r && npm test; sleep 5'), false);
  assert.equal(isPoll('Bash', 'cd /r && cat sleep.md'), false);
  assert.equal(isPoll('Bash', '. ./env.sh; git push'), false);
});

// ── OF20: tool errors per 100 calls ──────────────────────────────────────
//
// 2026-10-12: a Codex lane chat (one turn, 100 tool calls, 10 errors: 3 an
// `rg` with no output, 7 failures) against a Claude chat of 10 turns and
// 100 calls with 2 errors. The base fixture (10-10) is outside the window.

const D12 = '2026-10-12T';
const of20Rows = (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('sc','codex','/v/workspaces/lane',?,?), ('sd','claude','/v/workspaces/chat',?,?)`).run(D12, D12, D12, D12);
  const turn = db.prepare("insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values (?,?,?,?,?,'success','bypassPermissions')");
  const ledger = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, billing, outcome, spawned, basis, cost_status)
    values (?,?,?,'chat',?,?,'subscription','success',1,'per_run','unknown')`);
  const queue = db.prepare(`insert into run_queue (source, label, status, priority, requested_at, history_run_id, phase_timings_json) values ('chat','x','done',0,?,?,'{}')`);
  const step = db.prepare("insert into steps (session_key, turn_ord, seq, run_id, kind, name, started_at, status, checkpoint) values (?,?,?,?,'tool','Bash',?,?,?)");
  const checkpoint = (command, result, status) => JSON.stringify({ tool: 'Bash', input: JSON.stringify({ command }), result, status });
  turn.run('sc', 1, 'run-C1', D12 + '01:00:00Z', D12 + '03:00:00Z');
  ledger.run('k-C1', D12 + '03:00:00Z', D12 + '01:00:00Z', 'run-C1', 'codex');
  queue.run(D12 + '01:00:00Z', 'run-C1');
  for (let i = 1; i <= 100; i++) {
    let row = [`/bin/zsh -lc 'cat ${SECRET('FILE', 20)}'`, SECRET('OUT', 20), 'ok'];
    // An `rg` with no match: no output (Codex's aggregated_output is empty).
    if (i <= 3) row = [`/bin/zsh -lc 'cd /repo && rg -n ${SECRET('NEEDLE', 12)} src'`, '', 'error'];
    // An `rg` that failed with output, and six other failures (one piped, one redirected).
    else if (i === 4) row = [`/bin/zsh -lc 'rg -n x ${SECRET('MISSING', 12)}'`, `rg: ${SECRET('NOFILE', 20)}: No such file`, 'error'];
    else if (i === 5) row = [`/bin/zsh -lc 'cargo test 2>&1 | tail -3'`, `test result: FAILED ${SECRET('CARGO', 10)}`, 'error'];
    else if (i === 6) row = [`/bin/zsh -lc 'npm run gate > /tmp/${SECRET('LOG', 10)}'`, '', 'error'];
    // A canary as the first word (too long for a head) and a quoted JSON
    // blob piped to jq: both heads are `other`.
    else if (i === 7) row = [`/bin/zsh -lc '${SECRET('HEAD', 40)}.sh arg'`, `${SECRET('HEADOUT', 10)}`, 'error'];
    else if (i === 8) row = [`'{"token":"${SECRET('TOKEN', 20)}"}' | jq .token`, `jq: ${SECRET('JQ', 10)}`, 'error'];
    else if (i <= 10) row = [`/bin/zsh -lc 'python3 ${SECRET('SCRIPT', 10)}.py'`, `Traceback ${SECRET('TRACE', 10)}`, 'error'];
    step.run('sc', 1, i, 'run-C1', D12 + '01:00:00Z', row[2], checkpoint(...row));
  }
  for (let k = 1; k <= 10; k++) {
    const run = `run-D${k}`;
    turn.run('sd', k, run, `${D12}0${k - 1}:30:00Z`, `${D12}0${k - 1}:40:00Z`);
    ledger.run(`k-D${k}`, `${D12}0${k - 1}:40:00Z`, `${D12}0${k - 1}:30:00Z`, run, 'claude');
    queue.run(`${D12}0${k - 1}:30:00Z`, run);
    for (let i = 1; i <= 10; i++) {
      const failed = (k === 1 || k === 5) && i === 1;
      // A clean exit OF16 marked as a failed check (counted apart).
      const cp = failed ? checkpoint(`npm test ${SECRET('ARG', 10)}`, `Exit code 2\n${SECRET('ERR', 10)}`, 'error') : k === 2 && i === 2 ? JSON.stringify({ tool: 'Bash', checkFailed: true, status: 'ok' }) : null;
      step.run('sd', k, i, run, `${D12}0${k - 1}:31:00Z`, failed ? 'error' : 'ok', cp);
    }
  }
};
const of20 = { extra: of20Rows, from: Date.parse(D12 + '00:00:00Z'), to: Date.parse('2026-10-13T00:00:00Z') };

test('tool errors per 100 calls: 1 turn of 100 calls with 10 errors against 10 turns of 100 calls with 2', async () => {
  await withScorecard((s) => {
    const x = s.groups['chat/codex'].tools;
    const c = s.groups['chat/claude'].tools;
    // Per turn the Codex chat looks 50 times worse; per call, 5 times.
    assert.equal(x.errorsPerTurn, 10);
    assert.equal(c.errorsPerTurn, 0.2);
    assert.deepEqual(x, { turns: 1, turnsWithTools: 1, calls: 100, errors: 10, errorsPerTurn: 10, errorsPer100Calls: 10, expectedNonzero: 3, failures: 7, failuresPer100Calls: 7, checkFailed: 0, maskedFailures: null });
    assert.deepEqual(c, { turns: 10, turnsWithTools: 10, calls: 100, errors: 2, errorsPerTurn: 0.2, errorsPer100Calls: 2, expectedNonzero: 0, failures: 2, failuresPer100Calls: 2, checkFailed: 1, maskedFailures: null });
    // By agent and tool: the shell failures' pipe and redirect counts and
    // heads (`zsh -lc '…'` unwrapped, a leading `cd … &&` skipped).
    assert.deepEqual(s.toolErrors.byAgent.codex.all, { calls: 100, errors: 10, expectedNonzero: 3, failures: 7, checkFailed: 0, errorsPer100Calls: 10, failuresPer100Calls: 7 });
    assert.deepEqual(s.toolErrors.byAgent.codex.Bash, {
      calls: 100,
      errors: 10,
      expectedNonzero: 3,
      failures: 7,
      checkFailed: 0,
      failuresPiped: 2,
      failuresRedirected: 2,
      errorsPer100Calls: 10,
      failuresPer100Calls: 7,
      failureHeads: { other: 2, python3: 2, 'cargo test': 1, 'npm run': 1, rg: 1 },
    });
    assert.deepEqual(s.toolErrors.byAgent.claude.all, { calls: 100, errors: 2, expectedNonzero: 0, failures: 2, checkFailed: 1, errorsPer100Calls: 2, failuresPer100Calls: 2 });
    const md = renderSummary(s);
    assert.match(md, /\| Tool errors\/turn \| Tool errors\/100 calls \|$/m);
    assert.match(md, /\| chat\/codex \|.*\| 10 \| 10 \(10\/100\) \|$/m);
    assert.match(md, /- Tool errors per 100 calls: claude 2 \(2\/100; expected non-zero 0, failures 2 per 100\); codex 10 \(10\/100; expected non-zero 3, failures 7 per 100\)\./);
    assert.ok(!/secret/i.test(JSON.stringify(s) + md), 'a step input or result reached the report');
  }, [], of20);
});

test('a failed step keeps only its command head and booleans', async () => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-steps-${process.pid}-`));
  try {
    const { db } = await openDb(buildDb(dir, of20Rows));
    const rows = readDb(db, { since: of20.from, until: of20.to }).stepErrors.filter((r) => r.run_id === 'run-C1');
    db.close();
    assert.equal(rows.length, 10);
    for (const r of rows) assert.deepEqual(Object.keys(r), STEP_ERROR_FIELDS);
    assert.equal(rows.filter(expectedNonzero).length, 3);
    // Heads are lowercased: a case-sensitive check would miss a leaked one.
    assert.ok(!/secret/i.test(JSON.stringify(rows)), 'a step kept its input');
    assert.deepEqual(rows.filter((r) => r.head === 'other').length, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  // A command's shape: the head past `cd … &&` and a shell wrapper (a cut
  // input loses its closing quote), the last segment for the predicate.
  assert.deepEqual(commandShape("/bin/zsh -lc 'cd /r && rg foo src'"), { head: 'rg', pipe: false, redirect: false, predicate: true });
  assert.deepEqual(commandShape('rg foo src | head -3'), { head: 'rg', pipe: true, redirect: false, predicate: false });
  assert.deepEqual(commandShape('npm test > /tmp/x 2>&1 || true'), { head: 'npm test', pipe: false, redirect: true, predicate: false });
  assert.deepEqual(commandShape("echo 'a|b > c'"), { head: 'echo', pipe: false, redirect: false, predicate: false });
  // `[` and `[[` are heads though not name-shaped, so they stay predicates.
  assert.deepEqual(commandShape('[ -f x ]'), { head: '[', pipe: false, redirect: false, predicate: true });
  assert.deepEqual(commandShape('cd a && [ -d b ]'), { head: '[', pipe: false, redirect: false, predicate: true });
  assert.deepEqual(commandShape('ls x || [[ -e y ]]'), { head: 'ls', pipe: false, redirect: false, predicate: true });
  // Only a name-shaped first word is a head; a launcher keeps only a
  // name-shaped subcommand.
  assert.equal(commandShape(`'{"token":"sk-ant-0123"}' | jq .token`).head, 'other');
  assert.equal(commandShape(`${'A'.repeat(33)}.sh x`).head, 'other');
  assert.equal(commandShape('./scripts/gate.sh --fast').head, 'gate.sh');
  assert.equal(commandShape('npx {"x":1}').head, 'npx');
  assert.equal(shellCommandOf('{"command":"/bin/zsh -lc \'test -f a.md'), "/bin/zsh -lc 'test -f a.md");
  assert.equal(commandShape(shellCommandOf('{"command":"/bin/zsh -lc \'test -f a.md')).predicate, true);
  assert.equal(expectedNonzero({ noMatch: true, predicate: false, quiet: false }), true);
  assert.equal(expectedNonzero({ noMatch: false, predicate: true, quiet: false }), false);
  // runner-summaries.json's failure text, ANSI stripped; pass text is not.
  assert.equal(maskedRunner(' Tests  2 failed | 8 passed (10)'), 'vitest');
  assert.equal(maskedRunner('\u001b[31mtest result: FAILED\u001b[0m. 1 passed'), 'cargo-test');
  assert.equal(maskedRunner(' Test Files  4 passed (4)\n Tests  10 passed (10)'), null);
});

test('compare flags a per-100-call change outside its interval', async () => {
  await withScorecard((a) => {
    const b = structuredClone(a);
    Object.assign(b.groups['chat/codex'].tools, { errors: 2, errorsPer100Calls: 2 });
    b.toolErrors.byAgent.codex.all.errorsPer100Calls = 2;
    const c = compareScorecards(a, b);
    assert.deepEqual(c.groups['chat/codex'].toolErrorsPer100Calls.a, 10);
    assert.deepEqual(c.groups['chat/codex'].toolErrorsPer100Calls.b, 2);
    // 10/100 → 2/100: the Newcombe interval is below zero. One finished run: low n.
    assert.deepEqual(c.flags.map((f) => `${f.kind} ${f.group} ${f.change} ${f.lowN}`), ['tool-errors-per-100-calls-fall chat/codex 10 → 2 true']);
    assert.deepEqual(c.tools.errorsPer100CallsByAgent, { claude: { a: 2, b: 2 }, codex: { a: 10, b: 2 } });
    assert.match(renderComparison(c), /Tool errors per 100 calls claude 2 → 2; codex 10 → 2; masked failures — → —\./);
    // An older side without the key: worked out from its errors and calls.
    const old = structuredClone(a);
    delete old.groups['chat/codex'].tools.errorsPer100Calls;
    delete old.toolErrors;
    const o = compareScorecards(old, b);
    assert.equal(o.groups['chat/codex'].toolErrorsPer100Calls.a, 10);
    assert.deepEqual(o.tools.errorsPer100CallsByAgent.codex, { a: null, b: 2 });
  }, [], of20);
});

test('the refusal classifier is friction.rs error_class over a bounded prefix', () => {
  // The families are friction.rs's, word for word (comments stripped first).
  const src = rust('friction.rs');
  const from = src.indexOf('const FAMILIES');
  const body = src.slice(from, src.indexOf('];', from)).replace(/\/\/[^\n]*/g, '');
  const families = [...body.matchAll(/\(\s*"([\w-]+)",\s*&\[([\s\S]*?)\],?\s*\)/g)].map((m) => [m[1], [...m[2].matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((w) => w[1])]);
  assert.equal(families.length, 7);
  assert.deepEqual(ERROR_FAMILIES, families);
  assert.equal(errorClass('Permission to use Bash with command x has been denied.'), 'permission');
  assert.equal(errorClass('Compound commands require approval'), 'permission');
  assert.equal(errorClass('TASK_INVALID: no such task'), 'task_invalid');
  assert.equal(errorClass('the main thread was blocked on IO'), 'error');
  // Only the first RESULT_PREFIX characters are looked at.
  assert.equal(RESULT_PREFIX, 200);
  assert.equal(refusalClass(`${'x'.repeat(RESULT_PREFIX)} permission denied`), 'error');
  assert.equal(refusalClass([{ type: 'text', text: 'This command requires approval' }]), 'permission');
  assert.equal(refusalClass(null), 'error');
  // A result that starts with `Exit code N` came from a command that ran:
  // its own "Permission denied" is not a refusal (friction.rs exit_code).
  assert.equal(refusalClass('Exit code 128\ngit@github.com: Permission denied (publickey).'), 'exit-code');
  assert.equal(refusalClass([{ type: 'text', text: '\n  exit code 1 \nrm: x: Permission denied' }]), 'exit-code');
  assert.equal(refusalClass('Exit code: denied by hook'), 'permission');
  assert.equal(refusalClass('Error: Exit code 1 permission denied'), 'permission');
});

test('ALWAYS_LOAD matches app_mcp.rs', () => {
  const src = rust('app_mcp.rs');
  const m = /pub\(crate\) const ALWAYS_LOAD: \[&str; \d+\] = \[([\s\S]*?)\];/.exec(src);
  assert.ok(m, 'ALWAYS_LOAD not found in app_mcp.rs');
  assert.deepEqual(ALWAYS_LOAD, [...m[1].matchAll(/"(\w+)"/g)].map((x) => x[1]));
});

/** Every transcript line the guard sees may be read at these paths only
 *  (`[]` is any array index). */
const WHITELIST = new Set([
  'type', 'isSidechain', 'timestamp', 'uuid',
  'message', 'message.id', 'message.usage',
  'message.usage.input_tokens', 'message.usage.cache_creation_input_tokens', 'message.usage.cache_read_input_tokens', 'message.usage.output_tokens',
  // OF19: a write's 1-hour part, for its weight.
  'message.usage.cache_creation', 'message.usage.cache_creation.ephemeral_1h_input_tokens',
  // The parts array and, per part, its kind and a tool use's or result's ids.
  'message.content', 'message.content[]', 'message.content[].type',
  'message.content[].id', 'message.content[].name', 'message.content[].input',
  'message.content[].tool_use_id', 'message.content[].is_error',
]);
/** The input fields each tool may have read (one, or a list): Bash's command
 *  is read by the summary's tool pass too since OF19/OF20 (classified in
 *  memory as a poll, a Python edit script or a check run, and dropped); R8
 *  adds Read's numbers `offset` and `limit` and Grep's `pattern` (classified
 *  in memory as definition-shaped, and dropped). */
const INPUT_FIELD = { ToolSearch: 'query', Read: ['file_path', 'offset', 'limit'], Grep: 'pattern', Edit: 'file_path', Write: 'file_path', MultiEdit: 'file_path', Bash: 'command', mcp__hollow_app__run_action: 'id' };
const TOOL_PASS_FIELDS = INPUT_FIELD;
/** The fields before R8, to show the new reads are exactly R8's. */
const PRE_R8_FIELDS = { ...INPUT_FIELD, Read: 'file_path', Grep: [] };
/** The only text exception: a tool result's content, its parts' type and
 *  text, when it answers a Bash call (with is_error, the refusal class reads
 *  its first RESULT_PREFIX characters; a clean check run's is matched
 *  against runner-summaries.json, OF20) or a wait without an error (parsed
 *  for `finished` and `status`). A prompt (a string `message.content`) is
 *  never usable. */
const RESULT_TEXT = /^message\.content\[\]\.content(?:\[\](?:\.type|\.text)?)?$/;
/** What the anatomy (OF19, main-thread files) may also read, for byte sizes
 *  only: a prompt, a text or thinking part, any tool input or result,
 *  meta/compaction flags, a system line's subtype and an attachment's type
 *  and rendering. Nothing else: not `toolUseResult`, `cwd`, an attachment's
 *  own fields or a system line's content. */
const MEASURED = /^(?:message\.content(?:\[\]\.(?:text|thinking|input(?:\..+)?|content(?:\[\].*)?))?|isMeta|isCompactSummary|subtype|attachment|attachment\.type|rendered(?:[.[].*)?)$/;

/** tool_use id → { name, input }, from the raw fixture (the test may read it). */
function toolIndex(files) {
  const tools = new Map();
  for (const f of files) {
    for (const line of readFileSync(f, 'utf8').split('\n')) {
      let e;
      try {
        e = JSON.parse(line);
      } catch {
        continue;
      }
      if (e?.type !== 'assistant' || !Array.isArray(e.message?.content)) continue;
      for (const b of e.message.content) if (b?.type === 'tool_use') tools.set(b.id, { name: b.name, input: b.input });
    }
  }
  return tools;
}

/** A JSON.parse whose result records every field read into `violations`
 *  unless the whitelist allows it; a string it must not hand out comes back
 *  as a poison object that records any use. `fields` is the input field
 *  each tool may have read; `measured` adds the anatomy's MEASURED paths. */
function guardedParse(tools, violations, fields = INPUT_FIELD, { measured = false } = {}) {
  const isWait = (t) => t.name === 'mcp__hollow_app__wait_for_session' || (t.name === 'mcp__hollow_app__run_action' && t.input?.id === 'sessions.wait');
  const textOk = (part) => {
    const t = part?.type === 'tool_result' ? tools.get(part.tool_use_id) : null;
    return !!t && (t.name === 'Bash' || (isWait(t) && part.is_error !== true));
  };
  const allowed = (path, part) =>
    WHITELIST.has(path) || (part != null && Object.hasOwn(fields, part.name) && [fields[part.name]].flat().some((f) => path === `message.content[].input.${f}`)) || (RESULT_TEXT.test(path) && textOk(part)) || (measured && MEASURED.test(path));
  const poison = (path) =>
    new Proxy({}, {
      get(_, k) {
        violations.push(`${path} used (${String(k)})`);
        return undefined;
      },
      has() {
        violations.push(`${path} used (in)`);
        return false;
      },
      ownKeys() {
        violations.push(`${path} used (keys)`);
        return [];
      },
    });
  const wrap = (value, path, part) =>
    new Proxy(value, {
      get(target, key, receiver) {
        // Symbols, an array's length and its methods: no data.
        if (typeof key === 'symbol' || (Array.isArray(target) && (key === 'length' || !Object.hasOwn(target, key)))) return Reflect.get(target, key, receiver);
        const index = Array.isArray(target);
        const child = index ? `${path}[]` : path ? `${path}.${key}` : key;
        const v = Reflect.get(target, key, receiver);
        // A part of message.content is the context of every read below it.
        const owner = index && path === 'message.content' ? v : part;
        const ok = allowed(child, owner);
        if (!ok) violations.push(child);
        if (typeof v === 'string') return ok && (child !== 'message.content' || measured) ? v : poison(child);
        return v !== null && typeof v === 'object' ? wrap(v, child, owner) : v;
      },
    });
  return (line) => wrap(JSON.parse(line), '', null);
}

test('the tool pass reads only whitelisted fields: a read of any content or text fails', async () => {
  const childFile = join(transcripts, 'proj', 'ns-1', 'subagents', 'agent-x.jsonl');
  const files = [
    join(transcripts, 'proj', 'ns-1.jsonl'),
    join(transcripts, 'proj', 'ns-2.jsonl'),
    childFile,
    join(toolTranscripts, 'proj', 'ns-t1.jsonl'),
    join(toolTranscripts, 'proj', 'ns-t2.jsonl'),
  ];
  const tools = toolIndex(files);
  const violations = [];
  const parse = guardedParse(tools, violations);
  let uses = 0;
  // The summary's options, plus morning-report's toolUse (name and a Bash command).
  for (const f of files) {
    for (const c of await readCalls(f, f === childFile, { toolPass: true, toolUse: (name, command) => ({ name, bash: command != null }), parse })) uses += c.uses.length;
  }
  // ns-t1 and ns-t2's 19 main-thread uses (the sidechain line in ns-t1 is
  // skipped); the base fixture has none.
  assert.equal(uses, 19);
  assert.deepEqual(violations, []);

  // The summary's tool pass without morning-report's toolUse.
  const passOnly = [];
  const parsePassOnly = guardedParse(tools, passOnly, TOOL_PASS_FIELDS);
  let passUses = 0;
  for (const f of files) for (const c of await readCalls(f, f === childFile, { toolPass: true, parse: parsePassOnly })) passUses += c.uses.length;
  assert.equal(passUses, 19);
  assert.deepEqual(passOnly, []);

  // The guard itself: a text part, a prompt string and a Read result are caught.
  const caught = [];
  const p = guardedParse(tools, caught);
  const lines = readFileSync(join(toolTranscripts, 'proj', 'ns-t1.jsonl'), 'utf8').split('\n');
  void p(lines[4]).message.content[0].text; // a2's text part
  void p(lines[0]).message.content.length; // the first prompt
  void p(lines[7]).message.content[1].content; // tu4's Read result
  void p(lines[6]).message.content[0].input.description; // a Bash input other than its command
  assert.ok(caught.includes('message.content[].text'), caught.join('; '));
  assert.ok(caught.some((v) => v.startsWith('message.content used')), caught.join('; '));
  assert.ok(caught.includes('message.content[].content'), caught.join('; '));
  assert.ok(caught.includes('message.content[].input.description'), caught.join('; '));
  // Without the anatomy, a prompt's text stays poisoned.
  const prompt = guardedParse(tools, [], TOOL_PASS_FIELDS)(lines[0]).message.content;
  assert.notEqual(typeof prompt, 'string');
});

test('the anatomy reads sizes of content but nothing else: toolUseResult, cwd and attachment fields fail', async () => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-guard-${process.pid}-`));
  try {
    const root = anatomyTranscripts(dir);
    const files = [
      join(root, 'proj', 'ns-a1.jsonl'),
      join(root, 'proj', 'ns-a2.jsonl'),
      join(transcripts, 'proj', 'ns-1.jsonl'),
      join(toolTranscripts, 'proj', 'ns-t1.jsonl'),
      join(toolTranscripts, 'proj', 'ns-t2.jsonl'),
    ];
    const tools = toolIndex(files);
    const violations = [];
    const parse = guardedParse(tools, violations, TOOL_PASS_FIELDS, { measured: true });
    // The summary's options for a main-thread file.
    let carried = 0;
    for (const f of files) for (const c of await readCalls(f, false, { toolPass: true, anatomy: true, parse })) carried += Object.keys(c.carried).length;
    assert.ok(carried > 0);
    assert.deepEqual(violations, []);
    // The guard catches what the anatomy must not read.
    const caught = [];
    const p = guardedParse(tools, caught, TOOL_PASS_FIELDS, { measured: true });
    const lines = readFileSync(files[0], 'utf8').split('\n');
    void p(lines[7]).toolUseResult; // tu-a's result line
    void p(lines[1]).attachment.body;
    void p(lines[0]).cwd;
    assert.deepEqual(caught, ['toolUseResult', 'attachment.body', 'cwd']);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ── R8: definition lookups ───────────────────────────────────────────────
//
// Chat s10 on 10-10, native session ns-d1, two turns (every call carries
// 0 input, 0 write and the read below, so its context is that read):
//   L1 20:00–20:45 (45 min: a long run), calls of 90,000 context each:
//     e1 Grep `fn …` (a definition search)
//     e2 Read of a .rs file, whole, result 60,000 B (a hunt after e1)
//     e3 Grep `TODO …` (not definition-shaped) and a whole Read of a .md
//        file, 50,000 B (not source)
//     e4, e5 text
//   L2 21:00–21:10 (short), calls of 50,000 context each:
//     f1 Bash `cd … && rg -n 'struct\s+…' src` (a definition search)
//     f2 Read of a .rs file with offset and limit (windowed, after f1)
//     f3 Bash `cat <file>.rs`, result 50,000 B (whole, source, after f1)
//     f4 Read of a .ts file, an error (too large)
//     (compaction)
//     f5 Read of the same .ts file with a limit (a too-large retry; not
//        after a search: f1 is 4 uses back)
//     f6 text
// L1: e2's 60,000 B is carried by e3–e5: 15,000 × 3 = 45,000 tokens, of
// 450,000. L2: f3's 50,000 B by f4 only (the compaction drops it before
// f5): 12,500, of 300,000.

function lookupTranscripts(dir) {
  const root = join(dir, 'lookups');
  const proj = join(root, 'proj');
  mkdirSync(proj, { recursive: true });
  const call = (id, t, read, content) => JSON.stringify({ type: 'assistant', isSidechain: false, timestamp: A + t, uuid: `${id}-u`, message: { id, role: 'assistant', content, usage: { input_tokens: 0, cache_creation_input_tokens: 0, cache_read_input_tokens: read, output_tokens: 10 } } });
  const prompt = (t, content) => JSON.stringify({ type: 'user', timestamp: A + t, message: { role: 'user', content } });
  const result = (t, id, n, isError = false) => JSON.stringify({ type: 'user', timestamp: A + t, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: id, content: SECRET(`OUT-${id}`, n), ...(isError ? { is_error: true } : {}) }] } });
  const use = (id, name, input) => ({ type: 'tool_use', id, name, input });
  const text = { type: 'text', text: SECRET('TEXT', 20) };
  writeFileSync(join(proj, 'ns-d1.jsonl'), `${[
    prompt('20:00:01Z', SECRET('PROMPT', 100)),
    call('e1', '20:00:10Z', 90_000, [use('g-1', 'Grep', { pattern: `fn ${SECRET('PATTERN', 0)}`, path: `/repo/${SECRET('GREP-PATH', 0)}` })]),
    result('20:00:20Z', 'g-1', 300),
    call('e2', '20:01:00Z', 90_000, [use('r-1', 'Read', { file_path: `/repo/${SECRET('READ-PATH', 0)}.rs` })]),
    result('20:01:10Z', 'r-1', 60_000),
    call('e3', '20:02:00Z', 90_000, [use('g-2', 'Grep', { pattern: `TODO ${SECRET('PATTERN2', 0)}` }), use('r-2', 'Read', { file_path: `/repo/${SECRET('NOTES', 0)}.md` })]),
    result('20:02:10Z', 'g-2', 100),
    result('20:02:10Z', 'r-2', 50_000),
    call('e4', '20:10:00Z', 90_000, [text]),
    call('e5', '20:40:00Z', 90_000, [text]),
    prompt('21:00:01Z', SECRET('PROMPT2', 100)),
    call('f1', '21:00:10Z', 50_000, [use('b-1', 'Bash', { command: `cd /repo && rg -n 'struct\\s+${SECRET('RG', 0)}' src`, description: SECRET('DESC', 0) })]),
    result('21:00:20Z', 'b-1', 200),
    call('f2', '21:01:00Z', 50_000, [use('r-3', 'Read', { file_path: `/repo/${SECRET('WIN', 0)}.rs`, offset: 100, limit: 50 })]),
    result('21:01:10Z', 'r-3', 3000),
    call('f3', '21:02:00Z', 50_000, [use('b-2', 'Bash', { command: `cat /repo/src/${SECRET('CAT', 0)}.rs` })]),
    result('21:02:10Z', 'b-2', 50_000),
    call('f4', '21:03:00Z', 50_000, [use('r-4', 'Read', { file_path: `/repo/${SECRET('BIG', 0)}.ts` })]),
    result('21:03:10Z', 'r-4', 150, true),
    JSON.stringify({ type: 'system', subtype: 'compact_boundary', timestamp: A + '21:04:00Z' }),
    prompt('21:04:01Z', SECRET('SUMMARY', 200)),
    call('f5', '21:05:00Z', 50_000, [use('r-5', 'Read', { file_path: `/repo/${SECRET('BIG', 0)}.ts`, limit: 200 })]),
    result('21:05:10Z', 'r-5', 4000),
    call('f6', '21:06:00Z', 50_000, [text]),
  ].join('\n')}\n`);
  return root;
}

const lookupRows = (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('s10','claude','/v/workspaces/lane',?,?)`).run(A, A);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values (?,?,?,?,?,?,?)');
  const ledger = db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_usd, cost_status)
    values (?,?,?,'chat',?,'claude','ns-d1','subscription','success',1,'per_run',1,'estimated')`);
  for (const [ord, run, from, to] of [
    [1, 'run-L1', '20:00:00', '20:45:00'],
    [2, 'run-L2', '21:00:00', '21:10:00'],
  ]) {
    turn.run('s10', ord, run, `${A}${from}Z`, `${A}${to}Z`, 'success', 'bypassPermissions');
    ledger.run(`k-${run}`, `${A}${to}Z`, `${A}${from}Z`, run);
  }
};

async function withLookups(fn) {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-lookups-${process.pid}-`));
  try {
    const root = lookupTranscripts(dir);
    return await withScorecard((s) => fn(s, root), [root], { extra: lookupRows });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('R8: definition searches, whole and windowed reads are classified in memory', () => {
  for (const p of ['fn foo', 'pub fn\\s+foo', 'class\\s*Foo', 'struct\\s+\\w+', 'impl Foo', 'def run', 'export const X', 'interface  Props']) assert.ok(DEF_PATTERN.test(p), p);
  for (const p of ['TODO', 'fn(', 'functional', 'type=', 'const;', 'myfn foo']) assert.ok(!DEF_PATTERN.test(p), p);
  const none = { defSearch: false, read: null, source: false };
  assert.deepEqual(shellLookup("cd /r && rg -n 'fn\\s+run' src-tauri/src"), { ...none, defSearch: true });
  assert.deepEqual(shellLookup('grep -rn "class Foo" src | head -5'), { ...none, defSearch: true });
  assert.deepEqual(shellLookup('rg -n TODO src'), none);
  // A flag's value is not a pattern: `--type rust` is no `type` search.
  assert.deepEqual(shellLookup('rg --type rust -n foo src'), none);
  assert.deepEqual(shellLookup("rg -t ts -n 'interface Props' src"), { ...none, defSearch: true });
  assert.deepEqual(shellLookup('cat src/lib.rs'), { ...none, read: 'whole', source: true });
  assert.deepEqual(shellLookup('cd /r && cat -n README.md'), { ...none, read: 'whole' });
  // Piped on, or a heredoc, is no whole read.
  assert.deepEqual(shellLookup('cat src/lib.rs | head -50'), none);
  assert.deepEqual(shellLookup("cat > out.ts <<'EOF'\nx\nEOF"), none);
  assert.deepEqual(shellLookup("sed -n '100,180p' src/chat.rs"), { ...none, read: 'windowed', source: true });
  assert.deepEqual(shellLookup("sed -i '' 's/a/b/' src/chat.rs"), none);
  // A whole source read wins over a windowed one.
  assert.deepEqual(shellLookup("sed -n 1,5p a.md; cat b.ts"), { ...none, read: 'whole', source: true });
  assert.deepEqual(shellLookup(null), none);
});

test('R8: a Grep `fn` then a whole 60,000 B source read with 3 later calls is one hunt carrying 45,000 tokens', async () => {
  await withLookups((s) => {
    const d = s.transcripts.definitionLookups;
    assert.ok(d, 'no definitionLookups block');
    assert.deepEqual({ largeReadBytes: d.largeReadBytes, afterSearchUses: d.afterSearchUses, longRunMs: d.longRunMs }, { largeReadBytes: 40_000, afterSearchUses: 3, longRunMs: 30 * 60_000 });
    // L1 only: the long run.
    assert.deepEqual(d.longRuns, {
      turns: 1,
      contextTokens: 450_000,
      defSearches: 1,
      defSearchesPer100Turns: 100,
      wholeLargeSourceReads: { count: 1, bytes: 60_000, carriedTokens: 45_000, per100Turns: 100 },
      huntAfterSearch: { count: 1, per100Turns: 100 },
      windowedAfterSearch: { count: 0, per100Turns: 0 },
      tooLargeRetries: 0,
      shareOfContext: 0.1,
    });
    // L1 and L2: L2 adds the rg search, the windowed read after it, the
    // `cat` hunt carried by f4 alone (12,500) and the too-large retry.
    assert.deepEqual(d.all, {
      turns: 2,
      contextTokens: 750_000,
      defSearches: 2,
      defSearchesPer100Turns: 100,
      wholeLargeSourceReads: { count: 2, bytes: 110_000, carriedTokens: 57_500, per100Turns: 100 },
      huntAfterSearch: { count: 2, per100Turns: 100 },
      windowedAfterSearch: { count: 1, per100Turns: 50 },
      tooLargeRetries: 1,
      shareOfContext: 0.0767,
    });
    assert.match(renderSummary(s), /Definition lookups \(upper bound\) over 2 turns: 2 definition searches \(100 per 100 turns\); 2 whole large source reads/);
  });
});

test('R8: the tool pass reads only Read offset/limit and Grep pattern beyond its old fields, and no pattern or path reaches the report', async () => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-lookups-guard-${process.pid}-`));
  try {
    const root = lookupTranscripts(dir);
    const file = join(root, 'proj', 'ns-d1.jsonl');
    const tools = toolIndex([file]);
    const violations = [];
    const calls = await readCalls(file, false, { toolPass: true, parse: guardedParse(tools, violations, TOOL_PASS_FIELDS) });
    assert.deepEqual(violations, []);
    assert.deepEqual(calls.flatMap((c) => c.uses.map((u) => [u.name, u.defSearch, u.read, u.source])), [
      ['Grep', true, null, false],
      ['Read', false, 'whole', true],
      ['Grep', false, null, false],
      ['Read', false, 'whole', false],
      ['Bash', true, null, false],
      ['Read', false, 'windowed', true],
      ['Bash', false, 'whole', true],
      ['Read', false, 'whole', true],
      ['Read', false, 'windowed', true],
    ]);
    // Under the fields from before R8 the only extra reads are R8's, and
    // the poisoned pattern is never used (only its type is looked at).
    const before = [];
    const old = await readCalls(file, false, { toolPass: true, parse: guardedParse(tools, before, PRE_R8_FIELDS) });
    assert.deepEqual([...new Set(before)].sort(), ['message.content[].input.limit', 'message.content[].input.offset', 'message.content[].input.pattern']);
    assert.ok(!old.flatMap((c) => c.uses).some((u) => u.name === 'Grep' && u.defSearch));
    // No pattern, path, command or result string reaches the summary.
    await withScorecard((s) => {
      const out = JSON.stringify(s) + renderSummary(s);
      assert.ok(s.transcripts.definitionLookups.all.defSearches > 0);
      assert.ok(!out.includes('SECRET'), 'output holds a SECRET marker');
      assert.ok(!out.includes('/repo'), 'output holds a path');
    }, [root], { extra: lookupRows });
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ── SC4 · S08: implicit per-turn outcomes ────────────────────────────────
//
// Chat `so` on 10-10 beside the base fixture's six success turns (s1's five,
// s2's one; their steps fail Read once in run-A and Bash twice in run-F):
//   O1 02:00:00–02:01:00 cancelled, timings without firstTextMs (no text)
//   O2 `gap` s after O1 ended, asked by the person: a re-ask at 40 s, not at 130
//   O3 03:00:00–03:00:30 error, its chat ledger row `timeout`, first text at 0.8 s
//   O4 03:02:40, 130 s after O3 ended: no re-ask
//   O5 04:00:00–04:00:10 error, no timings anywhere
//   O6 04:00:30, asked by another agent: no re-ask

const D10 = '2026-10-10T';
/** O1 (`status`, cancelled by default) and O2 `gap` s after it ended, asked by `by`. */
const reaskRows = (gap, { by = null, status = 'cancelled' } = {}) => (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('so','claude','/v/workspaces/out',?,?)`).run(D10, D10);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode, asked_by) values (?,?,?,?,?,?,?,?)');
  const queue = db.prepare(`insert into run_queue (source, label, status, priority, requested_at, history_run_id, phase_timings_json) values ('chat','x','done',0,?,?,?)`);
  turn.run('so', 1, 'run-O1', D10 + '02:00:00Z', D10 + '02:01:00Z', status, 'bypassPermissions', null);
  queue.run(D10 + '02:00:00Z', 'run-O1', JSON.stringify({ queueWaitMs: 5, spawnMs: 100 }));
  const again = Date.parse(D10 + '02:01:00Z') + gap * 1000;
  turn.run('so', 2, 'run-O2', new Date(again).toISOString(), new Date(again + 60_000).toISOString(), 'success', 'bypassPermissions', by);
};
const outcomeRows = (db) => {
  reaskRows(40)(db);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode, asked_by) values (?,?,?,?,?,?,?,?)');
  const queue = db.prepare(`insert into run_queue (source, label, status, priority, requested_at, history_run_id, phase_timings_json) values ('chat','x','done',0,?,?,?)`);
  turn.run('so', 3, 'run-O3', D10 + '03:00:00Z', D10 + '03:00:30Z', 'error', 'bypassPermissions', null);
  ledgerRow(db, { key: 'ko3', started: D10 + '03:00:00Z', at: D10 + '03:00:30Z', run: 'run-O3', outcome: 'timeout' });
  queue.run(D10 + '03:00:00Z', 'run-O3', JSON.stringify({ spawnMs: 100, firstTextMs: 800 }));
  turn.run('so', 4, 'run-O4', D10 + '03:02:40Z', D10 + '03:03:00Z', 'success', 'bypassPermissions', null);
  turn.run('so', 5, 'run-O5', D10 + '04:00:00Z', D10 + '04:00:10Z', 'error', 'bypassPermissions', null);
  ledgerRow(db, { key: 'ko5', started: D10 + '04:00:00Z', at: D10 + '04:00:10Z', run: 'run-O5', outcome: 'error' });
  turn.run('so', 6, 'run-O6', D10 + '04:00:30Z', D10 + '04:01:00Z', 'success', 'bypassPermissions', 'agent');
};

test('S08: a cancel re-asked 40 s or exactly 120 s later is a re-ask, at 130 s, a negative gap or by a hollow: wake it is not, an interrupted turn is never stopped; stopped turns and tool failures per turn', async () => {
  assert.equal(REASK_MS, 120_000);
  await withScorecard((s) => {
    // 12 turns: the base six (success) and O1–O6, all finished.
    assert.deepEqual(s.outcomes, {
      turns: 12,
      finished: 12,
      byStatus: { cancelled: 1, error: 2, success: 9 },
      cancelled: { turns: 1, n: 12, share: 0.0833 },
      // O5; O3 is an error turn whose ledger row timed out.
      errored: { turns: 1, n: 12, share: 0.0833 },
      timedOut: { turns: 1, n: 12, share: 0.0833 },
      // O1 and O3 have timings, O1 with no first text; O5 has none.
      stoppedBeforeText: { turns: 1, n: 2, share: 0.5, unknownTiming: 1 },
      // O1 → O2 at 40 s; O3 → O4 at 130 s; O5 → O6 asked by an agent.
      reaskWithin120s: { turns: 1, n: 3, share: 0.3333 },
      // toolErrors' failures: Bash 2 (codex, run-F), Read 1 (claude, run-A), over 12 turns.
      toolFailuresPerTurn: { n: 12, all: { failures: 3, perTurn: 0.25 }, byTool: { Bash: { failures: 2, perTurn: 0.1667 }, Read: { failures: 1, perTurn: 0.0833 } } },
    });
    const md = renderSummary(s);
    assert.match(md, /- Turn outcomes \(proxies, never tuned on\) over 12 finished turns: cancelled 8\.3% \(1\/12\), errored 8\.3% \(1\/12\), timed out 8\.3% \(1\/12\); stopped before first text 50\.0% \(1\/2\) \(1 without timings\); re-asked within 120 s 33\.3% \(1\/3\); tool failures per turn 0\.25 \(Bash 0\.1667, Read 0\.0833\)\./);
  }, [], { extra: outcomeRows });
  // The same cancel with the person's next ask 130 s after it: redo 0.
  await withScorecard((s) => {
    assert.deepEqual(s.outcomes.cancelled, { turns: 1, n: 8, share: 0.125 });
    assert.deepEqual(s.outcomes.reaskWithin120s, { turns: 0, n: 1, share: 0 });
  }, [], { extra: reaskRows(130) });
  // 40 s with nothing else: redo 1, cancel 1.
  await withScorecard((s) => {
    assert.deepEqual(s.outcomes.cancelled, { turns: 1, n: 8, share: 0.125 });
    assert.deepEqual(s.outcomes.reaskWithin120s, { turns: 1, n: 1, share: 1 });
  }, [], { extra: reaskRows(40) });
  // Exactly 120 s counts (both bounds inclusive).
  await withScorecard((s) => {
    assert.deepEqual(s.outcomes.reaskWithin120s, { turns: 1, n: 1, share: 1 });
  }, [], { extra: reaskRows(120) });
  // The next turn started 10 s before the cancel ended: a negative gap, no re-ask.
  await withScorecard((s) => {
    assert.deepEqual(s.outcomes.reaskWithin120s, { turns: 0, n: 1, share: 0 });
  }, [], { extra: reaskRows(-10) });
  // Hollow's own wake 40 s later (`hollow:wake`): not the person, no re-ask.
  await withScorecard((s) => {
    assert.deepEqual(s.outcomes.reaskWithin120s, { turns: 0, n: 1, share: 0 });
  }, [], { extra: reaskRows(40, { by: 'hollow:wake' }) });
  // An interrupted turn (the restart reconciler's ended_at) is finished but
  // never stopped: no cancel, no stopped-before-text, no re-ask.
  await withScorecard((s) => {
    const none = { turns: 0, n: 8, share: 0 };
    assert.deepEqual({ ...s.outcomes, toolFailuresPerTurn: undefined }, {
      turns: 8,
      finished: 8,
      byStatus: { interrupted: 1, success: 7 },
      cancelled: none,
      errored: none,
      timedOut: none,
      stoppedBeforeText: { turns: 0, n: 0, share: null, unknownTiming: 0 },
      reaskWithin120s: { turns: 0, n: 0, share: null },
      toolFailuresPerTurn: undefined,
    });
  }, [], { extra: reaskRows(40, { status: 'interrupted' }) });
});

// ── SC4 · OF23: the Main agent's own turns ───────────────────────────────
//
// Main agent chat `sm` on 10-10: two wakes and a check-in Hollow asked
// (`hollow:wake`, `hollow:checkin`), and one turn the person asked.
//   W1 wake    $0.40   steps: read_session ×2, a failed Bash, a refused run_action   answer: text (SECRET)
//   W2 wake    $0.20   steps: read_session                                          answer: "  PULSE_OK\n"
//   K1 checkin unknown no steps                                                     answer: "PULSE_OK. …" (not exact)
//   P1 person  $5.00   steps: read_session ×3                                       answer: "PULSE_OK" (not Hollow's)
// W1 also has a $9 ask_agent row: a delegate, not the turn's own cost.

const REFUSED = rust('main_agent.rs').match(/const REFUSED: &str = "([^"]+)";/)[1];
const mainAgentRows = (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('sm','claude','/v/workspaces/main',?,?)`).run(D10, D10);
  const turn = db.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode, asked_by) values (?,?,?,?,?,?,?,?)');
  const step = db.prepare("insert into steps (session_key, turn_ord, seq, run_id, kind, name, started_at, status, checkpoint) values (?,?,?,?,'tool',?,?,?,?)");
  const stored = db.prepare('insert into transcripts (session_key, turn_ord, payload) values (?,?,?)');
  const rows = [
    [1, 'run-W1', '05', 'hollow:wake', 0.4, 'SECRET-WAKE-ANSWER: two chats need you'],
    [2, 'run-W2', '06', 'hollow:wake', 0.2, '  PULSE_OK\n'],
    [3, 'run-K1', '07', 'hollow:checkin', null, 'PULSE_OK. SECRET-CHECKIN-TAIL'],
    [4, 'run-P1', '08', null, 5, 'PULSE_OK'],
  ];
  for (const [ord, run, hh, by, cost, text] of rows) {
    turn.run('sm', ord, run, `${D10}${hh}:00:00Z`, `${D10}${hh}:01:00Z`, 'success', 'acceptEdits', by);
    ledgerRow(db, { key: `k-${run}`, started: `${D10}${hh}:00:00Z`, at: `${D10}${hh}:01:00Z`, run, cost, status: cost == null ? 'unknown' : 'estimated' });
    stored.run('sm', ord, JSON.stringify({ agent: 'claude', text, tools: [], incomplete: false }));
  }
  ledgerRow(db, { key: 'k-W1-ask', started: D10 + '05:00:10Z', at: D10 + '05:00:20Z', surface: 'delegate', parent: 'chat', run: 'run-W1', cost: 9, status: 'estimated' });
  const cp = (result) => JSON.stringify({ tool: 'x', input: '{}', result, status: 'error' });
  let seq = 0;
  for (const [ord, run, name, status, checkpoint] of [
    [1, 'run-W1', 'mcp__hollow_app__read_session', 'ok', null],
    [1, 'run-W1', 'mcp__hollow_app__read_session', 'ok', null],
    [1, 'run-W1', 'Bash', 'error', cp(`Exit code 1\n${SECRET('BASH', 30)}`)],
    [1, 'run-W1', 'mcp__hollow_app__run_action', 'error', cp(REFUSED)],
    [2, 'run-W2', 'mcp__hollow_app__read_session', 'ok', null],
    [4, 'run-P1', 'mcp__hollow_app__read_session', 'ok', null],
    [4, 'run-P1', 'mcp__hollow_app__read_session', 'ok', null],
    [4, 'run-P1', 'mcp__hollow_app__read_session', 'ok', null],
  ]) step.run('sm', ord, ++seq, run, name, `${D10}0${4 + ord}:00:30Z`, status, checkpoint);
};

test('OF23: two tagged wakes and a check-in give the hand-computed mainAgent block', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-main-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  // Suggestions answered in the 30 days before the window's end (10-11):
  // done 10-05, never 10-09, not_now 10-10; 09-01 is too old, 10-11 01:00 after the end.
  const suggestions = join(dir, 'suggestions.json');
  writeFileSync(suggestions, JSON.stringify({ outcomes: [
    { at: '2026-09-01T10:00:00Z', suggestion: SECRET('SUGGESTION', 30), outcome: 'done' },
    { at: '2026-10-05T10:00:00+00:00', suggestion: SECRET('SUGGESTION', 31), outcome: 'done' },
    { at: '2026-10-09T10:00:00Z', suggestion: SECRET('SUGGESTION', 32), outcome: 'never' },
    { at: '2026-10-10T12:00:00Z', suggestion: SECRET('SUGGESTION', 33), outcome: 'not_now' },
    { at: '2026-10-11T01:00:00Z', suggestion: SECRET('SUGGESTION', 34), outcome: 'done' },
  ] }));
  await withScorecard((s) => {
    const wake = {
      turns: 2,
      // 0.40 and 0.20: nearest-rank p50 is the 1st of 2; the $9 delegate row is not the turn's.
      cost: { knownTurns: 2, unknownTurns: 0, p50Usd: 0.2, sumUsd: 0.6 },
      toolCalls: 5, // W1 4, W2 1
      toolsPerTurn: 2.5,
      readSessionCalls: 3,
      readSessionPerTurn: 1.5,
      turnsWithReadSession: 2,
      // W1's run_action refusal; its failed Bash is not one.
      refusals: 1,
      turnsWithRefusal: 1,
      // W2's answer trims to PULSE_OK; W1's is text.
      pulseOk: { ok: 1, n: 2, share: 0.5 },
    };
    const checkin = {
      turns: 1,
      // No turn's cost is known: the sum is unknown (null), never $0.
      cost: { knownTurns: 0, unknownTurns: 1, p50Usd: null, sumUsd: null },
      toolCalls: 0,
      toolsPerTurn: 0,
      readSessionCalls: 0,
      readSessionPerTurn: 0,
      turnsWithReadSession: 0,
      refusals: 0,
      turnsWithRefusal: 0,
      // "PULSE_OK. …" is not exactly PULSE_OK.
      pulseOk: { ok: 0, n: 1, share: 0 },
    };
    assert.deepEqual(s.mainAgent, {
      status: 'tagged',
      turns: 3,
      all: {
        turns: 3,
        cost: { knownTurns: 2, unknownTurns: 1, p50Usd: 0.2, sumUsd: 0.6 },
        toolCalls: 5,
        toolsPerTurn: 1.67,
        readSessionCalls: 3,
        readSessionPerTurn: 1,
        turnsWithReadSession: 2,
        refusals: 1,
        turnsWithRefusal: 1,
        pulseOk: { ok: 1, n: 3, share: 0.3333 },
      },
      byKind: { checkin, wake },
      suggestions: { days: 30, answered: 3, done: 1, not_now: 1, never: 1, other: 0, takenShare: 0.3333 },
    });
    const md = renderSummary(s);
    assert.match(md, /- Main agent \(hollow: turns\): checkin 1 turns, cost p50 — \/ sum unknown \(1 without a cost\), tools\/turn 0, read_session\/turn 0, refusals 0, PULSE_OK 0\.0% \(0\/1\); wake 2 turns, cost p50 \$0\.20 \/ sum \$0\.60, tools\/turn 2\.5, read_session\/turn 1\.5, refusals 1, PULSE_OK 50\.0% \(1\/2\); suggestions over 30 days: done 1, not now 1, never 1 \(taken 33\.3% \(1\/3\)\)\./);
    assert.ok(!/SECRET/.test(JSON.stringify(s) + md), 'an answer, suggestion or step result reached the report');
    // Some turns of unknown cost: the known sum renders as a floor.
    const partial = structuredClone(s);
    partial.mainAgent.byKind = { all: s.mainAgent.all };
    assert.match(renderSummary(partial), /all 3 turns, cost p50 \$0\.20 \/ sum ≥ \$0\.60 \(1 without a cost\),/);
  }, [], { extra: mainAgentRows, suggestions });
  // No turn carries a hollow: tag (the live store before OB20): untagged, zeroed.
  await withScorecard((s) => {
    assert.deepEqual(s.mainAgent, {
      status: 'untagged',
      turns: 0,
      all: { turns: 0, cost: { knownTurns: 0, unknownTurns: 0, p50Usd: null, sumUsd: null }, toolCalls: 0, toolsPerTurn: null, readSessionCalls: 0, readSessionPerTurn: null, turnsWithReadSession: 0, refusals: 0, turnsWithRefusal: 0, pulseOk: { ok: 0, n: 0, share: null } },
      byKind: {},
      suggestions: null,
    });
    assert.match(renderSummary(s), /- Main agent \(hollow: turns\): untagged \(no turn carries a hollow: tag\)\./);
  }, []);
  // The log's place beside a live store, and none beside a test database.
  assert.equal(suggestionsFileFor('/s/Hollow/Sessions/vault-0123.db'), '/s/Hollow/MainAgent/vault-0123-suggestions.json');
  assert.equal(suggestionsFileFor('/tmp/x/sessions.db'), null);
});

test('OF23: readDb names a stored payload only inside PULSE_OK_SQL, never turns.asked, and refusals match main_agent.rs', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-sql-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const { db } = await openDb(buildDb(dir, mainAgentRows));
  const prepare = DatabaseSync.prototype.prepare;
  const seen = [];
  let rows;
  DatabaseSync.prototype.prepare = function (sql) {
    seen.push(sql);
    return prepare.call(this, sql);
  };
  try {
    rows = readDb(db, { since, until });
  } finally {
    DatabaseSync.prototype.prepare = prepare;
    db.close();
  }
  const payload = seen.filter((sql) => /payload/i.test(sql));
  assert.equal(payload.length, 1);
  assert.ok(payload[0].includes(PULSE_OK_SQL));
  assert.ok(!/payload/i.test(payload[0].split(PULSE_OK_SQL).join(' ')), 'a payload read outside the boolean');
  assert.ok(!seen.some((sql) => /\basked\b/i.test(sql)), 'turns.asked was named');
  // One 0/1 per tagged turn with a stored answer; P1 (the person's) is not read.
  assert.deepEqual(rows.pulse.map((r) => [r.session_key, r.ord, r.pulse_ok]).sort((a, b) => a[1] - b[1]), [['sm', 1, 0], ['sm', 2, 1], ['sm', 3, 0]]);
  assert.deepEqual(rows.refusals.map((r) => [r.run_id, r.n]), [['run-W1', 1]]);
  assert.ok(!/SECRET/.test(JSON.stringify(rows)), 'readDb kept a text');
  // Every refusal main_agent.rs `unattended::allows` returns that names an
  // unattended turn matches REFUSAL_LIKE (LIKE: % any run, ASCII case-insensitive).
  const src = rust('main_agent.rs');
  const body = src.slice(src.indexOf('const REFUSED: &str'), src.indexOf('/// One thing the Main agent did on its own'));
  const texts = [...body.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => m[1]).filter((x) => /unattended/i.test(x));
  assert.ok(texts.length >= 9, texts.join('\n'));
  // The patterns hold only letters, spaces and `%`.
  assert.ok(REFUSAL_LIKE.every((p) => /^[%A-Za-z ]+$/.test(p)));
  const like = REFUSAL_LIKE.map((p) => new RegExp('^' + p.split('%').join('.*') + '$', 'is'));
  for (const x of texts) assert.ok(like.some((re) => re.test(x)), x);
  assert.ok(!like.some((re) => re.test('Exit code 1\nNo such file')));
});

// ── SC4 · S13: native memory writes against learnings accepted ───────────
//
// Built in a temp folder: `<tmp>/.claude/projects` holds the worktree
// session's transcript (`-Users-x-wt/ns-m1.jsonl`) and the repo root's
// memory folder (`-Users-x-repo/memory/`, where Claude Code keys a
// worktree's memory). Chat run-M1 (06:00–06:10, workspace ws-m):
//   tu-m1 Write into the memory folder, ok          → a native write
//   tu-m2 Edit of /repo/src/a.ts, ok                → elsewhere
//   tu-m3 MultiEdit of memory/MEMORY.md, is_error   → a failed write
//   tu-m4 Write into memory/../../escape.md, ok     → `..`: not inside
// The vault's receipts: run-M1 with memoryWrites 1, one in the window
// without the field, one before it with 5. Its learnings: one accepted in
// the window, one before it, one undated, a non-learning note and one each
// in `restricted/` and `team/restricted/` (both skipped).

function memoryFixture(dir) {
  const root = join(dir, '.claude', 'projects');
  const proj = join(root, '-Users-x-wt');
  const memory = join(root, '-Users-x-repo', 'memory');
  mkdirSync(proj, { recursive: true });
  const usage = { input_tokens: 1, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 1 };
  const call = (id, time, part) => JSON.stringify({ type: 'assistant', isSidechain: false, timestamp: D10 + time, uuid: `${id}-u`, message: { id, role: 'assistant', content: [part], usage } });
  const result = (time, id, isError = false) => JSON.stringify({ type: 'user', timestamp: D10 + time, message: { role: 'user', content: [{ type: 'tool_result', tool_use_id: id, content: SECRET('RESULT', 30), ...(isError ? { is_error: true } : {}) }] } });
  const file = join(proj, 'ns-m1.jsonl');
  writeFileSync(file, `${[
    JSON.stringify({ type: 'user', timestamp: D10 + '06:00:01Z', message: { role: 'user', content: SECRET('PROMPT', 40) } }),
    call('m1', '06:00:10Z', { type: 'tool_use', id: 'tu-m1', name: 'Write', input: { file_path: join(memory, 'feedback.md'), content: SECRET('NOTE', 60) } }),
    result('06:00:11Z', 'tu-m1'),
    call('m2', '06:01:00Z', { type: 'tool_use', id: 'tu-m2', name: 'Edit', input: { file_path: '/repo/src/a.ts', old_string: SECRET('OLD', 20), new_string: SECRET('NEW', 20) } }),
    result('06:01:01Z', 'tu-m2'),
    call('m3', '06:02:00Z', { type: 'tool_use', id: 'tu-m3', name: 'MultiEdit', input: { file_path: join(memory, 'MEMORY.md'), edits: [{ old_string: SECRET('OLD', 21), new_string: SECRET('NEW', 21) }] } }),
    result('06:02:01Z', 'tu-m3', true),
    call('m4', '06:03:00Z', { type: 'tool_use', id: 'tu-m4', name: 'Write', input: { file_path: `${memory}/../../escape.md`, content: SECRET('NOTE', 61) } }),
    result('06:03:01Z', 'tu-m4'),
  ].join('\n')}\n`);
  const v = join(dir, 'vault');
  mkdirSync(join(v, 'learnings', 'restricted'), { recursive: true });
  mkdirSync(join(v, 'learnings', 'team', 'restricted'), { recursive: true });
  writeFileSync(join(v, '.evidence-receipts.json'), JSON.stringify({ schemaVersion: 1, receipts: [
    { runId: 'run-M1', source: 'chat', startedAt: D10 + '06:00:00Z', status: 'success', memoryWrites: 1 },
    { runId: 'run-X9', source: 'chat', startedAt: D10 + '09:00:00Z', status: 'success' },
    { runId: 'run-old', source: 'chat', startedAt: '2026-10-01T00:00:00Z', status: 'success', memoryWrites: 5 },
  ] }));
  const note = (rel, fm) => writeFileSync(join(v, 'learnings', rel), `---\n${fm.join('\n')}\n---\n\n${SECRET('LEARNING-BODY', 40)}\n`);
  note('use-pnpm-1a2b3c4d.md', ['type: learned-context', `title: ${SECRET('TITLE', 20)}`, 'trust: approved', 'learned_at: "2026-10-10T12:00:00+00:00"']);
  note('old-5e6f7a8b.md', ['type: learned-context', 'trust: approved', 'learned_at: 2026-10-01T00:00:00Z']);
  note('undated-9c0d1e2f.md', ['type: learned-context', 'trust: approved']);
  note('draft.md', ['type: note', 'learned_at: 2026-10-10T12:00:00Z']);
  note('restricted/hidden.md', ['type: learned-context', 'trust: approved', 'learned_at: 2026-10-10T12:00:00Z']);
  // A `restricted/` folder deeper down is skipped too.
  note('team/restricted/hidden.md', ['type: learned-context', 'trust: approved', 'learned_at: 2026-10-10T12:00:00Z']);
  return { root, file, vault: v };
}

const memoryRows = (db) => {
  db.prepare(`insert into sessions (key, agent, cwd, created_at, last_activity_at) values ('sx','claude','/v/workspaces/wt',?,?)`).run(D10, D10);
  db.prepare("insert into turns (session_key, ord, run_id, started_at, ended_at, status, permission_mode) values ('sx',1,'run-M1',?,?,'success','bypassPermissions')").run(D10 + '06:00:00Z', D10 + '06:10:00Z');
  db.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_status, workspace_id)
    values ('k-M1',?,?,'chat','run-M1','claude','ns-m1','subscription','success',1,'per_run','unknown','ws-m')`).run(D10 + '06:10:00Z', D10 + '06:00:00Z');
};

test('S13: one Write into a memory folder and one Edit elsewhere give nativeWrites 1, against learnings accepted', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-memory-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const fx = memoryFixture(dir);
  // The tool pass reads a MultiEdit's file_path now, and still nothing else.
  const tools = toolIndex([fx.file]);
  const violations = [];
  const calls = await readCalls(fx.file, false, { toolPass: true, memoryRoots: [fx.root], parse: guardedParse(tools, violations, TOOL_PASS_FIELDS) });
  assert.deepEqual(violations, []);
  assert.deepEqual(calls.flatMap((c) => c.uses.map((u) => [u.name, u.memoryWrite, u.error])), [['Write', true, false], ['Edit', false, false], ['MultiEdit', true, true], ['Write', false, false]]);
  const caught = [];
  const p = guardedParse(tools, caught, TOOL_PASS_FIELDS);
  const lines = readFileSync(fx.file, 'utf8').split('\n');
  void p(lines[5]).message.content[0].input.edits; // tu-m3's edits
  void p(lines[1]).message.content[0].input.content; // tu-m1's note
  assert.deepEqual(caught, ['message.content[].input.edits', 'message.content[].input.content']);
  // The folder rule: `<root>/<slug>/memory/<file>`, absolute, no `..`.
  const r = '/h/.claude/projects';
  assert.equal(isMemoryPath(`${r}/-a/memory/x.md`, [r]), true);
  assert.equal(isMemoryPath(`${r}/-a/memory/sub/x.md`, [`${r}/`]), true);
  assert.equal(isMemoryPath(`${r}/-a/memory.md`, [r]), false);
  assert.equal(isMemoryPath(`${r}/-a/notes/memory/x.md`, [r]), false);
  assert.equal(isMemoryPath(`${r}/-a/memory/../../x.md`, [r]), false);
  assert.equal(isMemoryPath('-a/memory/x.md', [r]), false);
  assert.equal(isMemoryPath('/h/Documents/projects/app/memory/x.md', [r]), false);
  assert.equal(isMemoryPath(`${r}/-a/memory/x.md`, []), false);

  await withScorecard((s) => {
    assert.deepEqual(s.memory, {
      nativeWrites: 1, // tu-m1; tu-m3 failed, tu-m4 climbs out, tu-m2 is elsewhere
      nativeWriteErrors: 1,
      nativeWriteRuns: 1,
      nativeWritesByWorkspace: { 'ws-m': 1 },
      // run-M1's 1; run-X9 lacks the field; run-old is before the window.
      receiptMemoryWrites: 1,
      receiptsWithMemoryWrites: 1,
      receiptsWithoutMemoryWrites: 1,
      learningsAccepted: 1,
      // Neither restricted note counts, at the top or a level down.
      learnings: { accepted: 1, approved: 3, undated: 1 },
      nativeWritesPerLearning: 1,
    });
    const md = renderSummary(s);
    assert.match(md, /- Native memory writes 1 in 1 run \(receipts 1 over 1 that carry it\) against learnings accepted 1\./);
    const out = JSON.stringify(s) + md;
    assert.ok(!/SECRET|\/memory\/|-Users-x-/.test(out), 'a path or text reached the report');
  }, [fx.root], { extra: memoryRows, vault: fx.vault });
  // No transcripts, and receipts without the field: null, not 0.
  await withScorecard((s) => {
    assert.equal(s.memory.nativeWrites, null);
    assert.equal(s.memory.receiptMemoryWrites, null);
    assert.equal(s.memory.receiptsWithMemoryWrites, 0);
    assert.equal(s.memory.learningsAccepted, null);
    assert.equal(s.memory.nativeWritesPerLearning, null);
  }, []);
});

// ── S27: pass claims in final replies against the checks seen ───────────
//
// A temp vault whose receipts join the base ledger's runs: run-A claimed a
// pass after a failed check, run-B one with no check after its last write,
// run-C's receipt predates S27 (no block), run-D made no claim, and run-F
// (codex) claimed a backed pass.

test('S27: pass claims per group and over the window, null and "not installed" when no receipt carries one', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-claims-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const v = join(dir, 'vault');
  mkdirSync(v, { recursive: true });
  const receipt = (run, hour, verification) => ({ runId: run, source: 'chat', startedAt: `${D10}${hour}:00:00Z`, status: 'success', ...(verification ? { verification } : {}) });
  writeFileSync(join(v, '.evidence-receipts.json'), JSON.stringify({ schemaVersion: 1, receipts: [
    receipt('run-A', '10', { claim: 'passed', claimAfterFail: true }),
    receipt('run-B', '11', { claim: 'passed', claimWithoutCheck: true }),
    receipt('run-C', '12', null),
    receipt('run-D', '13', { claim: 'none' }),
    receipt('run-F', '15', { claim: 'passed' }),
  ] }));
  await withScorecard((s) => {
    // chat/claude: run-A, B and D carry a claim (run-C doesn't): 2 of 3.
    assert.deepEqual(s.groups['chat/claude'].verification.claims, { n: 3, claims: 2, claimWithoutCheck: 1, claimAfterFail: 1, per100Runs: { claims: 66.67, claimWithoutCheck: 33.33, claimAfterFail: 33.33 } });
    assert.deepEqual(s.groups['chat/codex'].verification.claims, { n: 1, claims: 1, claimWithoutCheck: 0, claimAfterFail: 0, per100Runs: { claims: 100, claimWithoutCheck: 0, claimAfterFail: 0 } });
    // The title oneshot sharing run-A's id gets none of its receipt: null, not 0.
    assert.equal(s.groups['oneshot/claude'].verification.claims, null);
    // Over the window: the four receipts that carry a claim.
    assert.deepEqual(s.verification.claims, { n: 4, claims: 3, claimWithoutCheck: 1, claimAfterFail: 1, per100Runs: { claims: 75, claimWithoutCheck: 25, claimAfterFail: 25 } });
    assert.match(renderSummary(s), /- Pass claims in final replies: 3 \(75 per 100\) over 4 runs; without a check after the last write 1 \(25 per 100\); after a failed check 1 \(25 per 100\) \(chat\/claude 2 of 3, without a check 1, after a failed check 1; chat\/codex 1 of 1, without a check 0, after a failed check 0\)\./);
    // compare: both sides, then a scorecard from before S27 (—), never a flag.
    const b = structuredClone(s);
    b.verification.claims = { ...s.verification.claims, n: 10, per100Runs: { claims: 50, claimWithoutCheck: 10, claimAfterFail: 0 } };
    const c = compareScorecards(s, b);
    assert.deepEqual(c.verification, { n: { a: 4, b: 10 }, claimsPer100Runs: { a: 75, b: 50 }, claimWithoutCheckPer100Runs: { a: 25, b: 10 }, claimAfterFailPer100Runs: { a: 25, b: 0 } });
    assert.deepEqual(c.groups['chat/claude'].claims.claimAfterFailPer100Runs, { a: 33.33, b: 33.33 });
    assert.ok(!c.flags.some((f) => /claim/i.test(f.kind)));
    assert.match(renderComparison(c), /Pass claims per 100 runs 75 → 50 \(n 4 → 10\); without a check after the last write 25 → 10; after a failed check 25 → 0\./);
    const old = structuredClone(s);
    delete old.verification;
    for (const g of Object.values(old.groups)) delete g.verification;
    const o = compareScorecards(old, s);
    assert.deepEqual(o.verification.n, { a: null, b: 4 });
    assert.deepEqual(o.groups['chat/claude'].claims.claimsPer100Runs, { a: null, b: 66.67 });
    assert.match(renderComparison(o), /Pass claims per 100 runs — → 75 \(n — → 4\)/);
    assert.doesNotMatch(renderComparison(compareScorecards(old, old)), /Pass claims/);
  }, [], { vault: v });
  // The fixture's receipts predate S27.
  await withScorecard((s) => {
    assert.equal(s.verification.claims, null);
    assert.equal(s.groups['chat/claude'].verification.claims, null);
    assert.match(renderSummary(s), /- Pass claims in final replies: not installed \(no receipt carries verification\.claim\)\./);
  }, []);
});

// S27 review (red first): a turn that failed over has a ledger row per
// attempt, both joined to its one receipt; its claim counts once.
test('S27: a failed-over turn counts its receipt\'s pass claim once per group', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), `scorecard-claims-once-${process.pid}-`));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const v = join(dir, 'vault');
  mkdirSync(v, { recursive: true });
  const receipt = (run, hour, verification) => ({ runId: run, source: 'chat', startedAt: `${D10}${hour}:00:00Z`, status: 'success', verification });
  writeFileSync(join(v, '.evidence-receipts.json'), JSON.stringify({ schemaVersion: 1, receipts: [
    receipt('run-A', '10', { claim: 'passed', claimAfterFail: true }),
    receipt('run-B', '11', { claim: 'passed', claimWithoutCheck: true }),
    receipt('run-D', '13', { claim: 'none' }),
  ] }));
  // run-A's first attempt, on the account it ran out on (R1).
  const failedOver = (db) => ledgerRow(db, { key: 'k30', started: `${D10}09:59:00Z`, at: `${D10}10:00:00Z`, run: 'run-A', outcome: 'error' });
  await withScorecard((s) => {
    assert.deepEqual(s.groups['chat/claude'].verification.claims, { n: 3, claims: 2, claimWithoutCheck: 1, claimAfterFail: 1, per100Runs: { claims: 66.67, claimWithoutCheck: 33.33, claimAfterFail: 33.33 } });
    assert.equal(s.verification.claims.n, 3);
  }, [], { vault: v, extra: failedOver });
});

test('SC4: compare prints re-ask, stopped-before-text, wake read_session and native writes, and — for an older side', async () => {
  await withScorecard((a) => {
    const b = structuredClone(a);
    b.outcomes.reaskWithin120s = { turns: 0, n: 3, share: 0 };
    b.mainAgent = { status: 'tagged', turns: 2, all: {}, byKind: { wake: { turns: 2, readSessionPerTurn: 1.5 } }, suggestions: null };
    b.memory.nativeWrites = 4;
    const c = compareScorecards(a, b);
    assert.deepEqual(c.outcomes.reaskShare, { a: 0.3333, b: 0, n: { a: 3, b: 3 } });
    assert.deepEqual(c.outcomes.stoppedBeforeTextShare, { a: 0.5, b: 0.5, n: { a: 2, b: 2 }, unknownTiming: { a: 1, b: 1 } });
    assert.deepEqual(c.mainAgent, { status: { a: 'untagged', b: 'tagged' }, wakeReadSessionPerTurn: { a: null, b: 1.5, n: { a: null, b: 2 } } });
    assert.deepEqual(c.memory, { nativeWrites: { a: null, b: 4 }, learningsAccepted: { a: null, b: null } });
    // Proxies: never a flag.
    assert.deepEqual(c.flags, []);
    const md = renderComparison(c);
    assert.match(md, /Re-asked within 120 s 33\.3% \(n 3\) → 0\.0% \(n 3\); stopped before first text 50\.0% \(n 2\) → 50\.0% \(n 2\), without timings 1 → 1 \(proxies\)\./);
    assert.match(md, /Main agent wakes' read_session per turn — \(n —\) → 1\.5 \(n 2\) \(tags untagged → tagged\)\./);
    assert.match(md, /Native memory writes — → 4; learnings accepted — → —\./);
    // A scorecard from before SC4: its side is —, and with neither side, no lines.
    const old = structuredClone(a);
    delete old.outcomes;
    delete old.mainAgent;
    delete old.memory;
    const o = compareScorecards(old, b);
    assert.deepEqual(o.outcomes.reaskShare, { a: null, b: 0, n: { a: null, b: 3 } });
    assert.match(renderComparison(o), /Re-asked within 120 s — \(n —\) → 0\.0% \(n 3\)/);
    assert.doesNotMatch(renderComparison(compareScorecards(old, old)), /Re-asked|Main agent wakes|Native memory writes/);
  }, [], { extra: outcomeRows });
});

test('the CLI writes JSON and refuses bad arguments', async (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'scorecard-cli-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const dbFile = buildDb(dir);
  const log = console.log;
  const err = console.error;
  console.log = () => {};
  console.error = () => {};
  try {
    const out = join(dir, 'a.json');
    assert.equal(await main(['summary', '--vault', vault, '--db', dbFile, '--transcripts', transcripts, '--since', '2026-10-10T00:00:00Z', '--until', '2026-10-11T00:00:00Z', '--json', out]), 0);
    const s = JSON.parse(readFileSync(out, 'utf8'));
    assert.equal(s.kind, 'hollow-scorecard');
    assert.equal(s.groups['chat/claude'].runs, 7);
    writeFileSync(join(dir, 'b.json'), JSON.stringify(s));
    assert.equal(await main(['compare', out, join(dir, 'b.json'), '--json', join(dir, 'c.json')]), 0);
    assert.equal(JSON.parse(readFileSync(join(dir, 'c.json'), 'utf8')).flags.length, 0);
    assert.equal(await main(['summary']), 2);
    assert.equal(await main(['summary', '--vault', vault, '--since', '2026-10-11', '--until', '2026-10-10']), 2);
    assert.equal(await main(['compare', out]), 2);
    writeFileSync(join(dir, 'bad.json'), '{not json');
    assert.equal(await main(['compare', out, join(dir, 'bad.json')]), 2);
    assert.equal(await main(['nope']), 2);
    assert.throws(() => parseArgs(['summary', '--vault']), /needs a value/);
    assert.equal(parseArgs(['summary', '--overlap']).overlap, true);
    assert.equal(parseArgs(['summary']).overlap, false);
  } finally {
    console.log = log;
    console.error = err;
  }
});
