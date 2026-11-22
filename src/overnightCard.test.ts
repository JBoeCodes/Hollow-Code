import { describe, expect, it } from 'vitest';
import type { OvernightSummary } from './types';
import { commitLabel, failedLabel, overnightHeadline, prLabel } from './overnightCard';

const night = (over: Partial<OvernightSummary> = {}): OvernightSummary => ({
  night: '2026-10-09', from: '', to: '', ran: true, runs: 12, conversations: 2, succeeded: 10,
  failed: [], pullRequests: [], commits: [], waiting: { total: 0, titles: [] }, report: null, ...over,
});

describe('overnight card', () => {
  it('sums the night in one line, naming only what happened', () => {
    expect(overnightHeadline(night(), 0)).toBe('12 runs in 2 conversations');
    expect(overnightHeadline(night({
      runs: 1, conversations: 1,
      pullRequests: [
        { repo: 'o/r', number: 1, action: 'merged', at: '', sessionKey: 'k', sessionTitle: null },
        { repo: 'o/r', number: 2, action: 'opened', at: '', sessionKey: 'k', sessionTitle: null },
      ],
      failed: [{ status: 'error', resultKind: null, failedChecks: 0, at: '', sessionKey: null, sessionTitle: null, source: 'chat' }],
      waiting: { total: 9, titles: [] },
    }), 3)).toBe('1 run in 1 conversation · 1 pull request merged · 1 failed · 3 waiting on you');
  });

  it('labels pull requests, commits and failures in words', () => {
    expect(commitLabel({ repo: 'Hollow', from: 'aaaaaaaa', to: 'cccccccc', runs: 2, at: '' })).toBe('Hollow: aaaaaaaa → cccccccc (2 runs)');
    expect(commitLabel({ repo: 'Hollow', from: 'a', to: 'b', runs: 1, at: '' })).toBe('Hollow: a → b');
    expect(prLabel({ repo: 'example-org/hollow-code', number: 451, action: 'merged', at: '', sessionKey: 'k', sessionTitle: 'Overnight lane B' }))
      .toBe('Merged #451 in example-org/hollow-code · Overnight lane B');
    expect(prLabel({ repo: null, number: 7, action: 'opened', at: '', sessionKey: 'k', sessionTitle: null })).toBe('Opened #7');
    const f = { status: 'success', resultKind: null, failedChecks: 2, at: '', sessionKey: 'k', sessionTitle: 'Lane A', source: 'chat' };
    expect(failedLabel(f)).toBe('Lane A: 2 checks failed');
    expect(failedLabel({ ...f, failedChecks: 0, status: 'error', sessionTitle: null })).toBe('A conversation: stopped with an error');
    expect(failedLabel({ ...f, failedChecks: 0, status: 'timeout', resultKind: 'timedOut', sessionTitle: null, source: 'automation' }))
      .toBe('An automation run: timed out');
    expect(failedLabel({ ...f, failedChecks: 0, status: 'weird', resultKind: null, sessionTitle: null, source: 'heartbeat' }))
      .toBe('A heartbeat run: did not finish');
  });
});
