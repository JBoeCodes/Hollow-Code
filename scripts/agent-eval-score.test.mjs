// Agent effectiveness E1: the pilot suite and its scorer
// (scripts/agent-eval/pilot-tasks.json, scripts/agent-eval/score.mjs;
// docs/history/AGENT_EFFECTIVENESS.md "Evaluation harness"). Everything here reads
// recorded folders or temp copies: nothing calls a model, Jev or the bed.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  SLICES, checkRule, compareRounds, loadSuite, newcombe, percentile, renderComparison, renderReport, scoreArtifact,
  scoreRound, scoreRunDir, taskDigest, validateSuite, wilson,
} from './agent-eval/score.mjs';
import { files as vaultFiles } from './test-bed-vault.mjs';

const SUITE_FILE = fileURLToPath(new URL('./agent-eval/pilot-tasks.json', import.meta.url));
const ROUNDS = fileURLToPath(new URL('./agent-eval/fixtures/rounds/', import.meta.url));
const SCORE = fileURLToPath(new URL('./agent-eval/score.mjs', import.meta.url));
const suite = loadSuite(SUITE_FILE);
const byId = Object.fromEntries(suite.tasks.map((t) => [t.id, t]));
const tempRoot = (t) => {
  const root = mkdtempSync(join(realpathSync(tmpdir()), 'hollow-eval-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
};
const near = (a, b, eps = 1e-3) => Math.abs(a - b) < eps;

// ── The suite ────────────────────────────────────────────────────────────

test('the suite: about ten labelled tasks covering every slice the plan names, each asking for a named file under outputs/', () => {
  const tasks = suite.tasks;
  assert.ok(tasks.length >= 9 && tasks.length <= 12, `about ten tasks, not ${tasks.length}`);
  assert.deepEqual([...new Set(tasks.map((t) => t.slice))].sort(), [...SLICES].sort(), 'every slice has a task');
  const facets = tasks.map((t) => t.facet).join('\n');
  for (const want of [/fourth attached file/, /older decision behind newer notes/, /paraphrase/, /settled constraint/, /superseded/, /historical/, /named Hollow tool/, /skill/, /vault-wide search/]) {
    assert.match(facets, want);
  }
  for (const t of tasks) {
    assert.equal(t.workspace, 'eval-outcome', t.id);
    assert.match(t.score.artifact, /^outputs\/[a-z0-9-]+\.md$/, `${t.id}: the artifact is a file under outputs/`);
    assert.ok(t.prompt.includes(t.score.artifact), `${t.id}: the message names the file to write`);
    assert.ok(t.score.rules.length >= 1, t.id);
    assert.ok(t.judged.evidence.length >= 1 && t.judged.fails.length >= 1, `${t.id}: judged evidence, a reference answer and a failing answer`);
  }
});

test('the judged evidence is in the bed fixture, verbatim, at the path it names', () => {
  const all = vaultFiles();
  for (const t of suite.tasks) {
    for (const e of t.judged.evidence) {
      assert.ok(all[e.path] != null, `${t.id}: ${e.path} is in the fixture`);
      assert.ok(all[e.path].includes(e.quote), `${t.id}: ${e.path} says ${JSON.stringify(e.quote)}`);
    }
    for (const r of t.score.rules.filter((r) => r.kind === 'cites')) assert.ok(all[r.path] != null, `${t.id}: cites ${r.path}, which exists`);
  }
});

test('every rule is deterministic: the reference answer passes and each judged failure fails, for the reason given', () => {
  for (const t of suite.tasks) {
    const ok = scoreArtifact(t, t.judged.answer);
    assert.equal(ok.pass, true, `${t.id}: the reference answer passes (${ok.reasons.join('; ')})`);
    for (const f of t.judged.fails) {
      const got = scoreArtifact(t, f.text);
      assert.equal(got.pass, false, `${t.id}: "${f.text}" fails (${f.because})`);
      assert.ok(got.reasons.length >= 1);
      assert.deepEqual(scoreArtifact(t, f.text), got, 'same input, same score');
    }
    const missing = scoreArtifact(t, null);
    assert.equal(missing.pass, false);
    assert.match(missing.reasons[0], new RegExp(`no ${t.score.artifact.replace('.', '\\.')}`));
  }
});

test('the skill task sends the message Skills → Use in workspace composes, pointing at the fixture skill', () => {
  // Since agent effectiveness F9 (wave 4 · V1) the message is
  // skillCatalog.ts useSkillPrompt: read the skill through Hollow's app
  // tools (skills.read_skill), else with the agent's own file tools.
  const skills = readFileSync(new URL('../src/pages/Skills.tsx', import.meta.url), 'utf8');
  assert.ok(skills.includes('${useSkillPrompt(s)}'), 'Skills.tsx composes useSkillPrompt');
  const catalog = readFileSync(new URL('../src/skillCatalog.ts', import.meta.url), 'utf8');
  const template = 'Use the skill ${JSON.stringify(skill.name)} for this task. First read its instructions: with Hollow’s app tools, run_action skills.read_skill with {"path": ${path}}; without them, open ${path} with your own file tools (if that file is outside what you may read, say so and ask me to paste it). Then follow them for this task:\\n\\n';
  assert.ok(catalog.includes(template), 'skillCatalog.ts still composes this message');
  const path = JSON.stringify('{vault}/.agents/skills/kestrel-status-report/SKILL.md');
  const t = byId['tools-selected-skill'];
  assert.ok(t.prompt.startsWith(`Use the skill "kestrel-status-report" for this task. First read its instructions: with Hollow’s app tools, run_action skills.read_skill with {"path": ${path}}; without them, open ${path} with your own file tools (if that file is outside what you may read, say so and ask me to paste it). Then follow them for this task:\n\n`));
  assert.ok(vaultFiles()['.agents/skills/kestrel-status-report/SKILL.md'], 'the skill is in the fixture, in a dot folder retrieval skips');
});

test('suite validation refuses what a scorer could not judge', () => {
  const base = JSON.parse(readFileSync(SUITE_FILE, 'utf8'));
  const one = (patch) => ({ ...base, tasks: [{ ...base.tasks[0], ...patch(base.tasks[0]) }] });
  const bad = [
    [(t) => ({ slice: 'vibes' }), /slice/],
    [(t) => ({ score: { ...t.score, artifact: '../escape.md' } }), /artifact/],
    [(t) => ({ score: { ...t.score, artifact: 'outputs/other.md' } }), /names/],
    [(t) => ({ score: { ...t.score, rules: [] } }), /rules/],
    [(t) => ({ score: { ...t.score, rules: [{ id: 'x', kind: 'guess', any: ['a'] }] } }), /kind/],
    [(t) => ({ score: { ...t.score, rules: [{ id: 'x', kind: 'contains', pattern: '(' }] } }), /pattern/],
    [(t) => ({ score: { ...t.score, rules: [{ id: 'x', kind: 'contains' }] } }), /any or a pattern/],
    [(t) => ({ judged: { ...t.judged, evidence: [] } }), /evidence/],
    [(t) => ({ id: 'Bad Id' }), /id/],
  ];
  for (const [patch, why] of bad) assert.throws(() => validateSuite(one(patch)), why, String(why));
  assert.throws(() => validateSuite({ ...base, tasks: [base.tasks[0], base.tasks[0]] }), /twice/);
});

test('rules: text is normalised (case, spacing, emphasis, thin spaces) but the rule kinds stay exact', () => {
  assert.equal(checkRule({ id: 'a', kind: 'contains', any: ['48,500 eur'] }, 'The **48,500 EUR** ceiling').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'contains', pattern: '48[,. ]?500' }, '48 500 €').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'absent', any: ['USD'] }, 'all in EUR').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'cites', path: 'notes/kestrel/x.md', form: 'vault-path' }, 'See `/Users/a/vault/notes/kestrel/x.md`').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'cites', path: 'notes/kestrel/x.md', form: 'vault-path' }, 'See x.md').pass, false);
  assert.equal(checkRule({ id: 'a', kind: 'cites', path: 'notes/kestrel/x.md', form: 'basename' }, 'See x.md').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'firstLine', pattern: '^TITLE$', flags: '' }, '\n\nTITLE\nbody').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'firstLine', pattern: '^TITLE$', flags: '' }, 'Title\nbody').pass, false);
  assert.equal(checkRule({ id: 'a', kind: 'inOrder', items: ['## A', '## B'] }, '## A\n## B').pass, true);
  assert.equal(checkRule({ id: 'a', kind: 'inOrder', items: ['## A', '## B'] }, '## B\n## A').pass, false);
  const labelled = { id: 'a', kind: 'historyLabelled', stale: ['Friday'], markers: ['used to', 'not'] };
  assert.equal(checkRule(labelled, 'Tuesdays. It used to be Friday.').pass, true);
  assert.equal(checkRule(labelled, 'Tuesdays. Notes go out Friday.').pass, false, '"Notes" is not the word "not"');
  assert.equal(checkRule(labelled, 'Tuesdays.').pass, true, 'no stale mention at all');
});

test('the task digest names what a round sent: id, workspace and the exact message, in any order', () => {
  const a = taskDigest(suite.tasks);
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(taskDigest([...suite.tasks].reverse()), a);
  assert.notEqual(taskDigest(suite.tasks.map((t, i) => (i ? t : { ...t, prompt: `${t.prompt} ` }))), a);
  // Rules and labels can be corrected without making old rounds incomparable.
  assert.equal(taskDigest(suite.tasks.map((t) => ({ ...t, facet: 'x', score: null, judged: null }))), a);
});

// ── Intervals ────────────────────────────────────────────────────────────

test('Wilson 95% intervals and the Newcombe difference behave at the edges', () => {
  assert.equal(wilson(0, 0), null, 'no runs, no interval');
  const all = wilson(3, 3);
  assert.ok(near(all.lo, 0.4385) && all.hi === 1, JSON.stringify(all));
  const none = wilson(0, 3);
  assert.ok(none.lo === 0 && near(none.hi, 0.5615));
  const half = wilson(5, 10);
  assert.ok(near(half.lo, 0.2366) && near(half.hi, 0.7634));
  const d = newcombe(1, 2, 1, 3);
  assert.ok(near(d.diff, -1 / 6));
  assert.ok(d.lo < d.diff && d.diff < d.hi && d.lo >= -1 && d.hi <= 1);
  assert.equal(newcombe(0, 0, 1, 3), null);
  assert.equal(percentile([], 0.95), null);
  assert.equal(percentile([5, 1, 3], 0.5), 3);
  assert.equal(percentile([10, 20, 30, 40], 0.95), 40, 'nearest rank: with few runs p95 is the slowest');
});

// ── A recorded round ─────────────────────────────────────────────────────

test('a round before F11 (round 0): passing, failing, missing and not-run runs, with manifests absent, never zero', () => {
  const report = scoreRound(join(ROUNDS, 'r-base'), suite.tasks, { tasksFile: SUITE_FILE });
  assert.equal(report.round, 'r-base');
  assert.deepEqual(report.totals.runs, { planned: 9, scored: 8, notRun: 1, passed: 4 });
  assert.equal(report.totals.completion, 0.5);
  const run = (key) => report.runs.find((r) => r.key === key);
  assert.equal(run('research-fourth-attachment-1').pass, true);
  assert.equal(run('research-fourth-attachment-2').pass, true, '48 500 € with the source name');
  const astray = run('research-fourth-attachment-3');
  assert.equal(astray.pass, false);
  assert.match(astray.reasons.join(' '), /no outputs\/budget\.md.*outputs\/budget\.md at the vault root/);
  assert.match(run('memory-current-1').reasons.join(' '), /old-only-as-history/);
  assert.equal(run('memory-current-2').pass, true, 'the old cadence, labelled as history');
  const timeout = run('memory-current-3');
  assert.equal(timeout.status, 'timeout');
  assert.match(timeout.reasons.join(' '), /run ended timeout/);
  assert.match(run('continuity-settled-constraint-2').reasons.join(' '), /never-usd/);
  const notRun = report.notRun.find((r) => r.key === 'continuity-settled-constraint-3');
  assert.match(notRun.reason, /would pass the \$6\.00 budget/);
  // Per slice, over the repeats.
  const slice = (name) => report.slices.find((s) => s.slice === name);
  assert.deepEqual([slice('research').passed, slice('research').runs], [2, 3]);
  assert.deepEqual([slice('memory').passed, slice('memory').runs], [1, 3]);
  assert.deepEqual([slice('continuity').passed, slice('continuity').runs], [1, 2]);
  assert.ok(near(slice('research').ci.lo, 0.2077) && near(slice('research').ci.hi, 0.9385));
  // Manifests: absent is a count and a reason, never a zero-byte layer.
  assert.deepEqual(report.totals.manifests, { present: 0, absent: 8, unreadable: 0, mismatch: 0 });
  for (const r of report.runs) {
    assert.equal(r.manifest.status, 'absent');
    assert.match(r.manifest.reason, /before F11/);
    assert.equal(r.manifest.layersBytes, undefined);
  }
  for (const t of report.tasks) assert.equal(t.manifest.meanLayersBytes, null);
  // Cost: one run's is unknown, so the total is unknown; the known part is kept apart.
  assert.equal(report.totals.cost.totalUsd, null);
  assert.equal(report.totals.cost.unknownRuns, 1);
  assert.ok(near(report.totals.cost.knownUsd, 5.6));
  assert.equal(report.totals.tokens.input, null, 'a run without token counts makes the sum unknown');
  // Durations: the old pilot's wall clock, labelled as such.
  assert.deepEqual(report.totals.duration.sources, ['pilot-wall']);
  assert.equal(report.totals.duration.p95Ms, 600_000);
  assert.equal(report.digest.recorded, null, 'the old pilot recorded no task digest');
  assert.match(report.warnings.join(' '), /records no task digest/);
});

test('a later round: manifests present are summarised; refused, foreign and newer-schema ones are reported as such', () => {
  const report = scoreRound(join(ROUNDS, 'r-next'), suite.tasks, { tasksFile: SUITE_FILE });
  assert.deepEqual(report.totals.runs, { planned: 9, scored: 9, notRun: 0, passed: 7 });
  assert.equal(report.digest.matches, true, 'the round sent exactly this suite\'s messages');
  assert.deepEqual(report.totals.manifests, { present: 6, absent: 1, unreadable: 1, mismatch: 1 });
  const run = (key) => report.runs.find((r) => r.key === key);
  const m = run('research-fourth-attachment-3').manifest;
  assert.equal(m.status, 'present');
  assert.equal(m.layersBytes, 11_200);
  assert.deepEqual(m.retrieval, { engine: 'jev', considered: 5, kept: 2 });
  assert.deepEqual(m.connectors, { engine: 'local', considered: 2, attached: 1, named: 1 });
  assert.deepEqual(m.jev.map((j) => [j.use, j.outcome]), [['passages', 'ok'], ['connectors', 'off']]);
  assert.equal(m.jev[0].costUsd, null, 'Jev cost unknown stays null');
  assert.match(run('memory-current-2').manifest.reason, /refused run_context_manifest/);
  assert.match(run('memory-current-3').manifest.reason, /run-someone-else/);
  assert.match(run('continuity-settled-constraint-2').manifest.reason, /schema 2/);
  const research = report.tasks.find((t) => t.task === 'research-fourth-attachment');
  assert.equal(research.manifest.meanLayersBytes, 11_200);
  assert.deepEqual(research.manifest.jevOutcomes, { passages: { ok: 2, timeout: 1 }, connectors: { off: 3 } });
  assert.equal(research.manifest.fallbacks, 1);
  assert.deepEqual(report.totals.duration.sources, ['queue']);
  assert.equal(run('research-fourth-attachment-1').durationMs, 41_000);
  assert.deepEqual(run('research-fourth-attachment-1').tokens, { input: 2000, cached: 18_000, output: 400, cacheShare: 0.9 });
});

test('the report renders as JSON and a markdown table with per-task rows, slices with intervals, failures and totals', () => {
  const report = scoreRound(join(ROUNDS, 'r-base'), suite.tasks, { tasksFile: SUITE_FILE });
  const md = renderReport(report);
  assert.match(md, /^# Pilot round r-base/m);
  assert.match(md, /\| research \| 1 \| 3 \| 2 \| 66\.7% \| 20\.8–93\.9% \|/);
  assert.match(md, /\| research-fourth-attachment \| research \|/);
  assert.match(md, /## Failures/);
  assert.match(md, /memory-current-3.*run ended timeout/);
  assert.match(md, /## Not run/);
  assert.match(md, /absent 8/);
  assert.match(md, /unknown \(1 run without a cost\)/);
  assert.doesNotMatch(md, /\$0\.00 total/);
  assert.deepEqual(JSON.parse(JSON.stringify(report)), report, 'plain JSON');
});

test('a round that ran other messages than the suite is flagged, and a folder outside the plan is noted', (t) => {
  const root = tempRoot(t);
  cpSync(join(ROUNDS, 'r-next'), join(root, 'r-x'), { recursive: true });
  const plan = JSON.parse(readFileSync(join(root, 'r-x', 'plan.json'), 'utf8'));
  writeFileSync(join(root, 'r-x', 'plan.json'), JSON.stringify({ ...plan, taskDigest: 'f'.repeat(64) }));
  cpSync(join(root, 'r-x', 'memory-current-1'), join(root, 'r-x', 'memory-current-4'), { recursive: true });
  const report = scoreRound(join(root, 'r-x'), suite.tasks);
  assert.equal(report.digest.matches, false);
  assert.match(report.warnings.join(' '), /different messages/);
  assert.match(report.warnings.join(' '), /memory-current-4.*not in the round's plan/);
});

// ── Comparing rounds ─────────────────────────────────────────────────────

test('compare: paired per task with a difference and its interval; flags a slice drop over 3 points and a p95 rise over 10%', () => {
  const a = scoreRound(join(ROUNDS, 'r-base'), suite.tasks, { tasksFile: SUITE_FILE });
  const b = scoreRound(join(ROUNDS, 'r-next'), suite.tasks, { tasksFile: SUITE_FILE });
  const cmp = compareRounds(a, b);
  assert.deepEqual([cmp.a, cmp.b], ['r-base', 'r-next']);
  const task = (id) => cmp.tasks.find((t) => t.task === id);
  const research = task('research-fourth-attachment');
  assert.ok(near(research.diff, 1 / 3));
  assert.ok(research.ci.lo < research.diff && research.diff < research.ci.hi);
  assert.ok(near(task('memory-current').diff, 2 / 3));
  assert.ok(near(task('continuity-settled-constraint').diff, 1 / 3 - 1 / 2));
  const drop = cmp.flags.find((f) => f.kind === 'completion-drop');
  assert.equal(drop.slice, 'continuity');
  assert.ok(near(drop.points, -16.667, 0.01));
  assert.ok(!cmp.flags.some((f) => f.kind === 'completion-drop' && f.slice !== 'continuity'), 'only the slice that dropped');
  const slow = cmp.flags.filter((f) => f.kind === 'p95-rise').map((f) => f.slice);
  assert.ok(slow.includes('continuity'), JSON.stringify(cmp.flags));
  assert.ok(!slow.includes('research') && !slow.includes('all'), 'faster or a single slow timeout: no flag');
  assert.ok(cmp.flags.some((f) => f.kind === 'duration-source' && /pilot-wall.*queue/.test(f.detail)), 'the two rounds timed runs differently');
  assert.ok(cmp.flags.some((f) => f.kind === 'task-digest'), 'round 0 recorded no digest, so the messages cannot be shown equal');
  assert.equal(cmp.flags.some((f) => f.kind === 'settings'), false, 'same model, effort, mode and repeats');
  const md = renderComparison(cmp);
  assert.match(md, /^# Pilot rounds r-base → r-next/m);
  assert.match(md, /\| continuity-settled-constraint \| continuity \| 1\/2 \| 1\/3 \| -16\.7 \|/);
  assert.match(md, /completion-drop.*continuity/);
});

test('compare flags rounds run with different settings', (t) => {
  const root = tempRoot(t);
  cpSync(join(ROUNDS, 'r-next'), join(root, 'r-y'), { recursive: true });
  const plan = JSON.parse(readFileSync(join(root, 'r-y', 'plan.json'), 'utf8'));
  writeFileSync(join(root, 'r-y', 'plan.json'), JSON.stringify({ ...plan, effort: 'max' }));
  const cmp = compareRounds(scoreRound(join(ROUNDS, 'r-next'), suite.tasks), scoreRound(join(root, 'r-y'), suite.tasks));
  const flag = cmp.flags.find((f) => f.kind === 'settings');
  assert.match(flag.detail, /effort high → max/);
});

// ── The command ──────────────────────────────────────────────────────────

test('the command writes score.json and score.md, and compare writes its pair; it never touches a model or the bed', (t) => {
  const root = tempRoot(t);
  const out = join(root, 'out');
  const one = spawnSync(process.execPath, [SCORE, 'score', join(ROUNDS, 'r-base'), '--tasks', SUITE_FILE, '--out', out], { encoding: 'utf8' });
  assert.equal(one.status, 0, one.stderr);
  assert.ok(existsSync(join(out, 'score.json')) && existsSync(join(out, 'score.md')));
  assert.equal(JSON.parse(readFileSync(join(out, 'score.json'), 'utf8')).totals.runs.passed, 4);
  const two = spawnSync(process.execPath, [SCORE, 'compare', join(ROUNDS, 'r-base'), join(ROUNDS, 'r-next'), '--tasks', SUITE_FILE, '--out', out], { encoding: 'utf8' });
  assert.equal(two.status, 0, two.stderr);
  assert.ok(existsSync(join(out, 'compare-r-base-r-next.json')) && existsSync(join(out, 'compare-r-base-r-next.md')));
  assert.match(two.stdout, /completion-drop/);
  const bad = spawnSync(process.execPath, [SCORE, 'score'], { encoding: 'utf8' });
  assert.notEqual(bad.status, 0);
  assert.match(bad.stderr, /usage/);
  const src = readFileSync(SCORE, 'utf8');
  assert.doesNotMatch(src, /node:(http|https|net|child_process)|fetch\(|typesafe|test-bed\.mjs/, 'the scorer only reads files');
});

// ── Wave 1 follow-ups ────────────────────────────────────────────────────

test('follow-up: a stale value said in the plural is still the stale value', () => {
  const rule = { id: 'old', kind: 'historyLabelled', stale: ['Friday'], markers: ['used to', 'previously'] };
  assert.equal(checkRule(rule, 'Reports go out on Fridays.').pass, false, 'presented as current');
  assert.equal(checkRule(rule, 'Reports used to go out on Fridays.').pass, true, 'labelled as history');
  assert.equal(checkRule(rule, 'Reports go out on Fridayish days.').pass, true, 'only the plural, not any longer word');
});

test('follow-up: inOrder moves past each item by its normalised length', () => {
  const rule = { id: 'order', kind: 'inOrder', items: ['`x`', 'y'] };
  assert.equal(checkRule(rule, 'x y').pass, true, 'the ticks are not in the text, so they must not move the cursor');
  assert.equal(checkRule({ id: 'gap', kind: 'inOrder', items: ['a  b', 'c'] }, 'a bc').pass, true, 'a run of spaces is one space in the text');
  assert.equal(checkRule(rule, 'y x').pass, false);
});

test('follow-up: a directory where the artifact should be is a failed artifact, not a crash', (t) => {
  const root = tempRoot(t);
  const task = byId['research-fourth-attachment'];
  const dir = join(root, `${task.id}-1`);
  const at = join(dir, 'artifacts', 'workspaces', task.workspace, task.score.artifact);
  mkdirSync(at, { recursive: true });
  writeFileSync(join(at, 'notes.md'), '48,500 EUR\n');
  const run = { key: `${task.id}-1`, rep: 1, status: 'done', artifacts: { created: [`workspaces/${task.workspace}/${task.score.artifact}/notes.md`], modified: [] } };
  const scored = scoreRunDir(task, dir, run);
  assert.equal(scored.pass, false);
  assert.match(scored.reasons.join(' '), /not a file/);
});
