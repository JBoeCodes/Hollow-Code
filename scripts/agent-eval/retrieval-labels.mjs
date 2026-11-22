// The retrieval label report (HF06): how well the broker's scores predict
// what Jev kept, over real runs, and which fusion weights and semantic floor
// would have ordered those runs best.
//
//   node scripts/agent-eval/retrieval-labels.mjs <manifests-dir> [--json <file>]
//
// <manifests-dir> is a folder of run manifests (a vault's
// `.evidence-manifests/<runId>.json`). A run counts when its retrieval was
// labelled by Jev (`retrieval.engine === "jev"`); of its candidates only
// those Jev decided (`basis === "jev"`) are read, and the rest of a mixed
// pool is reported as `unlabeled` (counted, not scored; a Jev-decided one
// without a boolean `kept` or a rank as `malformed`). The label is `kept`: Jev
// kept the passage and it fit the evidence budget. Jev's keeps are a proxy
// for relevance, not ground truth.
//
// It reports:
// 1. AUC of each broker score against `kept`, Mann–Whitney with ties as 0.5:
//    the broker rank (negated: lower is better), `localScore` (query terms
//    the line matched) and `semanticScore` (a candidate without one is 0: no
//    hit at or above the floor). Pooled over every candidate, and the mean
//    of per-run AUCs; a run whose candidates were all kept or none kept has
//    no AUC and is counted, not averaged.
// 2. The keep rate (Wilson 95%) by `semanticScore` band and `localScore` bin.
// 3. A sweep of reciprocal-rank fusion weights (lexical × semantic, 0..2 by
//    0.25) and semantic floors (0.50..0.80 by 0.05). Each run's pool is
//    re-fused offline the way `retrieval_broker.rs` `blend` does it —
//    `wL / (60 + lexical rank) + wS / (60 + semantic rank)`, a missing half
//    adding 0, ties to a lexical rank, the better lexical rank, then the
//    better semantic rank — and scored by the AUC of the new position. The
//    lexical rank is rebuilt inside the recorded pool (localScore
//    descending, the recorded rank breaking ties), the semantic rank from
//    the distinct semanticScores at or above the floor (a note's passages
//    share its score). A semantic-only passage (localScore 0) under a raised
//    floor leaves the pool and ranks last. The rebuild is approximate: the
//    real lexical and semantic lists reach past the 24-passage pool, and
//    per-file diversity and the lifecycle policy need paths and notes, which
//    this report never reads (an earlier draft that rebuilt diversity from
//    paths moved no candidate, but that is not re-checked here). The lexical
//    rebuild breaks localScore ties by the recorded rank, which already holds
//    today's fused order, so lexical-heavy settings partly replay it: trust a
//    floor result more than a weight result. The manifests record no hit under
//    0.6, so floors below it are reported but equal 0.6. The report says how
//    closely the rebuild at the current setting reproduces the recorded
//    rank; compare the sweep with that rebuilt current setting, not with the
//    recorded rank's AUC.
// 4. Agent labels (S26), from what the agent itself did, on any engine's
//    runs: the citation rate (kept passages the final reply named,
//    `candidates[].cited`, over the kept ones, Wilson 95%) and each score's
//    AUC against `cited` among the kept, over runs whose reply was checked
//    (`retrieval.citedChecked`); and need-recall, the share of the notes the
//    agent opened outside its folder (`agentReads.notesOpened`) that were
//    among retrieval's candidates (and kept), over runs where retrieval ran
//    (candidates, and no `skipped-…` `queryBasis`); notes opened in the
//    others are counted apart (`noRetrieval`). Manifests from before
//    2026-10-17 carry neither, and count for nothing here.
//
// Only numbers and the kept/basis/cited labels reach the output: no path,
// passage text or reason (need-recall compares the opened notes' paths with
// the candidates' and reports counts). This file only reads files (and
// writes --json).
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { percentile, wilson } from './score.mjs';

/** `retrieval_broker.rs`: RRF_K, LEXICAL_WEIGHT, SEMANTIC_WEIGHT, SEMANTIC_MIN. */
export const RRF_K = 60;
export const CURRENT = Object.freeze({ lexicalWeight: 1, semanticWeight: 1, floor: 0.6 });
/** The lowest floor the manifests can tell apart (no hit under it is recorded). */
export const RECORDED_FLOOR = 0.6;
export const WEIGHTS = [0, 0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
export const FLOORS = [0.5, 0.55, 0.6, 0.65, 0.7, 0.75, 0.8];
/** Lower bounds; the last band is open-ended. */
export const SEMANTIC_BANDS = [0.6, 0.65, 0.7, 0.75, 0.8, 0.85, 0.9];
export const LOCAL_BINS = [0, 1, 2, 3, 4, 5, 7, 10];
export const SCORES = ['rank', 'local', 'semantic'];
export const PROXY_NOTE =
  'Jev labels are a proxy, not ground truth: `kept` is Jev keeping a passage that then fit the 8-passage evidence budget.';

const EPS = 1e-9;
const round4 = (n) => (n == null ? null : Math.round(n * 1e4) / 1e4);
const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

/** Each score as a number, higher meaning "more likely kept". */
const SCORE_OF = {
  rank: (c) => -c.rank,
  local: (c) => c.localScore,
  semantic: (c) => c.semanticScore ?? 0,
};

// ── AUC ─────────────────────────────────────────────────────────────────

/** Mann–Whitney AUC of `scores` against boolean `labels`: the chance a kept
 *  item outscores a dropped one, ties counting 0.5. Null when either class
 *  is empty. O(n log n): midranks of the pooled sort. */
export function auc(scores, labels) {
  const items = scores.map((s, i) => ({ s, l: Boolean(labels[i]) })).sort((a, b) => a.s - b.s);
  const pos = items.filter((x) => x.l).length;
  const neg = items.length - pos;
  if (!pos || !neg) return null;
  let rankSum = 0;
  for (let i = 0; i < items.length; ) {
    let j = i;
    while (j < items.length && items[j].s === items[i].s) j += 1;
    const mid = (i + 1 + j) / 2;
    for (let k = i; k < j; k += 1) if (items[k].l) rankSum += mid;
    i = j;
  }
  return (rankSum - (pos * (pos + 1)) / 2) / (pos * neg);
}

// ── Loading ─────────────────────────────────────────────────────────────

/** A manifest's Jev-labelled candidates, numbers only, or why it has none.
 *  In a mixed pool the candidates Jev did not decide are counted as
 *  `unlabeled`, and the ones Jev decided that lack a boolean `kept` or a
 *  rank as `malformed` (both read no further), never silently dropped; a
 *  skipped Jev run carries both counts too. */
export function labelledRun(manifest) {
  const r = manifest?.retrieval;
  if (!r || !Array.isArray(r.candidates) || !r.candidates.length) return { skip: 'noCandidates' };
  if (r.engine !== 'jev') return { skip: 'notJev' };
  const candidates = [];
  let unlabeled = 0;
  let malformed = 0;
  for (const c of r.candidates) {
    if (c?.basis !== 'jev') {
      unlabeled += 1;
      continue;
    }
    if (!Number.isFinite(c.rank) || typeof c.kept !== 'boolean') {
      malformed += 1;
      continue;
    }
    candidates.push({
      rank: c.rank,
      localScore: Number.isFinite(c.localScore) ? c.localScore : 0,
      semanticScore: Number.isFinite(c.semanticScore) ? c.semanticScore : null,
      kept: c.kept,
    });
  }
  if (!candidates.length) return { skip: 'notJev', unlabeled, malformed };
  candidates.sort((a, b) => a.rank - b.rank);
  return { runId: typeof manifest.runId === 'string' ? manifest.runId : null, candidates, unlabeled, malformed };
}

/** Every `.json` manifest in `dir`: the labelled runs and what was skipped
 *  (`unlabeled`: candidates Jev did not decide, `malformed`: ones it decided
 *  without a kept label or rank, both inside Jev runs, labelled or skipped),
 *  and `agent`, the runs with agent labels (`agentRun`), whatever their
 *  engine. */
export function loadRuns(dir) {
  const counts = { files: 0, unreadable: 0, noCandidates: 0, notJev: 0, labelled: 0, unlabeled: 0, malformed: 0 };
  const runs = [];
  const agent = [];
  for (const name of readdirSync(dir).filter((f) => f.endsWith('.json')).sort()) {
    counts.files += 1;
    let manifest;
    try {
      manifest = JSON.parse(readFileSync(join(dir, name), 'utf8'));
    } catch {
      counts.unreadable += 1;
      continue;
    }
    const labels = agentRun(manifest);
    if (labels) agent.push(labels);
    const run = labelledRun(manifest);
    counts.unlabeled += run.unlabeled ?? 0;
    counts.malformed += run.malformed ?? 0;
    if (run.skip) counts[run.skip] += 1;
    else {
      counts.labelled += 1;
      runs.push(run);
    }
  }
  return { runs, counts, agent };
}

// ── 1. AUC per score ────────────────────────────────────────────────────

/** Pooled and per-run AUC of each score (or of `scoreOf`'s, when given:
 *  {name: (candidate, run) => number}). Single-class runs are counted. */
export function summarize(runs, scoreOf = SCORE_OF) {
  const names = Object.keys(scoreOf);
  const usable = runs.filter((r) => {
    const k = r.candidates.filter((c) => c.kept).length;
    return k > 0 && k < r.candidates.length;
  });
  const allKept = runs.filter((r) => r.candidates.every((c) => c.kept)).length;
  const noneKept = runs.filter((r) => r.candidates.every((c) => !c.kept)).length;
  const pooledScores = Object.fromEntries(names.map((n) => [n, []]));
  const labels = [];
  for (const r of runs)
    for (const c of r.candidates) {
      labels.push(c.kept);
      for (const n of names) pooledScores[n].push(scoreOf[n](c, r));
    }
  const perRun = usable.map((r) => ({
    runId: r.runId ?? null,
    candidates: r.candidates.length,
    kept: r.candidates.filter((c) => c.kept).length,
    ...Object.fromEntries(names.map((n) => [n, round4(auc(r.candidates.map((c) => scoreOf[n](c, r)), r.candidates.map((c) => c.kept)))])),
  }));
  const scores = {};
  for (const n of names) {
    const each = usable.map((r) => auc(r.candidates.map((c) => scoreOf[n](c, r)), r.candidates.map((c) => c.kept)));
    scores[n] = {
      pooled: round4(auc(pooledScores[n], labels)),
      meanPerRun: round4(mean(each)),
      // Nearest-rank: the lower middle value when the count is even.
      medianPerRun: round4(percentile(each, 0.5)),
    };
  }
  return {
    runs: runs.length,
    usableRuns: usable.length,
    skipped: { allKept, noneKept },
    candidates: labels.length,
    kept: labels.filter(Boolean).length,
    unlabeled: runs.reduce((n, r) => n + (r.unlabeled ?? 0), 0),
    malformed: runs.reduce((n, r) => n + (r.malformed ?? 0), 0),
    scores,
    perRun,
  };
}

// ── 2. Keep rate by band ────────────────────────────────────────────────

/** Keep rate of `candidates` by `field`, banded at `edges` (lower bounds,
 *  ascending; the last band open-ended). A missing value is `none`, one
 *  under the first edge `below`; empty bands are left out. */
export function bands(candidates, field, edges, { decimals = 2 } = {}) {
  const fmt = (x) => (decimals ? x.toFixed(decimals) : String(x));
  const label = (i) => {
    if (i === edges.length - 1) return `≥${fmt(edges[i])}`;
    const hi = edges[i + 1];
    if (!decimals && hi - edges[i] === 1) return fmt(edges[i]);
    return decimals ? `${fmt(edges[i])}–${fmt(hi)}` : `${fmt(edges[i])}–${fmt(hi - 1)}`;
  };
  const rows = new Map();
  const add = (key, kept) => {
    const row = rows.get(key) ?? { band: key, n: 0, kept: 0 };
    row.n += 1;
    if (kept) row.kept += 1;
    rows.set(key, row);
  };
  for (const c of candidates) {
    const v = c[field];
    if (!Number.isFinite(v)) add('none', c.kept);
    else if (v < edges[0] - EPS) add('below', c.kept);
    else {
      let i = edges.length - 1;
      while (v < edges[i] - EPS) i -= 1;
      add(label(i), c.kept);
    }
  }
  const order = ['none', 'below', ...edges.map((_, i) => label(i))];
  return order
    .filter((k) => rows.has(k))
    .map((k) => {
      const r = rows.get(k);
      return { ...r, rate: round4(r.kept / r.n), ci: wilson(r.kept, r.n) };
    });
}

// ── 3. Fusion sweep ─────────────────────────────────────────────────────

/** One run's pool re-fused at `setting`: `positions[i]` is candidate i's new
 *  1-based place (a passage that leaves the pool shares the last place). */
export function fuse(candidates, { lexicalWeight, semanticWeight, floor }, k = RRF_K) {
  const lexical = candidates
    .map((c, i) => ({ c, i }))
    .filter(({ c }) => c.localScore > 0)
    .sort((a, b) => b.c.localScore - a.c.localScore || a.c.rank - b.c.rank);
  const lexRank = new Map(lexical.map(({ i }, n) => [i, n + 1]));
  const passes = (s) => s != null && s >= floor - EPS;
  const semValues = [...new Set(candidates.map((c) => c.semanticScore).filter(passes))].sort((a, b) => b - a);
  const semRank = new Map(semValues.map((s, n) => [s, n + 1]));
  const entries = candidates
    .map((c, i) => ({ c, i, l: lexRank.get(i) ?? null, s: passes(c.semanticScore) ? semRank.get(c.semanticScore) : null }))
    .filter((e) => e.l != null || e.s != null);
  const fused = (e) => (e.l == null ? 0 : lexicalWeight / (k + e.l)) + (e.s == null ? 0 : semanticWeight / (k + e.s));
  const BIG = Number.MAX_SAFE_INTEGER;
  entries.sort(
    (a, b) =>
      fused(b) - fused(a) ||
      (a.l == null) - (b.l == null) ||
      (a.l ?? BIG) - (b.l ?? BIG) ||
      (a.s ?? BIG) - (b.s ?? BIG) ||
      a.c.rank - b.c.rank,
  );
  const positions = new Array(candidates.length).fill(entries.length + 1);
  entries.forEach((e, n) => {
    positions[e.i] = n + 1;
  });
  return positions;
}

const distance = (s) =>
  Math.abs(s.lexicalWeight - CURRENT.lexicalWeight) + Math.abs(s.semanticWeight - CURRENT.semanticWeight) + Math.abs(s.floor - CURRENT.floor) * 10;
/** Better first: mean per-run AUC, then pooled, then nearest the current. */
const better = (a, b) => b.meanPerRun - a.meanPerRun || b.pooled - a.pooled || distance(a) - distance(b);

/** Every setting's AUC of the re-fused position against `kept` (pooled and
 *  mean per run), the current setting, the best by mean per-run AUC (ties to
 *  the setting nearest the current), and how closely the rebuild at the
 *  current setting reproduces the recorded rank. */
export function sweep(runs, { weights = WEIGHTS, floors = FLOORS } = {}) {
  const evaluate = (setting) => {
    const placed = runs.map((r) => ({ r, pos: fuse(r.candidates, setting) }));
    const s = summarize(
      placed.map(({ r, pos }) => ({ ...r, candidates: r.candidates.map((c, i) => ({ ...c, position: pos[i] })) })),
      { position: (c) => -c.position },
    );
    return {
      ...setting,
      identifiable: setting.floor >= RECORDED_FLOOR - EPS,
      pooled: s.scores.position.pooled,
      meanPerRun: s.scores.position.meanPerRun,
    };
  };
  const grid = [];
  for (const floor of floors) for (const lexicalWeight of weights) for (const semanticWeight of weights) grid.push(evaluate({ lexicalWeight, semanticWeight, floor }));
  const current = evaluate({ ...CURRENT });
  const best = grid
    .filter((g) => g.identifiable && g.meanPerRun != null)
    .sort(better)[0] ?? null;
  let same = 0;
  let total = 0;
  let moved = 0;
  for (const r of runs) {
    const pos = fuse(r.candidates, CURRENT);
    r.candidates.forEach((c, i) => {
      total += 1;
      if (pos[i] === c.rank) same += 1;
      moved += Math.abs(pos[i] - c.rank);
    });
  }
  const recorded = summarize(runs, { rank: SCORE_OF.rank }).scores.rank;
  return {
    grid,
    current,
    best,
    fidelity: {
      candidates: total,
      sameRank: same,
      rate: round4(total ? same / total : null),
      meanShift: total ? Math.round((moved / total) * 100) / 100 : null,
      recorded: { pooled: recorded.pooled, meanPerRun: recorded.meanPerRun },
    },
  };
}

// ── 4. Agent labels ─────────────────────────────────────────────────────

export const AGENT_NOTE =
  'Agent labels (S26) come from what the agent did: `cited` is a kept passage its final reply named by path or file name (4+ characters; a generic or date-shaped name only with its `.md`), need-recall is how many notes it opened outside its folder retrieval had found, over runs where retrieval ran.';

/** A manifest's agent labels (S26), or null when it has none. `cited`: the
 *  kept candidates (numbers only, labelled `kept` = cited, the label
 *  `summarize` reads) of a retrieval whose reply was checked
 *  (`retrieval.citedChecked`), else null. `need`: of the notes the agent
 *  opened outside its folder (`agentReads.notesOpened`), how many retrieval
 *  had among its candidates and how many it kept, only when retrieval ran
 *  (it has candidates and its `queryBasis` isn't `skipped-…`), else null.
 *  `unretrieved`: how many notes it opened when retrieval didn't run (0 when
 *  it did). Paths are compared here and never returned. */
export function agentRun(manifest) {
  const r = manifest?.retrieval;
  const candidates = (Array.isArray(r?.candidates) ? r.candidates : []).filter((c) => c && typeof c === 'object');
  const cited =
    r?.citedChecked === true
      ? candidates
          .filter((c) => c.kept === true && Number.isFinite(c.rank))
          .map((c) => ({
            rank: c.rank,
            localScore: Number.isFinite(c.localScore) ? c.localScore : 0,
            semanticScore: Number.isFinite(c.semanticScore) ? c.semanticScore : null,
            kept: c.cited === true,
          }))
          .sort((a, b) => a.rank - b.rank)
      : null;
  const opened = Array.isArray(manifest?.agentReads?.notesOpened)
    ? [...new Set(manifest.agentReads.notesOpened.filter((p) => typeof p === 'string'))]
    : [];
  const pool = new Set(candidates.map((c) => c.path));
  const kept = new Set(candidates.filter((c) => c.kept === true).map((c) => c.path));
  const skipped = typeof r?.queryBasis === 'string' && r.queryBasis.startsWith('skipped');
  const ran = candidates.length > 0 && !skipped;
  const need =
    ran && opened.length
      ? { opened: opened.length, candidates: opened.filter((p) => pool.has(p)).length, kept: opened.filter((p) => kept.has(p)).length }
      : null;
  const unretrieved = ran ? 0 : opened.length;
  if (!cited && !need && !unretrieved) return null;
  return { runId: typeof manifest.runId === 'string' ? manifest.runId : null, cited, need, unretrieved };
}

/** The agent label set over `agentRun`s: the citation rate, each score's AUC
 *  against `cited` among the kept (`summarize`: a run that cited none, or
 *  all, has no per-run AUC and is counted), and need-recall. */
export function agentLabels(agentRuns) {
  const checked = agentRuns.filter((r) => r.cited?.length);
  const keptN = checked.reduce((n, r) => n + r.cited.length, 0);
  const citedN = checked.reduce((n, r) => n + r.cited.filter((c) => c.kept).length, 0);
  const need = agentRuns.filter((r) => r.need);
  const sum = (key) => need.reduce((n, r) => n + r.need[key], 0);
  const opened = sum('opened');
  const among = sum('candidates');
  const amongKept = sum('kept');
  const unretrieved = agentRuns.filter((r) => r.unretrieved);
  return {
    note: AGENT_NOTE,
    citation: {
      runs: checked.length,
      kept: keptN,
      cited: citedN,
      rate: round4(keptN ? citedN / keptN : null),
      ci: wilson(citedN, keptN),
    },
    auc: summarize(checked.map((r) => ({ runId: r.runId, candidates: r.cited }))),
    needRecall: {
      runs: need.length,
      opened,
      candidates: among,
      kept: amongKept,
      rate: round4(opened ? among / opened : null),
      ci: wilson(among, opened),
      keptRate: round4(opened ? amongKept / opened : null),
      // Notes opened in runs where retrieval didn't run (no candidates, or
      // a skipped query): not need-recall's misses, counted apart.
      noRetrieval: { runs: unretrieved.length, opened: unretrieved.reduce((n, r) => n + r.unretrieved, 0) },
    },
  };
}

// ── Report ──────────────────────────────────────────────────────────────

/** The whole report for a folder of manifests. */
export function report(dir) {
  const { runs, counts, agent } = loadRuns(dir);
  const candidates = runs.flatMap((r) => r.candidates);
  return {
    kind: 'hollow-retrieval-labels',
    note: PROXY_NOTE,
    current: { ...CURRENT, rrfK: RRF_K },
    inputs: counts,
    auc: summarize(runs),
    bands: {
      semanticScore: bands(candidates, 'semanticScore', SEMANTIC_BANDS),
      localScore: bands(candidates, 'localScore', LOCAL_BINS, { decimals: 0 }),
    },
    sweep: sweep(runs),
    agent: agentLabels(agent),
  };
}

const cell = (v) => (v == null ? '–' : String(v));
const SCORE_NAMES = { rank: 'broker rank (lower better)', local: 'localScore', semantic: 'semanticScore (missing = 0)' };
const setting = (s) => `lexical ${s.lexicalWeight} · semantic ${s.semanticWeight} · floor ${s.floor.toFixed(2)}`;

/** The report as Markdown. */
export function render(r) {
  const a = r.auc;
  const i = r.inputs;
  const out = [
    '## Retrieval labels',
    '',
    `> ${r.note}`,
    '',
    `Manifests: ${i.files} (${i.labelled} Jev-labelled; skipped ${i.notJev} not labelled by Jev, ${i.noCandidates} without candidates, ${i.unreadable} unreadable). ` +
      `Candidates: ${a.candidates}, kept ${a.kept}${i.unlabeled ? `, unlabeled ${i.unlabeled} (not decided by Jev)` : ''}${i.malformed ? `, malformed ${i.malformed} (decided by Jev without a kept label or a rank)` : ''}. Runs with an AUC: ${a.usableRuns} of ${a.runs} (skipped: ${a.skipped.allKept} all kept, ${a.skipped.noneKept} none kept).`,
    '',
    '### AUC against `kept`',
    '',
    '| score | pooled | mean per run | median per run |',
    '|---|---|---|---|',
  ];
  for (const n of SCORES) out.push(`| ${SCORE_NAMES[n]} | ${cell(a.scores[n].pooled)} | ${cell(a.scores[n].meanPerRun)} | ${cell(a.scores[n].medianPerRun)} |`);
  for (const [field, rows] of Object.entries(r.bands)) {
    out.push('', `### Keep rate by ${field}`, '', '| band | n | kept | rate | 95% CI |', '|---|---|---|---|---|');
    for (const b of rows) out.push(`| ${b.band} | ${b.n} | ${b.kept} | ${b.rate} | ${b.ci ? `${b.ci.lo}–${b.ci.hi}` : '–'} |`);
  }
  const s = r.sweep;
  out.push(
    '',
    '### Fusion sweep',
    '',
    `The rebuild at the current setting puts ${s.fidelity.sameRank} of ${s.fidelity.candidates} candidates (${cell(s.fidelity.rate)}) at their recorded rank, ` +
      `${cell(s.fidelity.meanShift)} places off on average; its AUC is ${cell(s.current.meanPerRun)} per run (${cell(s.current.pooled)} pooled) against the recorded rank's ` +
      `${cell(s.fidelity.recorded.meanPerRun)} (${cell(s.fidelity.recorded.pooled)}). Compare settings with the rebuilt current one. ` +
      `Floors under ${RECORDED_FLOOR} equal ${RECORDED_FLOOR} here (no lower hit is recorded); only the weights' ratio matters.`,
    '',
    `- Current (${setting(s.current)}): mean per run ${cell(s.current.meanPerRun)}, pooled ${cell(s.current.pooled)}`,
    s.best
      ? `- Best (${setting(s.best)}): mean per run ${cell(s.best.meanPerRun)}, pooled ${cell(s.best.pooled)}`
      : '- Best: none (no run has an AUC)',
    '',
    '| floor | best weights (lexical · semantic) | mean per run | pooled | equal weights mean per run |',
    '|---|---|---|---|---|',
  );
  for (const floor of [...new Set(s.grid.map((g) => g.floor))]) {
    const at = s.grid.filter((g) => g.floor === floor && g.meanPerRun != null);
    const top = [...at].sort(better)[0];
    const equal = at.find((g) => g.lexicalWeight === 1 && g.semanticWeight === 1);
    const tag = floor < RECORDED_FLOOR - EPS ? ' (= 0.60)' : '';
    out.push(`| ${floor.toFixed(2)}${tag} | ${top ? `${top.lexicalWeight} · ${top.semanticWeight}` : '–'} | ${cell(top?.meanPerRun)} | ${cell(top?.pooled)} | ${cell(equal?.meanPerRun)} |`);
  }
  out.push(...renderAgent(r.agent));
  out.push('');
  return out.join('\n');
}

const ci = (c) => (c ? `${c.lo}–${c.hi}` : '–');

/** The agent label set's section of `render`. */
function renderAgent(g) {
  const out = ['', '### Agent labels', '', `> ${g.note}`, ''];
  const c = g.citation;
  const n = g.needRecall;
  if (!c.runs && !n.runs && !n.noRetrieval.runs) {
    out.push('No run carries agent labels (manifests written before 2026-10-17 have none).');
    return out;
  }
  out.push(
    `Citation: ${c.cited} of ${c.kept} kept passages named in the reply (${cell(c.rate)}, 95% CI ${ci(c.ci)}), over ${c.runs} runs whose reply was checked. ` +
      `Runs with an AUC: ${g.auc.usableRuns} of ${g.auc.runs} (skipped: ${g.auc.skipped.noneKept} cited none, ${g.auc.skipped.allKept} cited all).`,
    '',
    '| score (kept passages) | AUC against `cited`, pooled | mean per run | median per run |',
    '|---|---|---|---|',
  );
  for (const s of SCORES) out.push(`| ${SCORE_NAMES[s]} | ${cell(g.auc.scores[s].pooled)} | ${cell(g.auc.scores[s].meanPerRun)} | ${cell(g.auc.scores[s].medianPerRun)} |`);
  out.push(
    '',
    `Need-recall: of ${n.opened} notes the agent opened outside its folder (${n.runs} runs where retrieval ran), retrieval had ${n.candidates} among its candidates (${cell(n.rate)}, 95% CI ${ci(n.ci)}) and kept ${n.kept} (${cell(n.keptRate)}). ` +
      `Not counted: ${n.noRetrieval.opened} notes opened in ${n.noRetrieval.runs} runs where retrieval didn't run (no candidates, or a skipped query).`,
  );
  return out;
}

function main(argv) {
  const dirs = [];
  let json = null;
  for (let n = 0; n < argv.length; n += 1) {
    if (argv[n] === '--json') json = argv[++n];
    else dirs.push(argv[n]);
  }
  if (dirs.length !== 1 || (argv.includes('--json') && !json)) {
    console.error('usage: node scripts/agent-eval/retrieval-labels.mjs <manifests-dir> [--json <file>]');
    return 2;
  }
  if (dirs[0].startsWith('--')) {
    console.error('usage: node scripts/agent-eval/retrieval-labels.mjs <manifests-dir> [--json <file>]');
    return 2;
  }
  let result;
  try {
    result = report(dirs[0]);
  } catch (e) {
    console.error(`cannot read ${dirs[0]}: ${e.code ?? e.message}`);
    return 2;
  }
  const need = result.agent.needRecall;
  if (!result.inputs.labelled && !result.agent.citation.runs && !need.runs && !need.noRetrieval.runs) {
    console.error(`no Jev-labelled or agent-labelled runs found (${result.inputs.files} manifest(s) read)`);
    return 1;
  }
  if (json) {
    try {
      writeFileSync(json, `${JSON.stringify(result, null, 2)}\n`);
    } catch (e) {
      console.error(`cannot write ${json}: ${e.code ?? e.message}`);
      return 2;
    }
  }
  console.log(render(result));
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
