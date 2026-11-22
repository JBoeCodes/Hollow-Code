// The preparation baseline's report (agent effectiveness F8, wave 3 · lane
// O, PR O1: measure first). It reads preparation timelines and reports
// p50/p95 per phase and in total, per group:
//
//   node scripts/agent-eval/prep-bench.mjs <path>... [--json <file>]
//
// Each <path> is a file or a folder of `.json` files of either kind:
// - the deterministic bench's output (`src-tauri/src/prep_bench.rs`, kind
//   `hollow-prep-bench`, one file per Jev mode: off, score, timeout,
//   malformed), grouped by mode;
// - run manifests (a vault's `.evidence-manifests/<runId>.json`, or a
//   `run_context_manifest` answer, a RunContextView), grouped as
//   `manifests`; a manifest without `preparation.timeline` (from before
//   wave 3) is counted and skipped.
//
// "total" is the timeline's `totalMs` (admission's start → the agent's
// spawn) when it has one, else the end of the last preparation step that
// ran (the bench spawns no agent). The first-output steps never count
// toward it. "stage" (O2) is retrieval and connector selection together,
// the first one's start to the last one's end: what the preparation
// deadline bounds. Percentiles are nearest-rank over the samples where the
// step ran; `ran` says in how many. The deadline line counts what
// `preparation.deadline.cut` lists. This file only reads files.
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

/** The steps, in `prep_timing::Step` order (src-tauri/src/prep_timing.rs). */
export const STEPS = [
  'admission',
  'recall',
  'attachedSources',
  'retrieval',
  'retrievalSemantic',
  'retrievalJev',
  'connectors',
  'microsoftAuth',
  'connectorsJev',
  'contextCompile',
  'promptAssembly',
  'repoSnapshot',
  'command',
  'codexInventory',
  'spawn',
  'firstStdout',
  'firstText',
];

/** Steps measured from the spawn, not preparation. */
const AFTER_SPAWN = new Set(['firstStdout', 'firstText']);

/** Nearest-rank percentile `p` (0–100) of `values`; null when empty. */
export function percentile(values, p) {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (!sorted.length) return null;
  const rank = Math.max(1, Math.ceil((p / 100) * sorted.length));
  return sorted[Math.min(rank, sorted.length) - 1];
}

/** A timeline's total: `totalMs`, else the end of its last preparation step. */
export function totalOf(timeline) {
  if (Number.isFinite(timeline?.totalMs)) return timeline.totalMs;
  const ends = (timeline?.phases ?? [])
    .filter((p) => !AFTER_SPAWN.has(p.name) && Number.isFinite(p.startMs) && Number.isFinite(p.durationMs))
    .map((p) => p.startMs + p.durationMs);
  return ends.length ? Math.max(...ends) : null;
}

/** Retrieval and connector selection together (O2: prepared at once):
 *  the first one's start to the last one's end; null when neither ran.
 *  The span covers only retrieval and connectors: since S29 the repo
 *  snapshot runs beside them, and time it adds when it is the slower
 *  branch shows only in `total`. */
export function stageOf(timeline) {
  const spans = ['retrieval', 'connectors']
    .map((name) => (timeline?.phases ?? []).find((p) => p.name === name))
    .filter((p) => p && Number.isFinite(p.startMs) && Number.isFinite(p.durationMs));
  if (!spans.length) return null;
  return Math.max(...spans.map((p) => p.startMs + p.durationMs)) - Math.min(...spans.map((p) => p.startMs));
}

/** Per-step, stage and total p50/p95 over `timelines`. */
export function summarize(timelines) {
  const steps = STEPS.map((name) => {
    const durations = timelines
      .map((t) => (t.phases ?? []).find((p) => p.name === name))
      .filter((p) => p && Number.isFinite(p.durationMs))
      .map((p) => p.durationMs);
    return { name, ran: durations.length, p50: percentile(durations, 50), p95: percentile(durations, 95), max: durations.length ? Math.max(...durations) : null };
  });
  const totals = timelines.map(totalOf).filter((v) => v != null);
  const stages = timelines.map(stageOf).filter((v) => v != null);
  return {
    samples: timelines.length,
    steps,
    stage: { ran: stages.length, p50: percentile(stages, 50), p95: percentile(stages, 95), max: stages.length ? Math.max(...stages) : null },
    total: { p50: percentile(totals, 50), p95: percentile(totals, 95), max: totals.length ? Math.max(...totals) : null },
  };
}

/** The manifest a parsed file holds (a manifest or a manifest view). */
function manifestOf(value) {
  const manifest = value?.manifest ?? value;
  if (manifest?.schemaVersion == null || !('preparation' in manifest)) return undefined;
  return manifest;
}

/** Group every input: bench files by mode, manifests as `manifests`. */
export function loadInputs(paths) {
  const groups = new Map();
  const skipped = [];
  const add = (group, timeline, extra) => {
    if (!groups.has(group)) groups.set(group, { timelines: [], jev: [], sizes: [], deadlines: [], inventories: [] });
    const g = groups.get(group);
    g.timelines.push(timeline);
    if (extra?.jev) g.jev.push(extra.jev);
    if (extra?.sizes) g.sizes.push(extra.sizes);
    if (extra?.deadline) g.deadlines.push(extra.deadline);
    if (extra?.codexInventory) g.inventories.push(extra.codexInventory);
  };
  const files = [];
  for (const raw of paths) {
    const path = resolve(raw);
    if (!existsSync(path)) throw new Error(`${raw} does not exist`);
    if (statSync(path).isDirectory()) {
      for (const name of readdirSync(path).sort()) if (name.endsWith('.json')) files.push(join(path, name));
    } else files.push(path);
  }
  for (const file of files) {
    let value;
    try {
      value = JSON.parse(readFileSync(file, 'utf8'));
    } catch {
      skipped.push({ file, reason: 'not JSON' });
      continue;
    }
    if (value?.kind === 'hollow-prep-bench') {
      for (const sample of value.samples ?? []) add(value.mode, sample.timeline, sample);
      continue;
    }
    const manifest = manifestOf(value);
    const timeline = manifest === undefined ? undefined : (manifest.preparation?.timeline ?? null);
    if (timeline === undefined) skipped.push({ file, reason: 'neither a bench file nor a run manifest' });
    else if (timeline === null) skipped.push({ file, reason: 'a manifest from before the preparation timeline (wave 3)' });
    else add('manifests', timeline, { jev: manifest.jev, deadline: manifest.preparation?.deadline, codexInventory: manifest.preparation?.codexInventory });
  }
  return { groups, skipped };
}

/** Jev outcomes per use across a group's samples: `connectors: ok×20`. */
function outcomes(jevs) {
  const counts = new Map();
  for (const uses of jevs) for (const u of uses) counts.set(`${u.use} ${u.outcome}`, (counts.get(`${u.use} ${u.outcome}`) ?? 0) + 1);
  return [...counts].map(([k, n]) => `${k}×${n}`).join(', ') || 'none';
}

/** What the preparation deadline did across a group's samples (O2):
 *  `budget 3000 ms; cut connectorsJev×20, retrievalJev×20`. */
function deadlines(records) {
  const budgets = [...new Set(records.map((d) => d?.budgetMs).filter(Number.isFinite))];
  const counts = new Map();
  for (const d of records) for (const step of d?.cut ?? []) counts.set(step, (counts.get(step) ?? 0) + 1);
  const cut = [...counts].sort(([a], [b]) => (a < b ? -1 : 1)).map(([k, n]) => `${k}×${n}`).join(', ');
  return `budget ${budgets.join('/') || '—'} ms; ${cut ? `cut ${cut}` : 'nothing cut'}`;
}

/** How Codex's native inventory was served across a group's Codex runs
 *  (O3): `listed×1, cached×19`. */
function inventories(served) {
  const counts = new Map();
  for (const how of served) counts.set(how, (counts.get(how) ?? 0) + 1);
  return [...counts].sort(([a], [b]) => (a === 'listed' ? -1 : b === 'listed' ? 1 : a < b ? -1 : 1)).map(([k, n]) => `${k}×${n}`).join(', ');
}

const cell = (v) => (v == null ? '—' : String(v));

/** The report: one table per group. */
export function render(report) {
  const out = [];
  for (const [group, g] of Object.entries(report.groups)) {
    out.push(`### ${group} (${g.summary.samples} samples)`, '');
    if (g.jev) out.push(`Jev: ${g.jev}`, '');
    if (g.deadline) out.push(`Deadline: ${g.deadline}`, '');
    if (g.codexInventory) out.push(`Codex inventory: ${g.codexInventory}`, '');
    if (g.sizes) out.push(`Sizes (p50): ${Object.entries(g.sizes).map(([k, v]) => `${k} ${v}`).join(', ')}`, '');
    out.push('| step | ran | p50 ms | p95 ms | max ms |', '|---|---|---|---|---|');
    for (const s of g.summary.steps) out.push(`| ${s.name} | ${s.ran} | ${cell(s.p50)} | ${cell(s.p95)} | ${cell(s.max)} |`);
    const st = g.summary.stage;
    if (st?.ran) out.push(`| **stage** (retrieval ∪ connectors) | ${st.ran} | ${cell(st.p50)} | ${cell(st.p95)} | ${cell(st.max)} |`);
    const t = g.summary.total;
    out.push(`| **total** | ${g.summary.samples} | ${cell(t.p50)} | ${cell(t.p95)} | ${cell(t.max)} |`, '');
  }
  if (report.skipped.length) out.push(`Skipped: ${report.skipped.map((s) => `${s.file} (${s.reason})`).join('; ')}`, '');
  return out.join('\n');
}

/** Summaries for every group in `paths`. */
export function report(paths) {
  const { groups, skipped } = loadInputs(paths);
  const out = { groups: {}, skipped };
  for (const [group, g] of groups) {
    const sizes = {};
    for (const key of Object.keys(g.sizes[0] ?? {})) sizes[key] = percentile(g.sizes.map((s) => s[key]), 50);
    out.groups[group] = {
      summary: summarize(g.timelines),
      jev: g.jev.length ? outcomes(g.jev) : null,
      deadline: g.deadlines.length ? deadlines(g.deadlines) : null,
      codexInventory: g.inventories.length ? inventories(g.inventories) : null,
      sizes: g.sizes.length ? sizes : null,
    };
  }
  return out;
}

function main(argv) {
  const paths = [];
  let json = null;
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--json') json = argv[++i];
    else paths.push(argv[i]);
  }
  if (!paths.length) {
    console.error('usage: node scripts/agent-eval/prep-bench.mjs <bench-dir|manifest-dir|file>... [--json <file>]');
    return 2;
  }
  const result = report(paths);
  if (!Object.keys(result.groups).length) {
    console.error(`no timelines found (${result.skipped.length} file(s) skipped)`);
    return 1;
  }
  console.log(render(result));
  if (json) writeFileSync(json, `${JSON.stringify(result, null, 2)}\n`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) process.exit(main(process.argv.slice(2)));
