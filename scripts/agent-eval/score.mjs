// The agent effectiveness pilot scorer (E1). It reads a pilot round's
// folders (results/pilot/<round>/, written by the retired test bed's `pilot`
// command) and scores each run on the artifact its task
// asked for, never on what the agent said. docs/history/AGENT_EFFECTIVENESS.md
// "Evaluation harness".
//
//   node scripts/agent-eval/score.mjs score <roundDir> [--tasks <file>] [--out <dir>]
//        → <out>/score.json and score.md (default out: the round folder)
//   node scripts/agent-eval/score.mjs compare <roundDirA> <roundDirB> [--tasks <file>] [--out <dir>]
//        → <out>/compare-<A>-<B>.json and .md (default out: round B's folder)
//
// Per run it reads run.json (status, queue times, tokens, cache reads and
// cost from the candidate's usage ledger), artifacts/<vault path> and
// context-manifest.json when the candidate had one (F11's RunContextView,
// schema 1). Anything unknown stays null: a missing manifest is "absent"
// with its reason, an unknown cost makes the total unknown, never 0.
// Pure file reads: no model, Jev, bed or network.
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

/** The plan's slices ("Evaluation"), as the suite labels them. */
export const SLICES = ['simple', 'research', 'continuity', 'tools', 'memory', 'vault-search'];
export const RULE_KINDS = ['contains', 'absent', 'cites', 'firstLine', 'inOrder', 'historyLabelled'];
/** The manifest schema this scorer reads (run_manifest::SCHEMA_VERSION). */
export const MANIFEST_SCHEMA = 1;
export const DEFAULT_TASKS = fileURLToPath(new URL('./pilot-tasks.json', import.meta.url));
const TASK_ID = /^[a-z0-9][a-z0-9-]{0,47}$/;
const WORKSPACE_ID = /^[A-Za-z0-9_-][A-Za-z0-9._-]{0,127}$/;
const ARTIFACT = /^outputs\/[A-Za-z0-9._-]+(\/[A-Za-z0-9._-]+)*$/;
const Z95 = 1.959963984540054;

// ── The suite ────────────────────────────────────────────────────────────

const regex = (source, flags, at) => {
  try { return new RegExp(source, flags); } catch (error) { throw new Error(`${at}: pattern ${JSON.stringify(source)} is not a regular expression (${error.message})`); }
};
const strings = (v) => Array.isArray(v) && v.length > 0 && v.every((s) => typeof s === 'string' && s);

function validateRule(rule, at) {
  if (!rule || typeof rule !== 'object') throw new Error(`${at}: a rule must be an object`);
  if (typeof rule.id !== 'string' || !TASK_ID.test(rule.id)) throw new Error(`${at}: rule id must be lower-case letters, digits and -`);
  const where = `${at} rule ${rule.id}`;
  if (!RULE_KINDS.includes(rule.kind)) throw new Error(`${where}: kind must be one of ${RULE_KINDS.join(', ')}`);
  const flags = rule.flags ?? 'i';
  if (typeof flags !== 'string' || !/^[imsu]*$/.test(flags)) throw new Error(`${where}: flags may hold only i, m, s and u`);
  switch (rule.kind) {
    case 'contains':
    case 'absent':
      if (rule.any == null && rule.pattern == null) throw new Error(`${where}: needs any or a pattern`);
      if (rule.any != null && !strings(rule.any)) throw new Error(`${where}: any must be a list of strings`);
      if (rule.pattern != null) regex(rule.pattern, flags, where);
      break;
    case 'cites':
      if (typeof rule.path !== 'string' || !rule.path || rule.path.startsWith('/') || rule.path.split('/').includes('..')) throw new Error(`${where}: path must be a vault-relative path`);
      if (!['basename', 'vault-path'].includes(rule.form)) throw new Error(`${where}: form must be basename or vault-path`);
      break;
    case 'firstLine':
      if (typeof rule.pattern !== 'string') throw new Error(`${where}: needs a pattern`);
      regex(rule.pattern, flags, where);
      break;
    case 'inOrder':
      if (!strings(rule.items) || rule.items.length < 2) throw new Error(`${where}: items must list at least two strings`);
      break;
    case 'historyLabelled':
      if (!strings(rule.stale) || !strings(rule.markers)) throw new Error(`${where}: needs stale and markers lists`);
      break;
    default:
  }
  return { ...rule, flags };
}

/** One task's `score` block: the artifact (workspace-relative, under
 *  outputs/) and its rules. The message must name the artifact, so the
 *  agent is asked for exactly the file that is scored. */
export function validateScore(score, prompt, id) {
  if (!score || typeof score !== 'object') throw new Error(`${id}: score must be an object`);
  if (typeof score.artifact !== 'string' || !ARTIFACT.test(score.artifact) || score.artifact.split('/').includes('..')) {
    throw new Error(`${id}: score.artifact must be a file under outputs/`);
  }
  if (typeof prompt === 'string' && !prompt.includes(score.artifact)) throw new Error(`${id}: the message never names ${score.artifact}, so the agent isn't asked for it`);
  if (!Array.isArray(score.rules) || !score.rules.length) throw new Error(`${id}: score.rules must list at least one rule`);
  const rules = score.rules.map((r, i) => validateRule(r, `${id} rule ${i + 1}`));
  const ids = new Set();
  for (const r of rules) {
    if (ids.has(r.id)) throw new Error(`${id}: rule ${r.id} appears twice`);
    ids.add(r.id);
  }
  return { artifact: score.artifact, rules };
}

/** The suite file (`{ tasks: [...] }`): every task labelled, with its
 *  artifact, rules and judged evidence. Throws on anything a scorer could
 *  not judge. */
export function validateSuite(input) {
  const list = Array.isArray(input) ? input : input?.tasks;
  if (!Array.isArray(list) || !list.length) throw new Error('the suite needs a non-empty tasks array');
  const seen = new Set();
  return list.map((raw, i) => {
    const at = `task ${i + 1}`;
    if (!raw || typeof raw !== 'object') throw new Error(`${at} is not an object`);
    if (typeof raw.id !== 'string' || !TASK_ID.test(raw.id)) throw new Error(`${at}: id must be lower-case letters, digits and -`);
    if (seen.has(raw.id)) throw new Error(`${at}: id ${raw.id} appears twice`);
    seen.add(raw.id);
    if (!SLICES.includes(raw.slice)) throw new Error(`${raw.id}: slice must be one of ${SLICES.join(', ')}`);
    if (typeof raw.facet !== 'string' || !raw.facet) throw new Error(`${raw.id}: facet must say what the task probes`);
    const workspace = raw.workspace ?? 'eval-outcome';
    if (!WORKSPACE_ID.test(workspace)) throw new Error(`${raw.id}: workspace must be a workspace id`);
    if (typeof raw.prompt !== 'string' || !raw.prompt.trim()) throw new Error(`${raw.id}: prompt must be the message to send`);
    const score = validateScore(raw.score, raw.prompt, raw.id);
    const j = raw.judged;
    if (!j || typeof j.answer !== 'string' || !j.answer) throw new Error(`${raw.id}: judged.answer must be a reference answer`);
    if (!Array.isArray(j.evidence) || !j.evidence.length || !j.evidence.every((e) => typeof e?.path === 'string' && typeof e?.quote === 'string' && e.quote)) {
      throw new Error(`${raw.id}: judged.evidence must list { path, quote } from the fixture`);
    }
    if (!Array.isArray(j.fails) || !j.fails.every((f) => typeof f?.text === 'string' && typeof f?.because === 'string')) {
      throw new Error(`${raw.id}: judged.fails must list { text, because }`);
    }
    return { id: raw.id, slice: raw.slice, facet: raw.facet, workspace, prompt: raw.prompt, timeoutMin: raw.timeoutMin ?? 20, score, judged: j };
  });
}

/** The suite at `file`, validated, with its hash. */
export function loadSuite(file = DEFAULT_TASKS) {
  const body = readFileSync(file);
  return { file: resolve(file), sha256: createHash('sha256').update(body).digest('hex'), tasks: validateSuite(JSON.parse(body.toString('utf8'))) };
}

/** What a round sent: each task's id, workspace and exact message (before
 *  `{vault}` is filled in), order-free. The pilot records it in plan.json,
 *  so a comparison can tell whether two rounds asked the same things; rules
 *  and labels can be corrected without changing it. */
export function taskDigest(tasks) {
  const rows = tasks.map((t) => ({ id: t.id, workspace: t.workspace ?? 'eval-outcome', prompt: t.prompt })).sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return createHash('sha256').update(JSON.stringify(rows)).digest('hex');
}

// ── Rules ────────────────────────────────────────────────────────────────

/** Text as a rule reads it: NFKC, no-break and thin spaces as spaces, curly
 *  quotes straight, markdown emphasis and code ticks dropped. Lines kept. */
export function normalizeText(text) {
  return String(text)
    .normalize('NFKC')
    .replace(/[    ]/g, ' ')
    .replace(/[‘’]/g, "'")
    .replace(/[“”]/g, '"')
    .replace(/[*`]/g, '')
    .replace(/\r\n?/g, '\n');
}
const flat = (text) => text.replace(/[ \t]+/g, ' ').toLowerCase();
const found = (hay, s) => hay.includes(flat(normalizeText(s)));
const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** One rule over one artifact's text: `{ pass, detail }`. */
export function checkRule(rule, raw) {
  const text = normalizeText(raw);
  const hay = flat(text);
  const flags = rule.flags ?? 'i';
  const matchAny = () => {
    const hits = (rule.any ?? []).filter((s) => found(hay, s));
    const re = rule.pattern != null ? regex(rule.pattern, flags, rule.id) : null;
    const m = re ? text.replace(/[ \t]+/g, ' ').match(re) : null;
    return { hits, m };
  };
  switch (rule.kind) {
    case 'contains': {
      const { hits, m } = matchAny();
      if (hits.length || m) return { pass: true, detail: `found ${JSON.stringify(hits[0] ?? m[0])}` };
      return { pass: false, detail: `none of ${[...(rule.any ?? []).map((s) => JSON.stringify(s)), ...(rule.pattern ? [`/${rule.pattern}/`] : [])].join(', ')}` };
    }
    case 'absent': {
      const { hits, m } = matchAny();
      if (!hits.length && !m) return { pass: true, detail: 'absent' };
      return { pass: false, detail: `found ${JSON.stringify(hits[0] ?? m[0])}` };
    }
    case 'cites': {
      const want = rule.form === 'basename' ? basename(rule.path) : rule.path;
      const ok = hay.replace(/\\/g, '/').includes(want.toLowerCase());
      return { pass: ok, detail: ok ? `cites ${want}` : `never names ${want}` };
    }
    case 'firstLine': {
      const first = text.split('\n').find((l) => l.trim()) ?? '';
      const ok = regex(rule.pattern, flags, rule.id).test(first.trim());
      return { pass: ok, detail: ok ? 'first line matches' : `first line is ${JSON.stringify(first.trim().slice(0, 80))}` };
    }
    case 'inOrder': {
      let from = 0;
      for (const item of rule.items) {
        // The cursor moves by the item as matched (normalised), not as
        // written: ticks, emphasis and runs of spaces are gone from `hay`.
        const want = flat(normalizeText(item));
        const at = hay.indexOf(want, from);
        if (at < 0) return { pass: false, detail: `${JSON.stringify(item)} missing or out of order` };
        from = at + want.length;
      }
      return { pass: true, detail: 'in order' };
    }
    case 'historyLabelled': {
      const sentences = text.split(/\n+|(?<=[.!?])\s+/).map((s) => s.trim()).filter(Boolean);
      const words = (s, list, plural = '') => list.some((w) => new RegExp(`\\b${escapeRe(w)}${plural}\\b`, 'i').test(s));
      // A stale value counts in the plural too ("Fridays" is the Friday
      // cadence); a marker must be said as written.
      const bare = sentences.filter((s) => words(s, rule.stale, '(?:s|es)?') && !words(s, rule.markers));
      if (!bare.length) return { pass: true, detail: 'stale values only as history' };
      return { pass: false, detail: `presents ${rule.stale.join('/')} as current: ${JSON.stringify(bare[0].slice(0, 100))}` };
    }
    default:
      throw new Error(`unknown rule kind ${rule.kind}`);
  }
}

/** The artifact at `file`: its text, or null with why there is none
 *  (`missing`, or `not-a-file` for a directory or anything else that isn't
 *  a regular file, which reading would throw on). */
export function readArtifact(file) {
  let st;
  try { st = statSync(file); } catch { return { text: null, missing: 'missing' }; }
  if (!st.isFile()) return { text: null, missing: 'not-a-file' };
  return { text: readFileSync(file, 'utf8'), missing: null };
}

/** A task's verdict on its artifact text (null: no artifact; `missing`
 *  says why, from [`readArtifact`]). Every rule must pass. `reasons` names
 *  each failing rule. */
export function scoreArtifact(task, text, { wrote = [], missing = 'missing' } = {}) {
  const artifact = task.score.artifact;
  if (text == null && missing === 'not-a-file') {
    return { pass: false, artifact, present: false, rules: [], reasons: [`${artifact} in workspaces/${task.workspace}/ is not a file (a folder or another kind of entry)`] };
  }
  if (text == null) {
    const elsewhere = wrote.length ? `; the run wrote ${wrote.join(', ')}` : '; the run wrote no file';
    return { pass: false, artifact, present: false, rules: [], reasons: [`no ${artifact} in workspaces/${task.workspace}/${elsewhere}`] };
  }
  if (!String(text).trim()) return { pass: false, artifact, present: true, rules: [], reasons: [`${artifact} is empty`] };
  const rules = task.score.rules.map((r) => ({ id: r.id, kind: r.kind, ...checkRule(r, text) }));
  const reasons = rules.filter((r) => !r.pass).map((r) => `${r.id}: ${r.detail}`);
  return { pass: reasons.length === 0, artifact, present: true, rules, reasons };
}

// ── Statistics ───────────────────────────────────────────────────────────

const round4 = (n) => Math.round(n * 1e4) / 1e4;

/** Wilson score interval for k of n (95%), or null with no runs. */
export function wilson(k, n, z = Z95) {
  if (!n) return null;
  const p = k / n;
  const z2 = z * z;
  const centre = (p + z2 / (2 * n)) / (1 + z2 / n);
  const half = (z * Math.sqrt((p * (1 - p)) / n + z2 / (4 * n * n))) / (1 + z2 / n);
  return { lo: k === 0 ? 0 : round4(Math.max(0, centre - half)), hi: k === n ? 1 : round4(Math.min(1, centre + half)) };
}

/** B's completion minus A's, with Newcombe's hybrid score interval (95%),
 *  built from the two Wilson intervals. Null when either has no runs. */
export function newcombe(kA, nA, kB, nB) {
  if (!nA || !nB) return null;
  const pA = kA / nA;
  const pB = kB / nB;
  const a = wilson(kA, nA);
  const b = wilson(kB, nB);
  const diff = pB - pA;
  return {
    diff: round4(diff),
    lo: round4(Math.max(-1, diff - Math.sqrt((pB - b.lo) ** 2 + (a.hi - pA) ** 2))),
    hi: round4(Math.min(1, diff + Math.sqrt((b.hi - pB) ** 2 + (pA - a.lo) ** 2))),
  };
}

/** Nearest-rank percentile (q in 0..1), or null with no values. With three
 *  repeats p95 is the slowest run: a diagnostic, not a certification. */
export function percentile(values, q) {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return null;
  return v[Math.max(0, Math.ceil(q * v.length) - 1)];
}

// ── One run ──────────────────────────────────────────────────────────────

const readJson = (file) => (existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : null);
const ms = (from, to) => {
  const a = Date.parse(from);
  const b = Date.parse(to);
  return Number.isFinite(a) && Number.isFinite(b) && b >= a ? b - a : null;
};
const num = (v) => (Number.isFinite(v) ? v : null);

/** How long the person waited: the queue row's request to finish when the
 *  pilot recorded them (E1 on), else the pilot's own send-to-done clock,
 *  which also holds its settle and ledger waits. Labelled, so rounds timed
 *  differently are never compared silently. */
function duration(run) {
  const q = ms(run.queue?.requestedAt, run.queue?.finishedAt);
  if (q != null) return { durationMs: q, durationSource: 'queue' };
  const w = ms(run.startedAt, run.endedAt);
  return { durationMs: w, durationSource: w == null ? null : 'pilot-wall' };
}

/** F11's manifest for this run, summarised: sizes, decisions and Jev
 *  outcomes. Absent, unreadable or another run's are said as such. */
export function summarizeManifest(view, run) {
  if (view == null) {
    const reason = run?.context?.reason ?? (run?.context ? 'the pilot kept no manifest' : 'no context-manifest.json: a candidate before F11 (or a pilot before E1) records none');
    return { status: 'absent', reason };
  }
  const m = view.manifest ?? view;
  if (m?.schemaVersion !== MANIFEST_SCHEMA) return { status: 'unreadable', reason: `manifest schema ${m?.schemaVersion ?? 'missing'}, this scorer reads schema ${MANIFEST_SCHEMA}` };
  const own = run?.transcript?.historyRunId;
  if (own && m.runId !== own) return { status: 'mismatch', reason: `the manifest is run ${m.runId}'s, not this run's (${own})` };
  const layers = Array.isArray(m.layers) ? m.layers : [];
  const candidates = m.retrieval?.candidates ?? [];
  const decisions = m.connectors?.decisions ?? [];
  return {
    status: 'present',
    schemaVersion: m.schemaVersion,
    runId: m.runId,
    layersBytes: layers.reduce((n, l) => n + (num(l.bytes) ?? 0), 0),
    layers: Object.fromEntries(layers.map((l) => [l.name, num(l.bytes)])),
    retrieval: m.retrieval ? { engine: m.retrieval.engine ?? null, considered: candidates.length, kept: candidates.filter((c) => c.kept).length } : null,
    connectors: m.connectors ? { engine: m.connectors.engine ?? null, considered: decisions.length, attached: decisions.filter((d) => d.attach).length, named: decisions.filter((d) => d.basis === 'named').length } : null,
    jev: (m.jev ?? []).map((j) => ({ use: j.use, outcome: j.outcome, items: num(j.items), latencyMs: num(j.latencyMs), costUsd: num(j.costUsd) })),
    fallbacks: (m.fallbacks ?? []).length,
    preparation: m.preparation ?? {},
  };
}

function tokensOf(cost) {
  const input = num(cost?.inputTokens);
  const cached = num(cost?.cachedInputTokens);
  const output = num(cost?.outputTokens);
  // The ledger's input excludes cache reads (usage.rs), so the share of the
  // prompt served from cache is cached / (input + cached).
  const cacheShare = input != null && cached != null && input + cached > 0 ? round4(cached / (input + cached)) : null;
  return { input, cached, output, cacheShare };
}

/** One run folder, scored: verdict, reasons, status, duration, tokens,
 *  cost and manifest. */
export function scoreRunDir(task, dir, run) {
  const rel = `workspaces/${task.workspace}/${task.score.artifact}`;
  const { text, missing } = readArtifact(join(dir, 'artifacts', rel));
  const wrote = [...(run.artifacts?.created ?? []), ...(run.artifacts?.modified ?? [])]
    .filter((p) => p !== rel)
    .map((p) => (p.startsWith(`workspaces/${task.workspace}/`) ? p.slice(`workspaces/${task.workspace}/`.length) : `${p} at the vault root`));
  const verdict = scoreArtifact(task, text, { wrote, missing });
  const status = run.status ?? null;
  const reasons = verdict.pass || status === 'done' ? verdict.reasons : [`run ended ${status}`, ...verdict.reasons];
  const view = readJson(join(dir, 'context-manifest.json'));
  return {
    key: run.key, task: task.id, slice: task.slice, rep: run.rep ?? null, status,
    pass: verdict.pass, reasons, rules: verdict.rules,
    ...duration(run),
    tokens: tokensOf(run.cost),
    costUsd: num(run.cost?.costUsd),
    manifest: summarizeManifest(view, run),
  };
}

// ── A round ──────────────────────────────────────────────────────────────

const keyParts = (key) => {
  const m = String(key).match(/^(.*)-(\d+)$/);
  return m ? { id: m[1], rep: Number(m[2]) } : null;
};
const mean = (xs) => (xs.length ? round4(xs.reduce((a, b) => a + b, 0) / xs.length) : null);
/** Sum, or null when any value is unknown; null with nothing to sum. */
const sumKnown = (xs) => (xs.length && xs.every((x) => x != null) ? round4(xs.reduce((a, b) => a + b, 0)) : null);

function costTotals(runs) {
  const known = runs.filter((r) => r.costUsd != null);
  const unknownRuns = runs.length - known.length;
  const knownUsd = round4(known.reduce((n, r) => n + r.costUsd, 0));
  return { totalUsd: runs.length && !unknownRuns ? knownUsd : null, knownUsd, unknownRuns };
}

function durationTotals(runs) {
  const values = runs.map((r) => r.durationMs).filter((v) => v != null);
  return {
    p50Ms: percentile(values, 0.5),
    p95Ms: percentile(values, 0.95),
    sources: [...new Set(runs.map((r) => r.durationSource).filter(Boolean))].sort(),
  };
}

function completion(runs) {
  const passed = runs.filter((r) => r.pass).length;
  return { runs: runs.length, passed, completion: runs.length ? round4(passed / runs.length) : null, ci: wilson(passed, runs.length) };
}

function manifestTotals(runs) {
  const counts = { present: 0, absent: 0, unreadable: 0, mismatch: 0 };
  for (const r of runs) counts[r.manifest.status] += 1;
  return counts;
}

function manifestAggregate(runs) {
  const present = runs.map((r) => r.manifest).filter((m) => m.status === 'present');
  const jevOutcomes = {};
  for (const m of present) {
    for (const j of m.jev) {
      jevOutcomes[j.use] ??= {};
      jevOutcomes[j.use][j.outcome] = (jevOutcomes[j.use][j.outcome] ?? 0) + 1;
    }
  }
  return {
    ...manifestTotals(runs),
    meanLayersBytes: mean(present.map((m) => m.layersBytes)),
    retrievalKeptMean: mean(present.filter((m) => m.retrieval).map((m) => m.retrieval.kept)),
    retrievalConsideredMean: mean(present.filter((m) => m.retrieval).map((m) => m.retrieval.considered)),
    connectorsAttachedMean: mean(present.filter((m) => m.connectors).map((m) => m.connectors.attached)),
    jevOutcomes,
    fallbacks: present.reduce((n, m) => n + m.fallbacks, 0),
  };
}

/** Score the round in `dir` against `tasks` (validated suite tasks). */
export function scoreRound(dir, tasks, { tasksFile = null } = {}) {
  const plan = readJson(join(dir, 'plan.json'));
  const summary = readJson(join(dir, 'summary.json'));
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const warnings = [];
  const folders = readdirSync(dir).filter((name) => statSync(join(dir, name)).isDirectory() && keyParts(name) && byId.has(keyParts(name).id)).sort();
  const order = Array.isArray(plan?.order) ? plan.order : folders;
  if (!plan) warnings.push('no plan.json: the planned runs are the folders found');
  for (const name of folders) if (!order.includes(name)) warnings.push(`${name} is not in the round's plan; not scored`);
  const runs = [];
  const notRun = [];
  for (const key of order) {
    const parts = keyParts(key);
    const task = parts && byId.get(parts.id);
    if (!task) {
      warnings.push(`${key}: task ${parts?.id ?? key} is not in this suite; not scored`);
      continue;
    }
    const run = readJson(join(dir, key, 'run.json'));
    if (!run) {
      notRun.push({ key, task: task.id, slice: task.slice, reason: summary?.stopped?.reason ? `not run: the round stopped (${summary.stopped.reason})` : 'not run: no run.json' });
      continue;
    }
    if (run.status === 'send-failed') {
      notRun.push({ key, task: task.id, slice: task.slice, reason: `not sent: ${run.error ?? 'the driver refused'}` });
      continue;
    }
    runs.push(scoreRunDir(task, join(dir, key), { ...run, key }));
  }
  const planned = tasks.filter((t) => order.some((k) => keyParts(k)?.id === t.id));
  const recorded = typeof plan?.taskDigest === 'string' ? plan.taskDigest : null;
  const expected = taskDigest(planned);
  if (!recorded) warnings.push('the round records no task digest (a pilot before E1), so its messages cannot be shown to be this suite\'s');
  else if (recorded !== expected) warnings.push('the round sent different messages than this suite holds (task digest differs)');
  const taskRows = planned.map((t) => {
    const mine = runs.filter((r) => r.task === t.id);
    return {
      task: t.id, slice: t.slice, facet: t.facet, ...completion(mine),
      notRun: notRun.filter((r) => r.task === t.id).length,
      duration: durationTotals(mine),
      tokens: {
        input: sumKnown(mine.map((r) => r.tokens.input)), cached: sumKnown(mine.map((r) => r.tokens.cached)), output: sumKnown(mine.map((r) => r.tokens.output)),
        meanInput: mean(mine.map((r) => r.tokens.input).filter((x) => x != null)), meanCached: mean(mine.map((r) => r.tokens.cached).filter((x) => x != null)), meanOutput: mean(mine.map((r) => r.tokens.output).filter((x) => x != null)),
      },
      cost: costTotals(mine),
      manifest: manifestAggregate(mine),
    };
  });
  const sliceRows = SLICES.filter((s) => planned.some((t) => t.slice === s)).map((s) => {
    const mine = runs.filter((r) => r.slice === s);
    return { slice: s, tasks: planned.filter((t) => t.slice === s).length, ...completion(mine), duration: durationTotals(mine) };
  });
  const all = completion(runs);
  return {
    schema: 1,
    round: plan?.round ?? basename(resolve(dir)),
    dir: resolve(dir),
    mode: plan?.mode ?? null,
    settings: plan ? { agent: plan.agent ?? null, model: plan.model ?? null, effort: plan.effort ?? null, permissionMode: plan.permissionMode ?? null, repeats: plan.repeats ?? null, seed: plan.seed ?? null } : null,
    candidate: plan?.candidate ?? null,
    tasksFile,
    digest: { recorded, expected, matches: recorded == null ? null : recorded === expected },
    stopped: summary?.stopped ?? null,
    runs,
    notRun,
    tasks: taskRows,
    slices: sliceRows,
    failures: runs.filter((r) => !r.pass).map((r) => ({ key: r.key, task: r.task, slice: r.slice, status: r.status, reasons: r.reasons })),
    totals: {
      runs: { planned: order.filter((k) => byId.has(keyParts(k)?.id)).length, scored: runs.length, notRun: notRun.length, passed: all.passed },
      completion: all.completion,
      ci: all.ci,
      duration: durationTotals(runs),
      tokens: {
        input: sumKnown(runs.map((r) => r.tokens.input)), cached: sumKnown(runs.map((r) => r.tokens.cached)), output: sumKnown(runs.map((r) => r.tokens.output)),
        knownRuns: runs.filter((r) => r.tokens.input != null).length,
      },
      cost: costTotals(runs),
      manifests: manifestTotals(runs),
    },
    warnings,
  };
}

// ── Two rounds ───────────────────────────────────────────────────────────

/** Round B against round A, paired by task (only tasks scored in both):
 *  per-task and per-slice completion differences with Newcombe intervals,
 *  and flags for a slice that drops more than 3 points, a p95 duration
 *  rise over 10%, different settings, timing or messages. */
export function compareRounds(a, b, { dropPoints = 3, p95Rise = 0.1 } = {}) {
  const flags = [];
  const tasks = a.tasks.filter((ta) => ta.runs > 0 && b.tasks.some((tb) => tb.task === ta.task && tb.runs > 0)).map((ta) => {
    const tb = b.tasks.find((t) => t.task === ta.task);
    const d = newcombe(ta.passed, ta.runs, tb.passed, tb.runs);
    return { task: ta.task, slice: ta.slice, a: { passed: ta.passed, runs: ta.runs }, b: { passed: tb.passed, runs: tb.runs }, diff: d.diff, ci: { lo: d.lo, hi: d.hi } };
  });
  const paired = new Set(tasks.map((t) => t.task));
  const pooled = (report, slice) => report.runs.filter((r) => paired.has(r.task) && (slice === 'all' || r.slice === slice));
  const slices = [...SLICES.filter((s) => tasks.some((t) => t.slice === s)), 'all'].map((slice) => {
    const ra = pooled(a, slice);
    const rb = pooled(b, slice);
    const ca = completion(ra);
    const cb = completion(rb);
    const d = newcombe(ca.passed, ca.runs, cb.passed, cb.runs);
    const p95A = durationTotals(ra).p95Ms;
    const p95B = durationTotals(rb).p95Ms;
    const row = { slice, a: { passed: ca.passed, runs: ca.runs }, b: { passed: cb.passed, runs: cb.runs }, diff: d?.diff ?? null, ci: d ? { lo: d.lo, hi: d.hi } : null, p95MsA: p95A, p95MsB: p95B, p95Change: p95A && p95B != null ? round4((p95B - p95A) / p95A) : null };
    if (d && d.diff * 100 < -dropPoints) flags.push({ kind: 'completion-drop', slice, points: round4(d.diff * 100), detail: `${slice} completion ${ca.passed}/${ca.runs} → ${cb.passed}/${cb.runs} (${(d.diff * 100).toFixed(1)} points; flag at more than ${dropPoints})` });
    if (row.p95Change != null && row.p95Change > p95Rise) flags.push({ kind: 'p95-rise', slice, change: row.p95Change, detail: `${slice} p95 duration ${(p95A / 1000).toFixed(1)} s → ${(p95B / 1000).toFixed(1)} s (+${(row.p95Change * 100).toFixed(0)}%; flag over ${(p95Rise * 100).toFixed(0)}%)` });
    return row;
  });
  const srcA = a.totals.duration.sources.join('+') || 'none';
  const srcB = b.totals.duration.sources.join('+') || 'none';
  if (srcA !== srcB) flags.push({ kind: 'duration-source', detail: `durations were measured differently: ${a.round} ${srcA}, ${b.round} ${srcB}; p95 changes mix the pilot's own clock with the queue's` });
  if (!a.digest.recorded || !b.digest.recorded) flags.push({ kind: 'task-digest', detail: `${!a.digest.recorded ? a.round : b.round} records no task digest, so the rounds' messages cannot be shown to be the same` });
  else if (a.digest.recorded !== b.digest.recorded) flags.push({ kind: 'task-digest', detail: 'the rounds sent different messages (task digests differ)' });
  if (a.settings && b.settings) {
    const changed = ['agent', 'model', 'effort', 'permissionMode', 'repeats'].filter((k) => a.settings[k] !== b.settings[k]).map((k) => `${k} ${a.settings[k]} → ${b.settings[k]}`);
    if (changed.length) flags.push({ kind: 'settings', detail: `the rounds ran with different settings: ${changed.join(', ')}` });
  }
  const unpaired = [...a.tasks, ...b.tasks].map((t) => t.task).filter((id) => !paired.has(id));
  return { schema: 1, a: a.round, b: b.round, tasks, slices, flags, unpaired: [...new Set(unpaired)].sort() };
}

// ── Markdown ─────────────────────────────────────────────────────────────

/** A fraction as percent with one decimal, rounded half up (0.9385 → 93.9). */
const p1 = (x) => (Math.round(x * 1000) / 10).toFixed(1);
const pct = (x) => (x == null ? '—' : `${p1(x)}%`);
const ci = (c) => (c ? `${p1(c.lo)}–${p1(c.hi)}%` : '—');
const secs = (msv) => (msv == null ? '—' : (msv / 1000).toFixed(1));
const usd = (c) => (c.totalUsd != null ? `$${c.totalUsd.toFixed(2)}` : `unknown (${c.unknownRuns} run${c.unknownRuns === 1 ? '' : 's'} without a cost), $${c.knownUsd.toFixed(2)} known`);
const cell = (s) => String(s).replace(/\|/g, '\\|').replace(/\n/g, ' ');
const n0 = (x) => (x == null ? '—' : String(Math.round(x)));

/** The round as markdown: slices, tasks, failures, not-run, totals. */
export function renderReport(r) {
  const out = [`# Pilot round ${r.round}`, ''];
  const s = r.settings;
  out.push(`Mode ${r.mode ?? '—'}; ${s ? `${s.agent} ${s.model} at ${s.effort}, ${s.permissionMode}, ${s.repeats} repeats, seed ${s.seed}` : 'no plan.json'}; candidate ${r.candidate?.commit ?? '—'}.`);
  out.push(`Runs: ${r.totals.runs.planned} planned, ${r.totals.runs.scored} scored, ${r.totals.runs.notRun} not run, ${r.totals.runs.passed} passed. Completion ${pct(r.totals.completion)} (95% CI ${ci(r.totals.ci)}).`);
  if (r.stopped) out.push(`Stopped: ${r.stopped.reason}`);
  out.push('', '## Slices', '', '| Slice | Tasks | Runs | Passed | Completion | 95% CI (Wilson) | p95 s |', '|---|---|---|---|---|---|---|');
  for (const x of r.slices) out.push(`| ${x.slice} | ${x.tasks} | ${x.runs} | ${x.passed} | ${pct(x.completion)} | ${ci(x.ci)} | ${secs(x.duration.p95Ms)} |`);
  out.push('', '## Tasks', '', '| Task | Slice | Runs | Passed | Completion | 95% CI | p50 s | p95 s | Mean input / cached / output tokens | Cost | Manifests | Mean context bytes |', '|---|---|---|---|---|---|---|---|---|---|---|---|');
  for (const t of r.tasks) {
    const m = t.manifest;
    out.push(`| ${t.task} | ${t.slice} | ${t.runs} | ${t.passed} | ${pct(t.completion)} | ${ci(t.ci)} | ${secs(t.duration.p50Ms)} | ${secs(t.duration.p95Ms)} | ${n0(t.tokens.meanInput)} / ${n0(t.tokens.meanCached)} / ${n0(t.tokens.meanOutput)} | ${cell(usd(t.cost))} | present ${m.present}, absent ${m.absent}${m.unreadable ? `, unreadable ${m.unreadable}` : ''}${m.mismatch ? `, mismatch ${m.mismatch}` : ''} | ${n0(m.meanLayersBytes)} |`);
  }
  out.push('', '## Failures', '');
  if (!r.failures.length) out.push('None.');
  for (const f of r.failures) out.push(`- ${f.key} (${f.status ?? 'no status'}): ${cell(f.reasons.join('; '))}`);
  out.push('', '## Not run', '');
  if (!r.notRun.length) out.push('None.');
  for (const n of r.notRun) out.push(`- ${n.key}: ${cell(n.reason)}`);
  const tk = r.totals.tokens;
  const mt = r.totals.manifests;
  out.push('', '## Totals', '');
  out.push(`- Cost: ${usd(r.totals.cost)}`);
  out.push(`- Tokens: input ${tk.input ?? 'unknown'}, cached ${tk.cached ?? 'unknown'}, output ${tk.output ?? 'unknown'} (${tk.knownRuns} of ${r.totals.runs.scored} runs with counts)`);
  out.push(`- Duration: p50 ${secs(r.totals.duration.p50Ms)} s, p95 ${secs(r.totals.duration.p95Ms)} s (${r.totals.duration.sources.join(', ') || 'no times'})`);
  out.push(`- Context manifests: present ${mt.present}, absent ${mt.absent}, unreadable ${mt.unreadable}, mismatch ${mt.mismatch}`);
  if (r.warnings.length) {
    out.push('', '## Warnings', '');
    for (const w of r.warnings) out.push(`- ${cell(w)}`);
  }
  return `${out.join('\n')}\n`;
}

const pts = (d) => (d == null ? '—' : p1(d));
const ciPts = (c) => (c ? `${p1(c.lo)} to ${p1(c.hi)}` : '—');

/** The comparison as markdown. */
export function renderComparison(c) {
  const out = [`# Pilot rounds ${c.a} → ${c.b}`, '', 'Paired by task: only tasks scored in both rounds. Differences are B minus A in completion points, with Newcombe 95% intervals.', ''];
  out.push('## Flags', '');
  if (!c.flags.length) out.push('None.');
  for (const f of c.flags) out.push(`- ${f.kind}${f.slice ? ` (${f.slice})` : ''}: ${cell(f.detail)}`);
  out.push('', '## Slices', '', `| Slice | ${c.a} | ${c.b} | Difference (points) | 95% CI | p95 s ${c.a} | p95 s ${c.b} | p95 change |`, '|---|---|---|---|---|---|---|---|');
  for (const s of c.slices) out.push(`| ${s.slice} | ${s.a.passed}/${s.a.runs} | ${s.b.passed}/${s.b.runs} | ${pts(s.diff)} | ${ciPts(s.ci)} | ${secs(s.p95MsA)} | ${secs(s.p95MsB)} | ${s.p95Change == null ? '—' : `${(s.p95Change * 100).toFixed(0)}%`} |`);
  out.push('', '## Tasks', '', `| Task | Slice | ${c.a} | ${c.b} | Difference (points) | 95% CI |`, '|---|---|---|---|---|---|');
  for (const t of c.tasks) out.push(`| ${t.task} | ${t.slice} | ${t.a.passed}/${t.a.runs} | ${t.b.passed}/${t.b.runs} | ${pts(t.diff)} | ${ciPts(t.ci)} |`);
  if (c.unpaired.length) out.push('', `Not paired (scored in one round only): ${c.unpaired.join(', ')}.`);
  return `${out.join('\n')}\n`;
}

// ── The command ──────────────────────────────────────────────────────────

const USAGE = 'usage: node scripts/agent-eval/score.mjs score <roundDir> [--tasks <file>] [--out <dir>]\n       node scripts/agent-eval/score.mjs compare <roundDirA> <roundDirB> [--tasks <file>] [--out <dir>]';

function write(dir, name, value, md) {
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${name}.json`), `${JSON.stringify(value, null, 2)}\n`);
  writeFileSync(join(dir, `${name}.md`), md);
}

export function main(argv) {
  const [command, ...rest] = argv;
  const positional = [];
  let tasksFile = DEFAULT_TASKS;
  let out = null;
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--tasks') tasksFile = rest[++i];
    else if (rest[i] === '--out') out = rest[++i];
    else if (rest[i].startsWith('--')) throw new Error(`unknown flag ${rest[i]}\n${USAGE}`);
    else positional.push(rest[i]);
  }
  if (!tasksFile || (out === undefined)) throw new Error(USAGE);
  const suite = loadSuite(tasksFile);
  if (command === 'score' && positional.length === 1) {
    const report = scoreRound(positional[0], suite.tasks, { tasksFile: { path: suite.file, sha256: suite.sha256 } });
    const dir = out ?? positional[0];
    write(dir, 'score', report, renderReport(report));
    console.log(renderReport(report));
    console.log(`[agent-eval] wrote ${join(dir, 'score.json')} and score.md`);
    return report;
  }
  if (command === 'compare' && positional.length === 2) {
    const [a, b] = positional.map((p) => scoreRound(p, suite.tasks, { tasksFile: { path: suite.file, sha256: suite.sha256 } }));
    const cmp = compareRounds(a, b);
    const dir = out ?? positional[1];
    const name = `compare-${a.round}-${b.round}`;
    write(dir, name, cmp, renderComparison(cmp));
    console.log(renderComparison(cmp));
    console.log(`[agent-eval] wrote ${join(dir, `${name}.json`)} and .md`);
    return cmp;
  }
  throw new Error(USAGE);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}

