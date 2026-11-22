// Gate health from the records alone (overnight 10-17, OF11).
//
//   node scripts/gate-stats.mjs --since <iso> [--until <iso>] [--json]
//                               [--ref origin/main] [--repo <checkout>]
//
// Joins three things, all in the repository's common git directory or its
// history, and reads no chat or message text:
// - runs: `hollow-gate-runs/*.json`, one per `npm run gate`, failed ones too
//   (scripts/gate-run.mjs, from 2026-10-17 on);
// - passing records: `hollow-gate-records/*.json` (scripts/gate-record.mjs);
// - the PRs merged to `--ref`: `git log --first-parent --merges`.
//
// A passing gate belongs to the PR whose merge first brought its commit to
// main (`git rev-list <PR head> ^<main before>`). It is the useful one when no
// fingerprint input (the paths source-provenance.mjs hashes) differs between
// its commit and the merged head: a docs-only commit after the gate keeps it,
// as install readiness does. Any other gate on the PR was discarded, because
// main moved (main changed an input after the gate's base), because of an own
// fix (a non-merge commit on the branch changed an input), or both (mixed).
//
// A failed run is cancelled when a signal ended it (exit 128 + n: Ctrl-C, a
// killed turn); environment when it failed in a pre-check before any test ran
// (a refused node_modules or CARGO_TARGET_DIR, gate-record --start); a stale
// fixture when a failing test is a regenerate-and-commit drift test
// (FIXTURE_TEST); a flake when a test failed and a passing run has its
// fingerprint (same tree, other outcome); and other otherwise. Step wall times
// count completed steps only.
import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { RECORDS_DIR } from './gate-record.mjs';
import { RUNS_DIR } from './gate-run.mjs';

/** The fingerprint inputs: a copy of source-provenance.mjs's filter (gate-stats.test.mjs keeps them equal). */
export const INPUTS = /^(src\/|src-tauri\/src\/|scripts\/|public\/|package(-lock)?\.json$|index\.html$|vite\.config\.ts$|tsconfig\.json$|rust-toolchain\.toml$|docs\/(WHATS_NEW\.md|HOLLOW_APP_COMMAND_MAP\.tsv)$|src-tauri\/(Cargo\.(toml|lock)|tauri(\.[a-z-]+)?\.conf\.json|build\.rs|capabilities\/|icons\/))/;

/** Nearest-rank percentile of `values` (null when empty). */
export function percentile(values, p) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil((p / 100) * sorted.length) - 1)];
}

const round = n => Math.round(n * 100) / 100;

/**
 * The drift tests whose fix is to regenerate and commit (HOLLOW_REGENERATE_*):
 * the prompt fixtures (`*_fixture_is_current`), the IPC bindings
 * (`generated_types_are_current`) and IPC fixtures (`ts_bindings::fixture_*`),
 * and the AGENT_CONTEXT contract table. gate-stats.test.mjs keeps it in step
 * with src-tauri/src.
 */
export const FIXTURE_TEST = /(?:^|::)(?:\w+_fixture_is_current|generated_types_are_current|the_doc_holds_the_contract_table|ts_bindings::fixture_\w+)$/;

/** The steps that run before any test (gate-run.mjs's names): a failure there is the machine's, not the tree's. */
export const PRE_CHECKS = new Set(['start', 'gate:frontend', 'check-installed-lock', 'check-cargo-target']);

/** Why a finished, failed run failed; `passedFps` holds the fingerprints that passed. */
export function failureCause(run, passedFps) {
  const failing = run.failingTests ?? [];
  if (run.signal || run.exitCode >= 128) return 'cancelled';
  if (!failing.length && PRE_CHECKS.has(run.failedStep)) return 'environment';
  if (failing.some(t => FIXTURE_TEST.test(t))) return 'fixture';
  if (failing.length && passedFps.has(run.fingerprint)) return 'flake';
  return 'other';
}

function readJsonDir(dir) {
  let names = [];
  try {
    names = readdirSync(dir).filter(n => n.endsWith('.json'));
  } catch {
    return [];
  }
  return names
    .map(n => {
      try {
        return JSON.parse(readFileSync(join(dir, n), 'utf8'));
      } catch {
        return null;
      }
    })
    .filter(Boolean);
}

/** The git questions the analysis asks, answered by `git` in `repo`. */
export function gitOracle(repo) {
  const git = (...args) => execFileSync('git', args, { cwd: repo, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).trim();
  const touches = out => out.split('\n').some(p => p && INPUTS.test(p));
  const safe = (f, fallback) => {
    try {
      return f();
    } catch {
      return fallback;
    }
  };
  return {
    merges(ref, since) {
      const out = git('log', '--first-parent', '--merges', `--since=${since}`, '--format=%H%x09%P%x09%cI%x09%s', ref);
      return out
        .split('\n')
        .filter(Boolean)
        .map(line => {
          const [sha, parents, at, subject] = line.split('\t');
          const [p1, p2] = parents.split(' ');
          const m = subject.match(/^Merge pull request #(\d+) from (\S+)/);
          return { sha, p1, p2, at: new Date(at).toISOString(), pr: m ? Number(m[1]) : null, branch: m ? m[2].replace(/^[^/]+\//, '') : null };
        })
        .reverse();
    },
    branchCommits: (head, base) => new Set(git('rev-list', head, `^${base}`).split('\n').filter(Boolean)),
    sameInputs: (a, b) => safe(() => !touches(git('diff', '--name-only', a, b)), false),
    mainMoved: (commit, mainBefore) => safe(() => touches(git('diff', '--name-only', `${commit}...${mainBefore}`)), false),
    ownFix: (head, commit, mainBefore) => safe(() => touches(git('log', '--no-merges', '--format=', '--name-only', head, `^${commit}`, `^${mainBefore}`)), false),
    onRef: (commit, ref) => safe(() => (git('merge-base', '--is-ancestor', commit, ref), true), false),
  };
}

/** The numbers, from `runs`, passing `records` and the merges `git` knows. */
export function analyze({ runs, records, git, ref, since, until }) {
  const sinceIso = new Date(since).toISOString();
  const untilIso = new Date(until).toISOString();
  const inWindow = at => at && at >= sinceIso && at < untilIso;
  const merges = git.merges(ref, sinceIso).map(m => ({ ...m, commits: git.branchCommits(m.p2, m.p1) }));
  const windowMerges = merges.filter(m => m.at <= untilIso);
  const mergeOf = commit => merges.find(m => m.commits.has(commit)) ?? null;

  const passing = records.filter(r => inWindow(r.recordedAt)).sort((a, b) => a.recordedAt.localeCompare(b.recordedAt));
  const prs = windowMerges.map(m => ({ pr: m.pr, branch: m.branch, merge: m.sha, head: m.p2, at: m.at, gates: 0, useful: null, redundant: [], runs: 0, failedRuns: 0 }));
  const prOf = m => prs.find(p => p.merge === m.sha);
  const redundant = { total: 0, mainMoved: 0, ownFix: 0, mixed: 0, other: 0 };
  let unmerged = 0;
  let mergedLater = 0;
  let onMain = 0;
  for (const r of passing) {
    const m = mergeOf(r.commit);
    if (!m) {
      if (git.onRef(r.commit, ref)) onMain += 1;
      else unmerged += 1;
      continue;
    }
    const pr = prOf(m);
    if (!pr) {
      mergedLater += 1;
      continue;
    }
    pr.gates += 1;
    if (git.sameInputs(r.commit, m.p2)) {
      pr.useful = r.commit;
      continue;
    }
    const moved = git.mainMoved(r.commit, m.p1);
    const own = git.ownFix(m.p2, r.commit, m.p1);
    const cause = moved && own ? 'mixed' : moved ? 'main-moved' : own ? 'own-fix' : 'other';
    pr.redundant.push({ commit: r.commit, cause });
    redundant.total += 1;
    redundant[{ mixed: 'mixed', 'main-moved': 'mainMoved', 'own-fix': 'ownFix', other: 'other' }[cause]] += 1;
  }

  const windowRuns = runs.filter(r => inWindow(r.start));
  const passedFps = new Set(windowRuns.filter(r => r.exitCode === 0).map(r => r.fingerprint));
  const causes = { fixture: 0, flake: 0, cancelled: 0, environment: 0, other: 0 };
  const slowTests = {};
  const steps = {};
  let unfinished = 0;
  let onMergedPrs = 0;
  for (const r of windowRuns) {
    const m = mergeOf(r.commit);
    const pr = m && prOf(m);
    if (pr) {
      pr.runs += 1;
      onMergedPrs += 1;
    }
    for (const name of r.slowTests ?? []) slowTests[name] = (slowTests[name] ?? 0) + 1;
    if (r.end == null || r.exitCode == null) {
      unfinished += 1;
      continue;
    }
    for (const s of r.steps ?? []) if (s.ok) (steps[s.name] ??= []).push(s.seconds);
    if (r.exitCode === 0) continue;
    if (pr) pr.failedRuns += 1;
    const cause = failureCause(r, passedFps);
    r.cause = cause;
    causes[cause] += 1;
  }
  const failed = Object.values(causes).reduce((a, b) => a + b, 0);
  // Runs per machine (since 2026-11-01 a run names its `host`; hollow-slot
  // --remote brings a gate run on the remote runner back here).
  const hosts = {};
  for (const r of windowRuns) {
    const key = r.host ?? 'unrecorded';
    hosts[key] = (hosts[key] ?? 0) + 1;
  }
  return {
    since: sinceIso,
    until: untilIso,
    ref,
    merged: windowMerges.length,
    passing: { total: passing.length, mergedHeads: prs.filter(p => p.useful).length, redundant, mergedLater, unmerged, onMain },
    runs: {
      total: windowRuns.length,
      passed: windowRuns.filter(r => r.exitCode === 0).length,
      failed,
      unfinished,
      causes,
      hosts,
      perMergedPr: windowMerges.length ? round(onMergedPrs / windowMerges.length) : null,
      failures: windowRuns.filter(r => r.cause).map(r => ({ start: r.start, commit: r.commit, failedStep: r.failedStep ?? null, cause: r.cause, failingTests: r.failingTests ?? [] })),
    },
    flakes: causes.flake,
    slowTests,
    steps: Object.fromEntries(Object.entries(steps).map(([name, v]) => [name, { n: v.length, p50: percentile(v, 50), p95: percentile(v, 95) }])),
    prs,
  };
}

const CAUSE_TEXT = { 'main-moved': 'main moved', 'own-fix': 'own fix', mixed: 'mixed', other: 'other' };

/** The report a person reads. */
export function render(s) {
  const out = [];
  const { passing: p, runs } = s;
  out.push(`Gate stats ${s.since} .. ${s.until} (${s.ref})`);
  out.push(
    `Passing gates: ${p.total} (${p.mergedHeads} merged heads; ${p.redundant.total} redundant: ${p.redundant.mainMoved} main moved, ${p.redundant.ownFix} own fix, ${p.redundant.mixed} mixed, ${p.redundant.other} other; ${p.mergedLater} for PRs merged later, ${p.unmerged} unmerged, ${p.onMain} on main)`,
  );
  out.push(`Merged PRs: ${s.merged}`);
  if (runs.total) {
    out.push(`Runs: ${runs.total} (${runs.passed} passed, ${runs.failed} failed: ${runs.causes.fixture} fixture, ${runs.causes.flake} flake, ${runs.causes.cancelled} cancelled, ${runs.causes.environment} environment, ${runs.causes.other} other; ${runs.unfinished} unfinished)`);
    out.push(`Runs per merged PR: ${runs.perMergedPr ?? 'n/a'}`);
    const hosts = Object.entries(runs.hosts ?? {});
    if (hosts.some(([h]) => h !== 'unrecorded')) out.push(`Runs by machine: ${hosts.map(([h, n]) => `${h} ${n}`).join(', ')}`);
  } else {
    out.push('Runs: none recorded in this window (scripts/gate-run.mjs records them from 2026-10-17 on); failed runs are not counted.');
    if (s.merged) out.push(`Passing gates per merged PR: ${round(s.prs.reduce((n, pr) => n + pr.gates, 0) / s.merged)}`);
  }
  out.push(`Flakes: ${s.flakes}`);
  for (const pr of s.prs) {
    const why = pr.redundant.map(g => `${g.commit.slice(0, 8)} ${CAUSE_TEXT[g.cause]}`).join(', ');
    out.push(`  #${pr.pr ?? '?'} ${pr.branch ?? pr.merge.slice(0, 8)}: ${pr.gates} passing${pr.useful ? '' : ' (head not gated in window)'}${why ? `, discarded ${why}` : ''}; ${pr.runs} runs, ${pr.failedRuns} failed`);
  }
  for (const f of runs.failures) out.push(`  failed ${f.start} ${f.commit?.slice(0, 8) ?? '?'} at ${f.failedStep ?? '?'}: ${f.cause}${f.failingTests.length ? ` (${f.failingTests.join(', ')})` : ''}`);
  const names = Object.keys(s.steps);
  if (names.length) {
    out.push('Step wall (s): name n p50 p95');
    for (const name of names) out.push(`  ${name} ${s.steps[name].n} ${s.steps[name].p50} ${s.steps[name].p95}`);
  }
  const slow = Object.entries(s.slowTests);
  if (slow.length) out.push(`Over 60 s: ${slow.map(([n, c]) => `${n} x${c}`).join(', ')}`);
  return out.join('\n');
}

function args(argv) {
  const o = { json: false, ref: 'origin/main' };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--json') o.json = true;
    else if (['--since', '--until', '--ref', '--repo'].includes(a)) o[a.slice(2)] = argv[++i];
    else throw new Error(`unknown argument ${a}`);
  }
  if (!o.since || Number.isNaN(Date.parse(o.since))) throw new Error('--since <iso> is required');
  if (o.until && Number.isNaN(Date.parse(o.until))) throw new Error('--until must be a date');
  return o;
}

const isMain = () => {
  try {
    return resolve(process.argv[1]) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
};

if (process.argv[1] && isMain()) {
  try {
    const o = args(process.argv.slice(2));
    const repo = resolve(o.repo ?? fileURLToPath(new URL('..', import.meta.url)));
    const common = resolve(repo, execFileSync('git', ['rev-parse', '--git-common-dir'], { cwd: repo, encoding: 'utf8' }).trim());
    const s = analyze({
      runs: readJsonDir(join(common, RUNS_DIR)),
      records: readJsonDir(join(common, RECORDS_DIR)).filter(r => r.gate === 'passed'),
      git: gitOracle(repo),
      ref: o.ref,
      since: o.since,
      until: o.until ?? new Date().toISOString(),
    });
    console.log(o.json ? JSON.stringify(s, null, 2) : render(s));
  } catch (e) {
    console.error(`gate-stats: ${e?.message ?? e}`);
    process.exit(2);
  }
}
