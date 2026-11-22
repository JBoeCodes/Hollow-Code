import { describe, expect, it } from 'vitest';
import type { RunContextView } from '../types';
import { contextReportLine } from './contextReport';

function view(over: Partial<RunContextView['manifest']> = {}): RunContextView {
  return {
    runId: 'run-20261006-abc',
    source: 'chat',
    sessionKey: 's-1',
    finishedAt: '2026-10-06T12:00:00Z',
    manifest: {
      schemaVersion: 1,
      runId: 'run-20261006-abc',
      layers: [{ name: 'request', bytes: 12, sha256: 'a'.repeat(64) }],
      jev: [],
      capabilities: { ownedServers: [], connectors: [] },
      fallbacks: [],
      preparation: {},
      ...over,
    },
    jevCalls: { sent: 0, notSent: 0, latencyMs: 0, outcomes: {}, costUsd: null },
    costRollup: { ownUsd: null, descendantsUsd: 0, totalUsd: null, knownUsd: 0, unknown: 1, descendants: [] },
  };
}

const candidate = (kept: boolean) => ({
  path: 'notes/a.md', line: 1, source: 'note', coverage: 'excerpt', rank: 1, localScore: 2,
  kept, basis: 'jev', bytes: 10, sha256: 'b'.repeat(64),
});

describe('contextReportLine (agent effectiveness F11)', () => {
  it('summarises the named run from its manifest', () => {
    const line = contextReportLine(view({
      retrieval: { engine: 'jev', candidates: [candidate(true), candidate(false), candidate(true)], notices: [] },
      connectors: {
        engine: 'jev',
        decisions: [
          { id: 'crm', attach: true, basis: 'named' },
          { id: 'docs', attach: false, basis: 'jev', score: 0 },
        ],
      },
    }));
    expect(line).toBe('Last request (Jev; run run-20261006-abc): kept 2 of 3 passages, attached 1 of 2 “when relevant” connectors.');
  });

  it('says when Jev was unavailable and carries the fallbacks, not session resets', () => {
    const line = contextReportLine(view({
      retrieval: { engine: 'jev-fallback', candidates: [candidate(true)], notices: [] },
      fallbacks: [
        { stage: 'passages', detail: 'local order: Jev timed out.' },
        { stage: 'session', detail: 'fresh native session with the conversation rebuilt (config changed)' },
      ],
    }));
    expect(line).toBe('Last request (local, Jev unavailable; run run-20261006-abc): kept 1 of 1 passages. local order: Jev timed out.');
  });

  it('says Jev (reused) when the connectors reused the conversation’s last Jev decision (S28)', () => {
    const line = contextReportLine(view({
      retrieval: { engine: 'local', candidates: [candidate(true)], notices: [] },
      connectors: {
        engine: 'cached',
        decisions: [{ id: 'docs', attach: true, basis: 'cached', score: 3 }],
      },
    }));
    expect(line).toBe('Last request (Jev (reused); run run-20261006-abc): kept 1 of 1 passages, attached 1 of 1 “when relevant” connectors.');
  });

  it('is local with no retrieval or connectors', () => {
    expect(contextReportLine(view())).toBe('Last request (local; run run-20261006-abc): kept 0 of 0 passages.');
  });
});
