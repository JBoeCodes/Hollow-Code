import { describe, expect, it } from 'vitest';
import { capsSentence, checkedSentence, gateSentence, preflightFacts, routeReason, routeWhere, runnerStateLabel, runnerStatusStale } from './runnerStatus';
import type { RunnerGate, RunnerStatus } from './types';

const status = (over: Partial<RunnerStatus>): RunnerStatus => ({
  host: 'runner-host', state: 'ready', checkedAt: null, millis: null, message: null, machine: null,
  preflight: null, running: null, gateRunning: false, lastGate: null, ...over,
});
const gate = (over: Partial<RunnerGate>): RunnerGate => ({
  start: new Date(Date.now() - 20 * 60_000).toISOString(), end: new Date(Date.now() - 17 * 60_000).toISOString(),
  seconds: 151, ok: true, exitCode: 0, failedStep: null, commit: '4c5517b304bb29619c90acd925a00b13539e4e37', checkout: 'ri27-mm5', ...over,
});

describe('the remote runner status (overnight N1)', () => {
  it('reads the preflight line as labelled facts, the machine name aside', () => {
    expect(preflightFacts('ok host=build-mac free=243GB mem=92% rust=1.96.0 node=v26.3.1 browser=chromium-1228'))
      .toEqual(['Free disk 243 GB', 'Memory free 92%', 'Rust 1.96.0', 'Node v26.3.1', 'Browser chromium-1228']);
    expect(preflightFacts('no: free=12GB, under 20GB')).toEqual(['Free disk 12 GB']);
    expect(preflightFacts('ready; no hollow-run-preflight there')).toEqual(['ready; no hollow-run-preflight there']);
    expect(preflightFacts(null)).toEqual([]);
  });

  it('says how the last gate went, for which checkout and commit', () => {
    expect(gateSentence(gate({}), false)).toBe('Last gate there for ri27-mm5 at 4c5517b3: passed 17m ago in 2 min 31 s.');
    expect(gateSentence(gate({ ok: false, exitCode: 1, failedStep: 'cargo test' }), false))
      .toBe('Last gate there for ri27-mm5 at 4c5517b3: failed 17m ago at cargo test.');
    expect(gateSentence(gate({ end: null, seconds: null, ok: null }), true)).toBe('A gate is running there now for ri27-mm5 at 4c5517b3, started 20m ago.');
    expect(gateSentence(gate({ end: null, seconds: null, ok: null }), false)).toContain("didn't finish");
    expect(gateSentence(gate({ seconds: 900 }), true)).toBe('Last gate there for ri27-mm5 at 4c5517b3: passed 17m ago in 15 min. Another gate is running there now.');
    expect(gateSentence(null, false)).toBe('No gate has run there yet.');
  });

  it('checks again only when the last answer is missing or over five minutes old', () => {
    const now = Date.parse('2026-11-02T07:30:00Z');
    expect(runnerStatusStale(status({ state: 'unchecked' }), now)).toBe(true);
    expect(runnerStatusStale(status({ checkedAt: '2026-11-02T07:27:00Z' }), now)).toBe(false);
    expect(runnerStatusStale(status({ checkedAt: '2026-11-02T07:20:00Z' }), now)).toBe(true);
    expect(runnerStatusStale(status({ host: null, state: 'none' }), now)).toBe(false);
  });

  it('labels every state and says when it was checked', () => {
    expect(runnerStateLabel('ready')).toBe('Answering');
    expect(runnerStateLabel('unreachable')).toBe('Not answering');
    expect(runnerStateLabel('refusing')).toBe('Not taking work');
    expect(checkedSentence(status({ checkedAt: new Date(Date.now() - 90_000).toISOString(), millis: 1180 }))).toBe('checked 1m ago (1.2 s)');
    expect(checkedSentence(status({}))).toBe('');
  });
});

describe('the runner\'s tools and routing decisions (smart routing 3)', () => {
  it('says what the runner has and lacks, leaving out gcloud and its load', () => {
    expect(capsSentence('rust=1.96.0 node=26.3.1 uv=0.13.5 python3=3.14.6 postgres= playwright=chromium-1243 gcloud=yes slots=0/2 cargo=free'))
      .toBe('Has Rust 1.96.0, Node 26.3.1, uv 0.13.5, Python 3.14.6, Playwright chromium-1243; no Postgres, so commands needing it run here.');
    expect(capsSentence('rust= node= postgres=')).toBe('Has none of the tools Hollow looks for; no Rust or Node or Postgres, so commands needing them run here.');
    expect(capsSentence('something else')).toBe('something else');
  });
  it('names where a decision ran and what ran', () => {
    expect(routeWhere('here')).toBe('This Mac');
    expect(routeWhere('runner-host')).toBe('Runner');
    expect(routeWhere('runner-host', 'runner-host')).toBe('Runner');
    expect(routeWhere('build-box', 'runner-host')).toBe('build-box');
    expect(routeReason('gate', 'gate, usually 3 min')).toBe('gate, usually 3 min');
    expect(routeReason('pwd', 'asked for with --remote')).toBe('pwd: asked for with --remote');
    expect(routeReason('', 'why')).toBe('why');
  });
});
