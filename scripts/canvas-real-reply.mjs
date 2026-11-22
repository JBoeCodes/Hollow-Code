// Canvas C3 wave 2 C4: does a real Claude or Codex reply, given exactly the
// system prompt Hollow sends, draw valid hollow-canvas fences? BILLED.
//
//   node scripts/canvas-real-reply.mjs                 dry run (the default)
//   node scripts/canvas-real-reply.mjs --billed [--agent claude|codex|both]
//        [--claude-model opus] [--codex-model gpt-5-codex] [--request "…"]
//        [--out <dir>]
//
// The prompts come from tests/fixtures/prompts/canvas-real-reply.json, which
// a Rust test (chat_tests.rs `probe_prompts_fixture_is_current`) generates from
// `chat_append_prompt` and the real Claude/Codex command builders: Claude's
// `--append-system-prompt` and Codex's prompt preamble for a workspace chat
// at the default Canvases choice. Never a hand copy. Since agent
// effectiveness G1 (F6) the formats are not in that prompt: Hollow puts its
// visual guide ahead of a visual turn's request, as the fixture's
// `turn.visualGuide` block, and the probe sends it the same way (its
// request is a visual turn; the Rust test checks Hollow reads it so).
//
// The dry run sends nothing billable: it runs each CLI with the billed run's
// exact flags but a model id no provider has (`DRY_MODEL`), so a CLI that
// still parses those flags answers with a model error for free. `--billed`
// sends one realistic request to each chosen agent (one model call each,
// about $1–2 for Claude), keeps every reply under `--out`, validates every
// hollow-canvas fence with the chat's own parser (src/canvas/fence.ts, the
// S1 parser, bundled on the fly with esbuild) and prints the cost each CLI
// reports. Claude runs read-only (plan mode, no session saved, no Hollow
// tools, `--max-budget-usd` 2.00); Codex read-only and ephemeral.
//
// Exit codes: 0 every run passed, 1 a run failed (an invalid fence, no
// canvas, no prose, a CLI error), 2 refused (bad arguments).
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const FIXTURE = join(root, 'tests/fixtures/prompts/canvas-real-reply.json');
/** A model id no provider serves: the dry run's free flag check. */
export const DRY_MODEL = 'hollow-probe-no-such-model';
/** Claude's own spending backstop for the billed run. */
export const MAX_BUDGET_USD = '2.00';
/** The realistic request: status, numbers, a timeline and a choice. */
export const REQUEST = [
  'Where does the Q4 launch stand, and what should we do next? Here is what I have:',
  '- Test bed: 41 of 45 checks pass; 2 fail (login timeout, CSV export crash) and 2 are skipped.',
  '- Weekly sign-ups for the beta: W1 120, W2 180, W3 260, W4 310.',
  '- Steps: merge (done), test bed (running), install (waiting on the bed), sign-off (due Friday).',
  '- Option A: ship Friday with the export behind a flag. Option B: slip a week and fix both bugs.',
].join('\n');

const AGENTS = ['claude', 'codex'];

/** Parse argv. `{ error }` for anything it doesn't know. */
export function parseArgs(argv) {
  const opts = { billed: false, agents: [...AGENTS], claudeModel: null, codexModel: null, request: REQUEST, out: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const value = () => {
      const v = argv[++i];
      if (v === undefined || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    try {
      if (a === '--billed') opts.billed = true;
      else if (a === '--agent') {
        const v = value();
        if (v === 'both') opts.agents = [...AGENTS];
        else if (AGENTS.includes(v)) opts.agents = [v];
        else return { error: `--agent takes claude, codex or both, not ${v}` };
      } else if (a === '--claude-model') opts.claudeModel = value();
      else if (a === '--codex-model') opts.codexModel = value();
      else if (a === '--request') opts.request = value();
      else if (a === '--out') opts.out = value();
      else return { error: `unknown argument ${a}` };
    } catch (e) { return { error: e.message }; }
  }
  return { opts };
}

/** The generated prompts, checked for shape. */
export function readPrompts(path = FIXTURE) {
  const fixture = JSON.parse(readFileSync(path, 'utf8'));
  const system = fixture?.claude?.appendSystemPrompt;
  const preamble = fixture?.codex?.promptPreamble;
  if (typeof system !== 'string' || !system.startsWith('# Visual answers in Hollow')) throw new Error(`${path}: no Claude system prompt`);
  if (typeof preamble !== 'string' || !preamble.includes('# Visual answers in Hollow')) throw new Error(`${path}: no Codex preamble`);
  const guide = fixture?.turn?.visualGuide;
  if (typeof guide !== 'string' || !guide.startsWith('<hollow-visual-guide>')) throw new Error(`${path}: no visual guide block`);
  return { system, preamble, guide, canvases: fixture.canvases };
}

/** The turn's message as Hollow sends it: the visual guide, then the request. */
export function turnMessage(prompts, request) {
  return `${prompts.guide}${request}`;
}

/** argv for one run, after the binary. `model` null keeps the CLI default. */
export function claudeArgs({ system, request, model }) {
  return [
    '-p', request,
    '--output-format', 'json',
    '--permission-mode', 'plan',
    '--no-session-persistence',
    '--max-budget-usd', MAX_BUDGET_USD,
    ...(model ? ['--model', model] : []),
    // Hollow's isolation (agent::isolate_claude): no MCP but what it gives,
    // which here is nothing.
    '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
    '--append-system-prompt', system,
  ];
}

export function codexArgs({ preamble, request, model }) {
  return [
    '--disable', 'apps', '--disable', 'plugins', '--disable', 'remote_plugin',
    'exec', '--json', '--skip-git-repo-check', '-s', 'read-only',
    ...(model ? ['-m', model] : []),
    '--ephemeral',
    `${preamble}${request}`,
  ];
}

/** The reply text and reported cost from Claude's `--output-format json`. */
export function readClaude(stdout) {
  let doc = null;
  try { doc = JSON.parse(stdout.trim().split('\n').filter(Boolean).pop() ?? ''); } catch { /* not JSON */ }
  if (!doc || typeof doc !== 'object') return { reply: '', error: 'no JSON result', cost: null };
  const cost = typeof doc.total_cost_usd === 'number' ? `$${doc.total_cost_usd.toFixed(4)} (reported by Claude)` : null;
  const error = doc.is_error ? String(doc.result ?? doc.subtype ?? 'error') : null;
  return { reply: typeof doc.result === 'string' ? doc.result : '', error, cost };
}

/** The last agent message and token usage from `codex exec --json` lines.
 *  Codex reports tokens, not dollars. */
export function readCodex(stdout) {
  let reply = '';
  let usage = null;
  let error = null;
  for (const line of stdout.split('\n')) {
    let ev;
    try { ev = JSON.parse(line); } catch { continue; }
    const item = ev.item ?? ev.msg ?? {};
    if (ev.type === 'item.completed' && item.type === 'agent_message') reply = String(item.text ?? reply);
    if (ev.type === 'turn.completed' && ev.usage) usage = ev.usage;
    if (ev.type === 'error' || ev.type === 'turn.failed') error = String(ev.message ?? ev.error?.message ?? 'error');
  }
  const cost = usage
    ? `${usage.input_tokens ?? '?'} input (${usage.cached_input_tokens ?? 0} cached) + ${usage.output_tokens ?? '?'} output tokens (Codex reports no dollar cost)`
    : null;
  return { reply, error, cost };
}

let parserModule = null;
/** The chat's own segmenter and the S1 fence parser, bundled from src/. */
export async function chatParsers() {
  if (parserModule) return parserModule;
  const built = buildSync({
    stdin: {
      contents: "export { parseSegments } from './src/chatSegments.ts';\nexport { parseCanvasFence } from './src/canvas/fence.ts';\n",
      resolveDir: root,
      loader: 'ts',
    },
    bundle: true, write: false, format: 'esm', platform: 'neutral', logLevel: 'silent',
  });
  const code = built.outputFiles[0].text;
  parserModule = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
  return parserModule;
}

/** Every hollow-canvas fence in `reply`, parsed as the chat would once the
 *  reply settled, plus whether the reply also states its answer in prose. */
export async function judgeReply(reply) {
  const { parseSegments, parseCanvasFence } = await chatParsers();
  const segments = parseSegments(reply);
  const prose = segments.filter(s => s.kind === 'md').map(s => s.text).join('\n').trim();
  const canvases = segments.filter(s => s.kind === 'canvas').map(s => {
    const parsed = parseCanvasFence(s.code, true);
    return parsed.state === 'ready'
      ? { state: 'ready', id: parsed.root.id, title: parsed.root.title, bytes: Buffer.byteLength(s.code), closed: s.closed }
      : { state: parsed.state, reason: parsed.reason ?? null, bytes: Buffer.byteLength(s.code), closed: s.closed };
  });
  const problems = [];
  if (canvases.length === 0) problems.push('no hollow-canvas fence');
  for (const [i, c] of canvases.entries()) {
    if (c.state !== 'ready') problems.push(`canvas ${i + 1} is ${c.state}${c.reason ? `: ${c.reason}` : ''}`);
    else if (!c.closed) problems.push(`canvas ${c.id} has no closing fence`);
  }
  if (prose.length < 40) problems.push('the answer is not stated in prose');
  const html = segments.filter(s => s.kind === 'html').length;
  return { ok: problems.length === 0, problems, canvases, html, proseChars: prose.length };
}

/** A dry run passed when the CLI took every flag and failed on the model. */
export function judgeDryRun({ status, stdout, stderr }) {
  const text = `${stdout}\n${stderr}`;
  if (/unknown (option|argument|flag)|unexpected argument|unrecognized (option|argument)|invalid value for/i.test(text)) {
    return { ok: false, reason: 'the CLI refused a flag' };
  }
  if (status === 0) return { ok: false, reason: 'the CLI exited 0 with a made-up model, so a real call may have run' };
  if (!/model|not found|not exist|invalid|unsupported|404|not available/i.test(text)) {
    return { ok: false, reason: 'no model error: the flags may not have been reached' };
  }
  return { ok: true, reason: 'flags accepted; refused the made-up model' };
}

/** Which binary to run (an env override, for tests with a fake CLI). */
export function binaryFor(agent, env = process.env) {
  return agent === 'claude' ? env.HOLLOW_PROBE_CLAUDE_BIN || 'claude' : env.HOLLOW_PROBE_CODEX_BIN || 'codex';
}

function runCli(bin, args, cwd) {
  const r = spawnSync(bin, args, { cwd, encoding: 'utf8', timeout: 600_000, maxBuffer: 64 * 1024 * 1024 });
  return { status: r.status, stdout: r.stdout ?? '', stderr: (r.stderr ?? '') + (r.error ? `\n${r.error.message}` : '') };
}

/** One agent's run (dry or billed). Returns its report. */
export async function runAgent(agent, opts, prompts, { cwd, out, env = process.env } = {}) {
  const model = opts.billed ? (agent === 'claude' ? opts.claudeModel : opts.codexModel) : DRY_MODEL;
  const request = turnMessage(prompts, opts.request);
  const args = agent === 'claude'
    ? claudeArgs({ system: prompts.system, request, model })
    : codexArgs({ preamble: prompts.preamble, request, model });
  const result = runCli(binaryFor(agent, env), args, cwd);
  if (out) writeFileSync(join(out, `${agent}.${opts.billed ? 'billed' : 'dry'}.out.txt`), `${result.stdout}\n--- stderr ---\n${result.stderr}`);
  if (!opts.billed) return { agent, dry: true, ...judgeDryRun(result) };
  const read = agent === 'claude' ? readClaude(result.stdout) : readCodex(result.stdout);
  if (out && read.reply) writeFileSync(join(out, `${agent}.reply.md`), read.reply);
  if (!read.reply) return { agent, ok: false, problems: [read.error ?? `no reply (exit ${result.status})`], cost: read.cost };
  const judged = await judgeReply(read.reply);
  return { agent, ...judged, error: read.error, cost: read.cost };
}

function print(report) {
  if (report.dry) {
    console.log(`${report.agent}: dry run ${report.ok ? 'ok' : 'FAILED'}: ${report.reason}`);
    return;
  }
  console.log(`${report.agent}: ${report.ok ? 'ok' : 'FAILED'}; cost ${report.cost ?? 'not reported'}`);
  for (const c of report.canvases ?? []) console.log(`  canvas ${c.state}${c.id ? ` ${c.id} "${c.title}"` : ''} (${c.bytes} bytes)${c.reason ? `: ${c.reason}` : ''}`);
  if (report.html) console.log(`  ${report.html} html fence(s) too`);
  for (const p of report.problems ?? []) console.log(`  problem: ${p}`);
}

export async function main(argv = process.argv.slice(2)) {
  const parsed = parseArgs(argv);
  if (parsed.error) { console.error(`canvas-real-reply: ${parsed.error}`); return 2; }
  const { opts } = parsed;
  const prompts = readPrompts();
  const out = opts.out ? resolve(opts.out) : mkdtempSync(join(tmpdir(), 'hollow-canvas-probe-'));
  mkdirSync(out, { recursive: true });
  // A neutral folder: the agents run read-only and never see this repo.
  const cwd = mkdtempSync(join(tmpdir(), 'hollow-canvas-probe-cwd-'));
  console.log(opts.billed
    ? `BILLED: one ${opts.agents.join(' and one ')} call with Hollow's prompt (Canvases: ${prompts.canvases}). Replies in ${out}.`
    : `Dry run (no billed call; pass --billed to send): checking ${opts.agents.join(' and ')} flags with model ${DRY_MODEL}.`);
  let ok = true;
  for (const agent of opts.agents) {
    const report = await runAgent(agent, opts, prompts, { cwd, out });
    print(report);
    ok &&= report.ok;
  }
  writeFileSync(join(out, 'request.txt'), opts.request);
  return ok ? 0 : 1;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().then(code => { process.exitCode = code; }, error => { console.error(error); process.exitCode = 1; });
}
