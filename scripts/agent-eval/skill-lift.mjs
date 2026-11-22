// NV4: with and without skill lift. Does a skill help at all? Both arms get
// the same cases and the same grader; the lift is the difference with a
// paired bootstrap interval. (Idea from SkillEvaluator; method only.)
//
//   node scripts/agent-eval/skill-lift.mjs score <suite.json> <answers dir>
//        [--k <n>] [--json <file>]
//   node scripts/agent-eval/skill-lift.mjs run <suite.json> --out <dir>
//        [--repeats 3] [--library <skills dir>] [--claude-bin <path>] [--live]
//
// A suite names one skill and its cases: a prompt, the score.mjs rules its
// answer must meet, and `negative: true` for a case the skill does not apply
// to (the skill must not hurt it). `fixtures/skill-lift/humanizer.json` is
// the first suite (synthetic text only).
//
// Answers are `<dir>/<arm>/<case>.<attempt>.txt`, arm `with` or `without`.
// An attempt with a `.fail` note (the agent ended in an error, hit its turn
// limit or ran out of time) is a fail; one with a `.error` note (the run never
// got going: spawn error, no result) is left out and makes the report
// provisional. A `.skills.json` beside an answer lists the skills the agent
// was offered (its stream's init event), so the report can check the arms:
// the without arm must never have the skill and the with arm always. Per arm: pass rate (the mean of each case's share of passing
// attempts) and pass@k (cases with a pass in their first k attempts) with
// Wilson intervals. Lift is with minus without, over the cases both arms
// answered, with a seeded paired bootstrap interval over cases (the same
// resampled cases for both arms); negative cases get their own lift. A
// report under MIN_CASES paired cases, with no negative case, with a run
// left out, or with an arm that saw the wrong skills, is provisional.
//
// `run` is paid (Claude Code, one call per case, arm and attempt) and needs
// `--live`; otherwise it prints the plan. Each call is in an empty scratch
// folder: the with arm links the library's skill as a project skill, the
// without arm has none. Both arms run with project settings only (no user or
// plugin skills, no user settings), no file edits, no shell, no MCP server,
// default permissions, and the same prompt and flags, taking turns case by
// case so drift falls on both.
// Never in the gate; the tests use recorded answers and a fake spawn.
import { spawn as nodeSpawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { seeded } from './jev-study.mjs';
import { checkRule, validateScore, wilson } from './score.mjs';

export const ARMS = ['with', 'without'];
export const MIN_CASES = 10;
export const REPEATS = 3;
export const BOOTSTRAP = { resamples: 2000, seed: 20261118 };
export const DEFAULT_LIBRARY = join(homedir(), '.agents', 'skills');
export const DISALLOWED = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'Bash'];
export const CALL_TIMEOUT_MS = 300_000;
const ID = /^[a-z0-9][a-z0-9-]{0,63}$/;
const ANSWER = /^([a-z0-9][a-z0-9-]{0,63})\.(\d+)\.txt$/;
const round4 = (x) => (x == null ? null : Math.round(x * 1e4) / 1e4);

// ── Suite ────────────────────────────────────────────────────────────────

/** A checked suite: `{schema: 1, skill, cases: [{id, prompt, negative,
 *  rules}]}`; rules as score.mjs checks them. Throws with every problem. */
export function validateSuite(input) {
  const errors = [];
  if (input?.schema !== 1) errors.push('schema must be 1');
  if (typeof input?.skill !== 'string' || !ID.test(input.skill)) errors.push('skill must be a skill folder name');
  const cases = Array.isArray(input?.cases) && input.cases.length ? input.cases : (errors.push('cases must be a non-empty array'), []);
  const seen = new Set();
  const out = [];
  for (const [i, c] of cases.entries()) {
    const at = `cases[${i}]${typeof c?.id === 'string' ? ` (${c.id})` : ''}`;
    if (typeof c?.id !== 'string' || !ID.test(c.id)) errors.push(`${at}: id must match ${ID}`);
    else if (seen.has(c.id)) errors.push(`${at}: duplicate id`);
    else seen.add(c.id);
    if (typeof c?.prompt !== 'string' || !c.prompt.trim()) errors.push(`${at}: prompt is required`);
    if (c?.negative != null && typeof c.negative !== 'boolean') errors.push(`${at}: negative is true or false`);
    let rules = [];
    try {
      rules = validateScore({ artifact: 'outputs/answer.md', rules: c?.rules }, null, at).rules;
    } catch (e) {
      errors.push(e.message);
    }
    out.push({ id: c?.id, prompt: c?.prompt, negative: c?.negative === true, rules });
  }
  if (errors.length) throw new Error(`skill-lift suite:\n- ${errors.join('\n- ')}`);
  return { skill: input.skill, cases: out };
}

export function loadSuite(file) {
  return validateSuite(JSON.parse(readFileSync(file, 'utf8')));
}

/** One answer graded by the case's rules (the one grader both arms share). */
export function gradeAnswer(c, text) {
  const failed = c.rules.filter((r) => !checkRule(r, text).pass).map((r) => r.id);
  return { pass: String(text).trim() !== '' && failed.length === 0, failed };
}

// ── Answers and scoring ──────────────────────────────────────────────────

/** Whether a recorded skill list holds `skill` (`plugin:skill` too), or
 *  null when none was recorded. */
function offered(dir, file, skill) {
  try {
    const list = JSON.parse(readFileSync(join(dir, `${file}.skills.json`), 'utf8'));
    return Array.isArray(list) ? list.some((s) => String(s).split(':').pop() === skill) : null;
  } catch {
    return null;
  }
}

/** One arm's folder: `byCase` (case → [{attempt, pass}], in order), the
 *  attempts left out (`.error`), agent errors counted as fails (`.fail`) and
 *  how often the skill was offered (`yes`, `no`, `unknown`). */
export function readArm(suite, dir) {
  const byCase = new Map(suite.cases.map((c) => [c.id, []]));
  const arm = { byCase, leftOut: 0, agentErrors: 0, offered: { yes: 0, no: 0, unknown: 0 } };
  let files = [];
  try {
    files = readdirSync(dir);
  } catch {
    return arm;
  }
  for (const f of files.sort()) {
    const m = ANSWER.exec(f);
    if (!m || !byCase.has(m[1])) continue;
    if (existsSync(join(dir, `${f}.error`))) {
      arm.leftOut += 1;
      continue;
    }
    const seen = offered(dir, f, suite.skill);
    arm.offered[seen == null ? 'unknown' : seen ? 'yes' : 'no'] += 1;
    if (existsSync(join(dir, `${f}.fail`))) {
      arm.agentErrors += 1;
      byCase.get(m[1]).push({ attempt: Number(m[2]), pass: false, failed: ['agent-error'] });
      continue;
    }
    const c = suite.cases.find((x) => x.id === m[1]);
    byCase.get(m[1]).push({ attempt: Number(m[2]), ...gradeAnswer(c, readFileSync(join(dir, f), 'utf8')) });
  }
  for (const list of byCase.values()) list.sort((a, b) => a.attempt - b.attempt);
  return arm;
}

/** One arm over a set of cases: pass rate, attempts, pass@k with Wilson. */
export function armStats(attempts, ids, k) {
  const answered = ids.filter((id) => attempts.get(id)?.length);
  const rates = answered.map((id) => attempts.get(id).filter((a) => a.pass).length / attempts.get(id).length);
  const all = answered.flatMap((id) => attempts.get(id));
  const solved = answered.filter((id) => attempts.get(id).slice(0, k).some((a) => a.pass)).length;
  return {
    cases: answered.length,
    attempts: all.length,
    passRate: rates.length ? round4(rates.reduce((s, r) => s + r, 0) / rates.length) : null,
    attemptPasses: all.filter((a) => a.pass).length,
    attemptInterval: wilson(all.filter((a) => a.pass).length, all.length),
    passAtK: { k, solved, share: answered.length ? round4(solved / answered.length) : null, interval: wilson(solved, answered.length) },
  };
}

const rateOf = (list) => list.filter((a) => a.pass).length / list.length;

/** With minus without, mean over `ids` of each case's pass rate, with a
 *  seeded percentile bootstrap over cases. Null with no case. */
export function pairedLift(withA, withoutA, ids, { resamples = BOOTSTRAP.resamples, seed = BOOTSTRAP.seed } = {}) {
  const n = ids.length;
  if (!n) return null;
  const diffs = ids.map((id) => rateOf(withA.get(id)) - rateOf(withoutA.get(id)));
  const mean = (xs) => xs.reduce((s, x) => s + x, 0) / xs.length;
  const rand = seeded(seed);
  const boot = [];
  for (let r = 0; r < resamples; r++) boot.push(mean(Array.from({ length: n }, () => diffs[Math.floor(rand() * n)])));
  boot.sort((a, b) => a - b);
  const q = (p) => round4(boot[Math.min(boot.length - 1, Math.max(0, Math.floor(p * boot.length)))]);
  return { cases: n, lift: round4(mean(diffs)), low: q(0.025), high: q(0.975), resamples };
}

/** The lift report for a suite over an answers folder. */
export function liftReport(suite, dir, { k = null } = {}) {
  const read = Object.fromEntries(ARMS.map((arm) => [arm, readArm(suite, join(dir, arm))]));
  const arms = Object.fromEntries(ARMS.map((arm) => [arm, read[arm].byCase]));
  const most = Math.max(1, ...ARMS.flatMap((arm) => [...arms[arm].values()].map((l) => l.length)));
  const kk = k ?? most;
  const paired = suite.cases.filter((c) => arms.with.get(c.id).length && arms.without.get(c.id).length);
  const ids = (cases) => cases.map((c) => c.id);
  const positive = paired.filter((c) => !c.negative);
  const negative = paired.filter((c) => c.negative);
  const reasons = [];
  if (paired.length < MIN_CASES) reasons.push(`${paired.length} paired cases (under ${MIN_CASES})`);
  if (!negative.length) reasons.push('no negative case answered by both arms');
  const leftOut = read.with.leftOut + read.without.leftOut;
  if (leftOut) reasons.push(`${leftOut} runs failed and were left out (with ${read.with.leftOut}, without ${read.without.leftOut})`);
  if (read.without.offered.yes) reasons.push(`the without arm was offered ${suite.skill} in ${read.without.offered.yes} attempts`);
  if (read.with.offered.no) reasons.push(`the with arm lacked ${suite.skill} in ${read.with.offered.no} attempts`);
  return {
    eval: 'skill-lift',
    skill: suite.skill,
    cases: suite.cases.length,
    paired: paired.length,
    unpaired: suite.cases.filter((c) => !paired.includes(c)).map((c) => c.id),
    provisional: reasons.length > 0,
    provisionalReasons: reasons,
    arms: Object.fromEntries(ARMS.map((arm) => [arm, { ...armStats(arms[arm], ids(paired), kk), leftOut: read[arm].leftOut, agentErrors: read[arm].agentErrors, offered: read[arm].offered }])),
    lift: pairedLift(arms.with, arms.without, ids(paired)),
    liftPositive: pairedLift(arms.with, arms.without, ids(positive)),
    liftNegative: pairedLift(arms.with, arms.without, ids(negative)),
    perCase: paired.map((c) => ({ id: c.id, negative: c.negative, with: round4(rateOf(arms.with.get(c.id))), without: round4(rateOf(arms.without.get(c.id))) })),
  };
}

const pct = (x) => (x == null ? 'n/a' : `${Math.round(x * 1000) / 10}%`);
const pp = (x) => (x == null ? 'n/a' : `${x > 0 ? '+' : ''}${Math.round(x * 1000) / 10} pts`);
const ival = (i) => (i ? ` (95% ${pct(i.lo)} to ${pct(i.hi)})` : '');
const lift = (l) => (l ? `${pp(l.lift)} (95% ${pp(l.low)} to ${pp(l.high)}, ${l.cases} cases)` : 'n/a');

export function renderLift(r) {
  const lines = [`Skill lift: ${r.skill}, ${r.paired}/${r.cases} cases answered by both arms${r.provisional ? ` [PROVISIONAL: ${r.provisionalReasons.join('; ')}]` : ''}`];
  for (const arm of ARMS) {
    const a = r.arms[arm];
    lines.push(`- ${arm} the skill: pass rate ${pct(a.passRate)}; attempts ${a.attemptPasses}/${a.attempts}${ival(a.attemptInterval)}; pass@${a.passAtK.k} ${a.passAtK.solved}/${a.cases}${ival(a.passAtK.interval)}; agent errors ${a.agentErrors}, left out ${a.leftOut}; skill offered ${a.offered.yes}, not ${a.offered.no}, unknown ${a.offered.unknown}`);
  }
  lines.push(`Lift: ${lift(r.lift)}; where it applies ${lift(r.liftPositive)}; negative cases ${lift(r.liftNegative)}`);
  if (r.unpaired.length) lines.push(`Not in both arms: ${r.unpaired.join(', ')}`);
  return lines.join('\n');
}

// ── Live run (paid; never in the gate) ───────────────────────────────────

/** Claude Code's command, the same for both arms: only the scratch folder
 *  (the with arm's project skill link) differs. Project settings only, so no
 *  user or plugin skill loads in either arm. */
export function armCommand(prompt, { claudeBin = 'claude' } = {}) {
  return { bin: claudeBin, args: ['-p', prompt, '--output-format', 'stream-json', '--verbose', '--max-turns', '4', '--setting-sources', 'project', '--strict-mcp-config', '--permission-mode', 'default', '--disallowedTools', DISALLOWED.join(' ')] };
}

/** Every planned call, the arms taking turns case by case and attempt by
 *  attempt. */
export function planRun(suite, { out, repeats = REPEATS, claudeBin } = {}) {
  return suite.cases.flatMap((c) =>
    Array.from({ length: repeats }, (_, i) => ARMS.map((arm) => ({ arm, id: c.id, attempt: i + 1, answer: join(out, arm, `${c.id}.${i + 1}.txt`), ...armCommand(c.prompt, { claudeBin }) }))).flat(),
  );
}

/** An empty scratch folder; the with arm's links the skill as a project
 *  skill. */
export function armScratch(arm, skill, library, { base = tmpdir() } = {}) {
  const dir = mkdtempSync(join(base, `skill-lift-${arm}-`));
  if (arm === 'with') {
    mkdirSync(join(dir, '.claude', 'skills'), { recursive: true });
    symlinkSync(join(library, skill), join(dir, '.claude', 'skills', skill));
  }
  return dir;
}

/** One call. Writes the answer text and the skills it was offered; an agent
 *  error (an error result, the turn limit, the timeout) writes an empty
 *  answer and a `.fail` note (a fail), a run that never got going (spawn
 *  error, no result) a `.error` note (left out). */
export function runOne(run, cwd, { spawn = nodeSpawn, timeoutMs = CALL_TIMEOUT_MS, setTimer = setTimeout, clearTimer = clearTimeout } = {}) {
  mkdirSync(join(run.answer, '..'), { recursive: true });
  for (const note of ['error', 'fail', 'skills.json']) rmSync(`${run.answer}.${note}`, { force: true });
  return new Promise((done) => {
    const child = spawn(run.bin, run.args, { cwd, stdio: ['ignore', 'pipe', 'ignore'] });
    let out = '';
    let settled = false;
    const finish = ({ error = null, timedOut = false } = {}) => {
      if (settled) return;
      settled = true;
      clearTimer(timer);
      let skills = null;
      let result = null;
      for (const line of out.split('\n')) {
        let e;
        try {
          e = JSON.parse(line);
        } catch {
          continue;
        }
        // Skills are listed in `skills`, and as slash commands in older CLIs.
        if (e?.type === 'system' && e.subtype === 'init') skills = Array.isArray(e.skills) ? e.skills : Array.isArray(e.slash_commands) ? e.slash_commands : skills;
        if (e?.type === 'result') result = e;
      }
      if (skills) writeFileSync(`${run.answer}.skills.json`, `${JSON.stringify(skills)}\n`);
      let kind = null;
      let reason = null;
      if (result && !result.is_error && !String(result.subtype ?? '').startsWith('error') && typeof result.result === 'string') {
        writeFileSync(run.answer, result.result);
      } else {
        writeFileSync(run.answer, '');
        if (result) [kind, reason] = ['fail', `the agent ended in ${result.subtype ?? 'an error'}`];
        else if (timedOut) [kind, reason] = ['fail', `no answer within ${timeoutMs} ms`];
        else [kind, reason] = ['error', error ?? 'no result in the stream'];
        writeFileSync(`${run.answer}.${kind}`, `${reason}\n`);
      }
      done({ ok: !kind, kind, reason });
    };
    const timer = setTimer(() => {
      child.kill('SIGKILL');
      finish({ timedOut: true });
    }, timeoutMs);
    child.stdout.on('data', (d) => {
      out += d;
    });
    child.on('error', (e) => finish({ error: e.message }));
    child.on('close', (code) => finish({ error: code ? `exit ${code}` : null }));
  });
}

export function parseArgs(argv) {
  const [command, suite, maybeDir, ...rest] = argv;
  const o = { command, suite, dir: null, out: null, k: null, json: null, repeats: REPEATS, library: DEFAULT_LIBRARY, claudeBin: 'claude', live: false };
  const args = command === 'score' ? rest : [maybeDir, ...rest].filter((a) => a !== undefined);
  if (command === 'score') o.dir = maybeDir;
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    const next = () => {
      if (i + 1 >= args.length) throw new Error(`${a} needs a value`);
      return args[++i];
    };
    const whole = (v, min) => {
      const n = Number(v);
      if (!Number.isInteger(n) || n < min) throw new Error(`${a} must be a whole number of at least ${min}`);
      return n;
    };
    if (a === '--k') o.k = whole(next(), 1);
    else if (a === '--repeats') o.repeats = whole(next(), 1);
    else if (a === '--json') o.json = next();
    else if (a === '--out') o.out = next();
    else if (a === '--library') o.library = next();
    else if (a === '--claude-bin') o.claudeBin = next();
    else if (a === '--live') o.live = true;
    else throw new Error(`unknown argument: ${a}`);
  }
  if (!['score', 'run'].includes(command) || !suite) throw new Error('usage: skill-lift.mjs score <suite.json> <answers dir> | run <suite.json> --out <dir> [--live]');
  if (command === 'score' && !o.dir) throw new Error('score needs an answers dir');
  if (command === 'run' && !o.out) throw new Error('run needs --out <dir>');
  return o;
}

export async function main(argv, { spawn = nodeSpawn, log = console.log } = {}) {
  const o = parseArgs(argv);
  const suite = loadSuite(o.suite);
  if (o.command === 'run') {
    if (!existsSync(join(o.library, suite.skill, 'SKILL.md'))) throw new Error(`the library ${o.library} has no skill ${suite.skill}`);
    const plan = planRun(suite, { out: o.out, repeats: o.repeats, claudeBin: o.claudeBin });
    if (!o.live) {
      log(`dry run: ${plan.length} paid Claude calls planned (2 arms x ${suite.cases.length} cases x ${o.repeats} attempts), none made. Add --live to run them.`);
      return { plan };
    }
    // A live run starts clean: answers from an earlier run are never graded.
    for (const arm of ARMS) rmSync(join(o.out, arm), { recursive: true, force: true });
    const scratch = Object.fromEntries(ARMS.map((arm) => [arm, armScratch(arm, suite.skill, o.library)]));
    try {
      for (const run of plan) {
        const r = await runOne(run, scratch[run.arm], { spawn });
        log(`${run.arm} ${run.id} #${run.attempt}: ${r.ok ? 'answered' : `${r.kind === 'fail' ? 'agent error, a fail' : 'failed, left out'} (${r.reason})`}`);
      }
    } finally {
      for (const dir of Object.values(scratch)) rmSync(dir, { recursive: true, force: true });
    }
  }
  const report = liftReport(suite, o.command === 'run' ? o.out : o.dir, { k: o.k });
  if (o.json) writeFileSync(o.json, `${JSON.stringify(report, null, 2)}\n`);
  log(renderLift(report));
  return report;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
