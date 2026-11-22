import { describe, expect, it } from 'vitest';
import { stepDuration, stepLabel, stepStatus, timelineRows, type RunStep } from './flightRecorder';

const s = (seq: number, kind: RunStep['kind'], over: Partial<RunStep> = {}): RunStep => ({
  sessionKey: 's-1', turnOrd: 1, seq, runId: 'run-1', kind, name: kind === 'model' ? null : 'Bash', toolId: null,
  parentSeq: null, startedAt: '2026-10-09T05:00:00Z', endedAt: '2026-10-09T05:00:01.500Z', status: 'ok', files: [],
  checkpoint: null, ...over,
});

describe('flight recorder timeline', () => {
  it('orders steps by sequence and nests a delegate’s steps under it', () => {
    const steps = [
      s(5, 'model'),
      s(3, 'tool', { parentSeq: 2, name: 'Read' }),
      s(1, 'model'),
      s(2, 'delegate', { name: 'Task' }),
      s(4, 'tool', { parentSeq: 2, status: 'error' }),
    ];
    const rows = timelineRows({ steps, firstError: 4, fedBy: [2, 3] });
    expect(rows.map((r) => [r.step.seq, r.depth])).toEqual([[1, 0], [2, 0], [3, 1], [4, 1], [5, 0]]);
    expect(rows.find((r) => r.step.seq === 4)!.firstError).toBe(true);
    expect(rows.filter((r) => r.fedTheError).map((r) => r.step.seq)).toEqual([2, 3]);
  });

  it('shows a step whose parent is missing (or later) at the top level', () => {
    const rows = timelineRows({ steps: [s(2, 'tool', { parentSeq: 9 }), s(1, 'tool', { parentSeq: 3 })], firstError: null, fedBy: [] });
    expect(rows.map((r) => [r.step.seq, r.depth])).toEqual([[1, 0], [2, 0]]);
  });

  it('labels steps, durations and statuses for a person', () => {
    expect(stepLabel(s(1, 'write', { name: 'Write', files: [{ path: 'w/out/report.md', state: 'captured' }] }))).toBe('Write · report.md');
    expect(stepLabel(s(1, 'delegate', { name: 'Task' }))).toBe('Delegate · Task');
    expect(stepDuration(s(1, 'tool'))).toBe('1.5 s');
    expect(stepDuration(s(1, 'tool', { endedAt: null }))).toBe('—');
    expect(stepStatus('interrupted')).toEqual({ text: 'interrupted', bad: true });
    expect(stepStatus('ok').bad).toBe(false);
  });
});
