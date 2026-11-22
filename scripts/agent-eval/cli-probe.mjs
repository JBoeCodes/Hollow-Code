// An isolated, $0 probe harness for the Claude Code CLI (OB23). It runs the
// real `claude -p` against a stub Messages API on 127.0.0.1 and reads back the
// requests the CLI sends, so a question like "what does the Bash tool hand the
// model when its output is capped?" is answered from the wire, not the docs:
//
//   node scripts/agent-eval/cli-probe.mjs bash-output [--json out.json]
//        [--variant <name>]... [--cap <chars>] [--bin <claude>] [--record <dir>] [--keep]
//
// --cap (default 16000) is the limit the capped variants ask for; --record
// writes each variant's redacted request bodies as JSON lines; --keep leaves
// the temp config/project dirs in place.
//
// Isolation, so nothing can spend or leak:
// - the child env is built from an allowlist (PATH, HOME, TMPDIR, USER, SHELL,
//   LANG); no ANTHROPIC_*, CLAUDE_* or credential variable is inherited;
// - a fresh CLAUDE_CONFIG_DIR under os.tmpdir() (no OAuth credentials), a
//   dummy API key, and ANTHROPIC_BASE_URL at the stub (started first: a closed
//   port makes the CLI hang retrying);
// - every proxy variable points at a tripwire that refuses (403) and counts
//   every request and CONNECT; NO_PROXY lets only 127.0.0.1 through. Any
//   tripwire hit fails the run (exit 4);
// - the stub records request bodies only, never headers (redactForRecord).
//
// Output is names, sizes and booleans; no request or tool text is printed.
// Later probes reuse startStub/startTripwire/childEnv/runClaude with their
// own scripts: probe-agent-cli.mjs (OF22: alwaysLoad, prompt by mode, Codex
// and Grok `_meta` tolerance).
import { spawn } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { brotliDecompressSync, gunzipSync, inflateSync } from 'node:zlib';

export const DEFAULT_BIN = join(process.env.HOME ?? '', '.local/bin/claude');
export const DUMMY_KEY = 'sk-ant-dummy-probe';
export const MODEL = 'claude-opus-5-5';
export const TIMEOUT_MS = 120_000;
export const MARKERS = { head: 'PROBE_HEAD_MARK', tail: 'PROBE_TAIL_MARK' };
/** The cap the bash-output variants ask for (`--cap` overrides): the one
 *  OB23 ships, `agent::CLAUDE_BASH_MAX_OUTPUT_CHARS`. */
export const CAP = 16_000;
const ENV_ALLOW = ['PATH', 'HOME', 'TMPDIR', 'USER', 'SHELL', 'LANG'];

/** Harmless, deterministic, ~24,000 chars: the head marker first, then 300
 *  lines, then the tail marker last (where cargo/vitest put "FAILED"). */
export function probeCommand(lines = 300) {
  return `node -e "console.log('${MARKERS.head}');for(let i=0;i<${lines};i++)console.log('line '+i+' '+'x'.repeat(70));console.log('${MARKERS.tail} test result: FAILED')"`;
}

// ---------------------------------------------------------------- recording

/** The only shape a request is ever stored in: method, path and body. The
 *  headers (authorization, x-api-key, cookies) are dropped here. */
export function redactForRecord(req) {
  let body = req.body;
  if (typeof body === 'string' || Buffer.isBuffer(body)) {
    const text = String(body);
    try {
      body = JSON.parse(text);
    } catch {
      body = { unparsedChars: text.length };
    }
  }
  return { method: req.method ?? null, path: req.url ?? null, body: body ?? null };
}

/** A recorder that keeps redacted requests in memory and, with a file,
 *  appends them as JSON lines. It takes the incoming request (headers and
 *  all) and only ever stores redactForRecord's result. */
export function createRecorder(file = null) {
  const records = [];
  return {
    records,
    record(req, body) {
      const r = redactForRecord({ method: req.method, url: req.url, body });
      records.push(r);
      if (file) appendFileSync(file, JSON.stringify(r) + '\n');
      return r;
    },
  };
}

// --------------------------------------------------------------- classifier

const blockText = (content) =>
  typeof content === 'string'
    ? content
    : Array.isArray(content)
      ? content.map((b) => (b?.type === 'text' ? (b.text ?? '') : '')).join('')
      : '';

/** The latest tool_result for a tool_use of `toolName` in a Messages request
 *  body: `{id, text, isError}`, or null when there is none. */
export function findToolResult(body, toolName) {
  const messages = Array.isArray(body?.messages) ? body.messages : [];
  const ids = new Set();
  for (const m of messages) {
    if (m?.role !== 'assistant' || !Array.isArray(m.content)) continue;
    for (const b of m.content) if (b?.type === 'tool_use' && b.name === toolName) ids.add(b.id);
  }
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m?.role !== 'user' || !Array.isArray(m.content)) continue;
    for (let j = m.content.length - 1; j >= 0; j--) {
      const b = m.content[j];
      if (b?.type === 'tool_result' && ids.has(b.tool_use_id)) return { id: b.tool_use_id, text: blockText(b.content), isError: b.is_error === true };
    }
  }
  return null;
}

const PATH_RE = /(?:^|[\s:'"`(<\[=])(\/(?:[^\s'"`()<>\[\]]+\/)+[^\s'"`()<>\[\]]+)/g;

const PREVIEW_RE = /Preview \(first [^)]*\):\n([\s\S]*?)\n<\/persisted-output>/;

/** What the Bash tool handed the model in its latest tool_result: its size,
 *  the saved-output preview's bytes (null when the result is inline), whether
 *  the head and tail markers survived, and any file it points at. */
export function classifyToolResult(requestBody, markers = MARKERS) {
  const r = findToolResult(requestBody, 'Bash');
  if (!r) return { found: false, toolResultChars: 0, previewBytes: null, keepsHead: false, keepsTail: false, mentionsFile: false, overflowPath: null, isError: false };
  const paths = [...r.text.matchAll(PATH_RE)].map((m) => m[1].replace(/[.,;:]+$/, ''));
  const overflowPath = paths[0] ?? null;
  const preview = r.text.match(PREVIEW_RE)?.[1];
  return {
    found: true,
    toolResultChars: r.text.length,
    previewBytes: preview === undefined ? null : Buffer.byteLength(preview),
    keepsHead: r.text.includes(markers.head),
    keepsTail: r.text.includes(markers.tail),
    mentionsFile: overflowPath !== null || /\b(saved|written|persisted)\b[^\n]{0,40}\bfile\b|\bfile\b[^\n]{0,40}\b(saved|written|persisted)\b/i.test(r.text),
    overflowPath,
    isError: r.isError,
  };
}

// --------------------------------------------------------------- stub server

/** One scripted assistant turn: `{text}` ends the turn, `{toolUse: {name,
 *  input}}` asks for a tool. */
function message(spec, model, n) {
  const content = spec.toolUse
    ? [{ type: 'tool_use', id: `toolu_probe_${n}`, name: spec.toolUse.name, input: spec.toolUse.input }]
    : [{ type: 'text', text: spec.text ?? 'done' }];
  return {
    id: `msg_probe_${n}`,
    type: 'message',
    role: 'assistant',
    model: model ?? MODEL,
    content,
    stop_reason: spec.toolUse ? 'tool_use' : 'end_turn',
    stop_sequence: null,
    usage: { input_tokens: 10, output_tokens: 5, cache_creation_input_tokens: 0, cache_read_input_tokens: 0 },
  };
}

/** A complete streamed Messages response for one message. */
export function sseFor(msg) {
  const ev = (type, data) => `event: ${type}\ndata: ${JSON.stringify({ type, ...data })}\n\n`;
  let out = ev('message_start', { message: { ...msg, content: [], stop_reason: null, usage: { ...msg.usage, output_tokens: 1 } } });
  msg.content.forEach((b, index) => {
    if (b.type === 'tool_use') {
      out += ev('content_block_start', { index, content_block: { type: 'tool_use', id: b.id, name: b.name, input: {} } });
      out += ev('content_block_delta', { index, delta: { type: 'input_json_delta', partial_json: JSON.stringify(b.input) } });
    } else {
      out += ev('content_block_start', { index, content_block: { type: 'text', text: '' } });
      out += ev('content_block_delta', { index, delta: { type: 'text_delta', text: b.text } });
    }
    out += ev('content_block_stop', { index });
  });
  out += ev('message_delta', { delta: { stop_reason: msg.stop_reason, stop_sequence: null }, usage: { output_tokens: msg.usage.output_tokens } });
  out += ev('message_stop', {});
  return out;
}

const decode = (buf, encoding) => {
  if (encoding === 'gzip') return gunzipSync(buf);
  if (encoding === 'br') return brotliDecompressSync(buf);
  if (encoding === 'deflate') return inflateSync(buf);
  return buf;
};

/** Default: the main conversation is the request that offers the Bash tool;
 *  title, quota and other side calls do not consume the script. */
export const isMainThread = (body) => Array.isArray(body?.tools) && body.tools.some((t) => t?.name === 'Bash');

/**
 * The stub Messages API. `script` is a queue of steps, one per main-thread
 * request: each a spec or a function `(body, state) => spec`. When the queue
 * is empty the turn ends with "done". Side calls get a plain end_turn.
 * `respond(path, body, state)` (OF22) may answer a POST another API expects
 * (Codex's Responses) with `{status?, headers?, body}`; null falls through.
 */
export async function startStub({ script = [], isMain = isMainThread, recordFile = null, respond = null } = {}) {
  const recorder = createRecorder(recordFile);
  const state = { main: [], side: 0, endpoints: {}, step: 0, errors: 0 };
  let n = 0;
  const server = createServer((req, res) => {
    const chunks = [];
    req.on('data', (c) => chunks.push(c));
    req.on('end', () => {
      const path = (req.url ?? '').split('?')[0];
      state.endpoints[`${req.method} ${path}`] = (state.endpoints[`${req.method} ${path}`] ?? 0) + 1;
      try {
        const raw = decode(Buffer.concat(chunks), req.headers['content-encoding']).toString('utf8');
        if (req.method !== 'POST') return json(res, 200, {});
        const { body } = recorder.record(req, raw);
        const custom = respond?.(path, body, state);
        if (custom) {
          res.writeHead(custom.status ?? 200, custom.headers ?? { 'content-type': 'application/json' });
          return res.end(custom.body ?? '');
        }
        if (path.endsWith('/count_tokens')) return json(res, 200, { input_tokens: 1 });
        if (!path.endsWith('/v1/messages')) return json(res, 200, {});
        let spec = { text: 'ok' };
        if (isMain(body)) {
          state.main.push(body);
          const step = script[state.step++];
          spec = typeof step === 'function' ? step(body, state) : (step ?? { text: 'done' });
        } else {
          state.side++;
        }
        const msg = message(spec, body?.model, ++n);
        if (body?.stream) {
          res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', 'request-id': `req_probe_${n}` });
          res.end(sseFor(msg));
        } else {
          json(res, 200, msg);
        }
      } catch {
        state.errors++;
        json(res, 500, { type: 'error', error: { type: 'api_error', message: 'probe stub error' } });
      }
    });
  });
  await listen(server);
  return { port: server.address().port, state, records: recorder.records, close: () => close(server) };
}

function json(res, status, value) {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(JSON.stringify(value));
}
const listen = (server) => new Promise((ok, fail) => server.once('error', fail).listen(0, '127.0.0.1', ok));
const close = (server) =>
  new Promise((ok) => {
    server.closeAllConnections?.();
    server.close(() => ok());
  });

/** A proxy that refuses everything: any hit means the CLI tried to reach
 *  something other than the stub. Hits keep the target host only. */
export async function startTripwire() {
  const hits = [];
  const server = createServer((req, res) => {
    let target = req.headers.host ?? '';
    try {
      target = new URL(req.url).host;
    } catch {}
    hits.push({ kind: 'request', method: req.method, target });
    res.writeHead(403);
    res.end();
  });
  server.on('connect', (req, socket) => {
    hits.push({ kind: 'connect', target: req.url });
    socket.on('error', () => {});
    socket.end('HTTP/1.1 403 Forbidden\r\n\r\n');
  });
  await listen(server);
  return { port: server.address().port, hits, close: () => close(server) };
}

// ------------------------------------------------------------------- child

/** The child env: the allowlist from `parent`, then the isolation variables,
 *  then the variant's `extra`. */
export function childEnv({ configDir, stubPort, proxyPort, extra = {}, parent = process.env }) {
  const env = {};
  for (const k of ENV_ALLOW) if (parent[k] !== undefined) env[k] = parent[k];
  const proxy = `http://127.0.0.1:${proxyPort}`;
  Object.assign(env, {
    CLAUDE_CONFIG_DIR: configDir,
    ANTHROPIC_API_KEY: DUMMY_KEY,
    ANTHROPIC_BASE_URL: `http://127.0.0.1:${stubPort}`,
    DISABLE_TELEMETRY: '1',
    DISABLE_ERROR_REPORTING: '1',
    DISABLE_AUTOUPDATER: '1',
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    HTTPS_PROXY: proxy,
    HTTP_PROXY: proxy,
    https_proxy: proxy,
    http_proxy: proxy,
    NO_PROXY: '127.0.0.1,localhost',
    no_proxy: '127.0.0.1,localhost',
  });
  return { ...env, ...extra };
}

/** Spawns the CLI (stdin /dev/null) and resolves when it exits or is killed
 *  at `timeoutMs`. Stream-json lines are parsed; stderr is kept by size. */
export function runClaude({ bin = DEFAULT_BIN, args, cwd, env, timeoutMs = TIMEOUT_MS }) {
  return new Promise((done) => {
    // Its own process group, so a timeout also kills what its Bash tool started.
    const child = spawn(bin, args, { cwd, env, stdio: ['ignore', 'pipe', 'pipe'], detached: true });
    const kill = (sig) => {
      try {
        process.kill(-child.pid, sig);
      } catch {}
    };
    // A detached group misses the terminal's Ctrl-C: take it down with us.
    const onExit = () => kill('SIGKILL');
    const onSignal = (sig) => {
      kill('SIGKILL');
      process.exit(sig === 'SIGINT' ? 130 : 143);
    };
    process.on('exit', onExit);
    process.once('SIGINT', onSignal);
    process.once('SIGTERM', onSignal);
    let out = '';
    let err = '';
    let timedOut = false;
    child.stdout.on('data', (c) => (out += c));
    child.stderr.on('data', (c) => (err += c));
    const timer = setTimeout(() => {
      timedOut = true;
      kill('SIGTERM');
      setTimeout(() => kill('SIGKILL'), 3000).unref();
    }, timeoutMs);
    const finish = (exitCode, signal, spawnError = null) => {
      clearTimeout(timer);
      process.off('exit', onExit);
      process.off('SIGINT', onSignal);
      process.off('SIGTERM', onSignal);
      const events = [];
      for (const line of out.split('\n')) {
        if (!line.trim()) continue;
        try {
          events.push(JSON.parse(line));
        } catch {}
      }
      done({ exitCode, signal, timedOut, spawnError, events, stdout: out, stderr: err });
    };
    child.on('error', (e) => finish(null, null, e.code ?? 'spawn error'));
    child.on('close', (code, signal) => finish(code, signal));
  });
}

/** Facts from the stream-json events: the init line and the result line. */
export function summariseEvents(events) {
  const init = events.find((e) => e?.type === 'system' && e.subtype === 'init');
  const result = events.findLast((e) => e?.type === 'result');
  const resultText = typeof result?.result === 'string' ? result.result : '';
  const toolUseResultKeys = new Set();
  for (const e of events) {
    const t = e?.type === 'user' ? (e.tool_use_result ?? e.toolUseResult) : null;
    if (t && typeof t === 'object') for (const k of Object.keys(t)) toolUseResultKeys.add(k);
  }
  return {
    version: init?.claude_code_version ?? null,
    initMode: init?.permissionMode ?? null,
    resultSubtype: result?.subtype ?? null,
    resultIsError: result?.is_error ?? null,
    resultIsDone: resultText.trim() === 'done',
    apiErrorSeen: /api\.anthropic\.com|authentication_error|invalid x-api-key|API Error/i.test(resultText),
    toolUseResultKeys: [...toolUseResultKeys].sort(),
  };
}

// --------------------------------------------------------- bash-output probe

/** The OB23 variants. `readOverflow` adds a Read turn on the path the Bash
 *  result names. */
export function bashOutputVariants(cap = CAP) {
  const both = ['--allowed-tools', 'Bash', 'Read'];
  return [
    { variant: 'baseline', mode: 'bypassPermissions', env: {}, args: both },
    { variant: 'env-cap', mode: 'bypassPermissions', env: { BASH_MAX_OUTPUT_LENGTH: String(cap) }, args: both },
    { variant: 'settings-cap', mode: 'bypassPermissions', env: {}, args: [...both, '--settings', JSON.stringify({ bashOutputMaxChars: cap })] },
    // Control: an unknown settings key, so a settings-cap result is the key's doing.
    { variant: 'settings-control', mode: 'bypassPermissions', env: {}, args: [...both, '--settings', JSON.stringify({ probeUnknownKey: cap })] },
    { variant: 'settings-env-cap', mode: 'bypassPermissions', env: {}, args: [...both, '--settings', JSON.stringify({ env: { BASH_MAX_OUTPUT_LENGTH: String(cap) } })] },
    { variant: 'env-cap-acceptEdits', mode: 'acceptEdits', env: { BASH_MAX_OUTPUT_LENGTH: String(cap) }, args: both, readOverflow: true },
    { variant: 'env-cap-acceptEdits-no-read-allow', mode: 'acceptEdits', env: { BASH_MAX_OUTPUT_LENGTH: String(cap) }, args: ['--allowed-tools', 'Bash'], readOverflow: true },
    // Delegates and automations run with --no-session-persistence: is the overflow file still written?
    { variant: 'env-cap-acceptEdits-no-persist', mode: 'acceptEdits', env: { BASH_MAX_OUTPUT_LENGTH: String(cap) }, args: [...both, '--no-session-persistence'], readOverflow: true },
  ].map((v) => ({ cap: /^(baseline|settings-control)$/.test(v.variant) ? null : cap, ...v }));
}

/** Hides the temp roots in a path so it can be reported. */
const shape = (p, roots) => {
  if (!p) return null;
  let s = p;
  for (const [name, root] of Object.entries(roots)) {
    for (const r of [root, root.replace(/^\/private/, ''), '/private' + root]) if (r && s.startsWith(r)) s = `<${name}>` + s.slice(r.length);
  }
  return s.replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, '<uuid>');
};

export async function runBashOutputVariant(v, { bin = DEFAULT_BIN, recordDir = null, keep = false, timeoutMs = TIMEOUT_MS } = {}) {
  const root = mkdtempSync(join(tmpdir(), `cli-probe-${v.variant}-`));
  try {
    return await probeIn(root, v, { bin, recordDir, timeoutMs });
  } finally {
    if (!keep) rmSync(root, { recursive: true, force: true });
  }
}

async function probeIn(root, v, { bin, recordDir, timeoutMs }) {
  const configDir = join(root, 'config');
  const cwd = join(root, 'project');
  mkdirSync(configDir);
  mkdirSync(cwd);
  const script = [
    { toolUse: { name: 'Bash', input: { command: probeCommand(), description: 'Print probe output' } } },
    (body) => {
      const c = classifyToolResult(body);
      return v.readOverflow && c.overflowPath ? { toolUse: { name: 'Read', input: { file_path: c.overflowPath } } } : { text: 'done' };
    },
  ];
  const tripwire = await startTripwire();
  let stub = null;
  let run;
  try {
    stub = await startStub({ script, recordFile: recordDir ? join(recordDir, `${v.variant}.jsonl`) : null });
    // HOME points at the temp root, so the Bash tool snapshots no real shell profile.
    const env = childEnv({ configDir, stubPort: stub.port, proxyPort: tripwire.port, extra: { HOME: root, ...v.env } });
    const args = ['-p', 'probe', '--output-format', 'stream-json', '--verbose', '--model', MODEL, '--permission-mode', v.mode, '--strict-mcp-config', ...v.args];
    run = await runClaude({ bin, args, cwd, env, timeoutMs });
  } finally {
    await stub?.close();
    await tripwire.close();
  }
  const ev = summariseEvents(run.events);
  const last = stub.state.main.at(-1) ?? null;
  const bash = last ? classifyToolResult(last) : classifyToolResult(null);
  const read = last ? findToolResult(last, 'Read') : null;
  const overflowExists = bash.overflowPath ? existsSync(bash.overflowPath) : false;
  // The CLI names an unknown or invalid settings key in a warning.
  const settingsWarned = /bashOutputMaxChars/.test(run.stderr + run.stdout);
  const out = {
    variant: v.variant,
    mode: v.mode,
    ok: false,
    toolResultChars: bash.toolResultChars,
    previewBytes: bash.previewBytes,
    // The asked-for cap took effect: the result went to a file and what stayed
    // inline fits the cap. Null for a variant that asks for none.
    honoured: v.cap == null ? null : bash.found && bash.mentionsFile && bash.toolResultChars <= v.cap,
    keepsHead: bash.keepsHead,
    keepsTail: bash.keepsTail,
    mentionsFile: bash.mentionsFile,
    overflowPathFound: bash.overflowPath !== null,
    overflowPathShape: shape(bash.overflowPath, { CONFIG: configDir, CWD: cwd, ROOT: root, HOME: process.env.HOME ?? '' }),
    overflowExists,
    readAttempted: Boolean(v.readOverflow && bash.overflowPath),
    overflowReadable: Boolean(read && !read.isError && (read.text.includes(MARKERS.tail) || read.text.includes(MARKERS.head))),
    readIsError: read ? read.isError : null,
    readResultChars: read ? read.text.length : 0,
    readKeepsTail: read ? read.text.includes(MARKERS.tail) : false,
    tripwireHits: tripwire.hits.length,
    tripwireTargets: [...new Set(tripwire.hits.map((h) => h.target))],
    exitCode: run.exitCode,
    timedOut: run.timedOut,
    mainRequests: stub.state.main.length,
    sideRequests: stub.state.side,
    endpoints: stub.state.endpoints,
    stubErrors: stub.state.errors,
    stderrBytes: Buffer.byteLength(run.stderr),
    settingsWarned,
    ...ev,
    note: '',
  };
  const notes = [];
  if (run.spawnError) notes.push(`spawn failed: ${run.spawnError}`);
  if (run.timedOut) notes.push('killed at timeout');
  if (!bash.found) notes.push('no Bash tool_result reached the stub');
  if (ev.apiErrorSeen) notes.push('result reports an API error');
  if (ev.initMode && ev.initMode !== v.mode) notes.push(`init reports mode ${ev.initMode}`);
  out.note = notes.join('; ');
  out.ok = run.exitCode === 0 && !run.timedOut && tripwire.hits.length === 0 && bash.found && !ev.apiErrorSeen;
  return out;
}

// --------------------------------------------------------------------- CLI

const USAGE = `usage: node scripts/agent-eval/cli-probe.mjs bash-output [--json out.json]
       [--variant <name>]... [--cap <chars>] [--bin <claude>] [--record <dir>] [--keep]`;

export function parseArgs(argv) {
  const o = { probe: null, json: null, variants: [], bin: DEFAULT_BIN, record: null, keep: false, cap: CAP };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v === undefined || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--json') o.json = resolve(val());
    else if (a === '--variant') o.variants.push(val());
    else if (a === '--bin') o.bin = val();
    else if (a === '--record') o.record = resolve(val());
    else if (a === '--keep') o.keep = true;
    else if (a === '--cap') {
      o.cap = Number(val());
      if (!Number.isInteger(o.cap) || o.cap <= 0) throw new Error('--cap needs a positive integer');
    }
    else if (!a.startsWith('--') && !o.probe) o.probe = a;
    else throw new Error(`unknown argument ${a}`);
  }
  if (o.probe !== 'bash-output') throw new Error('name a probe: bash-output');
  return o;
}

export async function main(argv) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  if (o.record) mkdirSync(o.record, { recursive: true });
  const all = bashOutputVariants(o.cap);
  const picked = o.variants.length ? all.filter((v) => o.variants.includes(v.variant)) : all;
  if (!picked.length) {
    console.error(`no such variant; known: ${all.map((v) => v.variant).join(', ')}`);
    return 2;
  }
  const results = [];
  for (const v of picked) {
    const r = await runBashOutputVariant(v, { bin: o.bin, recordDir: o.record, keep: o.keep });
    results.push(r);
    console.error(`${r.variant}: ok=${r.ok} chars=${r.toolResultChars} head=${r.keepsHead} tail=${r.keepsTail} file=${r.mentionsFile} read=${r.overflowReadable} trip=${r.tripwireHits} exit=${r.exitCode}`);
    if (r.tripwireHits > 0 || r.apiErrorSeen) {
      console.error(`stopping: ${r.tripwireHits ? `tripwire saw ${r.tripwireTargets.join(', ')}` : 'the CLI reported an API error'}`);
      break;
    }
  }
  const report = { probe: 'bash-output', cap: o.cap, markers: MARKERS, results };
  const text = JSON.stringify(report, null, 2);
  if (o.json) writeFileSync(o.json, text + '\n');
  console.log(text);
  if (results.some((r) => r.tripwireHits > 0)) return 4;
  return results.every((r) => r.ok) ? 0 : 1;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).then((code) => process.exit(code));
}
