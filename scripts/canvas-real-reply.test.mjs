// The billed probe's free parts: arguments, the generated prompts, the argv
// it builds, reply parsing and judging, and a full run against a fake CLI.
// Nothing here starts a real claude or codex.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  DRY_MODEL, MAX_BUDGET_USD, REQUEST, binaryFor, claudeArgs, codexArgs, judgeDryRun, judgeReply, parseArgs,
  readClaude, readCodex, readPrompts, runAgent, turnMessage,
} from './canvas-real-reply.mjs';

const POLICY_START = '\n\nHollow is a local workspace';

const goodCanvas = '```hollow-canvas\n<hc-canvas id="q4-launch" title="Q4 launch">\n  <hc-stats><hc-stat label="Checks" value="41/45"></hc-stat></hc-stats>\n</hc-canvas>\n```';
const prose = 'The launch is close: 41 of 45 checks pass and two bugs block Friday. Ship with the export behind a flag.';

test('the default is a dry run; --billed must be explicit', () => {
  assert.equal(parseArgs([]).opts.billed, false);
  assert.deepEqual(parseArgs([]).opts.agents, ['claude', 'codex']);
  assert.equal(parseArgs(['--billed']).opts.billed, true);
  assert.deepEqual(parseArgs(['--agent', 'codex']).opts.agents, ['codex']);
  assert.match(parseArgs(['--agent', 'grok']).error, /claude, codex or both/);
  assert.match(parseArgs(['--bill']).error, /unknown argument/);
  assert.match(parseArgs(['--claude-model']).error, /needs a value/);
  assert.equal(parseArgs([]).opts.request, REQUEST);
});

test('the prompts are the generated copy of what Hollow sends', () => {
  const p = readPrompts();
  assert.ok(p.system.startsWith('# Visual answers in Hollow'));
  assert.equal(p.canvases, 'helps');
  // Claude: the chat prompt then the workspace policy; Codex: the policy,
  // the same chat prompt, then the request.
  const chat = p.system.slice(0, p.system.lastIndexOf(POLICY_START));
  assert.ok(chat.length > 2_000);
  // Since agent effectiveness F9 (wave 4 · V1) each agent's capability index
  // ends with its own line of the per-provider contract; the rest is the same.
  const own = chat.indexOf('\n- This agent (Claude): ');
  assert.ok(own > 0, 'Claude gets its contract line');
  const before = chat.slice(0, own);
  const after = chat.slice(chat.indexOf('\n', own + 1));
  assert.ok(p.preamble.includes(`\n\n${before}\n- This agent (Codex): `), 'Codex reads the same chat prompt, with its own contract line');
  assert.ok(p.preamble.includes(`${after}\n\n`), 'and the same text after it');
  assert.ok(p.preamble.endsWith('\n\n'));
  // Since agent effectiveness G1 the formats are the visual guide, which a
  // visual turn's message carries ahead of the request; the prompt says
  // where it is and has the ask "when" line. The probe runs with no Hollow
  // tools, so it gets the no-question-cards wording.
  assert.ok(chat.includes('$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md'));
  assert.ok(!chat.includes('\n## Ask-canvases\n'));
  assert.match(chat, /This chat has no question cards, so ask with an ask-canvas/);
  assert.ok(p.guide.includes('\n## Ask-canvases\n') && p.guide.includes('<hc-flow') && !p.guide.includes('## Boards'));
  assert.ok(p.guide.trimEnd().endsWith('</hollow-visual-guide>'));
  assert.equal(turnMessage(p, 'R'), `${p.guide}R`);
});

test('the billed argv carries the exact prompt, read-only, with no Hollow tools', () => {
  const p = readPrompts();
  const claude = claudeArgs({ system: p.system, request: 'R', model: 'opus' });
  assert.equal(claude[claude.indexOf('--append-system-prompt') + 1], p.system);
  assert.equal(claude[claude.indexOf('--permission-mode') + 1], 'plan');
  assert.equal(claude[claude.indexOf('--max-budget-usd') + 1], MAX_BUDGET_USD);
  assert.equal(claude[claude.indexOf('--mcp-config') + 1], '{"mcpServers":{}}');
  assert.ok(claude.includes('--no-session-persistence') && claude.includes('--strict-mcp-config'));
  assert.ok(!claudeArgs({ system: 's', request: 'R', model: null }).includes('--model'));
  const codex = codexArgs({ preamble: p.preamble, request: 'R', model: null });
  assert.equal(codex.at(-1), `${p.preamble}R`);
  assert.equal(codex[codex.indexOf('-s') + 1], 'read-only');
  assert.ok(codex.includes('--ephemeral') && !codex.includes('-m'));
  assert.ok(codex.indexOf('exec') < codex.indexOf('--json'));
});

test('replies are judged with the chat parser', async () => {
  const good = await judgeReply(`${prose}\n\n${goodCanvas}\n`);
  assert.equal(good.ok, true, good.problems.join('; '));
  assert.equal(good.canvases[0].id, 'q4-launch');
  const badId = await judgeReply(`${prose}\n\n${goodCanvas.replace('q4-launch', 'Q4 Launch')}`);
  assert.equal(badId.ok, false);
  assert.match(badId.problems[0], /invalid: The canvas id/);
  assert.deepEqual((await judgeReply(prose)).problems, ['no hollow-canvas fence']);
  assert.deepEqual((await judgeReply(goodCanvas)).problems, ['the answer is not stated in prose']);
  const open = await judgeReply(`${prose}\n\n${goodCanvas.replace(/\n```$/, '')}`);
  assert.match(open.problems.join(), /no closing fence/);
  assert.equal((await judgeReply(`${prose}\n${goodCanvas}\n\`\`\`html\n<p>x</p>\n\`\`\``)).html, 1);
});

test('reads each CLI\'s reply and reported cost', () => {
  const claude = readClaude(JSON.stringify({ type: 'result', is_error: false, result: 'hello', total_cost_usd: 1.23456 }));
  assert.deepEqual(claude, { reply: 'hello', error: null, cost: '$1.2346 (reported by Claude)' });
  assert.equal(readClaude('not json').error, 'no JSON result');
  const codex = readCodex([
    '{"type":"thread.started"}',
    '{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"hi there"}}',
    '{"type":"turn.completed","usage":{"input_tokens":900,"cached_input_tokens":100,"output_tokens":50}}',
  ].join('\n'));
  assert.equal(codex.reply, 'hi there');
  assert.match(codex.cost, /900 input \(100 cached\) \+ 50 output tokens/);
  assert.equal(readCodex('{"type":"turn.failed","error":{"message":"boom"}}').error, 'boom');
});

test('a dry run passes only when the CLI took every flag and refused the model', () => {
  assert.equal(judgeDryRun({ status: 1, stdout: '', stderr: `There's an issue with the selected model (${DRY_MODEL}). It may not exist.` }).ok, true);
  assert.equal(judgeDryRun({ status: 1, stdout: '', stderr: "error: unknown option '--max-budget-usd'" }).ok, false);
  assert.equal(judgeDryRun({ status: 2, stdout: '', stderr: "error: unexpected argument '--ephemeral' found" }).ok, false);
  assert.equal(judgeDryRun({ status: 0, stdout: '{"result":"hi","model":"x"}', stderr: '' }).ok, false);
  assert.equal(judgeDryRun({ status: 1, stdout: '', stderr: 'network down' }).ok, false);
});

test('runs end to end against a fake CLI: the dry run sends the made-up model', async t => {
  const dir = mkdtempSync(join(tmpdir(), 'probe-test-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const log = join(dir, 'argv.txt');
  const fake = join(dir, 'fake-claude.sh');
  // Records its argv; refuses the made-up model, answers otherwise.
  writeFileSync(fake, `#!/bin/sh
for a in "$@"; do printf '%s\\n' "$a" >> "${log}"; done
case "$*" in *${DRY_MODEL}*) echo "There's an issue with the selected model (${DRY_MODEL})." >&2; exit 1;; esac
cat <<'JSON'
{"type":"result","is_error":false,"total_cost_usd":0.5,"result":${JSON.stringify(`${prose}\n\n${goodCanvas}`)}}
JSON
`);
  chmodSync(fake, 0o755);
  const env = { HOLLOW_PROBE_CLAUDE_BIN: fake };
  assert.equal(binaryFor('claude', env), fake);
  assert.equal(binaryFor('codex', {}), 'codex');
  const prompts = readPrompts();
  const dry = await runAgent('claude', parseArgs([]).opts, prompts, { cwd: dir, out: dir, env });
  assert.deepEqual([dry.dry, dry.ok], [true, true]);
  assert.ok(readFileSync(log, 'utf8').split('\n').includes(DRY_MODEL));
  // The request goes with the visual guide ahead of it, as Hollow sends a visual turn.
  assert.ok(readFileSync(log, 'utf8').split('\n').includes('<hollow-visual-guide>'));
  const billed = await runAgent('claude', parseArgs(['--billed', '--claude-model', 'opus']).opts, prompts, { cwd: dir, out: dir, env });
  assert.equal(billed.ok, true, (billed.problems ?? []).join('; '));
  assert.equal(billed.cost, '$0.5000 (reported by Claude)');
  assert.equal(readFileSync(join(dir, 'claude.reply.md'), 'utf8'), `${prose}\n\n${goodCanvas}`);
});
