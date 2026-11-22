import { describe, expect, it } from 'vitest';
import { artifactContinuationPrompt, artifactSessionTurns, canResumeArtifactSession } from './artifactSession';
import type { ArtifactSessionContext } from './types';
import type { EvidenceReceiptWire } from './executionReceipt';

const fixture = (): ArtifactSessionContext => ({
  vaultPath: '/vault',
  detail: {
    summary: { key: 's-20260923-first', title: 'A project', agent: 'codex', harnessSessionId: 'thread-id',
      cwd: '/worktree', createdAt: '', lastActivityAt: '', endedAt: null, turnCount: 2,
      parentKey: null, forkKind: null, adopted: false, stints: [], resumable: 'native' },
    turns: [1, 2].map(ord => ({ ord, runId: `run-${ord}`, startedAt: '', endedAt: null, status: 'success', asked: `Question ${ord}` })),
    eventsRetained: true,
  },
  answers: { 1: 'First answer' },
  receipts: [],
});
const receipt = (runId: string): EvidenceReceiptWire => ({
  id: `receipt-${runId}`, runId, historyRunId: runId, source: 'chat', sourceId: null,
  startedAt: '', finishedAt: '', status: 'success', verificationState: 'changedUnverified', files: [], checks: [], failures: [],
});

describe('artifact session continuation', () => {
  it('joins receipts by exact run and keeps missing summaries explicit', () => {
    const context = fixture();
    context.receipts = [receipt('run-2'), receipt('unrelated')];
    const turns = artifactSessionTurns(context);
    expect(turns[1].text).toBe('First answer');
    expect(turns[1].receipt).toBeUndefined();
    expect(turns[3].receipt?.id).toBe('receipt-run-2');
    expect(turns[3].text).toContain('No retained assistant summary');
    // Paperclip PR 17: the assistant turn carries its exact recorded run.
    expect(turns[1].runId).toBe('run-1');
    expect(turns[3].runId).toBe('run-2');
    expect(turns[0].runId).toBeUndefined();
  });

  it('does not resume a most-recent-only or unknown provider even with an id', () => {
    const context = fixture();
    expect(canResumeArtifactSession(context)).toBe(true);
    for (const agent of ['antigravity', 'unknown']) {
      context.detail.summary.agent = agent;
      expect(canResumeArtifactSession(context)).toBe(false);
    }
    context.detail.summary.agent = 'claude';
    context.detail.summary.harnessSessionId = null;
    expect(canResumeArtifactSession(context)).toBe(false);
  });

  it('bounds the handoff and provides a vault root when cwd is elsewhere', () => {
    const context = fixture();
    context.detail.turns = Array.from({ length: 10 }, (_, index) => ({ ...context.detail.turns[0], ord: index + 1, asked: 'x'.repeat(10000) }));
    context.answers = { 10: 'y'.repeat(30000) };
    const prompt = artifactContinuationPrompt('notes/plan.md', context, false);
    expect(prompt).toContain('Hollow vault root: "/vault"');
    expect(prompt).toContain('new conversation');
    expect(prompt).toContain('Turn 8');
    expect(prompt).not.toContain('Turn 7');
    expect(prompt.length).toBeLessThan(11000);
    expect(artifactContinuationPrompt('notes/plan.md', context, true)).not.toContain('Assistant summary:');
  });
});
