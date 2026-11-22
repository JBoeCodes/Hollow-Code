// BI1: skill routing eval. Does the right skill fire, per agent, with
// confusable near misses as negatives (idea from beancount-io's
// run-routing.py; method only, no code copied).
//
//   node scripts/agent-eval/skill-routing.mjs score <cases.json> <streams dir>
//        [--agent claude|codex]... [--library <skills dir>] [--json <file>]
//   node scripts/agent-eval/skill-routing.mjs run <cases.json> --out <dir>
//        [--agent claude|codex]... [--library <skills dir>] [--limit <n>]
//        [--claude-bin <path>] [--codex-bin <path>] [--live]
//
// A case is a prompt, the one skill that should fire (`expect`, a skill's
// name or folder in the library; null when none should) and the skills it is
// easily confused with (`confusable`). `fixtures/skill-routing/cases.json` is
// a set over the shared library `~/.agents/skills`.
//
// The choice is the FIRST skill the agent reaches for, read from its recorded
// stream: Claude (`claude -p --output-format stream-json`) a `Skill` call, a
// `Read` of a `<skill>/SKILL.md`, `hollow_vault read_skill` or `run_action
// skills.read_skill`; Codex (`codex exec --json`) the first shell command that
// names a `<skill>/SKILL.md`, or the same two MCP reads. The two agents load
// a skill differently, so each is scored on its own. A case with no stream
// for an agent is `notRun`, never a miss.
//
// `score` is free and deterministic (the tests run it on recorded fixtures).
// `run` spawns real agent CLIs, one paid call per case and agent, in a scratch
// folder with file edits disallowed (Claude `--disallowedTools`, Codex
// `--sandbox read-only`), and stops each run as soon as its choice is seen.
// Without `--live` it only prints the plan. It never runs in the gate or CI.
//
// Privacy: a report holds case ids, skill names and counts only, never a
// prompt's or a stream's text.
import { spawn as nodeSpawn } from 'node:child_process';
import { createReadStream, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync, createWriteStream } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { wilson } from './score.mjs';

export const AGENTS = ['claude', 'codex'];
export const CASES_SCHEMA = 1;
export const DEFAULT_LIBRARY = join(homedir(), '.agents', 'skills');
/** Claude may not change files in the scratch folder. */
export const CLAUDE_DISALLOWED = ['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'Bash'];
/** A routing run needs only its first move; this caps a run that wanders. */
export const MAX_TURNS = 3;
const ID = /^[a-z0-9][a-z0-9-]{0,63}$/;
/** A skill's SKILL.md: the folder right after a `skills/` segment, so a
 *  shell variable (`$d/SKILL.md`) or another file named SKILL.md is no choice. */
const SKILL_MD = /(?:^|[^\w.@+-])skills\/([\w.@+-]+)\/SKILL\.md(?![\w.])/;
/** A run that has not chosen by then is stopped and counted as an error. */
export const RUN_TIMEOUT_MS = 180_000;

// ── Cases ────────────────────────────────────────────────────────────────

/** A checked case set: `{version, cases: [{id, prompt, expect, confusable}]}`.
 *  Throws with every problem listed. */
export function validateCases(input) {
  const errors = [];
  if (input?.version !== CASES_SCHEMA) errors.push(`version must be ${CASES_SCHEMA}`);
  const cases = Array.isArray(input?.cases) ? input.cases : (errors.push('cases must be an array'), []);
  const seen = new Set();
  for (const [i, c] of cases.entries()) {
    const at = `cases[${i}]${typeof c?.id === 'string' ? ` (${c.id})` : ''}`;
    if (typeof c?.id !== 'string' || !ID.test(c.id)) errors.push(`${at}: id must match ${ID}`);
    else if (seen.has(c.id)) errors.push(`${at}: duplicate id`);
    else seen.add(c.id);
    if (typeof c?.prompt !== 'string' || !c.prompt.trim()) errors.push(`${at}: prompt is required`);
    if (c?.expect !== null && (typeof c?.expect !== 'string' || !c.expect)) errors.push(`${at}: expect is a skill name or null`);
    const conf = c?.confusable ?? [];
    if (!Array.isArray(conf) || conf.some((s) => typeof s !== 'string' || !s)) errors.push(`${at}: confusable is a list of skill names`);
    else if (conf.includes(c?.expect)) errors.push(`${at}: confusable names the expected skill`);
  }
  if (!cases.length && Array.isArray(input?.cases)) errors.push('cases is empty');
  if (errors.length) throw new Error(`skill-routing cases:\n- ${errors.join('\n- ')}`);
  return cases.map((c) => ({ id: c.id, prompt: c.prompt, expect: c.expect, confusable: c.confusable ?? [] }));
}

export function loadCases(file) {
  return validateCases(JSON.parse(readFileSync(file, 'utf8')));
}

/** The library's skills: folder → frontmatter name (the folder when it has
 *  none). Only `<dir>/<folder>/SKILL.md`, the first 16 KB of each. */
export function libraryNames(dir) {
  const names = new Map();
  let folders = [];
  try {
    folders = readdirSync(dir);
  } catch {
    return names;
  }
  for (const folder of folders.sort()) {
    const file = join(dir, folder, 'SKILL.md');
    let head = '';
    try {
      head = readFileSync(file, 'utf8').slice(0, 16 * 1024);
    } catch {
      continue;
    }
    const front = /^---\r?\n([\s\S]*?)\r?\n---/.exec(head)?.[1] ?? '';
    const name = /^name:\s*["']?([^"'\r\n]+?)["']?\s*$/m.exec(front)?.[1] ?? folder;
    names.set(folder, name);
  }
  return names;
}

/** Skills the cases name that the library lacks (expected or confusable). */
export function unknownSkills(cases, names) {
  const known = new Set([...names.keys(), ...names.values()]);
  const named = cases.flatMap((c) => [c.expect, ...c.confusable]).filter((s) => s != null);
  return [...new Set(named)].filter((s) => !known.has(s)).sort();
}

/** A raw choice as the library names it: a plugin prefix (`plugin:skill`)
 *  dropped, a folder mapped to its frontmatter name. */
export function resolveSkill(raw, names = new Map()) {
  if (raw == null) return null;
  const bare = String(raw).split(':').pop();
  return names.get(bare) ?? bare;
}

// ── Choice detection ─────────────────────────────────────────────────────

const skillOfPath = (path) => (typeof path === 'string' ? SKILL_MD.exec(path)?.[1] ?? null : null);
const toolOf = (name) => /^mcp__[\w-]+__(\w+)$/.exec(name ?? '')?.[1] ?? name;

/** The skill an MCP-style read names: `read_skill {name}` or `run_action
 *  skills.read_skill {args.path}`. */
function mcpRead(tool, input) {
  if (!input || typeof input !== 'object') return null;
  if (tool === 'read_skill') return typeof input.name === 'string' ? input.name : skillOfPath(input.path);
  if (tool === 'run_action' && input.id === 'skills.read_skill') return skillOfPath(input.args?.path);
  return null;
}

/** The skill one Claude tool use reaches for, or null. */
function claudeUse(b) {
  const input = b.input && typeof b.input === 'object' ? b.input : {};
  if (b.name === 'Skill') return typeof input.skill === 'string' ? input.skill : typeof input.name === 'string' ? input.name : null;
  if (b.name === 'Read') return skillOfPath(input.file_path);
  if (b.name === 'Bash') return skillOfPath(input.command);
  return mcpRead(toolOf(b.name), input);
}

/** The first skill one parsed stream event reaches for, or null.
 *  `{skill, via}`: `via` is the tool (`Skill`, `Read`, `shell`, `read_skill`,
 *  `run_action`). */
export function choiceOf(agent, event) {
  if (agent === 'claude') {
    if (event?.type !== 'assistant' || !Array.isArray(event.message?.content)) return null;
    for (const b of event.message.content) {
      if (b?.type !== 'tool_use') continue;
      const skill = claudeUse(b);
      if (skill) return { skill, via: toolOf(b.name) };
    }
    return null;
  }
  if (agent === 'codex') {
    if (event?.type !== 'item.started' && event?.type !== 'item.completed') return null;
    const item = event.item ?? {};
    if (item.type === 'command_execution') {
      const skill = skillOfPath(item.command);
      return skill ? { skill, via: 'shell' } : null;
    }
    if (item.type === 'mcp_tool_call') {
      let args = item.arguments;
      if (typeof args === 'string') {
        try {
          args = JSON.parse(args);
        } catch {
          args = null;
        }
      }
      const skill = mcpRead(item.tool, args);
      return skill ? { skill, via: item.tool } : null;
    }
    return null;
  }
  throw new Error(`unknown agent: ${agent}`);
}

/** Stream lines worth parsing: a cheap substring filter, as trace-assert does. */
const worthParsing = (agent, line) => (agent === 'claude' ? line.includes('"tool_use"') : line.includes('"item.'));

/** How a stream with no choice ended: `done` when the agent finished its
 *  turn normally (it chose no skill), `failed` when it ended in an error, else
 *  null (no end seen: crashed, stopped or cut off). Claude's `result` line is
 *  `failed` when `is_error` or an `error_*` subtype, except `error_max_turns`:
 *  the agent worked for MAX_TURNS turns without reaching a skill, which is a
 *  routing answer. Codex: `turn.completed` is done, `turn.failed` or `error`
 *  failed. */
export function endOf(agent, event) {
  if (agent === 'claude') {
    if (event?.type !== 'result') return null;
    if (event.subtype === 'error_max_turns') return 'done';
    return event.is_error === true || String(event.subtype ?? '').startsWith('error') ? 'failed' : 'done';
  }
  if (event?.type === 'turn.completed') return 'done';
  if (event?.type === 'turn.failed' || event?.type === 'error') return 'failed';
  return null;
}

const ENDS = { claude: '"result"', codex: '"turn.' };

/** A recorded stream's first choice: `{choice}` (a `{skill, via}` or null for
 *  none), or `{failed: reason}` when it chose nothing and did not end
 *  normally (an empty stream, an error end, no end at all). */
export async function streamChoice(agent, file) {
  const rl = createInterface({ input: createReadStream(file, 'utf8'), crlfDelay: Infinity });
  let end = null;
  let lines = 0;
  try {
    for await (const line of rl) {
      if (!line.trim()) continue;
      lines += 1;
      const parsing = worthParsing(agent, line);
      if (!parsing && !line.includes(ENDS[agent]) && !line.includes('"error"')) continue;
      let event;
      try {
        event = JSON.parse(line);
      } catch {
        continue;
      }
      const choice = parsing ? choiceOf(agent, event) : null;
      if (choice) return { choice };
      end = endOf(agent, event) ?? end;
    }
  } finally {
    rl.close();
  }
  if (end === 'done') return { choice: null };
  return { failed: !lines ? 'empty stream' : end === 'failed' ? 'the agent ended in an error' : 'the stream has no end' };
}

// ── Scoring ──────────────────────────────────────────────────────────────

/** One agent's routing over the cases. `choices` maps a case id to its
 *  choice (`{skill}` or null for none) or `{failed: reason}`; a missing id
 *  is not run. A failed or unrun case is outside the accuracy. */
export function scoreAgent(agent, cases, choices, names = new Map()) {
  const rows = [];
  for (const c of cases) {
    if (!choices.has(c.id)) {
      rows.push({ id: c.id, expect: c.expect, chose: null, outcome: 'notRun' });
      continue;
    }
    const got = choices.get(c.id);
    if (got?.failed) {
      rows.push({ id: c.id, expect: c.expect, chose: null, outcome: 'error', reason: got.failed });
      continue;
    }
    const chose = resolveSkill(got?.skill ?? null, names);
    const expect = resolveSkill(c.expect, names);
    let outcome;
    if (chose === expect) outcome = 'correct';
    else if (expect == null) outcome = 'falseFire';
    else if (chose == null) outcome = 'missed';
    else if (c.confusable.map((s) => resolveSkill(s, names)).includes(chose)) outcome = 'confused';
    else outcome = 'wrong';
    rows.push({ id: c.id, expect, chose, outcome });
  }
  const ran = rows.filter((r) => r.outcome !== 'notRun' && r.outcome !== 'error');
  const correct = ran.filter((r) => r.outcome === 'correct').length;
  const count = (o, from = ran) => from.filter((r) => r.outcome === o).length;
  // Each confusable pair that failed: expected skill (or none) → the near
  // miss chosen. A none-expected case that fired one of its near misses is a
  // false fire and a failed pair too.
  const pairs = new Map();
  const nearMiss = (r) => r.outcome === 'confused' || (r.outcome === 'falseFire' && cases.find((c) => c.id === r.id).confusable.map((s) => resolveSkill(s, names)).includes(r.chose));
  for (const r of ran.filter(nearMiss)) {
    const expect = r.expect ?? '(none)';
    const key = `${expect} -> ${r.chose}`;
    const p = pairs.get(key) ?? { expect, chose: r.chose, cases: [] };
    p.cases.push(r.id);
    pairs.set(key, p);
  }
  const perSkill = {};
  for (const r of ran) {
    const key = r.expect ?? '(none)';
    perSkill[key] ??= { cases: 0, correct: 0 };
    perSkill[key].cases += 1;
    if (r.outcome === 'correct') perSkill[key].correct += 1;
  }
  return {
    agent,
    cases: cases.length,
    ran: ran.length,
    notRun: count('notRun', rows),
    errors: count('error', rows),
    correct,
    accuracy: ran.length ? Math.round((correct / ran.length) * 1e4) / 1e4 : null,
    interval: wilson(correct, ran.length),
    confused: count('confused'),
    wrong: count('wrong'),
    missed: count('missed'),
    falseFire: count('falseFire'),
    confusedPairs: [...pairs.values()].sort((a, b) => b.cases.length - a.cases.length || a.expect.localeCompare(b.expect)),
    perSkill,
    rows,
  };
}

/** Stream file of a case: `<dir>/<agent>/<id>.jsonl`. */
export const streamPath = (dir, agent, id) => join(dir, agent, `${id}.jsonl`);

/** Score every agent with streams in `dir`. */
export async function scoreDir(cases, dir, { agents = AGENTS, names = new Map() } = {}) {
  const report = { eval: 'skill-routing', cases: cases.length, agents: [] };
  for (const agent of agents) {
    const choices = new Map();
    for (const c of cases) {
      const file = streamPath(dir, agent, c.id);
      if (!existsSync(file) || !statSync(file).isFile()) continue;
      const got = await streamChoice(agent, file);
      // A run the live runner saw fail with no choice (spawn error, timeout,
      // non-zero exit) left a note; it counts as failed even if its stream
      // looks finished.
      const note = `${file}.error`;
      if (got.failed) choices.set(c.id, got);
      else if (!got.choice && existsSync(note)) choices.set(c.id, { failed: readFileSync(note, 'utf8').trim() || 'the run failed' });
      else choices.set(c.id, got.choice);
    }
    report.agents.push(scoreAgent(agent, cases, choices, names));
  }
  return report;
}

const pct = (x) => (x == null ? 'n/a' : `${Math.round(x * 1000) / 10}%`);

export function renderRouting(report) {
  const lines = [`Skill routing: ${report.cases} cases`];
  for (const a of report.agents) {
    if (!a.ran) {
      lines.push(`- ${a.agent}: ${a.errors ? `${a.errors} failed runs, none scored` : 'not run'}`);
      continue;
    }
    const ci = a.interval ? ` (95% ${pct(a.interval.lo)} to ${pct(a.interval.hi)})` : '';
    lines.push(`- ${a.agent}: ${a.correct}/${a.ran} routed right, ${pct(a.accuracy)}${ci}; confused ${a.confused}, wrong ${a.wrong}, missed ${a.missed}, fired with none expected ${a.falseFire}${a.errors ? `; ${a.errors} failed runs (left out)` : ''}${a.notRun ? `; ${a.notRun} not run` : ''}`);
    for (const p of a.confusedPairs) lines.push(`  confusable pair failed: ${p.expect} -> ${p.chose} (${p.cases.join(', ')})`);
  }
  return lines.join('\n');
}

// ── Live run (paid; never in the gate) ───────────────────────────────────

/** The command for one case. Arguments are argv entries (no shell). */
export function commandFor(agent, prompt, { claudeBin = 'claude', codexBin = 'codex' } = {}) {
  if (agent === 'claude') {
    // No MCP server (none is given to --strict-mcp-config) and default
    // permissions, so nothing outside the disallowed list can act unasked.
    return { bin: claudeBin, args: ['-p', prompt, '--output-format', 'stream-json', '--verbose', '--max-turns', String(MAX_TURNS), '--strict-mcp-config', '--permission-mode', 'default', '--disallowedTools', CLAUDE_DISALLOWED.join(' ')] };
  }
  if (agent === 'codex') {
    // The read-only sandbox covers shell commands, not MCP servers: without
    // the user config no MCP server or memory runs (auth is kept).
    return { bin: codexBin, args: ['exec', '--skip-git-repo-check', '--ephemeral', '--ignore-user-config', '--ignore-rules', '--json', '--color', 'never', '--sandbox', 'read-only', prompt] };
  }
  throw new Error(`unknown agent: ${agent}`);
}

/** Every planned run: one per case and agent, at most `limit` cases. */
export function planRun(cases, { agents = AGENTS, out, limit = Infinity, claudeBin, codexBin } = {}) {
  return agents.flatMap((agent) =>
    cases.slice(0, limit).map((c) => ({ agent, id: c.id, stream: streamPath(out, agent, c.id), ...commandFor(agent, c.prompt, { claudeBin, codexBin }) })),
  );
}

/** A scratch folder whose `.claude/skills` links each library skill (Claude
 *  finds skills there); Codex reads `~/.agents/skills` itself, and a library
 *  elsewhere is linked as the folder's `.agents/skills`. */
export function makeScratch(library, names, { base = tmpdir(), shared = DEFAULT_LIBRARY } = {}) {
  const dir = mkdtempSync(join(base, 'skill-routing-'));
  const link = (sub) => {
    const to = join(dir, sub);
    mkdirSync(to, { recursive: true });
    for (const folder of names.keys()) symlinkSync(join(library, folder), join(to, folder));
  };
  link('.claude/skills');
  if (resolve(library) !== resolve(shared)) link('.agents/skills');
  return dir;
}

/** Run one planned case; resolves with `{choice, error}`. Stops the agent
 *  once a choice is seen, or after `timeoutMs` with no choice. A run that
 *  failed with no choice (spawn error, timeout, non-zero exit) leaves
 *  `<stream>.error`, so scoring leaves it out. `spawn` and `setTimer` are
 *  injectable (the tests pass fakes). */
export function runOne(run, cwd, { spawn = nodeSpawn, timeoutMs = RUN_TIMEOUT_MS, setTimer = setTimeout, clearTimer = clearTimeout } = {}) {
  mkdirSync(join(run.stream, '..'), { recursive: true });
  rmSync(`${run.stream}.error`, { force: true });
  const out = createWriteStream(run.stream);
  return new Promise((done) => {
    const child = spawn(run.bin, run.args, { cwd, stdio: ['ignore', 'pipe', 'ignore'] });
    let choice = null;
    let settled = false;
    let timedOut = false;
    const timer = setTimer(() => {
      timedOut = true;
      child.kill('SIGTERM');
    }, timeoutMs);
    const finish = (error = null) => {
      if (settled) return;
      settled = true;
      clearTimer(timer);
      if (!choice && error) writeFileSync(`${run.stream}.error`, `${error}\n`);
      out.end(() => done({ choice, error: choice ? null : error }));
    };
    const rl = createInterface({ input: child.stdout, crlfDelay: Infinity });
    rl.on('line', (line) => {
      // Past the choice the run is being stopped: nothing more is kept.
      if (choice) return;
      out.write(`${line}\n`);
      if (!worthParsing(run.agent, line)) return;
      try {
        choice = choiceOf(run.agent, JSON.parse(line));
      } catch {
        return;
      }
      if (choice) child.kill('SIGTERM');
    });
    child.on('error', (e) => finish(e.message));
    child.on('close', (code) => finish(timedOut ? `no choice within ${timeoutMs} ms` : code ? `exit ${code}` : null));
  });
}

export function parseArgs(argv) {
  const [command, cases, dir, ...rest] = argv;
  const o = { command, cases, dir: command === 'score' ? dir : null, agents: [], library: DEFAULT_LIBRARY, json: null, out: null, limit: Infinity, live: false, claudeBin: 'claude', codexBin: 'codex' };
  const args = command === 'score' ? rest : [dir, ...rest].filter((a) => a !== undefined);
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    const next = () => {
      if (i + 1 >= args.length) throw new Error(`${a} needs a value`);
      return args[++i];
    };
    if (a === '--agent') {
      const v = next();
      if (!AGENTS.includes(v)) throw new Error(`--agent must be one of ${AGENTS.join(', ')}`);
      o.agents.push(v);
    } else if (a === '--library') o.library = next();
    else if (a === '--json') o.json = next();
    else if (a === '--out') o.out = next();
    else if (a === '--limit') {
      o.limit = Number(next());
      if (!Number.isInteger(o.limit) || o.limit < 1) throw new Error('--limit must be a positive whole number');
    } else if (a === '--claude-bin') o.claudeBin = next();
    else if (a === '--codex-bin') o.codexBin = next();
    else if (a === '--live') o.live = true;
    else throw new Error(`unknown argument: ${a}`);
  }
  if (!['score', 'run'].includes(command) || !cases) throw new Error('usage: skill-routing.mjs score <cases.json> <streams dir> | run <cases.json> --out <dir> [--live]');
  if (command === 'score' && !o.dir) throw new Error('score needs a streams dir');
  if (command === 'run' && !o.out) throw new Error('run needs --out <dir>');
  if (!o.agents.length) o.agents = [...AGENTS];
  return o;
}

export async function main(argv, { spawn = nodeSpawn, log = console.log } = {}) {
  const o = parseArgs(argv);
  const cases = loadCases(o.cases);
  const names = libraryNames(o.library);
  if (o.command === 'run') {
    const unknown = unknownSkills(cases, names);
    if (unknown.length) throw new Error(`the library ${o.library} has no skill named: ${unknown.join(', ')}`);
    const plan = planRun(cases, { agents: o.agents, out: o.out, limit: o.limit, claudeBin: o.claudeBin, codexBin: o.codexBin });
    if (!o.live) {
      log(`dry run: ${plan.length} paid agent calls planned (${o.agents.join(', ')} x ${Math.min(cases.length, o.limit)} cases), none made. Add --live to run them.`);
      return { plan };
    }
    const scratch = makeScratch(o.library, names);
    try {
      for (const run of plan) {
        const r = await runOne(run, scratch, { spawn });
        log(`${run.agent} ${run.id}: ${r.error ? `error (${r.error})` : resolveSkill(r.choice?.skill ?? null, names) ?? 'no skill'}`);
      }
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
  const dir = o.command === 'run' ? o.out : o.dir;
  const report = await scoreDir(o.command === 'run' ? cases.slice(0, o.limit) : cases, dir, { agents: o.agents, names });
  if (o.json) writeFileSync(o.json, `${JSON.stringify(report, null, 2)}\n`);
  log(renderRouting(report));
  return report;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(e.message);
    process.exit(1);
  });
}
