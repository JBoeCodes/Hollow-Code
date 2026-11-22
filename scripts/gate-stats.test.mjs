// gate-stats (overnight 10-17, OF11): gate runs, passing records and the PRs
// merged to main, joined into the morning's gate-health numbers. The fixture
// is a real scratch git repository, so the ancestry and diff checks run as
// they do on the real one.
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { analyze, failureCause, FIXTURE_TEST, INPUTS, percentile, render } from './gate-stats.mjs';

const root = fileURLToPath(new URL('..', import.meta.url));
const gitEnv = { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1' };

function fixtureRepo() {
  const repo = mkdtempSync(join(tmpdir(), 'gate-stats-'));
  let clock = Date.parse('2026-10-16T21:00:00Z');
  const tick = () => new Date((clock += 60_000)).toISOString();
  const git = (...args) => {
    const at = tick();
    return execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', ...args], {
      cwd: repo,
      encoding: 'utf8',
      env: { ...gitEnv, GIT_AUTHOR_DATE: at, GIT_COMMITTER_DATE: at },
    }).trim();
  };
  const commit = (path, text = path) => {
    mkdirSync(dirname(join(repo, path)), { recursive: true });
    writeFileSync(join(repo, path), `${text}\n`);
    git('add', '-A');
    git('commit', '-q', '-m', `edit ${path}`);
    return git('rev-parse', 'HEAD');
  };
  const merge = (branch, pr) => {
    git('checkout', '-q', 'main');
    git('merge', '-q', '--no-ff', '-m', `Merge pull request #${pr} from me/${branch}`, branch);
    return git('rev-parse', 'HEAD');
  };
  const records = [];
  const runs = [];
  const fp = sha => sha.padEnd(64, '0').slice(0, 64);
  const gate = (sha, extra = {}) => {
    const at = tick();
    records.push({ sourceFingerprint: fp(sha), commit: sha, gate: 'passed', recordedAt: at });
    runs.push({ commit: sha, fingerprint: fp(sha), start: at, end: at, exitCode: 0, ok: true, failedStep: null, failingTests: [], slowTests: [], steps: [{ name: 'test', seconds: 10, ok: true }, { name: 'cargo test', seconds: 100, ok: true }], ...extra });
  };
  const failed = (sha, extra) => {
    const at = tick();
    runs.push({ commit: sha, fingerprint: fp(sha), start: at, end: at, exitCode: 1, ok: false, failingTests: [], slowTests: [], steps: [{ name: 'test', seconds: 12, ok: true }, { name: 'cargo test', seconds: 50, ok: false }], ...extra });
  };

  git('init', '-q', '-b', 'main');
  commit('src/base.ts');
  commit('docs/base.md');
  // B: one gate, merged as gated.
  git('checkout', '-q', '-b', 'a');
  const a1 = commit('src/a1.ts');
  gate(a1);
  git('checkout', '-q', 'main');
  git('checkout', '-q', '-b', 'b');
  const b1 = commit('src/b1.ts');
  failed(b1, { failedStep: 'typecheck', fingerprint: 'f'.repeat(64) }); // run on uncommitted changes: another tree
  gate(b1);
  merge('b', 1);
  // A: main moved under a1, so A merges main and gates again.
  git('checkout', '-q', 'a');
  git('merge', '-q', '--no-edit', 'main');
  const a2 = git('rev-parse', 'HEAD');
  failed(a2, { failingTests: ['perf_hot_paths::tests::hot_paths_stay_under_their_ceilings'], slowTests: ['chat::slow'] });
  gate(a2);
  merge('a', 2);
  // C: an own fix after the first gate, then a docs-only commit after the second.
  git('checkout', '-q', '-b', 'c');
  const c1 = commit('src/c1.ts');
  failed(c1, { failingTests: ['chat::g2_budget_tests::turn_budget_fixture_is_current'] });
  // Ctrl-C on the tree that then passed, and a refused CARGO_TARGET_DIR on it: neither is a flake.
  failed(c1, { exitCode: 130, failedStep: 'cargo test', steps: [] });
  failed(c1, { failedStep: 'check-cargo-target', steps: [] });
  gate(c1);
  const c2 = commit('src/c1.ts', 'fixed');
  gate(c2);
  commit('docs/c.md');
  merge('c', 3);
  // D: gated, then main moved (E) and D fixed its own code too: mixed.
  git('checkout', '-q', '-b', 'd');
  const d1 = commit('src/d1.ts');
  gate(d1);
  git('checkout', '-q', 'main');
  git('checkout', '-q', '-b', 'e');
  const e1 = commit('src/e1.ts');
  gate(e1);
  merge('e', 4);
  git('checkout', '-q', 'd');
  git('merge', '-q', '--no-edit', 'main');
  const d3 = commit('src/d1.ts', 'fixed');
  gate(d3);
  merge('d', 5);
  // F: gated, never merged.
  git('checkout', '-q', '-b', 'f');
  gate(commit('src/f1.ts'));
  // A run still going (or killed outright): no end.
  runs.push({ commit: e1, fingerprint: fp(e1), start: tick(), end: null, exitCode: null });

  const common = join(repo, '.git');
  mkdirSync(join(common, 'hollow-gate-records', 'start'), { recursive: true });
  mkdirSync(join(common, 'hollow-gate-runs', 'logs'), { recursive: true });
  for (const r of records) writeFileSync(join(common, 'hollow-gate-records', `${r.sourceFingerprint}.json`), JSON.stringify(r));
  runs.forEach((r, i) => writeFileSync(join(common, 'hollow-gate-runs', `${r.start.replace(/[:.]/g, '-')}-${String(i).padStart(8, '0')}.json`), JSON.stringify(r)));
  return { repo, until: tick(), shas: { a1, a2, b1, c1, c2, d1, d3, e1 } };
}

const stats = (repo, ...args) => {
  const r = spawnSync(process.execPath, [join(root, 'scripts/gate-stats.mjs'), '--repo', repo, '--ref', 'main', ...args], { encoding: 'utf8', env: gitEnv });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
};

test('gate-stats tells a gate discarded because main moved from one discarded for an own fix', () => {
  const { repo, until, shas } = fixtureRepo();
  try {
    const s = JSON.parse(stats(repo, '--since', '2026-10-16T21:00Z', '--until', until, '--json'));
    assert.equal(s.passing.total, 9);
    assert.equal(s.merged, 5);
    assert.equal(s.passing.mergedHeads, 5, 'the docs-only commit after c2 still counts c2 for C');
    assert.deepEqual(s.passing.redundant, { total: 3, mainMoved: 1, ownFix: 1, mixed: 1, other: 0 });
    assert.equal(s.passing.unmerged, 1);
    const cause = sha => s.prs.flatMap(p => p.redundant).find(g => g.commit === sha)?.cause;
    assert.equal(cause(shas.a1), 'main-moved');
    assert.equal(cause(shas.c1), 'own-fix');
    assert.equal(cause(shas.d1), 'mixed');
    assert.equal(cause(shas.c2), undefined);
    assert.deepEqual(s.prs.map(p => p.pr), [1, 2, 3, 4, 5]);
    assert.deepEqual(s.runs.causes, { fixture: 1, flake: 1, cancelled: 1, environment: 1, other: 1 });
    assert.equal(s.runs.total, 15);
    assert.equal(s.runs.failed, 5);
    assert.equal(s.runs.unfinished, 1);
    assert.equal(s.runs.perMergedPr, 2.8, '14 finished-or-not runs on the 5 merged PRs');
    assert.equal(s.flakes, 1);
    assert.deepEqual(s.slowTests, { 'chat::slow': 1 });
    assert.deepEqual(s.steps.test, { n: 12, p50: 10, p95: 12 });
    assert.deepEqual(s.steps['cargo test'], { n: 9, p50: 100, p95: 100 }, 'a failed step has no wall time');
    // The text report says the same.
    const text = stats(repo, '--since', '2026-10-16T21:00Z', '--until', until);
    assert.match(text, /Passing gates: 9/);
    assert.match(text, /1 main moved, 1 own fix, 1 mixed/);
    assert.match(text, /Runs per merged PR: 2\.8/);
    assert.match(text, /5 failed: 1 fixture, 1 flake, 1 cancelled, 1 environment, 1 other/);
  } finally {
    rmSync(repo, { recursive: true, force: true });
  }
});

test('a failed run is a flake only when a test failed on a tree that passed; signals are cancelled, pre-check refusals environment', () => {
  const passed = new Set(['p'.repeat(64)]);
  const run = over => ({ fingerprint: 'p'.repeat(64), exitCode: 1, signal: null, failedStep: 'cargo test', failingTests: ['chat::tests::breaks'], ...over });
  assert.equal(failureCause(run(), passed), 'flake');
  assert.equal(failureCause(run({ fingerprint: 'q'.repeat(64) }), passed), 'other');
  assert.equal(failureCause(run({ exitCode: 130 }), passed), 'cancelled', 'Ctrl-C');
  assert.equal(failureCause(run({ exitCode: 143, signal: 'SIGTERM' }), passed), 'cancelled');
  assert.equal(failureCause(run({ failingTests: [] }), passed), 'other', 'no failing test: not a flake');
  for (const step of ['start', 'gate:frontend', 'check-installed-lock', 'check-cargo-target']) {
    assert.equal(failureCause(run({ failedStep: step, failingTests: [] }), passed), 'environment', step);
  }
  assert.equal(failureCause(run({ failingTests: ['chat::tests::mode_wording_fixture_is_current'] }), passed), 'fixture');
  assert.equal(failureCause(run({ failingTests: ['ts_bindings::generated_types_are_current'] }), passed), 'fixture');
  assert.equal(failureCause(run({ failingTests: ['ts_bindings::fixture_tasks'] }), passed), 'fixture');
  // A name that merely says "fixture" is a test like any other.
  assert.equal(failureCause(run({ failingTests: ['fixtures::loads_the_sample_vault'] }), passed), 'flake');
  assert.equal(failureCause(run({ failingTests: ['src/fixture-loader.test.ts > reads it'], fingerprint: 'q'.repeat(64) }), passed), 'other');
});

test('every regenerate-and-commit drift test in src-tauri counts as a stale fixture', () => {
  const names = [];
  const walk = dir => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      if (e.isDirectory()) walk(join(dir, e.name));
      else if (e.name.endsWith('.rs')) {
        const text = readFileSync(join(dir, e.name), 'utf8');
        if (!/HOLLOW_REGENERATE_/.test(text)) continue;
        for (const m of text.matchAll(/fn (\w+_(?:is|are)_current|the_doc_holds_the_contract_table)\(\)/g)) names.push(`some::tests::${m[1]}`);
        if (e.name === 'ts_bindings.rs') for (const m of text.matchAll(/#\[test\]\s*fn (fixture_\w+)\(\)/g)) names.push(`ts_bindings::${m[1]}`);
      }
    }
  };
  walk(join(root, 'src-tauri/src'));
  for (const want of ['generated_types_are_current', 'turn_budget_fixture_is_current', 'probe_prompts_fixture_is_current', 'the_doc_holds_the_contract_table', 'ts_bindings::fixture_tasks']) {
    assert.ok(names.some(n => n.endsWith(want)), `${want} is still in src-tauri/src`);
  }
  for (const name of names) assert.match(name, FIXTURE_TEST, name);
  // Other tests in those files are not fixtures.
  assert.doesNotMatch('ts_bindings::run_queue_enums_match_run_queue_variants', FIXTURE_TEST);
});

test('the fingerprint inputs match source-provenance.mjs', () => {
  const text = readFileSync(join(root, 'scripts/source-provenance.mjs'), 'utf8');
  assert.ok(text.includes(`.filter(path => /${INPUTS.source}/.test(path))`), 'INPUTS is a copy of source-provenance.mjs');
});

test('percentiles are nearest-rank', () => {
  assert.equal(percentile([], 50), null);
  assert.equal(percentile([5], 95), 5);
  assert.equal(percentile([1, 2, 3, 4], 50), 2);
  assert.equal(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 100], 95), 19);
});

test('runs are counted by the machine they ran on, and the report names them once any run says', () => {
  const git = { merges: () => [], branchCommits: () => new Set(), onRef: () => false, sameInputs: () => false, mainMoved: () => false, ownFix: () => false };
  const run = (host, start) => ({ start, end: start, exitCode: 0, fingerprint: 'f'.repeat(64), commit: 'c'.repeat(40), steps: [], ...(host ? { host } : {}) });
  const window = { ref: 'origin/main', since: '2026-11-01T00:00Z', until: '2026-11-02T00:00Z', records: [], git };
  const s = analyze({ ...window, runs: [run('build-mac', '2026-11-01T10:00:00.000Z'), run('build-mac', '2026-11-01T11:00:00.000Z'), run('dev-mac', '2026-11-01T12:00:00.000Z'), run(null, '2026-11-01T13:00:00.000Z')] });
  assert.deepEqual(s.runs.hosts, { 'build-mac': 2, 'dev-mac': 1, unrecorded: 1 });
  assert.match(render(s), /^Runs by machine: build-mac 2, dev-mac 1, unrecorded 1$/m);
  // Records from before hosts were recorded: no machine line.
  const old = analyze({ ...window, runs: [run(null, '2026-11-01T13:00:00.000Z')] });
  assert.doesNotMatch(render(old), /Runs by machine/);
});
