// A chat's own live context size (overnight 2026-10-16/17, OF27). A lane
// runs it after every merge and before every claim, and hands over once its
// main thread carries 350K tokens or more (board rule 5):
//
//   node scripts/agent-eval/my-context.mjs [--session <key>] [--json]
//        [--vault <dir> | --db <file>] [--marker <file>] [--transcripts <dir>]...
//
// The run comes from the scratch marker (`$HOLLOW_SCRATCH_DIR/.hollow-scratch`,
// scratch.rs), because a chat's Bash environment carries no run id; with
// `--session`, from that conversation's running turn instead. The usage
// ledger names the run's native session, and its Claude transcript's last
// main-thread call (not a subagent's) is the context the next call starts
// from: input + cache read + cache write.
//
// Read-only, usage blocks and timestamps only (scorecard.mjs readers): no
// message text, prompt or tool output is read into the report. Anything
// missing exits 3 with the reason; it never guesses.
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { LARGE_CONTEXT, defaultDb, indexTranscripts, openDb, readCalls } from './scorecard.mjs';

/** Board rule 5: hand over at this many context tokens. */
export const HANDOVER_AT = 350_000;
const MISSING = 3;

const USAGE = `usage: node scripts/agent-eval/my-context.mjs [--session <key>] [--json]
       [--vault <dir> | --db <file>] [--marker <file>] [--transcripts <dir>]...
Without --session the run comes from $HOLLOW_SCRATCH_DIR/.hollow-scratch. Without --vault or --db
every sessions database under Hollow's Application Support is searched for the run.`;

class Missing extends Error {}

export function parseArgs(argv) {
  const o = { transcripts: [], json: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v == null || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--session') o.session = val();
    else if (a === '--vault') o.vault = val();
    else if (a === '--db') o.db = val();
    else if (a === '--marker') o.marker = val();
    else if (a === '--transcripts') o.transcripts.push(val());
    else if (a === '--json') o.json = true;
    else throw new Error(`unknown argument ${a}`);
  }
  return o;
}

/** The run id in a scratch marker, or Missing. */
export function markerRunId(file) {
  if (!file || !existsSync(file)) throw new Missing(`no scratch marker at ${file ?? '$HOLLOW_SCRATCH_DIR/.hollow-scratch'}`);
  let m;
  try {
    m = JSON.parse(readFileSync(file, 'utf8'));
  } catch {
    throw new Missing(`the scratch marker ${file} is not JSON`);
  }
  if (typeof m?.runId !== 'string' || !m.runId) throw new Missing(`the scratch marker ${file} names no run`);
  return m.runId;
}

/** The sessions databases to search: the named one, or every one Hollow keeps. */
function databases(o, support) {
  if (o.db) return [resolve(o.db)];
  if (o.vault) return [defaultDb(resolve(o.vault), support)];
  const dir = join(support, 'Sessions');
  return existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith('.db')).map((f) => join(dir, f)) : [];
}

/** The ledger row for the run (or the session's running turn), from the
 *  first database that has one. Reads ids, times and the native session
 *  only; never turns.asked or any payload. */
async function findRun(o, runId, support) {
  const notes = [];
  let files;
  try {
    files = databases(o, support);
  } catch (e) {
    throw new Missing(`no sessions database: ${e.message}`);
  }
  for (const file of files) {
    if (!existsSync(file)) continue;
    let opened;
    try {
      opened = await openDb(file);
    } catch {
      continue;
    }
    const { db, mode, notes: dbNotes } = opened;
    try {
      let id = runId;
      if (o.session) {
        const turn = db.prepare("select run_id from turns where session_key = ? and status = 'running' and run_id is not null order by ord desc limit 1").get(o.session);
        if (!turn) continue;
        id = turn.run_id;
      }
      // The chat's own row: an ask_agent consultation (`delegate`, its
      // parent's run id) and helper calls (`oneshot`) share the run id.
      const row = db
        .prepare("select run_id, native_session, agent, outcome, started_at from usage_ledger where run_id = ? and parent_surface is null and surface not in ('oneshot', 'delegate') order by started_at desc limit 1")
        .get(id);
      if (row) return { row: { ...row }, source: mode, db: file };
      if (mode === 'immutable') notes.push(...dbNotes);
    } catch (e) {
      notes.push(`${file}: ${e.message}`);
    } finally {
      db.close();
    }
  }
  const why = o.session ? `no running turn with a ledger row for session ${o.session}` : `no ledger row for run ${runId}`;
  throw new Missing(notes.length ? `${why} (${notes.join('; ')})` : why);
}

/** Measure the run's live context. Throws Missing when a source is absent. */
export async function myContext(o, { env = process.env, support = join(homedir(), 'Library', 'Application Support', 'Hollow'), now = Date.now() } = {}) {
  const runId = o.session ? null : markerRunId(o.marker ?? (env.HOLLOW_SCRATCH_DIR && join(env.HOLLOW_SCRATCH_DIR, '.hollow-scratch')));
  const { row, source, db } = await findRun(o, runId, support);
  if (!row.native_session) throw new Missing(`run ${row.run_id} has no native session in the ledger yet`);
  const roots = o.transcripts.length
    ? o.transcripts.map((d) => resolve(d))
    : [join(env.CLAUDE_CONFIG_DIR || join(homedir(), '.claude'), 'projects')];
  let calls;
  try {
    const file = indexTranscripts(roots).get(row.native_session)?.main;
    if (!file) throw new Missing(`no transcript for native session ${row.native_session}`);
    calls = (await readCalls(file, false)).sort((a, b) => a.t - b.t);
  } catch (e) {
    throw e instanceof Missing ? e : new Missing(`the transcript for ${row.native_session} could not be read: ${e.message}`);
  }
  const last = calls.at(-1);
  if (!last) throw new Missing(`the transcript for ${row.native_session} has no main-thread call yet`);
  const ctx = (c) => c.input + c.cacheRead + c.cacheWrite;
  // This run's calls: a resumed conversation's transcript holds earlier turns too.
  const startedAt = Date.parse(row.started_at ?? '');
  if (!Number.isFinite(startedAt)) throw new Missing(`run ${row.run_id} has no readable start time in the ledger`);
  const run = calls.filter((c) => c.t >= startedAt - 1000);
  return {
    runId: row.run_id,
    nativeSession: row.native_session,
    agent: row.agent,
    outcome: row.outcome,
    source,
    db,
    contextTokens: ctx(last),
    calls: run.length,
    callsOver400k: run.filter((c) => ctx(c) >= LARGE_CONTEXT).length,
    peakTokens: Math.max(...run.map(ctx), 0),
    ageSeconds: Math.max(0, Math.round((now - last.t) / 1000)),
    handOverAt: HANDOVER_AT,
    handOver: ctx(last) >= HANDOVER_AT,
  };
}

const k = (n) => `${Math.round(n / 1000)}K`;

export function render(r) {
  const lines = [
    `Context ${k(r.contextTokens)} (${r.contextTokens} tokens) on the last main-thread call, ${r.ageSeconds} s ago.`,
    `This run: ${r.calls} calls, ${r.callsOver400k} at or over 400K, peak ${k(r.peakTokens)}.`,
    r.handOver
      ? `At or over ${k(r.handOverAt)}: finish or park the current item and hand over (board rule 5).`
      : `Under ${k(r.handOverAt)}: carry on.`,
  ];
  if (r.source === 'immutable') lines.push('The database was opened immutable: the newest ledger rows may be missing.');
  return lines.join('\n');
}

export async function main(argv, opts) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  try {
    const r = await myContext(o, opts);
    console.log(o.json ? JSON.stringify(r, null, 2) : render(r));
    return 0;
  } catch (e) {
    if (!(e instanceof Missing)) throw e;
    console.error(`my-context: ${e.message}`);
    return MISSING;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
