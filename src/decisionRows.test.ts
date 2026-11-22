import { describe, expect, it } from 'vitest';
import { decisionLabel, decisionRows, decisionScope, libraryDecisionRows, rejectedLine } from './decisionRows';
import type { DecisionRecord } from './recordTypes';

const d = (path: string, scope: string, status: DecisionRecord['status'], decidedAt: string, decidedBy = 'Ana'): DecisionRecord => ({
  path, text: path, decidedAt, decidedBy, scope, status, supersedes: null, supersededBy: null, evidence: null, source: null,
});

describe('decision rows (master plan #20)', () => {
  it('lists the workspace’s own, settled first, newest first, without superseded or global ones', () => {
    const rows = decisionRows([
      d('old', 'ops', 'settled', '2026-10-01'),
      d('new', 'ops', 'settled', '2026-10-05'),
      d('again', 'ops', 'revisit', '2026-10-06'),
      d('gone', 'ops', 'superseded', '2026-09-01'),
      d('global', 'global', 'settled', '2026-10-07'),
      d('other', 'rfp', 'settled', '2026-10-07'),
    ], 'ops');
    expect(rows.map(r => r.path)).toEqual(['new', 'old', 'again']);
  });
  it('keeps a retracted decision out of both lists (DR1: it stays in the records)', () => {
    const list = [d('kept', 'ops', 'settled', '2026-10-01'), d('withdrawn', 'ops', 'retracted', '2026-10-02')];
    expect(decisionRows(list, 'ops').map(r => r.path)).toEqual(['kept']);
    expect(libraryDecisionRows(list).map(r => r.path)).toEqual(['kept']);
  });
  it('labels who decided only when known', () => {
    expect(decisionLabel(d('x', 'ops', 'settled', '2026-10-04'))).toBe('Settled 2026-10-04 by Ana');
    expect(decisionLabel(d('x', 'ops', 'revisit', '2026-10-04', 'unknown'))).toBe('Revisit (decided 2026-10-04)');
  });
  it('Library, Records lists every scope, revisit first, without superseded ones (COO rebuild L4)', () => {
    const rows = libraryDecisionRows([
      d('old', 'ops', 'settled', '2026-10-01'),
      d('again', 'rfp', 'revisit', '2026-10-02'),
      d('global', 'global', 'settled', '2026-10-07'),
      d('gone', 'ops', 'superseded', '2026-10-09'),
    ]);
    expect(rows.map(r => r.path)).toEqual(['again', 'global', 'old']);
  });
  it('names a decision’s scope by its workspace, or Everywhere', () => {
    const titles: Record<string, string> = { ops: 'Ops review' };
    expect(decisionScope('ops', id => titles[id])).toBe('Ops review');
    expect(decisionScope('global', id => titles[id])).toBe('Everywhere');
    expect(decisionScope('gone', id => titles[id])).toBe('gone');
  });
  it('reads a proposal’s rejected text for the review card (DR1)', () => {
    expect(rejectedLine('Use Redis -> too heavy; Use SQLite')).toBe('Rejected: Use Redis (too heavy); Use SQLite');
    expect(rejectedLine('')).toBe('');
    expect(rejectedLine(undefined)).toBe('');
    expect(rejectedLine(' ; -> ')).toBe('');
  });
});
