import { fileURLToPath } from 'node:url';
// HF06: the retrieval label report (scripts/agent-eval/retrieval-labels.mjs)
// over a fixture of five manifests (fixtures/retrieval-labels/):
//
//   run-a  rank  local  semantic  kept
//          1     3      0.82      yes
//          2     2      –         no
//          3     2      0.71      yes
//          4     1      0.62      no
//   run-b  1     4      –         no
//          2     1      0.91      yes
//          3     4      0.64      no
//   run-c  1     2      0.75      yes   (c1 and c2: one note, one score)
//          2     2      0.75      no
//          3     0      0.66      yes   (semantic only)
//          4     3      –         no
//   run-d  1     2      0.70      yes   (all kept: no AUC, counted)
//          2     1      –         yes
//   run-e  engine "local": not labelled by Jev, skipped.
//
// Per-run AUC (kept × dropped pairs, a tie 0.5; a missing semantic score 0):
//   rank (-rank)  a: (-1>-2, -1>-4, -3<-2, -3>-4) 3/4 = 0.75
//                 b: (-2<-1, -2>-3) 1/2 = 0.5
//                 c: as a, 0.75                       mean 0.6667, median 0.75
//   local         a: (3>2, 3>1, 2=2, 2>1) 3.5/4 = 0.875
//                 b: (1<4, 1<4) 0
//                 c: (2=2, 2<3, 0<2, 0<3) 0.5/4 = 0.125   mean 0.3333
//   semantic      a: 4/4 = 1;  b: 2/2 = 1
//                 c: (.75=.75, .75>0, .66<.75, .66>0) 2.5/4 = 0.625  mean 0.875
// Pooled over all 13 candidates (7 kept × 6 dropped = 42 pairs; run-d's
// kept passages count):
//   rank      kept -1,-3,-2,-1,-3,-1,-2 vs dropped -2,-4,-1,-3,-2,-4:
//             5.5+2.5+4+5.5+2.5+5.5+4 = 29.5/42 = 0.7024
//   local     kept 3,2,1,2,0,2,1 vs dropped 2,1,4,4,2,3:
//             3.5+2+0.5+2+0+2+0.5 = 10.5/42 = 0.25
//   semantic  kept .82,.71,.91,.75,.66,.70,0 vs dropped 0,.62,0,.64,.75,0:
//             6+5+6+5.5+5+5+1.5 = 34/42 = 0.8095
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  CURRENT,
  FLOORS,
  LOCAL_BINS,
  RRF_K,
  SEMANTIC_BANDS,
  WEIGHTS,
  agentLabels,
  agentRun,
  auc,
  bands,
  fuse,
  labelledRun,
  loadRuns,
  render,
  report,
  summarize,
  sweep,
} from './agent-eval/retrieval-labels.mjs';

const FIXTURE = fileURLToPath(new URL('./agent-eval/fixtures/retrieval-labels/', import.meta.url));
const rust = (file) => readFileSync(new URL(`../src-tauri/src/${file}`, import.meta.url), 'utf8');

test('the current setting is retrieval_broker.rs\'s', () => {
  const source = rust('retrieval_broker.rs');
  const constant = (name) => Number(source.match(new RegExp(`pub const ${name}: f(?:32|64) = ([\\d.]+);`))[1]);
  assert.equal(constant('RRF_K'), RRF_K);
  assert.equal(constant('LEXICAL_WEIGHT'), CURRENT.lexicalWeight);
  assert.equal(constant('SEMANTIC_WEIGHT'), CURRENT.semanticWeight);
  assert.equal(constant('SEMANTIC_MIN'), CURRENT.floor);
});

test('AUC is Mann–Whitney with ties counted 0.5', () => {
  assert.equal(auc([1, 2, 3, 4], [false, false, true, true]), 1);
  assert.equal(auc([4, 3, 2, 1], [false, false, true, true]), 0);
  assert.equal(auc([1, 1], [true, false]), 0.5);
  // kept 2 vs dropped 1 (win) and 2 (tie): 1.5/2
  assert.equal(auc([1, 2, 2], [false, true, false]), 0.75);
  assert.equal(auc([1, 2], [true, true]), null);
  assert.equal(auc([1, 2], [false, false]), null);
});

test('only Jev-labelled runs load; an all-kept run is skipped from per-run AUCs and counted', () => {
  const { runs, counts } = loadRuns(FIXTURE);
  assert.deepEqual(counts, { files: 5, unreadable: 0, noCandidates: 0, notJev: 1, labelled: 4, unlabeled: 0, malformed: 0 });
  const s = summarize(runs);
  assert.equal(s.runs, 4);
  assert.equal(s.usableRuns, 3);
  assert.deepEqual(s.skipped, { allKept: 1, noneKept: 0 });
  assert.equal(s.candidates, 13);
  assert.equal(s.kept, 7);
  assert.deepEqual(s.scores, {
    rank: { pooled: 0.7024, meanPerRun: 0.6667, medianPerRun: 0.75 },
    local: { pooled: 0.25, meanPerRun: 0.3333, medianPerRun: 0.125 },
    semantic: { pooled: 0.8095, meanPerRun: 0.875, medianPerRun: 1 },
  });
  assert.deepEqual(
    s.perRun.map((r) => [r.runId, r.rank, r.local, r.semantic]),
    [
      ['run-a', 0.75, 0.875, 1],
      ['run-b', 0.5, 0, 1],
      ['run-c', 0.75, 0.125, 0.625],
    ],
  );
});

test('a mixed pool keeps its unlabeled count: non-Jev candidates are reported, not dropped', () => {
  // A Jev-labelled run whose pool also holds two candidates Jev never decided.
  const manifest = {
    runId: 'run-m',
    retrieval: {
      engine: 'jev',
      candidates: [
        { basis: 'jev', rank: 1, localScore: 2, semanticScore: 0.7, kept: true },
        { basis: 'local', rank: 2, localScore: 1, kept: false },
        { basis: 'jev', rank: 3, localScore: 1, kept: false },
        { basis: 'local', rank: 4, localScore: 0, semanticScore: 0.65 },
      ],
    },
  };
  const run = labelledRun(manifest);
  assert.deepEqual(run.candidates.map((c) => c.rank), [1, 3]);
  assert.equal(run.unlabeled, 2);
  assert.equal(summarize([run]).unlabeled, 2);
  const dir = mkdtempSync(join(tmpdir(), 'retrieval-labels-mixed-'));
  try {
    writeFileSync(join(dir, 'run-m.json'), JSON.stringify(manifest));
    const r = report(dir);
    assert.deepEqual(r.inputs, { files: 1, unreadable: 0, noCandidates: 0, notJev: 0, labelled: 1, unlabeled: 2, malformed: 0 });
    assert.match(render(r), /Candidates: 2, kept 1, unlabeled 2 \(not decided by Jev\)/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a Jev-decided candidate without kept or rank is malformed; a skipped Jev run keeps its counts', () => {
  const dir = mkdtempSync(join(tmpdir(), 'retrieval-labels-malformed-'));
  try {
    // run-x: two labelled, one Jev-decided without `kept`, one Jev never decided.
    writeFileSync(join(dir, 'run-x.json'), JSON.stringify({
      runId: 'run-x',
      retrieval: {
        engine: 'jev',
        candidates: [
          { basis: 'jev', rank: 1, localScore: 2, semanticScore: 0.7, kept: true },
          { basis: 'jev', rank: 2, localScore: 1, kept: false },
          { basis: 'jev', rank: 3, localScore: 1 },
          { basis: 'local', rank: 4, localScore: 0 },
        ],
      },
    }));
    // run-y: Jev's engine but nothing usable: skipped, its candidates still counted.
    writeFileSync(join(dir, 'run-y.json'), JSON.stringify({
      runId: 'run-y',
      retrieval: { engine: 'jev', candidates: [{ basis: 'local', rank: 1, localScore: 1 }, { basis: 'jev', kept: true }] },
    }));
    const x = labelledRun(JSON.parse(readFileSync(join(dir, 'run-x.json'), 'utf8')));
    assert.equal(x.unlabeled, 1);
    assert.equal(x.malformed, 1);
    const r = report(dir);
    assert.deepEqual(r.inputs, { files: 2, unreadable: 0, noCandidates: 0, notJev: 1, labelled: 1, unlabeled: 2, malformed: 2 });
    assert.equal(r.auc.unlabeled, 1);
    assert.equal(r.auc.malformed, 1);
    assert.match(render(r), /Candidates: 2, kept 1, unlabeled 2 \(not decided by Jev\), malformed 2 \(decided by Jev without a kept label or a rank\)\./);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('keep rate by band: missing scores are "none", the last band open-ended', () => {
  const { runs } = loadRuns(FIXTURE);
  const candidates = runs.flatMap((r) => r.candidates);
  const row = (rows, band) => rows.find((b) => b.band === band);
  const sem = bands(candidates, 'semanticScore', SEMANTIC_BANDS);
  assert.deepEqual(
    sem.map((b) => [b.band, b.n, b.kept]),
    [
      ['none', 4, 1],
      ['0.60–0.65', 2, 0],
      ['0.65–0.70', 1, 1],
      ['0.70–0.75', 2, 2],
      ['0.75–0.80', 2, 1],
      ['0.80–0.85', 1, 1],
      ['≥0.90', 1, 1],
    ],
  );
  assert.equal(row(sem, '0.75–0.80').rate, 0.5);
  assert.ok(row(sem, '0.75–0.80').ci.lo < 0.5 && row(sem, '0.75–0.80').ci.hi > 0.5);
  const local = bands(candidates, 'localScore', LOCAL_BINS, { decimals: 0 });
  assert.deepEqual(
    local.map((b) => [b.band, b.n, b.kept]),
    [
      ['0', 1, 1],
      ['1', 3, 2],
      ['2', 5, 3],
      ['3', 2, 1],
      ['4', 2, 0],
    ],
  );
  assert.equal(bands([{ localScore: 12, kept: true }], 'localScore', LOCAL_BINS, { decimals: 0 })[0].band, '≥10');
  assert.equal(bands([{ localScore: 6, kept: true }], 'localScore', LOCAL_BINS, { decimals: 0 })[0].band, '5–6');
});

test('fusion re-ranks a pool the way blend does', () => {
  const { runs } = loadRuns(FIXTURE);
  const c = runs.find((r) => r.runId === 'run-c').candidates;
  // Lexical ranks: rank-4 (local 3) 1, rank-1 2, rank-2 3; semantic: 0.75 → 1, 0.66 → 2.
  // Fused: rank-1 1/62+1/61, rank-2 1/63+1/61, rank-4 1/61, rank-3 (semantic only) 1/62.
  assert.deepEqual(fuse(c, CURRENT), [1, 2, 4, 3]);
  // A floor of 0.80 leaves no semantic rank: lexical order, and the
  // semantic-only passage leaves the pool and ranks last.
  assert.deepEqual(fuse(c, { ...CURRENT, floor: 0.8 }), [2, 3, 4, 1]);
  // No semantic weight: lexical order, the semantic-only passage (fused 0) last.
  assert.deepEqual(fuse(c, { ...CURRENT, semanticWeight: 0 }), [2, 3, 4, 1]);
  // No lexical weight: semantic order; rank-1 and rank-2 tie (one note) and
  // the better lexical rank goes first; rank-4 (no semantic rank, fused 0) last.
  assert.deepEqual(fuse(c, { ...CURRENT, lexicalWeight: 0 }), [1, 2, 3, 4]);
});

test('the sweep covers the grid, flags floors under the recorded one, and never ranks below the current', () => {
  const { runs } = loadRuns(FIXTURE);
  const s = sweep(runs);
  assert.equal(s.grid.length, WEIGHTS.length * WEIGHTS.length * FLOORS.length);
  const at = (floor) => s.grid.filter((g) => g.floor === floor).map((g) => [g.lexicalWeight, g.semanticWeight, g.meanPerRun, g.pooled]);
  assert.deepEqual(at(0.5), at(0.6));
  assert.ok(s.grid.filter((g) => g.floor < 0.6).every((g) => !g.identifiable));
  assert.equal(s.current.floor, 0.6);
  assert.ok(s.best.identifiable);
  assert.ok(s.best.meanPerRun >= s.current.meanPerRun);
  assert.equal(s.fidelity.candidates, 13);
  assert.deepEqual(s.fidelity.recorded, { pooled: 0.7024, meanPerRun: 0.6667 });
});

test('the report says labels are a proxy and carries no path, text or reason', () => {
  const r = report(FIXTURE);
  const md = render(r);
  const json = JSON.stringify(r);
  assert.match(md, /proxy, not ground truth/);
  assert.match(md, /1 all kept/);
  for (const out of [md, json]) {
    assert.doesNotMatch(out, /secret/);
    assert.doesNotMatch(out, /notes\//);
    assert.doesNotMatch(out, /"path"/);
    assert.doesNotMatch(out, /sha256|0{64}/);
  }
});

// S26: the agent label set over four manifests.
//   run-p (jev, reply checked)  rank  local  semantic  kept  cited
//                               1     3      0.80      yes   yes
//                               2     1      –         yes   no
//                               3     2      0.70      yes   yes
//                               4     0      0.65      no    –
//                               5     1      –         yes   no
//         opened outside its folder: notes/a.md (rank 1), notes/x.md (rank 4), notes/z.md (none)
//   run-q (local, reply checked) 1 local 2 kept, 2 local 1 kept; none cited (no AUC, counted)
//         opened: notes/w.md (none)
//   run-r (jev, before S26: no citedChecked, no notesOpened): no agent labels
//   run-s (reply checked, no candidates) opened: notes/q.md (retrieval didn't run)
//   run-t (local, skipped-followup query) opened: notes/t.md, a candidate (retrieval didn't run)
// Citation: 2 of 6 kept = 0.3333. AUC against cited among the kept:
//   run-p rank  cited -1,-3 vs -2,-5: 3/4 = 0.75; local 3,2 vs 1,1: 1; semantic .8,.7 vs 0,0: 1
//   pooled rank  cited -1,-3 vs -2,-5,-1,-2: (1+1+.5+1)+(0+1+0+0) = 4.5/8 = 0.5625
//   pooled local cited 3,2 vs 1,1,2,1: 4 + 3.5 = 7.5/8 = 0.9375; semantic 8/8 = 1
// Need-recall (run-p, run-q): 4 opened, 2 among candidates (0.5), 1 kept (0.25);
//   run-s and run-t: 2 notes opened where retrieval didn't run, counted apart.
const AGENT_MANIFESTS = {
  'run-p': {
    runId: 'run-p',
    agentReads: { readSource: 0, readSourceFailed: 0, searchVault: 0, notesOpened: ['notes/a.md', 'notes/x.md', 'notes/z.md'] },
    retrieval: {
      engine: 'jev',
      citedChecked: true,
      candidates: [
        { path: 'notes/a.md', basis: 'jev', rank: 1, localScore: 3, semanticScore: 0.8, kept: true, cited: true },
        { path: 'notes/b.md', basis: 'jev', rank: 2, localScore: 1, kept: true },
        { path: 'notes/c.md', basis: 'jev', rank: 3, localScore: 2, semanticScore: 0.7, kept: true, cited: true },
        { path: 'notes/x.md', basis: 'jev', rank: 4, localScore: 0, semanticScore: 0.65, kept: false },
        { path: 'notes/e.md', basis: 'jev', rank: 5, localScore: 1, kept: true },
      ],
    },
  },
  'run-q': {
    runId: 'run-q',
    agentReads: { readSource: 0, readSourceFailed: 0, searchVault: 0, notesOpened: ['notes/w.md'] },
    retrieval: {
      engine: 'local',
      citedChecked: true,
      candidates: [
        { path: 'notes/f.md', basis: 'local', rank: 1, localScore: 2, kept: true },
        { path: 'notes/g.md', basis: 'local', rank: 2, localScore: 1, kept: true },
      ],
    },
  },
  'run-r': {
    runId: 'run-r',
    agentReads: { readSource: 1, readSourceFailed: 0, searchVault: 0 },
    retrieval: {
      engine: 'jev',
      candidates: [
        { path: 'notes/h.md', basis: 'jev', rank: 1, localScore: 2, kept: true },
        { path: 'notes/i.md', basis: 'jev', rank: 2, localScore: 1, kept: false },
      ],
    },
  },
  'run-s': {
    runId: 'run-s',
    agentReads: { readSource: 0, readSourceFailed: 0, searchVault: 0, notesOpened: ['notes/q.md'] },
    retrieval: { engine: 'local', citedChecked: true, candidates: [] },
  },
  'run-t': {
    runId: 'run-t',
    agentReads: { readSource: 0, readSourceFailed: 0, searchVault: 0, notesOpened: ['notes/t.md'] },
    retrieval: {
      engine: 'local',
      queryBasis: 'skipped-followup',
      candidates: [{ path: 'notes/t.md', basis: 'local', rank: 1, localScore: 2, kept: true }],
    },
  },
};

test('agent labels: citation rate, AUC against cited and need-recall from the agent\'s own reads', async () => {
  assert.equal(agentRun(AGENT_MANIFESTS['run-r']), null, 'a manifest from before S26 has none');
  assert.deepEqual(agentRun(AGENT_MANIFESTS['run-p']).need, { opened: 3, candidates: 2, kept: 1 });
  // Where retrieval didn't run, the opened notes are no need-recall misses.
  for (const id of ['run-s', 'run-t']) {
    assert.equal(agentRun(AGENT_MANIFESTS[id]).need, null, id);
    assert.equal(agentRun(AGENT_MANIFESTS[id]).unretrieved, 1, id);
  }
  assert.equal(agentRun(AGENT_MANIFESTS['run-p']).unretrieved, 0);
  const dir = mkdtempSync(join(tmpdir(), 'retrieval-labels-agent-'));
  try {
    for (const [id, manifest] of Object.entries(AGENT_MANIFESTS)) writeFileSync(join(dir, `${id}.json`), JSON.stringify(manifest));
    const r = report(dir);
    const g = r.agent;
    assert.deepEqual(g, agentLabels(Object.values(AGENT_MANIFESTS).map(agentRun).filter(Boolean)));
    assert.equal(g.citation.runs, 2);
    assert.equal(g.citation.kept, 6);
    assert.equal(g.citation.cited, 2);
    assert.equal(g.citation.rate, 0.3333);
    assert.ok(g.citation.ci.lo < 0.3333 && g.citation.ci.hi > 0.3333);
    assert.equal(g.auc.usableRuns, 1);
    assert.deepEqual(g.auc.skipped, { allKept: 0, noneKept: 1 });
    assert.deepEqual(g.auc.scores, {
      rank: { pooled: 0.5625, meanPerRun: 0.75, medianPerRun: 0.75 },
      local: { pooled: 0.9375, meanPerRun: 1, medianPerRun: 1 },
      semantic: { pooled: 1, meanPerRun: 1, medianPerRun: 1 },
    });
    assert.deepEqual(
      { ...g.needRecall, ci: undefined },
      {
        runs: 2,
        opened: 4,
        candidates: 2,
        kept: 1,
        rate: 0.5,
        ci: undefined,
        keptRate: 0.25,
        noRetrieval: { runs: 2, opened: 2 },
      },
    );
    // The Jev label set is unchanged by them: run-p and run-r are Jev-labelled.
    assert.equal(r.inputs.labelled, 2);
    const md = render(r);
    assert.match(md, /### Agent labels/);
    assert.match(md, /Citation: 2 of 6 kept passages named in the reply \(0\.3333/);
    assert.match(md, /Need-recall: of 4 notes .* retrieval had 2 among its candidates \(0\.5/);
    assert.match(md, /Not counted: 2 notes opened in 2 runs where retrieval didn't run/);
    for (const out of [md, JSON.stringify(r)]) assert.doesNotMatch(out, /notes\/|"path"/);
    // A folder with agent labels only still reports.
    rmSync(join(dir, 'run-p.json'));
    rmSync(join(dir, 'run-r.json'));
    const { spawnSync } = await import('node:child_process');
    const script = fileURLToPath(new URL('./agent-eval/retrieval-labels.mjs', import.meta.url));
    const agentOnly = spawnSync(process.execPath, [script, dir], { encoding: 'utf8' });
    assert.equal(agentOnly.status, 0, agentOnly.stderr);
    assert.match(agentOnly.stdout, /Citation: 0 of 2 kept passages/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a report from manifests before S26 says it has no agent labels', () => {
  const r = report(FIXTURE);
  assert.equal(r.agent.citation.runs, 0);
  assert.equal(r.agent.needRecall.runs, 0);
  assert.match(render(r), /No run carries agent labels/);
});

test('the CLI writes JSON and Markdown; with no labelled runs it fails', async () => {
  const { spawnSync } = await import('node:child_process');
  const script = fileURLToPath(new URL('./agent-eval/retrieval-labels.mjs', import.meta.url));
  const dir = mkdtempSync(join(tmpdir(), 'retrieval-labels-'));
  try {
    const out = join(dir, 'report.json');
    const ok = spawnSync(process.execPath, [script, FIXTURE, '--json', out], { encoding: 'utf8' });
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(ok.stdout, /## Retrieval labels/);
    assert.equal(JSON.parse(readFileSync(out, 'utf8')).kind, 'hollow-retrieval-labels');
    writeFileSync(join(dir, 'x.json'), '{}');
    const none = spawnSync(process.execPath, [script, dir], { encoding: 'utf8' });
    assert.equal(none.status, 1);
    assert.equal(spawnSync(process.execPath, [script], { encoding: 'utf8' }).status, 2);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
