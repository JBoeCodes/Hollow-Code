// Overnight 2026-10-17 OF21: the morning report
// (scripts/agent-eval/morning-report.mjs). Tonight's lanes against the
// evening lanes from lane reports, the sessions database (the store's real
// DDL), Claude transcripts and a fake git log. Two lanes tonight: OB is one
// chat still running (a NULL-cost row), OF is a chain of two chats joined by a
// handover; the evening has one lane, HX, whose report names no chat. Every
// expected number is worked out by hand next to it.
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { after, test } from 'node:test';
import { isPollOnly, isPythonEdit, main, parseGitLog, parseLaneReport } from './agent-eval/morning-report.mjs';
import { PULSE_OK_SQL, USAGE_STATS_SQL, summaryFromVault } from './agent-eval/scorecard.mjs';

const rust = (file) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');
const CANARY = 'CANARY-content-never-read';

/** session_store.rs apply_schema's literal batches and added columns, so a
 *  renamed column the report reads breaks this test. */
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

const dir = mkdtempSync(join(tmpdir(), 'morning-report-'));
after(() => rmSync(dir, { recursive: true, force: true }));

const OB = 's-20261017-0b000001';
const OF1 = 's-20261017-0f000001';
const OF2 = 's-20261017-0f000002';
const CHILD = 's-20261017-0c000009';
const HX = 's-20261016-0a000001';
const OTHER = 's-20261017-0d000001';
// The evening lane HF's real key: the report's built-in evening default.
const HF = 's-20261016-39fdcf85';
const N = '2026-10-17T';
const E = '2026-10-16T';

const db = join(dir, 'sessions.db');
{
  const d = new DatabaseSync(db);
  d.exec(schema());
  const session = d.prepare("insert into sessions (key, title, agent, cwd, created_at, last_activity_at) values (?,?,'claude','/hw',?,?)");
  for (const [key, created] of [[OB, N + '02:24:30Z'], [OF1, N + '02:24:40Z'], [OF2, N + '03:29:05Z'], [CHILD, N + '02:40:00Z'], [HX, E + '21:30:00Z'], [OTHER, N + '03:00:00Z'], [HF, E + '21:11:00Z']]) {
    session.run(key, CANARY, created, created);
  }
  const turn = d.prepare('insert into turns (session_key, ord, run_id, started_at, ended_at, status, asked) values (?,1,?,?,?,?,?)');
  turn.run(OB, 'run-ob1', N + '02:24:30Z', null, 'running', CANARY);
  turn.run(OF1, 'run-of1', N + '02:24:40Z', N + '03:30:00Z', 'success', CANARY);
  turn.run(OF2, 'run-of2', N + '03:29:05Z', N + '04:30:00Z', 'success', CANARY);
  turn.run(HX, 'run-hx1', E + '21:30:00Z', E + '22:30:30Z', 'success', CANARY);
  turn.run(OTHER, 'run-x', N + '03:00:00Z', N + '03:10:00Z', 'success', CANARY);
  turn.run(HF, 'run-hf1', E + '21:11:00Z', E + '23:00:00Z', 'success', CANARY);
  const ledger = d.prepare(`insert into usage_ledger (run_key, at, started_at, surface, scope, parent_surface, run_id, agent, native_session, billing, outcome, spawned, basis,
    input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status, usage_json) values (?,?,?,?,?,?,?,'claude',?,'subscription',?,1,'per_run',10,10,10,?,?,?)`);
  // OB's lane chat is still running: no whole cost yet (partial; the $1.25
  // it carries so far is not a known cost).
  ledger.run('k-ob1', N + '04:50:00Z', N + '02:24:30Z', 'chat', OB, null, 'run-ob1', 'ns-ob1', 'running', 1.25, 'partial', CANARY);
  ledger.run('k-of1', N + '03:30:00Z', N + '02:24:40Z', 'chat', OF1, null, 'run-of1', 'ns-of1', 'success', 10, 'estimated', CANARY);
  // An ask_agent consultation under OF's first chat: its cost is the lane's,
  // its transcript (a 700K call) is not the lane's main thread.
  ledger.run('k-del', N + '03:01:00Z', N + '03:00:00Z', 'delegate', null, 'chat', 'run-of1', 'ns-del', 'success', 0.5, 'estimated', CANARY);
  ledger.run('k-of2', N + '04:30:00Z', N + '03:29:05Z', 'chat', OF2, null, 'run-of2', 'ns-of2', 'success', 5, 'estimated', CANARY);
  ledger.run('k-hx1', E + '22:30:30Z', E + '21:30:00Z', 'chat', HX, null, 'run-hx1', 'ns-hx1', 'success', 4, 'estimated', CANARY);
  // Not a lane.
  ledger.run('k-x', N + '03:10:00Z', N + '03:00:00Z', 'chat', OTHER, null, 'run-x', 'ns-x', 'success', 99, 'estimated', CANARY);
  // HF: the ledger counts 1,000,030 tokens; its transcript carries 410,000.
  d.prepare(`insert into usage_ledger (run_key, at, started_at, surface, scope, run_id, agent, native_session, billing, outcome, spawned, basis,
    input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status) values ('k-hf1',?,?,'chat',?,'run-hf1','claude','ns-hf1','subscription','success',1,'per_run',1000000,10,20,7,'estimated')`).run(E + '23:00:00Z', E + '21:11:00Z', HF);
  const delegation = d.prepare('insert into delegations (run_id, at, record_json) values (?,?,?)');
  delegation.run('run-of1', N + '03:29:00Z', JSON.stringify({ kind: 'childTask', tool: 'handover', childSession: OF2, packet: CANARY }));
  // A child task the lane started is not a successor.
  delegation.run('run-of1', N + '02:40:00Z', JSON.stringify({ kind: 'childTask', tool: 'start_session', childSession: CHILD, packet: CANARY }));
  const stint = d.prepare('insert into stints (session_key, agent, started_at, ended_at, reason) values (?,?,?,?,?)');
  stint.run(OB, 'claude', N + '00:00:00Z', N + '02:00:00Z', 'config changed'); // before the window
  stint.run(OB, 'claude', N + '02:24:30Z', null, null);
  stint.run(OF1, 'claude', N + '02:24:40Z', N + '03:30:00Z', null);
  stint.run(OF2, 'claude', N + '03:29:05Z', null, null);
  stint.run(HX, 'claude', E + '21:30:00Z', null, null);
  d.close();
}

// Transcripts. Two Python edit scripts (OB's b1, OF's f1) and one Edit
// (OB's b2) in the lanes' main threads; text, thinking, tool inputs other
// than a Bash command, and tool results all carry the canary.
const transcripts = join(dir, 'projects');
mkdirSync(join(transcripts, 'p', 'ns-ob1', 'subagents'), { recursive: true });
const usage = ([input, write, read, output]) => ({ input_tokens: input, cache_creation_input_tokens: write, cache_read_input_tokens: read, output_tokens: output });
const text = { type: 'text', text: CANARY };
const thinking = { type: 'thinking', thinking: CANARY };
const tool = (id, name, input) => ({ type: 'tool_use', id, name, input });
const bash = (id, command) => tool(id, 'Bash', { command, description: CANARY });
const call = (id, t, u, content, extra = {}) => JSON.stringify({ type: 'assistant', timestamp: t, message: { id, content, usage: usage(u) }, ...extra });
const user = (t) => JSON.stringify({ type: 'user', timestamp: t, message: { content: [{ type: 'tool_result', content: CANARY }] }, toolUseResult: CANARY });
const write = (rel, lines) => writeFileSync(join(transcripts, 'p', rel), `${lines.join('\n')}\n`);
const PY_HEREDOC = "cd ~/w && python3 - <<'EOF'\np='a.rs'; s=open(p).read()\ns=s.replace('a','b')\nopen(p,'w').write(s)\nEOF";
const PY_DASH_C = `cd ~/w && python3 -I -c "import pathlib; p=pathlib.Path('a.md'); p.write_text(p.read_text().replace('a','b'))"`;
write('ns-ob1.jsonl', [
  user(N + '02:24:31Z'),
  call('b0', N + '02:10:00Z', [0, 0, 900_000, 0], [bash('t0', 'git status')]), // an earlier turn: not this run
  call('b1', N + '02:26:00Z', [10, 5_000, 100_000, 100], [bash('t1', PY_HEREDOC)]),
  user(N + '02:26:30Z'),
  call('b2', N + '02:30:00Z', [0, 0, 450_000, 200], [tool('t2', 'Edit', { file_path: CANARY, old_string: CANARY, new_string: CANARY })]),
  call('b3', N + '02:40:00Z', [0, 1_000, 200_000, 50], [thinking]),
  call('b3', N + '02:40:00Z', [0, 1_000, 200_000, 50], [bash('t3', `node -e "setTimeout(()=>{},60000)"; tail -2 log`)]),
  // One call written as two content-block lines; its usage grows.
  call('b4', N + '02:50:00Z', [5, 0, 360_000, 40], [text]),
  call('b4', N + '02:50:00Z', [5, 0, 360_000, 100], [bash('t4', 'python3 -c "print(1)"')]),
  user(N + '02:51:00Z'),
]);
writeFileSync(join(transcripts, 'p', 'ns-ob1', 'subagents', 'agent-a.jsonl'), `${call('sb1', N + '02:35:00Z', [1_000, 0, 50_000, 1_000], [bash('s1', PY_HEREDOC)], { isSidechain: true })}\n`);
write('ns-of1.jsonl', [
  call('f1', N + '02:30:00Z', [0, 2_000, 300_000, 300], [bash('u1', PY_DASH_C)]),
  call('f2', N + '03:28:00Z', [0, 0, 380_000, 100], [text]),
  call('f3', N + '03:29:30Z', [0, 0, 390_000, 100], [tool('u3', 'mcp__hollow_app__send_to_session', { prompt: CANARY })]),
]);
write('ns-of2.jsonl', [user(N + '03:29:06Z'), call('g1', N + '03:30:00Z', [100, 50_000, 0, 100], [bash('v1', 'git status')])]);
write('ns-hx1.jsonl', [call('h1', E + '21:40:00Z', [0, 0, 410_000, 0], [tool('w1', 'Write', { file_path: CANARY, content: CANARY })])]);
write('ns-del.jsonl', [call('d1', N + '03:00:30Z', [0, 0, 700_000, 0], [bash('x1', PY_HEREDOC)])]);
write('ns-x.jsonl', [call('x1', N + '03:05:00Z', [0, 0, 600_000, 0], [bash('y1', PY_HEREDOC)])]);
write('ns-hf1.jsonl', [call('i1', E + '21:40:00Z', [0, 0, 410_000, 0], [tool('z1', 'Edit', { file_path: CANARY, old_string: CANARY, new_string: CANARY })])]);

// Lane reports, as the lanes write them: bullets with `→ #N → sha` claims,
// a link-form claim, a heading item (overnight-summary's format), and other
// lanes' PRs mentioned in passing.
const hw = join(dir, 'hw');
const tonight = join(hw, 'outputs', 'overnight-2026-10-17');
const evening = join(hw, 'outputs', 'evening-2026-10-16');
mkdirSync(tonight, { recursive: true });
mkdirSync(evening, { recursive: true });
writeFileSync(join(tonight, 'lane-ob.md'), `# Overnight · lane OB\n\nChat \`${OB}\`.\n\n## Done\n\n- **OB1** → #801 → merge \`aaaaaaa1\`. OF's #802 is mentioned in passing.\n`);
writeFileSync(
  join(tonight, 'lane-of.md'),
  `# Overnight · lane OF\n\n## Done\n\n- **OF1** → #802 → \`bbbbbbb2\`.\n- **OF2** → [#803](https://github.com/x/y/pull/803) → \`ccccccc3\`.\n\n## State\n\n- 03:30 · successor 1 (\`${OF2}\`). OB merged #801.\n`,
);
writeFileSync(
  join(evening, 'lane-hx.md'),
  `# Evening · lane HX\n\n## Done\n\n- **HX1** → #780 → \`ddddddd4\`.\n\n### X2 · a heading item — merged\n\n**PR:** [#781](https://github.com/x/y/pull/781)\n`,
);
const gitLog = join(dir, 'git-log.txt');
writeFileSync(
  gitLog,
  [
    `${'1'.repeat(40)}\t2026-10-17T00:00:00-04:00\tMerge pull request #802 from J/o17/OF1`,
    `${'2'.repeat(40)}\t2026-10-16T23:30:00-04:00\tMerge pull request #801 from J/o17/OB1`,
    `${'3'.repeat(40)}\t2026-10-16T22:00:00-04:00\tMerge pull request #799 from J/o17/X`, // before the window
    `${'4'.repeat(40)}\t2026-10-16T18:40:00-04:00\tMerge pull request #781 from J/e16/HX2`,
    `${'5'.repeat(40)}\t2026-10-16T18:30:00-04:00\tMerge pull request #780 from J/e16/HX1`,
    `${'6'.repeat(40)}\t2026-10-16T18:20:00-04:00\tMerge branch 'x'`,
  ].join('\n') + '\n',
);
const vault = join(dir, 'vault');
mkdirSync(vault);

const base = ['--since', N + '02:25:00Z', '--until', N + '07:00:00Z', '--hw', hw, '--vault', vault, '--db', db, '--transcripts', transcripts, '--git-log', gitLog,
  '--baseline-since', E + '21:00:00Z', '--baseline-until', E + '23:10:00Z'];

/** Run main with captured output, failing on any read of a content field: a
 *  SQL statement naming one, or a transcript's text, thinking, tool result,
 *  prompt, or a tool input other than a Bash command touched. */
/** The tool-use input fields the report may read, by field: Bash `command`
 *  (its own `toolUse`) and the scorecard tool pass's whitelist. */
const TOOL_PASS_INPUT = {
  command: new Set(['Bash']),
  query: new Set(['ToolSearch']),
  // MultiEdit since SC4 (S13: a write into the auto-memory folder).
  file_path: new Set(['Read', 'Edit', 'Write', 'MultiEdit']),
  id: new Set(['mcp__hollow_app__run_action']),
};

/** The product scorecard's one read of a stored transcript's `payload` (a
 *  fixed boolean, OF23), copied here by hand rather than imported: the guard
 *  must not trust the constant it guards, so a rewrite of PULSE_OK_SQL fails
 *  the equality test below instead of widening what the guard lets through. */
const PULSE_OK_EXPR = "(case when json_valid(tr.payload) then trim(coalesce(json_extract(tr.payload, '$.text'), ''), ' ' || char(9, 10, 13)) = 'PULSE_OK' end)";
/** The product scorecard's one read of a ledger row's `usage_json` (OB05's
 *  call figures, OF28), copied by hand for the same reason. */
const USAGE_STATS_EXPR = "(case when json_valid(usage_json) then json_object('apiCalls', json_extract(usage_json, '$.apiCalls'), 'peakContextTokens', json_extract(usage_json, '$.peakContextTokens'), 'subagents', json_type(usage_json, '$.subagents') = 'object', 'subagentCalls', json_extract(usage_json, '$.subagents.calls'), 'subagentTokens', json_extract(usage_json, '$.subagents.tokens')) end)";
/** A SQL statement that names a content column. PULSE_OK_EXPR and
 *  USAGE_STATS_EXPR are taken out first, character for character; any other
 *  mention of `payload` or `usage_json`, beside them or instead of them,
 *  still counts. */
const CONTENT_SQL = /(select|,)\s*\*|\.\*|\basked\b|payload|usage_json|\bdetail\b|\btitle\b|\bpacket\b|\bmessage\b|\bexamples\b/i;
const contentSql = (sql) => CONTENT_SQL.test(sql.split(PULSE_OK_EXPR).join(' ').split(USAGE_STATS_EXPR).join(' '));

/** `measured`: the product scorecard's summary runs too, whose context
 *  anatomy (OF19) measures the byte size of every prompt, text, thinking
 *  part, tool input and tool result: those reads are allowed, a transcript's
 *  `toolUseResult` and every SQL rule still aren't, and no canary may reach
 *  the output. */
async function run(argv, { measured = false } = {}) {
  const out = [];
  const err = [];
  const log = console.log;
  const error = console.error;
  const parse = JSON.parse;
  const prepare = DatabaseSync.prototype.prepare;
  const touched = [];
  const guard = (obj, key) => {
    if (!obj || typeof obj !== 'object' || !(key in obj)) return;
    const value = obj[key];
    Object.defineProperty(obj, key, { enumerable: true, get: () => (touched.push(key), value) });
  };
  console.log = (s) => out.push(String(s));
  console.error = (s) => err.push(String(s));
  JSON.parse = function (src, reviver) {
    const v = parse.call(this, src, reviver);
    // The summary's tool pass (OF02) reads a user line's tool_result parts
    // and one input field per tool (scorecard.mjs "The tool pass"); a
    // prompt, any text and every other input stay guarded. These fixtures
    // hold no failed Bash call and no wait, so no result's text is read.
    if (v?.type === 'user') {
      if (measured) {
        // Sizes only; the output check below still holds.
      } else if (Array.isArray(v.message?.content)) {
        for (const p of v.message.content) {
          guard(p, 'text');
          guard(p, 'content');
        }
      } else guard(v.message, 'content');
      guard(v, 'toolUseResult');
    } else if (v?.type === 'assistant' && Array.isArray(v.message?.content) && !measured) {
      for (const b of v.message.content) {
        guard(b, 'text');
        guard(b, 'thinking');
        if (b?.type === 'tool_use') for (const k of Object.keys(b.input ?? {})) if (!TOOL_PASS_INPUT[k]?.has(b.name)) guard(b.input, k);
      }
    }
    return v;
  };
  DatabaseSync.prototype.prepare = function (sql) {
    if (contentSql(sql)) touched.push(sql);
    return prepare.call(this, sql);
  };
  let code;
  try {
    code = await main(argv, { now: Date.parse(N + '07:00:00Z') });
  } finally {
    console.log = log;
    console.error = error;
    JSON.parse = parse;
    DatabaseSync.prototype.prepare = prepare;
  }
  assert.deepEqual(touched, [], 'a content field was read');
  const all = [...out, ...err].join('\n');
  assert.ok(!all.includes(CANARY), 'content reached the output');
  return { code, out: out.join('\n'), err: err.join('\n') };
}

async function report(extra = [], args = base) {
  const json = join(dir, `report-${process.pid}-${Math.random().toString(36).slice(2)}.json`);
  const md = `${json}.md`;
  const r = await run([...args, '--no-product', ...extra, '--json', json, '--out', md]);
  assert.equal(r.code, 0, r.err);
  const text = readFileSync(md, 'utf8');
  assert.ok(!text.includes(CANARY) && !readFileSync(json, 'utf8').includes(CANARY), 'content reached a written file');
  return { r, doc: JSON.parse(readFileSync(json, 'utf8')), md: text };
}
const lane = (doc, group, name) => doc.lanes[group].find((l) => l.lane === name);

test('the SQL guard lets exactly PULSE_OK_SQL name a transcript payload', () => {
  // The guard's own copy pins the constant: changing PULSE_OK_SQL fails here.
  assert.equal(PULSE_OK_SQL, PULSE_OK_EXPR);
  // The scorecard's fixed boolean passes, alone or in its real statement.
  assert.equal(contentSql(`select t.session_key, t.ord, ${PULSE_OK_SQL} pulse_ok from turns t join transcripts tr on tr.session_key = t.session_key and tr.turn_ord = t.ord where t.asked_by like 'hollow:%'`), false);
  // A plain read of the column, its text, or the boolean beside a raw read still fails.
  assert.equal(contentSql('select payload from transcripts'), true);
  assert.equal(contentSql("select json_extract(tr.payload, '$.text') from transcripts tr"), true);
  assert.equal(contentSql(`select ${PULSE_OK_SQL}, tr.payload from transcripts tr`), true);
  assert.equal(contentSql(`select ${PULSE_OK_SQL.replace("'PULSE_OK'", "'PULSE_OK' or 1")} from transcripts tr`), true);
  // turns.asked is still a content read; asked_by (a tag) is not.
  assert.equal(contentSql('select asked from turns'), true);
  assert.equal(contentSql('select asked_by from turns'), false);
  // OF28: the ledger's call figures pass; usage_json itself, its raw block
  // or its note do not, beside the expression or instead of it.
  assert.equal(USAGE_STATS_SQL, USAGE_STATS_EXPR);
  assert.equal(contentSql(`select run_key, ${USAGE_STATS_SQL} as usage_stats from usage_ledger`), false);
  assert.equal(contentSql('select usage_json from usage_ledger'), true);
  assert.equal(contentSql("select json_extract(usage_json, '$.note') from usage_ledger"), true);
  assert.equal(contentSql(`select ${USAGE_STATS_SQL}, usage_json from usage_ledger`), true);
});

test('lane reports give session keys and the PRs each lane claims', () => {
  const ob = parseLaneReport('lane-ob.md', readFileSync(join(tonight, 'lane-ob.md'), 'utf8'));
  assert.deepEqual(ob, { lane: 'OB', file: 'lane-ob.md', keys: [OB], successors: [], prs: ['801'] });
  // The link form, a successor named in the State section; #801 is OB's.
  const of = parseLaneReport('lane-of.md', readFileSync(join(tonight, 'lane-of.md'), 'utf8'));
  assert.deepEqual(of, { lane: 'OF', file: 'lane-of.md', keys: [OF2], successors: [OF2], prs: ['802', '803'] });
  // Only the key right after "successor" is one; its predecessor is not.
  assert.deepEqual(parseLaneReport('lane-zz.md', `handed over from \`${OF1}\` to successor 1 (\`${OF2}\`)`).successors, [OF2]);
  // A heading item with a **PR:** link (overnight-summary's ITEM/PR) counts too.
  assert.deepEqual(parseLaneReport('lane-hx.md', readFileSync(join(evening, 'lane-hx.md'), 'utf8')).prs, ['780', '781']);
  const log = parseGitLog(readFileSync(gitLog, 'utf8'));
  assert.equal(log.length, 5); // the merge with no PR number is dropped
  assert.deepEqual(log[0], { sha: '1'.repeat(40), at: Date.parse(N + '04:00:00Z'), pr: '802' });
});

test('Python edit scripts and poll-only calls are told apart by the command alone', () => {
  assert.equal(isPythonEdit(PY_HEREDOC), true);
  assert.equal(isPythonEdit(PY_DASH_C), true);
  assert.equal(isPythonEdit(`F=x; python3 - "$F" <<'EOF'\np=sys.argv[1]; s=open(p).read()\nopen(p, "w").write(s)\nEOF`), true);
  assert.equal(isPythonEdit('python3 -c "print(1)"'), false); // runs Python, writes nothing
  assert.equal(isPythonEdit(`python3 -I - <<'EOF'\nimport sys; sys.stdout.write('x')\nEOF`), false);
  assert.equal(isPythonEdit("cat > /tmp/q.py <<'EOF'\nopen('f','w').write('x')\nEOF"), false); // writes a script, runs no Python
  assert.equal(isPythonEdit('python3 -I /tmp/keepboth.py docs/BUILD_LOG.md'), false); // a script file, not inline
  assert.equal(isPollOnly([{ poll: true }]), true);
  assert.equal(isPollOnly([{ poll: true }, { poll: false }]), false);
  assert.equal(isPollOnly([]), false);
});

test('the per-lane block: calls, context, ≥400K share, handovers, edit scripts, polls, subagent share, cost and $ per merged PR', async () => {
  const { doc } = await report(['--lane', `hx=${HX}`]);
  const ob = lane(doc, 'tonight', 'OB');
  assert.deepEqual(ob.keys, [OB]);
  assert.equal(ob.chats, 1);
  assert.equal(ob.stints, 1); // the 00:00 stint ended before the window
  assert.equal(ob.runs, 1);
  const t = ob.transcript;
  // b1–b4 (b0 is an earlier turn) plus the subagent's sb1.
  assert.equal(t.calls, 5);
  assert.equal(t.mainCalls, 4);
  assert.equal(t.subagentCalls, 1);
  // Contexts 105,010 · 201,000 · 360,005 · 450,000: nearest rank 2nd and 4th.
  assert.deepEqual(t.contextPerCall, { n: 4, p50: 201_000, p95: 450_000 });
  // Weights (1, 1.25, 0.1, 5): b1 16,760, b2 46,000, b3 21,500, b4 36,505,
  // sb1 11,000 = 131,765. At ≥ 400K: b2 → 46,000 / 131,765.
  assert.equal(t.costShareAtLarge, 0.3491);
  assert.equal(t.subagentShare, 0.0835); // 11,000 / 131,765
  assert.equal(t.callsOverHandoverLine, 2); // b2, b4 at ≥ 350K
  // Bash b1 (edit), b3 (poll), b4 (print only); the subagent's is not counted.
  assert.equal(t.bashCalls, 3);
  assert.equal(t.pythonEditScripts, 1);
  assert.equal(t.pythonEditsPer100Bash, 33.33);
  assert.equal(t.editCalls, 1);
  assert.equal(t.writeCalls, 0);
  // The subagent's calls are classified too, and kept apart: sb1 is a
  // Python edit script in a Bash call.
  assert.equal(t.subagentBashCalls, 1);
  assert.equal(t.subagentPythonEditScripts, 1);
  assert.equal(t.subagentEditCalls, 0);
  assert.equal(t.subagentWriteCalls, 0);
  assert.equal(t.pollOnlyCalls, 1);
  assert.deepEqual(ob.handovers, []);
  // Running, no whole cost yet: unknown, never $0; the partial $1.25 is kept apart.
  assert.equal(ob.cost.knownRuns, 0);
  assert.equal(ob.cost.unknownRuns, 1);
  assert.deepEqual(ob.cost.unknownByStatus, { partial: 1 });
  assert.equal(ob.cost.partialCostUsd, 1.25);
  assert.equal(ob.cost.totalUsd, null);
  assert.equal(ob.cost.complete, false);
  assert.deepEqual(ob.mergedPrs, ['801']);
  assert.deepEqual(ob.claimedNotMerged, []);
  assert.deepEqual(ob.usdPerMergedPr, { usd: 0, complete: false });

  const of = lane(doc, 'tonight', 'OF');
  // The report names only the successor; the predecessor comes from the
  // handover record. The start_session child is not part of the lane.
  assert.deepEqual(of.keys, [OF1, OF2]);
  assert.equal(of.chats, 2);
  assert.equal(of.stints, 2);
  assert.equal(of.runs, 3); // two chat rows and the ask_agent consultation
  // f1 302,000 · f2 380,000 · f3 390,000 · g1 50,100; the consultation's 700K is not here.
  assert.deepEqual(of.transcript.contextPerCall, { n: 4, p50: 302_000, p95: 390_000 });
  assert.equal(of.transcript.costShareAtLarge, 0);
  assert.equal(of.transcript.subagentShare, 0);
  assert.equal(of.transcript.bashCalls, 2);
  assert.equal(of.transcript.pythonEditScripts, 1);
  assert.equal(of.transcript.pythonEditsPer100Bash, 50);
  assert.equal(of.transcript.editCalls, 0);
  assert.equal(of.transcript.pollOnlyCalls, 0);
  // The last main-thread call at or before the 03:29:00 handover is f2.
  assert.deepEqual(of.handovers, [{ at: N + '03:29:00.000Z', from: OF1, to: OF2, contextTokens: 380_000 }]);
  assert.equal(of.cost.totalUsd, 15.5); // 10 + 0.5 + 5
  assert.equal(of.cost.complete, true);
  assert.equal(of.cost.unknownRuns, 0);
  assert.deepEqual(of.mergedPrs, ['802']);
  assert.deepEqual(of.claimedNotMerged, ['803']);
  assert.deepEqual(of.usdPerMergedPr, { usd: 15.5, complete: true });

  // All of tonight: 8 main calls, 4th and 8th by rank; 46,000 / 306,865.
  const all = doc.totals.tonight;
  assert.deepEqual(all.transcript.contextPerCall, { n: 8, p50: 302_000, p95: 450_000 });
  assert.equal(all.transcript.costShareAtLarge, 0.1499);
  assert.equal(all.transcript.pythonEditsPer100Bash, 40); // 2 of 5
  assert.equal(all.cost.floorUsd, 15.5);
  assert.equal(all.cost.unknownRuns, 1);
  assert.equal(all.merged, 2);
  assert.equal(all.chats, 3);
  assert.equal(all.handovers, 1);
  assert.deepEqual(all.usdPerMergedPr, { usd: 7.75, complete: false });

  // The evening lane, named on the command line; 410K on its one call.
  const hx = lane(doc, 'baseline', 'HX');
  assert.deepEqual(hx.keys, [HX]);
  assert.deepEqual(hx.transcript.contextPerCall, { n: 1, p50: 410_000, p95: 410_000 });
  assert.equal(hx.transcript.costShareAtLarge, 1);
  assert.equal(hx.transcript.writeCalls, 1);
  assert.equal(hx.transcript.pythonEditsPer100Bash, null); // no Bash calls
  assert.deepEqual(hx.mergedPrs, ['780', '781']);
  assert.deepEqual(hx.usdPerMergedPr, { usd: 2, complete: true });
});

test('the Markdown leads with the lane table, tonight against the evening', async () => {
  const { md, r } = await report(['--lane', `hx=${HX}`]);
  const lines = md.split('\n');
  assert.match(lines[0], /^# Morning report 2026-10-17T02:25 → 2026-10-17T07:00$/);
  const first = lines.findIndex((l) => l.startsWith('## '));
  assert.equal(lines[first], '## Lanes tonight against the evening');
  assert.ok(lines.some((l) => /^\| OB \| 1 \| 4 \+ 1 \| 201K \/ 450K \| 34\.9% \| 0 \| 33\.3 \(1\/3\) · sub 1\/1 \| 1 \+ 0 \| 1 \| 8\.3% \| unknown \(1 without a cost\); \$0\.00 known \| 1 \| ≥ \$0\.00 \|$/.test(l)), md);
  assert.ok(lines.some((l) => /^\| OF \| 2 \| 4 \+ 0 \| 302K \/ 390K \| 0\.0% \| 1 \(380K\) \| 50\.0 \(1\/2\) · sub 0\/0 \| 0 \+ 0 \| 0 \| 0\.0% \| \$15\.50 \| 1 \| \$15\.50 \|$/.test(l)), md);
  assert.ok(lines.some((l) => l.startsWith('| All tonight | 3 | 8 + 1 | 302K / 450K | 15.0% |')), md);
  assert.ok(lines.some((l) => l.startsWith('| HX (evening) | 1 | 1 + 0 | 410K / 410K | 100.0% |')), md);
  assert.match(md, /OF: chats `s-20261017-0f000001` → `s-20261017-0f000002`; 2 stints; cost \$15\.50 over 3 ledger rows \(3 known, 0 unknown\); merged #802; claimed, not merged in the window: #803\./);
  // One status names itself; a partial row's carried cost is shown.
  assert.match(md, /OB: chats `s-20261017-0b000001`; 1 stint; cost unknown \(1 without a cost\); \$0\.00 known over 1 ledger row \(0 known, 1 unknown \(partial\); \$1\.25 on partial rows\); merged #801\./);
  assert.match(r.out, /wrote /);
});

test('a lane report that names no chat says so instead of guessing, and its cost is unknown, not $0', async () => {
  const { doc, md } = await report();
  const hx = lane(doc, 'baseline', 'HX');
  assert.deepEqual(hx.keys, []);
  assert.equal(hx.transcript, null);
  assert.equal(hx.cost.totalUsd, null);
  assert.equal(hx.cost.complete, false);
  assert.deepEqual(hx.mergedPrs, ['780', '781']);
  assert.deepEqual(hx.usdPerMergedPr, { usd: 0, complete: false });
  // The totals drop their complete flag with it.
  assert.equal(doc.totals.baseline.cost.totalUsd, null);
  assert.equal(doc.totals.baseline.cost.complete, false);
  assert.deepEqual(doc.totals.baseline.usdPerMergedPr, { usd: 0, complete: false });
  assert.match(md, /^- lane HX \(evening\): no chat named in lane-hx\.md; pass --lane hx=<key>$/m);
  assert.ok(md.split('\n').some((l) => /^\| HX \(evening\) \| 0 \| .*\| unknown \(no chat named\); \$0\.00 known \| 2 \| ≥ \$0\.00 \|$/.test(l)), md);
  assert.ok(md.split('\n').some((l) => /^\| All evening \| 0 \| .*\| unknown \(HX: no chat named\); \$0\.00 known \| 2 \| ≥ \$0\.00 \|$/.test(l)), md);
  assert.doesNotMatch(md, /\$0\.00 \| 2 \| \$0\.00 \|/);

  // A named chat missing from the database: unknown too, the seed named.
  const missing = 's-20261016-0a00dead';
  const m = await report(['--lane', `hx=${missing}`]);
  assert.equal(lane(m.doc, 'baseline', 'HX').cost.complete, false);
  assert.match(m.md, new RegExp(`^- lane HX \\(evening\\): ${missing} is not in the sessions database: its cost is unknown$`, 'm'));
});

test('a successor named with no handover record to its predecessor leaves the lane cost unknown', async () => {
  const night = join(dir, 'night-successor');
  mkdirSync(night, { recursive: true });
  // OTHER ($99, known) is called a successor; no handover names it.
  writeFileSync(join(night, 'lane-zz.md'), `# lane ZZ\n\n- 03:00 · successor 1 (\`${OTHER}\`).\n`);
  const { doc, md } = await report(['--tonight', night, '--lane', `hx=${HX}`]);
  const zz = lane(doc, 'tonight', 'ZZ');
  assert.equal(zz.cost.floorUsd, 99);
  assert.equal(zz.cost.totalUsd, null);
  assert.equal(zz.cost.complete, false);
  assert.match(md, new RegExp(`^- lane ZZ: lane-zz\\.md calls ${OTHER} a successor, but no handover record links its predecessor: its cost is unknown; pass --lane zz=<predecessor key>$`, 'm'));
  // With the predecessor named, the lane is whole again (here: OF1 as a stand-in).
  const named = await report(['--tonight', night, '--lane', `hx=${HX}`, '--lane', `zz=${OF1}`]);
  assert.equal(lane(named.doc, 'tonight', 'ZZ').cost.complete, true);
});

test('a chat in two lanes is counted once in the totals, with a note', async () => {
  const night = join(dir, 'night-overlap');
  mkdirSync(night, { recursive: true });
  // Both reports reach the OF chain (one names the successor, one the predecessor).
  writeFileSync(join(night, 'lane-oa.md'), `# lane OA\n\nChat \`${OF2}\`.\n`);
  writeFileSync(join(night, 'lane-oc.md'), `# lane OC\n\nChat \`${OF1}\`.\n`);
  const { doc, md } = await report(['--tonight', night, '--lane', `hx=${HX}`]);
  assert.equal(lane(doc, 'tonight', 'OA').cost.totalUsd, 15.5);
  assert.equal(lane(doc, 'tonight', 'OC').cost.totalUsd, 15.5);
  const all = doc.totals.tonight;
  assert.equal(all.cost.totalUsd, 15.5); // not 31
  assert.equal(all.cost.knownRuns, 3);
  assert.equal(all.chats, 2);
  assert.equal(all.handovers, 1);
  assert.equal(all.transcript.calls, 4);
  assert.match(md, new RegExp(`^- ${OF1} is in lanes OA and OC: counted once in the totals$`, 'm'));
  assert.match(md, new RegExp(`^- ${OF2} is in lanes OA and OC: counted once in the totals$`, 'm'));
});

test('the evening baseline uses the built-in HB/HF keys when the window is the default; overrides and other windows do not', async () => {
  const eve = join(dir, 'defaults', 'evening-2026-10-16');
  mkdirSync(eve, { recursive: true });
  writeFileSync(join(eve, 'lane-hf.md'), '# Evening · lane HF\n\nOne chat.\n');
  const { doc, md } = await report(['--baseline', eve]);
  const hf = lane(doc, 'baseline', 'HF');
  assert.deepEqual(hf.keys, [HF]);
  assert.equal(hf.cost.totalUsd, 7);
  assert.match(md, new RegExp(`^- lane HF \\(evening\\): no chat named in lane-hf\\.md; used the built-in evening key ${HF}$`, 'm'));
  // 410,000 transcript tokens against 1,000,030 in the ledger.
  assert.match(md, /^- lane HF \(evening\): transcripts carry 41\.0% of the ledger's tokens \(below 98%\): some calls are missing$/m);
  // Another window: no default.
  const other = await report(['--baseline', eve, '--baseline-since', E + '21:05:00Z']);
  assert.deepEqual(lane(other.doc, 'baseline', 'HF').keys, []);
  // An explicit --lane hf wins.
  const over = await report(['--baseline', eve, '--lane', `hf=${HX}`]);
  assert.deepEqual(lane(over.doc, 'baseline', 'HF').keys, [HX]);
});

test('the night folder is the one whose lane reports name the window\'s chats, with a note when two dates exist', async () => {
  const stale = join(hw, 'outputs', 'overnight-2026-10-16');
  mkdirSync(stale, { recursive: true });
  // The night before's lane report: its chat ran before this window.
  writeFileSync(join(stale, 'lane-a.md'), `# lane A\n\nChat \`${HX}\`.\n`);
  try {
    // A launch at 23:00Z (UTC date 10-16): the UTC-date folder is the stale one.
    const early = await report(['--lane', `hx=${HX}`, '--since', E + '23:00:00Z']);
    assert.equal(early.doc.folders.tonight, 'overnight-2026-10-17');
    assert.match(early.md, /^- overnight-2026-10-16 and overnight-2026-10-17 both exist: read overnight-2026-10-17 \(its lane reports name 3 chats with a turn in the window\)$/m);
    // 02:25Z: the ET date (10-16) is the stale one.
    const usual = await report(['--lane', `hx=${HX}`]);
    assert.equal(usual.doc.folders.tonight, 'overnight-2026-10-17');
    assert.match(usual.md, /both exist: read overnight-2026-10-17/);
  } finally {
    rmSync(stale, { recursive: true, force: true });
  }
});

test('notes: one per lane for runs without a transcript; an older database without stints or delegations is noted, not thrown', async () => {
  const empty = join(dir, 'no-transcripts');
  mkdirSync(empty, { recursive: true });
  const noRoots = base.filter((a, i) => a !== '--transcripts' && base[i - 1] !== '--transcripts');
  const { doc } = await report(['--lane', `hx=${HX}`, '--transcripts', empty], noRoots);
  const of = doc.notes.filter((n) => /^lane OF: no transcript/.test(n));
  assert.deepEqual(of, ["lane OF: no transcript for 2 runs' native sessions"]);

  const old = join(dir, 'old.db');
  const d = new DatabaseSync(old);
  d.exec(schema());
  d.exec('drop table stints; drop table delegations;');
  d.prepare("insert into sessions (key, title, agent, cwd, created_at, last_activity_at) values (?,'t','claude','/hw',?,?)").run(OB, N + '02:24:30Z', N + '02:24:30Z');
  d.prepare("insert into turns (session_key, ord, run_id, started_at, ended_at, status) values (?,1,'run-ob1',?,?,'success')").run(OB, N + '02:24:30Z', N + '04:00:00Z');
  d.close();
  const r = await report(['--lane', `hx=${HX}`, '--db', old]);
  assert.equal(lane(r.doc, 'tonight', 'OB').chats, 1);
  assert.equal(lane(r.doc, 'tonight', 'OB').stints, 0);
  assert.match(r.md, /^- no stints table \(an older database\): stints are empty$/m);
  assert.match(r.md, /^- no delegations table \(an older database\): no handovers, chains are the named chats$/m);
});

test('later items fill their placeholders by dropping a JSON in the night folder; numbers only', async () => {
  writeFileSync(join(tonight, 'gate-stats.json'), JSON.stringify({ runsPerMergedPr: 1.5, merged: 12, causes: { mainMoved: 3, ownFix: 1 }, note: CANARY, ok: true }));
  try {
    const { md, doc } = await report(['--lane', `hx=${HX}`]);
    assert.match(md, /^- OF11 gate health \(gate-stats\.json\): runsPerMergedPr 1\.5, merged 12, causes\.mainMoved 3, causes\.ownFix 1, ok true$/m);
    assert.match(md, /^- OB15 envelope diff: not yet \(`envelope-diff\*\.json`\)$/m);
    assert.equal(doc.placeholders.find((p) => p.id === 'OF11').file, 'gate-stats.json');
  } finally {
    rmSync(join(tonight, 'gate-stats.json'));
  }
});

test('the product scorecard compare is labelled as not installed', async () => {
  const baseline = await summaryFromVault({ vault, dbFile: db, transcriptRoots: [transcripts], since: Date.parse(E + '21:00:00Z'), until: Date.parse(E + '23:10:00Z') });
  writeFileSync(join(tonight, 'scorecard-baseline.json'), JSON.stringify(baseline));
  try {
    const { code, out } = await run([...base, '--lane', `hx=${HX}`, '--installed-at', N + '02:22:25Z'], { measured: true });
    assert.equal(code, 0);
    assert.match(out, /^## Product scorecard against scorecard-baseline\.json \(not installed: product numbers move after install\)$/m);
    assert.match(out, /build installed at 2026-10-17T02:22:25Z/);
    assert.match(out, /Resets per 100 turns/);
  } finally {
    rmSync(join(tonight, 'scorecard-baseline.json'));
  }
});

test('bad arguments exit 2', async () => {
  assert.equal((await run(['--nope'])).code, 2);
  assert.equal((await run(['--hw', hw])).code, 2); // no --since
  assert.equal((await run([...base, '--lane', 'hx'])).code, 2);
  assert.equal((await run(['--since', 'yesterday', '--hw', hw])).code, 2);
  // A time with no zone would be read as local time: refused.
  for (const flag of ['--since', '--until', '--baseline-since', '--baseline-until']) {
    const r = await run([...base, flag, '2026-10-17T02:25']);
    assert.equal(r.code, 2, flag);
    assert.match(r.err, new RegExp(`^${flag} needs a zone \\(Z or an offset like -04:00\\)`), flag);
  }
  assert.equal((await run([...base, '--since', '2026-10-16T22:25:00-04:00'])).code, 0);
  // An empty window is refused before anything is read or written.
  const out = join(dir, 'never-written.md');
  const r = await run([...base, '--since', N + '07:00:00Z', '--until', N + '02:25:00Z', '--db', join(dir, 'no-such.db'), '--out', out]);
  assert.equal(r.code, 2);
  assert.match(r.err, /^--since must be before --until/);
  assert.ok(!existsSync(out));
  const b = await run([...base, '--baseline-since', E + '23:10:00Z', '--baseline-until', E + '21:00:00Z']);
  assert.equal(b.code, 2);
  assert.match(b.err, /^--baseline-since must be before --baseline-until/);
  // No --until: the window ends now, so a --since after now is empty too.
  assert.equal((await run(['--since', N + '08:00:00Z', '--hw', hw])).code, 2);
});
