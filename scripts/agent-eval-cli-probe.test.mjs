// OB23: the isolated CLI probe harness (scripts/agent-eval/cli-probe.mjs).
// The fixtures are trimmed requests Claude Code 2.1.301 sent to the probe's
// stub (plus one synthetic middle-truncation shape), so the classifier is
// checked against what the CLI really sends. No network, no CLI spawn here.
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, test } from 'node:test';
import {
  CAP,
  DUMMY_KEY,
  MARKERS,
  childEnv,
  classifyToolResult,
  createRecorder,
  findToolResult,
  isMainThread,
  parseArgs,
  probeCommand,
  redactForRecord,
  sseFor,
} from './agent-eval/cli-probe.mjs';

const fixture = (name) => JSON.parse(readFileSync(new URL(`./agent-eval/fixtures/probe/${name}.json`, import.meta.url), 'utf8'));
const dir = mkdtempSync(join(tmpdir(), 'cli-probe-test-'));
after(() => rmSync(dir, { recursive: true, force: true }));

/** Slack for the wrapper text around a capped result. */
const SLACK = 512;
const preview = (text) => text.match(/Preview \(first 2KB\):\n([\s\S]*?)\n<\/persisted-output>/)?.[1] ?? null;

test('a capped Bash result (2.1.301): within the cap, a 2 KB head preview, tail dropped, overflow file named', () => {
  const f = fixture('capped-drops-tail');
  const c = classifyToolResult(f.body);
  assert.equal(c.found, true);
  assert.ok(c.toolResultChars <= f.cap + SLACK, `${c.toolResultChars} chars`);
  assert.equal(c.keepsHead, true);
  assert.equal(c.keepsTail, false, 'the FAILED line is past the preview');
  assert.equal(c.mentionsFile, true);
  assert.match(c.overflowPath, /^\/tmp\/cli-probe-fixture\/config\/projects\/[^/]+\/[0-9a-f-]{36}\/tool-results\/\w+\.txt$/);
  const p = preview(findToolResult(f.body, 'Bash').text);
  assert.ok(p !== null, 'a persisted-output preview');
  assert.equal(c.previewBytes, Buffer.byteLength(p));
  assert.ok(c.previewBytes <= 2048 + 80, `${c.previewBytes} preview bytes`);
  assert.ok(p.startsWith(MARKERS.head));
});

test('a middle-truncating cap keeps head and tail and names no file', () => {
  const f = fixture('capped-keeps-tail');
  const c = classifyToolResult(f.body);
  assert.ok(c.toolResultChars <= f.cap + SLACK);
  assert.equal(c.keepsHead, true);
  assert.equal(c.keepsTail, true);
  assert.equal(c.mentionsFile, false);
  assert.equal(c.overflowPath, null);
});

test('the uncapped baseline keeps head and tail and names no file', () => {
  const f = fixture('uncapped-baseline');
  assert.ok(f.originalToolResultChars > CAP, 'the recorded result was over the cap the other variants ask for');
  const c = classifyToolResult(f.body);
  assert.deepEqual([c.keepsHead, c.keepsTail, c.mentionsFile, c.overflowPath, c.previewBytes], [true, true, false, null, null]);
});

test('the Read of the overflow file is found by tool name; the Bash result stays the latest Bash one', () => {
  const f = fixture('capped-read');
  const read = findToolResult(f.body, 'Read');
  assert.equal(read.isError, false);
  assert.ok(read.text.includes(MARKERS.head) && read.text.includes(MARKERS.tail));
  const readUse = f.body.messages.flatMap((m) => (Array.isArray(m.content) ? m.content : [])).find((b) => b.type === 'tool_use' && b.name === 'Read');
  const c = classifyToolResult(f.body);
  assert.equal(readUse.input.file_path, c.overflowPath);
  assert.equal(c.keepsTail, false);
});

test('no Bash tool_result: not found; a result for another tool is not taken for Bash', () => {
  assert.equal(classifyToolResult(null).found, false);
  assert.equal(findToolResult({ messages: [] }, 'Bash'), null);
  const body = {
    messages: [
      { role: 'assistant', content: [{ type: 'tool_use', id: 't1', name: 'Read', input: {} }] },
      { role: 'user', content: [{ type: 'tool_result', tool_use_id: 't1', content: [{ type: 'text', text: MARKERS.tail }] }] },
      { role: 'system', content: 'not a turn' },
    ],
  };
  assert.equal(classifyToolResult(body).found, false);
  assert.equal(findToolResult(body, 'Read').text, MARKERS.tail);
});

test('recording a request keeps its body and never its headers or token', () => {
  const req = fixture('request-with-auth').body;
  const r = redactForRecord(req);
  assert.deepEqual(Object.keys(r).sort(), ['body', 'method', 'path']);
  assert.deepEqual(r.body, req.body);

  const file = join(dir, 'record.jsonl');
  const recorder = createRecorder(file);
  // As the stub calls it: the incoming request (headers and all) and the raw body.
  const incoming = { ...req, rawHeaders: Object.entries(req.headers).flat() };
  recorder.record(incoming, JSON.stringify(req.body));
  recorder.record(incoming, Buffer.from('not json'));
  const written = readFileSync(file, 'utf8');
  for (const s of ['FAKE-TOKEN-FIXTURE', 'authorization', 'x-api-key', 'Bearer', 'headers']) assert.ok(!written.includes(s), `${s} was written`);
  assert.ok(!JSON.stringify(recorder.records).includes('FAKE-TOKEN-FIXTURE'));
  assert.deepEqual(recorder.records[1].body, { unparsedChars: 8 });
  assert.equal(written.trim().split('\n').length, 2);
});

test('the child env is an allowlist plus the isolation variables', () => {
  const parent = {
    PATH: '/bin',
    HOME: '/h',
    TMPDIR: '/t',
    USER: 'u',
    SHELL: '/bin/zsh',
    LANG: 'C',
    ANTHROPIC_API_KEY: 'real',
    ANTHROPIC_BASE_URL: 'https://api.anthropic.com',
    CLAUDE_CODE_OAUTH_TOKEN: 'real',
    CLAUDE_CONFIG_DIR: '/h/.claude',
    GITHUB_TOKEN: 'real',
    AWS_SECRET_ACCESS_KEY: 'real',
    HTTPS_PROXY: 'http://elsewhere',
  };
  const env = childEnv({ configDir: '/c', stubPort: 1111, proxyPort: 2222, extra: { BASH_MAX_OUTPUT_LENGTH: '8000' }, parent });
  assert.ok(!Object.values(env).includes('real'));
  assert.equal(env.GITHUB_TOKEN, undefined);
  assert.equal(env.ANTHROPIC_API_KEY, DUMMY_KEY);
  assert.equal(env.ANTHROPIC_BASE_URL, 'http://127.0.0.1:1111');
  assert.equal(env.CLAUDE_CONFIG_DIR, '/c');
  for (const k of ['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy']) assert.equal(env[k], 'http://127.0.0.1:2222');
  assert.equal(env.NO_PROXY, '127.0.0.1,localhost');
  assert.equal(env.BASH_MAX_OUTPUT_LENGTH, '8000');
  assert.equal(env.PATH, '/bin');
});

test('the stub stream carries the scripted tool_use input intact', () => {
  const input = { command: probeCommand(), description: 'x' };
  const msg = { id: 'm', type: 'message', role: 'assistant', model: 'x', stop_reason: 'tool_use', usage: { input_tokens: 1, output_tokens: 1 }, content: [{ type: 'tool_use', id: 't', name: 'Bash', input }] };
  const events = sseFor(msg)
    .trim()
    .split('\n\n')
    .map((e) => JSON.parse(e.split('\n')[1].slice('data: '.length)));
  assert.deepEqual(events.map((e) => e.type), ['message_start', 'content_block_start', 'content_block_delta', 'content_block_stop', 'message_delta', 'message_stop']);
  assert.deepEqual(JSON.parse(events[2].delta.partial_json), input);
  assert.equal(events[4].delta.stop_reason, 'tool_use');
  assert.ok(isMainThread({ tools: [{ name: 'Bash' }] }));
  assert.ok(!isMainThread({ messages: [] }));
});

test('the command prints the head marker first and the tail marker last', () => {
  const c = probeCommand();
  assert.ok(c.indexOf(MARKERS.head) < c.indexOf(MARKERS.tail));
  assert.match(c, /^node -e "/);
});

test('bad arguments are refused', () => {
  assert.throws(() => parseArgs([]), /name a probe/);
  assert.throws(() => parseArgs(['bash-output', '--nope']), /unknown argument/);
  assert.throws(() => parseArgs(['bash-output', '--cap', 'x']), /positive integer/);
  assert.equal(parseArgs(['bash-output', '--cap', '20000']).cap, 20000);
});
