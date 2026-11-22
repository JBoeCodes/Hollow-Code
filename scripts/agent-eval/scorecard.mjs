// The free efficiency scorecard (evening 2026-10-16, HF01). It joins what
// Hollow already records about every run and reports it per surface and
// agent, so a harness change can be measured before and after without a
// paid eval run:
//
//   node scripts/agent-eval/scorecard.mjs summary --vault <dir> [--since <iso>] [--until <iso>]
//        [--overlap] [--db <file>] [--transcripts <dir>]... [--no-transcripts] [--json <file>]
//   node scripts/agent-eval/scorecard.mjs compare <A.json> <B.json> [--json <file>]
//   node scripts/agent-eval/scorecard.mjs trace-assert <stream.jsonl>... (NV5: trace-assert.mjs)
//
// Runs are windowed by when they started; `--overlap` counts every run whose
// [started_at, at] overlaps the window instead (a running row's span is
// open-ended; an interrupted one counts only when it started inside). Either
// way the report lists the runs that started before the window and ended
// inside it or still run (`straddling`; never an interrupted one), the
// top-level runs over 30 minutes (`longRuns`, a lane's 2-hour turn), which
// stay out of the latency percentiles, and `counts.stillRunning`.
//
// Sources, all read-only:
// - the sessions database (`<support>/Sessions/<vault stem>.db`): the usage
//   ledger (outcome, tokens, cost per run, OB07's floor for a partial run,
//   and OB05's call figures out of `usage_json` through USAGE_STATS_SQL
//   only, OF28), run_queue (phase timings),
//   turns, stints (why a native session was reset) and steps (tool errors);
// - the vault's `.evidence-receipts.json` (phase timings per run, S13's
//   `memoryWrites` and S27's `verification` claim flags);
// - the vault's `.evidence-manifests/<runId>.json` (layer bytes, the
//   compiler's session state new/resume/reset, preparation time);
// - Claude transcripts (`~/.claude/projects/*/<native session>.jsonl` and
//   their `subagents/`), joined by the ledger's native session: API calls per
//   turn, context per call, cache writes, a ledger reconciliation, and the
//   tool pass (ToolSearch, always-loaded tools, first calls, waits, Bash
//   refusals and handovers); the context anatomy (OF19: what each main-thread
//   call carries, by kind, and why cache writes happen); and Workflow-tool
//   agents (`subagents/workflows/*/*.jsonl`), kept apart from the run's calls;
// - the Main agent's suggestion log (`<support>/MainAgent/<vault stem>-
//   suggestions.json`, main_agent.rs): each outcome's `at` and `outcome` only;
// - the vault's `learnings/*.md`: each note's frontmatter `type`, `trust` and
//   `learned_at` only (the first 4 KB is read; never a `restricted/` folder).
//
// It reads usage blocks, timestamps and the tool pass's whitelisted fields
// (tool names, ToolSearch `select:` names, Read/Edit/Write/MultiEdit paths
// kept in memory, Read's offset/limit, a Grep pattern classified in memory,
// is_error and a refusal class: see "The tool pass" below),
// and for the anatomy the byte size of each prompt, text, tool input, tool
// result and rendered attachment; a Bash command and a Bash result are
// classified in memory by regex. In SQL it works out two booleans over text
// that never leaves it: a step result's unattended-turn refusal and a
// Hollow-asked turn's exact `PULSE_OK` (OF23); out of a ledger row's
// `usage_json` it takes four numbers and a boolean, nothing else (USAGE_STATS_SQL,
// OF28). It writes aggregates only: counts, bytes and booleans'
// tallies; no message text, prompt, command, path inside a transcript or tool
// output ever reaches the report. Unknown cost stays null; the known part is
// reported beside it as a floor, never as a total. Percentiles are
// nearest-rank (score.mjs).
import { createHash } from 'node:crypto';
import { closeSync, createReadStream, existsSync, openSync, readFileSync, readSync, readdirSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { newcombe, percentile } from './score.mjs';

/** 2 (overnight 2026-10-17, OF12): `finished`, the `oneshot/jev` group and
 *  `unpricedByDesign`, `wallMs`, `longRuns`, `straddling` and `queue`.
 *  compare still reads a schema-1 file (its `oneshot/unknown` is Jev).
 *  Purely additive blocks keep the number (OF02's top-level `transcripts`
 *  and `delegation.handover`; OF19's anatomy keys in it and OF20's
 *  `toolErrors` and per-100-call `tools` keys): compare prints — for a side
 *  without them. OF19's 1-hour cache-write weight changes the weighted
 *  shares (`costShareAtLarge`) of transcripts that split their writes.
 *  SC4's `outcomes` (S08), `mainAgent` (OF23) and `memory` (S13) blocks are
 *  additive too, and so are OF28's `ledger` blocks, `cost.partialFloorUsd`,
 *  `latency.firstModelMs` and `toolsBeforeText` (compare reads null for a
 *  side without them), and S27's `verification.claims` per group and over
 *  the window, and R8's `transcripts.definitionLookups`. */
export const SCHEMA = 2;
export const KIND = 'hollow-scorecard';
/** A top-level run longer than this is a long run, out of latency percentiles. */
export const LONG_RUN_MS = 30 * 60_000;
/** compare marks a pair with fewer finished runs than this on either side as low-n. */
export const LOW_N = 20;
/** A call at or over this many context tokens counts as "large". */
export const LARGE_CONTEXT = 400_000;
/** Anthropic's relative token rates (input 1, cache write 1.25 for the
 *  5-minute cache and 2 for the 1-hour one, cache read 0.1, output 5): enough
 *  to split a run's cost across its calls without pricing any model. A
 *  write's 1-hour part comes from the usage block's `cache_creation` split;
 *  a write without the split counts at 1.25 (overnight 2026-10-17, OF19: all
 *  writes counted at 1.25 before, while 59.26M of 60.04M main-thread writes
 *  were 1-hour). */
const WEIGHT = { input: 1, cacheWrite: 1.25, cacheWrite1h: 2, cacheRead: 0.1, output: 5 };
const OUTCOMES = ['success', 'cancelled', 'error', 'timeout', 'interrupted'];
/** A stopped turn's next turn in the same conversation, asked by the person
 *  this soon after it ended, is a re-ask (S08). */
export const REASK_MS = 120_000;
/** The Main agent's suggestion outcomes are counted over this many days
 *  before the window's end, as main_agent.rs `stats` does before now. */
export const SUGGESTION_DAYS = 30;
/** Phase latencies per group. `firstModelMs` (OB08a: ms to the first model
 *  event) is null in a window whose receipts predate it. */
const TIMINGS = ['queueWaitMs', 'preparationMs', 'retrievalMs', 'spawnMs', 'firstModelMs', 'firstTextMs', 'turnMs', 'baselineSnapshotMs', 'finalizeDiffMs'];

const round4 = (n) => (n == null || !Number.isFinite(n) ? null : Math.round(n * 1e4) / 1e4);
const round2 = (n) => (n == null || !Number.isFinite(n) ? null : Math.round(n * 100) / 100);
const round1 = (n) => (n == null || !Number.isFinite(n) ? null : Math.round(n * 10) / 10);
const num = (v) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const at = (s) => {
  const t = Date.parse(s ?? '');
  return Number.isFinite(t) ? t : null;
};
const sum = (xs) => xs.reduce((a, b) => a + (num(b) ?? 0), 0);
/** p50/p95/n of the finite values. */
export function dist(values) {
  const v = values.filter((x) => Number.isFinite(x));
  return { n: v.length, p50: percentile(v, 0.5), p95: percentile(v, 0.95) };
}
/** A run id's start (`run-<unix ns>-…`), for manifests without a ledger row. */
export function runIdTime(runId) {
  const m = /^run-(\d{16,20})-/.exec(runId ?? '');
  return m ? Number(BigInt(m[1]) / 1_000_000n) : null;
}

// ── Locating the sources ─────────────────────────────────────────────────

/** `{folder name}-{sha256(canonical vault path)[..16]}`, as session_store.rs. */
export function vaultStem(vault) {
  const canonical = realpathSync(vault);
  const name = basename(canonical).replace(/[^\p{L}\p{N}]/gu, '-');
  return `${name}-${createHash('sha256').update(canonical).digest('hex').slice(0, 16)}`;
}

export function defaultDb(vault, support = join(homedir(), 'Library', 'Application Support', 'Hollow')) {
  return join(support, 'Sessions', `${vaultStem(vault)}.db`);
}

/** Open read-only. The live store can refuse a plain read-only open from a
 *  sandbox (CANTOPEN); `immutable=1` then reads the main file as it stands,
 *  so rows still only in a WAL are missed: the report says so. */
export async function openDb(file) {
  const { DatabaseSync } = await import('node:sqlite');
  const url = (q) => `file:${encodeURI(file).replace(/\?/g, '%3F').replace(/#/g, '%23')}?${q}`;
  const notes = [];
  try {
    const db = new DatabaseSync(url('mode=ro'), { readOnly: true });
    db.prepare('select count(*) from sqlite_master').get();
    return { db, mode: 'ro', notes };
  } catch {
    const db = new DatabaseSync(url('immutable=1'), { readOnly: true });
    const wal = `${file}-wal`;
    if (existsSync(wal) && statSync(wal).size > 0) notes.push('the database has a write-ahead log the immutable read cannot see; the newest rows may be missing');
    return { db, mode: 'immutable', notes };
  }
}

function tableColumns(db, table) {
  try {
    return new Set(db.prepare(`pragma table_info("${table}")`).all().map((c) => c.name));
  } catch {
    return new Set();
  }
}

/** Every table the scorecard reads, filtered to [since, until). Ledger rows
 *  by start, or with `overlap` by any overlap of [started_at, at]; the rows
 *  that started before `since` and ended inside (or are still running) are
 *  `straddling` either way. */
export function readDb(db, { since, until, overlap = false }) {
  const inWindow = (t) => t != null && t >= since && t < until;
  const has = (t) => tableColumns(db, t).size > 0;
  // Never read: turns.asked, run_queue.payload_json/detail, event payloads,
  // friction messages/examples or delegation packets beyond their kind, tool
  // and child session key, or a stored transcript's payload beyond
  // PULSE_OK_SQL's one boolean.
  const ledgerCols = tableColumns(db, 'usage_ledger');
  const col = (c) => (ledgerCols.has(c) ? c : `null as ${c}`);
  // OF28: `usage_json` only through USAGE_STATS_SQL (its `raw` block and
  // `note` stay in SQL), and OB07's `cost_floor_usd`; NULL on an older store.
  const stats = ledgerCols.has('usage_json') ? `${USAGE_STATS_SQL} as usage_stats` : 'null as usage_stats';
  const allLedger = ledgerCols.size
    ? db.prepare(`select run_key, at, started_at, surface, ${col('scope')}, parent_surface, run_id, agent, ${col('model')}, native_session, outcome, spawned, basis, input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status, ${col('workspace_id')}, ${col('cost_floor_usd')}, ${stats} from usage_ledger`).all()
        .map((r) => ({ ...r, usage_stats: usageStatsOf(r.usage_stats) }))
    : [];
  // A running row's `at` is its last checkpoint (written only when its
  // token totals change), not an end: it runs on, so its span is open. An
  // interrupted row's `at` is the reconciler's sweep, not when it stopped:
  // it never straddles, and overlaps only when it started inside.
  const span = (r) => {
    const s = at(r.started_at);
    return [s, r.outcome === 'running' ? Infinity : Math.max(at(r.at) ?? s, s)];
  };
  const ledger = allLedger.filter((r) => {
    const [s, e] = span(r);
    if (!overlap || r.outcome === 'interrupted') return inWindow(s);
    return s != null && s < until && e >= since;
  });
  const straddling = allLedger.filter((r) => {
    const [s, e] = span(r);
    return s != null && s < since && r.outcome !== 'interrupted' && (inWindow(e) || e === Infinity);
  });
  const queueCols = tableColumns(db, 'run_queue');
  const queue = queueCols.size
    ? db.prepare('select source, status, history_run_id, phase_timings_json from run_queue where history_run_id is not null').all()
    : [];
  // Lease waits (S14): one row per queued run with a waiting episode, by when
  // it began. An episode can hold 0 waits (claimed before a tick counted one).
  const waits = queueCols.has('wait_count')
    ? db.prepare('select waiting_since, claimed_at, last_waited_at, waiting_holder, wait_count from run_queue where waiting_since is not null').all()
        .filter((r) => inWindow(at(r.waiting_since)))
    : [];
  const sessions = has('sessions') ? db.prepare('select key, agent, cwd from sessions').all() : [];
  // All turns, so a reset's previous turn can be found across the window's
  // start; `inWindow` marks the ones counted.
  // `asked_by` (who asked: NULL the person, `agent`, or Hollow's own
  // `hollow:<kind>`, OB20) is a tag, not text.
  const turnCols = tableColumns(db, 'turns');
  const mode = turnCols.has('permission_mode') ? 'permission_mode' : 'null as permission_mode';
  const askedBy = turnCols.has('asked_by') ? 'asked_by' : 'null as asked_by';
  const allTurns = has('turns') ? db.prepare(`select session_key, ord, run_id, started_at, ended_at, status, ${mode}, ${askedBy} from turns`).all() : [];
  const turns = allTurns.filter((r) => inWindow(at(r.started_at)));
  // OF23: whether a Hollow-asked turn's stored answer is exactly PULSE_OK
  // (one boolean per tagged turn, worked out in SQL; the text never leaves
  // it). NULL when its payload isn't JSON; no row without a stored transcript.
  const pulse = turnCols.has('asked_by') && has('transcripts')
    ? db.prepare(`select t.session_key, t.ord, ${PULSE_OK_SQL} pulse_ok from turns t join transcripts tr on tr.session_key = t.session_key and tr.turn_ord = t.ord where t.asked_by like 'hollow:%'`).all()
    : [];
  // A stint's reason is why it was closed, written with its ended_at: a
  // reset happened at ended_at. Stints are counted by start, resets by end.
  const parts = tableColumns(db, 'stints').has('config_parts') ? 'config_parts' : 'null as config_parts';
  const allStints = has('stints') ? db.prepare(`select session_key, agent, started_at, ended_at, reason, ${parts} from stints order by rowid`).all() : [];
  const stints = allStints.filter((r) => inWindow(at(r.started_at)));
  const resets = allStints.filter((r) => r.reason && inWindow(at(r.ended_at)));
  // Tool steps by run, tool and status, with how many clean exits OF16's
  // step spine marked `checkFailed` (a check's failure text behind exit 0).
  const stepCols = tableColumns(db, 'steps');
  const checked = (expr, otherwise) => (stepCols.has('checkpoint') ? `case when json_valid(checkpoint) then ${expr} else ${otherwise} end` : otherwise);
  const steps = stepCols.size
    ? db.prepare(`select run_id, name, status, count(*) n, sum(${checked("json_extract(checkpoint, '$.checkFailed') = 1", '0')}) check_failed from steps where kind = 'tool' and run_id is not null group by run_id, name, status`).all()
    : [];
  // OF20: each failed tool step, reduced to STEP_ERROR_FIELDS. A shell
  // step's input (the tool input as JSON text, cut at 300 characters) is
  // read here and reduced at once to its command head and three booleans;
  // the result's text never leaves SQL (two booleans: nothing but an `Exit
  // code 1` or empty, and Codex's `Exit code 1 (no match)` prefix, OF18).
  const stepErrors = stepCols.has('checkpoint')
    ? db.prepare(`select run_id, name,
        ${checked(`case when name in (${SHELL_TOOLS.map((t) => `'${t}'`).join(', ')}) then json_extract(checkpoint, '$.input') end`, 'null')} input,
        ${checked("trim(coalesce(json_extract(checkpoint, '$.result'), ''), ' ' || char(9, 10, 13)) in ('', 'Exit code 1', 'Exit code 1 (no output)')", '0')} quiet,
        ${checked("coalesce(json_extract(checkpoint, '$.result'), '') like 'Exit code 1 (no match)%'", '0')} no_match
        from steps where kind = 'tool' and status = 'error' and run_id is not null`).all().map(stepErrorOf)
    : [];
  // OF23: failed tool steps per run whose result is the Main agent's
  // unattended-turn refusal (REFUSAL_LIKE; a count, the text stays in SQL).
  const refusals = stepCols.has('checkpoint')
    ? db.prepare(`select run_id, count(*) n from steps where kind = 'tool' and status = 'error' and run_id is not null
        and ${checked(`(${REFUSAL_LIKE.map((p) => `coalesce(json_extract(checkpoint, '$.result'), '') like '${p}'`).join(' or ')})`, '0')} group by run_id`).all()
    : [];
  const delegations = has('delegations')
    ? db.prepare("select run_id, at, json_extract(record_json, '$.kind') kind, json_extract(record_json, '$.tool') tool, json_extract(record_json, '$.childSession') child from delegations where json_valid(record_json)").all()
        .filter((r) => inWindow(at(r.at)))
    : [];
  const friction = has('friction')
    ? db.prepare('select tool, error_class, count, week_count, last_seen from friction').all().filter((r) => inWindow(at(r.last_seen)))
    : [];
  // Which of OF28's ledger sources this store has, so the summary can tell
  // "none in this window" from "not installed".
  const ledgerColumns = { usageJson: ledgerCols.has('usage_json'), costFloorUsd: ledgerCols.has('cost_floor_usd') };
  return { ledger, straddling, queue, waits, sessions, turns, allTurns, stints, allStints, resets, steps, stepErrors, refusals, pulse, delegations, friction, ledgerColumns };
}

/** The one read of a stored transcript's payload (OF23): 1 when the turn's
 *  answer (`$.text`, session_store.rs StoredTranscript) is exactly
 *  `PULSE_OK` once surrounding spaces, tabs and newlines are trimmed, 0 when
 *  not, NULL when the payload isn't JSON. Selected as that boolean only;
 *  the morning report's SQL guard allows exactly this expression and nothing
 *  else naming `payload`, and keeps its own literal copy, so changing this
 *  constant fails that test. */
export const PULSE_OK_SQL = "(case when json_valid(tr.payload) then trim(coalesce(json_extract(tr.payload, '$.text'), ''), ' ' || char(9, 10, 13)) = 'PULSE_OK' end)";
/** What every main_agent.rs `unattended::allows` refusal says (the test
 *  checks REFUSED matches): an unattended turn's limit, not an error. LIKE is
 *  case-insensitive for ASCII. */
export const REFUSAL_LIKE = ['%unattended turn%', '%unattended Main agent turn%', '%unattended COO turn%'];
/** The one read of a ledger row's `usage_json` (OF28): OB05's call figures
 *  as a small JSON object built in SQL, so the row's `raw` usage block and
 *  `note` never leave it. NULL when the text isn't JSON (the row then
 *  carries none of them); a key the row lacks is null inside it, never 0;
 *  `subagents` is 1 when the row carries a subagents object. The morning
 *  report's SQL guard allows exactly this expression beside PULSE_OK_SQL
 *  and keeps its own literal copy. */
export const USAGE_STATS_SQL = "(case when json_valid(usage_json) then json_object('apiCalls', json_extract(usage_json, '$.apiCalls'), 'peakContextTokens', json_extract(usage_json, '$.peakContextTokens'), 'subagents', json_type(usage_json, '$.subagents') = 'object', 'subagentCalls', json_extract(usage_json, '$.subagents.calls'), 'subagentTokens', json_extract(usage_json, '$.subagents.tokens')) end)";

/** USAGE_STATS_SQL's object as `{ apiCalls, peakContextTokens, subagents:
 *  { calls, tokens } | null }` (each a finite number or null), or null for
 *  a row without one or text that doesn't parse. */
export function usageStatsOf(v) {
  let o = v;
  if (typeof v === 'string') {
    try {
      o = JSON.parse(v);
    } catch {
      return null;
    }
  }
  if (!o || typeof o !== 'object' || Array.isArray(o)) return null;
  return {
    apiCalls: num(o.apiCalls),
    peakContextTokens: num(o.peakContextTokens),
    subagents: o.subagents ? { calls: num(o.subagentCalls) ?? 0, tokens: num(o.subagentTokens) ?? 0 } : null,
  };
}

// ── Shell commands, classified in memory (OF19, OF20) ────────────────────

/** friction.rs SHELL_TOOLS, plus Claude's and Codex's `Bash`. */
export const SHELL_TOOLS = ['Bash', 'bash', 'run_terminal_command'];

/** friction.rs `shell_command`: the command of a step's input (JSON text cut
 *  at 300 characters, so a long one is unterminated and read as far as it
 *  goes). */
export function shellCommandOf(input) {
  if (typeof input !== 'string') return null;
  try {
    const v = JSON.parse(input);
    return typeof v?.command === 'string' ? v.command : null;
  } catch {
    const at0 = input.indexOf('"command":"');
    if (at0 < 0) return null;
    let out = '';
    for (let i = at0 + 11; i < input.length; i++) {
      const c = input[i];
      if (c === '"') break;
      if (c === '\\') {
        const n = input[++i];
        if (n == null) break;
        out += 'ntr'.includes(n) ? ' ' : n;
      } else out += c;
    }
    return out.trim() ? out : null;
  }
}

/** run_effects.rs `unwrap_shell_wrapper`: `zsh -lc '<script>'` → the script
 *  (Codex runs every command so; a step input cut at 300 characters loses
 *  the closing quote). */
const SHELL_WRAPPER = /^\s*(?:\/bin\/)?(?:ba|z)?sh\s+-l?c\s+(['"])([\s\S]*?)(?:\1\s*)?$/;
export const unwrapShell = (command) => SHELL_WRAPPER.exec(command)?.[2] ?? command;

/** A command split outside quotes on `&&`, `||`, `;`, `|` and newlines, and
 *  whether it holds a pipe (`|`, not `||`) or an output redirect (`>`,
 *  `2>&1` included) outside quotes. */
export function shellSegments(command) {
  const segments = [];
  let cur = '';
  let quote = null;
  let pipe = false;
  let redirect = false;
  for (let i = 0; i < command.length; i++) {
    const c = command[i];
    if (quote) {
      if (c === '\\' && quote === '"') {
        cur += c + (command[++i] ?? '');
        continue;
      }
      if (c === quote) quote = null;
      cur += c;
      continue;
    }
    if (c === "'" || c === '"') quote = c;
    else if (c === '\\') {
      cur += c + (command[++i] ?? '');
      continue;
    } else if (c === '>') redirect = true;
    else if (c === '|' || c === ';' || c === '\n' || (c === '&' && command[i + 1] === '&')) {
      if (c === '|' && command[i + 1] !== '|') pipe = true;
      if ((c === '|' && command[i + 1] === '|') || c === '&') i++;
      segments.push(cur);
      cur = '';
      continue;
    }
    cur += c;
  }
  segments.push(cur);
  return { segments: segments.map((s) => s.trim()).filter(Boolean), pipe, redirect };
}

/** Launchers whose subcommand is part of a head (friction.rs LAUNCHERS). */
const LAUNCHERS = new Set(['npx', 'npm', 'pnpm', 'yarn', 'bun', 'bunx', 'cargo', 'git', 'gh', 'uv', 'uvx']);
const WRAPPERS = new Set(['env', 'time', 'sudo', 'nohup', 'exec']);
/** What a head may be (after basename and lowercase); any other first word
 *  is `other` (except `[` and `[[`), a subcommand that isn't one is dropped. */
const HEAD_WORD = /^[\w.+-]{1,32}$/;
/** Heads kept though they aren't name-shaped: `test`'s bracket forms, which
 *  PREDICATES must still see. */
const BRACKET_HEADS = new Set(['[', '[[']);
/** After friction.rs `command_head`: the first word past `NAME=value`,
 *  wrappers, `timeout N` and a leading `cd <dir> &&` / `set …;`, a script by
 *  its file name, with a launcher's subcommand (`npx vitest`, `git -C d
 *  push`). Unlike friction.rs, a first word that isn't name-shaped
 *  (HEAD_WORD: `[\w.+-]`, at most 32 characters) is `other`, and a launcher's
 *  subcommand that isn't is dropped, so a JSON blob or a long canary never
 *  becomes a head. */
export function commandHead(command) {
  const words = unwrapShell(String(command ?? '')).split(/\s+/).filter(Boolean);
  const i = headIndex(words);
  const word = words[i]?.replace(/;+$/, '');
  if (!word) return null;
  const first = word.split('/').at(-1).toLowerCase();
  if (!first) return null;
  // Only a name-shaped word is kept: a quoted JSON blob or a long canary
  // fed to `| jq` is not a command name.
  if (BRACKET_HEADS.has(first)) return first;
  if (!HEAD_WORD.test(first)) return 'other';
  const at0 = first === 'git' && words[i + 1] === '-C' ? i + 3 : i + 1;
  const sub = words[at0]?.replace(/;+$/, '').toLowerCase();
  return LAUNCHERS.has(first) && sub && /^[a-z]/.test(sub) && HEAD_WORD.test(sub) ? `${first} ${sub}` : first;
}

/** The index of a command's first word past `NAME=value`, wrappers,
 *  `timeout N` and a leading `cd <dir> &&` / `set …;`. */
function headIndex(words) {
  let i = 0;
  while (i < words.length) {
    const w = words[i];
    if (/^[A-Za-z_]\w*=/.test(w) || WRAPPERS.has(w)) i++;
    else if (w === 'timeout') i += 2;
    else if (w === 'cd' && ['&&', ';'].includes(words[i + 2])) i += 3;
    else if (w === 'cd' && words[i + 1]?.endsWith(';')) i += 2;
    else if (w === 'set' || w === 'set;') {
      // A14: Codex scripts open with `set -euo pipefail` and a newline (a
      // step input turns it into a space), not `;`: skip the flags and an
      // `-o`'s option name, up to and including a word ending in `;`.
      const start = i;
      let o = false;
      i++;
      while (!w.endsWith(';') && i < words.length) {
        const x = words[i];
        const bare = x.replace(/;+$/, '');
        if (x === '&&' || x === ';') {
          i++;
          break;
        }
        if (/^[-+][a-zA-Z]+$/.test(bare)) o = /o$/.test(bare);
        else if (o && /^[a-z]+$/.test(bare)) o = false;
        else break;
        i++;
        if (x.endsWith(';')) break;
      }
      // `set -x` and nothing after: the head is `set`.
      if (i >= words.length) {
        i = start;
        break;
      }
    } else if (['source', '.'].includes(w)) {
      const end = words.slice(i).findIndex((x) => x.endsWith(';') || x === '&&');
      if (end < 0) break;
      i += end + 1;
    } else break;
  }
  return i;
}

/** Commands whose exit 1 with no output means "no match" or "false", not a
 *  failure (OF18's list; `[` is `test`). */
const PREDICATES = new Set(['rg', 'grep', 'egrep', 'fgrep', 'test', '[', '[[', 'diff', 'cmp']);

/** What OF20 keeps of a shell command: its head and three booleans. */
export function commandShape(command) {
  if (typeof command !== 'string' || !command.trim()) return { head: null, pipe: false, redirect: false, predicate: false };
  const { segments, pipe, redirect } = shellSegments(unwrapShell(command));
  return { head: commandHead(command), pipe, redirect, predicate: PREDICATES.has(commandHead(segments.at(-1) ?? '') ?? '') };
}

/** The only fields a failed step keeps (the test checks every row). */
export const STEP_ERROR_FIELDS = ['run_id', 'name', 'head', 'pipe', 'redirect', 'predicate', 'quiet', 'noMatch'];
function stepErrorOf(r) {
  const { head, pipe, redirect, predicate } = commandShape(shellCommandOf(r.input));
  return { run_id: r.run_id, name: r.name, head, pipe, redirect, predicate, quiet: !!r.quiet, noMatch: !!r.no_match };
}

/** An error that is a command's ordinary "no" (OF20): Codex's `Exit code 1
 *  (no match)` (OF18), or a predicate (rg, grep, test, diff…) as the last
 *  segment with no output. Every other error is a failure. */
export const expectedNonzero = (e) => e.noMatch || (e.predicate && e.quiet);

/** Runs Python inline: `python3 - <<'EOF'`, `python3 -I - "$F" <<'EOF'`,
 *  `python3 -c "…"`, `python <<EOF`. A script file (`python3 x.py`) is not.
 *  (OF19's; morning-report.mjs imports it.) */
const PYTHON_INLINE = /\bpython3?(?:\.\d+)?(?:\s+-[A-Za-z]+)*\s+(?:-c\b|-(?=\s)|<<)/;
/** …and writes a file: open(…, 'w'|'a'|'x'|'r+'), .write_text(, .write_bytes(,
 *  or .write( on anything but sys.stdout/stderr. */
const PYTHON_WRITES = /\bopen\([^)\n]*,\s*['"](?:[wax]|r\+)b?\+?['"]|\.write_(?:text|bytes)\(|(?<!stdout|stderr)\.write\(/;
/** A Bash command that opens with a wait (`sleep 60`, `node -e "setTimeout…"`,
 *  a `sleep_ms` helper, a `wait*.sh`), whatever status check follows it,
 *  once SETUP_PREFIX is off. */
const POLL_COMMAND = /^\s*(?:\w+\s*\(\)\s*\{[^}]*\}\s*;?\s*)?(?:sleep\s+\d|sleep_ms\s+\d|node\s+-e\s+["']\s*setTimeout\b|\S*\bwait[-\w]*\.sh\b)/;
/** Leading segments that only set up the shell (`cd x &&`, `set -e;`,
 *  `export A=1;`, `source f;`, `PR=12;`), stripped before POLL_COMMAND. */
const SETUP_PREFIX = /^\s*(?:(?:(?:cd|pushd|set|export|source|\.)(?=\s)[^;&|\n]*|[A-Za-z_]\w*=\S*?)\s*(?:&&|;|\n)\s*)+/;
const POLL_TOOLS = /^(?:TaskOutput|BashOutput|mcp__[\w-]+__wait_for_session)$/;

/** An inline Python script that writes a file: an edit made outside Edit. */
export const isPythonEdit = (command) => PYTHON_INLINE.test(command) && PYTHON_WRITES.test(command);
export const isPoll = (name, command) => POLL_TOOLS.test(name) || (name === 'Bash' && command != null && POLL_COMMAND.test(unwrapShell(command).replace(SETUP_PREFIX, '')));
/** A call is poll-only when it used tools and every one was a wait. */
export const isPollOnly = (tools) => tools.length > 0 && tools.every((t) => t.poll);

/** A command that runs a test, build, typecheck or lint runner (or Hollow's
 *  gate): only such a command's clean exit can mask a failure. Decided from
 *  each segment's head and first arguments, never from the text anywhere in
 *  the command, so `grep -n vitest BUILD_LOG.md`, `cat tsconfig.json` and
 *  `npm run dev` are not checks (rg, grep, sed, cat, git, echo… are never
 *  runners). */
export function isCheck(command) {
  if (typeof command !== 'string') return false;
  return shellSegments(unwrapShell(command)).segments.some((s) => {
    // A subshell's or group's brackets are off its words, so
    // `(cd app && npm test)` reads `npm test` and `{ cargo test; }` `cargo test`.
    const words = s.split(/\s+/).map((w) => w.replace(/^[({]+|[)}]+$/g, '')).filter(Boolean);
    const at = headIndex(words);
    const tool = words[at]?.split('/').at(-1).toLowerCase();
    return !!tool && checkWords(tool, words.slice(at + 1));
  });
}
/** Runners that are a check whatever follows them. */
const CHECK_TOOLS = new Set(['vitest', 'jest', 'tsc', 'vue-tsc', 'eslint', 'pytest', 'cargo-nextest']);
/** Runners that are a check only with one of these subcommands. */
const CHECK_SUBS = { cargo: /^(?:test|build|check|clippy|nextest)$/, go: /^(?:test|vet|build)$/, playwright: /^test$/, vite: /^build$/, tauri: /^build$/ };
/** A package script that runs a check (`test`, `test:unit`, `gate`, `lint`…). */
const CHECK_SCRIPT = /^(?:test|t|tst|gate|lint|typecheck|type-check|check|tsc|build|e2e|smoke|verify|ci)(?:[:._-][\w:.-]*)?$/;
function checkWords(tool, args) {
  const firstArg = args.findIndex((a) => !/^[-+]/.test(a));
  const sub = firstArg < 0 ? null : args[firstArg].toLowerCase();
  const after = args.slice(firstArg + 1);
  if (CHECK_TOOLS.has(tool)) return true;
  if (Object.hasOwn(CHECK_SUBS, tool)) return !!sub && CHECK_SUBS[tool].test(sub);
  // `node --test …`: the flag before the script.
  if (tool === 'node') return args.slice(0, firstArg < 0 ? args.length : firstArg).some((a) => a === '--test' || a.startsWith('--test='));
  if (/^python[\d.]*$/.test(tool)) return args[0] === '-m' && args[1] === 'pytest';
  if (!sub) return false;
  // A package runner's binary: `npx vitest`, `bunx tsc`, `pnpm exec eslint`.
  if (['npx', 'bunx', 'pnpx', 'uvx'].includes(tool)) return checkWords(sub.split('/').at(-1), after);
  if (tool === 'npm') {
    if (sub === 'run' || sub === 'run-script') return CHECK_SCRIPT.test(after.find((a) => !a.startsWith('-')) ?? '');
    if (sub === 'exec' || sub === 'x') return !!after[0] && checkWords(after[0].split('/').at(-1).toLowerCase(), after.slice(1));
    return /^(?:test|t|tst)$/.test(sub);
  }
  if (['pnpm', 'yarn', 'bun'].includes(tool)) {
    if (['run', 'exec', 'dlx', 'x'].includes(sub)) {
      const next = after.find((a) => !a.startsWith('-'))?.toLowerCase() ?? '';
      return CHECK_SCRIPT.test(next) || (!!next && checkWords(next.split('/').at(-1), after.slice(after.findIndex((a) => !a.startsWith('-')) + 1)));
    }
    return CHECK_SCRIPT.test(sub) || checkWords(sub, after);
  }
  return false;
}

/** runner-summaries.json (OF16, shared with the Rust receipts): every
 *  runner's failure patterns, by runner id. */
const RUNNERS = JSON.parse(readFileSync(new URL('./runner-summaries.json', import.meta.url), 'utf8'));
export const RUNNER_FAILS = RUNNERS.runners.flatMap((r) => (r.fail ?? []).map((p) => [r.id, new RegExp(p)]));
const ANSI = /\u001b\[[0-9;?]*[A-Za-z]/g;
/** The first runner whose failure text a clean exit's output holds, or null
 *  (the file's `about`: strip ANSI, a newline at both ends). */
export function maskedRunner(text) {
  const t = `\n${String(text ?? '').replace(ANSI, '')}\n`;
  return RUNNER_FAILS.find(([, re]) => re.test(t))?.[0] ?? null;
}

/** Is this session the Main agent's (its cwd is the vault's `workspaces/main`)? */
export const isMainAgent = (cwd) => /[/\\]workspaces[/\\]main[/\\]?$/.test(cwd ?? '');

export function readReceipts(vault) {
  const file = join(vault, '.evidence-receipts.json');
  if (!existsSync(file)) return [];
  try {
    const doc = JSON.parse(readFileSync(file, 'utf8'));
    return Array.isArray(doc) ? doc : (doc.receipts ?? []);
  } catch {
    return null;
  }
}

/** Manifests for the given run ids (and, when `window` is set, any other
 *  manifest whose run id starts inside it). Only the fields we report. */
export function readManifests(vault, runIds, window = null) {
  const dir = join(vault, '.evidence-manifests');
  if (!existsSync(dir)) return new Map();
  const out = new Map();
  for (const f of readdirSync(dir)) {
    if (!f.endsWith('.json')) continue;
    const id = f.slice(0, -5);
    const t = runIdTime(id);
    if (!runIds.has(id) && !(window && t != null && t >= window.since && t < window.until)) continue;
    let m;
    try {
      m = JSON.parse(readFileSync(join(dir, f), 'utf8'));
    } catch {
      continue;
    }
    const stats = m.retrieval?.stats;
    const wsBudget = Array.isArray(m.compiler?.budget?.layers) ? m.compiler.budget.layers.find((l) => l?.name === 'workspace') : null;
    out.set(id, {
      layers: (m.layers ?? []).filter((l) => l && typeof l.name === 'string').map((l) => ({ name: l.name, bytes: num(l.bytes) ?? 0 })),
      session: typeof m.compiler?.session === 'string' ? m.compiler.session : null,
      entry: typeof m.compiler?.entry === 'string' ? m.compiler.entry : null,
      preparationMs: num(m.preparation?.timeline?.totalMs),
      retrievalMs: num(m.preparation?.retrievalMs),
      retrieval: stats
        ? { notesEligible: num(stats.notesEligible), notesRead: num(stats.notesRead), bytesRead: num(stats.bytesRead) }
        : null,
      // The compiler's budget cut the workspace brief (granted < wanted).
      workspaceCut: wsBudget && num(wsBudget.wanted) != null && num(wsBudget.granted) != null ? wsBudget.granted < wsBudget.wanted : null,
    });
  }
  return out;
}

/** The Main agent's suggestion log for the database `dbFile` names: it sits
 *  at `<support>/MainAgent/<stem>-suggestions.json` beside
 *  `<support>/Sessions/<stem>.db` (main_agent.rs `suggestions_path`). Null
 *  for a database outside a `Sessions` folder (a test's, a copy). */
export function suggestionsFileFor(dbFile) {
  const dir = dirname(dbFile);
  if (basename(dir) !== 'Sessions' || !dbFile.endsWith('.db')) return null;
  return join(dirname(dir), 'MainAgent', `${basename(dbFile, '.db')}-suggestions.json`);
}

/** The suggestion outcomes of the SUGGESTION_DAYS before `until`, by
 *  outcome (`done`, `not_now`, `never`; anything else `other`), as
 *  main_agent.rs `stats`: each entry's `at` and `outcome` only, never its
 *  `suggestion`. Null without a file; `{ unreadable: true }` when it
 *  doesn't parse. */
export function readSuggestions(file, until) {
  if (!file || !existsSync(file)) return null;
  let doc;
  try {
    doc = JSON.parse(readFileSync(file, 'utf8'));
  } catch {
    return { unreadable: true };
  }
  const since = until - SUGGESTION_DAYS * 86_400_000;
  const byOutcome = { done: 0, not_now: 0, never: 0, other: 0 };
  for (const o of Array.isArray(doc?.outcomes) ? doc.outcomes : []) {
    const t = at(o?.at);
    if (t == null || t < since || t >= until) continue;
    byOutcome[['done', 'not_now', 'never'].includes(o.outcome) ? o.outcome : 'other']++;
  }
  const answered = sum(Object.values(byOutcome));
  return { days: SUGGESTION_DAYS, answered, ...byOutcome, takenShare: answered ? round4(byOutcome.done / answered) : null };
}

/** A note's frontmatter keys `keys` (the block between the opening and the
 *  closing `---` in its first 4 KB; the body is never read). */
function frontmatterOf(file, keys) {
  const buf = Buffer.alloc(4096);
  const fd = openSync(file, 'r');
  let n;
  try {
    n = readSync(fd, buf, 0, buf.length, 0);
  } finally {
    closeSync(fd);
  }
  const lines = buf.toString('utf8', 0, n).split(/\r?\n/);
  if (lines[0]?.trim() !== '---') return {};
  const out = {};
  for (const line of lines.slice(1)) {
    if (line.trim() === '---') break;
    const m = /^([\w-]+):\s*(.*)$/.exec(line);
    if (m && keys.includes(m[1])) out[m[1]] = m[2].trim().replace(/^(['"])(.*)\1$/, '$2');
  }
  return out;
}

/** Learnings accepted in the window (S13): the vault's `learnings/**.md`
 *  notes an accept wrote (vault_learning.rs `write_context`: `type:
 *  learned-context`, `trust: approved`) whose `learned_at` falls inside it.
 *  An undone accept removes its note, so this counts the ones that stand.
 *  Null without a `learnings/` folder. A `restricted` folder is skipped at
 *  any depth. */
export function readLearnings(vault, { since, until }) {
  const root = join(vault, 'learnings');
  if (!existsSync(root)) return null;
  const out = { accepted: 0, approved: 0, undated: 0 };
  const walk = (dir) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) {
        if (e.name !== 'restricted' && !e.name.startsWith('.')) walk(join(dir, e.name));
        continue;
      }
      if (!e.isFile() || !e.name.endsWith('.md')) continue;
      const fm = frontmatterOf(join(dir, e.name), ['type', 'trust', 'learned_at']);
      if (fm.type !== 'learned-context' || fm.trust !== 'approved') continue;
      out.approved++;
      const t = at(fm.learned_at);
      if (t == null) out.undated++;
      else if (t >= since && t < until) out.accepted++;
    }
  };
  walk(root);
  return out;
}

// ── Transcripts (usage blocks only) ──────────────────────────────────────

/** native session id → { main: file|null, subagents: [file], workflows:
 *  [file] } under each root. Workflow-tool agents live one level deeper
 *  (`subagents/workflows/<run>/*.jsonl`, OF19) and stay out of `subagents`:
 *  the ledger doesn't meter them yet (S31), so they would break the
 *  reconciliation. */
export function indexTranscripts(roots) {
  const idx = new Map();
  const entry = (id) => idx.get(id) ?? idx.set(id, { main: null, subagents: [], workflows: [] }).get(id);
  for (const root of roots) {
    if (!existsSync(root)) continue;
    for (const project of readdirSync(root, { withFileTypes: true })) {
      if (!project.isDirectory()) continue;
      const dir = join(root, project.name);
      for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.isFile() && e.name.endsWith('.jsonl')) entry(e.name.slice(0, -6)).main ??= join(dir, e.name);
        else if (e.isDirectory()) {
          const sub = join(dir, e.name, 'subagents');
          if (existsSync(sub)) for (const s of readdirSync(sub)) if (s.endsWith('.jsonl')) entry(e.name).subagents.push(join(sub, s));
          const wf = join(sub, 'workflows');
          if (!existsSync(wf)) continue;
          for (const run of readdirSync(wf, { withFileTypes: true })) {
            if (!run.isDirectory()) continue;
            for (const s of readdirSync(join(wf, run.name))) if (s.endsWith('.jsonl')) entry(e.name).workflows.push(join(wf, run.name, s));
          }
        }
      }
    }
  }
  return idx;
}

// ── The tool pass (overnight 2026-10-17, OF02) ───────────────────────────
//
// What `readCalls({ toolPass: true })` reads, and nothing else (the test
// parses every fixture line through a guard that fails on any other read):
// - an assistant line's `message.id`, `message.usage`, `timestamp`,
//   `isSidechain`, and per content part its `type`; for a `tool_use` part
//   its `id` and `name` and these input fields by tool: ToolSearch `query`
//   (only a `select:` list's names are kept), Read, Edit, Write and
//   MultiEdit `file_path` (kept in memory to count rereads, and an edit's
//   checked against the auto-memory folders, S13; never reported), Read's
//   numbers `offset` and `limit` (R8: is it windowed?), Grep `pattern`
//   (R8: classified in memory as definition-shaped, and dropped),
//   run_action `id` (is it `sessions.wait`?) and Bash `command` (OF19/OF20:
//   classified in memory as a poll, a Python edit script or a check run;
//   R8: its grep/rg, cat and sed segments as a definition search, a whole
//   read or a windowed read, and whether the file is source; then dropped);
// - a user line's `tool_result` parts: `tool_use_id` and `is_error`; and
//   the result's text in exactly three cases: a Bash result with `is_error`
//   (its first RESULT_PREFIX characters, classified like friction.rs's
//   `error_class` and dropped), a clean Bash result whose command runs a
//   check (matched against runner-summaries.json's failure patterns: a
//   masked failure, OF20; only the runner id is kept) and a wait's result
//   without an error (wait_for_session, or run_action `sessions.wait`;
//   parsed when it is a short JSON object, keeping its boolean `finished`
//   and, when it is one of WAIT_STATUSES, its `status`).
// No text part, thinking part, prompt or other tool result is read, except
// by the anatomy (`anatomy: true`, main-thread files only), which measures
// the byte size of every prompt, text and thinking part, tool input, tool
// result and rendered attachment, and reads a system line's `subtype` (a
// compaction empties what later calls carry). Sizes only are kept.

/** The app tools Claude Code loads with the session (src-tauri/src/app_mcp.rs
 *  ALWAYS_LOAD; the test checks the two lists match). */
export const ALWAYS_LOAD = ['run_action', 'list_sessions', 'read_session', 'describe_action', 'start_session', 'send_to_session'];
/** `mcp__hollow_app__<tool>` → `<tool>`. */
export const appTool = (name) => /^mcp__hollow_app__(\w+)$/.exec(name ?? '')?.[1] ?? null;
/** How much of a failed Bash result the refusal classifier sees. */
export const RESULT_PREFIX = 200;
/** A wait result longer than this is not parsed (a wait answer is ~300 B). */
const WAIT_TEXT_MAX = 4096;
/** A `select:` entry is counted by name only when it looks like a tool
 *  name (an MCP tool or a built-in's PascalCase name); any other entry is
 *  counted as `other`, its text dropped. */
const TOOL_NAME = /^(mcp__\w+|[A-Z][A-Za-z]+)$/;
/** A still-running wait's `status`, as run_queue.rs RowStatus writes the
 *  non-terminal ones: `queued`, else `running` (`claimed` is the pump's
 *  lease on a turn that is starting). Any other value is `other`. */
const WAIT_STATUSES = { queued: 'queued', claimed: 'running', running: 'running' };

/** friction.rs `error_class`, over at most RESULT_PREFIX characters: an
 *  UPPER_SNAKE code it starts with, else the first family whose word it
 *  contains, else `error`. The families are friction.rs's (the test checks). */
export const ERROR_FAMILIES = [
  ['permission', ['permission', 'denied', 'not allowed', 'refused', "isn't allowed", 'requires approval', 'require approval', 'requires explicit approval', 'was blocked. for security', 'simple_expansion', 'changes directory before running', "haven't granted"]],
  ['timeout', ['timed out', 'timeout', 'timed_out']],
  ['conflict', ['conflict', 'no longer matches', 'already exists']],
  ['not-found', ['not found', 'no such', "doesn't exist", 'does not exist', 'unknown']],
  ['invalid', ['invalid', 'must be', 'is not a', 'malformed', 'expected']],
  ['limit', ['limit', 'quota', 'rate', 'cap reached', 'too large']],
  ['network', ['network', 'connection', 'unreachable', 'http']],
];
export function errorClass(message) {
  const first = String(message ?? '').slice(0, RESULT_PREFIX).trimStart();
  const code = /^[A-Z0-9_]*/.exec(first)[0];
  if (code.length >= 4 && code.includes('_')) return code.toLowerCase();
  const lower = first.toLowerCase();
  return ERROR_FAMILIES.find(([, words]) => words.some((w) => lower.includes(w)))?.[0] ?? 'error';
}

/** A tool result's text, as a string or its first text part. Called only for
 *  the two result kinds above. */
function resultText(content, max) {
  if (typeof content === 'string') return content.slice(0, max);
  if (!Array.isArray(content)) return '';
  const part = content.find((p) => p?.type === 'text');
  return typeof part?.text === 'string' ? part.text.slice(0, max) : '';
}

/** friction.rs `exit_code`: the code when the first non-empty line is
 *  `Exit code N` (any case). A command that ran and failed carries one; a
 *  harness or hook refusal never ran, so it carries none. */
export function exitCode(text) {
  const first = String(text ?? '').split('\n').map((l) => l.trim()).find((l) => l);
  return /^exit code \s*(\d+)$/i.exec(first ?? '')?.[1] ?? null;
}

/** A failed Bash result's class, from its first RESULT_PREFIX characters:
 *  `exit-code` when it starts with `Exit code N` (the command ran: its
 *  output saying "Permission denied" is the command's, not a refusal),
 *  else friction.rs's `error_class`. */
export const refusalClass = (content) => {
  const text = resultText(content, RESULT_PREFIX);
  return exitCode(text) != null ? 'exit-code' : errorClass(text);
};

/** A wait_for_session answer's `finished` (false: the child still runs)
 *  and, for an unfinished one, its status (WAIT_STATUSES, else `other`);
 *  null when the result is not a short JSON object carrying `finished`. */
function waitAnswer(content) {
  const text = resultText(content, WAIT_TEXT_MAX + 1);
  if (text.length > WAIT_TEXT_MAX || !text.trimStart().startsWith('{')) return null;
  try {
    const v = JSON.parse(text);
    if (typeof v?.finished !== 'boolean') return null;
    return { finished: v.finished, status: v.finished ? null : (Object.hasOwn(WAIT_STATUSES, v.status) ? WAIT_STATUSES[v.status] : 'other') };
  } catch {
    return null;
  }
}

/** A clean result's whole text (every text part): read only for a check's
 *  masked-failure match. */
function fullText(content) {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content.filter((p) => p?.type === 'text' && typeof p.text === 'string').map((p) => p.text).join('\n');
}

/** UTF-8 bytes of a prompt, text, tool input or result (an image part: 0,
 *  its tokens aren't its base64 bytes). */
export function bytesOf(v) {
  if (v == null) return 0;
  if (typeof v === 'string') return Buffer.byteLength(v);
  if (Array.isArray(v)) return sum(v.map((p) => (p?.type === 'image' ? 0 : p?.type === 'text' && typeof p.text === 'string' ? Buffer.byteLength(p.text) : bytesOf(p))));
  return Buffer.byteLength(JSON.stringify(v) ?? '');
}

/** The anatomy's kinds (`carried.byKind`): what a main-thread call carries,
 *  by where it entered the thread. A rendered attachment counts under its
 *  kind here, else `otherAttachments`. */
export const CARRIED_KINDS = ['toolResults', 'bashCommands', 'toolInputs', 'firstPrompts', 'prompts', 'compactSummary', 'instructions', 'skillListing', 'otherAttachments', 'assistantText', 'thinking'];
const ATTACHMENT_KINDS = { instructions: 'instructions', skill_listing: 'skillListing' };

/** The tools that write a file at `file_path` (run_manifest.rs
 *  `is_memory_write`, the receipt's `memoryWrites`). NotebookEdit takes a
 *  `notebook_path` and never writes a memory note: not read. */
const WRITE_TOOLS = new Set(['Write', 'Edit', 'MultiEdit']);

/** Is `path` inside an auto-memory folder: `<root>/<project slug>/memory/…`
 *  under one of `roots` (the transcript roots, `~/.claude/projects`)? An
 *  absolute path with no `..` only, as run_manifest.rs reads it when the
 *  folder can't be resolved. Any project's folder counts: Claude Code keys
 *  memory by the repo root, so a worktree's session writes under another
 *  slug than its transcript's (S13). */
export function isMemoryPath(path, roots) {
  if (typeof path !== 'string' || !path.startsWith('/') || path.split('/').includes('..')) return false;
  return roots.some((r) => {
    const root = String(r).replace(/\/+$/, '');
    if (!root || !path.startsWith(`${root}/`)) return false;
    const rest = path.slice(root.length + 1).split('/');
    return rest.length >= 3 && rest[0] !== '' && rest[1] === 'memory' && rest.at(-1) !== '';
  });
}

/** R8: a definition-shaped search pattern (`fn foo`, `class\s+Bar`). */
export const DEF_PATTERN = /\b(fn|function|class|struct|enum|trait|interface|type|impl|def|const)(\s|\\s[+*]?)+\\?\w/;
/** R8: a source file, by its extension. */
export const SOURCE_FILE = /\.(?:rs|ts|tsx|js|mjs|cjs|py|go|swift|kt|java)$/i;
const SEARCH_TOOLS = new Set(['grep', 'egrep', 'fgrep', 'rg']);
/** `sed -n`'s line-range print: `120,180p`, `120p`, `120,$p`. */
const SED_RANGE = /^\d+(?:,(?:\d+|\$))?p$/;
const unquote = (w) => w.replace(/^['"]+|['"]+$/g, '');
/** grep/rg flags whose next word is their value, not a pattern. */
const VALUE_FLAGS = new Set(['-t', '-T', '--type', '--type-not', '-g', '--glob', '--iglob', '--include', '--exclude', '--exclude-dir', '-m', '--max-count', '-A', '-B', '-C']);
/** A grep/rg's words without its flags and their values: `rg --type rust
 *  foo` searches `foo`, not `type rust`. */
function searchArgs(args) {
  const out = [];
  for (let k = 0; k < args.length; k++) {
    if (VALUE_FLAGS.has(args[k])) k++;
    else if (!args[k].startsWith('-')) out.push(args[k]);
  }
  return out;
}

/** R8: a Bash command's definition lookups, classified in memory from its
 *  segments (a leading `cd x &&` and other setup skipped): `defSearch`, a
 *  grep/rg segment whose arguments (flags and their values left out,
 *  searchArgs) hold a DEF_PATTERN match; `read`, a
 *  `cat <file>…` in a command with no pipe (`whole`) or a `sed -n 'A,Bp'
 *  <file>` (`windowed`); `source`, whether that read's file is source
 *  (SOURCE_FILE; a whole source read wins over a windowed one, and source
 *  over other). Only these three are kept. */
export function shellLookup(command) {
  const out = { defSearch: false, read: null, source: false };
  if (typeof command !== 'string') return out;
  const { segments, pipe } = shellSegments(unwrapShell(command));
  const reads = [];
  for (const s of segments) {
    const words = s.split(/\s+/).map((w) => w.replace(/^[({]+|[)}]+$/g, '')).filter(Boolean);
    const at0 = headIndex(words);
    const tool = words[at0]?.split('/').at(-1).toLowerCase();
    const args = words.slice(at0 + 1);
    if (SEARCH_TOOLS.has(tool)) out.defSearch ||= DEF_PATTERN.test(searchArgs(args).join(' '));
    else if (tool === 'cat' && !pipe) {
      const files = [];
      for (const a of args) {
        // A redirect or heredoc ends the files (`cat <<EOF > f` reads none).
        if (/^\d*[<>&]/.test(a)) break;
        if (!a.startsWith('-')) files.push(unquote(a));
      }
      if (files.length) reads.push({ read: 'whole', source: files.some((f) => SOURCE_FILE.test(f)) });
    } else if (tool === 'sed' && args.includes('-n')) {
      const i = args.findIndex((a) => SED_RANGE.test(unquote(a)));
      const files = i < 0 ? [] : args.slice(i + 1).filter((a) => !a.startsWith('-') && !/^\d*[<>&]/.test(a)).map(unquote);
      if (files.length) reads.push({ read: 'windowed', source: files.some((f) => SOURCE_FILE.test(f)) });
    }
  }
  const rank = (r) => (r.source ? 2 : 0) + (r.read === 'whole' ? 1 : 0);
  const best = reads.sort((a, b) => rank(b) - rank(a))[0];
  if (best) Object.assign(out, best);
  return out;
}

/** What the tool pass keeps of one tool use (its name, its input fields'
 *  classes and a Bash command's). R8: `defSearch` (a Grep or a grep/rg
 *  command with a DEF_PATTERN pattern), `read` (`whole` or `windowed`: a
 *  Read without or with `offset`/`limit`, or shellLookup's) and `source`. */
function toolUseOf(name, input, memoryRoots = []) {
  const use = { name, search: null, select: [], file: null, memoryWrite: false, wait: false, error: null, refusal: null, waitFinished: null, waitStatus: null, poll: false, pythonEdit: false, check: false, masked: null, resultBytes: null, defSearch: false, read: null, source: false };
  if (name === 'ToolSearch') {
    const q = typeof input?.query === 'string' ? input.query : '';
    use.search = q.startsWith('select:') ? 'select' : 'keyword';
    if (use.search === 'select') use.select = q.slice(7).split(',').map((n) => n.trim()).filter(Boolean).map((n) => (TOOL_NAME.test(n) ? n : 'other'));
  } else if (name === 'Read' || WRITE_TOOLS.has(name)) {
    use.file = typeof input?.file_path === 'string' ? input.file_path : null;
    use.memoryWrite = WRITE_TOOLS.has(name) && isMemoryPath(use.file, memoryRoots);
    if (name === 'Read') {
      use.read = input?.offset != null || input?.limit != null ? 'windowed' : 'whole';
      use.source = use.file != null && SOURCE_FILE.test(use.file);
    }
  } else if (name === 'Grep') use.defSearch = typeof input?.pattern === 'string' && DEF_PATTERN.test(input.pattern);
  else if (appTool(name) === 'wait_for_session') use.wait = true;
  else if (appTool(name) === 'run_action') use.wait = input?.id === 'sessions.wait';
  else if (name === 'Bash' && typeof input?.command === 'string') {
    const command = input.command;
    use.poll = isPoll(name, command);
    use.pythonEdit = isPythonEdit(command);
    use.check = isCheck(command);
    Object.assign(use, shellLookup(command));
    return use;
  }
  // A wait through run_action `sessions.wait` is a poll too.
  use.poll = use.wait || isPoll(name, null);
  return use;
}

/** One API call per assistant message id: its usage counters and time.
 *  Lines without a usage block are skipped before parsing. With `toolUse`
 *  (morning-report.mjs), each call also gets `tools`: what
 *  `toolUse(name, command)` returns for each of its tool uses, where
 *  `command` is a Bash call's command and null for every other tool. Only
 *  that callback's return value is kept. With `toolPass` (the summary),
 *  each call gets `uses` (toolUseOf, above), and the user lines carrying a
 *  `tool_result` are parsed too, to mark each use's error, refusal class
 *  and wait outcome. With `anatomy` (the summary, main-thread files only)
 *  every line is parsed and each call gets `carried`: the bytes by kind
 *  (CARRIED_KINDS, plus `tool:<name>` per tool's results and
 *  `pythonEditScripts`, a part of `bashCommands`) of everything that
 *  entered the thread before it, since the last compaction; each use gets
 *  `resultBytes`. `memoryRoots` (the tool pass, S13): the transcript roots
 *  whose `<slug>/memory/` folders mark a Write/Edit/MultiEdit use as a
 *  `memoryWrite`. `parse` is JSON.parse; the test wraps it to watch every
 *  field read. */
export async function readCalls(file, sidechain, { toolUse = null, toolPass = false, anatomy = false, memoryRoots = [], parse = JSON.parse } = {}) {
  const calls = new Map();
  const uses = new Map();
  // The anatomy's running sums: bytes by kind in the thread so far.
  let carry = {};
  // R8: one object per stretch of the thread between compactions; each call
  // keeps its stretch's (non-enumerable `segment`), so a result is carried
  // only by later calls that share it.
  let segment = {};
  let prompted = false;
  const add = (k, n) => {
    if (n) carry[k] = (carry[k] ?? 0) + n;
  };
  const rl = createInterface({ input: createReadStream(file, 'utf8'), crlfDelay: Infinity });
  for await (const line of rl) {
    const usage = line.includes('"usage"') && line.includes('"assistant"');
    if (!anatomy && !usage && !(toolPass && line.includes('"tool_result"'))) continue;
    let e;
    try {
      e = parse(line);
    } catch {
      continue;
    }
    if (anatomy && e?.type === 'system') {
      // A compaction replaces the thread with its summary (the next prompt).
      if (e.subtype === 'compact_boundary') {
        carry = {};
        segment = {};
      }
      continue;
    }
    if (anatomy && e?.type === 'attachment') {
      // Only a rendered attachment reaches the model.
      if ((sidechain || !e.isSidechain) && e.rendered != null) add(ATTACHMENT_KINDS[e.attachment?.type] ?? 'otherAttachments', bytesOf(e.rendered));
      continue;
    }
    if ((toolPass || anatomy) && e?.type === 'user') {
      if (!sidechain && e.isSidechain) continue;
      const parts = e.message?.content;
      // A prompt's content is a string: only the anatomy measures it. The
      // thread's first prompt that isn't a meta message is `firstPrompts`.
      let kind = null;
      const promptKind = () => (kind ??= e.isCompactSummary ? 'compactSummary' : !e.isMeta && !prompted ? ((prompted = true), 'firstPrompts') : 'prompts');
      if (anatomy && typeof parts === 'string') add(promptKind(), bytesOf(parts));
      for (const p of Array.isArray(parts) ? parts : []) {
        if (anatomy && p?.type === 'text') add(promptKind(), bytesOf(p.text));
        if (p?.type !== 'tool_result') continue;
        const use = uses.get(p.tool_use_id);
        if (anatomy) {
          const b = bytesOf(p.content);
          add('toolResults', b);
          add(`tool:${use?.name ?? 'unknown'}`, b);
          if (use) use.resultBytes = b;
        }
        if (!toolPass || !use) continue;
        use.error = p.is_error === true;
        if (use.error && use.name === 'Bash') use.refusal = refusalClass(p.content);
        else if (!use.error && use.check) use.masked = maskedRunner(fullText(p.content));
        else if (!use.error && use.wait) {
          const a = waitAnswer(p.content);
          use.waitFinished = a?.finished ?? null;
          use.waitStatus = a?.status ?? null;
        }
      }
      continue;
    }
    if (e?.type !== 'assistant' || (!sidechain && e.isSidechain)) continue;
    const u = e.message?.usage;
    const id = e.message?.id ?? e.uuid;
    const t = at(e.timestamp);
    if (!u || !id || t == null) continue;
    // Claude Code writes one line per content block, each repeating the
    // message's usage snapshot, which only grows: keep each field's max.
    const prev = calls.get(id);
    const max = (k, v) => Math.max(prev?.[k] ?? 0, num(v) ?? 0);
    const next = {
      t: prev?.t ?? t,
      input: max('input', u.input_tokens),
      cacheWrite: max('cacheWrite', u.cache_creation_input_tokens),
      // The write's 1-hour part (OF19's weight), when the usage splits it.
      cacheWrite1h: max('cacheWrite1h', u.cache_creation?.ephemeral_1h_input_tokens),
      cacheRead: max('cacheRead', u.cache_read_input_tokens),
      output: max('output', u.output_tokens),
      sidechain: !!sidechain,
    };
    // What the call carried: the thread before its first line.
    if (anatomy) {
      next.carried = prev?.carried ?? { ...carry };
      Object.defineProperty(next, 'segment', { value: prev?.segment ?? segment, enumerable: false });
    }
    // The parts are looked at only by a pass that needs them (my-context's
    // usage-only read never touches `message.content`).
    const parts = (toolUse || toolPass || anatomy) && Array.isArray(e.message?.content) ? e.message.content : [];
    if (anatomy) {
      for (const b of parts) {
        if (b?.type === 'text') add('assistantText', bytesOf(b.text));
        else if (b?.type === 'thinking') add('thinking', bytesOf(b.thinking));
      }
    }
    if (toolUse || toolPass || anatomy) {
      // One line per content block: a tool use is kept once, by its id.
      const seen = prev?.toolIds ?? new Set();
      if (toolUse) next.tools = prev?.tools ?? [];
      if (toolPass) next.uses = prev?.uses ?? [];
      for (const b of parts) {
        if (b?.type !== 'tool_use' || seen.has(b.id)) continue;
        seen.add(b.id);
        const name = b.name ?? 'unknown';
        const command = name === 'Bash' && typeof b.input?.command === 'string' ? b.input.command : null;
        if (toolUse) next.tools.push(toolUse(name, command));
        let use = null;
        if (toolPass) {
          use = toolUseOf(name, b.input, memoryRoots);
          uses.set(b.id, use);
          next.uses.push(use);
        }
        if (anatomy) {
          // A tool input is carried as the JSON the model wrote (a Bash
          // command with its escapes and description).
          const b0 = bytesOf(b.input);
          if (command == null) add('toolInputs', b0);
          else {
            add('bashCommands', b0);
            if (use?.pythonEdit ?? isPythonEdit(command)) add('pythonEditScripts', b0);
          }
          // A result answers a use the anatomy can name without the tool pass.
          if (!use) uses.set(b.id, { name });
        }
      }
      Object.defineProperty(next, 'toolIds', { value: seen, enumerable: false });
    }
    calls.set(id, next);
  }
  return [...calls.values()];
}

/** A call's context (input + cache write + cache read) and its weight (a
 *  write's 1-hour part at WEIGHT.cacheWrite1h, the rest at cacheWrite). */
export const context = (c) => c.input + c.cacheWrite + c.cacheRead;
export const weight = (c) => {
  const oneHour = Math.min(c.cacheWrite1h ?? 0, c.cacheWrite);
  return c.input * WEIGHT.input + (c.cacheWrite - oneHour) * WEIGHT.cacheWrite + oneHour * WEIGHT.cacheWrite1h + c.cacheRead * WEIGHT.cacheRead + c.output * WEIGHT.output;
};

// ── The Codex pass (overnight 9 A14) ─────────────────────────────────────
//
// Codex keeps no Claude-style transcript; its rollout
// (`$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<time>-<thread id>.jsonl`,
// codex_rollout.rs) is joined by the ledger's native session (the thread
// id). Only three line kinds are parsed, found by the top-level `type` in a
// line's head: `event_msg` naming `token_count` (one per model call:
// `info.last_token_usage`'s input, cached and output numbers, and
// `total_token_usage.total_tokens` to drop a repeat), `compacted` (counted),
// and a `response_item` whose payload is a tool call (its `name` only, when
// name-shaped). A message, reasoning or tool output line is never parsed,
// and no argument, text or path is kept.

/** A rollout's thread id: the UUID its file name ends with. */
const CODEX_THREAD = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$/;
/** How far into a line its top-level `type` (and a payload's) must appear. */
const CODEX_HEAD = 320;
const CODEX_TOOL_ITEM = /"payload":\{"type":"(function_call|custom_tool_call|local_shell_call)"/;
/** A tool name is kept only when it is name-shaped. */
const CODEX_TOOL_NAME = /^[\w.:-]{1,64}$/;

/** thread id → rollout file, under `<home>/sessions/YYYY/MM/DD/`. */
export function indexCodexRollouts(home) {
  const idx = new Map();
  const walk = (dir, depth) => {
    let entries = [];
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of entries) {
      const p = join(dir, e.name);
      if (e.isDirectory() && depth < 3) walk(p, depth + 1);
      else if (e.isFile()) {
        const m = CODEX_THREAD.exec(e.name);
        if (m && !idx.has(m[1])) idx.set(m[1], p);
      }
    }
  };
  if (home) walk(join(home, 'sessions'), 0);
  return idx;
}

/** A rollout line's top-level `type`, from its head only. */
function codexLineType(line) {
  return /^\{(?:"[a-z_]+":(?:"[^"]*"|\d+),)*"type":"([a-z_]+)"/.exec(line.slice(0, CODEX_HEAD))?.[1] ?? null;
}

/** One rollout's calls (Claude-shaped, so the transcript blocks read them:
 *  `input` is uncached input, `cacheRead` the cached part, no cache writes),
 *  compaction times and tool calls by name. See the section comment. */
export async function readCodexCalls(file, { parse = JSON.parse } = {}) {
  const calls = [];
  const compactions = [];
  const tools = [];
  let lastTotal = null;
  const rl = createInterface({ input: createReadStream(file, 'utf8'), crlfDelay: Infinity });
  for await (const line of rl) {
    const type = codexLineType(line);
    const head = line.slice(0, CODEX_HEAD);
    if (type === 'event_msg' ? !head.includes('"type":"token_count"') : type === 'response_item' ? !CODEX_TOOL_ITEM.test(head) : type !== 'compacted') continue;
    let e;
    try {
      e = parse(line);
    } catch {
      continue;
    }
    const t = at(e?.timestamp);
    const p = e?.payload;
    if (t == null || !p || typeof p !== 'object') continue;
    if (e.type === 'compacted') compactions.push(t);
    else if (e.type === 'response_item' && ['function_call', 'custom_tool_call', 'local_shell_call'].includes(p.type)) {
      const name = p.type === 'local_shell_call' ? 'local_shell' : p.name;
      tools.push({ t, name: typeof name === 'string' && CODEX_TOOL_NAME.test(name) ? name : 'other' });
    } else if (e.type === 'event_msg' && p.type === 'token_count') {
      const last = p.info?.last_token_usage;
      const total = p.info?.total_token_usage?.total_tokens ?? null;
      if (!last || !Number.isFinite(last.input_tokens) || !Number.isFinite(last.output_tokens)) continue;
      // The same totals again (a rate-limit update) is not a new call.
      if (total != null && total === lastTotal) continue;
      lastTotal = total;
      const cached = Number.isFinite(last.cached_input_tokens) ? Math.min(last.cached_input_tokens, last.input_tokens) : 0;
      calls.push({ t, sidechain: false, input: last.input_tokens - cached, cacheWrite: 0, cacheRead: cached, output: last.output_tokens, uses: [] });
    }
  }
  return { calls, compactions, tools };
}

/** A group's Codex extras: compactions and tool calls by name, over its
 *  runs joined to a rollout (`codex`: run key → { compactions, tools }). */
export function codexBlock(runs, codex) {
  const joined = runs.filter((r) => codex.has(r.row.run_key));
  if (!joined.length) return null;
  const tools = {};
  for (const r of joined) for (const n of codex.get(r.row.run_key).tools) tools[n] = (tools[n] ?? 0) + 1;
  const compactions = joined.map((r) => codex.get(r.row.run_key).compactions);
  return {
    joinedRuns: joined.length,
    compactions: sum(compactions),
    compactedRuns: compactions.filter((n) => n > 0).length,
    toolCalls: Object.fromEntries(Object.entries(tools).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))),
  };
}

// ── Summary ──────────────────────────────────────────────────────────────

/** A Jev request: jev.rs meters each as `oneshot` with scope and model
 *  `jev` and no agent. Jev reports no cost, by design. */
const isJev = (row) => !row.agent && (row.scope === 'jev' || row.model === 'jev');
const groupKey = (row) => `${row.surface ?? 'unknown'}/${row.agent ?? (isJev(row) ? 'jev' : 'unknown')}`;

function timingsOf(receipt, queueRow) {
  if (receipt?.phaseTimings) return receipt.phaseTimings;
  if (queueRow?.phase_timings_json) {
    try {
      return JSON.parse(queueRow.phase_timings_json);
    } catch {
      return null;
    }
  }
  return null;
}

/** Cost statuses whose cost is the run's whole cost. A `partial` row can
 *  carry a cost (the budget guard's estimate at a kill): it stays apart. */
const KNOWN_COST = new Set(['reported', 'estimated']);

export function costBlock(rows) {
  const isKnown = (r) => KNOWN_COST.has(r.cost_status) && num(r.cost_usd) != null;
  const known = rows.filter(isKnown).map((r) => r.cost_usd);
  // Jev requests carry no cost by design: counted apart, never as unknown,
  // and never as $0 (the total stays null while any is in the block).
  const unpriced = rows.filter((r) => !isKnown(r) && isJev(r));
  const unknown = rows.filter((r) => !isKnown(r) && !isJev(r));
  const tokensOf = (r) => (num(r.input_tokens) ?? 0) + (num(r.cached_input_tokens) ?? 0) + (num(r.output_tokens) ?? 0);
  const byStatus = {};
  for (const r of unknown) byStatus[r.cost_status ?? 'none'] = (byStatus[r.cost_status ?? 'none'] ?? 0) + 1;
  // OB07's priced floor on a run without a known cost: never known cost.
  const floors = rows.filter((r) => !isKnown(r) && num(r.cost_floor_usd) != null).map((r) => r.cost_floor_usd);
  const d = dist(known);
  return {
    knownRuns: known.length,
    unknownRuns: unknown.length,
    unknownByStatus: byStatus,
    unpricedByDesign: unpriced.length,
    // A total only when every run has a cost; otherwise null, with the
    // known part as a floor (never read a floor as a total).
    totalUsd: unknown.length || unpriced.length ? null : round4(sum(known)),
    floorUsd: round4(sum(known)),
    p50Usd: round4(d.p50),
    p95Usd: round4(d.p95),
    // Token volume on runs without a known cost: what the unknown part covers.
    tokensWithoutCost: sum(unknown.map(tokensOf)),
    // Costs carried by partial rows: a lower bound on part of the unknown.
    partialCostUsd: round4(sum(unknown.map((r) => r.cost_usd))),
    // OB07's `cost_floor_usd` summed over the runs that carry one (n:
    // `partialFloorRuns`), kept out of floorUsd and totalUsd; null when no
    // run carries one (a store before OB07 included).
    partialFloorUsd: floors.length ? round4(sum(floors)) : null,
    partialFloorRuns: floors.length,
  };
}

/** Surfaces whose rows are a helper call made for a run, never a run of
 *  their own (budgets.rs): a title or extraction `oneshot`, an `ingest`, an
 *  ask_agent `delegate` (which may carry no parent surface). */
const HELPER_SURFACES = new Set(['oneshot', 'ingest', 'delegate']);
/** A finished top-level run (OF28 review): no parent surface, not a helper
 *  call, and not `running` (a running row's figures are its last
 *  checkpoint's). The window-wide ledger block pools only these, so a
 *  child's or a checkpoint's peak never sets the top-level percentiles. */
export const isFinishedTopLevel = (r) => !r.parent_surface && !HELPER_SURFACES.has(r.surface) && r.outcome !== 'running';

/** OB05's call figures per run, out of the ledger's `usage_json` (OF28),
 *  each over the rows that carry it: `peakContext` (the largest prompt one
 *  main-thread message sent) and `apiCalls` as p50/p95/n, `subagents` (the
 *  rows with a subagents object, their calls and tokens). Null, never 0,
 *  when no row carries it (Codex and Antigravity rows, rows before OB05);
 *  `runs` is the n they are out of. */
export function ledgerBlock(rows) {
  const stats = rows.map((r) => (typeof r.usage_stats === 'string' ? usageStatsOf(r.usage_stats) : r.usage_stats)).filter(Boolean);
  const carried = (xs) => {
    const d = dist(xs);
    return d.n ? d : null;
  };
  const sub = stats.filter((u) => u.subagents);
  return {
    runs: rows.length,
    peakContext: carried(stats.map((u) => u.peakContextTokens)),
    apiCalls: carried(stats.map((u) => u.apiCalls)),
    subagents: sub.length ? { runs: sub.length, calls: sum(sub.map((u) => u.subagents.calls)), tokens: sum(sub.map((u) => u.subagents.tokens)) } : null,
  };
}

/** Transcript usage over `runs` ({ row, calls }[]; morning-report.mjs too). */
export function transcriptBlock(runs) {
  const joined = runs.filter((r) => r.calls);
  if (!joined.length) return null;
  const calls = joined.flatMap((r) => r.calls);
  const main = calls.filter((c) => !c.sidechain);
  const all = (k) => sum(calls.map((c) => c[k]));
  const w = sum(calls.map(weight));
  const wLarge = sum(calls.filter((c) => context(c) >= LARGE_CONTEXT).map(weight));
  const inputSide = all('input') + all('cacheWrite') + all('cacheRead');
  const ledgerTokens = sum(joined.map((r) => (num(r.row.input_tokens) ?? 0) + (num(r.row.cached_input_tokens) ?? 0) + (num(r.row.output_tokens) ?? 0)));
  const transcriptTokens = inputSide + all('output');
  // Calls per turn count subagent calls too; context per call is the main
  // thread's (a subagent's context is its own).
  const perTurn = dist(joined.map((r) => r.calls.length));
  const mainPerTurn = dist(joined.map((r) => r.calls.filter((c) => !c.sidechain).length));
  const ctx = dist(main.map(context));
  return {
    joinedRuns: joined.length,
    calls: calls.length,
    subagentCalls: calls.length - main.length,
    callsPerTurn: perTurn,
    mainCallsPerTurn: mainPerTurn,
    contextPerCall: ctx,
    cacheWriteTokens: all('cacheWrite'),
    cacheReadTokens: all('cacheRead'),
    cacheReadShare: inputSide ? round4(all('cacheRead') / inputSide) : null,
    largeContextCalls: calls.filter((c) => context(c) >= LARGE_CONTEXT).length,
    costShareAtLarge: w ? round4(wLarge / w) : null,
    reconciliation: {
      ledgerTokens,
      transcriptTokens,
      ratio: ledgerTokens ? round4(transcriptTokens / ledgerTokens) : null,
    },
  };
}

function retrievalBlock(runs) {
  const r = runs.map((x) => x.manifest?.retrieval).filter(Boolean);
  if (!r.length) return null;
  return {
    runs: r.length,
    notesRead: dist(r.map((x) => x.notesRead)),
    bytesRead: dist(r.map((x) => x.bytesRead)),
    // The share of eligible notes the prefilter let through to be read.
    prefilterPassRate: dist(r.filter((x) => x.notesEligible).map((x) => round4(x.notesRead / x.notesEligible))),
  };
}

/** Per 100, two decimals (null over 0). */
const per100 = (k, n) => (n ? round2((k * 100) / n) : null);

/** Masked failures over these calls' tool uses (OF20): clean Bash results
 *  whose command runs a check, and those whose output holds a runner's
 *  failure text, by runner id. Null when no call carries tool uses. */
export function maskedBlock(calls) {
  const withUses = calls.filter((c) => c.uses);
  if (!withUses.length) return null;
  const checks = withUses.flatMap((c) => c.uses).filter((u) => u.name === 'Bash' && u.error === false && u.check);
  const byRunner = {};
  for (const u of checks) if (u.masked) byRunner[u.masked] = (byRunner[u.masked] ?? 0) + 1;
  return { cleanCheckResults: checks.length, masked: sum(Object.values(byRunner)), byRunner: Object.fromEntries(Object.entries(byRunner).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))) };
}

/** `verification.claims` (S27) over receipts' `verification` blocks: of the
 *  `n` that carry a `claim`, how many final replies claimed a pass
 *  (`claims`), how many of those had no recognised check after the run's
 *  last write (`claimWithoutCheck`) or ended on a failed check
 *  (`claimAfterFail`), each also per 100 of the n runs (`per100Runs`).
 *  Null when none carries a claim (receipts from before S27, surfaces with
 *  no reply): not installed, never 0. */
export function claimsBlock(verifications) {
  const carrying = verifications.filter((v) => typeof v?.claim === 'string');
  if (!carrying.length) return null;
  const passed = carrying.filter((v) => v.claim === 'passed');
  const claims = passed.length;
  const claimWithoutCheck = passed.filter((v) => v.claimWithoutCheck === true).length;
  const claimAfterFail = passed.filter((v) => v.claimAfterFail === true).length;
  const n = carrying.length;
  return { n, claims, claimWithoutCheck, claimAfterFail, per100Runs: { claims: per100(claims, n), claimWithoutCheck: per100(claimWithoutCheck, n), claimAfterFail: per100(claimAfterFail, n) } };
}

/** A group's runs' receipt `verification` blocks, one per run id (S27
 *  review): a turn that failed over has a ledger row per attempt, each
 *  joined to the run's one receipt, and counts once. */
function verificationsOncePerRun(runs) {
  const byRun = new Map();
  for (const r of runs) if (r.verification && !byRun.has(r.row.run_id)) byRun.set(r.row.run_id, r.verification);
  return [...byRun.values()];
}

/** The scorecard over already-read sources (pure; the tests call this).
 *  `workflows`: native session → its Workflow-tool agents' calls in the
 *  window ({ file, calls }[]). `suggestions`: readSuggestions; `learnings`:
 *  readLearnings (either null when absent). */
export function summarize({ db, receipts, manifests, transcripts = new Map(), workflows = new Map(), codex = new Map(), window, notes = [], suggestions = null, learnings = null }) {
  const receiptByRun = new Map(receipts.map((r) => [r.runId ?? r.historyRunId, r]));
  const queueByRun = new Map(db.queue.map((q) => [q.history_run_id, q]));
  const toolByRun = new Map();
  const toolOf = (runId) => toolByRun.get(runId) ?? toolByRun.set(runId, { calls: 0, errors: 0, expectedNonzero: 0, checkFailed: 0, byTool: {} }).get(runId);
  for (const s of db.steps) {
    const t = toolOf(s.run_id);
    t.calls += s.n;
    t.checkFailed += num(s.check_failed) ?? 0;
    if (s.status === 'error') {
      t.errors += s.n;
      t.byTool[s.name ?? 'unknown'] = (t.byTool[s.name ?? 'unknown'] ?? 0) + s.n;
    }
  }
  for (const e of db.stepErrors ?? []) if (expectedNonzero(e) && toolByRun.has(e.run_id)) toolOf(e.run_id).expectedNonzero++;

  // One unit per ledger row. Timings, manifest and tools belong to the
  // queued run itself: a delegate or oneshot row (a title, an extraction)
  // can carry the same run id, so they attach only to the top-level row
  // whose surface is the queue row's or receipt's source.
  const runs = db.ledger.map((row) => {
    const receipt0 = receiptByRun.get(row.run_id);
    const queue0 = queueByRun.get(row.run_id);
    const source = receipt0?.source ?? queue0?.source ?? null;
    const top = !row.parent_surface && source != null && row.surface === source;
    const receipt = top ? receipt0 : null;
    const timings = top ? timingsOf(receipt, queue0) : null;
    const manifest = top ? manifests.get(row.run_id) : null;
    // The ledger's start → end. A running row has no end yet, and an
    // interrupted one's `at` is when the reconciler noticed.
    const ended = row.outcome !== 'running' && row.outcome !== 'interrupted';
    const wallMs = ended && at(row.at) != null && at(row.started_at) != null ? at(row.at) - at(row.started_at) : null;
    return {
      row,
      key: groupKey(row),
      wallMs,
      long: !row.parent_surface && wallMs != null && wallMs > LONG_RUN_MS,
      timings: timings || manifest
        ? {
            queueWaitMs: num(timings?.queueWaitMs),
            spawnMs: num(timings?.spawnMs),
            firstModelMs: num(timings?.firstModelMs),
            firstTextMs: num(timings?.firstTextMs),
            toolsBeforeText: num(timings?.toolsBeforeText),
            turnMs: num(timings?.turnMs),
            baselineSnapshotMs: num(timings?.baselineSnapshotMs),
            finalizeDiffMs: num(timings?.finalizeDiffMs),
            preparationMs: manifest?.preparationMs ?? null,
            retrievalMs: manifest?.retrievalMs ?? null,
          }
        : null,
      manifest,
      tools: top ? toolByRun.get(row.run_id) ?? null : null,
      calls: transcripts.get(row.run_key) ?? null,
      // S27's claim flags, from the run's own receipt.
      verification: receipt?.verification ?? null,
    };
  });

  const groups = {};
  for (const key of [...new Set(runs.map((r) => r.key))].sort()) {
    const g = runs.filter((r) => r.key === key);
    const outcomes = {};
    for (const r of g) outcomes[r.row.outcome] = (outcomes[r.row.outcome] ?? 0) + 1;
    // Rates over finished runs: a row still running has no outcome yet.
    const finished = g.filter((r) => r.row.outcome !== 'running').length;
    const rate = (n) => (finished ? round4(n / finished) : null);
    const rates = Object.fromEntries(OUTCOMES.map((o) => [o, rate(outcomes[o] ?? 0)]));
    rates.other = rate(finished - sum(OUTCOMES.map((o) => outcomes[o] ?? 0)));
    // Long runs count everywhere but here: one 2-hour lane turn would set p95.
    const short = g.filter((r) => !r.long);
    const latency = Object.fromEntries(TIMINGS.map((k) => [k, dist(short.map((r) => r.timings?.[k]))]));
    const tooled = g.filter((r) => r.tools);
    const topLevel = g.filter((r) => r.timings || r.manifest).length;
    const [calls, errors, expected] = ['calls', 'errors', 'expectedNonzero'].map((k) => sum(tooled.map((r) => r.tools[k])));
    const inputTok = sum(g.map((r) => r.row.input_tokens));
    const cachedTok = sum(g.map((r) => r.row.cached_input_tokens));
    groups[key] = {
      runs: g.length,
      finished,
      outcomes,
      rates,
      latency,
      // OB08a's tool calls before the first text (a count, not a latency).
      toolsBeforeText: dist(short.map((r) => r.timings?.toolsBeforeText)),
      // The ledger's start → end (Jev's latency: it has no phase timings).
      wallMs: dist(short.map((r) => r.wallMs)),
      longRuns: g.length - short.length,
      cost: costBlock(g.map((r) => r.row)),
      ledger: ledgerBlock(g.map((r) => r.row)),
      tokens: {
        inputInclCacheWrites: inputTok,
        cacheRead: cachedTok,
        output: sum(g.map((r) => r.row.output_tokens)),
        cacheReadShare: inputTok + cachedTok ? round4(cachedTok / (inputTok + cachedTok)) : null,
      },
      // Per turn (kept) and per 100 calls (OF20): a single-turn lane chat
      // of 130 calls looks ten times worse per turn than per call.
      tools: tooled.length
        ? {
            turns: topLevel,
            turnsWithTools: tooled.length,
            calls,
            errors,
            errorsPerTurn: round4(errors / Math.max(topLevel, tooled.length)),
            errorsPer100Calls: per100(errors, calls),
            expectedNonzero: expected,
            failures: errors - expected,
            failuresPer100Calls: per100(errors - expected, calls),
            checkFailed: sum(tooled.map((r) => r.tools.checkFailed)),
            maskedFailures: maskedBlock(tooled.flatMap((r) => r.calls ?? [])),
          }
        : null,
      retrieval: retrievalBlock(g),
      transcripts: transcriptBlock(g),
      codex: codexBlock(g, codex),
      verification: { claims: claimsBlock(verificationsOncePerRun(g)) },
    };
  }

  // Native-session continuity: stints reset per 100 turns, and why. A
  // flip reset is one whose turn ran in another permission mode than the
  // session's turn before it (the HB01 measure).
  const sessionOf = new Map(db.sessions.map((s) => [s.key, s]));
  // A turn is filed under the agent of the stint it ran in (the stint that
  // covers its start; the latest-started if two do), so a session that
  // switched agents keeps its older turns under the older agent. A turn no
  // stint covers falls back to the session's current agent.
  const stintsBySession = new Map();
  for (const s of db.allStints ?? db.stints) (stintsBySession.get(s.session_key) ?? stintsBySession.set(s.session_key, []).get(s.session_key)).push(s);
  const stintOf = (t) => {
    const t0 = at(t.started_at);
    let best = null;
    for (const s of stintsBySession.get(t.session_key) ?? []) {
      const [from, to] = [at(s.started_at), at(s.ended_at)];
      if (from == null || t0 == null || from > t0 || (to != null && t0 >= to)) continue;
      if (!best || from >= at(best.started_at)) best = s;
    }
    return best;
  };
  const agentOf = (t) => stintOf(t)?.agent ?? sessionOf.get(t.session_key)?.agent ?? 'unknown';
  const turnsBySession = new Map();
  for (const t of db.allTurns ?? db.turns) (turnsBySession.get(t.session_key) ?? turnsBySession.set(t.session_key, []).get(t.session_key)).push(t);
  for (const list of turnsBySession.values()) list.sort((x, y) => at(x.started_at) - at(y.started_at) || x.ord - y.ord);
  // The turn a reset happened in is the session's latest turn at the reset:
  // the turn's row is written first, and chat.rs announce_reset closes the
  // stint inside it, after the turn started (index into its session's list,
  // -1 for none). A move or an agent change closes a stint with no turn
  // running: never a flip.
  const resetTurnIndex = (reset) => {
    const list = turnsBySession.get(reset.session_key) ?? [];
    const t0 = at(reset.ended_at);
    let i = -1;
    if (t0 != null) for (let k = 0; k < list.length; k++) if (at(list[k].started_at) <= t0) i = k;
    return i;
  };
  const flipOf = (reset) => {
    if (reset.reason !== 'config changed') return null;
    const list = turnsBySession.get(reset.session_key) ?? [];
    const i = resetTurnIndex(reset);
    if (i < 1) return null;
    const [was, now] = [list[i - 1].permission_mode, list[i].permission_mode];
    return was && now && was !== now ? `${was}->${now}` : null;
  };
  // The tool pass's `reset` turns: per turn (`session|ord`), the times a
  // stint closed with a reset reason, attributed as above. The tool pass
  // keeps a time only when it is at or before the turn's end.
  const resetsByTurn = new Map();
  for (const s of db.allStints ?? db.stints) {
    if (!RESET_REASONS.has(s.reason)) continue;
    const t = (turnsBySession.get(s.session_key) ?? [])[resetTurnIndex(s)];
    if (!t) continue;
    const k = `${t.session_key}|${t.ord}`;
    (resetsByTurn.get(k) ?? resetsByTurn.set(k, []).get(k)).push(at(s.ended_at));
  }
  // What a config-changed reset changed (OB03's `stints.config_parts`): the
  // parts whose hash differs between the closed stint and the session's next
  // one. `unknown` when either side has no parts (stamped before OB03, or a
  // fresh attempt that never recorded a native id) or nothing differs, as
  // chat_resume::changed_parts. Usually the parts the notice named; not when
  // the notice compared the last closed stint (a pending reset) or the next
  // stint was stamped by a later turn: those read `unknown` or that turn's
  // config.
  const partsJson = (text) => {
    try {
      const o = JSON.parse(text);
      return o && typeof o === 'object' && !Array.isArray(o) ? o : null;
    } catch {
      return null;
    }
  };
  const partsOf = (reset) => {
    const t0 = at(reset.ended_at);
    let next = null;
    for (const s of stintsBySession.get(reset.session_key) ?? []) {
      const from = at(s.started_at);
      if (s === reset || from == null || t0 == null || from < t0) continue;
      if (!next || from < at(next.started_at)) next = s;
    }
    const [was, now] = [partsJson(reset.config_parts), partsJson(next?.config_parts)];
    if (!was || !now) return ['unknown'];
    const changed = [...new Set([...Object.keys(was), ...Object.keys(now)])].filter((k) => was[k] !== now[k]).sort();
    return changed.length ? changed : ['unknown'];
  };
  const continuity = {};
  const resetRows = db.resets ?? db.stints.filter((s) => s.reason);
  const turnAgent = new Map(db.turns.map((t) => [t, agentOf(t)]));
  const agents = [...new Set([...turnAgent.values(), ...db.stints.map((s) => s.agent ?? 'unknown'), ...resetRows.map((s) => s.agent ?? 'unknown')])].sort();
  const scopes = [
    ['all', () => true, () => true],
    ...agents.map((a) => [a, (t) => turnAgent.get(t) === a, (s) => (s.agent ?? 'unknown') === a]),
    ['mainAgent', (t) => isMainAgent(sessionOf.get(t.session_key)?.cwd), (s) => isMainAgent(sessionOf.get(s.session_key)?.cwd)],
  ];
  for (const [scope, turnIn, stintIn] of scopes) {
    const turns = db.turns.filter(turnIn);
    const stints = db.stints.filter(stintIn);
    const resetsIn = resetRows.filter(stintIn);
    if (scope === 'mainAgent' && !turns.length && !stints.length) continue;
    const causes = {};
    const flips = {};
    const byPart = {};
    for (const s of resetsIn) {
      causes[s.reason] = (causes[s.reason] ?? 0) + 1;
      const f = flipOf(s);
      if (f) flips[f] = (flips[f] ?? 0) + 1;
      if (s.reason === 'config changed') for (const p of partsOf(s)) byPart[p] = (byPart[p] ?? 0) + 1;
    }
    const resets = sum(Object.values(causes));
    const per100 = (n) => (turns.length ? round2((n * 100) / turns.length) : null);
    continuity[scope] = {
      turns: turns.length,
      stints: stints.length,
      resets,
      causes,
      flipResets: sum(Object.values(flips)),
      flips,
      resetsPer100Turns: per100(resets),
      configChangedPer100Turns: per100(causes['config changed'] ?? 0),
      flipResetsPer100Turns: per100(sum(Object.values(flips))),
      // A reset that changed two parts counts under both.
      resetsByPart: Object.fromEntries(Object.entries(byPart).sort(([a], [b]) => a.localeCompare(b))),
    };
  }

  // Injected bytes per layer, overall and by the compiler's session state.
  const withManifest = runs.filter((r) => r.manifest);
  const layerNames = [...new Set(withManifest.flatMap((r) => r.manifest.layers.map((l) => l.name)))].sort();
  const layerStats = (rs) => ({
    runs: rs.length,
    total: dist(rs.map((r) => sum(r.manifest.layers.map((l) => l.bytes)))),
    layers: Object.fromEntries(layerNames.map((n) => [n, dist(rs.flatMap((r) => r.manifest.layers.filter((l) => l.name === n).map((l) => l.bytes)))]).filter(([, d]) => d.n)),
  });
  const states = {};
  for (const r of withManifest) (states[r.manifest.session ?? 'none'] ??= []).push(r);
  const entries = {};
  for (const r of withManifest) (entries[r.manifest.entry ?? 'none'] ??= []).push(r);
  const budgeted = withManifest.filter((r) => r.manifest.workspaceCut != null);
  const cut = budgeted.filter((r) => r.manifest.workspaceCut).length;
  const injected = {
    ...layerStats(withManifest),
    bySession: Object.fromEntries(Object.entries(states).sort().map(([s, rs]) => [s, layerStats(rs)])),
    byEntry: Object.fromEntries(Object.entries(entries).sort().map(([s, rs]) => [s, layerStats(rs)])),
    workspaceCut: { runs: budgeted.length, cut, rate: budgeted.length ? round4(cut / budgeted.length) : null },
  };

  // Where runs fail: tool errors by tool name, and the friction table's
  // families seen in the window (tool and class only, never the message).
  const byTool = {};
  for (const r of runs) for (const [name, n] of Object.entries(r.tools?.byTool ?? {})) byTool[name] = (byTool[name] ?? 0) + n;
  // `count` is the store's own 30-day count, not the window's.
  const fam = {};
  for (const f of db.friction ?? []) {
    const k = `${f.tool}|${f.error_class}`;
    fam[k] = (fam[k] ?? 0) + (num(f.count) ?? 0);
  }
  const failures = {
    byTool: Object.fromEntries(Object.entries(byTool).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 15)),
    frictionTop: Object.entries(fam).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 10).map(([k, count]) => {
      const [tool, errorClass] = k.split('|');
      return { tool, errorClass, count30d: count };
    }),
  };
  const toolErrors = toolErrorsBlock({ steps: db.steps, stepErrors: db.stepErrors ?? [], turns: db.turns, agentOf, runs });
  const timingsOfRun = (runId) => timingsOf(receiptByRun.get(runId), queueByRun.get(runId));
  const outcomes = outcomesBlock({ turns: db.turns, turnsBySession, ledger: db.ledger, timingsOfRun, toolErrors });
  const mainAgent = mainAgentBlock({ turns: db.turns, ledger: db.ledger, steps: db.steps, refusals: db.refusals ?? [], pulse: db.pulse ?? [], suggestions });
  const memory = memoryBlock({ runs, receipts, learnings });

  // Delegation: ask_agent consultations are the ledger's delegate rows;
  // starts, sends and handovers are the delegation records.
  const asks = db.ledger.filter((r) => r.surface === 'delegate');
  const kinds = {};
  for (const d of db.delegations ?? []) kinds[d.tool ?? d.kind ?? 'unknown'] = (kinds[d.tool ?? d.kind ?? 'unknown'] ?? 0) + 1;
  const timeouts = asks.filter((r) => r.outcome === 'timeout').length;
  // A14: the tool pass is Claude's (its tools, its cache writes); Codex runs
  // get their own pass, over their own handovers, so neither skews the other.
  const isCodex = (r) => r.row.agent === 'codex';
  const codexRunIds = new Set(runs.filter(isCodex).map((r) => r.row.run_id).filter(Boolean));
  const fromCodex = (d) => codexRunIds.has(d.run_id);
  // The `handover` block counts every record; Claude's handover context
  // only Claude predecessors, Codex's only Codex ones.
  const tooling = toolPassBlocks({ runs: runs.filter((r) => !isCodex(r)), turns: db.turns, turnsBySession, resetsByTurn, delegations: db.delegations ?? [] });
  if (tooling.transcripts) {
    tooling.transcripts.workflowAgents = workflowBlock(runs, workflows);
    tooling.transcripts.handoverContext = toolPassBlocks({ runs: runs.filter((r) => !isCodex(r)), turns: db.turns, turnsBySession, resetsByTurn, delegations: (db.delegations ?? []).filter((d) => !fromCodex(d)) }).transcripts.handoverContext;
  }
  const codexTooling = toolPassBlocks({ runs: runs.filter(isCodex), turns: db.turns, turnsBySession, resetsByTurn, delegations: (db.delegations ?? []).filter(fromCodex) }).transcripts;
  // Codex writes no cache, so cache-write figures would read a measured 0:
  // they are null, and a cold resume shows as the first call's cache read
  // (and the share of first calls reading none), by resume gap.
  const noWrites = (fc) => (fc ? { ...fc, cacheWriteP50: null } : fc);
  const codexPass = codexTooling
    ? {
        turns: codexTooling.turns,
        firstCall: {
          bySession: Object.fromEntries(Object.entries(codexTooling.firstCall.bySession).map(([k, v]) => [k, noWrites(v)])),
          resumeByGap: Object.fromEntries(Object.entries(codexTooling.firstCall.resumeByGap).map(([k, v]) => [k, noWrites(v)])),
        },
        coldResume: noWrites(codexTooling.firstCall.resumeByGap.gt60m),
        handoverContext: codexTooling.handoverContext,
        ...codexBlock(runs, codex),
      }
    : null;
  const delegation = {
    askAgent: { runs: asks.length, timeouts, timeoutRate: asks.length ? round4(timeouts / asks.length) : null },
    records: kinds,
    handovers: kinds.handover ?? 0,
    handover: tooling.handover,
  };

  // History snapshot and diff cost per workspace (top-level runs only).
  const history = {};
  for (const r of runs) {
    if (!r.timings || r.timings.baselineSnapshotMs == null) continue;
    (history[r.row.workspace_id ?? 'none'] ??= []).push(r);
  }
  const historyByWorkspace = Object.fromEntries(Object.entries(history).sort().map(([w, rs]) => [w, { runs: rs.length, baselineSnapshotMs: dist(rs.map((r) => r.timings.baselineSnapshotMs)), finalizeDiffMs: dist(rs.map((r) => r.timings.finalizeDiffMs)) }]));

  // Runs that started before the window and ended inside it (in the groups
  // only with --overlap), and the top-level runs over LONG_RUN_MS.
  const byKey = (keys) => {
    const out = {};
    for (const k of keys) out[k] = (out[k] ?? 0) + 1;
    return Object.fromEntries(Object.entries(out).sort());
  };
  const straddlingRows = db.straddling ?? [];
  const straddling = { runs: straddlingRows.length, cost: costBlock(straddlingRows), groups: byKey(straddlingRows.map(groupKey)) };
  const long = runs.filter((r) => r.long);
  const longRuns = {
    thresholdMs: LONG_RUN_MS,
    runs: long.length,
    minutes: dist(long.map((r) => round1(r.wallMs / 60_000))),
    cost: costBlock(long.map((r) => r.row)),
    groups: byKey(long.map((r) => r.key)),
  };

  // Lease waits per queued row (S14's debounce): waits over the time from
  // the first wait to the claim (to the last wait when it was never claimed).
  // The first wait sets waiting_since, so a 1-wait row times no gap between
  // waits (only the wait-to-claim time): a rate needs at least 2 waits.
  const waitRows = (db.waits ?? []).map((w) => {
    const from = at(w.waiting_since);
    const to = at(w.claimed_at) ?? at(w.last_waited_at);
    const waits = num(w.wait_count) ?? 0;
    const secs = waits >= 2 && from != null && to != null && to > from ? (to - from) / 1000 : null;
    return { holder: w.waiting_holder ?? 'unknown', waits, perSecond: secs ? waits / secs : null };
  });
  // The max rate, and the waits on the row that set it.
  const waitStats = (ws) => {
    const top = ws.filter((w) => w.perSecond != null).reduce((a, w) => (!a || w.perSecond > a.perSecond ? w : a), null);
    return { waits: sum(ws.map((w) => w.waits)), rows: ws.length, maxWaitsPerSecond: top ? round1(top.perSecond) : null, maxRateWaits: top ? top.waits : null };
  };
  const holders = [...new Set(waitRows.map((w) => w.holder))].sort();
  const queue = { ...waitStats(waitRows), byHolder: Object.fromEntries(holders.map((h) => [h, waitStats(waitRows.filter((w) => w.holder === h))])) };

  const counts = {
    ledgerRows: db.ledger.length,
    receiptsJoined: runs.filter((r) => r.timings && receiptByRun.has(r.row.run_id)).length,
    manifestsJoined: withManifest.length,
    transcriptRunsJoined: runs.filter((r) => r.calls).length,
    turns: db.turns.length,
    stints: db.stints.length,
    // Rows with no end yet, in the groups or straddling (by run key).
    stillRunning: new Set([...db.ledger, ...straddlingRows].filter((r) => r.outcome === 'running').map((r) => r.run_key)).size,
  };
  return {
    schema: SCHEMA,
    kind: KIND,
    window: { since: new Date(window.since).toISOString(), until: new Date(window.until).toISOString(), overlap: !!window.overlap },
    counts,
    groups,
    // OB05's call figures over the window's finished top-level runs (OF28;
    // the groups' blocks keep every row of theirs).
    ledger: ledgerBlock(db.ledger.filter(isFinishedTopLevel)),
    // Whether the store has OF28's ledger columns at all: a column with no
    // value in the window is "none", a missing one "not installed".
    sources: { usageJson: !!db.ledgerColumns?.usageJson, costFloorUsd: !!db.ledgerColumns?.costFloorUsd },
    straddling,
    longRuns,
    queue,
    continuity,
    injected,
    failures,
    toolErrors,
    outcomes,
    mainAgent,
    memory,
    // S27: over every receipt in the window (the groups' over their own
    // top-level runs' receipts).
    verification: { claims: claimsBlock(receipts.map((r) => r?.verification)) },
    delegation,
    transcripts: tooling.transcripts,
    // A14: Codex runs joined to their rollouts (null without any).
    codex: codexPass,
    history: historyByWorkspace,
    notes,
  };
}

// ── Tool errors per call (OF20) ──────────────────────────────────────────

/** `toolErrors`: the steps of the window's turns (every turn, by the agent
 *  of the stint it ran in), per agent and tool, `all` first: calls, errors,
 *  `expectedNonzero` (expectedNonzero above), `failures` (the rest), each
 *  per 100 calls, and `checkFailed` (clean exits OF16's step spine marked
 *  as a failed check; 0 before that is installed). A shell tool also gets
 *  its failures' pipe and redirect counts and their top command heads.
 *  `maskedFailures`: maskedBlock over every joined transcript. */
export function toolErrorsBlock({ steps, stepErrors, turns, agentOf, runs }) {
  const turnOfRun = new Map(turns.filter((t) => t.run_id).map((t) => [t.run_id, t]));
  const byAgent = {};
  const entry = (agent, tool) => ((byAgent[agent] ??= {})[tool] ??= { calls: 0, errors: 0, expectedNonzero: 0, failures: 0, checkFailed: 0 });
  for (const s of steps) {
    const t = turnOfRun.get(s.run_id);
    if (!t) continue;
    for (const k of ['all', s.name ?? 'unknown']) {
      const b = entry(agentOf(t), k);
      b.calls += s.n;
      b.checkFailed += num(s.check_failed) ?? 0;
      if (s.status === 'error') b.errors += s.n;
    }
  }
  const heads = new Map();
  for (const e of stepErrors) {
    const t = turnOfRun.get(e.run_id);
    if (!t) continue;
    const agent = agentOf(t);
    const tool = e.name ?? 'unknown';
    if (expectedNonzero(e)) {
      entry(agent, 'all').expectedNonzero++;
      entry(agent, tool).expectedNonzero++;
    } else if (SHELL_TOOLS.includes(tool)) {
      const b = entry(agent, tool);
      b.failuresPiped = (b.failuresPiped ?? 0) + (e.pipe ? 1 : 0);
      b.failuresRedirected = (b.failuresRedirected ?? 0) + (e.redirect ? 1 : 0);
      const h = heads.get(b) ?? heads.set(b, {}).get(b);
      h[e.head ?? 'unknown'] = (h[e.head ?? 'unknown'] ?? 0) + 1;
    }
  }
  const out = {};
  for (const agent of Object.keys(byAgent).sort()) {
    const tools = Object.entries(byAgent[agent]).sort(([a, x], [b, y]) => (a === 'all' ? -1 : b === 'all' ? 1 : y.calls - x.calls || a.localeCompare(b)));
    out[agent] = Object.fromEntries(
      tools.map(([tool, b]) => {
        // A step error recorded without its steps row (it can't be) never
        // makes failures negative.
        const failures = Math.max(0, b.errors - b.expectedNonzero);
        const h = heads.get(b);
        return [tool, {
          ...b,
          failures,
          errorsPer100Calls: per100(b.errors, b.calls),
          failuresPer100Calls: per100(failures, b.calls),
          ...(h ? { failureHeads: Object.fromEntries(Object.entries(h).sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0])).slice(0, 10)) } : {}),
        }];
      }),
    );
  }
  const calls = [...new Set(runs.flatMap((r) => r.calls ?? []))];
  return { byAgent: out, maskedFailures: maskedBlock(calls) };
}

// ── Implicit turn outcomes (S08) ─────────────────────────────────────────

/** `outcomes`: what the window's turns (every agent, `turns` rows) did,
 *  read from statuses, timestamps and timings only. These are proxies for
 *  a turn that went wrong, never a verdict on one: report them, never tune
 *  anything on them automatically. Definitions:
 *  - finished: turns with a status other than `running` (`interrupted`
 *    included, as the groups' rates count it); every rate's n.
 *  - cancelled / errored / timedOut: a turn's `cancelled`, `error` or
 *    `timeout` status; chat.rs writes a timeout as `error`, so an `error`
 *    turn whose top-level chat ledger row says `timeout` is timedOut. That
 *    row says `timeout` for a no-output stall too (usage_ledger.rs
 *    `outcome_for`: `Ended::Stalled` → Timeout, the watchdog's bound), so
 *    timedOut counts stalls as well as time limits.
 *  - An `interrupted` turn is none of these and never a stopped turn: the
 *    restart reconciler (session_store.rs `interrupt_chat_run_in`) closes a
 *    turn a previous process left running and writes its `ended_at` at the
 *    sweep, not when it stopped, and neither the person nor an agent
 *    stopped it.
 *  - stoppedBeforeText: of those stopped turns whose phase timings are
 *    known (the receipt's, else the queue row's), the ones with no
 *    `firstTextMs`: the turn ended before the agent wrote any text (a stop
 *    before the spawn included). `unknownTiming` counts the rest.
 *  - reaskWithin120s: of the stopped turns with an `ended_at`, those whose
 *    next turn in the same conversation, asked by the person (`asked_by`
 *    NULL: not another agent or Hollow), started 0 to REASK_MS after it
 *    ended, both bounds inclusive: the person asked again. Caveat: NULL is
 *    only "the person" where the column is filled. `turns.asked_by` exists
 *    since 2026-10-07 (older rows are all NULL), and Hollow's own wakes and
 *    check-ins carry no `hollow:` tag until OB20 merges, so until then a
 *    wake that follows a stopped turn within 120 s reads as a re-ask.
 *    `turns.asked` is never read to tell them apart.
 *  - toolFailuresPerTurn: `toolErrors`' failures (errors less the expected
 *    non-zero exits) summed over agents, per tool, over the window's turns. */
export function outcomesBlock({ turns, turnsBySession, ledger, timingsOfRun, toolErrors }) {
  const share = (k, d) => (d ? round4(k / d) : null);
  const timedOutRuns = new Set(ledger.filter((r) => r.surface === 'chat' && !r.parent_surface && r.run_id && r.outcome === 'timeout').map((r) => r.run_id));
  const kindOf = (t) => {
    if (t.status === 'cancelled') return 'cancelled';
    if (t.status === 'timeout' || (t.status === 'error' && timedOutRuns.has(t.run_id))) return 'timeout';
    // `interrupted` is not a stop (the reconciler's sweep, see above).
    return t.status === 'error' ? 'error' : null;
  };
  const byStatus = {};
  for (const t of turns) byStatus[t.status ?? 'none'] = (byStatus[t.status ?? 'none'] ?? 0) + 1;
  const finished = turns.filter((t) => t.status != null && t.status !== 'running').length;
  const stopped = turns.filter((t) => kindOf(t));
  const count = (k) => stopped.filter((t) => kindOf(t) === k).length;
  const rate = (k) => ({ turns: count(k), n: finished, share: share(count(k), finished) });

  const timed = stopped.map((t) => (t.run_id ? timingsOfRun(t.run_id) : null));
  const known = timed.filter(Boolean);
  const silent = known.filter((x) => num(x.firstTextMs) == null).length;

  const ended = stopped.filter((t) => at(t.ended_at) != null);
  const reasked = ended.filter((t) => {
    const list = turnsBySession.get(t.session_key) ?? [];
    const i = list.findIndex((x) => x.ord === t.ord);
    const next = i < 0 ? null : list[i + 1];
    if (!next || next.asked_by != null) return false;
    const gap = at(next.started_at) - at(t.ended_at);
    return Number.isFinite(gap) && gap >= 0 && gap <= REASK_MS;
  }).length;

  const failures = {};
  for (const tools of Object.values(toolErrors?.byAgent ?? {})) {
    for (const [tool, b] of Object.entries(tools)) if (tool !== 'all') failures[tool] = (failures[tool] ?? 0) + b.failures;
  }
  const n = turns.length;
  const all = sum(Object.values(failures));
  return {
    turns: n,
    finished,
    byStatus: Object.fromEntries(Object.entries(byStatus).sort(([a], [b]) => a.localeCompare(b))),
    cancelled: rate('cancelled'),
    errored: rate('error'),
    timedOut: rate('timeout'),
    stoppedBeforeText: { turns: silent, n: known.length, share: share(silent, known.length), unknownTiming: stopped.length - known.length },
    reaskWithin120s: { turns: reasked, n: ended.length, share: share(reasked, ended.length) },
    toolFailuresPerTurn: {
      n,
      all: { failures: all, perTurn: share(all, n) },
      byTool: Object.fromEntries(Object.entries(failures).filter(([, f]) => f).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 15).map(([k, f]) => [k, { failures: f, perTurn: share(f, n) }])),
    },
  };
}

// ── The Main agent's own turns (OF23) ────────────────────────────────────

/** `mainAgent`: the window's turns Hollow asked itself (`turns.asked_by`
 *  `hollow:<kind>`, OB20: wake, checkin, weekly, wake_me…), overall (`all`)
 *  and by kind. Per kind: turns; cost over the turns whose top-level chat
 *  ledger rows all carry a known cost (p50 and their sum, a floor while
 *  `unknownTurns` > 0, null when no turn's cost is known: never $0); tool
 *  calls (steps) and per turn; read_session calls (a step whose tool name
 *  ends `read_session`), per turn and the turns with
 *  one; refusals (failed steps whose result is an unattended-turn limit,
 *  REFUSAL_LIKE) and the turns with one; `pulseOk`, the turns whose stored
 *  answer is exactly `PULSE_OK` over those with a stored answer
 *  (PULSE_OK_SQL). `suggestions`: readSuggestions (the SUGGESTION_DAYS
 *  before the window's end; null without a log). No tagged turn: `status:
 *  "untagged"`, with `all` zeroed, not a missing block. */
export function mainAgentBlock({ turns, ledger, steps, refusals = [], pulse = [], suggestions = null }) {
  const tagged = turns.filter((t) => typeof t.asked_by === 'string' && t.asked_by.startsWith('hollow:'));
  const costByRun = new Map();
  for (const r of ledger) {
    if (r.surface !== 'chat' || r.parent_surface || !r.run_id) continue;
    const c = costByRun.get(r.run_id) ?? costByRun.set(r.run_id, { usd: 0, unknown: false }).get(r.run_id);
    if (KNOWN_COST.has(r.cost_status) && num(r.cost_usd) != null) c.usd += r.cost_usd;
    else c.unknown = true;
  }
  const toolsByRun = new Map();
  for (const s of steps) {
    const t = toolsByRun.get(s.run_id) ?? toolsByRun.set(s.run_id, { calls: 0, readSession: 0 }).get(s.run_id);
    t.calls += s.n;
    if (String(s.name ?? '').endsWith('read_session')) t.readSession += s.n;
  }
  const refusedByRun = new Map(refusals.map((r) => [r.run_id, num(r.n) ?? 0]));
  const pulseByTurn = new Map(pulse.map((p) => [`${p.session_key}|${p.ord}`, p.pulse_ok == null ? null : !!p.pulse_ok]));
  const per = (k, n) => (n ? round2(k / n) : null);
  const stats = (ts) => {
    const n = ts.length;
    const known = ts.map((t) => costByRun.get(t.run_id)).filter((c) => c && !c.unknown).map((c) => c.usd);
    const tools = ts.map((t) => toolsByRun.get(t.run_id) ?? { calls: 0, readSession: 0 });
    const refused = ts.map((t) => refusedByRun.get(t.run_id) ?? 0);
    const pulses = ts.map((t) => pulseByTurn.get(`${t.session_key}|${t.ord}`)).filter((v) => v != null);
    const [calls, reads, refusalCount, ok] = [sum(tools.map((x) => x.calls)), sum(tools.map((x) => x.readSession)), sum(refused), pulses.filter(Boolean).length];
    return {
      turns: n,
      cost: { knownTurns: known.length, unknownTurns: n - known.length, p50Usd: round4(percentile(known, 0.5)), sumUsd: known.length ? round4(sum(known)) : null },
      toolCalls: calls,
      toolsPerTurn: per(calls, n),
      readSessionCalls: reads,
      readSessionPerTurn: per(reads, n),
      turnsWithReadSession: tools.filter((x) => x.readSession > 0).length,
      refusals: refusalCount,
      turnsWithRefusal: refused.filter((x) => x > 0).length,
      pulseOk: { ok, n: pulses.length, share: pulses.length ? round4(ok / pulses.length) : null },
    };
  };
  const kinds = [...new Set(tagged.map((t) => t.asked_by.slice('hollow:'.length) || 'unknown'))].sort();
  return {
    status: tagged.length ? 'tagged' : 'untagged',
    turns: tagged.length,
    all: stats(tagged),
    byKind: Object.fromEntries(kinds.map((k) => [k, stats(tagged.filter((t) => (t.asked_by.slice('hollow:'.length) || 'unknown') === k))])),
    suggestions,
  };
}

// ── Native memory writes against learnings accepted (S13) ────────────────

/** `memory`: Claude Code's own auto-memory notes against Hollow's accepted
 *  learnings, over the window. Definitions:
 *  - nativeWrites: the joined runs' transcript Write/Edit/MultiEdit uses
 *    (main thread and subagents, each call once) into an auto-memory folder
 *    (isMemoryPath) whose result came back without an error, as the
 *    receipt counts them; null without transcripts. `nativeWriteErrors`
 *    counts the failed ones, `nativeWriteRuns` the runs with a write, and
 *    `nativeWritesByWorkspace` splits them by the run's ledger workspace.
 *  - receiptMemoryWrites: the window's receipts' `memoryWrites` (PR #736)
 *    summed over the receipts that carry it (`receiptsWithMemoryWrites`);
 *    null when none does (older receipts and other agents lack it: unknown,
 *    not 0).
 *  - learningsAccepted: readLearnings' `accepted` (null without a
 *    `learnings/` folder); `nativeWritesPerLearning` the ratio. */
export function memoryBlock({ runs, receipts, learnings }) {
  const joined = runs.filter((r) => r.calls);
  const seen = new Set();
  let writes = 0;
  let errors = 0;
  const runsWith = new Set();
  const byWorkspace = {};
  for (const r of joined) {
    for (const c of r.calls) {
      if (seen.has(c)) continue;
      seen.add(c);
      for (const u of c.uses ?? []) {
        if (!u.memoryWrite) continue;
        if (u.error === true) errors++;
        if (u.error !== false) continue;
        writes++;
        runsWith.add(r.row.run_key);
        const ws = r.row.workspace_id ?? 'none';
        byWorkspace[ws] = (byWorkspace[ws] ?? 0) + 1;
      }
    }
  }
  const carrying = receipts.filter((x) => num(x?.memoryWrites) != null);
  const nativeWrites = joined.length ? writes : null;
  const accepted = learnings?.accepted ?? null;
  return {
    nativeWrites,
    nativeWriteErrors: joined.length ? errors : null,
    nativeWriteRuns: joined.length ? runsWith.size : null,
    nativeWritesByWorkspace: Object.fromEntries(Object.entries(byWorkspace).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))),
    receiptMemoryWrites: carrying.length ? sum(carrying.map((x) => x.memoryWrites)) : null,
    receiptsWithMemoryWrites: carrying.length,
    receiptsWithoutMemoryWrites: receipts.length - carrying.length,
    learningsAccepted: accepted,
    learnings,
    nativeWritesPerLearning: nativeWrites != null && accepted ? round2(nativeWrites / accepted) : null,
  };
}

// ── Workflow-tool agents (OF19) ──────────────────────────────────────────

/** `transcripts.workflowAgents`: the Workflow tool's agents under the
 *  native sessions of the window's Claude ledger rows, by call time in the
 *  window (`workflows`: session → { file, calls }[]): agents (files with a
 *  call), sessions, calls, tokens, weighted tokens and their share of all
 *  weight (the joined runs' main and subagent calls plus these), and
 *  `usdAtJoinedRate`, an estimate: their weight at the joined runs' known $
 *  per weighted token (the ledger doesn't meter them yet, S31). */
export function workflowBlock(runs, workflows) {
  const lists = [...workflows.values()];
  const calls = lists.flatMap((l) => l.flatMap((x) => x.calls));
  const joined = runs.filter((r) => r.calls);
  const runCalls = [...new Set(joined.flatMap((r) => r.calls))];
  const w = sum(calls.map(weight));
  const all = w + sum(runCalls.map(weight));
  const priced = joined.filter((r) => KNOWN_COST.has(r.row.cost_status) && num(r.row.cost_usd) != null);
  // Deduped like runCalls: a call two runs share is weighed once.
  const pricedWeight = sum([...new Set(priced.flatMap((r) => r.calls))].map(weight));
  const rate = pricedWeight ? sum(priced.map((r) => r.row.cost_usd)) / pricedWeight : null;
  return {
    agents: sum(lists.map((l) => l.length)),
    sessions: lists.length,
    calls: calls.length,
    tokens: { input: sum(calls.map((c) => c.input)), cacheWrite: sum(calls.map((c) => c.cacheWrite)), cacheRead: sum(calls.map((c) => c.cacheRead)), output: sum(calls.map((c) => c.output)) },
    weightedTokens: round1(w),
    weightShare: all ? round4(w / all) : null,
    usdAtJoinedRate: rate == null ? null : round2(w * rate),
  };
}

// ── The tool pass's blocks (OF02) ────────────────────────────────────────

/** First calls: cache write and read p50, and the share that read no cache
 *  (a cold start), over n. */
function firstCallStats(calls) {
  const cold = calls.filter((c) => c.cacheRead === 0).length;
  return {
    n: calls.length,
    cacheWriteP50: percentile(calls.map((c) => c.cacheWrite), 0.5),
    cacheReadP50: percentile(calls.map((c) => c.cacheRead), 0.5),
    coldShare: calls.length ? round4(cold / calls.length) : null,
  };
}

/** The stint close reasons a reset writes (chat_resume.rs
 *  ResetReason::as_str). `agent changed` is left out: session_store.rs
 *  writes it when the session row is upserted under another agent, before
 *  the new turn's row exists, so it can't be pinned to a turn this way. */
export const RESET_REASONS = new Set(['config changed', 'resume failed', 'moved', 'invalid session id']);

/** A resume's gap since the previous turn's end: under 5 min, 5 to 60
 *  (inclusive), over 60, or unknown (the previous turn has no end). */
export const RESUME_GAPS = ['lt5m', 'm5to60', 'gt60m', 'unknown'];
const gapBucket = (ms) => (ms == null ? 'unknown' : ms < 5 * 60_000 ? 'lt5m' : ms <= 60 * 60_000 ? 'm5to60' : 'gt60m');

/** The tool pass over Claude chat turns (OF02). Definitions:
 *  - a turn is a `turns` row in the window (the scorecard's turn count)
 *    whose top-level `chat` ledger row joined a Claude transcript; `n` is
 *    their count, the denominator of every per-turn rate here. Only main-
 *    thread calls count (a subagent's tools are its own), and a run with
 *    two chat rows (an account failover) is one turn.
 *  - toolSearchPer100Turns: ToolSearch tool uses × 100 / n.
 *  - toolSearch.selectedByTool: tool names in `select:` queries, by count
 *    (an entry that isn't a tool name counts as `other`).
 *  - alwaysLoad.usedShare[tool]: the share of turns that called
 *    `mcp__hollow_app__<tool>` at least once; `any`: one of the six.
 *  - firstCall.bySession: each turn's first main-thread call (the earliest
 *    distinct message id in its slice) by the turn's place in its session:
 *    `new` (the session has no earlier turn), `reset` (a stint of the
 *    session closed with a RESET_REASONS reason inside this turn: at or
 *    after its start and at or before its end, ended_at or else its first
 *    call; a new native session) or `resume`; resumeByGap splits resumes
 *    by the time since the previous turn's `ended_at`.
 *  - waitOnlyCalls: wait_for_session calls (or run_action `sessions.wait`),
 *    and how many answered `finished: false` (the wait ran out with the
 *    child still running: a wait that bought nothing), split by the
 *    answer's status in stillRunningByStatus (`queued`: the child hadn't
 *    started; `running`; `other`); a result that is an error or not a
 *    short JSON object is `unclassified`.
 *  - bashRefusalRate[mode]: Bash results with is_error, no leading `Exit
 *    code N` (the command ran) and class `permission`, over the Bash calls
 *    of turns in that permission mode.
 *  - handoverContext: context (input + cache write + cache read) of the
 *    predecessor turn's last main-thread call at or before the handover;
 *    `unjoined` counts the handovers left out: no predecessor turn in the
 *    window, no joined predecessor call at or before it (noTranscript),
 *    and (for the overlap) no joined successor first turn.
 *  - handoverRereadOverlap: of the distinct files the successor's first
 *    turn Read, how many the predecessor chat's turns had touched (Read,
 *    Edit or Write), with the predecessor sets' sizes summed in
 *    predecessorTouched. Main thread only: Bash reads and subagents' reads
 *    are not seen. Counts only.
 *  - delegation.handover.childFirstTurnOk: handover successors whose first
 *    turn ended `success`, over those whose first turn has ended. */
export function toolPassBlocks({ runs, turns, turnsBySession, resetsByTurn = new Map(), delegations }) {
  const turnByRun = new Map(turns.filter((t) => t.run_id).map((t) => [t.run_id, t]));
  const firstTurnOf = (key) => (turnsBySession.get(key) ?? [])[0] ?? null;
  const share = (k, d) => (d ? round4(k / d) : null);
  const handovers = delegations.filter((d) => d.tool === 'handover');

  // Successors' first turns, from the turns table alone.
  let ok = 0;
  let ended = 0;
  for (const h of handovers) {
    const first = h.child ? firstTurnOf(h.child) : null;
    if (!first || first.status == null || first.status === 'running') continue;
    ended++;
    if (first.status === 'success') ok++;
  }
  const handover = { records: handovers.length, childFirstTurnOk: { ok, n: ended, share: share(ok, ended) }, noEndedFirstTurn: handovers.length - ended };

  const byRun = new Map();
  for (const r of runs) {
    if (!r.calls || r.row.surface !== 'chat' || r.row.parent_surface || !turnByRun.has(r.row.run_id)) continue;
    const set = byRun.get(r.row.run_id) ?? byRun.set(r.row.run_id, new Set()).get(r.row.run_id);
    for (const c of r.calls) if (!c.sidechain) set.add(c);
  }
  const joined = [...byRun]
    .map(([runId, set]) => {
      const main = [...set].sort((a, b) => a.t - b.t);
      return { turn: turnByRun.get(runId), main, uses: main.flatMap((c) => c.uses ?? []) };
    })
    .filter((j) => j.main.length);
  if (!joined.length) return { transcripts: null, handover };
  const n = joined.length;
  const all = joined.flatMap((j) => j.uses);

  const searches = all.filter((u) => u.search);
  const selected = {};
  for (const u of searches) for (const name of u.select) selected[name] = (selected[name] ?? 0) + 1;
  const turnsWithSearch = joined.filter((j) => j.uses.some((u) => u.search)).length;
  const toolSearch = {
    calls: searches.length,
    selectCalls: searches.filter((u) => u.search === 'select').length,
    keywordCalls: searches.filter((u) => u.search === 'keyword').length,
    turnsWith: turnsWithSearch,
    turnsWithShare: share(turnsWithSearch, n),
    selectedByTool: Object.fromEntries(Object.entries(selected).sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))),
  };

  const turnsUsing = {};
  for (const tool of [...ALWAYS_LOAD, 'any']) {
    const hit = (u) => (tool === 'any' ? ALWAYS_LOAD.includes(appTool(u.name)) : appTool(u.name) === tool);
    turnsUsing[tool] = joined.filter((j) => j.uses.some(hit)).length;
  }
  const alwaysLoad = { n, turnsUsing, usedShare: Object.fromEntries(Object.entries(turnsUsing).map(([k, v]) => [k, share(v, n)])) };

  const firsts = { new: [], reset: [], resume: [] };
  const gaps = Object.fromEntries(RESUME_GAPS.map((g) => [g, []]));
  for (const j of joined) {
    const t = j.turn;
    const list = turnsBySession.get(t.session_key) ?? [];
    const i = list.findIndex((x) => x.ord === t.ord);
    const prev = i > 0 ? list[i - 1] : null;
    // A reset closes the old stint inside the turn, after it started: the
    // turn is `reset` when a reset attributed to it (summarize's
    // resetsByTurn, the same attribution as the flip count) closed at or
    // before its end (its ended_at, else its first call).
    const turnEnd = at(t.ended_at) ?? j.main[0].t;
    const reset = (resetsByTurn.get(`${t.session_key}|${t.ord}`) ?? []).some((e) => e != null && e <= turnEnd);
    const kind = !prev ? 'new' : reset ? 'reset' : 'resume';
    firsts[kind].push(j.main[0]);
    j.kind = kind;
    if (kind === 'resume') {
      const end = at(prev.ended_at);
      j.gap = gapBucket(end == null ? null : at(t.started_at) - end);
      gaps[j.gap].push(j.main[0]);
    }
  }
  const firstCall = {
    bySession: Object.fromEntries(Object.entries(firsts).map(([k, cs]) => [k, firstCallStats(cs)])),
    resumeByGap: Object.fromEntries(Object.entries(gaps).map(([k, cs]) => [k, firstCallStats(cs)])),
  };

  const waits = all.filter((u) => u.wait);
  const still = waits.filter((u) => u.waitFinished === false).length;
  const done = waits.filter((u) => u.waitFinished === true).length;
  const byStatus = { queued: 0, running: 0, other: 0 };
  for (const u of waits) if (u.waitFinished === false) byStatus[u.waitStatus ?? 'other']++;
  const waitOnlyCalls = { calls: waits.length, stillRunning: still, stillRunningByStatus: byStatus, ended: done, unclassified: waits.length - still - done, stillRunningShare: share(still, waits.length) };

  const modes = {};
  for (const j of joined) {
    for (const m of ['all', j.turn.permission_mode ?? 'unknown']) {
      const b = (modes[m] ??= { bashCalls: 0, errors: 0, refused: 0 });
      for (const u of j.uses) {
        if (u.name !== 'Bash') continue;
        b.bashCalls++;
        if (u.error) b.errors++;
        if (u.refusal === 'permission') b.refused++;
      }
    }
  }
  const bashRefusalRate = Object.fromEntries(Object.entries(modes).sort().map(([m, b]) => [m, { ...b, rate: share(b.refused, b.bashCalls) }]));

  const joinedByRun = new Map(joined.map((j) => [j.turn.run_id, j]));
  // Main-thread file paths only (in memory, never reported): a successor's
  // Reads, a predecessor's Reads, Edits and Writes. Bash reads (cat, sed)
  // and subagents' reads are not seen.
  const filesOf = (js, names) => new Set(js.flatMap((j) => j.uses.filter((u) => names.includes(u.name) && u.file).map((u) => u.file)));
  const contexts = [];
  const unjoined = { noPredTurnInWindow: 0, noTranscript: 0, noSuccessorJoined: 0 };
  const overlap = { handovers: 0, successorReads: 0, predecessorTouched: 0, reread: 0 };
  for (const h of handovers) {
    const predKey = turnByRun.get(h.run_id)?.session_key;
    if (!predKey) {
      unjoined.noPredTurnInWindow++;
      continue;
    }
    const pred = joinedByRun.get(h.run_id);
    const t0 = at(h.at);
    const last = pred?.main.filter((c) => c.t <= t0).at(-1);
    if (last) contexts.push(context(last));
    else unjoined.noTranscript++;
    const succ = h.child ? joinedByRun.get(firstTurnOf(h.child)?.run_id) : null;
    if (!succ) {
      unjoined.noSuccessorJoined++;
      continue;
    }
    const before = filesOf(joined.filter((j) => j.turn.session_key === predKey && at(j.turn.started_at) <= t0), ['Read', 'Edit', 'Write']);
    const after = filesOf([succ], ['Read']);
    overlap.handovers++;
    overlap.successorReads += after.size;
    overlap.predecessorTouched += before.size;
    overlap.reread += [...after].filter((f) => before.has(f)).length;
  }

  return {
    handover,
    transcripts: {
      turns: n,
      toolCalls: all.length,
      toolSearchPer100Turns: round2((searches.length * 100) / n),
      toolSearch,
      alwaysLoad,
      firstCall,
      waitOnlyCalls,
      bashRefusalRate,
      handoverContext: { ...dist(contexts), unjoined },
      handoverRereadOverlap: { ...overlap, share: share(overlap.reread, overlap.successorReads) },
      ...anatomyBlocks(joined, gaps),
      definitionLookups: definitionLookupsBlock(joined),
    },
  };
}

/** p50/p95/p99 and the sum of byte sizes. */
function bytesDist(values) {
  const v = values.filter((x) => Number.isFinite(x));
  return { n: v.length, p50: percentile(v, 0.5), p95: percentile(v, 0.95), p99: percentile(v, 0.99), total: sum(v) };
}

/** The context anatomy (OF19) over the tool pass's joined turns (main
 *  thread; `j.kind` and `j.gap` as firstCall sets them). Definitions:
 *  - cacheWrite.byCause: main-thread cache-write tokens by why they were
 *    written: `sessionFirst` (a new or reset turn's first call: a new native
 *    session), `coldResume` (a resume's first call over 60 minutes after the
 *    previous turn ended), `warmResume` (one within 60), `resumeUnknownGap`
 *    (the previous turn has no end), `incremental` (every other call), each
 *    with its share; `oneHourTokens`, the writes' 1-hour part.
 *  - coldResume: those over-60-minute resumes: turns, first-call cache write
 *    p50 against the warm resumes' p50, tokens and share of all writes.
 *  - carried: per kind, bytes/4 × the later main-thread calls that carry it
 *    (each call's `carried`, summed over the joined calls), with its share
 *    of the calls' context (input + cache write + cache read) in
 *    `contextTokens`; `toolResultsByTool` splits toolResults, and
 *    `pythonEditScripts` is the part of bashCommands that is inline Python
 *    writing a file. `measuredShare`: all kinds over the context (the rest
 *    is the system prompt, tool definitions and anything not measured).
 *  - pollOnlyCalls: main-thread calls whose every tool use is a wait
 *    (isPollOnly), with their context, overall and per permission mode.
 *  - pythonEditScripts: Bash calls, the inline Python edit scripts among
 *    them (per 100 Bash calls) and Edit/MultiEdit calls, per mode.
 *  - bashResultBytes and toolResultBytes.byTool: result sizes.
 *  - costShareAtLarge: weighted main-thread tokens on calls at or over
 *    LARGE_CONTEXT. */
function anatomyBlocks(joined, gaps) {
  const share = (k, d) => (d ? round4(k / d) : null);
  const main = joined.flatMap((j) => j.main);

  const causes = { sessionFirst: 0, coldResume: 0, warmResume: 0, resumeUnknownGap: 0, incremental: 0 };
  for (const j of joined) {
    const [first, ...rest] = j.main;
    const cause = j.kind !== 'resume' ? 'sessionFirst' : j.gap === 'gt60m' ? 'coldResume' : j.gap === 'unknown' ? 'resumeUnknownGap' : 'warmResume';
    causes[cause] += first.cacheWrite;
    causes.incremental += sum(rest.map((c) => c.cacheWrite));
  }
  const writes = sum(Object.values(causes));
  const cacheWrite = {
    tokens: writes,
    oneHourTokens: sum(main.map((c) => Math.min(c.cacheWrite1h ?? 0, c.cacheWrite))),
    byCause: Object.fromEntries(Object.entries(causes).map(([k, v]) => [k, { tokens: v, share: share(v, writes) }])),
  };
  const warm = [...gaps.lt5m, ...gaps.m5to60];
  const coldResume = {
    turns: gaps.gt60m.length,
    cacheWriteP50: percentile(gaps.gt60m.map((c) => c.cacheWrite), 0.5),
    warmCacheWriteP50: percentile(warm.map((c) => c.cacheWrite), 0.5),
    warmTurns: warm.length,
    cacheWriteTokens: causes.coldResume,
    share: share(causes.coldResume, writes),
  };

  const contextTokens = sum(main.map(context));
  const totals = {};
  for (const c of main) for (const [k, b] of Object.entries(c.carried ?? {})) totals[k] = (totals[k] ?? 0) + b / 4;
  const tok = (k) => Math.round(totals[k] ?? 0);
  const tools = Object.keys(totals).filter((k) => k.startsWith('tool:')).sort((a, b) => totals[b] - totals[a] || a.localeCompare(b));
  const carried = {
    calls: main.length,
    contextTokens,
    byKind: Object.fromEntries(CARRIED_KINDS.map((k) => [k, { tokens: tok(k), share: share(tok(k), contextTokens) }])),
    toolResultsByTool: Object.fromEntries(tools.slice(0, 20).map((k) => [k.slice(5), { tokens: tok(k), share: share(tok(k), contextTokens) }])),
    pythonEditScripts: { tokens: tok('pythonEditScripts'), share: share(tok('pythonEditScripts'), contextTokens) },
    measuredShare: share(sum(CARRIED_KINDS.map(tok)), contextTokens),
  };

  const modes = (fn) => {
    const out = { all: fn(joined) };
    for (const m of [...new Set(joined.map((j) => j.turn.permission_mode ?? 'unknown'))].sort()) out[m] = fn(joined.filter((j) => (j.turn.permission_mode ?? 'unknown') === m));
    return out;
  };
  const pollOnlyCalls = modes((js) => {
    const polls = js.flatMap((j) => j.main).filter((c) => isPollOnly(c.uses ?? []));
    return { calls: polls.length, contextTokens: sum(polls.map(context)) };
  });
  const pythonEditScripts = modes((js) => {
    const uses = js.flatMap((j) => j.uses);
    const bash = uses.filter((u) => u.name === 'Bash');
    const scripts = bash.filter((u) => u.pythonEdit).length;
    return { bashCalls: bash.length, scripts, per100Bash: per100(scripts, bash.length), editCalls: uses.filter((u) => u.name === 'Edit' || u.name === 'MultiEdit').length };
  });

  const uses = joined.flatMap((j) => j.uses).filter((u) => u.resultBytes != null);
  const byTool = {};
  for (const u of uses) (byTool[u.name] ??= []).push(u.resultBytes);
  const toolResultBytes = {
    byTool: Object.fromEntries(Object.entries(byTool).map(([k, v]) => [k, bytesDist(v)]).sort((a, b) => b[1].total - a[1].total || a[0].localeCompare(b[0]))),
  };
  const bashResultBytes = bytesDist(byTool.Bash ?? []);

  const w = sum(main.map(weight));
  const costShareAtLarge = w ? round4(sum(main.filter((c) => context(c) >= LARGE_CONTEXT).map(weight)) / w) : null;
  return { cacheWrite, coldResume, carried, pollOnlyCalls, pythonEditScripts, bashResultBytes, toolResultBytes, costShareAtLarge };
}

/** R8: a whole read this large (result bytes) is a large one. */
export const LARGE_READ_BYTES = 40_000;
/** R8: a read within this many uses after a definition search follows it. */
export const AFTER_SEARCH_USES = 3;

/** R8's definition lookups over one set of joined turns (main thread; the
 *  uses in call order). Counts only: no path or pattern is kept.
 *  - defSearches: Grep uses, and Bash grep/rg commands, with a
 *    definition-shaped pattern (DEF_PATTERN), and per 100 turns.
 *  - wholeLargeSourceReads: whole reads (a Read with neither offset nor
 *    limit, a Bash `cat <file>` with no pipe) of a source file (SOURCE_FILE)
 *    whose result is at least LARGE_READ_BYTES: count, bytes, and
 *    carriedTokens, Σ bytes/4 × the later main-thread calls of the turn in
 *    the same stretch between compactions (the anatomy's `segment`).
 *  - huntAfterSearch: those within AFTER_SEARCH_USES uses after a
 *    defSearch in the same turn; windowedAfterSearch: windowed reads of a
 *    source file (a Read with offset or limit, a `sed -n 'A,Bp'`) there.
 *  - tooLargeRetries: a Read error followed within 2 uses by a windowed
 *    Read of the same file (compared in memory).
 *  - shareOfContext: carriedTokens over the calls' context tokens. An upper
 *    bound on hunting: a whole read may serve more than one lookup. */
function lookupStats(js) {
  let defSearches = 0;
  let hunts = 0;
  let windowed = 0;
  let retries = 0;
  let carried = 0;
  const large = [];
  for (const j of js) {
    const flat = j.main.flatMap((c, i) => (c.uses ?? []).map((u) => ({ u, i })));
    flat.forEach(({ u, i }, k) => {
      if (u.defSearch) defSearches++;
      const afterSearch = flat.slice(Math.max(0, k - AFTER_SEARCH_USES), k).some((x) => x.u.defSearch);
      if (u.read === 'whole' && u.source && (u.resultBytes ?? 0) >= LARGE_READ_BYTES) {
        large.push(u.resultBytes);
        const later = j.main.slice(i + 1).filter((c) => c.segment === j.main[i].segment).length;
        carried += (u.resultBytes / 4) * later;
        if (afterSearch) hunts++;
      }
      if (afterSearch && u.read === 'windowed' && u.source) windowed++;
      // Any Read error counts, not only "too large": the error text is not
      // read, so a missing file retried with a window counts too.
      if (u.name === 'Read' && u.error === true && u.file != null && flat.slice(k + 1, k + 3).some((x) => x.u.name === 'Read' && x.u.read === 'windowed' && x.u.file === u.file)) retries++;
    });
  }
  const n = js.length;
  const contextTokens = sum(js.flatMap((j) => j.main).map(context));
  const carriedTokens = Math.round(carried);
  return {
    turns: n,
    contextTokens,
    defSearches,
    defSearchesPer100Turns: per100(defSearches, n),
    wholeLargeSourceReads: { count: large.length, bytes: sum(large), carriedTokens, per100Turns: per100(large.length, n) },
    huntAfterSearch: { count: hunts, per100Turns: per100(hunts, n) },
    windowedAfterSearch: { count: windowed, per100Turns: per100(windowed, n) },
    tooLargeRetries: retries,
    shareOfContext: contextTokens ? round4(carriedTokens / contextTokens) : null,
  };
}

/** `transcripts.definitionLookups` (R8): lookupStats over every joined turn
 *  (`all`: the tool pass covers the chat/claude group only) and over the
 *  long-run (lane) turns, those running LONG_RUN_MS or more from the turn's
 *  start to its end (else its last main-thread call). */
function definitionLookupsBlock(joined) {
  const wall = (j) => {
    const start = at(j.turn.started_at);
    return start == null ? null : (at(j.turn.ended_at) ?? j.main.at(-1).t) - start;
  };
  return {
    largeReadBytes: LARGE_READ_BYTES,
    afterSearchUses: AFTER_SEARCH_USES,
    longRunMs: LONG_RUN_MS,
    all: lookupStats(joined),
    longRuns: lookupStats(joined.filter((j) => (wall(j) ?? 0) >= LONG_RUN_MS)),
  };
}

/** Read everything for `vault` and summarize it. `suggestionsFile`: the
 *  Main agent's suggestion log (default: the one beside the database, when
 *  it sits in a `Sessions` folder). */
export async function summaryFromVault({ vault, dbFile, transcriptRoots, codexHome = null, since, until, overlap = false, suggestionsFile }) {
  const window = { since, until, overlap };
  const notes = [];
  const file = dbFile ?? defaultDb(vault);
  let dbRows = { ledger: [], straddling: [], queue: [], waits: [], sessions: [], turns: [], allTurns: [], stints: [], allStints: [], resets: [], steps: [], stepErrors: [], refusals: [], pulse: [], delegations: [], friction: [], ledgerColumns: { usageJson: false, costFloorUsd: false } };
  if (existsSync(file)) {
    const { db, mode, notes: n } = await openDb(file);
    notes.push(...n);
    if (mode === 'immutable') notes.push('database opened immutable (read-only open refused)');
    dbRows = readDb(db, window);
    db.close();
  } else notes.push('no sessions database found: ledger, turns and stints are empty');
  const receiptsRead = readReceipts(vault);
  if (receiptsRead == null) notes.push('.evidence-receipts.json could not be parsed: receipts skipped');
  const ids = new Set(dbRows.ledger.map((r) => r.run_id).filter(Boolean));
  // With --overlap a run that began before the window keeps its receipt.
  const receipts = (receiptsRead ?? []).filter((r) => {
    const t = at(r.startedAt);
    return (t != null && t >= since && t < until) || (overlap && ids.has(r.runId ?? r.historyRunId));
  });
  const manifests = readManifests(vault, ids);
  const transcripts = new Map();
  const workflows = new Map();
  // run id → the end of the turn before its turn in the same session: a
  // slice starts after it, so the previous turn's last call (inside the
  // 1 s slack) can't become this turn's first call.
  const prevEnd = new Map();
  const bySession = new Map();
  for (const t of dbRows.allTurns ?? dbRows.turns) (bySession.get(t.session_key) ?? bySession.set(t.session_key, []).get(t.session_key)).push(t);
  for (const list of bySession.values()) {
    list.sort((x, y) => at(x.started_at) - at(y.started_at) || x.ord - y.ord);
    for (let k = 1; k < list.length; k++) if (list[k].run_id && at(list[k - 1].ended_at) != null) prevEnd.set(list[k].run_id, at(list[k - 1].ended_at));
  }
  /** A finished row's turn slice: from its start (after the previous turn's end) to its end, 1 s slack. */
  const sliceOf = (row) => {
    const before = prevEnd.get(row.run_id);
    return [Math.max(at(row.started_at) - 1000, before == null ? -Infinity : before + 1), at(row.at) + 1000];
  };
  if (transcriptRoots.length) {
    const idx = indexTranscripts(transcriptRoots);
    const read = new Map();
    // The anatomy measures main-thread files only (a subagent's thread is its own).
    const callsIn = (f, side) => read.get(f) ?? read.set(f, readCalls(f, side, { toolPass: true, anatomy: !side, memoryRoots: transcriptRoots })).get(f);
    for (const row of dbRows.ledger) {
      // A running row has no end yet; an interrupted one's `at` is when the
      // reconciler noticed, which can cover later turns of the session.
      if (row.agent !== 'claude' || !row.native_session || row.outcome === 'running' || row.outcome === 'interrupted') continue;
      const files = idx.get(row.native_session);
      if (!files?.main) continue;
      const [from, to] = sliceOf(row);
      const calls = [];
      for (const [f, side] of [[files.main, false], ...files.subagents.map((s) => [s, true])]) {
        for (const c of await callsIn(f, side)) if (c.t >= from && c.t <= to) calls.push(c);
      }
      if (calls.some((c) => !c.sidechain)) transcripts.set(row.run_key, calls);
    }
    // Workflow agents (usage only), by native session over the window, not
    // a turn's slice: the Workflow tool runs in the background, so its
    // agents go on calling after the turn that started them has ended (and
    // a lane's turn may still be running).
    const sessions = new Set(dbRows.ledger.filter((r) => r.agent === 'claude' && r.native_session).map((r) => r.native_session));
    for (const s of sessions) {
      const wf = [];
      for (const f of idx.get(s)?.workflows ?? []) {
        const cs = (await readCalls(f, true)).filter((c) => c.t >= since && c.t < until);
        if (cs.length) wf.push({ file: f, calls: cs });
      }
      if (wf.length) workflows.set(s, wf);
    }
  }
  // A14: Codex runs, by their thread's rollout (see "The Codex pass").
  const codex = new Map();
  if (codexHome) {
    const idx = indexCodexRollouts(codexHome);
    const read = new Map();
    const rollout = (f) => read.get(f) ?? read.set(f, readCodexCalls(f)).get(f);
    for (const row of dbRows.ledger) {
      if (row.agent !== 'codex' || !row.native_session || row.outcome === 'running' || row.outcome === 'interrupted') continue;
      const f = idx.get(row.native_session);
      if (!f) continue;
      const [from, to] = sliceOf(row);
      const inside = (t) => t >= from && t <= to;
      const r = await rollout(f);
      const calls = r.calls.filter((c) => inside(c.t));
      if (!calls.length) continue;
      transcripts.set(row.run_key, calls);
      codex.set(row.run_key, { compactions: r.compactions.filter(inside).length, tools: r.tools.filter((x) => inside(x.t)).map((x) => x.name) });
    }
  }
  const suggestions = readSuggestions(suggestionsFile === undefined ? suggestionsFileFor(file) : suggestionsFile, until);
  if (suggestions?.unreadable) notes.push('the Main agent suggestion log could not be parsed: suggestions skipped');
  const learnings = readLearnings(vault, window);
  return summarize({ db: dbRows, receipts, manifests, transcripts, workflows, codex, window, notes, suggestions: suggestions?.unreadable ? null : suggestions, learnings });
}

// ── Compare ──────────────────────────────────────────────────────────────

const change = (a, b) => (a == null || b == null || a === 0 ? null : round4((b - a) / a));
/** Two p50/p95/n blocks (either null or missing) and their changes. */
const distPair = (x, y) => {
  const side = (d) => (d ? { p50: d.p50 ?? null, p95: d.p95 ?? null, n: d.n ?? null } : null);
  return { a: side(x), b: side(y), p50Change: change(x?.p50, y?.p50), p95Change: change(x?.p95, y?.p95) };
};
/** S27: two `verification.claims` blocks' n and per-100-run figures (a
 *  block that is null, or missing in an older scorecard, reads null). */
const claimsPair = (x, y) => {
  const side = (v, k) => v?.per100Runs?.[k] ?? null;
  return {
    n: { a: x?.n ?? null, b: y?.n ?? null },
    ...Object.fromEntries(['claims', 'claimWithoutCheck', 'claimAfterFail'].map((k) => [`${k}Per100Runs`, { a: side(x, k), b: side(y, k) }])),
  };
};

/** A schema-1 scorecard keyed Jev requests `oneshot/unknown` (agent NULL)
 *  and counted them as unknown cost: read it as schema 2's `oneshot/jev`.
 *  The whole group is assumed Jev: when schema-1 files were written, no
 *  agent-NULL oneshot row other than Jev's existed. A schema-2 file keeps
 *  any other agent-NULL oneshot (a title, say) in `oneshot/unknown`. */
export function upgradeScorecard(s) {
  if ((s.schema ?? 1) >= 2 || !s.groups?.['oneshot/unknown'] || s.groups['oneshot/jev']) return s;
  const { 'oneshot/unknown': legacy, ...rest } = s.groups;
  const cost = { ...legacy.cost, unknownRuns: 0, unknownByStatus: {}, unpricedByDesign: legacy.cost?.unknownRuns ?? 0 };
  return { ...s, groups: Object.fromEntries(Object.entries({ ...rest, 'oneshot/jev': { ...legacy, cost } }).sort()) };
}

/** B against A: per group, latency p50/p95 changes (flag a p95 rise over
 *  10%), success with a Newcombe interval, cost, cache and tools; and the
 *  continuity and injected-bytes headline; OF28's peak context, API calls
 *  and partial-run floor (never flagged). Groups in one only are listed.
 *  A pair with under LOW_N finished runs on either side is marked low-n,
 *  and so are the flags it raises. */
export function compareScorecards(a0, b0, { p95Rise = 0.1 } = {}) {
  for (const s of [a0, b0]) if (s?.kind !== KIND) throw new Error('compare takes two scorecard JSON files (kind hollow-scorecard)');
  const [a, b] = [upgradeScorecard(a0), upgradeScorecard(b0)];
  const flags = [];
  const groups = {};
  for (const key of Object.keys(a.groups).filter((k) => b.groups[k]).sort()) {
    const ga = a.groups[key];
    const gb = b.groups[key];
    // Over finished runs, as the rates are (older scorecards: all runs).
    const [na, nb] = [ga.finished ?? ga.runs, gb.finished ?? gb.runs];
    const lowN = na < LOW_N || nb < LOW_N;
    const flag = (f) => flags.push(lowN ? { ...f, lowN } : f);
    const latency = {};
    for (const k of TIMINGS) {
      // A key a side predates (OF28's firstModelMs) is null on that side.
      const la = ga.latency?.[k] ?? {};
      const lb = gb.latency?.[k] ?? {};
      latency[k] = { a: { p50: la.p50 ?? null, p95: la.p95 ?? null, n: la.n ?? null }, b: { p50: lb.p50 ?? null, p95: lb.p95 ?? null, n: lb.n ?? null }, p50Change: change(la.p50, lb.p50), p95Change: change(la.p95, lb.p95) };
      // A p95 over fewer than LOW_N samples is low-n even in a large group
      // (a group whose runs are mostly long keeps few latency samples).
      if (latency[k].p95Change != null && latency[k].p95Change > p95Rise) {
        const f = { kind: 'p95-rise', group: key, metric: k, change: latency[k].p95Change };
        if (!lowN && (la.n < LOW_N || lb.n < LOW_N)) flags.push({ ...f, lowN: true });
        else flag(f);
      }
    }
    const success = newcombe(ga.outcomes.success ?? 0, na, gb.outcomes.success ?? 0, nb);
    if (success && success.hi < 0) flag({ kind: 'success-drop', group: key, diff: success.diff });
    // Error and cancel rates: flagged only when the change is outside the
    // Newcombe interval (zero not inside it).
    const rateChanges = {};
    for (const o of ['error', 'cancelled', 'timeout']) {
      const d = newcombe(ga.outcomes[o] ?? 0, na, gb.outcomes[o] ?? 0, nb);
      rateChanges[o] = d;
      if (d && d.lo > 0) flag({ kind: `${o}-rise`, group: key, diff: d.diff });
    }
    // Tool errors per 100 calls (OF20): a change outside the Newcombe
    // interval of errors over calls is flagged either way. An older side
    // without the key is worked out from its errors and calls.
    const toolPer100 = (t) => t?.errorsPer100Calls ?? (t?.calls ? per100(t.errors, t.calls) : null);
    const te = ga.tools?.calls && gb.tools?.calls ? newcombe(Math.min(ga.tools.errors, ga.tools.calls), ga.tools.calls, Math.min(gb.tools.errors, gb.tools.calls), gb.tools.calls) : null;
    const tePair = { a: toolPer100(ga.tools), b: toolPer100(gb.tools) };
    if (te && te.lo > 0) flag({ kind: 'tool-errors-per-100-calls-rise', group: key, change: `${tePair.a} → ${tePair.b}` });
    if (te && te.hi < 0) flag({ kind: 'tool-errors-per-100-calls-fall', group: key, change: `${tePair.a} → ${tePair.b}` });
    groups[key] = {
      runs: { a: ga.runs, b: gb.runs },
      n: { a: na, b: nb },
      lowN,
      success,
      rateChanges,
      latency,
      cost: {
        p50Change: change(ga.cost.p50Usd, gb.cost.p50Usd),
        p95Change: change(ga.cost.p95Usd, gb.cost.p95Usd),
        unknownRuns: { a: ga.cost.unknownRuns, b: gb.cost.unknownRuns },
        unpricedByDesign: { a: ga.cost.unpricedByDesign ?? 0, b: gb.cost.unpricedByDesign ?? 0 },
      },
      cacheReadShare: { a: ga.tokens.cacheReadShare, b: gb.tokens.cacheReadShare },
      toolErrorsPerTurn: { a: ga.tools?.errorsPerTurn ?? null, b: gb.tools?.errorsPerTurn ?? null },
      toolErrorsPer100Calls: { ...tePair, diff: te },
      maskedFailures: { a: ga.tools?.maskedFailures?.masked ?? null, b: gb.tools?.maskedFailures?.masked ?? null },
      contextPerCallP50: { a: ga.transcripts?.contextPerCall.p50 ?? null, b: gb.transcripts?.contextPerCall.p50 ?? null },
      costShareAtLarge: { a: ga.transcripts?.costShareAtLarge ?? null, b: gb.transcripts?.costShareAtLarge ?? null },
      // OF28 (null on a side that predates it or carries none).
      peakContext: distPair(ga.ledger?.peakContext, gb.ledger?.peakContext),
      partialFloorUsd: { a: ga.cost?.partialFloorUsd ?? null, b: gb.cost?.partialFloorUsd ?? null },
      // S27 (never flagged: a proxy).
      claims: claimsPair(ga.verification?.claims, gb.verification?.claims),
    };
  }
  const ca = a.continuity.all;
  const cb = b.continuity.all;
  const pair = (k, x = ca, y = cb) => ({ a: x?.[k] ?? null, b: y?.[k] ?? null });
  const ma = a.continuity.mainAgent;
  const mb = b.continuity.mainAgent;
  const turnsLowN = !(ca?.turns >= LOW_N && cb?.turns >= LOW_N);
  const continuity = {
    turns: pair('turns'),
    lowN: turnsLowN,
    resetsPer100Turns: pair('resetsPer100Turns'),
    configChangedPer100Turns: pair('configChangedPer100Turns'),
    flipResetsPer100Turns: pair('flipResetsPer100Turns'),
    mainAgentConfigChangedPer100Turns: pair('configChangedPer100Turns', ma, mb),
    // Counts, not rates; a summary from before OF03 has none (null).
    resetsByPart: { a: ca?.resetsByPart ?? null, b: cb?.resetsByPart ?? null },
    // Resets per turn, as a rate: a change outside the interval is real.
    resets: ca && cb && ca.turns && cb.turns ? newcombe(Math.min(ca.resets, ca.turns), ca.turns, Math.min(cb.resets, cb.turns), cb.turns) : null,
  };
  const turnsFlag = (f) => flags.push(turnsLowN ? { ...f, lowN: true } : f);
  if (continuity.resets && continuity.resets.lo > 0) turnsFlag({ kind: 'resets-rise', diff: continuity.resets.diff });
  if (continuity.resets && continuity.resets.hi < 0) turnsFlag({ kind: 'resets-fall', diff: continuity.resets.diff });
  const injected = {
    totalP50: { a: a.injected.total.p50, b: b.injected.total.p50 },
    totalP95: { a: a.injected.total.p95, b: b.injected.total.p95 },
    resetP50: { a: a.injected.bySession?.reset?.total.p50 ?? null, b: b.injected.bySession?.reset?.total.p50 ?? null },
    workspaceCutRate: { a: a.injected.workspaceCut?.rate ?? null, b: b.injected.workspaceCut?.rate ?? null },
  };
  const askA = a.delegation?.askAgent;
  const askB = b.delegation?.askAgent;
  const askAgentTimeouts = askA && askB ? newcombe(askA.timeouts, askA.runs, askB.timeouts, askB.runs) : null;
  if (askAgentTimeouts && askAgentTimeouts.lo > 0) {
    flags.push(askA.runs < LOW_N || askB.runs < LOW_N ? { kind: 'ask-agent-timeouts-rise', diff: askAgentTimeouts.diff, lowN: true } : { kind: 'ask-agent-timeouts-rise', diff: askAgentTimeouts.diff });
  }
  // Blocks a schema-1 scorecard lacks are null on its side.
  const queue = {
    waits: { a: a.queue?.waits ?? null, b: b.queue?.waits ?? null },
    maxWaitsPerSecond: { a: a.queue?.maxWaitsPerSecond ?? null, b: b.queue?.maxWaitsPerSecond ?? null },
  };
  const longRuns = { a: a.longRuns?.runs ?? null, b: b.longRuns?.runs ?? null };
  // The tool pass (OF02): null on a side whose scorecard predates it.
  const [ta, tb] = [a.transcripts, b.transcripts];
  const tools = {
    turns: { a: ta?.turns ?? null, b: tb?.turns ?? null },
    toolSearchPer100Turns: { a: ta?.toolSearchPer100Turns ?? null, b: tb?.toolSearchPer100Turns ?? null },
    firstCallColdShare: Object.fromEntries(['new', 'reset', 'resume'].map((k) => [k, { a: ta?.firstCall?.bySession?.[k]?.coldShare ?? null, b: tb?.firstCall?.bySession?.[k]?.coldShare ?? null, n: { a: ta?.firstCall?.bySession?.[k]?.n ?? null, b: tb?.firstCall?.bySession?.[k]?.n ?? null } }])),
    // OF20: per agent, from `toolErrors` (null on a side that predates it).
    errorsPer100CallsByAgent: Object.fromEntries(
      [...new Set([...Object.keys(a.toolErrors?.byAgent ?? {}), ...Object.keys(b.toolErrors?.byAgent ?? {})])].sort().map((k) => [k, { a: a.toolErrors?.byAgent?.[k]?.all?.errorsPer100Calls ?? null, b: b.toolErrors?.byAgent?.[k]?.all?.errorsPer100Calls ?? null }]),
    ),
    maskedFailures: { a: a.toolErrors?.maskedFailures?.masked ?? null, b: b.toolErrors?.maskedFailures?.masked ?? null },
  };
  // SC4's headline numbers (null on a side that predates them): the S08
  // proxies with their n, the Main agent's wakes' read_session per turn and
  // native memory writes against learnings accepted. Never flagged: proxies.
  const ratePair = (f) => ({ a: f(a)?.share ?? null, b: f(b)?.share ?? null, n: { a: f(a)?.n ?? null, b: f(b)?.n ?? null } });
  const stopped = (s) => s.outcomes?.stoppedBeforeText;
  const outcomes = {
    reaskShare: ratePair((s) => s.outcomes?.reaskWithin120s),
    // The stopped turns without timings beside the share: its n leaves them out.
    stoppedBeforeTextShare: { ...ratePair(stopped), unknownTiming: { a: stopped(a)?.unknownTiming ?? null, b: stopped(b)?.unknownTiming ?? null } },
  };
  const wake = (s) => s.mainAgent?.byKind?.wake;
  const mainAgent = {
    status: { a: a.mainAgent?.status ?? null, b: b.mainAgent?.status ?? null },
    wakeReadSessionPerTurn: { a: wake(a)?.readSessionPerTurn ?? null, b: wake(b)?.readSessionPerTurn ?? null, n: { a: wake(a)?.turns ?? null, b: wake(b)?.turns ?? null } },
  };
  const memory = {
    nativeWrites: { a: a.memory?.nativeWrites ?? null, b: b.memory?.nativeWrites ?? null },
    learningsAccepted: { a: a.memory?.learningsAccepted ?? null, b: b.memory?.learningsAccepted ?? null },
  };
  // OF28's ledger figures over the whole window, and the partial-run floor
  // summed over the groups (null on a side where no run carries them).
  const floorOf = (s) => {
    const f = Object.values(s.groups).map((g) => g.cost?.partialFloorUsd).filter((x) => num(x) != null);
    return f.length ? round4(sum(f)) : null;
  };
  const ledger = {
    peakContext: distPair(a.ledger?.peakContext, b.ledger?.peakContext),
    apiCalls: distPair(a.ledger?.apiCalls, b.ledger?.apiCalls),
    subagentCalls: { a: a.ledger?.subagents?.calls ?? null, b: b.ledger?.subagents?.calls ?? null },
    partialFloorUsd: { a: floorOf(a), b: floorOf(b) },
  };
  const only = [...Object.keys(a.groups).filter((k) => !b.groups[k]).map((k) => `A:${k}`), ...Object.keys(b.groups).filter((k) => !a.groups[k]).map((k) => `B:${k}`)];
  // S27's pass claims over the window (null on a side that predates them).
  const verification = claimsPair(a.verification?.claims, b.verification?.claims);
  return { schema: SCHEMA, kind: `${KIND}-comparison`, schemas: { a: a0.schema ?? 1, b: b0.schema ?? 1 }, a: a.window, b: b.window, groups, continuity, injected, askAgentTimeouts, queue, longRuns, tools, outcomes, mainAgent, memory, ledger, verification, flags, unpaired: only };
}

// ── Markdown ─────────────────────────────────────────────────────────────

const s1 = (msv) => (msv == null ? '—' : `${(msv / 1000).toFixed(1)} s`);
const pc = (x) => (x == null ? '—' : `${(x * 100).toFixed(1)}%`);
const kb = (b) => (b == null ? '—' : `${(b / 1000).toFixed(1)} KB`);
const ktok = (t) => (t == null ? '—' : `${Math.round(t / 1000)}K`);
const usd = (c) => {
  if (c.totalUsd != null) return `$${c.totalUsd.toFixed(2)}`;
  const unpriced = c.unpricedByDesign ?? 0;
  if (!c.unknownRuns && unpriced) return `unpriced by design (${unpriced})${c.knownRuns ? `; $${c.floorUsd.toFixed(2)} known` : ''}`;
  return `unknown (${c.unknownRuns} without a cost${unpriced ? `, ${unpriced} unpriced by design` : ''}); $${c.floorUsd.toFixed(2)} known`;
};
/** A rate with its count and n: `50.0% (3/6)`. */
const pcn = (k, n) => (n ? `${pc(k / n)} (${k}/${n})` : '—');
const runsWord = (n) => `${n} run${n === 1 ? '' : 's'}`;
const counted = (o) => Object.entries(o).map(([k, n]) => `${k} ${n}`).join(', ');

export function renderSummary(s) {
  const out = [`# Efficiency scorecard ${s.window.since.slice(0, 16)} → ${s.window.until.slice(0, 16)}${s.window.overlap ? ' (runs overlapping the window)' : ''}`, ''];
  const c = s.counts;
  out.push(`Ledger rows ${c.ledgerRows}; receipts joined ${c.receiptsJoined}; manifests ${c.manifestsJoined}; transcripts ${c.transcriptRunsJoined}; turns ${c.turns}; stints ${c.stints}${c.stillRunning != null ? `; still running ${c.stillRunning}` : ''}.`, '');
  // Tool errors per top-level turn and per 100 tool calls (OF20), each
  // under its own header.
  out.push('| Surface/agent | Runs | Success | Cancel | Error | Timeout | First text p50/p95 | Turn p50/p95 | Prep p50 | Queue p95 | Cost | Cache read | Tool errors/turn | Tool errors/100 calls |', '|---|---:|---:|---:|---:|---:|---|---|---|---|---|---:|---:|---:|');
  for (const [k, g] of Object.entries(s.groups)) {
    const l = g.latency;
    const n = g.finished ?? g.runs;
    const rate = (o) => pcn(g.outcomes[o] ?? 0, n);
    out.push(`| ${k} | ${g.runs} | ${rate('success')} | ${rate('cancelled')} | ${rate('error')} | ${rate('timeout')} | ${s1(l.firstTextMs.p50)} / ${s1(l.firstTextMs.p95)} | ${s1(l.turnMs.p50)} / ${s1(l.turnMs.p95)} | ${s1(l.preparationMs.p50)} | ${s1(l.queueWaitMs.p95)} | ${usd(g.cost)} | ${pc(g.tokens.cacheReadShare)} | ${g.tools?.errorsPerTurn ?? '—'} | ${g.tools ? `${g.tools.errorsPer100Calls ?? '—'} (${g.tools.errors}/${g.tools.calls})` : '—'} |`);
  }
  out.push('');
  for (const [, g] of Object.entries(s.groups).filter(([, g]) => g.cost.unpricedByDesign)) {
    out.push(`- Jev requests: ${g.cost.unpricedByDesign}, latency p50/p95 ${s1(g.wallMs?.p50)} / ${s1(g.wallMs?.p95)}; ${counted(g.outcomes)}; unpriced by design (Jev reports no cost).`);
  }
  const st = s.straddling;
  if (st) out.push(`- Straddling the window's start: ${runsWord(st.runs)}, ${usd(st.cost)}${st.runs ? ` (${counted(st.groups)}; ${s.window.overlap ? 'counted in the groups' : 'not in the groups; --overlap counts them'})` : ''}.`);
  const lr = s.longRuns;
  if (lr) out.push(`- Long runs (over ${lr.thresholdMs / 60_000} min, out of latency): ${runsWord(lr.runs)}, ${usd(lr.cost)}${lr.runs ? `; minutes p50/p95 ${lr.minutes.p50} / ${lr.minutes.p95} (${counted(lr.groups)})` : ''}.`);
  const q = s.queue;
  const maxRate = (v) => (v.maxWaitsPerSecond == null ? '—/s' : `${v.maxWaitsPerSecond}/s (${v.maxRateWaits} waits)`);
  if (q) out.push(`- Queue waits: ${q.waits} over ${q.rows} rows, max ${maxRate(q)}${q.rows ? ` (${Object.entries(q.byHolder).map(([h, v]) => `${h} ${v.waits} over ${v.rows}, max ${maxRate(v)}`).join('; ')})` : ''}.`);
  // OF28: a key no run in the window carries is "not installed", never 0,
  // unless the store has its column (only the floor's can be told: usage_json
  // predates OB05, and receipts can't say either): then "none in this window".
  const lg = s.ledger;
  if (lg) {
    const missing = (n) => `not installed (0 of ${runsWord(n)} carry it)`;
    const pd = (d, f) => `p50/p95 ${f(d.p50)} / ${f(d.p95)} (n ${d.n} of ${runsWord(lg.runs)})`;
    const sa = lg.subagents;
    const subs = lg.peakContext || lg.apiCalls ? `; subagents ${sa ? `in ${runsWord(sa.runs)}, ${sa.calls} calls, ${ktok(sa.tokens)} tokens` : 'none'}` : '';
    out.push(`- Peak context over top-level runs: ${lg.peakContext ? pd(lg.peakContext, ktok) : missing(lg.runs)}; API calls per run: ${lg.apiCalls ? pd(lg.apiCalls, (x) => x) : missing(lg.runs)}${subs}.`);
    const floors = Object.values(s.groups).map((g) => g.cost).filter((x) => x?.partialFloorUsd != null);
    const noFloor = s.sources?.costFloorUsd ? `none in this window (0 of ${runsWord(c.ledgerRows)})` : missing(c.ledgerRows);
    out.push(`- Partial-run cost floor (never in known cost): ${floors.length ? `$${sum(floors.map((x) => x.partialFloorUsd)).toFixed(2)} over ${runsWord(sum(floors.map((x) => x.partialFloorRuns)))}` : noFloor}.`);
    const fm = Object.entries(s.groups).filter(([, g]) => g.latency.firstModelMs?.n);
    const tbt = (g) => (g.toolsBeforeText?.n ? `, tools before text p50 ${g.toolsBeforeText.p50}` : '');
    out.push(`- First model event${fm.length ? ` p50/p95: ${fm.map(([k, g]) => `${k} ${s1(g.latency.firstModelMs.p50)} / ${s1(g.latency.firstModelMs.p95)} (n ${g.latency.firstModelMs.n})${tbt(g)}`).join('; ')}` : `: ${missing(c.ledgerRows)}`}.`);
  }
  const t = Object.entries(s.groups).filter(([, g]) => g.transcripts);
  if (t.length) {
    out.push('', '| Transcripts | Runs | Calls/turn p50/p95 | Context/call p50/p95 | Cache writes | Calls ≥400K | Cost share ≥400K | Transcript/ledger |', '|---|---:|---|---|---:|---:|---:|---:|');
    for (const [k, g] of t) {
      const x = g.transcripts;
      out.push(`| ${k} | ${x.joinedRuns} | ${x.callsPerTurn.p50} / ${x.callsPerTurn.p95} | ${ktok(x.contextPerCall.p50)} / ${ktok(x.contextPerCall.p95)} | ${ktok(x.cacheWriteTokens)} | ${x.largeContextCalls} | ${pc(x.costShareAtLarge)} | ${x.reconciliation.ratio ?? '—'} |`);
    }
  }
  out.push('', '| Continuity | Turns | Stints | Resets/100 turns | Config changed/100 | Mode-flip resets/100 | Causes |', '|---|---:|---:|---:|---:|---:|---|');
  for (const [k, v] of Object.entries(s.continuity)) out.push(`| ${k} | ${v.turns} | ${v.stints} | ${v.resetsPer100Turns ?? '—'} | ${v.configChangedPer100Turns ?? '—'} | ${v.flipResetsPer100Turns ?? '—'} | ${Object.entries(v.causes).map(([r, n]) => `${r} ${n}`).join(', ') || '—'} |`);
  const byPart = Object.entries(s.continuity.all?.resetsByPart ?? {});
  if (byPart.length) out.push('', `Config-changed resets by what changed: ${byPart.map(([p, n]) => `${p} ${n}`).join(', ')}.`);
  const cut = s.injected.workspaceCut;
  out.push('', `Injected context per run: p50 ${kb(s.injected.total.p50)}, p95 ${kb(s.injected.total.p95)} over ${s.injected.runs} manifests; workspace brief cut by the budget in ${cut ? `${cut.cut} of ${cut.runs}` : '—'}.`);
  for (const [st, v] of Object.entries(s.injected.bySession)) out.push(`- ${st}: ${v.runs} runs, p50 ${kb(v.total.p50)}, p95 ${kb(v.total.p95)}`);
  const ret = Object.entries(s.groups).filter(([, g]) => g.retrieval);
  for (const [k, g] of ret) out.push(`- Retrieval ${k}: notes read p50 ${g.retrieval.notesRead.p50 ?? '—'}, bytes read p50 ${kb(g.retrieval.bytesRead.p50)}, prefilter pass p50 ${pc(g.retrieval.prefilterPassRate.p50)} (n ${g.retrieval.prefilterPassRate.n}); retrieval p50/p95 ${s1(g.latency.retrievalMs.p50)} / ${s1(g.latency.retrievalMs.p95)}`);
  const d = s.delegation;
  if (d) out.push(`- Delegation: ask_agent ${d.askAgent.runs} runs, ${d.askAgent.timeouts} timed out (${pc(d.askAgent.timeoutRate)} of ${d.askAgent.runs}); ${Object.entries(d.records).map(([k, n]) => `${k} ${n}`).join(', ') || 'no records'}.`);
  const ok = d?.handover?.childFirstTurnOk;
  if (ok) out.push(`- Handover successors whose first turn ended ok: ${pcn(ok.ok, ok.n)}.`);
  const tp = s.transcripts;
  if (tp) {
    const al = tp.alwaysLoad;
    const w = tp.waitOnlyCalls;
    const fc = (v) => `cache write/read p50 ${ktok(v.cacheWriteP50)} / ${ktok(v.cacheReadP50)}, cold ${pc(v.coldShare)} (n ${v.n})`;
    out.push(`- Tools over ${tp.turns} Claude turns: ToolSearch ${tp.toolSearchPer100Turns} per 100 turns, in ${pcn(tp.toolSearch.turnsWith, tp.turns)} of turns; always-loaded tools used in ${Object.entries(al.usedShare).map(([k, v]) => `${k} ${pc(v)}`).join(', ')}.`);
    out.push(`- Waits: ${w.calls} wait_for_session calls, ${pcn(w.stillRunning, w.calls)} ended with the child still running. Bash permission refusals: ${Object.entries(tp.bashRefusalRate).map(([m, v]) => `${m} ${pcn(v.refused, v.bashCalls)}`).join(', ') || '—'}.`);
    out.push(`- First calls: ${Object.entries(tp.firstCall.bySession).map(([k, v]) => `${k} ${fc(v)}`).join('; ')}. Resumes by gap: ${Object.entries(tp.firstCall.resumeByGap).filter(([, v]) => v.n).map(([k, v]) => `${k} ${fc(v)}`).join('; ') || '—'}.`);
    const ho = tp.handoverRereadOverlap;
    out.push(`- Handovers: predecessor context p50/p95 ${ktok(tp.handoverContext.p50)} / ${ktok(tp.handoverContext.p95)} (n ${tp.handoverContext.n}); successor first-turn reads the predecessor had touched ${pcn(ho.reread, ho.successorReads)} over ${ho.handovers} handovers.`);
    if (tp.cacheWrite) {
      const cw = tp.cacheWrite;
      const cr = tp.coldResume;
      out.push(`- Main-thread cache writes ${ktok(cw.tokens)} (1-hour ${pc(cw.tokens ? cw.oneHourTokens / cw.tokens : null)}) by cause: ${Object.entries(cw.byCause).map(([k, v]) => `${k} ${pc(v.share)}`).join(', ')}. Cold resumes (over 60 min): ${cr.turns} turns, first-call write p50 ${ktok(cr.cacheWriteP50)} vs warm ${ktok(cr.warmCacheWriteP50)} (n ${cr.warmTurns}).`);
      const ca = tp.carried;
      const kinds = Object.entries(ca.byKind).filter(([, v]) => v.tokens).sort((a, b) => b[1].tokens - a[1].tokens);
      out.push(`- Carried context (bytes/4 × later calls, of ${ktok(ca.contextTokens)} over ${ca.calls} calls): ${kinds.map(([k, v]) => `${k} ${pc(v.share)}`).join(', ') || '—'}; tool results by tool ${Object.entries(ca.toolResultsByTool).slice(0, 4).map(([k, v]) => `${k} ${pc(v.share)}`).join(', ') || '—'}; Python edit scripts ${pc(ca.pythonEditScripts.share)} (${ktok(ca.pythonEditScripts.tokens)}); measured ${pc(ca.measuredShare)}.`);
      const po = tp.pollOnlyCalls.all;
      const py = tp.pythonEditScripts.all;
      out.push(`- Poll-only calls ${po.calls} re-reading ${ktok(po.contextTokens)}; Python edit scripts ${py.scripts} in ${py.bashCalls} Bash calls (${py.per100Bash ?? '—'} per 100) against ${py.editCalls} Edit calls; Bash result bytes p50/p95/p99 ${tp.bashResultBytes.p50 ?? '—'} / ${tp.bashResultBytes.p95 ?? '—'} / ${tp.bashResultBytes.p99 ?? '—'}; main-thread cost share ≥400K ${pc(tp.costShareAtLarge)}.`);
    }
    const dl = tp.definitionLookups;
    if (dl) {
      const line = (x) => `${x.defSearches} definition searches (${x.defSearchesPer100Turns ?? '—'} per 100 turns); ${x.wholeLargeSourceReads.count} whole large source reads (${ktok(x.wholeLargeSourceReads.bytes / 4)} read, ${ktok(x.wholeLargeSourceReads.carriedTokens)} carried, ${pc(x.shareOfContext)} of context); hunts after a search ${x.huntAfterSearch.count} (${x.huntAfterSearch.per100Turns ?? '—'} per 100), windowed reads after one ${x.windowedAfterSearch.count}; too-large retries ${x.tooLargeRetries}`;
      out.push(`- Definition lookups (upper bound) over ${dl.all.turns} turns: ${line(dl.all)}. Long-run turns (${dl.longRuns.turns}): ${dl.longRuns.turns ? line(dl.longRuns) : '—'}.`);
    }
    const wa = tp.workflowAgents;
    if (wa) out.push(`- Workflow agents: ${wa.agents} in ${wa.sessions} sessions, ${wa.calls} calls, ${pc(wa.weightShare)} of joined weighted tokens${wa.usdAtJoinedRate != null ? `, about $${wa.usdAtJoinedRate.toFixed(2)} at the joined runs' rate` : ''}.`);
  }
  const te = s.toolErrors;
  if (te && Object.keys(te.byAgent).length) {
    out.push(`- Tool errors per 100 calls: ${Object.entries(te.byAgent).map(([k, v]) => `${k} ${v.all.errorsPer100Calls ?? '—'} (${v.all.errors}/${v.all.calls}; expected non-zero ${v.all.expectedNonzero}, failures ${v.all.failuresPer100Calls ?? '—'} per 100)`).join('; ')}${te.maskedFailures ? `; masked failures ${te.maskedFailures.masked} of ${te.maskedFailures.cleanCheckResults} clean check results` : ''}.`);
  }
  if (s.failures) out.push(`- Tool errors by tool: ${Object.entries(s.failures.byTool).slice(0, 6).map(([k, n]) => `${k} ${n}`).join(', ') || '—'}.`);
  const oc = s.outcomes;
  if (oc) {
    const r = (v) => pcn(v.turns, v.n);
    const tf = oc.toolFailuresPerTurn;
    out.push(`- Turn outcomes (proxies, never tuned on) over ${oc.finished} finished turns: cancelled ${r(oc.cancelled)}, errored ${r(oc.errored)}, timed out ${r(oc.timedOut)}; stopped before first text ${r(oc.stoppedBeforeText)}${oc.stoppedBeforeText.unknownTiming ? ` (${oc.stoppedBeforeText.unknownTiming} without timings)` : ''}; re-asked within 120 s ${r(oc.reaskWithin120s)}; tool failures per turn ${tf.all.perTurn ?? '—'}${Object.keys(tf.byTool).length ? ` (${Object.entries(tf.byTool).slice(0, 5).map(([k, v]) => `${k} ${v.perTurn}`).join(', ')})` : ''}.`);
  }
  const ma = s.mainAgent;
  if (ma) {
    // A sum with turns of unknown cost is a floor (≥), none known is unknown, never $0.
    const sumText = (c) => (c.sumUsd == null ? 'unknown' : `${c.unknownTurns ? '≥ ' : ''}$${c.sumUsd.toFixed(2)}`);
    const kind = ([k, v]) => `${k} ${v.turns} turns, cost p50 ${v.cost.p50Usd == null ? '—' : `$${v.cost.p50Usd.toFixed(2)}`} / sum ${sumText(v.cost)}${v.cost.unknownTurns ? ` (${v.cost.unknownTurns} without a cost)` : ''}, tools/turn ${v.toolsPerTurn ?? '—'}, read_session/turn ${v.readSessionPerTurn ?? '—'}, refusals ${v.refusals}, PULSE_OK ${pcn(v.pulseOk.ok, v.pulseOk.n)}`;
    const sg = ma.suggestions;
    const sugg = sg ? `; suggestions over ${sg.days} days: done ${sg.done}, not now ${sg.not_now}, never ${sg.never} (taken ${pcn(sg.done, sg.answered)})` : '';
    out.push(`- Main agent (hollow: turns): ${ma.status === 'untagged' ? 'untagged (no turn carries a hollow: tag)' : Object.entries(ma.byKind).map(kind).join('; ')}${sugg}.`);
  }
  const mem = s.memory;
  if (mem) out.push(`- Native memory writes ${mem.nativeWrites ?? '—'}${mem.nativeWriteRuns ? ` in ${runsWord(mem.nativeWriteRuns)}` : ''} (receipts ${mem.receiptMemoryWrites ?? '—'} over ${mem.receiptsWithMemoryWrites} that carry it) against learnings accepted ${mem.learningsAccepted ?? '—'}.`);
  // S27: no receipt in the window carries a claim: "not installed", never 0.
  if (s.verification) {
    const vc = s.verification.claims;
    const pv = (v, k) => `${v[k]} (${v.per100Runs[k] ?? '—'} per 100)`;
    const byGroup = Object.entries(s.groups).filter(([, g]) => g.verification?.claims);
    const groupsText = byGroup.length ? ` (${byGroup.map(([k, g]) => `${k} ${g.verification.claims.claims} of ${g.verification.claims.n}, without a check ${g.verification.claims.claimWithoutCheck}, after a failed check ${g.verification.claims.claimAfterFail}`).join('; ')})` : '';
    out.push(`- Pass claims in final replies: ${vc ? `${pv(vc, 'claims')} over ${runsWord(vc.n)}; without a check after the last write ${pv(vc, 'claimWithoutCheck')}; after a failed check ${pv(vc, 'claimAfterFail')}${groupsText}` : 'not installed (no receipt carries verification.claim)'}.`);
  }
  for (const n of s.notes) out.push(`- Note: ${n}`);
  return out.join('\n');
}

export function renderComparison(c) {
  const out = [`# Scorecard comparison`, '', `A ${c.a.since.slice(0, 16)} → ${c.a.until.slice(0, 16)}; B ${c.b.since.slice(0, 16)} → ${c.b.until.slice(0, 16)}.`, ''];
  out.push(`Under ${LOW_N} finished runs on either side is marked low n: read its changes as noise.`, '');
  out.push('| Surface/agent | Runs A→B (n finished) | Success diff (95% CI) | First text p95 | Turn p95 | Cost p50 | Cache read A→B |', '|---|---|---|---|---|---|---|');
  for (const [k, g] of Object.entries(c.groups)) {
    const ch = (x) => (x == null ? '—' : `${x > 0 ? '+' : ''}${(x * 100).toFixed(0)}%`);
    const n = g.n ?? g.runs;
    out.push(`| ${k}${g.lowN ? ' (low n)' : ''} | ${g.runs.a}→${g.runs.b} (n ${n.a}→${n.b}) | ${g.success ? `${(g.success.diff * 100).toFixed(1)} (${(g.success.lo * 100).toFixed(1)}…${(g.success.hi * 100).toFixed(1)})` : '—'} | ${ch(g.latency.firstTextMs.p95Change)} | ${ch(g.latency.turnMs.p95Change)} | ${ch(g.cost.p50Change)} | ${pc(g.cacheReadShare.a)}→${pc(g.cacheReadShare.b)} |`);
  }
  const r = c.continuity;
  out.push('', `Resets per 100 turns ${r.resetsPer100Turns.a ?? '—'} → ${r.resetsPer100Turns.b ?? '—'}; config changed ${r.configChangedPer100Turns.a ?? '—'} → ${r.configChangedPer100Turns.b ?? '—'}; mode-flip resets ${r.flipResetsPer100Turns.a ?? '—'} → ${r.flipResetsPer100Turns.b ?? '—'}; Main agent config changed ${r.mainAgentConfigChangedPer100Turns.a ?? '—'} → ${r.mainAgentConfigChangedPer100Turns.b ?? '—'}.`);
  const partsLine = (p) => (p ? Object.entries(p).map(([k, n]) => `${k} ${n}`).join(', ') || 'none' : '—');
  if (r.resetsByPart?.a || r.resetsByPart?.b) out.push(`Config-changed resets by what changed: ${partsLine(r.resetsByPart.a)} → ${partsLine(r.resetsByPart.b)}.`);
  if (r.turns) out.push(`Turns ${r.turns.a ?? '—'} → ${r.turns.b ?? '—'}${r.lowN ? ' (low n)' : ''}.`);
  out.push(`Injected per run p50 ${kb(c.injected.totalP50.a)} → ${kb(c.injected.totalP50.b)}; p95 ${kb(c.injected.totalP95.a)} → ${kb(c.injected.totalP95.b)}.`);
  if (c.queue) out.push(`Queue waits ${c.queue.waits.a ?? '—'} → ${c.queue.waits.b ?? '—'}; max waits per second ${c.queue.maxWaitsPerSecond.a ?? '—'} → ${c.queue.maxWaitsPerSecond.b ?? '—'}. Long runs ${c.longRuns?.a ?? '—'} → ${c.longRuns?.b ?? '—'}.`);
  const byAgent = Object.entries(c.tools?.errorsPer100CallsByAgent ?? {});
  if (byAgent.length) out.push(`Tool errors per 100 calls ${byAgent.map(([k, v]) => `${k} ${v.a ?? '—'} → ${v.b ?? '—'}`).join('; ')}; masked failures ${c.tools.maskedFailures.a ?? '—'} → ${c.tools.maskedFailures.b ?? '—'}.`);
  // Tool pass lines when either side has the block (an older file: —).
  const tp = c.tools;
  if (tp && (tp.turns.a != null || tp.turns.b != null)) {
    out.push(`ToolSearch per 100 turns ${tp.toolSearchPer100Turns.a ?? '—'} → ${tp.toolSearchPer100Turns.b ?? '—'} (turns ${tp.turns.a ?? '—'} → ${tp.turns.b ?? '—'}).`);
    out.push(`First-call cold share ${Object.entries(tp.firstCallColdShare).map(([k, v]) => `${k} ${pc(v.a)} (n ${v.n.a ?? '—'}) → ${pc(v.b)} (n ${v.n.b ?? '—'})`).join('; ')}.`);
  }
  // SC4's lines when either side has the block (an older file: —).
  const withN = (v) => `${pc(v.a)} (n ${v.n.a ?? '—'}) → ${pc(v.b)} (n ${v.n.b ?? '—'})`;
  const oc = c.outcomes;
  const untimed = oc?.stoppedBeforeTextShare.unknownTiming;
  if (oc && (oc.reaskShare.n.a != null || oc.reaskShare.n.b != null)) out.push(`Re-asked within 120 s ${withN(oc.reaskShare)}; stopped before first text ${withN(oc.stoppedBeforeTextShare)}, without timings ${untimed?.a ?? '—'} → ${untimed?.b ?? '—'} (proxies).`);
  const ma = c.mainAgent;
  if (ma && (ma.status.a != null || ma.status.b != null)) out.push(`Main agent wakes' read_session per turn ${ma.wakeReadSessionPerTurn.a ?? '—'} (n ${ma.wakeReadSessionPerTurn.n.a ?? '—'}) → ${ma.wakeReadSessionPerTurn.b ?? '—'} (n ${ma.wakeReadSessionPerTurn.n.b ?? '—'})${ma.status.a === 'untagged' || ma.status.b === 'untagged' ? ` (tags ${ma.status.a ?? '—'} → ${ma.status.b ?? '—'})` : ''}.`);
  const mem = c.memory;
  if (mem && (mem.nativeWrites.a != null || mem.nativeWrites.b != null || mem.learningsAccepted.a != null || mem.learningsAccepted.b != null)) out.push(`Native memory writes ${mem.nativeWrites.a ?? '—'} → ${mem.nativeWrites.b ?? '—'}; learnings accepted ${mem.learningsAccepted.a ?? '—'} → ${mem.learningsAccepted.b ?? '—'}.`);
  // OF28's lines when either side carries the key (a side without it: —).
  const lc = c.ledger;
  const pdc = (v, f) => (v ? `${f(v.p50)} / ${f(v.p95)} (n ${v.n})` : '—');
  const money = (x) => (x == null ? '—' : `$${x.toFixed(2)}`);
  const plain = (x) => x ?? '—';
  if (lc && [lc.peakContext.a, lc.peakContext.b, lc.apiCalls.a, lc.apiCalls.b, lc.partialFloorUsd.a, lc.partialFloorUsd.b].some((x) => x != null)) {
    out.push(`Peak context p50/p95 ${pdc(lc.peakContext.a, ktok)} → ${pdc(lc.peakContext.b, ktok)}; API calls per run ${pdc(lc.apiCalls.a, plain)} → ${pdc(lc.apiCalls.b, plain)}; partial-run cost floor ${money(lc.partialFloorUsd.a)} → ${money(lc.partialFloorUsd.b)}.`);
  }
  const fm = Object.entries(c.groups).filter(([, g]) => g.latency.firstModelMs?.a?.n || g.latency.firstModelMs?.b?.n);
  // S27's line when either side carries pass claims (a side without them: —).
  const vf = c.verification;
  if (vf && (vf.n.a != null || vf.n.b != null)) {
    const p = (v) => `${v.a ?? '—'} → ${v.b ?? '—'}`;
    out.push(`Pass claims per 100 runs ${p(vf.claimsPer100Runs)} (n ${p(vf.n)}); without a check after the last write ${p(vf.claimWithoutCheckPer100Runs)}; after a failed check ${p(vf.claimAfterFailPer100Runs)}.`);
  }
  if (fm.length) out.push(`First model event p50 ${fm.map(([k, g]) => `${k} ${s1(g.latency.firstModelMs.a.p50)} (n ${g.latency.firstModelMs.a.n ?? '—'}) → ${s1(g.latency.firstModelMs.b.p50)} (n ${g.latency.firstModelMs.b.n ?? '—'})`).join('; ')}.`);
  out.push('', c.flags.length ? '## Flags' : 'No flags.');
  for (const f of c.flags) out.push(`- ${f.kind}${f.group ? ` ${f.group}` : ''}${f.metric ? ` ${f.metric}` : ''}: ${f.change ?? f.diff}${f.lowN ? ' (low n)' : ''}`);
  if (c.unpaired.length) out.push('', `In one scorecard only: ${c.unpaired.join(', ')}.`);
  return out.join('\n');
}

// ── CLI ──────────────────────────────────────────────────────────────────

const USAGE = `usage:
  scorecard.mjs summary --vault <dir> [--since <iso>] [--until <iso>] [--overlap] [--db <file>]
                        [--transcripts <dir>]... [--codex-home <dir>] [--no-transcripts] [--json <file>]
  scorecard.mjs compare <A.json> <B.json> [--json <file>]
  scorecard.mjs trace-assert <stream.jsonl | folder>... [--json <file>] [--skill <name>]... [--exempt <id>]...
--since defaults to 7 days before --until (default now). Runs count by start;
--overlap counts every run overlapping the window. Transcripts default to
~/.claude/projects, Codex rollouts to $CODEX_HOME (else ~/.codex)/sessions;
--no-transcripts reads neither.`;

export function parseArgs(argv) {
  const [cmd, ...rest] = argv;
  const o = { cmd, files: [], transcripts: [], noTranscripts: false, overlap: false };
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    const val = () => {
      const v = rest[++i];
      if (v == null || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--vault') o.vault = val();
    else if (a === '--since') o.since = val();
    else if (a === '--until') o.until = val();
    else if (a === '--db') o.db = val();
    else if (a === '--json') o.json = val();
    else if (a === '--transcripts') o.transcripts.push(val());
    else if (a === '--codex-home') o.codexHome = val();
    else if (a === '--no-transcripts') o.noTranscripts = true;
    else if (a === '--overlap') o.overlap = true;
    else if (a.startsWith('--')) throw new Error(`unknown option ${a}`);
    else o.files.push(a);
  }
  return o;
}

export async function main(argv) {
  // NV5: the flags over a recorded stream, with its own options. Loaded here
  // because trace-assert.mjs imports this file.
  if (argv[0] === 'trace-assert') return (await import('./trace-assert.mjs')).main(argv.slice(1));
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  if (o.cmd === 'summary') {
    if (!o.vault || !existsSync(o.vault)) {
      console.error(`--vault must name an existing folder\n${USAGE}`);
      return 2;
    }
    const until = o.until ? at(o.until) : Date.now();
    const since = o.since ? at(o.since) : until - 7 * 86_400_000;
    if (until == null || since == null || since >= until) {
      console.error(`--since and --until must be ISO times, since before until\n${USAGE}`);
      return 2;
    }
    const roots = o.noTranscripts ? [] : o.transcripts.length ? o.transcripts.map((d) => resolve(d)) : [join(homedir(), '.claude', 'projects')];
    const codexHome = o.noTranscripts ? null : resolve(o.codexHome ?? (process.env.CODEX_HOME || join(homedir(), '.codex')));
    const s = await summaryFromVault({ vault: resolve(o.vault), dbFile: o.db && resolve(o.db), transcriptRoots: roots, codexHome, since, until, overlap: o.overlap });
    if (o.json) writeFileSync(o.json, `${JSON.stringify(s, null, 2)}\n`);
    console.log(renderSummary(s));
    return 0;
  }
  if (o.cmd === 'compare') {
    if (o.files.length !== 2) {
      console.error(USAGE);
      return 2;
    }
    let c;
    try {
      const [a, b] = o.files.map((f) => JSON.parse(readFileSync(f, 'utf8')));
      c = compareScorecards(a, b);
    } catch (e) {
      console.error(e.message);
      return 2;
    }
    if (o.json) writeFileSync(o.json, `${JSON.stringify(c, null, 2)}\n`);
    console.log(renderComparison(c));
    return 0;
  }
  console.error(USAGE);
  return 2;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
