// The Jev study (agent effectiveness E1 scaffold, E3 runner; decision Q3:
// measure all four Jev uses). One condition per use, each with a judged
// fixture (scripts/agent-eval/jev-fixtures/<condition>.json) and a scorer.
// The decisions come from the production code, run by the binary's hidden
// `hollow jev-study` subcommand (src-tauri/src/jev_study.rs) against a
// loopback fake or, only when the orchestrator says --typesafe, TypeSafe:
//
//   hollow jev-study scripts/agent-eval/jev-fixtures <study-dir> --endpoint http://127.0.0.1:<port>/v1/systemone
//   node scripts/agent-eval/jev-study.mjs score <study-dir> [--fixtures <dir>] [--out <dir>]
//        → <out>/score.json and score.md (default out: the study folder)
//
// This file only reads files: nothing here calls Jev, a model or the
// network.
//
// Plan (README "Jev"): keep a use only where it beats the local arm on
// quality, cost or latency, with zero explicit-name and cardinality
// failures. The `gate` below checks the failures that disqualify outright;
// `compareJev` puts the Jev condition beside the local one on the same
// judged items, with paired bootstrap intervals.
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

/** The three uses (docs/JEV.md): what each decides and the arms compared. */
export const JEV_CONDITIONS = {
  'connector-relevance': {
    use: 'connectors',
    decides: 'which library connectors a request attaches (score ≥ 2 attaches)',
    arms: ['local', 'jev', 'agent'],
  },
  'passage-rerank': {
    use: 'passages',
    decides: 'which retrieved passages stay as evidence (score < 1 dropped)',
    arms: ['local', 'jev', 'agent'],
  },
  'proposal-dedup': {
    use: 'decisions',
    decides: 'whether a task or learning proposal repeats an existing one (≥ 0.85 drops, ≥ 0.5 flags)',
    arms: ['exact', 'jev'],
  },
};
/** `off`: the local condition (Jev not asked). `not-sent`: Jev was on but
 *  production sends nothing for the item (a named connector only, under two
 *  passages, an exact duplicate). */
export const JEV_OUTCOMES = ['ok', 'timeout', 'malformed', 'error', 'off', 'not-sent'];
/** Outcomes where production fell back to its local decision. The runner
 *  records that fallback as the answer, so every result carries one; a
 *  result with one of these outcomes and no answer is scored as a failure
 *  (`missingAnswers`), not refused (#386 review). An `ok` or `not-sent`
 *  result without its answer is a malformed file. */
export const FALLBACK_OUTCOMES = ['timeout', 'malformed', 'error', 'off'];
/** The answer field each condition's results carry. */
export const ANSWER_FIELD = {
  'connector-relevance': 'decisions', 'passage-rerank': 'kept', 'proposal-dedup': 'verdict',
};
/** The case labels a fixture item may carry (`kind`), per condition. */
export const ITEM_KINDS = {
  'connector-relevance': ['plain', 'ambiguous', 'paraphrase', 'explicit', 'cardinality', 'none'],
  'passage-rerank': ['plain', 'buried', 'stale', 'historical', 'paraphrase', 'decoy', 'multi', 'none', 'single'],
  'proposal-dedup': ['exact', 'reworded', 'related', 'novel'],
};

const ID = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const fail = (msg) => { throw new Error(msg); };
const ids = (list, at) => {
  if (!Array.isArray(list) || !list.length) fail(`${at} must be a non-empty list`);
  const seen = new Set();
  for (const x of list) {
    if (typeof x?.id !== 'string' || !ID.test(x.id)) fail(`${at}: every entry needs an id`);
    if (seen.has(x.id)) fail(`${at}: id ${x.id} appears twice`);
    seen.add(x.id);
  }
  return seen;
};
const subset = (list, of, at) => {
  if (!Array.isArray(list) || !list.every((id) => of.has(id))) fail(`${at} must name ids from the item's own list`);
};
const text = (s, at) => (typeof s === 'string' && s.trim() ? s : fail(`${at} must be text`));

const ITEM = {
  'connector-relevance': (i, at) => {
    text(i.request, `${at}.request`);
    const c = ids(i.candidates, `${at}.candidates`);
    i.candidates.forEach((x, n) => text(x.description, `${at}.candidates[${n}].description`));
    subset(i.relevant, c, `${at}.relevant`);
    subset(i.named ?? [], new Set(i.relevant), `${at}.named (every named connector is relevant)`);
  },
  'passage-rerank': (i, at) => {
    text(i.request, `${at}.request`);
    const p = ids(i.passages, `${at}.passages`);
    i.passages.forEach((x, n) => { text(x.title, `${at}.passages[${n}].title`); text(x.excerpt, `${at}.passages[${n}].excerpt`); });
    subset(i.relevant, p, `${at}.relevant`);
    subset(i.stale ?? [], p, `${at}.stale`);
  },
  'proposal-dedup': (i, at) => {
    if (!['task', 'learning'].includes(i.subject)) fail(`${at}.subject must be task or learning`);
    text(i.proposal?.title, `${at}.proposal.title`);
    const e = ids(i.existing, `${at}.existing`);
    if (i.duplicateOf != null && !e.has(i.duplicateOf)) fail(`${at}.duplicateOf must be null (novel) or an existing id`);
  },
};

/** A judged fixture: `{ schema: 1, condition, about, items: [...] }`. */
export function validateJevFixture(json) {
  if (!json || json.schema !== 1) fail('a Jev fixture needs schema 1');
  if (!JEV_CONDITIONS[json.condition]) fail(`condition must be one of ${Object.keys(JEV_CONDITIONS).join(', ')}`);
  if (!Array.isArray(json.items) || !json.items.length) fail('items must be a non-empty list');
  ids(json.items, 'items');
  json.items.forEach((item, n) => {
    const at = `items[${n}] (${item.id})`;
    if (item.kind != null && !ITEM_KINDS[json.condition].includes(item.kind)) fail(`${at}.kind must be one of ${ITEM_KINDS[json.condition].join(', ')}`);
    ITEM[json.condition](item, at);
  });
  return json;
}

export function loadJevFixture(file) {
  const raw = readFileSync(file);
  const fixture = validateJevFixture(JSON.parse(raw.toString('utf8')));
  Object.defineProperty(fixture, 'sha256', { value: createHash('sha256').update(raw).digest('hex') });
  return fixture;
}

export const DEDUP_VERDICTS = ['new', 'possible', 'duplicate'];

/** Each condition's answer, checked for shape before anything is scored:
 *  a malformed answer is refused, never scored as right or as a miss.
 *  Cardinality (one decision per connector, only offered passages) is left
 *  to the scorer, which counts it as the failure it measures. */
const ANSWER = {
  'connector-relevance': (r, item, at) => {
    if (!Array.isArray(r.decisions)) fail(`${at}: decisions must be a list of { id, attach }`);
    r.decisions.forEach((d, n) => {
      if (typeof d?.id !== 'string' || !d.id) fail(`${at}: decisions[${n}].id must be a connector id`);
      if (typeof d.attach !== 'boolean') fail(`${at}: decisions[${n}].attach must be true or false`);
    });
  },
  'passage-rerank': (r, item, at) => {
    if (!Array.isArray(r.kept) || !r.kept.every((id) => typeof id === 'string' && id)) fail(`${at}: kept must be a list of passage ids`);
  },
  'proposal-dedup': (r, item, at) => {
    if (!DEDUP_VERDICTS.includes(r.verdict)) fail(`${at}: verdict must be one of ${DEDUP_VERDICTS.join(', ')}`);
    if (r.verdict === 'new') {
      if (r.of != null) fail(`${at}: a new proposal's of must be null`);
    } else if (!item.existing.some((e) => e.id === r.of)) {
      fail(`${at}: of must name one of the item's existing records for a ${r.verdict} verdict`);
    }
  },
};

/** One arm's recorded outputs: `{ schema: 1, condition, arm, complete?,
 *  results: [{ id, outcome, latencyMs, costUsd, calls?, ...answer }] }`, one
 *  per item. The answer is `decisions: [{ id, attach }]` (connectors),
 *  `kept: [id]` (passages), `verdict` (new, possible or duplicate) + `of`
 *  (dedup),
 *  checked by [`ANSWER`]. A result whose outcome is a fallback
 *  ([`FALLBACK_OUTCOMES`]) may lack it: that is a failure the scorer counts.
 *  `complete: false` (the runner stopped at --max-calls) is refused: an arm
 *  that didn't answer every item isn't scored. */
export function validateJevOutputs(fixture, out) {
  if (!out || out.schema !== 1) fail('outputs need schema 1');
  if (out.condition !== fixture.condition) fail(`outputs are for condition ${out.condition}, the fixture is ${fixture.condition}`);
  if (!JEV_CONDITIONS[fixture.condition].arms.includes(out.arm)) fail(`arm must be one of ${JEV_CONDITIONS[fixture.condition].arms.join(', ')}`);
  if (out.complete === false) fail(`the ${out.arm} arm of ${out.condition} stopped before every item was answered (see the study's run.json); an incomplete arm is not scored`);
  const items = new Map(fixture.items.map((i) => [i.id, i]));
  const seen = new Set();
  for (const r of out.results ?? []) {
    if (!items.has(r.id)) fail(`result ${r.id} is not in the fixture`);
    if (seen.has(r.id)) fail(`result ${r.id} appears twice`);
    seen.add(r.id);
    if (!JEV_OUTCOMES.includes(r.outcome)) fail(`${r.id}: outcome must be one of ${JEV_OUTCOMES.join(', ')}`);
    if (r.latencyMs != null && !Number.isFinite(r.latencyMs)) fail(`${r.id}: latencyMs must be a number or null`);
    if (r.costUsd != null && !Number.isFinite(r.costUsd)) fail(`${r.id}: costUsd must be a number or null (unknown)`);
    if (r.calls != null && !(Number.isInteger(r.calls) && r.calls >= 0)) fail(`${r.id}: calls must be a whole number`);
    if (missingAnswer(fixture.condition, r)) continue;
    ANSWER[fixture.condition](r, items.get(r.id), r.id);
  }
  for (const id of items.keys()) if (!seen.has(id)) fail(`no result for item ${id}`);
  return out;
}

/** A fallback result without its answer: scored as a failure. */
function missingAnswer(condition, r) {
  return FALLBACK_OUTCOMES.includes(r.outcome) && r[ANSWER_FIELD[condition]] === undefined;
}

const round4 = (n) => Math.round(n * 1e4) / 1e4;
const ratio = (a, b) => (b ? round4(a / b) : null);
const nearest = (values, q) => {
  const v = values.filter(Number.isFinite).sort((a, b) => a - b);
  return v.length ? v[Math.max(0, Math.ceil(q * v.length) - 1)] : null;
};
const sum = (rows, key) => rows.reduce((n, r) => n + (r[key] ?? 0), 0);
const f1 = (tp, fp, fn) => (2 * tp + fp + fn ? round4((2 * tp) / (2 * tp + fp + fn)) : null);

/** Each condition's per-item counts, so metrics are sums over items and a
 *  paired bootstrap can resample items. `missing`: a fallback result with no
 *  answer, scored as an empty one. */
const COUNT = {
  'connector-relevance': (item, r) => {
    const decisions = Array.isArray(r.decisions) ? r.decisions : [];
    const sameSet = decisions.length === item.candidates.length && item.candidates.every((c) => decisions.some((d) => d.id === c.id));
    const on = new Set(decisions.filter((d) => d.attach).map((d) => d.id));
    const tp = item.relevant.filter((id) => on.has(id)).length;
    return {
      tp, fp: on.size - tp, fn: item.relevant.length - tp,
      namedDropped: (item.named ?? []).filter((id) => !on.has(id)).length,
      cardinality: sameSet ? 0 : 1,
    };
  },
  'passage-rerank': (item, r) => {
    const list = Array.isArray(r.kept) ? r.kept : [];
    if (new Set(list).size !== list.length) fail(`${item.id}: a passage is kept twice`);
    const tp = item.relevant.filter((id) => list.includes(id)).length;
    return {
      tp, fp: list.length - tp, fn: item.relevant.length - tp,
      staleKept: (item.stale ?? []).filter((id) => list.includes(id)).length,
      cardinality: list.every((id) => item.passages.some((p) => p.id === id)) ? 0 : 1,
    };
  },
  'proposal-dedup': (item, r) => {
    const dup = r.verdict === 'duplicate';
    const right = dup && item.duplicateOf != null && r.of === item.duplicateOf;
    return {
      // The positive decision is "drop it as a duplicate".
      tp: right ? 1 : 0,
      fp: dup && !right ? 1 : 0,
      fn: item.duplicateOf != null && !right ? 1 : 0,
      falseOmission: dup && item.duplicateOf == null ? 1 : 0,
      missedDuplicate: r.verdict === 'new' && item.duplicateOf != null ? 1 : 0,
      wrongTarget: dup && item.duplicateOf != null && !right ? 1 : 0,
      flagged: r.verdict === 'possible' ? 1 : 0,
      correct: right || (r.verdict === 'new' && item.duplicateOf == null) ? 1 : 0,
    };
  },
};

/** Metrics and the disqualifying failures from summed counts. */
const METRICS = {
  'connector-relevance': (c) => ({
    metrics: {
      recall: ratio(c.tp, c.tp + c.fn), precision: ratio(c.tp, c.tp + c.fp), f1: f1(c.tp, c.fp, c.fn),
      namedDropped: c.namedDropped, cardinalityFailures: c.cardinality,
    },
    failures: [
      ...(c.namedDropped ? [`${c.namedDropped} named connector(s) not attached`] : []),
      ...(c.cardinality ? [`${c.cardinality} result(s) without exactly one decision per candidate`] : []),
    ],
  }),
  'passage-rerank': (c) => ({
    metrics: {
      recall: ratio(c.tp, c.tp + c.fn), precision: ratio(c.tp, c.tp + c.fp), f1: f1(c.tp, c.fp, c.fn),
      droppedRelevant: c.fn, staleKept: c.staleKept, cardinalityFailures: c.cardinality,
    },
    failures: c.cardinality ? [`${c.cardinality} result(s) keep passages that were not offered`] : [],
  }),
  'proposal-dedup': (c) => ({
    metrics: {
      // Precision and recall of "drop as a duplicate of the right record".
      recall: ratio(c.tp, c.tp + c.fn), precision: ratio(c.tp, c.tp + c.fp), f1: f1(c.tp, c.fp, c.fn),
      correct: c.correct, falseOmissions: c.falseOmission, missedDuplicates: c.missedDuplicate, wrongTarget: c.wrongTarget, flagged: c.flagged,
    },
    // A novel proposal dropped is lost work; a wrong target drops one too.
    failures: [
      ...(c.falseOmission ? [`${c.falseOmission} novel proposal(s) dropped as duplicates`] : []),
      ...(c.wrongTarget ? [`${c.wrongTarget} dropped against the wrong record`] : []),
    ],
  }),
};

const EMPTY_ANSWER = { 'connector-relevance': { decisions: [] }, 'passage-rerank': { kept: [] }, 'proposal-dedup': { verdict: null, of: null } };

/** Per item: its fixture entry, its result and its counts. */
function itemCounts(fixture, out) {
  return fixture.items.map((item) => {
    const r = out.results.find((x) => x.id === item.id);
    const missing = missingAnswer(fixture.condition, r);
    return { item, r, missing, counts: COUNT[fixture.condition](item, missing ? { ...r, ...EMPTY_ANSWER[fixture.condition] } : r) };
  });
}

function totals(rows) {
  const out = {};
  for (const { counts } of rows) for (const [k, v] of Object.entries(counts)) out[k] = (out[k] ?? 0) + v;
  return out;
}

/** Score one arm's outputs against its judged fixture: quality metrics,
 *  the gate's disqualifying failures, outcomes, latency, requests and cost
 *  (an unknown cost stays null, never 0), and the metrics per item kind. */
export function scoreJev(fixture, out) {
  validateJevFixture(fixture);
  validateJevOutputs(fixture, out);
  const rows = itemCounts(fixture, out);
  const { metrics, failures } = METRICS[fixture.condition](totals(rows));
  const missingAnswers = rows.filter((x) => x.missing).length;
  metrics.missingAnswers = missingAnswers;
  if (missingAnswers) failures.push(`${missingAnswers} result(s) without an answer (a fallback must record the decision production kept)`);
  const outcomes = Object.fromEntries(JEV_OUTCOMES.map((o) => [o, out.results.filter((r) => r.outcome === o).length]));
  const known = out.results.filter((r) => r.costUsd != null);
  const kinds = [...new Set(fixture.items.map((i) => i.kind).filter(Boolean))].sort();
  return {
    status: 'scored',
    condition: fixture.condition,
    arm: out.arm,
    items: fixture.items.length,
    metrics,
    byKind: Object.fromEntries(kinds.map((k) => {
      const of = rows.filter((x) => x.item.kind === k);
      return [k, { items: of.length, ...METRICS[fixture.condition](totals(of)).metrics }];
    })),
    outcomes,
    latency: { p50Ms: nearest(out.results.map((r) => r.latencyMs), 0.5), p95Ms: nearest(out.results.map((r) => r.latencyMs), 0.95) },
    calls: out.results.every((r) => Number.isInteger(r.calls)) ? sum(out.results, 'calls') : null,
    cost: { totalUsd: known.length === out.results.length ? round4(known.reduce((n, r) => n + r.costUsd, 0)) : null, unknownResults: out.results.length - known.length },
    gate: { pass: failures.length === 0, reasons: failures },
  };
}

/** A seeded generator (mulberry32), so an interval is the same every run
 *  (also skill-lift.mjs's bootstrap). */
export function seeded(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export const BOOTSTRAP = { resamples: 2000, seed: 20261006 };

/** Jev minus local on `metric`, with a 95% paired bootstrap interval over
 *  items (percentile method, seeded): the same resampled items for both
 *  arms. `null` when the metric is undefined for either arm. */
export function pairedDifference(condition, localRows, jevRows, metric, { resamples = BOOTSTRAP.resamples, seed = BOOTSTRAP.seed } = {}) {
  const value = (rows) => METRICS[condition](totals(rows)).metrics[metric];
  const a = value(localRows);
  const b = value(jevRows);
  if (a == null || b == null) return null;
  const rand = seeded(seed);
  const n = localRows.length;
  const diffs = [];
  for (let k = 0; k < resamples; k++) {
    const pick = Array.from({ length: n }, () => Math.floor(rand() * n));
    const la = value(pick.map((i) => localRows[i]));
    const lb = value(pick.map((i) => jevRows[i]));
    if (la != null && lb != null) diffs.push(lb - la);
  }
  diffs.sort((x, y) => x - y);
  const q = (p) => (diffs.length ? round4(diffs[Math.min(diffs.length - 1, Math.max(0, Math.floor(p * diffs.length)))]) : null);
  return { local: a, jev: b, difference: round4(b - a), low: q(0.025), high: q(0.975), resamples: diffs.length };
}

/** The headline metric each condition is compared on. */
export const HEADLINE = { 'connector-relevance': 'f1', 'passage-rerank': 'f1', 'proposal-dedup': 'f1' };

/** The local and Jev conditions of one use on the same judged items. The
 *  `finding` reads only the headline interval; whether Jev stays is the
 *  plan's call (zero explicit-name and cardinality failures, and a net gain
 *  after its cost), recorded under `keep`. */
export function compareJev(fixture, local, jev) {
  const a = scoreJev(fixture, local);
  const b = scoreJev(fixture, jev);
  const la = itemCounts(fixture, local);
  const lb = itemCounts(fixture, jev);
  const metrics = ['f1', 'precision', 'recall'];
  const difference = Object.fromEntries(metrics.map((m) => [m, pairedDifference(fixture.condition, la, lb, m)]));
  const head = difference[HEADLINE[fixture.condition]];
  const finding = head == null ? 'not comparable'
    : head.low === 0 && head.high === 0 && head.difference === 0 ? 'no difference (the same score on every resample)'
    : head.low > 0 ? 'jev better (interval above 0)'
    : head.high < 0 ? 'jev worse (interval below 0)'
    : 'no difference shown (interval spans 0)';
  const disqualified = [
    ...(b.metrics.namedDropped ? ['a named connector dropped'] : []),
    ...(b.metrics.cardinalityFailures ? ['a cardinality failure'] : []),
    ...(b.metrics.missingAnswers ? ['a result without an answer'] : []),
  ];
  return {
    condition: fixture.condition,
    items: fixture.items.length,
    arms: { local: a, jev: b },
    difference,
    latency: { p50MsAdded: a.latency.p50Ms == null || b.latency.p50Ms == null ? null : round4(b.latency.p50Ms - a.latency.p50Ms), p95MsAdded: a.latency.p95Ms == null || b.latency.p95Ms == null ? null : round4(b.latency.p95Ms - a.latency.p95Ms) },
    calls: b.calls,
    costUsd: b.cost.totalUsd,
    finding,
    keep: {
      disqualified,
      // Plan gate: zero explicit-name/cardinality failures and a gain that
      // survives its interval; the cost is unknown (null) until TypeSafe
      // reports one, so "after its cost" stays open.
      candidate: !disqualified.length && head != null && head.low > 0,
      costKnown: b.cost.totalUsd != null,
    },
  };
}

/** A study folder as the runner wrote it: run.json and two arms per
 *  condition. Each condition is compared on its fixture (the run's own
 *  `fixtures` folder unless `fixturesDir` is given), whose digest must match
 *  the one the run recorded; an incomplete arm, or a use that is gone, is
 *  reported, not scored. */
export function scoreStudy(dir, { fixturesDir = null } = {}) {
  const runFile = join(dir, 'run.json');
  if (!existsSync(runFile)) fail(`${dir} has no run.json: not a jev-study folder`);
  const run = JSON.parse(readFileSync(runFile, 'utf8'));
  if (run.schema !== 1) fail('run.json needs schema 1');
  const fixtures = fixturesDir ?? run.fixtures;
  const conditions = [];
  for (const c of run.conditions ?? []) {
    // A use that no longer exists (a study recorded before it went) is
    // reported, and the rest of the study still scores.
    if (!Object.prototype.hasOwnProperty.call(JEV_CONDITIONS, c.condition)) {
      const why = c.condition === 'whiteboard-layout' ? 'it went with whiteboards on 2026-10-07' : 'it is not a use this scorer knows';
      conditions.push({ condition: c.condition, status: 'retired', reason: `not scored: ${why}` });
      continue;
    }
    const fixture = loadJevFixture(join(fixtures, `${c.condition}.json`));
    const items = run.items ? { ...fixture, items: fixture.items.slice(0, run.items) } : fixture;
    if (c.fixtureSha256 && c.fixtureSha256 !== fixture.sha256) {
      conditions.push({ condition: c.condition, status: 'fixture-changed', reason: `the fixture in ${fixtures} is not the one the run used (sha256 ${c.fixtureSha256.slice(0, 12)}…)` });
      continue;
    }
    const arms = Object.entries(c.arms ?? {});
    const localArm = arms.find(([arm]) => arm !== 'jev');
    const jevArm = arms.find(([arm]) => arm === 'jev');
    // An interrupted run (state running or failed) may not have written
    // every arm yet: a missing file is an incomplete condition, not a throw.
    const read = (file) => (existsSync(join(dir, file)) ? JSON.parse(readFileSync(join(dir, file), 'utf8')) : null);
    const local = localArm ? read(localArm[1]) : null;
    const jev = jevArm ? read(jevArm[1]) : null;
    if (!local || !jev || local.complete === false || jev.complete === false) {
      const reason = run.stopped
        ? `the run stopped: ${run.stopped.reason} (before ${run.stopped.condition} ${run.stopped.before})`
        : run.state === 'running' || run.state === 'failed'
          ? `the run did not finish (run.json state ${run.state}${run.error ? `: ${run.error}` : ''}); continue it with --resume`
          : 'an arm is missing';
      conditions.push({ condition: c.condition, status: 'incomplete', reason });
      continue;
    }
    conditions.push({ status: 'compared', ...compareJev(items, local, jev) });
  }
  return {
    study: dir,
    model: run.model ?? null,
    endpoint: run.endpoint?.kind ?? null,
    version: run.version ?? null,
    calls: run.calls ?? null,
    maxCalls: run.maxCalls ?? null,
    state: run.state ?? null,
    stopped: run.stopped ?? null,
    costUsd: run.costUsd ?? null,
    bootstrap: BOOTSTRAP,
    conditions,
  };
}

const pct = (v) => (v == null ? '–' : `${(v * 100).toFixed(1)}`);
const pts = (v) => (v == null ? '–' : (v * 100).toFixed(1));
const span = (d) => (d == null ? '–' : `${d.difference >= 0 ? '+' : ''}${pts(d.difference)} [${pts(d.low)}, ${pts(d.high)}]`);
const msv = (v) => (v == null ? '–' : `${v}`);

export function renderStudy(r) {
  const lines = [
    `# Jev study: ${r.endpoint ?? '?'} endpoint, ${r.model ?? '?'}`, '',
    `Requests: ${r.calls ?? '?'} of at most ${r.maxCalls ?? '?'}${r.stopped ? `; stopped: ${r.stopped.reason}` : ''}. Cost: ${r.costUsd == null ? 'unknown (null)' : `$${r.costUsd}`}.`,
    `Intervals: 95% paired bootstrap over items (${r.bootstrap.resamples} resamples, seed ${r.bootstrap.seed}), in points.`, '',
    '| Condition | Items | Local P / R / F1 | Jev P / R / F1 | Jev − local (headline) | Jev outcomes | p50 / p95 ms added | Gate (Jev) | Finding |',
    '|---|---|---|---|---|---|---|---|---|',
  ];
  for (const c of r.conditions) {
    if (c.status !== 'compared') { lines.push(`| ${c.condition} | – | – | – | – | – | – | – | ${c.status}: ${c.reason} |`); continue; }
    const m = (arm) => `${pct(arm.metrics.precision)} / ${pct(arm.metrics.recall)} / ${pct(arm.metrics.f1)}`;
    const outcomes = Object.entries(c.arms.jev.outcomes).filter(([, n]) => n).map(([o, n]) => `${o} ${n}`).join(', ');
    lines.push(`| ${c.condition} | ${c.items} | ${m(c.arms.local)} | ${m(c.arms.jev)} | ${HEADLINE[c.condition]} ${span(c.difference[HEADLINE[c.condition]])} | ${outcomes} | ${msv(c.latency.p50MsAdded)} / ${msv(c.latency.p95MsAdded)} | ${c.arms.jev.gate.pass ? 'pass' : c.arms.jev.gate.reasons.join('; ')} | ${c.finding} |`);
  }
  lines.push('');
  return `${lines.join('\n')}\n`;
}

const USAGE = 'usage: node scripts/agent-eval/jev-study.mjs score <study-dir> [--fixtures <dir>] [--out <dir>]';

export function main(argv) {
  const [command, ...rest] = argv;
  const positional = [];
  let fixturesDir = null;
  let out = null;
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--fixtures') fixturesDir = rest[++i];
    else if (rest[i] === '--out') out = rest[++i];
    else if (rest[i].startsWith('--')) throw new Error(`unknown flag ${rest[i]}\n${USAGE}`);
    else positional.push(rest[i]);
  }
  if (command !== 'score' || positional.length !== 1 || fixturesDir === undefined || out === undefined) throw new Error(USAGE);
  const report = scoreStudy(positional[0], { fixturesDir });
  const dir = out ?? positional[0];
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'score.json'), `${JSON.stringify(report, null, 2)}\n`);
  writeFileSync(join(dir, 'score.md'), renderStudy(report));
  console.log(renderStudy(report));
  console.log(`[jev-study] wrote ${join(dir, 'score.json')} and score.md`);
  return report;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
