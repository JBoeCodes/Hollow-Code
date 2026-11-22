// Overnight 2026-10-16/17 OF27: a chat's own live context
// (scripts/agent-eval/my-context.mjs) from its scratch marker, through the
// usage ledger, to the last main-thread call of its Claude transcript. The
// fixture database uses the store's real DDL; the transcript has three main
// calls and one larger subagent call that must not count.
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { after, test } from 'node:test';
import { main, myContext } from './agent-eval/my-context.mjs';

const rust = (file) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');
const CANARY = 'CANARY-content-never-read';

/** session_store.rs apply_schema's literal batches and added columns, so a
 *  renamed column the script reads breaks this test. */
function schema() {
  const unwrap = (s) => s.replace(/\\\n\s*/g, ' ');
  const store = rust('session_store.rs');
  const body = store.slice(store.indexOf('fn apply_schema'), store.indexOf('\nfn ', store.indexOf('fn apply_schema') + 1));
  const sql = [...body.matchAll(/execute_batch\(\s*"([\s\S]*?)",?\s*\)/g)].map((m) => unwrap(m[1]));
  for (const m of body.matchAll(/ensure_column\(\s*conn,\s*"(\w+)",\s*"\w+",\s*"([^"]+)",?\s*\)/g)) sql.push(`ALTER TABLE ${m[1]} ADD COLUMN ${m[2]}`);
  for (const m of body.matchAll(/for \(column, ddl\) in \[([\s\S]*?)\] \{\s*ensure_column\(conn, "(\w+)", column, ddl\)/g)) {
    for (const c of m[1].matchAll(/\("\w+", "([^"]+)"\)/g)) sql.push(`ALTER TABLE ${m[2]} ADD COLUMN ${c[1]}`);
  }
  return sql.join(';\n');
}

const dir = mkdtempSync(join(tmpdir(), 'my-context-'));
after(() => rmSync(dir, { recursive: true, force: true }));

const T = '2026-10-17T03:';
const db = join(dir, 'sessions.db');
{
  const d = new DatabaseSync(db);
  d.exec(schema());
  const ledger = d.prepare(`insert into usage_ledger (run_key, at, started_at, surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_status, usage_json)
    values (?,?,?,'chat',?,'claude',?,'subscription',?,1,'per_run','partial',?)`);
  ledger.run('k1', T + '10:00Z', T + '00:00Z', 'run-live', 'ns-live', 'running', CANARY);
  // A running row whose native session is not known yet.
  ledger.run('k2', T + '10:00Z', T + '00:00Z', 'run-early', null, 'running', null);
  // An ask_agent consultation and a helper call under the live run: they
  // share its run id, start later and have their own native sessions.
  const other = d.prepare(`insert into usage_ledger (run_key, at, started_at, surface, parent_surface, run_id, agent, native_session, billing, outcome, spawned, basis, cost_status)
    values (?,?,?,?,?,'run-live','claude',?,'subscription','success',1,'per_run','estimated')`);
  other.run('k3', T + '02:00Z', T + '01:30Z', 'delegate', 'chat', 'ns-delegate');
  other.run('k4', T + '02:10Z', T + '01:40Z', 'oneshot', null, 'ns-oneshot');
  const turn = d.prepare('insert into turns (session_key, ord, run_id, started_at, status, asked) values (?,?,?,?,?,?)');
  turn.run('s-live', 1, 'run-old', '2026-10-16T20:00:00Z', 'success', CANARY);
  turn.run('s-live', 2, 'run-live', T + '00:00Z', 'running', CANARY);
  d.close();
}

// Three main calls (the second written as two content-block lines, as
// Claude Code does), a larger subagent call, and a turn before the run.
// The delegate's and helper's transcripts are larger still.
const transcripts = join(dir, 'projects');
mkdirSync(join(transcripts, 'p'), { recursive: true });
const call = (id, t, input, read, write, extra = {}) =>
  JSON.stringify({
    type: 'assistant',
    timestamp: t,
    message: { id, content: [{ type: 'text', text: CANARY }], usage: { input_tokens: input, cache_read_input_tokens: read, cache_creation_input_tokens: write, output_tokens: 10 } },
    ...extra,
  });
writeFileSync(
  join(transcripts, 'p', 'ns-live.jsonl'),
  [
    JSON.stringify({ type: 'user', timestamp: T + '00:01Z', message: { content: CANARY } }),
    call('m0', '2026-10-16T20:00:05Z', 1, 450_000, 0), // an earlier turn: not this run
    call('m1', T + '00:10Z', 10, 100_000, 5_000),
    call('m2', T + '01:00Z', 10, 410_000, 1_000),
    call('m2', T + '01:00Z', 10, 410_000, 2_000),
    call('m3', T + '02:00Z', 5, 300_000, 4_000),
    call('s1', T + '02:30Z', 5, 600_000, 0, { isSidechain: true }),
    JSON.stringify({ type: 'user', timestamp: T + '02:40Z', toolUseResult: CANARY, message: { content: CANARY } }),
  ].join('\n') + '\n',
);
for (const ns of ['ns-delegate', 'ns-oneshot']) writeFileSync(join(transcripts, 'p', `${ns}.jsonl`), call(`${ns}-1`, T + '02:50Z', 0, 380_000, 0) + '\n');
const marker = (runId) => {
  const f = join(dir, `marker-${runId}`);
  writeFileSync(f, JSON.stringify({ pid: 1, runId, createdAt: '2026-10-17T03:00:00Z' }));
  return f;
};
const now = Date.parse(T + '03:00Z');
const base = ['--db', db, '--transcripts', transcripts];

/** Run main with captured output, failing on any read of a content field:
 *  a SQL statement naming one, or a transcript line's content touched. */
async function run(argv) {
  const out = [];
  const err = [];
  const log = console.log;
  const error = console.error;
  const parse = JSON.parse;
  const prepare = DatabaseSync.prototype.prepare;
  const touched = [];
  console.log = (s) => out.push(String(s));
  console.error = (s) => err.push(String(s));
  JSON.parse = function (text, reviver) {
    const v = parse.call(this, text, reviver);
    for (const [obj, key] of [[v?.message, 'content'], [v, 'toolUseResult']]) {
      if (obj && typeof obj === 'object' && key in obj) {
        const value = obj[key];
        Object.defineProperty(obj, key, { enumerable: true, get: () => (touched.push(key), value) });
      }
    }
    return v;
  };
  DatabaseSync.prototype.prepare = function (sql) {
    if (/(select|,)\s*\*|\.\*|\basked\b|payload|usage_json|\bdetail\b|\btitle\b/i.test(sql)) touched.push(sql);
    return prepare.call(this, sql);
  };
  let code;
  try {
    code = await main(argv, { env: {}, support: join(dir, 'no-support'), now });
  } finally {
    console.log = log;
    console.error = error;
    JSON.parse = parse;
    DatabaseSync.prototype.prepare = prepare;
  }
  assert.deepEqual(touched, [], 'a content field was read');
  const text = [...out, ...err].join('\n');
  assert.ok(!text.includes(CANARY), 'content reached the output');
  return { code, out: out.join('\n'), err: err.join('\n') };
}

test('the last main-thread call is the context; the larger subagent call and the earlier turn are ignored', async () => {
  const { code, out } = await run(['--json', '--marker', marker('run-live'), ...base]);
  assert.equal(code, 0);
  const r = JSON.parse(out);
  assert.equal(r.runId, 'run-live');
  assert.equal(r.nativeSession, 'ns-live');
  assert.equal(r.source, 'ro');
  // m3: 5 + 300,000 + 4,000.
  assert.equal(r.contextTokens, 304_005);
  // This run: m1, m2 (one call over two lines), m3; m0 is before started_at.
  assert.equal(r.calls, 3);
  // m2 is 410,010 + 2,000 (the larger of its two snapshots).
  assert.equal(r.callsOver400k, 1);
  assert.equal(r.peakTokens, 412_010);
  assert.equal(r.ageSeconds, 60);
  assert.equal(r.handOver, false);
});

test('--session finds the conversation’s running turn; the text form says what to do', async () => {
  const { code, out } = await run(['--session', 's-live', ...base]);
  assert.equal(code, 0);
  assert.match(out, /^Context 304K \(304005 tokens\) on the last main-thread call, 60 s ago\.$/m);
  assert.match(out, /Under 350K: carry on\./);
});

test('at 350K or more it says to hand over', async () => {
  const r = await myContext({ marker: marker('run-live'), db, transcripts: [transcripts] }, { env: {}, now });
  assert.equal(r.handOver, false);
  const { out } = await run(['--marker', marker('run-live'), ...base]);
  assert.doesNotMatch(out, /hand over/);
  // A copy of the transcript with the last main call grown past the line.
  const grownDir = join(dir, 'grown');
  mkdirSync(join(grownDir, 'p'), { recursive: true });
  const live = readFileSync(join(transcripts, 'p', 'ns-live.jsonl'), 'utf8');
  writeFileSync(join(grownDir, 'p', 'ns-live.jsonl'), live + call('m4', T + '02:50Z', 0, 360_000, 0) + '\n');
  const grown = await run(['--marker', marker('run-live'), '--db', db, '--transcripts', grownDir]);
  assert.match(grown.out, /At or over 350K: finish or park the current item and hand over/);
});

test('anything missing exits 3 with the reason, never a guess', async () => {
  const cases = [
    [['--marker', join(dir, 'absent'), ...base], /no scratch marker/],
    [[...base], /no scratch marker/], // no --marker and no HOLLOW_SCRATCH_DIR
    [['--marker', marker('run-nowhere'), ...base], /no ledger row for run run-nowhere/],
    [['--marker', marker('run-early'), ...base], /no native session/],
    [['--session', 's-none', ...base], /no running turn/],
    [['--marker', marker('run-live'), '--db', db, '--transcripts', join(dir, 'empty')], /no transcript for native session ns-live/],
    [['--marker', marker('run-live'), '--vault', join(dir, 'no-vault'), '--transcripts', transcripts], /no sessions database/],
  ];
  for (const [argv, why] of cases) {
    const { code, err } = await run(argv);
    assert.equal(code, 3, argv.join(' '));
    assert.match(err, why);
  }
  writeFileSync(join(dir, 'bad-marker'), '{');
  assert.equal((await run(['--marker', join(dir, 'bad-marker'), ...base])).code, 3);
});

test('a bad argument exits 2', async () => {
  assert.equal((await run(['--nope'])).code, 2);
  assert.equal((await run(['--session'])).code, 2);
});
