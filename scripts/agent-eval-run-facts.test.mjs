// LF3: run facts beside the gate digest. One lane transcript fixture gives
// the counters (repeated calls, repeated errors, a ping-pong pair, reworks per
// file, wall time against active time, token concentration), and the patches
// the tool suggests are only ever one of four kinds a person applies.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { CONCENTRATION_MIN, ERROR_REPEAT_MIN, IDLE_MS, PATCH_KINDS, REPEAT_MIN, computeFacts, makePatch, readRun, render, suggestPatches } from './agent-eval/run-facts.mjs';

const script = fileURLToPath(new URL('./agent-eval/run-facts.mjs', import.meta.url));
const fixture = fileURLToPath(new URL('./agent-eval/fixtures/run-facts/lane-d.jsonl', import.meta.url));

/** A transcript builder: each `call` is a tool use then its result, `gap` seconds apart. */
function builder(start = '2026-11-17T01:00:00Z') {
  let t = Date.parse(start);
  let n = 0;
  const entries = [];
  const stamp = (s) => new Date((t += s * 1000)).toISOString();
  return {
    entries,
    call(name, input, { result = 'ok', error = false, gap = 5, sidechain = false } = {}) {
      const id = `tu${++n}`;
      entries.push({ type: 'assistant', isSidechain: sidechain, timestamp: stamp(gap), message: { id: `m${n}`, content: [{ type: 'tool_use', id, name, input }] } });
      entries.push({ type: 'user', isSidechain: sidechain, timestamp: stamp(1), message: { content: [{ type: 'tool_result', tool_use_id: id, ...(error ? { is_error: true } : {}), content: result }] } });
    },
  };
}

test('the fixture lane shows every counter', async () => {
  const facts = computeFacts(await readRun(fixture));
  assert.equal(facts.calls, 26);
  assert.equal(facts.errors, 12);
  // 14 identical `cargo test memory_guard` runs, 12 of them failing.
  assert.deepEqual(facts.repeats.map((r) => [r.call, r.count, r.errors]), [['Bash: cargo test memory_guard', 14, 12]]);
  // The same compiler error 12 times, with the line numbers folded.
  assert.deepEqual(facts.repeatedErrors, [{ text: 'error[e#]: mismatched types', count: 12 }]);
  // One file edited 4 times; each of the 3 later edits followed a failing check.
  assert.deepEqual(facts.nodes, [{ node: 'src-tauri/src/memory_guard.rs', attempts: 4, reworks: 3 }]);
  // Two reads alternating for 6 calls.
  assert.equal(facts.pingPong.length, 1);
  assert.equal(facts.pingPong[0].calls, 6);
  assert.deepEqual([...facts.pingPong[0].pair].sort(), ['Read src-tauri/src/memory_maintenance.rs', 'Read src-tauri/src/memory_pressure.rs']);
  // 3431 s from the first line to the last, 2400 s of it one idle gap.
  assert.equal(facts.time.wallMs, 3_431_000);
  assert.equal(facts.time.idleMs, 2_400_000);
  assert.equal(facts.time.activeMs, 1_031_000);
  assert.equal(facts.time.idleGaps, 1);
  // The 70-line Read is most of what the tools returned.
  assert.equal(facts.concentration.tool, 'Read');
  assert.ok(facts.concentration.share > 0.5 && facts.concentration.share < 1, `share ${facts.concentration.share}`);
});

test('the text report is under 30 lines and names the counters; --json has the same facts', async () => {
  const facts = computeFacts(await readRun(fixture));
  const patches = suggestPatches(facts);
  const lines = render(facts, patches);
  assert.ok(lines.length < 30, `${lines.length} lines`);
  const text = lines.join('\n');
  assert.match(text, /same call 14 times \(12 failed\): Bash: cargo test memory_guard/);
  assert.match(text, /same error 12 times: error\[e#\]: mismatched types/);
  assert.match(text, /memory_guard\.rs: 4 attempts, 3 reworks/);
  assert.match(text, /ping-pong: .* 6 calls/);
  assert.match(text, /wall 57\.2 min, active 17\.2 min, idle 40\.0 min/);
  assert.match(text, /% of tool-result tokens in one Read result/);
  assert.match(text, /a person applies them/);
  const cli = spawnSync(process.execPath, [script, fixture, '--json'], { encoding: 'utf8' });
  assert.equal(cli.status, 0, cli.stderr);
  const json = JSON.parse(cli.stdout);
  assert.deepEqual(json.facts, facts);
  assert.deepEqual(json.patches, patches);
  const plain = spawnSync(process.execPath, [script, fixture], { encoding: 'utf8' });
  assert.equal(plain.status, 0);
  assert.equal(plain.stdout.trimEnd(), text);
  assert.equal(spawnSync(process.execPath, [script], { encoding: 'utf8' }).status, 2);
});

test('the thresholds are 12 identical calls, 3 repeated errors, and idle gaps over 5 minutes', () => {
  assert.deepEqual([REPEAT_MIN, ERROR_REPEAT_MIN, IDLE_MS], [12, 3, 300_000]);
  const eleven = builder();
  for (let i = 0; i < 11; i++) eleven.call('Bash', { command: 'git status' });
  assert.deepEqual(computeFacts(eleven.entries).repeats, []);
  const twelve = builder();
  for (let i = 0; i < 12; i++) twelve.call('Bash', { command: 'git status', description: `try ${i}` });
  assert.deepEqual(computeFacts(twelve.entries).repeats.map((r) => [r.call, r.count, r.errors]), [['Bash: git status', 12, 0]]);
  // A gap of exactly 5 minutes is active; one second more is idle.
  const gaps = builder();
  gaps.call('Read', { file_path: 'a.rs' }, { gap: 1 });
  gaps.call('Read', { file_path: 'b.rs' }, { gap: 300 });
  gaps.call('Read', { file_path: 'c.rs' }, { gap: 301 });
  const t = computeFacts(gaps.entries).time;
  assert.equal(t.idleGaps, 1);
  assert.equal(t.idleMs, 301_000);
  assert.equal(computeFacts(gaps.entries, { idleMs: 600_000 }).time.idleGaps, 0);
});

test('a ping-pong needs two full cycles of two different calls', () => {
  const one = builder();
  for (const f of ['a', 'b', 'a']) one.call('Read', { file_path: `${f}.rs` });
  assert.deepEqual(computeFacts(one.entries).pingPong, []);
  const two = builder();
  for (const f of ['a', 'b', 'a', 'b']) two.call('Read', { file_path: `${f}.rs` });
  assert.equal(computeFacts(two.entries).pingPong[0].calls, 4);
  const same = builder();
  for (let i = 0; i < 6; i++) same.call('Read', { file_path: 'a.rs' });
  assert.deepEqual(computeFacts(same.entries).pingPong, []);
});

test('sidechain lines and a lane with no tools are left out; an empty run still reports', () => {
  const b = builder();
  b.call('Bash', { command: 'ls' });
  for (let i = 0; i < 14; i++) b.call('Bash', { command: 'make', }, { sidechain: true, error: true });
  const facts = computeFacts(b.entries);
  assert.equal(facts.calls, 1);
  assert.deepEqual(facts.repeats, []);
  const empty = computeFacts([]);
  assert.equal(empty.calls, 0);
  assert.equal(empty.time.wallMs, 0);
  assert.equal(empty.concentration, null);
  assert.ok(render(empty, suggestPatches(empty)).length < 30);
});

test('a secret in a repeated command is masked in the report', () => {
  const b = builder();
  for (let i = 0; i < 12; i++) b.call('Bash', { command: 'curl -H "Authorization: Bearer abcdefghijklmnopqrstuvwxyz123456" https://example.test' }, { error: true, result: 'Exit code 7\ncurl failed with token=sk-abcdefghijklmnopqrstuvwxyz' });
  const text = render(computeFacts(b.entries), suggestPatches(computeFacts(b.entries))).join('\n');
  assert.doesNotMatch(text, /abcdefghijklmnopqrstuvwxyz123456|sk-abcdefghij/);
  assert.match(text, /masked/);
});

test('each patch kind comes from the facts that justify it', () => {
  const fixtureKinds = [];
  return readRun(fixture).then((entries) => {
    const patches = suggestPatches(computeFacts(entries));
    fixtureKinds.push(...patches.map((p) => p.kind));
    assert.deepEqual(fixtureKinds, ['change-retry-limit', 'add-gate', 'append-role-prompt-line', 'append-role-prompt-line']);
    assert.deepEqual(patches[0], { kind: 'change-retry-limit', target: 'Bash: cargo test memory_guard', limit: 3, because: 'the same call ran 14 times and failed 12' });
    assert.equal(patches[1].after, 'src-tauri/src/memory_guard.rs');
    // A passing check repeated 12 times is a gate to remove, not to retry.
    const passing = builder();
    for (let i = 0; i < 12; i++) passing.call('Bash', { command: 'npm run lint' });
    const kinds = suggestPatches(computeFacts(passing.entries)).map((p) => p.kind);
    assert.deepEqual(kinds, ['remove-gate']);
    // The same error text without 12 identical calls still names a retry limit.
    const errors = builder();
    for (let i = 0; i < 4; i++) errors.call('Bash', { command: `cargo test t${i}` }, { error: true, result: 'Exit code 101\nthread panicked: budget 7 over' });
    assert.deepEqual(suggestPatches(computeFacts(errors.entries)).map((p) => p.kind), ['change-retry-limit']);
  });
});

test('an append-role-prompt-line patch is dropped when the role prompt already has the line', async () => {
  const facts = computeFacts(await readRun(fixture));
  const all = suggestPatches(facts);
  const lines = all.filter((p) => p.kind === 'append-role-prompt-line').map((p) => p.line);
  assert.equal(lines.length, 2);
  // Same words, different case, spacing and trailing full stop.
  const prompt = `# Role\n\n- ${lines[0].toUpperCase()}  \n`;
  const kept = suggestPatches(facts, { rolePrompt: prompt });
  assert.deepEqual(kept.filter((p) => p.kind === 'append-role-prompt-line').map((p) => p.line), [lines[1]]);
  assert.equal(kept.length, all.length - 1);
});

test('nothing outside the four kinds is ever emitted, and a suggestion is never applied', async () => {
  assert.deepEqual([...PATCH_KINDS], ['change-retry-limit', 'add-gate', 'remove-gate', 'append-role-prompt-line']);
  assert.ok(Object.isFrozen(PATCH_KINDS));
  for (const bad of ['edit-file', 'delete-gate', 'run-command', 'change-model', 'append-role-prompt', '__proto__', '']) {
    assert.throws(() => makePatch(bad, { because: 'x' }), /not a patch kind/, bad);
  }
  assert.throws(() => makePatch('add-gate', { after: 'a', gate: 'g', because: 'b', command: 'rm -rf /' }), /unknown field/);
  assert.throws(() => makePatch('change-retry-limit', { target: 'a', limit: 0, because: 'b' }), /limit/);
  assert.throws(() => makePatch('append-role-prompt-line', { line: '', because: 'b' }), /line/);
  const fields = { 'change-retry-limit': ['kind', 'target', 'limit', 'because'], 'add-gate': ['kind', 'after', 'gate', 'because'], 'remove-gate': ['kind', 'gate', 'because'], 'append-role-prompt-line': ['kind', 'line', 'because'] };
  // Runs that trip every rule at once, and runs that trip none.
  const runs = [await readRun(fixture)];
  const mixed = builder();
  for (let i = 0; i < 13; i++) mixed.call('Read', { file_path: 'same.rs' });
  for (let i = 0; i < 13; i++) mixed.call('Bash', { command: 'npm test' }, { error: i % 2 === 0, result: i % 2 === 0 ? 'Exit code 1\nfail' : 'ok' });
  for (let i = 0; i < 8; i++) mixed.call('Edit', { file_path: 'x.rs', old_string: String(i) }, { error: i % 3 === 0, result: 'Error: no match' });
  runs.push(mixed.entries, []);
  for (const entries of runs) {
    for (const rolePrompt of [undefined, '']) {
      const patches = suggestPatches(computeFacts(entries), { rolePrompt });
      for (const p of patches) {
        assert.ok(PATCH_KINDS.includes(p.kind), p.kind);
        assert.deepEqual(Object.keys(p), fields[p.kind]);
      }
    }
  }
  // The tool only reads: it never writes, so the person applies a patch by hand.
  const source = readFileSync(script, 'utf8');
  assert.doesNotMatch(source, /writeFile|appendFile|unlink|rmSync|child_process|\bfetch\(/);
  const dir = mkdtempSync(join(tmpdir(), 'run-facts-'));
  try {
    const prompt = join(dir, 'role.md');
    writeFileSync(prompt, '# Role\nBe brief.\n');
    const cli = spawnSync(process.execPath, [script, fixture, '--role-prompt', prompt], { encoding: 'utf8' });
    assert.equal(cli.status, 0, cli.stderr);
    assert.equal(readFileSync(prompt, 'utf8'), '# Role\nBe brief.\n');
    assert.match(cli.stdout, /append-role-prompt-line/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
  assert.equal(CONCENTRATION_MIN, 0.4);
});
