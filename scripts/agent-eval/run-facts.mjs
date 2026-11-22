// Run facts for one lane's run (LF3, 2026-11-17), the counters beside GD1's
// gate digest. A gate digest answers "did it pass"; this answers "how did the
// run behave": which calls it repeated, which pairs it bounced between, which
// files it reworked, how much of the wall clock it actually worked, and where
// the tokens went. Plain text under 30 lines, or `--json`. No model, no network.
//
//   node scripts/agent-eval/run-facts.mjs <transcript.jsonl> [--json]
//        [--idle-min 5] [--role-prompt <file>]
//
// Input is one Claude transcript (the shape scorecard.mjs reads:
// `~/.claude/projects/<slug>/<native session>.jsonl`): assistant lines carry
// `tool_use` blocks, user lines carry the `tool_result` blocks, every line a
// `timestamp`. Sidechain (subagent) lines are left out: this is the lane's own
// thread. Only tool names, tool inputs, timestamps and error text are read; a
// command or error that is quoted is masked and clipped (gate-digest's mask).
//
// Facts:
// - repeats: a tool call with identical input run 12 or more times (a Bash
//   `description` is not part of the identity) with how many of them failed;
// - repeated errors: the same failure text (digits folded) 3 or more times;
// - ping-pong: two different calls alternating for 4 or more calls (A B A B);
// - nodes: per edited file, attempts (edit calls) and reworks (an edit after
//   the previous edit of that file with a failed tool result in between);
// - time: wall time (first to last line), idle time (gaps over 5 minutes
//   between lines, `--idle-min`) and active time (the difference);
// - concentration: the largest tool result's share of all tool-result tokens
//   (an estimate: bytes / 4).
//
// Suggested patches are a closed set, and a person applies them: this tool
// never writes a file and never runs anything. The only kinds are
// `change-retry-limit`, `add-gate`, `remove-gate` and `append-role-prompt-line`
// (the last is dropped when `--role-prompt <file>` already holds that line).
// `makePatch` is the only way one is built and rejects any other kind or field.
// Ideas only from LoopForge's run evaluation; no code taken.
import { createReadStream, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { mask } from '../gate-digest.mjs';
import { bytesOf, exitCode } from './scorecard.mjs';

/** Identical calls at least this many times is a repeat (LoopForge's 12). */
export const REPEAT_MIN = 12;
/** The same error text at least this many times is repeated. */
export const ERROR_REPEAT_MIN = 3;
/** A gap between two lines longer than this is idle (ms). */
export const IDLE_MS = 300_000;
/** A ping-pong is at least this many alternating calls (two full cycles). */
export const PING_PONG_MIN = 4;
/** The largest tool result's share that counts as concentrated, and its floor in tokens. */
export const CONCENTRATION_MIN = 0.4;
const CONCENTRATION_FLOOR_TOKENS = 1000;
/** The limit a retry-limit patch suggests. */
const SUGGESTED_RETRY_LIMIT = 3;
const REWORK_MIN = 2;
const MAX_LINE = 200;
/** The part of an absolute path before the repo's top folder (a lane's worktree prefix). */
const PROJECT_ROOT = /^.*?\/(?=(?:src-tauri|src|scripts|docs)\/)/;
const EDIT_TOOLS = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit']);

/** The closed set of suggestions. Nothing else is ever emitted. */
export const PATCH_KINDS = Object.freeze(['change-retry-limit', 'add-gate', 'remove-gate', 'append-role-prompt-line']);
const PATCH_FIELDS = {
  'change-retry-limit': ['target', 'limit', 'because'],
  'add-gate': ['after', 'gate', 'because'],
  'remove-gate': ['gate', 'because'],
  'append-role-prompt-line': ['line', 'because'],
};

/** A patch of one of the four kinds with exactly its fields; throws on anything else. */
export function makePatch(kind, fields) {
  if (typeof kind !== 'string' || !Object.hasOwn(PATCH_FIELDS, kind) || !PATCH_KINDS.includes(kind)) throw new Error(`${JSON.stringify(kind)} is not a patch kind`);
  const allowed = PATCH_FIELDS[kind];
  for (const k of Object.keys(fields ?? {})) if (!allowed.includes(k)) throw new Error(`unknown field ${k} for ${kind}`);
  for (const k of allowed) {
    const v = fields?.[k];
    if (k === 'limit' ? !Number.isInteger(v) || v < 1 : typeof v !== 'string' || !v.trim()) throw new Error(`${kind} needs a valid ${k}`);
  }
  return Object.freeze({ kind, ...Object.fromEntries(allowed.map((k) => [k, fields[k]])) });
}

const clip = (s, n = MAX_LINE) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);
const short = (s) => clip(mask(String(s).replace(/\s+/g, ' ').trim()), 80);

/** A stable string for a tool input, key order and a Bash description ignored. */
function stable(v) {
  if (Array.isArray(v)) return `[${v.map(stable).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${stable(v[k])}`).join(',')}}`;
  return JSON.stringify(v) ?? 'null';
}

/** What a call is called in a report: the Bash command, or the tool and the file it names. */
function labelOf(name, input) {
  if (name === 'Bash' && typeof input?.command === 'string') return `Bash: ${short(input.command)}`;
  const file = input?.file_path ?? input?.notebook_path ?? input?.path ?? input?.pattern;
  return file ? `${name} ${short(String(file).replace(PROJECT_ROOT, ''))}` : name;
}

const resultText = (content) => (typeof content === 'string' ? content : Array.isArray(content) ? content.filter((p) => p?.type === 'text' && typeof p.text === 'string').map((p) => p.text).join('\n') : '');

/** An error's identity: its first line that is not `Exit code N`, lower case, digits folded. */
function errorKey(text) {
  const line = text.split('\n').map((l) => l.trim()).find((l) => l && !/^exit code\s+\d+$/i.test(l)) ?? '';
  return short(line.toLowerCase().replace(/\d+/g, '#'));
}

/** Every line of one transcript, parsed (a line that isn't JSON is skipped). */
export async function readRun(file) {
  const out = [];
  for await (const line of createInterface({ input: createReadStream(file, 'utf8'), crlfDelay: Infinity })) {
    try {
      out.push(JSON.parse(line));
    } catch {
      /* a torn or non-JSON line carries no facts */
    }
  }
  return out;
}

/** The facts of a run from its parsed transcript lines. Pure. */
export function computeFacts(entries, { idleMs = IDLE_MS } = {}) {
  const uses = new Map();
  const order = [];
  const times = [];
  const nodes = new Map();
  const errors = new Map();
  let fails = 0;
  let errorCount = 0;
  let largest = null;
  let totalTokens = 0;
  for (const e of entries) {
    if (!e || e.isSidechain) continue;
    const t = Date.parse(e.timestamp);
    if (Number.isFinite(t)) times.push(t);
    const parts = Array.isArray(e.message?.content) ? e.message.content : [];
    if (e.type === 'assistant') {
      for (const b of parts) {
        if (b?.type !== 'tool_use' || uses.has(b.id)) continue;
        const name = b.name ?? 'unknown';
        // A Bash `description` is prose, not part of what the call does.
        const input = name === 'Bash' && b.input && typeof b.input === 'object' ? Object.fromEntries(Object.entries(b.input).filter(([k]) => k !== 'description')) : b.input;
        const use = { name, key: `${name}\u0000${stable(input)}`, label: labelOf(name, b.input), failed: false };
        uses.set(b.id, use);
        order.push(use);
        const file = EDIT_TOOLS.has(name) ? (b.input?.file_path ?? b.input?.notebook_path) : null;
        if (typeof file === 'string') {
          const node = nodes.get(file) ?? { node: file.replace(PROJECT_ROOT, ''), attempts: 0, reworks: 0, failsAtLast: null };
          node.attempts += 1;
          if (node.failsAtLast != null && fails > node.failsAtLast) node.reworks += 1;
          node.failsAtLast = fails;
          nodes.set(file, node);
        }
      }
    } else if (e.type === 'user') {
      for (const p of parts) {
        if (p?.type !== 'tool_result') continue;
        const use = uses.get(p.tool_use_id);
        if (!use) continue;
        const text = resultText(p.content);
        const tokens = Math.ceil(bytesOf(p.content) / 4);
        totalTokens += tokens;
        if (!largest || tokens > largest.tokens) largest = { tool: use.name, tokens };
        const code = exitCode(text);
        if (p.is_error === true || (code != null && code !== '0')) {
          use.failed = true;
          fails += 1;
          errorCount += 1;
          const k = errorKey(text);
          if (k) errors.set(k, (errors.get(k) ?? 0) + 1);
        }
      }
    }
  }

  const groups = new Map();
  for (const u of order) {
    const g = groups.get(u.key) ?? { call: u.label, count: 0, errors: 0 };
    g.count += 1;
    if (u.failed) g.errors += 1;
    groups.set(u.key, g);
  }
  const repeats = [...groups.values()].filter((g) => g.count >= REPEAT_MIN).sort((a, b) => b.count - a.count);

  // Ping-pong: maximal runs where call i equals call i+2 and differs from call i+1.
  const pingPong = [];
  for (let i = 0; i + 1 < order.length; ) {
    if (order[i].key === order[i + 1].key) {
      i += 1;
      continue;
    }
    let j = i + 2;
    while (j < order.length && order[j].key === order[j - 2].key) j += 1;
    if (j - i >= PING_PONG_MIN) {
      pingPong.push({ pair: [order[i].label, order[i + 1].label], calls: j - i });
      i = j - 1;
    } else i += 1;
  }

  const repeatedErrors = [...errors].filter(([, n]) => n >= ERROR_REPEAT_MIN).map(([text, count]) => ({ text, count })).sort((a, b) => b.count - a.count);

  const wallMs = times.length ? Math.max(...times) - Math.min(...times) : 0;
  const sorted = [...times].sort((a, b) => a - b);
  let idle = 0;
  let idleGaps = 0;
  for (let i = 1; i < sorted.length; i++) {
    const gap = sorted[i] - sorted[i - 1];
    if (gap > idleMs) {
      idle += gap;
      idleGaps += 1;
    }
  }

  return {
    calls: order.length,
    errors: errorCount,
    repeats,
    repeatedErrors,
    pingPong,
    nodes: [...nodes.values()].map(({ node, attempts, reworks }) => ({ node, attempts, reworks })),
    time: { wallMs, idleMs: idle, activeMs: wallMs - idle, idleGaps },
    concentration: largest && totalTokens ? { tool: largest.tool, tokens: largest.tokens, totalTokens, share: Math.round((largest.tokens / totalTokens) * 1000) / 1000 } : null,
  };
}

const norm = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim();

const LINES = {
  pingPong: 'When two calls alternate twice with nothing changed between them, stop and try a different approach.',
  repeat: 'Do not run the same tool call a third time with the same input; change the input or the approach first.',
  concentration: 'Read large files and logs with an offset and limit, or search them, instead of printing them whole.',
};

/** The suggested patches for `facts`, only from the closed set. `rolePrompt` (text) drops a line it already holds. Pure; nothing is applied. */
export function suggestPatches(facts, { rolePrompt = '' } = {}) {
  const out = [];
  const have = norm(rolePrompt ?? '');
  const append = (line, because) => {
    if (norm(line) && have.includes(norm(line))) return;
    if (out.some((p) => p.kind === 'append-role-prompt-line' && p.line === line)) return;
    out.push(makePatch('append-role-prompt-line', { line, because }));
  };
  for (const r of facts.repeats) {
    if (r.errors > 0) out.push(makePatch('change-retry-limit', { target: r.call, limit: SUGGESTED_RETRY_LIMIT, because: `the same call ran ${r.count} times and failed ${r.errors}` }));
    else if (r.call.startsWith('Bash: ')) out.push(makePatch('remove-gate', { gate: r.call.slice(6), because: `the same command ran ${r.count} times and never failed` }));
    else append(LINES.repeat, `${r.call} ran ${r.count} times with the same input`);
  }
  if (!facts.repeats.length && facts.repeatedErrors.length) {
    const e = facts.repeatedErrors[0];
    out.push(makePatch('change-retry-limit', { target: `calls that fail with "${e.text}"`, limit: SUGGESTED_RETRY_LIMIT, because: `the same error came back ${e.count} times` }));
  }
  for (const n of facts.nodes) {
    if (n.reworks >= REWORK_MIN) out.push(makePatch('add-gate', { after: n.node, gate: 'a focused check or review before the next edit of this file', because: `${n.node} was edited ${n.attempts} times, ${n.reworks} of them after a failure` }));
  }
  if (facts.pingPong.length) append(LINES.pingPong, `${facts.pingPong.length} ping-pong pair${facts.pingPong.length === 1 ? '' : 's'}, the longest ${Math.max(...facts.pingPong.map((p) => p.calls))} calls`);
  const c = facts.concentration;
  if (c && c.share >= CONCENTRATION_MIN && c.tokens >= CONCENTRATION_FLOOR_TOKENS) append(LINES.concentration, `${Math.round(c.share * 100)}% of tool-result tokens came from one ${c.tool} result`);
  return out;
}

const minutes = (ms) => `${(ms / 60_000).toFixed(1)} min`;

/** The report as plain text lines (under 30). */
export function render(facts, patches) {
  const out = [`run facts: ${facts.calls} tool calls, ${facts.errors} failed`];
  const t = facts.time;
  out.push(`time: wall ${minutes(t.wallMs)}, active ${minutes(t.activeMs)}, idle ${minutes(t.idleMs)} (${t.idleGaps} gap${t.idleGaps === 1 ? '' : 's'})`);
  for (const r of facts.repeats.slice(0, 3)) out.push(clip(`repeat: same call ${r.count} times (${r.errors} failed): ${r.call}`));
  for (const e of facts.repeatedErrors.slice(0, 2)) out.push(clip(`repeat: same error ${e.count} times: ${e.text}`));
  for (const p of facts.pingPong.slice(0, 2)) out.push(clip(`ping-pong: ${p.pair[0]} <-> ${p.pair[1]}, ${p.calls} calls`));
  const reworked = facts.nodes.filter((n) => n.reworks > 0).sort((a, b) => b.reworks - a.reworks);
  for (const n of reworked.slice(0, 3)) out.push(clip(`rework: ${n.node}: ${n.attempts} attempts, ${n.reworks} reworks`));
  if (!reworked.length && facts.nodes.length) out.push(`rework: none (${facts.nodes.length} file${facts.nodes.length === 1 ? '' : 's'} edited)`);
  if (facts.concentration) out.push(`tokens: ${Math.round(facts.concentration.share * 100)}% of tool-result tokens in one ${facts.concentration.tool} result (about ${facts.concentration.tokens} of ${facts.concentration.totalTokens})`);
  if (!facts.repeats.length && !facts.repeatedErrors.length && !facts.pingPong.length) out.push('no repeats or ping-pong found');
  if (patches.length) {
    out.push('suggested patches (a person applies them; nothing was changed):');
    for (const p of patches.slice(0, 8)) {
      const what = p.kind === 'change-retry-limit' ? `limit ${p.limit} for ${p.target}` : p.kind === 'add-gate' ? `${p.gate} (after ${p.after})` : p.kind === 'remove-gate' ? p.gate : p.line;
      out.push(clip(`  ${p.kind}: ${what}`));
    }
    if (patches.length > 8) out.push(`  and ${patches.length - 8} more (see --json)`);
  }
  return out;
}

async function main(argv) {
  const args = { json: false };
  const files = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--json') args.json = true;
    else if (a === '--idle-min') args.idleMin = Number(argv[++i]);
    else if (a === '--role-prompt') args.rolePrompt = argv[++i];
    else if (a.startsWith('--')) {
      console.error(`unknown argument ${a}`);
      return 2;
    } else files.push(a);
  }
  if (files.length !== 1 || (args.idleMin != null && !(args.idleMin > 0))) {
    console.error('usage: node scripts/agent-eval/run-facts.mjs <transcript.jsonl> [--json] [--idle-min 5] [--role-prompt <file>]');
    return 2;
  }
  let entries;
  let rolePrompt = '';
  try {
    entries = await readRun(files[0]);
    if (args.rolePrompt) rolePrompt = readFileSync(args.rolePrompt, 'utf8');
  } catch (e) {
    console.error(e.message);
    return 2;
  }
  const facts = computeFacts(entries, { idleMs: args.idleMin ? args.idleMin * 60_000 : IDLE_MS });
  const patches = suggestPatches(facts, { rolePrompt });
  console.log(args.json ? JSON.stringify({ facts, patches }, null, 2) : render(facts, patches).join('\n'));
  return 0;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exit(await main(process.argv.slice(2)));
