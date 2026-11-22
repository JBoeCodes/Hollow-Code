// The stdio MCP server probe-agent-cli.mjs (OF22) attaches to each CLI:
// newline-delimited JSON-RPC on stdin/stdout, no dependencies. It lists three
// tools; only `probe_flagged` carries the tool-level `_meta` Hollow puts on
// its hot app tools (`app_mcp::ALWAYS_LOAD`, HB05). A call answers "ok".
//
// With `--no-meta` it is the control server instead: two tools, `probe_ctl_a`
// and `probe_ctl_b`, and no `_meta` anywhere. A CLI that offers the control
// tools but not `probe_flagged` dropped the flagged one (drift); one that
// offers neither never reached the MCP tools at all (inconclusive).
//
// `handle(message, tools)` is exported for the tests; the stdio loop runs
// only when this file is the main module.
import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

export const FLAGGED_META = { 'anthropic/alwaysLoad': true };
/** The one protocol version the stub speaks, whatever the client asks for. */
export const PROTOCOL_VERSION = '2025-06-18';
const schema = { type: 'object', properties: {}, additionalProperties: false };

export const TOOLS = [
  { name: 'probe_flagged', description: 'Probe tool flagged to load with the session.', inputSchema: schema, _meta: FLAGGED_META },
  { name: 'probe_other_a', description: 'Probe tool without a flag (a).', inputSchema: schema },
  { name: 'probe_other_b', description: 'Probe tool without a flag (b).', inputSchema: schema },
];

export const CONTROL_TOOLS = [
  { name: 'probe_ctl_a', description: 'Control probe tool (a).', inputSchema: schema },
  { name: 'probe_ctl_b', description: 'Control probe tool (b).', inputSchema: schema },
];

/** The reply to one JSON-RPC message, or null when none is due (a
 *  notification, or a client's reply to a server request). `tools` is the
 *  list `tools/list` answers with. */
export function handle(message, tools = TOOLS) {
  if (!message || typeof message !== 'object' || Array.isArray(message)) return null;
  const { id, method } = message;
  if (id === undefined || id === null || typeof method !== 'string') return null;
  const ok = (result) => ({ jsonrpc: '2.0', id, result });
  switch (method) {
    case 'initialize':
      return ok({
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: tools === CONTROL_TOOLS ? 'probe-control' : 'probe', version: '0.0.0' },
      });
    case 'ping':
      return ok({});
    case 'tools/list':
      return ok({ tools });
    case 'tools/call':
      return ok({ content: [{ type: 'text', text: 'ok' }] });
    case 'resources/list':
      return ok({ resources: [] });
    case 'resources/templates/list':
      return ok({ resourceTemplates: [] });
    case 'prompts/list':
      return ok({ prompts: [] });
    default:
      return { jsonrpc: '2.0', id, error: { code: -32601, message: `method not found: ${method}` } };
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const tools = process.argv.slice(2).includes('--no-meta') ? CONTROL_TOOLS : TOOLS;
  const lines = createInterface({ input: process.stdin });
  lines.on('line', (line) => {
    if (!line.trim()) return;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      return;
    }
    const reply = handle(message, tools);
    if (reply) process.stdout.write(JSON.stringify(reply) + '\n');
  });
  lines.on('close', () => process.exit(0));
}
