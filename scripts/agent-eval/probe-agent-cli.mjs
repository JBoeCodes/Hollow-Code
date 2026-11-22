// A $0 probe of three agent-CLI facts Hollow rests on (OF22), read from the
// wire so a CLI upgrade that changes one shows up:
//
//   node scripts/agent-eval/probe-agent-cli.mjs [--json out.json]
//        [--agent claude|codex|grok]... [--claude-bin <path>]
//        [--codex-bin <path>] [--grok-bin <path>] [--keep]
//
// (a) alwaysLoad: Claude Code, with tool search on (ENABLE_TOOL_SEARCH=true;
//     a custom base URL turns it off otherwise), sends an MCP tool whose
//     tools/list entry carries `_meta {"anthropic/alwaysLoad": true}` in full
//     and keeps the rest deferred (HB05, `app_mcp::ALWAYS_LOAD`).
// (b) prompt by mode: the system text and each tool description's bytes,
//     `--permission-mode acceptEdits` against `bypassPermissions`.
// (c) `_meta` tolerance: Codex and Grok, given the same stub MCP server,
//     exit 0 and still offer the flagged tool. A second, control server
//     (`stub-mcp.mjs --no-meta`, tools `probe_ctl_a`/`probe_ctl_b`, no
//     `_meta`) tells the outcomes apart: both servers' tools offered is
//     tolerated, control tools without `probe_flagged` is drift, neither is
//     inconclusive (the CLI never offered MCP tools; not ok, not drift).
//
// Every CLI talks to the stub MCP server `fixtures/probe/stub-mcp.mjs` (three
// `probe_*` tools, `_meta` on `probe_flagged` only) and to cli-probe.mjs's
// stub on 127.0.0.1, with its isolation: an allowlisted env (childEnv), HOME
// and the CLI's own home and TMPDIR under os.tmpdir(), dummy keys, and every
// proxy variable at the tripwire. Any tripwire hit fails that agent's run. A
// hit other than Codex's Statsig metrics exporter (`CONNECT
// ab.chatgpt.com:443` from the Codex run only, reported as
// `blockedTelemetry`) also stops the probe (exit 4). Codex runs only when
// `codexPinned` agrees its config can reach nothing but the stub; otherwise
// it records `{probed: false, reason}`. Env `PROBE_CODEX_DISABLE=probe-only`
// drops Hollow's inherited `--disable` list from the Codex run (codexArgs).
//
// Output is names, counts, offsets and byte sizes; no request, prompt or tool
// text is printed or written. Exit 0 clean; 1 a run failed, was inconclusive,
// or an agent asked for was not probed; 3 a fact Hollow relies on changed
// (`drift`, which outranks 1); 4 the tripwire saw traffic other than blocked
// telemetry; 2 bad arguments.
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { DEFAULT_BIN, DUMMY_KEY, MODEL, TIMEOUT_MS, childEnv, runClaude, startStub, startTripwire, summariseEvents } from './cli-probe.mjs';

export const PROBE_TOOLS = ['probe_flagged', 'probe_other_a', 'probe_other_b'];
/** The control server's tools (`stub-mcp.mjs --no-meta`). */
export const CONTROL_TOOLS = ['probe_ctl_a', 'probe_ctl_b'];
export const FLAGGED = 'probe_flagged';
export const MODES = ['acceptEdits', 'bypassPermissions'];
export const AGENTS = ['claude', 'codex', 'grok'];
export const STUB_MCP = fileURLToPath(new URL('./fixtures/probe/stub-mcp.mjs', import.meta.url));
export const CODEX_BIN = '/opt/homebrew/bin/codex';
export const GROK_BIN = join(process.env.HOME ?? '', '.local/bin/grok');
/** The MCP server name each CLI is given (Claude names its tools `mcp__probe__<tool>`). */
export const SERVER = 'probe';
/** The control MCP server's name (Codex and Grok only). */
export const CONTROL_SERVER = 'control';
export const CODEX_PROVIDER = 'probe-stub';
export const GROK_MODEL = 'probe-stub';
const VERSION_TIMEOUT_MS = 15_000;
/** Codex features off, as Hollow's tool-free helpers turn them off (the list
 *  codex-cli 0.162.1 accepted: `codex_transform::DISABLED_FEATURES`), so no
 *  connector, plugin, browser or memory reaches for the network. MCP stays on. */
export const CODEX_DISABLED = [
  'apps', 'plugins', 'shell_tool', 'unified_exec', 'code_mode', 'code_mode_host', 'code_mode_only',
  'browser_use', 'browser_use_external', 'browser_use_full_cdp_access', 'computer_use', 'goals',
  'multi_agent', 'multi_agent_v2', 'image_generation', 'in_app_browser', 'memories', 'skill_search',
  'tool_suggest', 'workspace_dependencies', 'current_time_reminder', 'token_budget',
  'enable_request_compression', 'hooks',
];
/** Features the probe turns off on top of Hollow's list (all four are
 *  feature names in codex-cli 0.163.0's binary): tool search, so the MCP
 *  tools are offered in `tools` rather than deferred behind a search, and
 *  runtime metrics and remote model discovery, which reach for OpenAI. */
export const CODEX_PROBE_DISABLED = ['tool_search', 'tool_search_always_defer_mcp_tools', 'runtime_metrics', 'remote_models'];

/** The one tripwire hit a run may make without stopping the probe, from the
 *  Codex run only (the same hit from Claude or Grok is hard): Codex's
 *  Statsig OTLP metrics exporter (`https://ab.chatgpt.com/otlp/v1/metrics`),
 *  seen once from codex-cli 0.163.0 before the config turned `[otel]` and
 *  `runtime_metrics` off. The tripwire refused it, and it still fails the
 *  agent's run; it only doesn't abort the agents after it. Matched exactly. */
export const BLOCKED_TELEMETRY = [{ agent: 'codex', kind: 'connect', target: 'ab.chatgpt.com:443' }];

// ------------------------------------------------------------- classifiers

/** A tool name without Claude Code's `mcp__<server>__` prefix. */
export const bareName = (name) => String(name ?? '').replace(/^mcp__.+?__/, '');

const PROBE_NAME = /^probe_[a-z0-9]+(?:_[a-z0-9]+)*$/;
const PROBE_RE = /(?:mcp__[A-Za-z0-9_-]+?__)?(probe_[a-z0-9_]+)/g;

function strings(value, out = []) {
  if (typeof value === 'string') out.push(value);
  else if (Array.isArray(value)) for (const v of value) strings(v, out);
  else if (value && typeof value === 'object') for (const v of Object.values(value)) strings(v, out);
  return out;
}

/** Every `probe_*` tool name anywhere in `value` (tools, system or
 *  messages), with any `mcp__<server>__` prefix dropped. Sorted. */
export function probeNamesIn(value) {
  const names = new Set();
  for (const s of strings(value)) for (const m of s.matchAll(PROBE_RE)) if (PROBE_NAME.test(m[1])) names.add(m[1]);
  return [...names].sort();
}

/**
 * Which probe tools a Messages request sends in full and which it defers.
 * A tool object in `tools` without `defer_loading: true` is sent in full; one
 * with it, one only named in the system or message text (Claude Code's
 * deferred-tools reminder), and a known one absent altogether are deferred.
 * Only `probe_*` names are classified; built-ins (Bash, ToolSearch) are not.
 */
export function classifyTools(body, known = PROBE_TOOLS) {
  const tools = Array.isArray(body?.tools) ? body.tools : [];
  const full = new Set();
  const flagged = new Set();
  for (const t of tools) {
    const name = bareName(t?.name);
    if (!PROBE_NAME.test(name)) continue;
    (t.defer_loading === true ? flagged : full).add(name);
  }
  const all = new Set([...known, ...flagged, ...probeNamesIn([body?.system, body?.messages]), ...full]);
  return { alwaysLoaded: [...full].sort(), deferred: [...all].filter((n) => !full.has(n)).sort() };
}

/** The system text of a Messages request: a string, or its text blocks joined. */
export const systemText = (body) => systemBlocks(body).join('');

const bytes = (s) => Buffer.byteLength(s ?? '');
const descBytes = (body) =>
  new Map((Array.isArray(body?.tools) ? body.tools : []).filter((t) => typeof t?.name === 'string').map((t) => [t.name, bytes(t.description)]));

export const ROOT_TOKEN = '<ROOT>';
export const UUID_TOKEN = '<UUID>';
const UUID_RE = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi;

/** A path as Claude Code slugs it into a directory name (every
 *  non-alphanumeric character a `-`), as in `projects/<slug>/`. */
export const slugOf = (path) => path.replace(/[^A-Za-z0-9]/g, '-');

/** `text` with each of `roots` (a run's temp root, as created and as
 *  resolved), and each root's slug form, replaced by `<ROOT>`, longest first
 *  so `/private/var/…` goes before the `/var/…` inside it; then every UUID
 *  (a session id differs per run) replaced by `<UUID>`. */
export function normaliseRoot(text, roots = []) {
  let out = text ?? '';
  const paths = roots.filter((r) => typeof r === 'string' && r.length > 0);
  const rs = [...new Set([...paths, ...paths.map(slugOf)])].sort((x, y) => y.length - x.length);
  for (const r of rs) out = out.split(r).join(ROOT_TOKEN);
  return out.replace(UUID_RE, UUID_TOKEN);
}

/** A Messages request's system blocks' texts (a string system is one block). */
export function systemBlocks(body) {
  const s = body?.system;
  if (typeof s === 'string') return [s];
  return Array.isArray(s) ? s.map((b) => (typeof b === 'string' ? b : (b?.text ?? ''))) : [];
}

/** Where two requests' system texts first differ, each normalised with its
 *  own roots: `systemIdentical`, `systemFirstDiffOffset` (the character
 *  offset into the joined normalised text, null when identical) and
 *  `systemDiffBlock` (the index of the first system block that differs,
 *  null when identical). Offsets only; no text. */
export function systemDiff(a, b, rootsA = [], rootsB = []) {
  const ba = systemBlocks(a).map((t) => normaliseRoot(t, rootsA));
  const bb = systemBlocks(b).map((t) => normaliseRoot(t, rootsB));
  const [ta, tb] = [ba.join(''), bb.join('')];
  if (ta === tb) return { systemIdentical: true, systemFirstDiffOffset: null, systemDiffBlock: null };
  let offset = 0;
  while (offset < ta.length && offset < tb.length && ta[offset] === tb[offset]) offset++;
  let block = 0;
  while (block < ba.length && block < bb.length && ba[block] === bb[block]) block++;
  return { systemIdentical: false, systemFirstDiffOffset: offset, systemDiffBlock: block };
}

/** Byte deltas from request `a` to request `b` (B − A): the joined system
 *  text normalised with each run's roots (`rootsA`, `rootsB`), and each
 *  tool description that differs (a tool only one side sends counts as 0
 *  bytes on the other). */
export function modeDiff(a, b, rootsA = [], rootsB = []) {
  const da = descBytes(a);
  const db = descBytes(b);
  const tools = {};
  for (const name of [...new Set([...da.keys(), ...db.keys()])].sort()) {
    const d = (db.get(name) ?? 0) - (da.get(name) ?? 0);
    if (d !== 0) tools[name] = d;
  }
  return { system: bytes(normaliseRoot(systemText(b), rootsB)) - bytes(normaliseRoot(systemText(a), rootsA)), tools };
}

/** Whether the request runs tool search: a ToolSearch tool, a server tool
 *  search, or a tool sent with `defer_loading`. */
export const toolSearchOn = (body) =>
  (Array.isArray(body?.tools) ? body.tools : []).some((t) => t?.name === 'ToolSearch' || t?.defer_loading === true || /^tool_search/.test(t?.type ?? ''));

/** Tool names a request offers, any API shape: Messages `{name}`, Responses
 *  `{type: 'function', name}`, Chat `{function: {name}}`, and the tools
 *  nested in a group (`{tools: [...]}`). Prefixes kept. */
export function toolNames(body) {
  const out = [];
  const walk = (t) => {
    if (!t || typeof t !== 'object') return;
    const name = t.name ?? t.function?.name;
    if (typeof name === 'string') out.push(name);
    if (Array.isArray(t.tools)) t.tools.forEach(walk);
  };
  (Array.isArray(body?.tools) ? body.tools : []).forEach(walk);
  return out;
}

/** The `type`s of a request's tools, nested groups included. Sorted, unique. */
export function toolTypes(body) {
  const out = new Set();
  const walk = (t) => {
    if (!t || typeof t !== 'object') return;
    if (typeof t.type === 'string') out.add(t.type);
    if (Array.isArray(t.tools)) t.tools.forEach(walk);
  };
  (Array.isArray(body?.tools) ? body.tools : []).forEach(walk);
  return [...out].sort();
}

/**
 * What a Codex or Grok request list shows about `_meta` tolerance, from the
 * probe tools and control tools it lists in its `tools` (a name only
 * mentioned in instructions or input text doesn't count, since the model
 * can't call it): `tolerated` when the flagged tool
 * and the control server's tools are both on offer, `drift` when the control
 * tools are but the flagged one isn't, `inconclusive` otherwise (the CLI
 * never offered the control server's tools, so the run proves nothing).
 */
export function metaOutcome(probeListed = [], controlListed = []) {
  if (!controlListed.length) return 'inconclusive';
  return probeListed.includes(FLAGGED) ? 'tolerated' : 'drift';
}

// ----------------------------------------------------------------- report

const NOT_ASKED = { probed: false, reason: 'not asked' };
const uniqueSorted = (xs) => [...new Set(xs)].sort();

/** Whether a tripwire hit from `agent`'s run is exactly one
 *  `BLOCKED_TELEMETRY` names for that agent (Codex's only). */
export const isBlockedTelemetry = (h, agent) =>
  BLOCKED_TELEMETRY.some((t) => t.agent === agent && h?.kind === t.kind && h?.target === t.target);

/** `agent`'s raw run's tripwire hits that are not its blocked telemetry. */
export const hardHits = (r, agent) => (Array.isArray(r?.tripwireHitList) ? r.tripwireHitList : []).filter((h) => !isBlockedTelemetry(h, agent));

function runFacts(r, agent) {
  const hits = Array.isArray(r.tripwireHitList) ? r.tripwireHitList : [];
  const soft = hits.filter((h) => isBlockedTelemetry(h, agent));
  const tripwireHits = r.tripwireHits ?? hits.length;
  return {
    exitCode: r.exitCode ?? null,
    timedOut: Boolean(r.timedOut),
    spawnError: r.spawnError ?? null,
    tripwireHits,
    tripwireTargets: r.tripwireTargets ?? [],
    // Every hit fails the run's isolation; only the rest stop the probe.
    isolationOk: tripwireHits === 0,
    blockedTelemetry: uniqueSorted(soft.map((h) => `${h.kind} ${h.target}`)),
    blockedTelemetryHits: soft.length,
    hardTripwireHits: tripwireHits - soft.length,
    mainRequests: r.bodies?.length ?? 0,
    sideRequests: r.sideRequests ?? 0,
    unparsedBodies: r.unparsedBodies ?? 0,
    endpoints: r.endpoints ?? {},
    stubErrors: r.stubErrors ?? 0,
    stderrBytes: r.stderrBytes ?? 0,
    eventTypes: r.eventTypes ?? [],
  };
}

/** A Claude run's temp root as created and as resolved, for normaliseRoot. */
const rootsOf = (r) => [r?.root, r?.realRoot];

function summariseClaude(c) {
  if (!c?.probed) return c ?? NOT_ASKED;
  const runs = (c.runs ?? []).map((r) => {
    const body = r.bodies?.[0] ?? null;
    const facts = runFacts(r, 'claude');
    const { alwaysLoaded, deferred } = classifyTools(body);
    const initMode = r.events?.initMode ?? null;
    const mcpStatus = r.mcpStatus ?? [];
    const searchOn = toolSearchOn(body);
    const clean = facts.exitCode === 0 && !facts.timedOut && body !== null && !r.events?.apiErrorSeen;
    // What the alwaysLoad check needs before it means anything: the stub
    // server connected, tool search on, and the mode asked for in force.
    const unmet = [
      ...(mcpStatus.includes(`${SERVER}:connected`) ? [] : ['mcp']),
      ...(searchOn ? [] : ['toolSearch']),
      ...(initMode === r.mode ? [] : ['mode']),
    ];
    // The fact HB05 relies on: the flagged tool alone is sent in full.
    const alwaysLoadHonoured = alwaysLoaded.length === 1 && alwaysLoaded[0] === FLAGGED;
    const outcome = !clean ? 'failed' : unmet.length ? 'inconclusive' : alwaysLoadHonoured ? 'honoured' : 'drift';
    return {
      mode: r.mode,
      ok: facts.isolationOk && (outcome === 'honoured' || outcome === 'drift'),
      outcome,
      unmet,
      version: r.events?.version ?? r.version ?? null,
      initMode,
      mcpStatus,
      ...facts,
      toolCount: Array.isArray(body?.tools) ? body.tools.length : 0,
      toolTypes: toolTypes(body),
      systemBytes: bytes(normaliseRoot(systemText(body), rootsOf(r))),
      toolSearchOn: searchOn,
      probeToolsSeen: body ? probeNamesIn(body) : [],
      alwaysLoaded,
      deferred,
      alwaysLoadHonoured,
    };
  });
  const byMode = Object.fromEntries((c.runs ?? []).map((r) => [r.mode, r]));
  const [ra, rb] = MODES.map((m) => byMode[m] ?? null);
  const [a, b] = [ra?.bodies?.[0] ?? null, rb?.bodies?.[0] ?? null];
  return {
    probed: true,
    ok: runs.length === MODES.length && runs.every((r) => r.ok),
    runs,
    // The system texts compare with each run's temp root (and its slug) as
    // `<ROOT>` and UUIDs as `<UUID>`: those differ per run, not per mode.
    modeDiff:
      a && b
        ? {
            from: MODES[0],
            to: MODES[1],
            ...modeDiff(a, b, rootsOf(ra), rootsOf(rb)),
            ...systemDiff(a, b, rootsOf(ra), rootsOf(rb)),
          }
        : null,
  };
}

function summariseOther(o, agent) {
  if (!o?.probed) return o ?? NOT_ASKED;
  const bodies = o.bodies ?? [];
  const facts = runFacts(o, agent);
  const clean = facts.exitCode === 0 && !facts.timedOut && bodies.length > 0;
  const names = uniqueSorted(bodies.flatMap((b) => toolNames(b).map(bareName)));
  const listed = names.filter((n) => PROBE_NAME.test(n));
  // The outcome rests on tools actually listed; a name anywhere in the
  // request (instructions, input) is reported as seen, for information only.
  const probeToolsListed = listed.filter((n) => PROBE_TOOLS.includes(n));
  const controlToolsListed = listed.filter((n) => CONTROL_TOOLS.includes(n));
  const mentioned = probeNamesIn(bodies);
  const outcome = clean ? metaOutcome(probeToolsListed, controlToolsListed) : 'failed';
  return {
    probed: true,
    ok: facts.isolationOk && (outcome === 'tolerated' || outcome === 'drift'),
    outcome,
    version: o.version ?? null,
    ...facts,
    toolCount: Math.max(0, ...bodies.map((b) => toolNames(b).length)),
    toolTypes: uniqueSorted(bodies.flatMap(toolTypes)),
    probeToolsListed,
    controlToolsListed,
    probeToolsSeen: mentioned.filter((n) => PROBE_TOOLS.includes(n)),
    controlToolsSeen: mentioned.filter((n) => CONTROL_TOOLS.includes(n)),
    flaggedListed: listed.includes(FLAGGED),
    // Codex: which `--disable` set the run used, and whether the built-in
    // `update_plan` (forced on) was listed, which tells "no tools sent at
    // all" from "MCP tools missing".
    ...(agent === 'codex' ? { codexDisable: o.codexDisable ?? 'hollow', updatePlanListed: names.includes('update_plan') } : {}),
    // Whether the CLI passed the MCP `_meta` on to the model's tool list.
    metaForwarded: bodies.some((b) => JSON.stringify(b?.tools ?? []).includes('anthropic/alwaysLoad')),
    // It ran clean with the flagged tool and the control tools on offer.
    toleratesMeta: outcome === 'tolerated',
  };
}

/**
 * The JSON report from the raw runs: `{claude: {probed, runs: [raw]},
 * codex: raw, grok: raw}`, each raw run holding its main request bodies,
 * which the report reduces to names and sizes. An agent left out is
 * `{probed: false, reason: 'not asked'}`; one asked for but not probed
 * (`{probed: false, reason}`) makes the report not ok.
 */
export function summarise({ claude, codex, grok } = {}) {
  const report = { probe: 'agent-cli', claude: summariseClaude(claude), codex: summariseOther(codex, 'codex'), grok: summariseOther(grok, 'grok') };
  const agents = [report.claude, report.codex, report.grok];
  const asked = agents.filter((r) => r.probed || r.reason !== NOT_ASKED.reason);
  const runs = [...(report.claude.runs ?? []), report.codex, report.grok].filter((r) => r.probed !== false);
  const sum = (k) => runs.reduce((n, r) => n + (r[k] ?? 0), 0);
  report.tripwireHits = sum('tripwireHits');
  report.blockedTelemetryHits = sum('blockedTelemetryHits');
  report.hardTripwireHits = sum('hardTripwireHits');
  const drift = [];
  for (const r of report.claude.runs ?? []) if (r.outcome === 'drift') drift.push(`claude.alwaysLoad.${r.mode}`);
  for (const name of ['codex', 'grok']) if (report[name].outcome === 'drift') drift.push(`${name}.meta`);
  report.drift = drift;
  report.ok = asked.length > 0 && asked.every((r) => r.probed && r.ok);
  return report;
}

/**
 * The probe's exit code from its report and the agents asked for: 4 when
 * the tripwire saw anything but blocked telemetry, 3 on any drift (even if
 * another agent was inconclusive: a changed fact outranks a run that proved
 * nothing), 1 when an agent asked for was not probed or its run was not ok
 * (failed, inconclusive, or any tripwire hit), else 0.
 */
export function exitCodeFor(report, agents = AGENTS) {
  if ((report?.hardTripwireHits ?? 0) > 0) return 4;
  if (report?.drift?.length) return 3;
  const asked = agents.map((a) => report?.[a] ?? NOT_ASKED);
  return asked.some((r) => !r.probed || !r.ok) ? 1 : 0;
}

// ------------------------------------------------------------- CLI configs

const toml = (s) => JSON.stringify(s);

/** A Codex `[mcp_servers.<name>]` table: required, and started before the
 *  first request only once its tool catalog is listed (`startup_readiness =
 *  "catalog"`, an `McpStartupReadiness` variant in codex-cli 0.163.0). */
const codexMcpServer = (name, node, args) => [
  `[mcp_servers.${name}]`,
  `command = ${toml(node)}`,
  `args = [${args.map(toml).join(', ')}]`,
  'startup_timeout_sec = 30',
  'startup_readiness = "catalog"',
  'required = true',
  '',
];

/** `$CODEX_HOME/config.toml`: the stub as the only model provider (Responses
 *  API, no OpenAI auth, no retries), analytics and every OpenTelemetry
 *  exporter off, file credentials for the CLI and MCP OAuth (no keychain),
 *  and two MCP servers: the
 *  stub (`_meta` on one tool) and the control (`--no-meta`). Every key is in
 *  codex-cli 0.163.0's binary; the run passes `--strict-config`, so one a
 *  later version drops fails the run instead of being ignored. */
export function codexConfig({ port, node = process.execPath, stub = STUB_MCP }) {
  return [
    `model_provider = ${toml(CODEX_PROVIDER)}`,
    'web_search = "disabled"',
    'project_doc_max_bytes = 0',
    'check_for_update_on_startup = false',
    'cli_auth_credentials_store = "file"',
    'mcp_oauth_credentials_store = "file"',
    '',
    '[analytics]',
    'enabled = false',
    '',
    '[otel]',
    'exporter = "none"',
    'trace_exporter = "none"',
    'metrics_exporter = "none"',
    '',
    `[model_providers.${CODEX_PROVIDER}]`,
    'name = "Probe stub"',
    `base_url = ${toml(`http://127.0.0.1:${port}/v1`)}`,
    'wire_api = "responses"',
    'requires_openai_auth = false',
    'request_max_retries = 0',
    'stream_max_retries = 0',
    '',
    ...codexMcpServer(SERVER, node, [stub]),
    ...codexMcpServer(CONTROL_SERVER, node, [stub, '--no-meta']),
  ].join('\n');
}

/** The Codex `--disable` set: `hollow` (Hollow's inherited list plus the
 *  probe's, the default) or `probe-only` (the probe's four alone), chosen by
 *  env `PROBE_CODEX_DISABLE=probe-only`. The experiment for a run that sent
 *  no tools at all: does one of Hollow's 24 take the MCP tools with it? With
 *  fewer features off, more of Codex may reach for the network; the tripwire
 *  still stops that (exit 4). */
export const CODEX_DISABLE_MODES = ['hollow', 'probe-only'];

export function codexDisableMode(env = process.env) {
  const v = env.PROBE_CODEX_DISABLE;
  if (v === undefined || v === '') return 'hollow';
  if (!CODEX_DISABLE_MODES.includes(v)) throw new Error(`PROBE_CODEX_DISABLE is one of ${CODEX_DISABLE_MODES.join(', ')}`);
  return v;
}

/** The `codex exec` arguments: no git check, no session files, no rules,
 *  strict config, JSONL events, read-only sandbox, the `update_plan` tool
 *  forced on (its presence in the request tells "tools sent at all" from
 *  "MCP tools missing"), the disabled features of `disable` (above), then
 *  the prompt. */
export const codexArgs = (prompt = 'probe', { disable = 'hollow' } = {}) => [
  'exec', '--skip-git-repo-check', '--ephemeral', '--ignore-rules', '--strict-config', '--json', '--color', 'never', '--sandbox', 'read-only',
  '-c', 'tools.update_plan.enabled=true',
  ...(disable === 'probe-only' ? CODEX_PROBE_DISABLED : [...CODEX_DISABLED, ...CODEX_PROBE_DISABLED]).flatMap((f) => ['--disable', f]),
  prompt,
];

const OPENAI_CREDENTIALS = ['OPENAI_API_KEY', 'CODEX_API_KEY', 'OPENAI_BASE_URL', 'AZURE_OPENAI_API_KEY', 'OPENAI_ORG_ID', 'OPENAI_PROJECT_ID'];

/** Whether a Codex run with this config and env can only reach the stub:
 *  one provider, at 127.0.0.1, no OpenAI auth, no OpenAI credential in the
 *  env, a fresh CODEX_HOME under the temp dir, every proxy at loopback, and
 *  no system config dir (`/etc/codex`: `config.toml`, `managed_config.toml`
 *  and `requirements.toml` there layer over CODEX_HOME's).
 *  Not covered: macOS managed preferences (the `com.openai.codex` domain,
 *  e.g. an MDM profile) are read from the system, not from HOME, so a temp
 *  HOME does not sandbox them; the tripwire is what catches their effect. */
export function codexPinned(config, env, tmp = tmpdir(), { systemDir = '/etc/codex', exists = existsSync } = {}) {
  const no = (why) => ({ pinned: false, reason: `not pinned: ${why}` });
  if (exists(systemDir)) return no(`${systemDir} exists`);
  if (!/^model_provider = "probe-stub"$/m.test(config)) return no('model_provider is not the stub');
  const urls = config.match(/^base_url = .*$/gm) ?? [];
  if (urls.length !== 1 || !/^base_url = "http:\/\/127\.0\.0\.1:\d+\/v1"$/.test(urls[0])) return no('base_url is not the stub alone');
  if (!/^requires_openai_auth = false$/m.test(config)) return no('provider asks for OpenAI auth');
  if (!/^wire_api = "responses"$/m.test(config)) return no('wire_api is not responses');
  const leaked = OPENAI_CREDENTIALS.filter((k) => env[k] !== undefined);
  if (leaked.length) return no(`env carries ${leaked.join(', ')}`);
  if (!env.CODEX_HOME || !resolve(env.CODEX_HOME).startsWith(resolve(tmp) + sep)) return no('CODEX_HOME is not under the temp dir');
  for (const k of ['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy']) if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(env[k] ?? '')) return no(`${k} is not the tripwire`);
  return { pinned: true, reason: null };
}

/** `$GROK_HOME/config.toml`: a custom model on the stub's Messages API with
 *  a dummy key, updates and telemetry off, and the stub and control MCP
 *  servers (the control tells an inconclusive run from drift, as for Codex). */
export function grokConfig({ port, node = process.execPath, stub = STUB_MCP }) {
  return [
    '[cli]',
    'auto_update = false',
    '',
    '[features]',
    'telemetry = false',
    '',
    '[models]',
    `default = ${toml(GROK_MODEL)}`,
    '',
    `[model.${GROK_MODEL}]`,
    `model = ${toml(MODEL)}`,
    `base_url = ${toml(`http://127.0.0.1:${port}/v1`)}`,
    'name = "Probe stub"',
    `api_key = ${toml(DUMMY_KEY)}`,
    'api_backend = "messages"',
    'context_window = 200000',
    '',
    `[mcp_servers.${SERVER}]`,
    `command = ${toml(node)}`,
    `args = [${toml(stub)}]`,
    '',
    `[mcp_servers.${CONTROL_SERVER}]`,
    `command = ${toml(node)}`,
    `args = [${toml(stub)}, ${toml('--no-meta')}]`,
    '',
  ].join('\n');
}

/** A complete Responses event stream for one assistant message (the shape
 *  Hollow's Codex fixture answers with, `codex_transform.rs`). */
export function responsesSse(text = 'done') {
  const events = [
    { type: 'response.created', response: { id: 'resp_probe' } },
    { type: 'response.output_item.done', item: { type: 'message', role: 'assistant', id: 'msg_probe', content: [{ type: 'output_text', text }] } },
    {
      type: 'response.completed',
      response: { id: 'resp_probe', usage: { input_tokens: 3, input_tokens_details: null, output_tokens: 4, output_tokens_details: null, total_tokens: 7 } },
    },
  ];
  return events.map((e) => `event: ${e.type}\ndata: ${JSON.stringify(e)}\n\n`).join('');
}

/** The stub's answer to Codex: every Responses request is a main one and
 *  gets a plain "done"; anything else falls through to cli-probe's stub. */
export function codexRespond(path, body, state) {
  if (!path.endsWith('/responses')) return null;
  state.main.push(body);
  return { headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' }, body: responsesSse('done') };
}

// ------------------------------------------------------------------ runs

/** Runs `fn(root)` in a fresh temp root holding `tmp/`, which each run
 *  passes as the child's TMPDIR (childEnv's `extra` overrides the parent's)
 *  so the CLI's scratch paths stay under the root too. */
async function inTemp(prefix, keep, fn) {
  const root = mkdtempSync(join(tmpdir(), prefix));
  try {
    mkdirSync(join(root, 'tmp'));
    return await fn(root);
  } finally {
    if (!keep) rmSync(root, { recursive: true, force: true });
  }
}

/** Starts the tripwire and the stub, runs `fn(stub, tripwire)`, closes both
 *  and returns the raw facts: `fn`'s run, the stub's main bodies and counts,
 *  the tripwire's targets. `fn` may return `{skip: reason}`. */
async function withStub(stubOptions, fn) {
  const tripwire = await startTripwire();
  let stub = null;
  let out;
  try {
    stub = await startStub(stubOptions);
    out = await fn(stub, tripwire);
  } finally {
    await stub?.close();
    await tripwire.close();
  }
  if (out?.skip) return { probed: false, reason: out.skip };
  const { run, ...extra } = out;
  return {
    probed: true,
    exitCode: run.exitCode,
    timedOut: run.timedOut,
    spawnError: run.spawnError,
    stderrBytes: Buffer.byteLength(run.stderr ?? ''),
    tripwireHits: tripwire.hits.length,
    tripwireTargets: uniqueSorted(tripwire.hits.map((h) => h.target)),
    // Kind and host only (no method or path), for isBlockedTelemetry.
    tripwireHitList: tripwire.hits.map((h) => ({ kind: h.kind, target: h.target })),
    bodies: stub.state.main,
    unparsedBodies: stub.records.filter((r) => r.body && typeof r.body === 'object' && 'unparsedChars' in r.body).length,
    // Event types only (Codex `--json`, Claude stream-json), never their text.
    eventTypes: uniqueSorted((run.events ?? []).map((e) => e?.type).filter((t) => typeof t === 'string')),
    sideRequests: stub.state.side,
    endpoints: stub.state.endpoints,
    stubErrors: stub.state.errors,
    ...extra,
  };
}

async function cliVersion(bin, env, cwd) {
  const r = await runClaude({ bin, args: ['--version'], cwd, env, timeoutMs: VERSION_TIMEOUT_MS });
  return r.exitCode === 0 ? (r.stdout.trim().split('\n')[0] ?? '').slice(0, 80) : null;
}

const mcpServers = () => ({ mcpServers: { [SERVER]: { type: 'stdio', command: process.execPath, args: [STUB_MCP] } } });

/** One `claude -p` with the stub MCP server and tool search on. It returns
 *  the run's temp root as created and resolved (`root`, `realRoot`), which
 *  the report replaces with `<ROOT>` in the system text before comparing
 *  modes: the working directory it names differs by the mkdtemp suffix. */
export function runClaudeMode(mode, { bin = DEFAULT_BIN, keep = false, timeoutMs = TIMEOUT_MS } = {}) {
  return inTemp('probe-agent-cli-claude-', keep, async (root) => {
    const realRoot = realpathSync(root);
    const configDir = join(root, 'config');
    const cwd = join(root, 'project');
    mkdirSync(configDir);
    mkdirSync(cwd);
    const mcpConfig = join(root, 'mcp.json');
    writeFileSync(mcpConfig, JSON.stringify(mcpServers()));
    const raw = await withStub({}, async (stub, tripwire) => {
      const env = childEnv({ configDir, stubPort: stub.port, proxyPort: tripwire.port, extra: { HOME: root, TMPDIR: join(root, 'tmp'), ENABLE_TOOL_SEARCH: 'true' } });
      const args = ['-p', 'probe', '--output-format', 'stream-json', '--verbose', '--model', MODEL, '--permission-mode', mode, '--mcp-config', mcpConfig, '--strict-mcp-config'];
      const run = await runClaude({ bin, args, cwd, env, timeoutMs });
      const init = run.events.find((e) => e?.type === 'system' && e.subtype === 'init');
      const mcpStatus = (Array.isArray(init?.mcp_servers) ? init.mcp_servers : []).map((s) => `${s?.name}:${s?.status}`);
      return { run, events: summariseEvents(run.events), mcpStatus };
    });
    return { mode, root, realRoot, ...raw };
  });
}

/** One `codex exec` against the stub's Responses API, with a fresh
 *  CODEX_HOME whose config.toml names the stub provider and MCP servers;
 *  `disable` picks the `--disable` set (codexDisableMode). */
export function runCodex({ bin = CODEX_BIN, keep = false, timeoutMs = TIMEOUT_MS, disable = 'hollow' } = {}) {
  return inTemp('probe-agent-cli-codex-', keep, async (root) => {
    const home = join(root, 'codex-home');
    const cwd = join(root, 'project');
    mkdirSync(home);
    mkdirSync(cwd);
    return withStub({ respond: codexRespond }, async (stub, tripwire) => {
      const config = codexConfig({ port: stub.port });
      const env = childEnv({ configDir: join(root, 'unused-claude-config'), stubPort: stub.port, proxyPort: tripwire.port, extra: { HOME: root, TMPDIR: join(root, 'tmp'), CODEX_HOME: home } });
      const pin = codexPinned(config, env);
      if (!pin.pinned) return { skip: pin.reason };
      writeFileSync(join(home, 'config.toml'), config);
      const version = await cliVersion(bin, env, cwd);
      return { run: await runClaude({ bin, args: codexArgs('probe', { disable }), cwd, env, timeoutMs }), version, codexDisable: disable };
    });
  });
}

/** One `grok -p` against the stub's Messages API through a custom model
 *  (Grok documents `[model.<name>] base_url` and `api_backend = "messages"`). */
export function runGrok({ bin = GROK_BIN, keep = false, timeoutMs = TIMEOUT_MS } = {}) {
  return inTemp('probe-agent-cli-grok-', keep, async (root) => {
    const home = join(root, 'grok-home');
    const cwd = join(root, 'project');
    mkdirSync(home);
    mkdirSync(cwd);
    const isMain = (body) => Array.isArray(body?.tools) && body.tools.length > 0;
    return withStub({ isMain }, async (stub, tripwire) => {
      writeFileSync(join(home, 'config.toml'), grokConfig({ port: stub.port }));
      const env = childEnv({
        configDir: join(root, 'unused-claude-config'),
        stubPort: stub.port,
        proxyPort: tripwire.port,
        extra: {
          HOME: root,
          TMPDIR: join(root, 'tmp'),
          GROK_HOME: home,
          // A dummy key, so a headless run never starts a browser sign-in.
          XAI_API_KEY: 'xai-dummy-probe',
          GROK_DISABLE_AUTOUPDATER: '1',
          GROK_TELEMETRY_ENABLED: '0',
          GROK_TELEMETRY_TRACE_UPLOAD: '0',
          GROK_TELEMETRY_MIXPANEL_ENABLED: '0',
          GROK_FEEDBACK_ENABLED: '0',
          GROK_MEMORY: '0',
          GROK_SUBAGENTS: '0',
          GROK_WEB_FETCH: '0',
          GROK_WORKFLOWS: '0',
        },
      });
      const version = await cliVersion(bin, env, cwd);
      const args = ['-p', 'probe', '--output-format', 'json', '--no-auto-update', '-m', GROK_MODEL, '--max-turns', '1', '--cwd', cwd];
      return { run: await runClaude({ bin, args, cwd, env, timeoutMs }), version };
    });
  });
}

// --------------------------------------------------------------------- CLI

const USAGE = `usage: node scripts/agent-eval/probe-agent-cli.mjs [--json out.json]
       [--agent claude|codex|grok]... [--claude-bin <path>] [--codex-bin <path>] [--grok-bin <path>] [--keep]
env PROBE_CODEX_DISABLE=probe-only: Codex gets only the probe's --disable flags`;

export function parseArgs(argv, env = process.env) {
  const o = { json: null, agents: [], claudeBin: DEFAULT_BIN, codexBin: CODEX_BIN, grokBin: GROK_BIN, keep: false, codexDisable: codexDisableMode(env) };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const val = () => {
      const v = argv[++i];
      if (v === undefined || v.startsWith('--')) throw new Error(`${a} needs a value`);
      return v;
    };
    if (a === '--json') o.json = resolve(val());
    else if (a === '--agent') {
      const v = val();
      if (!AGENTS.includes(v)) throw new Error(`--agent is one of ${AGENTS.join(', ')}`);
      if (!o.agents.includes(v)) o.agents.push(v);
    } else if (a === '--claude-bin') o.claudeBin = val();
    else if (a === '--codex-bin') o.codexBin = val();
    else if (a === '--grok-bin') o.grokBin = val();
    else if (a === '--keep') o.keep = true;
    else throw new Error(`unknown argument ${a}`);
  }
  if (!o.agents.length) o.agents = [...AGENTS];
  return o;
}

const line = (name, r, agent = name) => {
  const telemetry = (r.tripwireHitList ?? []).filter((h) => isBlockedTelemetry(h, agent)).length;
  const isolation = r.probed === false ? '' : ` isolation ok=${(r.tripwireHits ?? 0) === 0}${telemetry ? ` (blocked telemetry ${telemetry})` : ''}`;
  return `${name}: probed=${r.probed ?? true} exit=${r.exitCode ?? '-'} requests=${r.bodies?.length ?? 0} trip=${r.tripwireHits ?? 0}${isolation}${r.reason ? ` (${r.reason})` : ''}`;
};

export async function main(argv) {
  let o;
  try {
    o = parseArgs(argv);
  } catch (e) {
    console.error(`${e.message}\n${USAGE}`);
    return 2;
  }
  const input = {};
  // Codex's blocked telemetry fails its run but doesn't stop the others.
  const hard = () =>
    [...(input.claude?.runs ?? []).map((r) => ['claude', r]), ['codex', input.codex ?? {}], ['grok', input.grok ?? {}]].flatMap(([a, r]) => hardHits(r, a));
  const tripped = () => hard().length > 0;
  const stopped = { probed: false, reason: 'skipped: the tripwire saw traffic' };
  if (o.agents.includes('claude')) {
    input.claude = { probed: true, runs: [] };
    for (const mode of MODES) {
      const r = await runClaudeMode(mode, { bin: o.claudeBin, keep: o.keep });
      input.claude.runs.push(r);
      console.error(line(`claude ${mode}`, r, 'claude'));
      if (tripped()) break;
    }
  }
  for (const [name, run, bin] of [['codex', runCodex, o.codexBin], ['grok', runGrok, o.grokBin]]) {
    if (!o.agents.includes(name)) continue;
    if (tripped()) {
      input[name] = stopped;
      continue;
    }
    input[name] = await run({ bin, keep: o.keep, disable: o.codexDisable });
    console.error(line(name, input[name]));
  }
  const report = summarise(input);
  const text = JSON.stringify(report, null, 2);
  if (o.json) writeFileSync(o.json, text + '\n');
  console.log(text);
  const code = exitCodeFor(report, o.agents);
  if (code === 4) console.error(`stopping: the tripwire saw ${uniqueSorted(hard().map((h) => `${h.kind} ${h.target}`)).join(', ')}`);
  return code;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  main(process.argv.slice(2)).then((code) => process.exit(code));
}
