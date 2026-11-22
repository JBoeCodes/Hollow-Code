// OF22: the $0 agent-CLI probe (scripts/agent-eval/probe-agent-cli.mjs) and
// its stub MCP server. The request bodies are hand-written fixtures in the
// shapes the CLIs send; nothing here spawns a CLI or opens a socket.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { childEnv } from './agent-eval/cli-probe.mjs';
import { CONTROL_TOOLS as STUB_CONTROL_TOOLS, FLAGGED_META, handle } from './agent-eval/fixtures/probe/stub-mcp.mjs';
import {
  CODEX_DISABLED,
  CODEX_PROBE_DISABLED,
  bareName,
  classifyTools,
  codexArgs,
  codexConfig,
  codexDisableMode,
  codexPinned,
  codexRespond,
  exitCodeFor,
  grokConfig,
  hardHits,
  isBlockedTelemetry,
  metaOutcome,
  modeDiff,
  normaliseRoot,
  parseArgs,
  probeNamesIn,
  responsesSse,
  slugOf,
  summarise,
  systemDiff,
  toolNames,
  toolTypes,
} from './agent-eval/probe-agent-cli.mjs';

const fixture = (name) => JSON.parse(readFileSync(new URL(`./agent-eval/fixtures/probe/${name}.json`, import.meta.url), 'utf8'));
const acceptEdits = fixture('mode-acceptEdits').body;
const bypass = fixture('mode-bypassPermissions').body;

test('classifyTools: the alwaysLoad tool is sent in full, the other two are deferred, built-ins are not classified', () => {
  const body = fixture('always-load-request').body;
  assert.deepEqual(classifyTools(body), { alwaysLoaded: ['probe_flagged'], deferred: ['probe_other_a', 'probe_other_b'] });
  assert.equal(bareName('mcp__probe__probe_other_a'), 'probe_other_a');
  assert.equal(bareName('Bash'), 'Bash');
});

test('classifyTools: a tool sent with defer_loading, and a known one absent altogether, count as deferred', () => {
  const body = {
    tools: [
      { name: 'Bash', description: 'x' },
      { name: 'mcp__probe__probe_flagged', description: 'x' },
      { name: 'mcp__probe__probe_other_a', description: 'x', defer_loading: true },
    ],
    messages: [{ role: 'user', content: 'probe' }],
  };
  assert.deepEqual(classifyTools(body), { alwaysLoaded: ['probe_flagged'], deferred: ['probe_other_a', 'probe_other_b'] });
  // Nothing in full when tool search defers everything, or nothing arrived.
  assert.deepEqual(classifyTools(null), { alwaysLoaded: [], deferred: ['probe_flagged', 'probe_other_a', 'probe_other_b'] });
});

// The fixtures' 311-byte Bash gap is synthetic (hand-written); the live
// 2.1.301 run measured −310. The tests pin the arithmetic, not the number.
test('modeDiff: acceptEdits to bypassPermissions keeps the system text and shortens Bash by the fixtures\' 311 bytes', () => {
  assert.deepEqual(modeDiff(acceptEdits, bypass), { system: 0, tools: { Bash: -311 } });
  assert.deepEqual(modeDiff(bypass, acceptEdits), { system: 0, tools: { Bash: 311 } });
  // A string system prompt counts too, and a tool one side lacks counts as 0 bytes there.
  assert.deepEqual(modeDiff({ system: 'ab', tools: [] }, { system: 'abcd', tools: [{ name: 'Read', description: 'xyz' }] }), { system: 2, tools: { Read: 3 } });
});

test('the stub MCP server lists _meta on probe_flagged only', () => {
  const reply = handle({ jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} });
  assert.equal(reply.id, 2);
  const tools = reply.result.tools;
  assert.deepEqual(tools.map((t) => t.name), ['probe_flagged', 'probe_other_a', 'probe_other_b']);
  assert.deepEqual(tools.filter((t) => '_meta' in t).map((t) => t.name), ['probe_flagged']);
  assert.deepEqual(tools[0]._meta, { 'anthropic/alwaysLoad': true });
  assert.deepEqual(FLAGGED_META, { 'anthropic/alwaysLoad': true });
});

test('the control MCP server (--no-meta) lists probe_ctl_a and probe_ctl_b, no _meta', () => {
  const tools = handle({ jsonrpc: '2.0', id: 2, method: 'tools/list', params: {} }, STUB_CONTROL_TOOLS).result.tools;
  assert.deepEqual(tools.map((t) => t.name), ['probe_ctl_a', 'probe_ctl_b']);
  assert.ok(tools.every((t) => !('_meta' in t)));
  assert.equal(handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: {} }, STUB_CONTROL_TOOLS).result.serverInfo.name, 'probe-control');
});

test('the stub MCP server answers initialize with its own protocol version, ignores notifications, refuses unknown methods', () => {
  // Always 2025-06-18, whatever the client asks for (never echoed).
  for (const asked of ['2025-06-18', '2024-11-05', '2099-01-01', undefined]) {
    const init = handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: asked, capabilities: {} } });
    assert.equal(init.result.protocolVersion, '2025-06-18');
  }
  const init = handle({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {} } });
  assert.deepEqual(init.result.capabilities, { tools: {} });
  assert.equal(handle({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  assert.equal(handle({ jsonrpc: '2.0', id: 3, result: {} }), null);
  assert.equal(handle({ jsonrpc: '2.0', id: 4, method: 'nope' }).error.code, -32601);
  assert.equal(handle({ jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'probe_flagged' } }).result.content[0].text, 'ok');
});

/** A raw run as the runners return it. */
const rawRun = (extra) => ({ probed: true, exitCode: 0, timedOut: false, spawnError: null, stderrBytes: 10, tripwireHits: 0, tripwireTargets: [], tripwireHitList: [], sideRequests: 1, endpoints: { 'POST /v1/messages': 2 }, stubErrors: 0, ...extra });
/** A Claude raw run that met every precondition (stub connected, mode in force). */
const claudeRun = (mode, body, extra = {}) =>
  rawRun({ mode, bodies: [body], events: { version: '2.1.301', initMode: mode, apiErrorSeen: false }, mcpStatus: ['probe:connected'], ...extra });
const responsesTool = (name) => ({ type: 'function', name, description: 'x' });
const codexBody = {
  model: 'm',
  instructions: 'secret instructions text',
  tools: [
    { type: 'function', name: 'shell', description: 'x' },
    { type: 'function', name: 'mcp__probe__probe_flagged', description: 'Probe tool flagged to load with the session.' },
    responsesTool('mcp__probe__probe_other_a'),
    responsesTool('mcp__control__probe_ctl_a'),
    responsesTool('mcp__control__probe_ctl_b'),
  ],
};

test('summarise: names and byte sizes only, the mode diff, and Codex tolerance from a Responses body', () => {
  const report = summarise({
    claude: { probed: true, runs: [claudeRun('acceptEdits', acceptEdits), claudeRun('bypassPermissions', bypass)] },
    codex: rawRun({ bodies: [codexBody, { tools: [], input: [] }], version: 'codex-cli 0.163.0', unparsedBodies: 1, eventTypes: ['item.completed', 'thread.started', 'turn.completed', 'turn.started'] }),
  });
  assert.equal(report.ok, true);
  assert.deepEqual(report.drift, []);
  assert.equal(report.tripwireHits, 0);
  assert.equal(exitCodeFor(report, ['claude', 'codex']), 0);
  const [a, b] = report.claude.runs;
  assert.deepEqual([a.alwaysLoaded, a.deferred, a.alwaysLoadHonoured, a.toolSearchOn], [['probe_flagged'], ['probe_other_a', 'probe_other_b'], true, true]);
  assert.deepEqual([a.outcome, a.unmet, a.isolationOk], ['honoured', [], true]);
  assert.equal(b.mode, 'bypassPermissions');
  assert.equal(a.systemBytes, b.systemBytes);
  assert.deepEqual(report.claude.modeDiff, {
    from: 'acceptEdits',
    to: 'bypassPermissions',
    system: 0,
    tools: { Bash: -311 },
    systemIdentical: true,
    systemFirstDiffOffset: null,
    systemDiffBlock: null,
  });
  assert.deepEqual([report.codex.probeToolsListed, report.codex.controlToolsListed], [['probe_flagged', 'probe_other_a'], ['probe_ctl_a', 'probe_ctl_b']]);
  assert.deepEqual([report.codex.probeToolsSeen, report.codex.controlToolsSeen], [['probe_flagged', 'probe_other_a'], ['probe_ctl_a', 'probe_ctl_b']]);
  assert.deepEqual([report.codex.outcome, report.codex.flaggedListed, report.codex.metaForwarded, report.codex.toleratesMeta], ['tolerated', true, false, true]);
  assert.deepEqual([report.codex.codexDisable, report.codex.updatePlanListed], ['hollow', false]);
  assert.ok(!('updatePlanListed' in summarise({ grok: rawRun({ bodies: [codexBody] }) }).grok));
  assert.deepEqual([report.codex.toolTypes, report.codex.unparsedBodies, report.codex.eventTypes], [['function'], 1, ['item.completed', 'thread.started', 'turn.completed', 'turn.started']]);
  assert.deepEqual(report.grok, { probed: false, reason: 'not asked' });
  const text = JSON.stringify(report);
  for (const s of ['Executes a given bash command', 'You are Claude Code', 'secret instructions', 'Probe tool flagged', 'system-reminder']) assert.ok(!text.includes(s), `${s} leaked into the report`);
  assert.ok(!('bodies' in report.codex) && !('bodies' in a));
});

test('summarise: a flagged tool deferred like the rest is drift; a tripwire hit or a failed exit is not ok', () => {
  const allDeferred = { ...bypass, tools: bypass.tools.filter((t) => !t.name.startsWith('mcp__')) };
  const report = summarise({
    claude: { probed: true, runs: [claudeRun('acceptEdits', allDeferred), claudeRun('bypassPermissions', allDeferred)] },
    codex: rawRun({ bodies: [], exitCode: 1 }),
    grok: rawRun({ bodies: [{ tools: [{ name: 'x' }] }], tripwireHits: 2, tripwireTargets: ['api.x.ai:443'] }),
  });
  assert.deepEqual(report.drift, ['claude.alwaysLoad.acceptEdits', 'claude.alwaysLoad.bypassPermissions']);
  assert.deepEqual(report.claude.runs.map((r) => [r.outcome, r.ok]), [['drift', true], ['drift', true]]);
  assert.deepEqual([report.codex.ok, report.codex.outcome], [false, 'failed']);
  assert.equal(report.grok.ok, false);
  assert.deepEqual([report.tripwireHits, report.hardTripwireHits, report.blockedTelemetryHits], [2, 2, 0]);
  assert.equal(report.ok, false);
  assert.equal(exitCodeFor(report), 4);
});

test('summarise: a Claude run without the stub connected, tool search on, or its mode in force is inconclusive, not drift', () => {
  const allDeferred = { ...bypass, tools: bypass.tools.filter((t) => !t.name.startsWith('mcp__')) };
  const noSearch = { ...acceptEdits, tools: acceptEdits.tools.filter((t) => t.name !== 'ToolSearch') };
  const report = summarise({
    claude: {
      probed: true,
      runs: [
        claudeRun('acceptEdits', noSearch, { mcpStatus: ['probe:failed'] }),
        claudeRun('bypassPermissions', allDeferred, { events: { version: '2.1.301', initMode: 'default', apiErrorSeen: false } }),
      ],
    },
  });
  assert.deepEqual(report.claude.runs.map((r) => [r.outcome, r.ok, r.unmet]), [['inconclusive', false, ['mcp', 'toolSearch']], ['inconclusive', false, ['mode']]]);
  assert.deepEqual(report.drift, []);
  assert.equal(report.ok, false);
  assert.equal(exitCodeFor(report, ['claude']), 1);
});

test('summarise: the mode diff compares system texts with each run\'s temp root as <ROOT>', () => {
  const withCwd = (body, cwd) => ({ ...body, system: [body.system[0], { type: 'text', text: `Working directory: ${cwd}/project\nEnv: ${cwd}` }] });
  const rootA = '/var/folders/ab/T/probe-agent-cli-claude-AAAAAA';
  const rootB = '/var/folders/ab/T/probe-agent-cli-claude-BBBBBB';
  // One run names its resolved root (/private/var/…), the other the root as created.
  const a = withCwd(bypass, `/private${rootA}`);
  const b = withCwd(bypass, rootB);
  const runs = [claudeRun('acceptEdits', a, { root: rootA, realRoot: `/private${rootA}` }), claudeRun('bypassPermissions', b, { root: rootB, realRoot: `/private${rootB}` })];
  const report = summarise({ claude: { probed: true, runs } });
  assert.deepEqual(report.claude.modeDiff, {
    from: 'acceptEdits',
    to: 'bypassPermissions',
    system: 0,
    tools: {},
    systemIdentical: true,
    systemFirstDiffOffset: null,
    systemDiffBlock: null,
  });
  // Without the roots the texts differ, by the 8-byte `/private` twice,
  // first in block 1 just after `Working directory: /`.
  const bare = summarise({ claude: { probed: true, runs: runs.map((r) => ({ ...r, root: undefined, realRoot: undefined })) } });
  const { systemIdentical, system, systemFirstDiffOffset, systemDiffBlock } = bare.claude.modeDiff;
  assert.deepEqual([systemIdentical, system, systemDiffBlock], [false, -16, 1]);
  assert.equal(systemFirstDiffOffset, bypass.system[0].text.length + 'Working directory: /'.length);
  assert.equal(normaliseRoot(`/private${rootA}/x ${rootA}/y`, [rootA, `/private${rootA}`]), '<ROOT>/x <ROOT>/y');
  assert.equal(normaliseRoot('no root here', []), 'no root here');
  assert.equal(JSON.stringify(report).includes('AAAAAA'), false);
});

test('normaliseRoot: a root\'s slug form and any UUID normalise too, so per-run ids leave the system texts identical', () => {
  const rootA = '/var/folders/ab/T/probe-agent-cli-claude-AAAAAA';
  const rootB = '/var/folders/ab/T/probe-agent-cli-claude-BBBBBB';
  const text = (root, uuid) => `Memory: ~/.claude/projects/${slugOf(`/private${root}/project`)}/memory\nSession ${uuid}\nCwd: ${root}/project`;
  const a = { system: [{ type: 'text', text: 'You are a probe.' }, { type: 'text', text: text(rootA, '0f599db5-351d-4dc0-8a1b-2c3d4e5f6a7b') }] };
  const b = { system: [{ type: 'text', text: 'You are a probe.' }, { type: 'text', text: text(rootB, 'A1B2C3D4-E5F6-4A7B-8C9D-0E1F2A3B4C5D') }] };
  const roots = (r) => [r, `/private${r}`];
  assert.equal(slugOf('/private/var/x_y.z'), '-private-var-x-y-z');
  assert.deepEqual(systemDiff(a, b, roots(rootA), roots(rootB)), { systemIdentical: true, systemFirstDiffOffset: null, systemDiffBlock: null });
  assert.deepEqual(modeDiff(a, b, roots(rootA), roots(rootB)), { system: 0, tools: {} });
  assert.equal(normaliseRoot(text(rootA, '0f599db5-351d-4dc0-8a1b-2c3d4e5f6a7b'), roots(rootA)), 'Memory: ~/.claude/projects/<ROOT>-project/memory\nSession <UUID>\nCwd: <ROOT>/project');
  // Without the roots only the UUIDs normalise; the slugs still differ, in block 1.
  const bare = systemDiff(a, b);
  assert.deepEqual([bare.systemIdentical, bare.systemDiffBlock], [false, 1]);
  assert.equal(bare.systemFirstDiffOffset, 'You are a probe.'.length + text(rootA, '').indexOf('AAAAAA'));
  // A string system prompt is one block.
  assert.deepEqual(systemDiff({ system: 'ab' }, { system: 'ac' }), { systemIdentical: false, systemFirstDiffOffset: 1, systemDiffBlock: 0 });
});

test('Codex and Grok outcomes: control tools tell inconclusive from drift', () => {
  assert.equal(metaOutcome([], []), 'inconclusive');
  assert.equal(metaOutcome(['probe_flagged'], []), 'inconclusive');
  assert.equal(metaOutcome(['probe_other_a'], ['probe_ctl_a']), 'drift');
  assert.equal(metaOutcome(['probe_flagged', 'probe_other_a'], ['probe_ctl_a', 'probe_ctl_b']), 'tolerated');
  const shell = { tools: [{ type: 'function', name: 'shell' }] };
  const controlOnly = { tools: [responsesTool('mcp__control__probe_ctl_a'), responsesTool('mcp__control__probe_ctl_b'), responsesTool('mcp__probe__probe_other_a')] };
  const report = summarise({ codex: rawRun({ bodies: [shell] }), grok: rawRun({ bodies: [controlOnly] }) });
  assert.deepEqual([report.codex.outcome, report.codex.ok, report.codex.toleratesMeta], ['inconclusive', false, false]);
  assert.deepEqual([report.codex.probeToolsSeen, report.codex.controlToolsSeen], [[], []]);
  assert.deepEqual([report.grok.outcome, report.grok.ok, report.grok.toleratesMeta], ['drift', true, false]);
  // Inconclusive is not drift; it fails the run, but drift elsewhere outranks it.
  assert.deepEqual(report.drift, ['grok.meta']);
  assert.equal(exitCodeFor(report, ['codex', 'grok']), 3);
  assert.equal(exitCodeFor(summarise({ codex: rawRun({ bodies: [shell] }) }), ['codex']), 1);
  assert.equal(exitCodeFor(summarise({ grok: rawRun({ bodies: [controlOnly] }) }), ['grok']), 3);
  // Drift with an agent asked for but not probed is still 3; a hard tripwire hit still 4.
  assert.equal(exitCodeFor(summarise({ grok: rawRun({ bodies: [controlOnly] }) }), ['codex', 'grok']), 3);
  assert.equal(exitCodeFor(summarise({ grok: rawRun({ bodies: [controlOnly], tripwireHits: 1, tripwireHitList: [{ kind: 'connect', target: 'api.x.ai:443' }] }) }), ['grok']), 4);
  assert.deepEqual(toolTypes({ tools: [{ type: 'function' }, { type: 'namespace', tools: [{ type: 'function' }, { type: 'custom' }] }, { name: 'x' }] }), ['custom', 'function', 'namespace']);
});

test('Codex and Grok outcomes rest on tools listed in `tools`; names only mentioned in text are informational', () => {
  // Control tools named only in the instructions, flagged tool listed: nothing proves MCP tools were offered.
  const mentionedControl = { instructions: 'mcp__control__probe_ctl_a mcp__control__probe_ctl_b', tools: [responsesTool('mcp__probe__probe_flagged')] };
  const r1 = summarise({ codex: rawRun({ bodies: [mentionedControl] }) }).codex;
  assert.deepEqual([r1.outcome, r1.ok, r1.controlToolsListed, r1.controlToolsSeen], ['inconclusive', false, [], ['probe_ctl_a', 'probe_ctl_b']]);
  // Control tools listed, flagged tool only named in the input: drift, not tolerated.
  const mentionedFlagged = { input: [{ role: 'user', content: 'probe_flagged' }], tools: [responsesTool('mcp__control__probe_ctl_a'), responsesTool('mcp__control__probe_ctl_b')] };
  const r2 = summarise({ codex: rawRun({ bodies: [mentionedFlagged] }) }).codex;
  assert.deepEqual([r2.outcome, r2.probeToolsListed, r2.probeToolsSeen, r2.flaggedListed], ['drift', [], ['probe_flagged'], false]);
  // A Codex namespace group lists its tools too.
  const grouped = { tools: [{ type: 'namespace', name: 'mcp__probe__', tools: [{ type: 'function', name: 'probe_flagged' }] }, responsesTool('mcp__control__probe_ctl_a'), { type: 'function', name: 'update_plan' }] };
  const r3 = summarise({ codex: rawRun({ bodies: [grouped], codexDisable: 'probe-only' }) }).codex;
  assert.deepEqual([r3.outcome, r3.updatePlanListed, r3.codexDisable], ['tolerated', true, 'probe-only']);
});

test('blocked telemetry (CONNECT ab.chatgpt.com:443 from Codex only) fails the agent but neither stops the probe nor exits 4', () => {
  const telemetry = { kind: 'connect', target: 'ab.chatgpt.com:443' };
  assert.equal(isBlockedTelemetry(telemetry, 'codex'), true);
  for (const agent of ['claude', 'grok', undefined]) assert.equal(isBlockedTelemetry(telemetry, agent), false, String(agent));
  for (const other of [{ kind: 'request', target: 'ab.chatgpt.com:443' }, { kind: 'connect', target: 'ab.chatgpt.com:80' }, { kind: 'connect', target: 'api.openai.com:443' }, { kind: 'request', target: 'ab.chatgpt.com' }]) {
    assert.equal(isBlockedTelemetry(other, 'codex'), false, JSON.stringify(other));
  }
  const codex = rawRun({ bodies: [codexBody], tripwireHits: 1, tripwireTargets: ['ab.chatgpt.com:443'], tripwireHitList: [telemetry] });
  // main() stops the remaining agents only on hard hits: none here.
  assert.deepEqual(hardHits(codex, 'codex'), []);
  // The same hit from Grok or Claude is hard: exit 4.
  assert.deepEqual(hardHits(codex, 'grok'), [telemetry]);
  const grokHit = summarise({ grok: codex });
  assert.deepEqual([grokHit.grok.blockedTelemetryHits, grokHit.grok.hardTripwireHits, grokHit.hardTripwireHits], [0, 1, 1]);
  assert.equal(exitCodeFor(grokHit, ['grok']), 4);
  const claudeHit = summarise({ claude: { probed: true, runs: [claudeRun('acceptEdits', acceptEdits, { tripwireHits: 1, tripwireHitList: [telemetry] }), claudeRun('bypassPermissions', bypass)] } });
  assert.deepEqual([claudeHit.hardTripwireHits, exitCodeFor(claudeHit, ['claude'])], [1, 4]);
  const grok = rawRun({ bodies: [codexBody] });
  const report = summarise({ codex, grok });
  assert.deepEqual([report.codex.ok, report.codex.isolationOk, report.codex.outcome], [false, false, 'tolerated']);
  assert.deepEqual([report.codex.blockedTelemetry, report.codex.blockedTelemetryHits, report.codex.hardTripwireHits], [['connect ab.chatgpt.com:443'], 1, 0]);
  assert.deepEqual([report.tripwireHits, report.blockedTelemetryHits, report.hardTripwireHits], [1, 1, 0]);
  assert.equal(report.grok.ok, true);
  assert.equal(exitCodeFor(report, ['codex', 'grok']), 1);
  // Any other hit beside it is hard: exit 4, and main() stops there.
  const plain = { kind: 'request', target: 'ab.chatgpt.com' };
  const both = rawRun({ bodies: [codexBody], tripwireHits: 2, tripwireHitList: [telemetry, plain] });
  assert.deepEqual(hardHits(both, 'codex'), [plain]);
  assert.equal(exitCodeFor(summarise({ codex: both, grok }), ['codex', 'grok']), 4);
});

test('an agent asked for but not probed fails the exit; one not asked does not', () => {
  const claude = { probed: true, runs: [claudeRun('acceptEdits', acceptEdits), claudeRun('bypassPermissions', bypass)] };
  const notPinned = { probed: false, reason: 'not pinned: /etc/codex exists' };
  const report = summarise({ claude, codex: notPinned });
  assert.equal(report.ok, false);
  assert.equal(exitCodeFor(report, ['claude', 'codex']), 1);
  const claudeOnly = summarise({ claude });
  assert.equal(claudeOnly.ok, true);
  assert.equal(exitCodeFor(claudeOnly, ['claude']), 0);
  assert.equal(exitCodeFor(claudeOnly, ['claude', 'grok']), 1);
});

test('the Codex config names only the stub; codexPinned refuses a real credential, another base URL or a non-loopback proxy', () => {
  const config = codexConfig({ port: 4321, node: '/usr/bin/node', stub: '/x/stub-mcp.mjs' });
  assert.match(config, /^model_provider = "probe-stub"$/m);
  assert.match(config, /^base_url = "http:\/\/127\.0\.0\.1:4321\/v1"$/m);
  const server = (name, args) =>
    `[mcp_servers.${name}]\ncommand = "/usr/bin/node"\nargs = [${args}]\nstartup_timeout_sec = 30\nstartup_readiness = "catalog"\nrequired = true\n`;
  assert.ok(config.includes(server('probe', '"/x/stub-mcp.mjs"')));
  assert.ok(config.includes(server('control', '"/x/stub-mcp.mjs", "--no-meta"')));
  // Telemetry off, credentials in the temp CODEX_HOME, all before the first table.
  assert.ok(config.includes('\n[analytics]\nenabled = false\n'));
  assert.ok(config.includes('\n[otel]\nexporter = "none"\ntrace_exporter = "none"\nmetrics_exporter = "none"\n'));
  for (const key of ['cli_auth_credentials_store', 'mcp_oauth_credentials_store']) {
    assert.match(config, new RegExp(`^${key} = "file"$`, 'm'));
    assert.ok(config.indexOf(`${key} = "file"`) < config.indexOf('['), key);
  }
  const home = join(tmpdir(), 'probe-agent-cli-codex-test', 'codex-home');
  const env = childEnv({ configDir: '/c', stubPort: 4321, proxyPort: 5555, extra: { CODEX_HOME: home }, parent: { PATH: '/bin', OPENAI_API_KEY: 'real' } });
  const noEtc = { exists: () => false };
  assert.deepEqual(codexPinned(config, env, tmpdir(), noEtc), { pinned: true, reason: null });
  assert.match(codexPinned(config, { ...env, OPENAI_API_KEY: 'real' }, tmpdir(), noEtc).reason, /^not pinned: env carries OPENAI_API_KEY/);
  assert.match(codexPinned(config.replace('127.0.0.1:4321', 'api.openai.com'), env, tmpdir(), noEtc).reason, /^not pinned: base_url/);
  assert.match(codexPinned(config.replace('requires_openai_auth = false', 'requires_openai_auth = true'), env, tmpdir(), noEtc).reason, /OpenAI auth/);
  assert.match(codexPinned(config, { ...env, HTTPS_PROXY: 'http://proxy.example:8080' }, tmpdir(), noEtc).reason, /HTTPS_PROXY/);
  assert.match(codexPinned(config, { ...env, CODEX_HOME: '/Users/someone/.codex' }, tmpdir(), noEtc).reason, /CODEX_HOME/);
  // A sibling of the temp dir that only shares its prefix is not under it.
  assert.match(codexPinned(config, { ...env, CODEX_HOME: '/tmp/probe-evil/codex-home' }, '/tmp/probe', noEtc).reason, /CODEX_HOME/);
  assert.deepEqual(codexPinned(config, { ...env, CODEX_HOME: '/tmp/probe/codex-home' }, '/tmp/probe', noEtc), { pinned: true, reason: null });
  // A system config dir layers over CODEX_HOME's config: refuse.
  const seen = [];
  const etc = codexPinned(config, env, tmpdir(), { exists: (p) => (seen.push(p), p === '/etc/codex') });
  assert.deepEqual([etc, seen], [{ pinned: false, reason: 'not pinned: /etc/codex exists' }, ['/etc/codex']]);
  assert.ok(CODEX_DISABLED.includes('enable_request_compression') && !CODEX_DISABLED.some((f) => /mcp/.test(f)));
  const args = codexArgs('probe');
  for (const flag of ['--strict-config', '--json', '--ephemeral', '--skip-git-repo-check']) assert.ok(args.includes(flag), flag);
  const after = (xs, flag) => xs.flatMap((a, i) => (xs[i - 1] === flag ? [a] : []));
  const disabled = after(args, '--disable');
  assert.deepEqual(disabled, [...CODEX_DISABLED, ...CODEX_PROBE_DISABLED]);
  assert.equal(CODEX_DISABLED.length, 24);
  for (const f of ['tool_search', 'tool_search_always_defer_mcp_tools', 'runtime_metrics', 'remote_models']) assert.ok(disabled.includes(f), f);
  assert.deepEqual([args[0], args.at(-1)], ['exec', 'probe']);
  // update_plan is forced on in both switches; probe-only drops Hollow's 24.
  const probeOnly = codexArgs('probe', { disable: 'probe-only' });
  for (const xs of [args, probeOnly]) assert.deepEqual(after(xs, '-c'), ['tools.update_plan.enabled=true']);
  assert.deepEqual(after(probeOnly, '--disable'), CODEX_PROBE_DISABLED);
  assert.deepEqual([probeOnly[0], probeOnly.at(-1)], ['exec', 'probe']);
  assert.equal(codexDisableMode({}), 'hollow');
  assert.equal(codexDisableMode({ PROBE_CODEX_DISABLE: '' }), 'hollow');
  assert.equal(codexDisableMode({ PROBE_CODEX_DISABLE: 'probe-only' }), 'probe-only');
  assert.throws(() => codexDisableMode({ PROBE_CODEX_DISABLE: 'none' }), /one of hollow, probe-only/);
  const grok = grokConfig({ port: 4321, node: '/usr/bin/node', stub: '/x/stub-mcp.mjs' });
  assert.match(grok, /^base_url = "http:\/\/127\.0\.0\.1:4321\/v1"$/m);
  assert.match(grok, /^api_backend = "messages"$/m);
  assert.ok(grok.includes('[mcp_servers.control]\ncommand = "/usr/bin/node"\nargs = ["/x/stub-mcp.mjs", "--no-meta"]\n'));
});

test('the stub answers Codex Responses requests with a complete stream and leaves other paths alone', () => {
  const state = { main: [] };
  const reply = codexRespond('/v1/responses', { tools: [] }, state);
  assert.equal(reply.headers['content-type'], 'text/event-stream');
  assert.equal(state.main.length, 1);
  assert.equal(codexRespond('/v1/models', {}, state), null);
  const events = responsesSse('done')
    .trim()
    .split('\n\n')
    .map((e) => JSON.parse(e.split('\n')[1].slice('data: '.length)));
  assert.deepEqual(events.map((e) => e.type), ['response.created', 'response.output_item.done', 'response.completed']);
  assert.equal(events[1].item.content[0].text, 'done');
});

test('tool names in any API shape, and probe names in reminder text', () => {
  assert.deepEqual(toolNames({ tools: [{ name: 'a' }, { type: 'function', function: { name: 'b' } }, { type: 'namespace', name: 'mcp__probe__', tools: [{ name: 'probe_flagged' }] }] }), ['a', 'b', 'mcp__probe__', 'probe_flagged']);
  assert.deepEqual(probeNamesIn({ text: 'tools:\nmcp__probe__probe_other_b\nprobe_flagged, probe' }), ['probe_flagged', 'probe_other_b']);
});

test('bad arguments are refused; every agent by default', () => {
  assert.deepEqual(parseArgs([], {}).agents, ['claude', 'codex', 'grok']);
  assert.equal(parseArgs([], {}).codexDisable, 'hollow');
  assert.equal(parseArgs([], { PROBE_CODEX_DISABLE: 'probe-only' }).codexDisable, 'probe-only');
  assert.throws(() => parseArgs([], { PROBE_CODEX_DISABLE: 'all' }), /PROBE_CODEX_DISABLE/);
  assert.deepEqual(parseArgs(['--agent', 'codex', '--agent', 'codex']).agents, ['codex']);
  assert.throws(() => parseArgs(['--agent', 'gemini']), /one of/);
  assert.throws(() => parseArgs(['--json']), /needs a value/);
  assert.throws(() => parseArgs(['--nope']), /unknown argument/);
});
