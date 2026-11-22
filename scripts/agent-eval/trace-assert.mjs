// NV5: deterministic assertions over a recorded tool-call sequence. The
// scorecard scores the artifact and the cost; this scores the path:
//
//   node scripts/agent-eval/trace-assert.mjs <stream.jsonl>... [--json <file>]
//        [--skill <name>]... [--exempt <action id>]...
//   node scripts/agent-eval/scorecard.mjs trace-assert <stream.jsonl>... (same flags)
//
// A stream is a Claude transcript (`~/.claude/projects/*/<session>.jsonl`) or
// a `claude -p --output-format stream-json` capture: both put each tool use in
// an assistant line's `message.content` and its result in a later user line's
// `tool_result` part. The file is read the way scorecard.mjs's readCalls reads
// one (line by line, a cheap substring filter before the parse, `parse`
// injectable, one tool use kept once by its id however many lines repeat it)
// but it needs no `usage` block, so a bare stream-json capture reads too.
//
// Four checks, each `pass`, `fail` or `na` (not applicable):
// - `skill-never-read`: a skill the run named (a `Skill` call, a read of a
//   file inside `skills/<name>/`, or `--skill <name>`, the skill an eval row
//   expects) whose SKILL.md was never loaded before its other files were used.
//   `na` when no skill is named.
// - `run-action-without-describe`: a `run_action` of an id with no earlier
//   successful `describe_action` of the same id (the path is list_actions,
//   describe_action, run_action). `na` when the run never calls `run_action`.
//   `--exempt <id>` skips an id the agent may know (sessions.wait).
// - `errors-recovered`: each tool error is followed, within RECOVERY_WINDOW
//   calls, by a call that succeeded (a retry or another call). `na` when there
//   is no error.
// - `wasteful-calls`: the share of eligible calls that repeat an earlier
//   identical call whose result was fine, with nothing that changes state in
//   between. `na` under MIN_ELIGIBLE eligible calls; fails above WASTE_MAX.
//
// A summary over several streams counts `na` apart from `pass` and `fail`: a
// check that did not apply to a run is never a zero in its share.
//
// Privacy, as in scorecard.mjs: only tool names, their ids, the skill names and
// action ids, an input's hash (never kept or printed) and each error's class
// (errorClass over RESULT_PREFIX characters, then dropped) are read. No prompt,
// text part, thinking or result text reaches a report.
import { createHash } from 'node:crypto';
import { createReadStream, existsSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { basename, join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';
import { RESULT_PREFIX, appTool, commandHead, errorClass, isPoll, unwrapShell } from './scorecard.mjs';

export const CHECKS = ['skill-never-read', 'run-action-without-describe', 'errors-recovered', 'wasteful-calls'];
/** The two flags an eval round reports (the NV5 done-when). */
export const FLAGS = ['skill-never-read', 'run-action-without-describe'];
export const RECOVERY_WINDOW = 3;
export const MIN_ELIGIBLE = 5;
export const WASTE_MAX = 0.2;
/** Tools whose repeat can change what the next identical call returns. */
const MUTATING = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit', 'Bash', 'BashOutput', 'TaskOutput', 'run_action', 'start_session', 'send_to_session', 'manage_session']);
/** Tools a repeat of which is never counted as waste: they act or wait. */
const NEVER_WASTE = new Set(['Bash', 'BashOutput', 'TaskOutput', 'run_action', 'start_session', 'send_to_session', 'manage_session', 'stop_session', 'wait_for_session', 'Skill']);
/** Commands that read a file (a SKILL.md read through the shell). */
const READERS = new Set(['cat', 'sed', 'head', 'tail', 'bat', 'less', 'more', 'awk', 'nl']);
const SKILL_FILE = /(?:^|[\s'"=:(])((?:[^\s'"]*\/)?([\w.@+-]+)\/(SKILL\.md))(?=$|[\s'"):;|&])/;
const SKILL_DIR_FILE = /\/skills\/([\w.@+-]+)\/(?!SKILL\.md(?:$|[\s'"]))[^\s'"]+/;
const SKILL_DIR_FILE_G = new RegExp(SKILL_DIR_FILE.source, 'g');

const text = (content) => {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content.filter((p) => p?.type === 'text' && typeof p.text === 'string').map((p) => p.text).join('\n');
};
const hashOf = (name, input) => createHash('sha1').update(name).update('\0').update(JSON.stringify(input ?? null)).digest('hex');
/** A tool's own name: `mcp__<server>__<tool>` → `<tool>`, else the name. */
const toolOf = (name) => /^mcp__[\w-]+__(\w+)$/.exec(name ?? '')?.[1] ?? name ?? 'unknown';

/** One stream's calls in order: { i, id, name, tool, hash, error, errorClass,
 *  poll, action, skill, readSkill, usesSkillFile }. `error` is true, false, or
 *  null while no result was recorded. `parse` is JSON.parse; a test wraps it. */
export async function readTrace(file, { parse = JSON.parse } = {}) {
  const calls = [];
  const byId = new Map();
  const rl = createInterface({ input: createReadStream(file, 'utf8'), crlfDelay: Infinity });
  for await (const line of rl) {
    const asst = line.includes('"tool_use"');
    const user = line.includes('"tool_result"');
    if (!asst && !user) continue;
    let e;
    try {
      e = parse(line);
    } catch {
      continue;
    }
    const parts = e?.message?.content;
    if (!Array.isArray(parts)) continue;
    if (e.type === 'assistant') {
      for (const b of parts) {
        if (b?.type !== 'tool_use' || byId.has(b.id)) continue;
        const call = callOf(b, calls.length);
        byId.set(b.id, call);
        calls.push(call);
      }
    } else if (e.type === 'user') {
      for (const p of parts) {
        const call = p?.type === 'tool_result' ? byId.get(p.tool_use_id) : null;
        if (!call) continue;
        call.error = p.is_error === true;
        if (call.error) call.errorClass = errorClass(text(p.content).slice(0, RESULT_PREFIX));
      }
    }
  }
  return calls;
}

/** What one tool use keeps. Paths, commands and ids are classified here and
 *  dropped, except the skill names and action ids the report names. */
function callOf(b, i) {
  const name = typeof b.name === 'string' ? b.name : 'unknown';
  const input = b.input && typeof b.input === 'object' ? b.input : {};
  const tool = toolOf(name);
  const call = { i, id: b.id, name, tool, hash: hashOf(name, input), error: null, errorClass: null, poll: false, action: null, skill: null, readSkill: null, usesSkillFile: null, readsOnly: false };
  const command = name === 'Bash' && typeof input.command === 'string' ? input.command : null;
  call.poll = isPoll(name, command) || (appTool(name) === 'run_action' && input.id === 'sessions.wait');
  const path = typeof input.file_path === 'string' ? input.file_path : typeof input.path === 'string' ? input.path : null;
  if (appTool(name) === 'describe_action' || appTool(name) === 'run_action') call.action = typeof input.id === 'string' ? input.id : null;
  if (name === 'Skill') call.skill = typeof input.skill === 'string' ? input.skill : typeof input.name === 'string' ? input.name : null;
  // A SKILL.md read: Read, hollow_vault read_skill, skills.read_skill, or a shell reader.
  const skillPath = name === 'Read' || tool === 'read_skill' ? path : appTool(name) === 'run_action' && input.id === 'skills.read_skill' ? input.args?.path : null;
  if (typeof skillPath === 'string') {
    const m = /(?:^|\/)([\w.@+-]+)\/SKILL\.md$/.exec(skillPath);
    if (m) call.readSkill = m[1];
    else if (name === 'Read') call.usesSkillFile = SKILL_DIR_FILE.exec(skillPath)?.[1] ?? null;
  } else if (command != null) {
    const shell = unwrapShell(command);
    const m = READERS.has(commandHead(command) ?? '') ? SKILL_FILE.exec(shell) : null;
    if (m) call.readSkill = m[2];
    else call.usesSkillFile = [...shell.matchAll(SKILL_DIR_FILE_G)][0]?.[1] ?? null;
  } else if (name === 'Grep' || name === 'Glob') {
    const dir = typeof input.path === 'string' ? input.path : typeof input.pattern === 'string' ? input.pattern : '';
    call.usesSkillFile = SKILL_DIR_FILE.exec(dir)?.[1] ?? null;
  }
  return call;
}

const verdict = (status, detail = {}) => ({ status, ...detail });

/** skill-never-read. */
function skillCheck(calls, expected) {
  const named = new Map(); // skill → { firstUse: index|null, loaded: index|null }
  const entry = (s) => named.get(s) ?? named.set(s, { firstUse: null, loaded: null }).get(s);
  for (const s of expected) entry(s);
  for (const c of calls) {
    if (c.skill) {
      // The Skill tool loads the SKILL.md itself, so a call that failed loaded nothing.
      const s = entry(c.skill);
      if (c.error !== true) s.loaded ??= c.i;
      else s.firstUse ??= c.i;
    }
    if (c.readSkill && c.error !== true) entry(c.readSkill).loaded ??= c.i;
    if (c.usesSkillFile) entry(c.usesSkillFile).firstUse ??= c.i;
  }
  if (!named.size) return verdict('na');
  const bad = [...named]
    .map(([skill, s]) => ({ skill, reason: s.loaded == null ? 'never' : s.firstUse != null && s.firstUse < s.loaded ? 'after-use' : null }))
    .filter((r) => r.reason);
  return verdict(bad.length ? 'fail' : 'pass', { skills: [...named.keys()], offenders: bad });
}

/** run-action-without-describe. */
function describeCheck(calls, exempt) {
  const runs = calls.filter((c) => appTool(c.name) === 'run_action');
  if (!runs.length) return verdict('na');
  const described = new Set();
  const offenders = [];
  for (const c of calls) {
    if (appTool(c.name) === 'describe_action' && c.action && c.error !== true) described.add(c.action);
    else if (appTool(c.name) === 'run_action' && c.action && !described.has(c.action) && !exempt.has(c.action)) offenders.push({ action: c.action, at: c.i });
  }
  return verdict(offenders.length ? 'fail' : 'pass', { runs: runs.length, offenders });
}

/** errors-recovered. */
function recoveryCheck(calls) {
  const errors = calls.filter((c) => c.error === true);
  if (!errors.length) return verdict('na');
  const unrecovered = errors
    .filter((c) => !calls.slice(c.i + 1, c.i + 1 + RECOVERY_WINDOW).some((n) => n.error === false))
    .map((c) => ({ tool: c.tool, class: c.errorClass, at: c.i }));
  return verdict(unrecovered.length ? 'fail' : 'pass', { errors: errors.length, recovered: errors.length - unrecovered.length, unrecovered });
}

/** wasteful-calls. */
function wasteCheck(calls) {
  const eligible = calls.filter((c) => !NEVER_WASTE.has(c.tool) && !c.poll);
  if (eligible.length < MIN_ELIGIBLE) return verdict('na', { eligible: eligible.length });
  const pool = new Set(eligible);
  const seen = new Map(); // hash → index of the last call with that input
  const repeats = [];
  for (const c of calls) {
    if (!pool.has(c)) continue;
    const prev = seen.get(c.hash);
    if (prev != null && calls[prev].error !== true && !calls.slice(prev + 1, c.i).some((m) => MUTATING.has(m.tool))) repeats.push({ tool: c.tool, at: c.i });
    seen.set(c.hash, c.i);
  }
  const share = repeats.length / eligible.length;
  return verdict(share > WASTE_MAX ? 'fail' : 'pass', { eligible: eligible.length, wasteful: repeats.length, share, repeats });
}

/** The four checks over one trace (an array from readTrace). */
export function assertTrace(calls, { skills = [], exempt = [] } = {}) {
  return {
    calls: calls.length,
    checks: {
      'skill-never-read': skillCheck(calls, skills),
      'run-action-without-describe': describeCheck(calls, new Set(exempt)),
      'errors-recovered': recoveryCheck(calls),
      'wasteful-calls': wasteCheck(calls),
    },
  };
}

/** Per check over many results: counts, and `failShare` = fail / (pass + fail),
 *  null when no run applied. A `na` run is in `na` only; it never dilutes the share. */
export function summarizeTraces(results) {
  const out = {};
  for (const check of CHECKS) {
    const n = { pass: 0, fail: 0, na: 0 };
    for (const r of results) n[r.checks[check].status]++;
    const applicable = n.pass + n.fail;
    out[check] = { ...n, applicable, failShare: applicable ? n.fail / applicable : null };
  }
  return out;
}

/** The lines an eval round prints for the two flags, then the other checks. */
export function renderFlags(results, summary = summarizeTraces(results.map((r) => r.result))) {
  const lines = [];
  for (const { file, result } of results) {
    const c = result.checks;
    for (const o of c['skill-never-read'].offenders ?? []) lines.push(`${file}: skill never read: ${o.skill}${o.reason === 'after-use' ? ' (its files were used before SKILL.md)' : ''}`);
    for (const o of c['run-action-without-describe'].offenders ?? []) lines.push(`${file}: run_action called with no describe_action first: ${o.action}`);
    for (const o of c['errors-recovered'].unrecovered ?? []) lines.push(`${file}: tool error not recovered: ${o.tool} (${o.class})`);
    const w = c['wasteful-calls'];
    if (w.status === 'fail') lines.push(`${file}: wasteful calls: ${w.wasteful} of ${w.eligible} repeat an earlier identical call`);
  }
  lines.push('');
  for (const check of CHECKS) {
    const s = summary[check];
    lines.push(`${check}: ${s.fail} fail, ${s.pass} pass, ${s.na} not applicable${s.failShare == null ? ' (no run applied)' : ` (fail share ${(s.failShare * 100).toFixed(0)}% of ${s.applicable})`}`);
  }
  return lines.join('\n');
}

/** Every `.jsonl` under a path (a file, or a folder's files one level down and
 *  its `subagents/`), sorted. */
export function streamFiles(path) {
  if (!existsSync(path)) return [];
  if (statSync(path).isFile()) return [path];
  return readdirSync(path, { withFileTypes: true }).flatMap((e) => (e.isFile() && e.name.endsWith('.jsonl') ? [join(path, e.name)] : e.isDirectory() ? streamFiles(join(path, e.name)) : [])).sort();
}

export async function traceFiles(files, opts = {}) {
  const results = [];
  for (const file of files) results.push({ file: basename(file), result: assertTrace(await readTrace(file, opts), opts) });
  return results;
}

const USAGE = `usage: trace-assert.mjs <stream.jsonl | folder>... [--json <file>] [--skill <name>]... [--exempt <action id>]...`;

export function parseTraceArgs(argv) {
  const o = { files: [], skills: [], exempt: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v == null || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--json') o.json = val();
    else if (a === '--skill') o.skills.push(val());
    else if (a === '--exempt') o.exempt.push(val());
    else if (a.startsWith('--')) throw new Error(`unknown option ${a}`);
    else o.files.push(a);
  }
  return o;
}

export async function main(argv) {
  let o;
  try {
    o = parseTraceArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  const files = o.files.flatMap((f) => streamFiles(resolve(f)));
  if (!files.length) {
    console.error(`no stream found\n${USAGE}`);
    return 2;
  }
  const results = await traceFiles(files, { skills: o.skills, exempt: o.exempt });
  const summary = summarizeTraces(results.map((r) => r.result));
  if (o.json) writeFileSync(o.json, `${JSON.stringify({ kind: 'hollow-trace-assert', schema: 1, summary, runs: results }, null, 2)}\n`);
  console.log(renderFlags(results, summary));
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.exitCode = await main(process.argv.slice(2));
}
