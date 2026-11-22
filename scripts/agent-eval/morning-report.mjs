// The morning report (overnight 2026-10-17, OF21): one number sheet for the
// morning. Tonight's lanes against the evening lanes, $ per merged PR, the
// later items' gate-health and offline-delta JSONs, and a product scorecard
// compare. Plain node, $0, read-only:
//
//   node scripts/agent-eval/morning-report.mjs --since <launch> --hw <HW> [--vault <v> | --db <file>]
//        [--repo <checkout>] [--until <iso>] [--baseline-since <iso> --baseline-until <iso>]
//        [--tonight <dir>] [--baseline <dir>] [--lane <name>=<session key>]...
//        [--ref <git ref>] [--git-log <file>] [--transcripts <dir>]...
//        [--product-baseline <file>] [--no-product] [--installed-at <iso>] [--json <file>] [--out <md>]
//
// A lane is a chain of chats. Its seeds are the session keys its
// `lane-*.md` names (and `--lane`; the default evening window adds the
// evening lanes' own keys, EVENING_LANES, unless `--lane` names them); the
// chain adds handover successors and predecessors from the `delegations`
// records (`tool` handover, `childSession`). A lane's runs are its chats'
// turns that overlap the window, whole: a lane's first turn starts before
// `--since` (OF12's straddling), and its cost is the run's. A lane with no
// chat, a seed missing from the database, or a successor with no handover
// record to its predecessor has an unknown cost (never $0), and so do the
// totals; a chat in two lanes counts once in the totals. Per lane:
// - transcript usage (scorecard.mjs readers): calls, main-thread context per
//   call p50/p95, the share of weighted tokens on calls at 400K or more, and
//   the subagent share of weighted tokens;
// - handovers, with the main-thread context of the last call at or before
//   each, and calls at or over the 350K handover line (board rule 5);
// - Python edit scripts per 100 Bash calls, Edit/MultiEdit and Write calls
//   (main thread and subagents apart), and poll-only calls, classified in
//   memory from the tool name and a Bash call's command (OF19's regexes,
//   imported from scorecard.mjs);
// - chats, stints, cost (known, unknown, partial) from the usage ledger, the
//   PRs the lane report claims (`#N → sha`) that `git log --first-parent
//   --merges` shows merged in the window, and known $ per merged PR.
// The evening baseline is the same block over the evening lanes' window.
//
// Read-only (the database through scorecard.mjs's openDb), and aggregates
// only: no message text, prompt, tool result or tool input other than a Bash
// command is read, and no command text reaches the report. Unknown cost stays
// unknown: a $ per PR over runs without a cost is a floor (≥).
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { PR, parseReport } from '../overnight-summary.mjs';
import { HANDOVER_AT } from './my-context.mjs';
import { compareScorecards, context, costBlock, defaultDb, indexTranscripts, isPoll, isPollOnly, isPythonEdit, openDb, readCalls, renderComparison, summaryFromVault, transcriptBlock, weight } from './scorecard.mjs';

export const SCHEMA = 1;
export const KIND = 'hollow-morning-report';
/** The evening lanes' window (HB and HF, 2026-10-16), the default baseline. */
export const EVENING = { since: '2026-10-16T21:00:00Z', until: '2026-10-16T23:10:00Z' };
/** The evening lanes' chats, so the baseline doesn't hang on lane-hf.md
 *  (which names none): used over the default window when no `--lane` names
 *  that lane. */
export const EVENING_LANES = { hb: 's-20261016-0e4cbde5', hf: 's-20261016-39fdcf85' };

const round2 = (n) => (n == null || !Number.isFinite(n) ? null : Math.round(n * 100) / 100);
const round4 = (n) => (n == null || !Number.isFinite(n) ? null : Math.round(n * 1e4) / 1e4);
const at = (s) => {
  const t = Date.parse(s ?? '');
  return Number.isFinite(t) ? t : null;
};
const sum = (xs) => xs.reduce((a, b) => a + (typeof b === 'number' && Number.isFinite(b) ? b : 0), 0);
const iso = (t) => new Date(t).toISOString();

// ── Classifying a tool use (counts only) ─────────────────────────────────

// The Python-edit and poll classifiers are OF19's, in scorecard.mjs (its
// `transcripts.pythonEditScripts` and `pollOnlyCalls` count with the same
// regexes); re-exported here for this report's callers.
export { isPoll, isPollOnly, isPythonEdit };
/** What readCalls keeps per tool use: the name and two booleans. */
const classify = (name, command) => ({ name, pythonEdit: command != null && isPythonEdit(command), poll: isPoll(name, command) });

// ── Lane reports and git ─────────────────────────────────────────────────

const SESSION_KEY = /\bs-\d{8}-[0-9a-f]{8}\b/g;
/** A claim: `#709 → \`sha\``, `→ #708 → merge \`sha\``, or the link form
 *  overnight-summary.mjs reads (`[#709](…/pull/709) → \`sha\``). A PR the
 *  prose mentions in passing (another lane's) has no arrow and sha. */
const CLAIM = new RegExp(String.raw`(?:${PR.source}|#(\d+))\s*→\s*(?:merge\s+)?` + '`[0-9a-f]{7,40}`', 'g');
const byNumber = (a, b) => Number(a) - Number(b);
/** A key the report calls a successor: `successor 1 (\`s-…\`)`. Only the key
 *  right after the word, so "from `A` to successor `B`" names B alone. */
const SUCCESSOR = /\bsuccessor\b[^`\n]{0,24}`(s-\d{8}-[0-9a-f]{8})`/gi;

/** One lane report: its lane (from `lane-<x>.md`), the session keys it
 *  names (and which it calls a successor), and the PRs it claims (arrow
 *  claims, plus overnight-summary's merged heading items with their PR
 *  links). */
export function parseLaneReport(file, text) {
  const lane = /^lane-(\w+)\.md$/.exec(file)?.[1]?.toUpperCase() ?? file;
  const keys = [...new Set(text.match(SESSION_KEY) ?? [])];
  const successors = [...new Set([...text.matchAll(SUCCESSOR)].map((m) => m[1]))];
  const prs = new Set();
  for (const m of text.matchAll(CLAIM)) prs.add(m[1] ?? m[2]);
  for (const it of parseReport(file, text)) if (/\bmerged\b/i.test(it.status)) for (const n of it.prs) prs.add(n);
  return { lane, file, keys, successors, prs: [...prs].sort(byNumber) };
}

export function readLaneReports(dir) {
  if (!dir || !existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => /^lane-\w+\.md$/.test(f))
    .sort()
    .map((f) => parseLaneReport(f, readFileSync(join(dir, f), 'utf8')));
}

/** `git log --format=%H%x09%cI%x09%s` lines → the merges of a PR. */
export function parseGitLog(text) {
  const out = [];
  for (const line of text.split('\n')) {
    const [sha, when, subject] = line.split('\t');
    const pr = /^Merge pull request #(\d+)\b/.exec(subject ?? '')?.[1];
    const t = at(when);
    if (sha && pr && t != null) out.push({ sha, at: t, pr });
  }
  return out;
}

function gitMerges(repo, ref, since) {
  // A day of margin: --since is the committer date; the window is applied here.
  const text = execFileSync('git', ['-C', repo, 'log', '--first-parent', '--merges', `--since=${iso(since - 86_400_000)}`, '--format=%H%x09%cI%x09%s', ref], { encoding: 'utf8', maxBuffer: 16 << 20 });
  return parseGitLog(text);
}

// ── The sessions database ────────────────────────────────────────────────

/** Ids, times, outcomes and costs only; never turns.asked, a title, a
 *  usage_json or a delegation packet. An older database without `stints`
 *  or `delegations` gets a note and empty rows. */
export function readLaneDb(db, notes = []) {
  const tables = new Set(db.prepare("select name from sqlite_master where type = 'table'").all().map((t) => t.name));
  const optional = (table, sql, without) => {
    if (tables.has(table)) return db.prepare(sql).all();
    notes.push(`no ${table} table (an older database): ${without}`);
    return [];
  };
  return {
    sessions: db.prepare('select key, created_at from sessions').all(),
    turns: db.prepare('select session_key, ord, run_id, started_at, ended_at, status from turns').all(),
    stints: optional('stints', 'select session_key, started_at, ended_at from stints', 'stints are empty'),
    handovers: optional(
      'delegations',
      "select run_id, at, json_extract(record_json, '$.childSession') child from delegations where json_valid(record_json) and json_extract(record_json, '$.tool') = 'handover'",
      'no handovers, chains are the named chats',
    ),
    ledger: db
      .prepare('select run_key, at, started_at, surface, scope, parent_surface, run_id, agent, model, native_session, outcome, input_tokens, cached_input_tokens, output_tokens, cost_usd, cost_status from usage_ledger')
      .all(),
  };
}

/** The seeds plus every chat a handover links to them, either way. */
export function laneChain(seeds, { turns, handovers }) {
  const sessionOfRun = new Map(turns.filter((t) => t.run_id).map((t) => [t.run_id, t.session_key]));
  const keys = new Set(seeds);
  for (let grew = true; grew; ) {
    grew = false;
    for (const h of handovers) {
      const from = sessionOfRun.get(h.run_id);
      if (!from || !h.child) continue;
      if (keys.has(from) && !keys.has(h.child)) grew = !!keys.add(h.child);
      if (keys.has(h.child) && !keys.has(from)) grew = !!keys.add(from);
    }
  }
  return keys;
}

// ── One lane ─────────────────────────────────────────────────────────────

/** The lane block's transcript part over `runs` ({ row, calls }[]). */
function laneTranscript(runs) {
  const base = transcriptBlock(runs);
  if (!base) return null;
  const calls = runs.filter((r) => r.calls).flatMap((r) => r.calls);
  const main = calls.filter((c) => !c.sidechain);
  const w = sum(calls.map(weight));
  const tools = main.flatMap((c) => c.tools ?? []);
  const side = calls.filter((c) => c.sidechain).flatMap((c) => c.tools ?? []);
  const isBash = (t) => t.name === 'Bash';
  const isEdit = (t) => t.name === 'Edit' || t.name === 'MultiEdit';
  const isWrite = (t) => t.name === 'Write';
  const bash = tools.filter(isBash).length;
  const py = tools.filter((t) => t.pythonEdit).length;
  const polls = main.filter((c) => isPollOnly(c.tools ?? []));
  return {
    calls: base.calls,
    mainCalls: main.length,
    subagentCalls: base.subagentCalls,
    contextPerCall: base.contextPerCall,
    largeContextCalls: base.largeContextCalls,
    costShareAtLarge: base.costShareAtLarge,
    callsOverHandoverLine: main.filter((c) => context(c) >= HANDOVER_AT).length,
    subagentShare: w ? round4(sum(calls.filter((c) => c.sidechain).map(weight)) / w) : null,
    bashCalls: bash,
    pythonEditScripts: py,
    pythonEditsPer100Bash: bash ? round2((py * 100) / bash) : null,
    editCalls: tools.filter(isEdit).length,
    writeCalls: tools.filter(isWrite).length,
    subagentBashCalls: side.filter(isBash).length,
    subagentPythonEditScripts: side.filter((t) => t.pythonEdit).length,
    subagentEditCalls: side.filter(isEdit).length,
    subagentWriteCalls: side.filter(isWrite).length,
    pollOnlyCalls: polls.length,
    pollOnlyContextTokens: sum(polls.map(context)),
    reconciliation: base.reconciliation,
  };
}

/** A chat's own ledger row, not a consultation or helper sharing its run id. */
const isChatRow = (r) => !r.parent_surface && r.surface !== 'oneshot' && r.surface !== 'delegate';
const perPr = (cost, merged) => (merged ? { usd: round2(cost.floorUsd / merged), complete: cost.complete } : null);
/** A cost block with `complete`: a total only when every run has a cost and
 *  nothing the lane should have counted is missing (`missing`, short reasons). */
const withMissing = (cost, missing) => (missing.length ? { ...cost, totalUsd: null, complete: false, missing } : { ...cost, complete: cost.totalUsd != null });

/** The chats of a lane chain with a turn overlapping [since, until). */
function chatsInWindow(seeds, data, { since, until }) {
  const chain = laneChain(seeds, data);
  const keys = new Set();
  for (const t of data.turns) {
    const s = at(t.started_at);
    if (chain.has(t.session_key) && s != null && s < until && (at(t.ended_at) ?? until) >= since) keys.add(t.session_key);
  }
  return keys.size;
}

/** Measure one lane over [since, until). `label` names it in notes. */
async function measureLane(report, seeds, { data, window, merges, readFile, idx, notes, label }) {
  const { since, until } = window;
  const known = new Set(data.sessions.map((s) => s.key));
  const created = new Map(data.sessions.map((s) => [s.key, s.created_at ?? '']));
  const missing = [];
  if (!seeds.length) missing.push('no chat named');
  for (const k of seeds.filter((k) => !known.has(k))) {
    notes.push(`${label}: ${k} is not in the sessions database: its cost is unknown`);
    missing.push('a named chat not in the database');
  }
  const chain = laneChain(seeds.filter((k) => known.has(k)), data);
  // A successor with no handover record into it, and no earlier chat in the
  // chain: its predecessor's turns (and cost) are not here.
  const handedTo = new Set(data.handovers.map((h) => h.child));
  for (const k of report.successors ?? []) {
    if (!chain.has(k) || handedTo.has(k) || [...chain].some((c) => (created.get(c) ?? '') < (created.get(k) ?? ''))) continue;
    notes.push(`${label}: ${report.file} calls ${k} a successor, but no handover record links its predecessor: its cost is unknown; pass --lane ${report.lane.toLowerCase()}=<predecessor key>`);
    missing.push('a predecessor not linked');
  }
  const keys = [...chain].sort((a, b) => (created.get(a) ?? '').localeCompare(created.get(b) ?? '') || a.localeCompare(b));
  const rowsByRun = new Map();
  for (const r of data.ledger) if (r.run_id) (rowsByRun.get(r.run_id) ?? rowsByRun.set(r.run_id, []).get(r.run_id)).push(r);
  // The lane's turns overlapping the window; a running turn has no end yet.
  const turns = data.turns.filter((t) => {
    if (!chain.has(t.session_key) || !t.run_id) return false;
    const s = at(t.started_at);
    const chat = (rowsByRun.get(t.run_id) ?? []).find(isChatRow);
    const e = at(t.ended_at) ?? (chat && chat.outcome !== 'running' ? at(chat.at) : null) ?? until;
    return s != null && s < until && e >= since;
  });
  const runIds = new Set(turns.map((t) => t.run_id));
  const rows = data.ledger.filter((r) => runIds.has(r.run_id));
  const turnOf = new Map(turns.map((t) => [t.run_id, t]));
  // The chat's own rows carry its transcript: not an ask_agent consultation
  // (`delegate`) or a helper call (`oneshot`) sharing the run id.
  const runs = [];
  let untranscribed = 0;
  for (const row of rows) {
    if (!isChatRow(row) || row.agent !== 'claude' || !row.native_session) continue;
    const files = idx.get(row.native_session);
    if (!files?.main) {
      untranscribed++;
      continue;
    }
    const from = at(row.started_at) - 1000;
    const to = row.outcome === 'running' ? until : row.outcome === 'interrupted' ? (at(turnOf.get(row.run_id)?.ended_at) ?? until) : at(row.at) + 1000;
    const calls = [];
    for (const c of await readFile(files.main, false)) if (c.t >= from && c.t <= to) calls.push(c);
    for (const f of files.subagents) for (const c of await readFile(f, true)) if (c.t >= from && c.t <= to) calls.push(c);
    if (calls.some((c) => !c.sidechain)) runs.push({ row, calls });
  }
  if (untranscribed) notes.push(`${label}: no transcript for ${untranscribed === 1 ? "1 run's native session" : `${untranscribed} runs' native sessions`}`);
  const sessionOfRun = new Map(data.turns.filter((t) => t.run_id).map((t) => [t.run_id, t.session_key]));
  const handovers = data.handovers
    .filter((h) => runIds.has(h.run_id) && at(h.at) != null)
    .sort((a, b) => at(a.at) - at(b.at))
    .map((h) => {
      const main = runs.filter((r) => r.row.run_id === h.run_id).flatMap((r) => r.calls.filter((c) => !c.sidechain && c.t <= at(h.at)));
      const last = main.sort((a, b) => a.t - b.t).at(-1);
      return { at: iso(at(h.at)), from: sessionOfRun.get(h.run_id), to: h.child, contextTokens: last ? context(last) : null };
    });
  const stints = data.stints.filter((s) => chain.has(s.session_key) && at(s.started_at) < until && (s.ended_at == null || at(s.ended_at) >= since));
  const mergedInWindow = new Set(merges.filter((m) => m.at >= since && m.at < until).map((m) => m.pr));
  const mergedPrs = report.prs.filter((n) => mergedInWindow.has(n));
  const cost = withMissing(costBlock(rows), missing);
  const transcript = laneTranscript(runs);
  const ratio = transcript?.reconciliation?.ratio;
  if (ratio != null && ratio < 0.98) notes.push(`${label}: transcripts carry ${(ratio * 100).toFixed(1)}% of the ledger's tokens (below 98%): some calls are missing`);
  const chats = [...new Set(turns.map((t) => t.session_key))];
  return {
    lane: report.lane,
    file: report.file,
    keys,
    chats: chats.length,
    stints: stints.length,
    runs: rows.length,
    transcript,
    handovers,
    cost,
    mergedPrs,
    claimedNotMerged: report.prs.filter((n) => !mergedInWindow.has(n)),
    usdPerMergedPr: perPr(cost, mergedPrs.length),
    _chats: chats,
    _runs: runs,
    _rows: rows,
  };
}

/** Last one wins per key. */
const uniqueBy = (xs, key) => [...new Map(xs.map((x) => [key(x), x])).values()];

/** The lanes together. A chat two lanes' chains reach counts once: ledger
 *  rows by run_key, transcript runs by their row's run_key (a run id can
 *  carry two chat rows after an account failover), chats by key, handovers
 *  by run and child. Any incomplete lane leaves the total unknown. */
function totals(lanes) {
  const rows = uniqueBy(lanes.flatMap((l) => l._rows), (r) => r.run_key);
  const missing = lanes.flatMap((l) => (l.cost.missing ?? []).map((m) => `${l.lane}: ${m}`));
  const cost = withMissing(costBlock(rows), missing);
  const merged = new Set(lanes.flatMap((l) => l.mergedPrs)).size;
  return {
    lanes: lanes.length,
    chats: new Set(lanes.flatMap((l) => l._chats)).size,
    handovers: uniqueBy(lanes.flatMap((l) => l.handovers), (h) => `${h.at}/${h.to}`).length,
    transcript: laneTranscript(uniqueBy(lanes.flatMap((l) => l._runs), (r) => r.row.run_key)),
    cost,
    merged,
    usdPerMergedPr: perPr(cost, merged),
  };
}

// ── Placeholders: later items drop a JSON in the night folder ────────────

export const PLACEHOLDERS = [
  { id: 'OF11', label: 'gate health', match: /^gate-stats.*\.json$/, pattern: 'gate-stats*.json' },
  { id: 'OB15', label: 'envelope diff', match: /^envelope-diff.*\.json$/, pattern: 'envelope-diff*.json' },
  { id: 'OF17', label: 'real-vault replay probes', match: /^(?:of17|replay-probes).*\.json$/i, pattern: 'replay-probes*.json' },
  { id: 'S07', label: 'replay bench', match: /^(?:s07|replay-bench).*\.json$/i, pattern: 'replay-bench*.json' },
  { id: 'OF01a/b', label: 'retrieval eval', match: /^(?:of01|retrieval-eval).*\.json$/i, pattern: 'retrieval-eval*.json' },
  { id: 'OB18/OB25', label: 'host-turn and follow-up retrieval', match: /^(?:ob18|ob25|retrieval-skips).*\.json$/i, pattern: 'retrieval-skips*.json' },
];

/** Number and boolean leaves only, dotted, three levels deep at most. */
function numbers(v, prefix = '', depth = 0, out = []) {
  if (out.length >= 16 || v == null || typeof v !== 'object' || Array.isArray(v) || depth > 2) return out;
  for (const [k, x] of Object.entries(v)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if ((typeof x === 'number' && Number.isFinite(x)) || typeof x === 'boolean') out.push([key, x]);
    else numbers(x, key, depth + 1, out);
    if (out.length >= 16) break;
  }
  return out;
}

function placeholders(dir, notes) {
  const files = dir && existsSync(dir) ? readdirSync(dir).sort() : [];
  return PLACEHOLDERS.map(({ id, label, match, pattern }) => {
    const file = files.filter((f) => match.test(f)).at(-1) ?? null;
    let values = null;
    if (file) {
      try {
        values = Object.fromEntries(numbers(JSON.parse(readFileSync(join(dir, file), 'utf8'))));
      } catch {
        notes.push(`${file} could not be parsed`);
      }
    }
    return { id, label, pattern, file, values };
  });
}

// ── The report ───────────────────────────────────────────────────────────

const ET_DATE = new Intl.DateTimeFormat('en-CA', { timeZone: 'America/New_York', year: 'numeric', month: '2-digit', day: '2-digit' });

/** `<hw>/outputs/<kind>-<date>` for a window starting at `t`. The date can be
 *  the UTC date of `t`, the next one (an overnight folder is named for its
 *  morning), or the New York date (an evening folder): when more than one
 *  exists, the one whose lane reports name the most chats with a turn in the
 *  window (`score`), ties in that order; a note says so. */
function pickDir(hw, kind, t, score, notes) {
  const dates = [...new Set([iso(t).slice(0, 10), iso(t + 86_400_000).slice(0, 10), ET_DATE.format(new Date(t))])];
  const dirs = dates.map((d) => join(hw, 'outputs', `${kind}-${d}`)).filter((d) => existsSync(d));
  if (dirs.length < 2) return dirs[0] ?? join(hw, 'outputs', `${kind}-${dates[0]}`);
  const scored = dirs.map((d) => ({ d, n: score(d) }));
  const best = scored.reduce((a, b) => (b.n > a.n ? b : a));
  const names = dirs.map((d) => basename(d)).sort();
  notes.push(`${names.join(' and ')} ${names.length === 2 ? 'both ' : ''}exist: read ${basename(best.d)} (its lane reports name ${plural(best.n, 'chat')} with a turn in the window)`);
  return best.d;
}

export async function morningReport(o, { now = Date.now(), env = process.env } = {}) {
  const notes = [];
  const since = at(o.since);
  const until = o.until ? at(o.until) : now;
  const bSince = at(o.baselineSince ?? EVENING.since);
  const bUntil = at(o.baselineUntil ?? EVENING.until);

  const dbFile = o.db ? resolve(o.db) : o.vault ? defaultDb(resolve(o.vault)) : null;
  let data = { sessions: [], turns: [], stints: [], handovers: [], ledger: [] };
  if (dbFile && existsSync(dbFile)) {
    const { db, mode, notes: n } = await openDb(dbFile);
    notes.push(...n);
    if (mode === 'immutable') notes.push('database opened immutable (read-only open refused)');
    try {
      data = readLaneDb(db, notes);
    } finally {
      db.close();
    }
  } else notes.push('no sessions database: chats, cost and transcripts are empty');

  // A lane's seeds: its report's keys, then `--lane`, else (the default
  // evening window only) the evening lanes' built-in keys.
  const extra = new Map();
  for (const [name, key] of o.lanes) (extra.get(name) ?? extra.set(name, []).get(name)).push(key);
  const defaults = bSince === at(EVENING.since) && bUntil === at(EVENING.until) ? EVENING_LANES : {};
  const seedsOf = (report, builtIn) => {
    const name = report.lane.toLowerCase();
    return [...new Set([...report.keys, ...(extra.get(name) ?? (builtIn[name] ? [builtIn[name]] : []))])];
  };
  const score = (builtIn, window) => (dir) => sum(readLaneReports(dir).map((r) => chatsInWindow(seedsOf(r, builtIn), data, window)));
  const tonightDir = o.tonight ? resolve(o.tonight) : o.hw ? pickDir(resolve(o.hw), 'overnight', since, score({}, { since, until }), notes) : null;
  const baselineDir = o.baseline ? resolve(o.baseline) : o.hw ? pickDir(resolve(o.hw), 'evening', bSince, score(defaults, { since: bSince, until: bUntil }), notes) : null;
  for (const d of [tonightDir, baselineDir]) if (d && !existsSync(d)) notes.push(`no folder ${basename(d)}: its lanes are empty`);
  const kind = /^(\w+)-\d{4}/.exec(baselineDir ? basename(baselineDir) : '')?.[1] ?? 'baseline';

  const roots = o.transcripts.length ? o.transcripts.map((d) => resolve(d)) : [join(env.CLAUDE_CONFIG_DIR || join(homedir(), '.claude'), 'projects')];
  const idx = indexTranscripts(roots);
  const cache = new Map();
  const readFile = (f, side) => cache.get(f) ?? cache.set(f, readCalls(f, side, { toolUse: classify })).get(f);

  let merges = [];
  try {
    merges = o.gitLog ? parseGitLog(readFileSync(o.gitLog, 'utf8')) : gitMerges(resolve(o.repo ?? '.'), o.ref ?? 'origin/main', Math.min(since, bSince));
  } catch (e) {
    notes.push(`git log failed (${String(e.message).split('\n')[0]}): no merged PRs`);
  }

  const group = async (dir, window, builtIn, suffix) => {
    const out = [];
    for (const report of readLaneReports(dir)) {
      const name = report.lane.toLowerCase();
      const label = `lane ${report.lane}${suffix}`;
      const seeds = seedsOf(report, builtIn);
      if (!seeds.length) notes.push(`${label}: no chat named in ${report.file}; pass --lane ${name}=<key>`);
      else if (!report.keys.length && !extra.has(name)) notes.push(`${label}: no chat named in ${report.file}; used the built-in ${kind} key ${seeds.join(', ')}`);
      out.push(await measureLane(report, seeds, { data, window, merges, readFile, idx, notes, label }));
    }
    // A chat two lanes reach: each lane keeps it, the totals count it once.
    const lanesOf = new Map();
    for (const l of out) for (const k of l.keys) (lanesOf.get(k) ?? lanesOf.set(k, []).get(k)).push(l.lane);
    for (const [k, ls] of lanesOf) if (ls.length > 1) notes.push(`${k} is in lanes ${ls.join(' and ')}: counted once in the totals`);
    return out;
  };
  const tonight = await group(tonightDir, { since, until }, {}, '');
  const baseline = await group(baselineDir, { since: bSince, until: bUntil }, defaults, ` (${kind})`);
  const named = new Set([...tonight, ...baseline].map((l) => l.lane.toLowerCase()));
  for (const name of extra.keys()) if (!named.has(name)) notes.push(`--lane ${name}: no lane-${name}.md in either folder`);

  let product = null;
  if (!o.noProduct) {
    const candidates = o.productBaseline
      ? [resolve(o.productBaseline)]
      : [tonightDir && join(tonightDir, 'scorecard-baseline-v2.json'), tonightDir && join(tonightDir, 'scorecard-baseline.json'), baselineDir && join(baselineDir, 'scorecard-baseline.json')].filter(Boolean);
    const file = candidates.find((f) => existsSync(f));
    if (!file) notes.push('no scorecard-baseline.json: the product compare is skipped');
    else if (!o.vault) notes.push('no --vault: the product compare is skipped');
    else {
      try {
        const a = JSON.parse(readFileSync(file, 'utf8'));
        const b = await summaryFromVault({ vault: resolve(o.vault), dbFile: dbFile ?? undefined, transcriptRoots: roots, since, until });
        product = { baseline: basename(file), installedAt: o.installedAt ?? null, comparison: compareScorecards(a, b) };
      } catch (e) {
        notes.push(`product compare failed: ${String(e.message).split('\n')[0]}`);
      }
    }
  }

  const strip = (l) => Object.fromEntries(Object.entries(l).filter(([k]) => !k.startsWith('_')));
  const filled = placeholders(tonightDir, notes);
  return {
    schema: SCHEMA,
    kind: KIND,
    window: { since: iso(since), until: iso(until) },
    baselineWindow: { since: iso(bSince), until: iso(bUntil) },
    folders: { tonight: tonightDir && basename(tonightDir), baseline: baselineDir && basename(baselineDir) },
    lanes: { tonight: tonight.map(strip), baseline: baseline.map(strip) },
    totals: { tonight: totals(tonight), baseline: totals(baseline) },
    placeholders: filled,
    product,
    notes: [...new Set(notes)],
  };
}

// ── Markdown ─────────────────────────────────────────────────────────────

const ktok = (t) => (t == null ? '—' : `${Math.round(t / 1000)}K`);
const pc = (x) => (x == null ? '—' : `${(x * 100).toFixed(1)}%`);
const money = (c) => {
  if (c.totalUsd != null) return `$${c.totalUsd.toFixed(2)}`;
  const known = `$${c.floorUsd.toFixed(2)} known`;
  if (!c.unknownRuns && !c.missing?.length && c.unpricedByDesign) return `unpriced by design (${c.unpricedByDesign}); ${known}`;
  const why = [...(c.missing ?? []), ...(c.unknownRuns ? [`${c.unknownRuns} without a cost`] : [])];
  return `unknown (${why.join(', ')}); ${known}`;
};
/** "1 known, 1 unknown (partial)", "3 unknown (2 partial, 1 none)", and a
 *  partial row's carried cost. */
const rowsText = (c) => {
  const statuses = Object.entries(c.unknownByStatus ?? {});
  const by = statuses.length === 1 ? ` (${statuses[0][0]})` : statuses.length ? ` (${statuses.map(([s, n]) => `${n} ${s}`).join(', ')})` : '';
  const carried = c.partialCostUsd ? `; $${c.partialCostUsd.toFixed(2)} on partial rows` : '';
  return `${c.knownRuns} known, ${c.unknownRuns} unknown${by}${carried}`;
};
const perPrText = (p) => (p == null ? '—' : `${p.complete ? '' : '≥ '}$${p.usd.toFixed(2)}`);
const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

function row(label, chats, handovers, t, cost, merged, usdPerPr) {
  const h = handovers == null ? '—' : handovers.length ? `${handovers.length} (${handovers.map((x) => ktok(x.contextTokens)).join(', ')})` : '0';
  if (!t) return `| ${label} | ${chats} | — | — | — | ${h} | — | — | — | — | ${money(cost)} | ${merged} | ${perPrText(usdPerPr)} |`;
  const py = `${t.pythonEditsPer100Bash == null ? '—' : `${t.pythonEditsPer100Bash.toFixed(1)} (${t.pythonEditScripts}/${t.bashCalls})`} · sub ${t.subagentPythonEditScripts}/${t.subagentBashCalls}`;
  return `| ${label} | ${chats} | ${t.mainCalls} + ${t.subagentCalls} | ${ktok(t.contextPerCall.p50)} / ${ktok(t.contextPerCall.p95)} | ${pc(t.costShareAtLarge)} | ${h} | ${py} | ${t.editCalls} + ${t.subagentEditCalls} | ${t.pollOnlyCalls} | ${pc(t.subagentShare)} | ${money(cost)} | ${merged} | ${perPrText(usdPerPr)} |`;
}

export function render(r) {
  const kind = /^(\w+)-\d{4}/.exec(r.folders.baseline ?? '')?.[1] ?? 'baseline';
  const out = [`# Morning report ${r.window.since.slice(0, 16)} → ${r.window.until.slice(0, 16)}`, ''];
  out.push(
    `Tonight: the lane reports in \`${r.folders.tonight ?? '—'}\`. Against: \`${r.folders.baseline ?? '—'}\`, ${r.baselineWindow.since.slice(0, 16)} → ${r.baselineWindow.until.slice(0, 16)}. A lane counts its chats' whole turns that overlap the window; cost is the usage ledger's, context and shares are transcript usage. Read-only, aggregates only.`,
    '',
    '## Lanes tonight against the evening',
    '',
    '| Lane | Chats | Calls (main + subagent) | Context/call p50 / p95 | Cost at ≥400K | Handovers (context) | Python edits per 100 Bash (main · subagent scripts/Bash) | Edit calls (main + subagent) | Poll-only calls | Subagent share | Cost | Merged PRs | $ per merged PR |',
    '|---|---:|---|---|---:|---|---|---:|---:|---:|---|---:|---:|',
  );
  const lanes = (ls, suffix) => ls.map((l) => row(`${l.lane}${suffix}`, l.chats, l.handovers, l.transcript, l.cost, l.mergedPrs.length, l.usdPerMergedPr));
  const all = (t, label) => row(label, t.chats, null, t.transcript, t.cost, t.merged, t.usdPerMergedPr);
  out.push(...lanes(r.lanes.tonight, ''), all(r.totals.tonight, 'All tonight'), ...lanes(r.lanes.baseline, ` (${kind})`), all(r.totals.baseline, `All ${kind}`));
  out.push('', `Python edit scripts: inline Python (\`python3 - <<EOF\`, \`-c\`) that writes a file. Edit calls are Edit/MultiEdit; Write calls are in the JSON. Poll-only: every tool use a wait. Handover line ${ktok(HANDOVER_AT)}: main-thread calls at or over it, tonight ${r.totals.tonight.transcript?.callsOverHandoverLine ?? '—'}, ${kind} ${r.totals.baseline.transcript?.callsOverHandoverLine ?? '—'}.`);

  out.push('', '## Per lane', '');
  for (const [ls, suffix] of [[r.lanes.tonight, ''], [r.lanes.baseline, ` (${kind})`]]) {
    for (const l of ls) {
      if (!l.keys.length) {
        out.push(`- ${l.lane}${suffix}: no chat named in ${l.file}; pass --lane ${l.lane.toLowerCase()}=<key>; cost unknown.`);
        continue;
      }
      const c = l.cost;
      const parts = [
        `chats ${l.keys.map((k) => `\`${k}\``).join(' → ')}`,
        plural(l.stints, 'stint'),
        `cost ${money(c)} over ${plural(l.runs, 'ledger row')} (${rowsText(c)})`,
        l.mergedPrs.length ? `merged ${l.mergedPrs.map((n) => `#${n}`).join(', ')}` : 'no merged PR',
      ];
      if (l.claimedNotMerged.length) parts.push(`claimed, not merged in the window: ${l.claimedNotMerged.map((n) => `#${n}`).join(', ')}`);
      out.push(`- ${l.lane}${suffix}: ${parts.join('; ')}.`);
    }
  }

  out.push('', '## Gate health and offline deltas', '');
  for (const p of r.placeholders) {
    if (!p.file) out.push(`- ${p.id} ${p.label}: not yet (\`${p.pattern}\`)`);
    else out.push(`- ${p.id} ${p.label} (${p.file}): ${Object.entries(p.values ?? {}).map(([k, v]) => `${k} ${v}`).join(', ') || 'no numbers'}`);
  }

  if (r.product) {
    out.push('', `## Product scorecard against ${r.product.baseline} (not installed: product numbers move after install)`, '');
    out.push(`${r.product.installedAt ? `This window ran the build installed at ${r.product.installedAt}; t` : 'T'}onight's merges reach these numbers only after the next install.`, '');
    out.push(...renderComparison(r.product.comparison).split('\n').slice(2));
  }
  if (r.notes.length) out.push('', '## Notes', '', ...r.notes.map((n) => `- ${n}`));
  return `${out.join('\n')}\n`;
}

// ── CLI ──────────────────────────────────────────────────────────────────

const USAGE = `usage: node scripts/agent-eval/morning-report.mjs --since <launch iso> --hw <HW> [--vault <v> | --db <file>]
       [--repo <checkout>] [--until <iso>] [--baseline-since <iso> --baseline-until <iso>]
       [--tonight <dir>] [--baseline <dir>] [--lane <name>=<session key>]... [--ref <git ref>] [--git-log <file>]
       [--transcripts <dir>]... [--product-baseline <file>] [--no-product] [--installed-at <iso>]
       [--json <file>] [--out <md>]
Times need Z or an offset. Lane reports: <HW>/outputs/overnight-<date> and evening-<date>, the date of
--since / --baseline-since in UTC, the day after, or New York; of two, the one naming the window's chats
(baseline default ${EVENING.since} → ${EVENING.until}, which also seeds ${Object.entries(EVENING_LANES).map(([k, v]) => `${k}=${v}`).join(' ')}).
Merges: git log --first-parent --merges <ref> (origin/main).`;

export function parseArgs(argv) {
  const o = { transcripts: [], lanes: [], noProduct: false };
  const flags = { '--since': 'since', '--until': 'until', '--hw': 'hw', '--vault': 'vault', '--db': 'db', '--repo': 'repo', '--ref': 'ref', '--git-log': 'gitLog', '--tonight': 'tonight', '--baseline': 'baseline', '--baseline-since': 'baselineSince', '--baseline-until': 'baselineUntil', '--product-baseline': 'productBaseline', '--installed-at': 'installedAt', '--json': 'json', '--out': 'out' };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v == null || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (flags[a]) o[flags[a]] = val();
    else if (a === '--transcripts') o.transcripts.push(val());
    else if (a === '--lane') {
      const m = /^(\w+)=(s-\d{8}-[0-9a-f]{8})$/.exec(val());
      if (!m) throw new Error('--lane takes <name>=<session key>, e.g. ob=s-20261017-6b4c62eb');
      o.lanes.push([m[1].toLowerCase(), m[2]]);
    } else if (a === '--no-product') o.noProduct = true;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!o.since) throw new Error('--since is required');
  if (!o.hw && !(o.tonight || o.baseline)) throw new Error('--hw (or --tonight/--baseline) is required');
  const flag = (k) => `--${k.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
  for (const k of ['since', 'until', 'baselineSince', 'baselineUntil', 'installedAt']) if (o[k] != null && at(o[k]) == null) throw new Error(`${flag(k)} must be an ISO time`);
  // Date.parse reads a time with no zone as local time: the window would
  // shift by the machine's offset without a word.
  for (const k of ['since', 'until', 'baselineSince', 'baselineUntil']) if (o[k] != null && !ZONED.test(o[k])) throw new Error(`${flag(k)} needs a zone (Z or an offset like -04:00): ${o[k]} would be read as local time`);
  if (o.until != null && at(o.since) >= at(o.until)) throw new Error('--since must be before --until');
  if (at(o.baselineSince ?? EVENING.since) >= at(o.baselineUntil ?? EVENING.until)) throw new Error('--baseline-since must be before --baseline-until');
  return o;
}
const ZONED = /(?:Z|[+-]\d{2}:?\d{2})$/i;

export async function main(argv, opts = {}) {
  let o;
  try {
    o = parseArgs(argv);
    // No --until: the window ends now. Checked before anything is read.
    if (o.until == null && at(o.since) >= (opts.now ?? Date.now())) throw new Error('--since must be before --until (now)');
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  const r = await morningReport(o, opts);
  const md = render(r);
  if (o.json) writeFileSync(o.json, `${JSON.stringify(r, null, 2)}\n`);
  if (o.out) {
    writeFileSync(o.out, md);
    console.log(`wrote ${o.out}`);
  } else console.log(md);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
