// NV6: judge qualification. Before a judged score is trusted, measure the
// judge: does it agree with itself across repeats, and with the labels?
// (Idea from SkillEvaluator; method only, no code copied.)
//
//   node scripts/agent-eval/judge-qualify.mjs run --judge rules|claude|codex
//        --out <dir> [--items <file>]... [--repeats 3] [--live] [--json <f>]
//   node scripts/agent-eval/judge-qualify.mjs score <verdicts.jsonl>
//        [--items <file>]... [--json <f>]
//
// Items: every judged pilot task in pilot-tasks.json gives its reference
// answer (label `pass`) and each of its `fails` (label `fail`), with the
// task's prompt and fixture evidence; `--items <file>` adds hand-labelled
// items (`{schema: 1, items: [{id, prompt, evidence, answer, label}]}`).
//
// Each item is graded `--repeats` times (default 3). The items are split by
// a hash of their task (a hand-labelled item: its id), about half into a
// held-out set, so a task's reference and failing answers stay on one side:
// the dev set is where a judge prompt may be tuned, and only the held-out set
// decides qualification. Per set: self-agreement (share of items whose
// repeats all agree, pairwise agreement, the ids that flipped) and label
// agreement (the majority verdict against the label, per label, Cohen's
// kappa with undecided counted as a miss). A judge is `qualified` when the
// held-out set has both labels, every label and the whole set meet LABEL_MIN,
// kappa meets KAPPA_MIN and self-agreement meets SELF_MIN, with every
// verdict present; `provisional` when it does on fewer than MIN_HELD_OUT
// items; else `not qualified` with the reasons.
//
// Judges: `rules` is score.mjs's deterministic rule scorer (free; a sanity
// baseline that must be unanimous); `claude` and `codex` are paid model
// calls, one per item and repeat, run only with `--live`, with no tools, no
// MCP server and the read-only sandbox, on the synthetic fixture text only.
// Never in the gate: the tests use the rules judge and a fake spawn.
import { spawn as nodeSpawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { DEFAULT_TASKS, loadSuite, scoreArtifact } from './score.mjs';

export const JUDGES = ['rules', 'claude', 'codex'];
export const MODEL_JUDGES = ['claude', 'codex'];
export const REPEATS = 3;
/** Held-out share of items, by id hash. */
export const HELD_OUT_SHARE = 0.5;
/** Qualification thresholds on the held-out set. */
export const SELF_MIN = 0.95;
export const LABEL_MIN = 0.9;
export const MIN_HELD_OUT = 10;
export const KAPPA_MIN = 0.8;
/** A model judge call that has not answered by then is a failed verdict. */
export const CALL_TIMEOUT_MS = 120_000;
const LABELS = ['pass', 'fail'];
const ID = /^[a-z0-9][a-z0-9/_-]{0,127}$/;

// ── Items ────────────────────────────────────────────────────────────────

/** The pilot suite's judged items: each task's reference answer (pass) and
 *  failing answers (fail). `task` keeps the rule scorer's view. */
export function pilotItems(tasks) {
  return tasks.flatMap((t) => [
    { id: `${t.id}/reference`, source: 'pilot', task: t, prompt: t.prompt, evidence: t.judged.evidence, answer: t.judged.answer, label: 'pass' },
    ...t.judged.fails.map((f, i) => ({ id: `${t.id}/fail-${i + 1}`, source: 'pilot', task: t, prompt: t.prompt, evidence: t.judged.evidence, answer: f.text, label: 'fail', because: f.because })),
  ]);
}

/** Hand-labelled items from a file. Throws with every problem listed. */
export function validateItems(input) {
  const errors = [];
  if (input?.schema !== 1) errors.push('schema must be 1');
  const items = Array.isArray(input?.items) ? input.items : (errors.push('items must be an array'), []);
  for (const [i, it] of items.entries()) {
    const at = `items[${i}]`;
    if (typeof it?.id !== 'string' || !ID.test(it.id)) errors.push(`${at}: id must match ${ID}`);
    if (typeof it?.prompt !== 'string' || !it.prompt.trim()) errors.push(`${at}: prompt is required`);
    if (typeof it?.answer !== 'string') errors.push(`${at}: answer is required`);
    if (!LABELS.includes(it?.label)) errors.push(`${at}: label must be pass or fail`);
    if (it?.evidence != null && (!Array.isArray(it.evidence) || it.evidence.some((e) => typeof e?.quote !== 'string'))) errors.push(`${at}: evidence lists { path, quote }`);
  }
  if (errors.length) throw new Error(`judge items:\n- ${errors.join('\n- ')}`);
  return items.map((it) => ({ id: it.id, source: 'file', task: null, prompt: it.prompt, evidence: it.evidence ?? [], answer: it.answer, label: it.label }));
}

/** Pilot items plus any item files; ids must be unique. */
export function loadItems(files = [], { tasksFile = DEFAULT_TASKS } = {}) {
  const items = [...pilotItems(loadSuite(tasksFile).tasks), ...files.flatMap((f) => validateItems(JSON.parse(readFileSync(f, 'utf8'))))];
  const seen = new Set();
  for (const it of items) {
    if (seen.has(it.id)) throw new Error(`judge items: duplicate id ${it.id}`);
    seen.add(it.id);
  }
  return items;
}

/** `held-out` or `dev`, fixed by the key's hash. */
export function splitOf(key, share = HELD_OUT_SHARE) {
  const n = createHash('sha256').update(key).digest().readUInt32BE(0) / 0x1_0000_0000;
  return n < share ? 'held-out' : 'dev';
}

/** An item's split key: its task, so one task's answers share a side. */
export const splitKey = (item) => item.task?.id ?? item.id;

// ── Judges ───────────────────────────────────────────────────────────────

/** The judge's prompt: the task, its evidence, the answer. Never the label
 *  or the reference answer. */
export function judgePrompt(item) {
  const evidence = item.evidence.length ? item.evidence.map((e) => `- ${e.path ?? 'evidence'}: "${e.quote}"`).join('\n') : '- (none)';
  return [
    'You are grading one answer to a task. Decide only whether the answer is correct and complete for the task, using the evidence quoted from the workspace. Do not use tools.',
    '',
    `Task: ${item.prompt}`,
    '',
    'Evidence:',
    evidence,
    '',
    'Answer to grade:',
    '<<<',
    item.answer,
    '>>>',
    '',
    'Reply with exactly one line: VERDICT: PASS or VERDICT: FAIL.',
  ].join('\n');
}

/** The verdict in a judge's reply text (its last VERDICT line), or null. */
export function parseVerdict(text) {
  const all = [...String(text ?? '').matchAll(/VERDICT:\s*\**\s*(PASS|FAIL)\b/gi)];
  return all.length ? all.at(-1)[1].toLowerCase() : null;
}

/** The rule judge: score.mjs's rules over the answer as the artifact. Only
 *  pilot items carry rules. */
export function rulesVerdict(item) {
  if (!item.task) return null;
  return scoreArtifact(item.task, item.answer).pass ? 'pass' : 'fail';
}

/** A model judge's command (argv, no shell). */
export function judgeCommand(judge, prompt, { claudeBin = 'claude', codexBin = 'codex' } = {}) {
  if (judge === 'claude') return { bin: claudeBin, args: ['-p', prompt, '--output-format', 'json', '--max-turns', '1', '--tools', '', '--strict-mcp-config', '--permission-mode', 'default'] };
  if (judge === 'codex') return { bin: codexBin, args: ['exec', '--skip-git-repo-check', '--ephemeral', '--ignore-user-config', '--ignore-rules', '--json', '--color', 'never', '--sandbox', 'read-only', prompt] };
  throw new Error(`not a model judge: ${judge}`);
}

/** A model judge's reply text from its stdout: Claude's `--output-format
 *  json` `result`, Codex's last `agent_message`. */
export function replyText(judge, stdout) {
  if (judge === 'claude') {
    try {
      const r = JSON.parse(stdout);
      return r?.is_error ? null : typeof r?.result === 'string' ? r.result : null;
    } catch {
      return null;
    }
  }
  let text = null;
  for (const line of String(stdout).split('\n')) {
    if (!line.includes('agent_message')) continue;
    try {
      const e = JSON.parse(line);
      if (e?.type === 'item.completed' && e.item?.type === 'agent_message' && typeof e.item.text === 'string') text = e.item.text;
    } catch {
      // not an event line
    }
  }
  return text;
}

/** One model judge call: resolves with its verdict (null when it failed or
 *  gave none) and an error, if any. */
export function callJudge(judge, item, { spawn = nodeSpawn, timeoutMs = CALL_TIMEOUT_MS, setTimer = setTimeout, clearTimer = clearTimeout, claudeBin, codexBin } = {}) {
  const { bin, args } = judgeCommand(judge, judgePrompt(item), { claudeBin, codexBin });
  // An empty folder of its own, so the judge can't find the labels (the
  // suite file) or the repository's instructions on disk.
  const cwd = mkdtempSync(join(tmpdir(), 'judge-qualify-'));
  return new Promise((done) => {
    const child = spawn(bin, args, { cwd, stdio: ['ignore', 'pipe', 'ignore'] });
    let out = '';
    let settled = false;
    // The call settles at the timeout even if the child ignores the signal.
    const timer = setTimer(() => {
      child.kill('SIGKILL');
      finish(`no answer within ${timeoutMs} ms`);
    }, timeoutMs);
    const finish = (error) => {
      if (settled) return;
      settled = true;
      clearTimer(timer);
      rmSync(cwd, { recursive: true, force: true });
      const verdict = error ? null : parseVerdict(replyText(judge, out));
      done({ verdict, error: error ?? (verdict ? null : 'no verdict in the reply') });
    };
    child.stdout.on('data', (d) => {
      out += d;
    });
    child.on('error', (e) => finish(e.message));
    child.on('close', (code) => finish(code ? `exit ${code}` : null));
  });
}

// ── Scoring ──────────────────────────────────────────────────────────────

const round4 = (x) => (x == null ? null : Math.round(x * 1e4) / 1e4);
const share = (k, n) => (n ? round4(k / n) : null);

/** Cohen's kappa between two pass/fail lists of equal length, or null. */
export function cohenKappa(a, b) {
  const n = a.length;
  if (!n) return null;
  const agree = a.filter((x, i) => x === b[i]).length / n;
  const pa = a.filter((x) => x === 'pass').length / n;
  const pb = b.filter((x) => x === 'pass').length / n;
  const chance = pa * pb + (1 - pa) * (1 - pb);
  return chance === 1 ? (agree === 1 ? 1 : null) : round4((agree - chance) / (1 - chance));
}

/** One item's repeats: unanimous, the majority (`pass`, `fail`, or
 *  `undecided` on a tie or with no parsed verdict) and agreeing pairs. */
export function itemAgreement(verdicts) {
  const v = verdicts.map((x) => (LABELS.includes(x) ? x : 'none'));
  const count = (l) => v.filter((x) => x === l).length;
  const pass = count('pass');
  const fail = count('fail');
  let pairs = 0;
  let agreeing = 0;
  for (let i = 0; i < v.length; i++) {
    for (let j = i + 1; j < v.length; j++) {
      pairs += 1;
      if (v[i] === v[j] && v[i] !== 'none') agreeing += 1;
    }
  }
  return {
    unanimous: v.length > 0 && v.every((x) => x === v[0]) && v[0] !== 'none',
    majority: pass > fail && pass > v.length / 2 ? 'pass' : fail > pass && fail > v.length / 2 ? 'fail' : 'undecided',
    pairs,
    agreeing,
  };
}

/** Self- and label agreement over a set of graded items. */
export function agreement(rows) {
  const n = rows.length;
  const pairs = rows.reduce((s, r) => s + r.pairs, 0);
  const agreeing = rows.reduce((s, r) => s + r.agreeing, 0);
  const decided = rows.filter((r) => r.majority !== 'undecided');
  const perLabel = {};
  for (const l of LABELS) {
    const of = rows.filter((r) => r.label === l);
    perLabel[l] = { items: of.length, agree: of.filter((r) => r.majority === l).length, share: share(of.filter((r) => r.majority === l).length, of.length) };
  }
  return {
    items: n,
    self: {
      unanimous: rows.filter((r) => r.unanimous).length,
      share: share(rows.filter((r) => r.unanimous).length, n),
      pairwise: share(agreeing, pairs),
      flipped: rows.filter((r) => !r.unanimous).map((r) => r.id),
    },
    label: {
      agree: rows.filter((r) => r.majority === r.label).length,
      share: share(rows.filter((r) => r.majority === r.label).length, n),
      perLabel,
      undecided: n - decided.length,
      // An undecided item counts as a miss: the verdict opposite its label.
      kappa: cohenKappa(rows.map((r) => (r.majority === 'undecided' ? (r.label === 'pass' ? 'fail' : 'pass') : r.majority)), rows.map((r) => r.label)),
      wrong: rows.filter((r) => r.majority !== r.label).map((r) => ({ id: r.id, label: r.label, majority: r.majority })),
    },
  };
}

/** The qualification verdict on the held-out set. `problems` are record
 *  faults (incomplete, unknown or duplicate verdicts): any one refuses. */
export function qualify(heldOut, problems = []) {
  const reasons = [...problems];
  if (!heldOut.items) return { status: 'not qualified', reasons: [...reasons, 'no held-out items were graded'] };
  if (heldOut.self.share < SELF_MIN) reasons.push(`self-agreement ${heldOut.self.share} under ${SELF_MIN} (${heldOut.self.flipped.length} items flipped)`);
  if (heldOut.label.share < LABEL_MIN) reasons.push(`label agreement ${heldOut.label.share} under ${LABEL_MIN}`);
  for (const l of LABELS) {
    const p = heldOut.label.perLabel[l];
    if (!p.items) reasons.push(`no held-out item labelled ${l}`);
    else if (p.share < LABEL_MIN) reasons.push(`${l} agreement ${p.share} under ${LABEL_MIN}`);
  }
  if (heldOut.label.kappa == null || heldOut.label.kappa < KAPPA_MIN) reasons.push(`kappa ${heldOut.label.kappa ?? 'n/a'} under ${KAPPA_MIN}`);
  if (reasons.length) return { status: 'not qualified', reasons };
  if (heldOut.items < MIN_HELD_OUT) return { status: 'provisional', reasons: [`only ${heldOut.items} held-out items (under ${MIN_HELD_OUT})`] };
  return { status: 'qualified', reasons: [] };
}

/** Score verdict records `{item, repeat, verdict}` against the items. An
 *  item with fewer than `repeats` records is incomplete and left out. */
export function scoreVerdicts(items, records, { judge = null, repeats = REPEATS } = {}) {
  const byItem = new Map(items.map((it) => [it.id, []]));
  const unknown = new Set();
  const duplicate = new Set();
  for (const r of records) {
    if (!byItem.has(r.item)) unknown.add(r.item);
    else if (!Number.isInteger(r.repeat) || r.repeat < 0 || r.repeat >= repeats) unknown.add(`${r.item}#${r.repeat}`);
    else if (byItem.get(r.item)[r.repeat] !== undefined) duplicate.add(`${r.item}#${r.repeat}`);
    else byItem.get(r.item)[r.repeat] = r.verdict ?? null;
  }
  const rows = [];
  const incomplete = [];
  for (const it of items) {
    const v = byItem.get(it.id);
    const got = Array.from({ length: repeats }, (_, i) => v[i]);
    if (got.some((x) => x === undefined)) {
      incomplete.push(it.id);
      continue;
    }
    rows.push({ id: it.id, label: it.label, split: splitOf(splitKey(it)), verdicts: got, ...itemAgreement(got) });
  }
  const dev = agreement(rows.filter((r) => r.split === 'dev'));
  const heldOut = agreement(rows.filter((r) => r.split === 'held-out'));
  return {
    eval: 'judge-qualification',
    judge,
    repeats,
    thresholds: { selfMin: SELF_MIN, labelMin: LABEL_MIN, kappaMin: KAPPA_MIN, minHeldOut: MIN_HELD_OUT },
    items: items.length,
    graded: rows.length,
    incomplete,
    unknown: [...unknown].sort(),
    duplicate: [...duplicate].sort(),
    all: agreement(rows),
    dev,
    heldOut,
    qualification: qualify(heldOut, [
      ...(incomplete.length ? [`${incomplete.length} items without all ${repeats} verdicts`] : []),
      ...(unknown.size ? [`${unknown.size} verdicts for unknown items or repeats`] : []),
      ...(duplicate.size ? [`${duplicate.size} duplicate verdicts`] : []),
    ]),
  };
}

export function renderQualification(r) {
  const line = (name, a) =>
    `- ${name}: ${a.items} items; self-agreement ${a.self.unanimous}/${a.items} unanimous (${a.self.share ?? 'n/a'}, pairwise ${a.self.pairwise ?? 'n/a'}); label agreement ${a.label.agree}/${a.items} (${a.label.share ?? 'n/a'}; pass ${a.label.perLabel.pass.agree}/${a.label.perLabel.pass.items}, fail ${a.label.perLabel.fail.agree}/${a.label.perLabel.fail.items}; kappa ${a.label.kappa ?? 'n/a'})`;
  const out = [`Judge qualification: ${r.judge ?? 'judge'}, ${r.repeats} repeats, ${r.graded}/${r.items} items graded`, line('held-out', r.heldOut), line('dev', r.dev)];
  if (r.all.self.flipped.length) out.push(`  flipped: ${r.all.self.flipped.join(', ')}`);
  if (r.all.label.wrong.length) out.push(`  against the label: ${r.all.label.wrong.map((w) => `${w.id} (${w.label}, judged ${w.majority})`).join(', ')}`);
  if (r.incomplete.length) out.push(`  incomplete: ${r.incomplete.length}`);
  out.push(`Qualification (held-out): ${r.qualification.status}${r.qualification.reasons.length ? `: ${r.qualification.reasons.join('; ')}` : ''}`);
  return out.join('\n');
}

// ── Run ──────────────────────────────────────────────────────────────────

/** Grade every item `repeats` times; returns the verdict records. The rules
 *  judge is free; a model judge needs `live`. */
export async function grade(items, { judge, repeats = REPEATS, live = false, spawn = nodeSpawn, log = () => {}, ...call } = {}) {
  if (!JUDGES.includes(judge)) throw new Error(`--judge must be one of ${JUDGES.join(', ')}`);
  if (MODEL_JUDGES.includes(judge) && !live) throw new Error(`the ${judge} judge makes ${items.length * repeats} paid calls; add --live to run them`);
  const records = [];
  for (const it of items) {
    for (let repeat = 0; repeat < repeats; repeat++) {
      if (judge === 'rules') {
        records.push({ item: it.id, repeat, verdict: rulesVerdict(it) });
        continue;
      }
      const r = await callJudge(judge, it, { spawn, ...call });
      records.push({ item: it.id, repeat, verdict: r.verdict, ...(r.error ? { error: r.error } : {}) });
      log(`${it.id} #${repeat + 1}: ${r.verdict ?? `no verdict (${r.error})`}`);
    }
  }
  return records;
}

export function readVerdicts(file) {
  return readFileSync(file, 'utf8')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
}

export function parseArgs(argv) {
  const [command, ...rest] = argv;
  const o = { command, verdicts: null, judge: null, out: null, items: [], repeats: REPEATS, live: false, json: null, claudeBin: 'claude', codexBin: 'codex' };
  const args = command === 'score' ? rest.slice(1) : rest;
  if (command === 'score') o.verdicts = rest[0];
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    const next = () => {
      if (i + 1 >= args.length) throw new Error(`${a} needs a value`);
      return args[++i];
    };
    if (a === '--judge') o.judge = next();
    else if (a === '--out') o.out = next();
    else if (a === '--items') o.items.push(next());
    else if (a === '--repeats') {
      o.repeats = Number(next());
      if (!Number.isInteger(o.repeats) || o.repeats < 2) throw new Error('--repeats must be a whole number of at least 2');
    } else if (a === '--live') o.live = true;
    else if (a === '--json') o.json = next();
    else if (a === '--claude-bin') o.claudeBin = next();
    else if (a === '--codex-bin') o.codexBin = next();
    else throw new Error(`unknown argument: ${a}`);
  }
  if (command === 'run') {
    if (!JUDGES.includes(o.judge)) throw new Error(`run needs --judge ${JUDGES.join('|')}`);
    if (!o.out) throw new Error('run needs --out <dir>');
  } else if (command === 'score') {
    if (!o.verdicts) throw new Error('score needs a verdicts.jsonl file');
  } else throw new Error('usage: judge-qualify.mjs run --judge rules|claude|codex --out <dir> [--live] | score <verdicts.jsonl>');
  return o;
}

export async function main(argv, { spawn = nodeSpawn, log = console.log, tasksFile = DEFAULT_TASKS } = {}) {
  const o = parseArgs(argv);
  const items = loadItems(o.items, { tasksFile });
  let records;
  let judge = o.judge;
  if (o.command === 'run') {
    records = await grade(items, { judge: o.judge, repeats: o.repeats, live: o.live, spawn, log, claudeBin: o.claudeBin, codexBin: o.codexBin });
    mkdirSync(o.out, { recursive: true });
    writeFileSync(join(o.out, 'verdicts.jsonl'), records.map((r) => JSON.stringify(r)).join('\n') + '\n');
  } else {
    records = readVerdicts(o.verdicts);
    const repeats = Math.max(0, ...records.map((r) => r.repeat + 1));
    if (repeats >= 2) o.repeats = repeats;
  }
  const report = scoreVerdicts(items, records, { judge, repeats: o.repeats });
  if (o.json) writeFileSync(o.json, `${JSON.stringify(report, null, 2)}\n`);
  log(renderQualification(report));
  return report;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
